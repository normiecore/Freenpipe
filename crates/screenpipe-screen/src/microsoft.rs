//! Windows OCR through the WinRT `OcrEngine`.
//!
//! Every WinRT call runs on one long-lived thread, `screenpipe-windows-ocr`
//! (see [`crate::ocr_worker`]), which joins the multithreaded apartment once
//! and keeps one engine per language set. Live capture and the on-demand
//! frame OCR route share its bounded queue: when it is full a request is
//! refused at once and the frame is stored without OCR text. Images are
//! converted to BGRA and checked (zero or oversized dimensions, buffer length)
//! before a `SoftwareBitmap` is created from them.

#[cfg(target_os = "windows")]
use anyhow::{anyhow, Result};
#[cfg(target_os = "windows")]
use image::DynamicImage;
#[cfg(any(target_os = "windows", test))]
use screenpipe_core::Language;

#[cfg(target_os = "windows")]
use crate::ocr_worker::{
    handler_factory, validate_bgra_layout, BgraImage, DedicatedWorker, OcrImageError,
    OcrWorkerError, WorkerConfig, WorkerStats, FALLBACK_MAX_IMAGE_DIMENSION, LIVE_OCR_TIMEOUT,
    OCR_PIXEL_BUDGET, WINDOWS_OCR_THREAD_NAME,
};
#[cfg(target_os = "windows")]
use once_cell::sync::Lazy;
#[cfg(target_os = "windows")]
use std::collections::HashMap;
#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicU32, Ordering};
#[cfg(target_os = "windows")]
use std::time::{Duration, Instant};
#[cfg(target_os = "windows")]
use tracing::{debug, info, warn};

#[cfg(target_os = "windows")]
use windows::{
    core::HSTRING,
    Globalization::Language as WindowsLanguage,
    Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine as WindowsOcrEngine,
    Storage::Streams::DataWriter,
    Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
};

/// A word the engine recognised, with its box in bitmap pixels.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq)]
struct RecognizedWord {
    text: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// Joins recognised words into the flat text and the per-word JSON, with
/// boxes normalised to 0..1 of the bitmap like the other OCR backends.
/// `fallback_text` is used when no word carried text.
#[cfg(any(target_os = "windows", test))]
fn assemble_ocr_output(
    words: &[RecognizedWord],
    fallback_text: &str,
    width: u32,
    height: u32,
) -> (String, String) {
    let img_w = width.max(1) as f32;
    let img_h = height.max(1) as f32;
    let mut full_text = String::new();
    let mut boxes: Vec<serde_json::Value> = Vec::with_capacity(words.len());
    for word in words.iter().filter(|w| !w.text.is_empty()) {
        if !full_text.is_empty() {
            full_text.push(' ');
        }
        full_text.push_str(&word.text);
        boxes.push(serde_json::json!({
            "text": word.text,
            "left": (word.x / img_w).to_string(),
            "top": (word.y / img_h).to_string(),
            "width": (word.width / img_w).to_string(),
            "height": (word.height / img_h).to_string(),
            // WinRT gives no per-word confidence.
            "conf": "1.0"
        }));
    }
    if full_text.is_empty() {
        full_text = fallback_text.to_string();
    }
    let json = serde_json::to_string(&boxes).unwrap_or_else(|_| "[]".to_string());
    (full_text, json)
}

/// Cache key of an engine: the requested languages, or "" for the user's
/// profile languages.
#[cfg(any(target_os = "windows", test))]
fn language_key(languages: &[Language]) -> String {
    languages
        .iter()
        .map(|l| l.as_lang_code())
        .collect::<Vec<_>>()
        .join("+")
}

/// What the OCR thread reports about the installed engine.
#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowsOcrProbe {
    /// BCP-47 tags of the installed OCR recognizer languages.
    pub recognizer_languages: Vec<String>,
    /// Whether an engine could be created from the user's profile languages.
    pub user_profile_engine: bool,
    /// `OcrEngine::MaxImageDimension`.
    pub max_image_dimension: u32,
}

#[cfg(target_os = "windows")]
enum WinOcrJob {
    Recognize {
        image: BgraImage,
        languages: Vec<Language>,
    },
    Probe,
}

#[cfg(target_os = "windows")]
enum WinOcrReply {
    Text(std::result::Result<(String, String, Option<f64>), OcrWorkerError>),
    Probe(WindowsOcrProbe),
}

/// Largest side the engine accepts, as read by the worker thread (zero until
/// the thread has started).
#[cfg(target_os = "windows")]
static MAX_IMAGE_DIMENSION: AtomicU32 = AtomicU32::new(0);

#[cfg(target_os = "windows")]
fn caller_max_side() -> u32 {
    match MAX_IMAGE_DIMENSION.load(Ordering::Relaxed) {
        0 => FALLBACK_MAX_IMAGE_DIMENSION,
        v => v,
    }
}

/// How long a failed engine creation is remembered before it is retried
/// (a language pack may have been installed meanwhile).
#[cfg(target_os = "windows")]
const ENGINE_RETRY_AFTER: Duration = Duration::from_secs(60);

#[cfg(target_os = "windows")]
static WINDOWS_OCR_WORKER: Lazy<DedicatedWorker<WinOcrJob, WinOcrReply>> = Lazy::new(|| {
    DedicatedWorker::new(
        WorkerConfig::windows_ocr(),
        handler_factory(new_windows_ocr_handler),
    )
});

#[cfg(target_os = "windows")]
fn new_windows_ocr_handler() -> impl FnMut(WinOcrJob) -> WinOcrReply {
    let mut thread = WindowsOcrThread::enter();
    move |job| thread.handle(job)
}

/// Joins the multithreaded WinRT apartment for the life of the OCR thread.
#[cfg(target_os = "windows")]
struct WinRtApartment {
    joined: bool,
}

#[cfg(target_os = "windows")]
impl WinRtApartment {
    fn enter() -> Self {
        // SAFETY: called once on the dedicated OCR thread before any WinRT
        // use; balanced by RoUninitialize in Drop on the same thread.
        match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            Ok(()) => Self { joined: true },
            Err(e) => {
                warn!(
                    "windows OCR thread could not join the WinRT apartment ({}); continuing",
                    e
                );
                Self { joined: false }
            }
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for WinRtApartment {
    fn drop(&mut self) {
        if self.joined {
            // SAFETY: same thread that joined in `enter`.
            unsafe { RoUninitialize() };
        }
    }
}

#[cfg(target_os = "windows")]
enum CachedEngine {
    Ready {
        engine: WindowsOcrEngine,
        tag: String,
    },
    Failed {
        message: String,
        at: Instant,
    },
}

/// State owned by the OCR thread. Field order matters: the engines are
/// released before the apartment is left.
#[cfg(target_os = "windows")]
struct WindowsOcrThread {
    engines: HashMap<String, CachedEngine>,
    max_side: u32,
    _apartment: WinRtApartment,
}

#[cfg(target_os = "windows")]
impl WindowsOcrThread {
    fn enter() -> Self {
        let apartment = WinRtApartment::enter();
        let max_side = match WindowsOcrEngine::MaxImageDimension() {
            Ok(v) if v > 0 => v,
            Ok(_) => FALLBACK_MAX_IMAGE_DIMENSION,
            Err(e) => {
                debug!(
                    "could not read the OCR image size limit ({}); assuming the default",
                    e
                );
                FALLBACK_MAX_IMAGE_DIMENSION
            }
        };
        MAX_IMAGE_DIMENSION.store(max_side, Ordering::Relaxed);
        info!(
            thread = WINDOWS_OCR_THREAD_NAME,
            max_image_dimension = max_side,
            "windows OCR engine thread {} ready",
            WINDOWS_OCR_THREAD_NAME
        );
        Self {
            engines: HashMap::new(),
            max_side,
            _apartment: apartment,
        }
    }

    fn handle(&mut self, job: WinOcrJob) -> WinOcrReply {
        match job {
            WinOcrJob::Probe => WinOcrReply::Probe(self.probe()),
            WinOcrJob::Recognize { image, languages } => {
                WinOcrReply::Text(self.recognize(image, &languages))
            }
        }
    }

    fn probe(&self) -> WindowsOcrProbe {
        let recognizer_languages = match available_windows_ocr_language_tags() {
            Ok(tags) => tags,
            Err(e) => {
                warn!("could not list the Windows OCR recognizer languages: {}", e);
                Vec::new()
            }
        };
        let user_profile_engine = WindowsOcrEngine::TryCreateFromUserProfileLanguages().is_ok();
        WindowsOcrProbe {
            recognizer_languages,
            user_profile_engine,
            max_image_dimension: self.max_side,
        }
    }

    fn recognize(
        &mut self,
        image: BgraImage,
        languages: &[Language],
    ) -> std::result::Result<(String, String, Option<f64>), OcrWorkerError> {
        if crate::ocr_availability::ocr_fault_injected() {
            crate::metrics::set_ocr_engine_available(false);
            return Err(OcrWorkerError::Unavailable(
                crate::ocr_availability::SIMULATED_REASON.to_string(),
            ));
        }
        // The caller scaled against its own idea of the limit; enforce the
        // engine's real one before any WinRT object is created.
        let image = image.fit_within(self.max_side, OCR_PIXEL_BUDGET)?;
        let (width, height) = (image.width(), image.height());
        validate_bgra_layout(
            width,
            height,
            width as usize * 4,
            image.bytes().len(),
            self.max_side,
        )?;
        let overflow = OcrImageError::Overflow { width, height };
        let w = i32::try_from(width).map_err(|_| OcrWorkerError::InvalidImage(overflow))?;
        let h = i32::try_from(height).map_err(|_| OcrWorkerError::InvalidImage(overflow))?;

        let (engine, tag) = self.engine_for(languages)?;
        let started = Instant::now();
        let (text, json) = run_recognition(&engine, &image, w, h)
            .map_err(|e| OcrWorkerError::Engine(e.to_string()))?;
        crate::metrics::set_ocr_engine_available(true);
        debug!(
            language = %tag,
            width,
            height,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "windows OCR done"
        );
        Ok((text, json, Some(1.0)))
    }

    fn engine_for(
        &mut self,
        languages: &[Language],
    ) -> std::result::Result<(WindowsOcrEngine, String), OcrWorkerError> {
        let key = language_key(languages);
        match self.engines.get(&key) {
            Some(CachedEngine::Ready { engine, tag }) => return Ok((engine.clone(), tag.clone())),
            Some(CachedEngine::Failed { message, at }) if at.elapsed() < ENGINE_RETRY_AFTER => {
                crate::metrics::set_ocr_engine_available(false);
                return Err(OcrWorkerError::Unavailable(message.clone()));
            }
            _ => {}
        }
        match create_windows_ocr_engine(languages) {
            Ok((engine, tag)) => {
                info!(language = %tag, "windows OCR engine created");
                self.engines.insert(
                    key,
                    CachedEngine::Ready {
                        engine: engine.clone(),
                        tag: tag.clone(),
                    },
                );
                Ok((engine, tag))
            }
            Err(e) => {
                let message = e.to_string();
                warn!("{}", message);
                // Reported on /health as vision_reason=ocr_unavailable.
                crate::metrics::set_ocr_engine_available(false);
                self.engines.insert(
                    key,
                    CachedEngine::Failed {
                        message: message.clone(),
                        at: Instant::now(),
                    },
                );
                Err(OcrWorkerError::Unavailable(message))
            }
        }
    }
}

/// Copies the BGRA pixels into a `SoftwareBitmap` and recognises it.
#[cfg(target_os = "windows")]
fn run_recognition(
    engine: &WindowsOcrEngine,
    image: &BgraImage,
    width: i32,
    height: i32,
) -> windows::core::Result<(String, String)> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(image.bytes())?;
    let buffer = writer.DetachBuffer()?;
    let bitmap =
        SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, width, height)?;
    let result = engine.RecognizeAsync(&bitmap)?.get()?;

    let mut words = Vec::new();
    for line in result.Lines()? {
        for word in line.Words()? {
            let text = word.Text()?.to_string();
            if text.is_empty() {
                continue;
            }
            let rect = word.BoundingRect()?;
            words.push(RecognizedWord {
                text,
                x: rect.X,
                y: rect.Y,
                width: rect.Width,
                height: rect.Height,
            });
        }
    }
    let fallback = if words.is_empty() {
        result.Text()?.to_string()
    } else {
        String::new()
    };
    // Free the pixel copy now rather than whenever COM gets to it.
    let _ = bitmap.Close();
    Ok(assemble_ocr_output(
        &words,
        &fallback,
        image.width(),
        image.height(),
    ))
}

/// Recognises the text in `image` on the Windows OCR thread, waiting at most
/// [`LIVE_OCR_TIMEOUT`]. Fails fast (without blocking) when the queue is full
/// or no engine is installed; such errors carry an [`OcrWorkerError`] whose
/// [`OcrWorkerError::is_skip`] is true.
#[cfg(target_os = "windows")]
pub async fn perform_ocr_windows(
    image: &DynamicImage,
    languages: &[Language],
) -> Result<(String, String, Option<f64>)> {
    perform_ocr_windows_with_timeout(image, languages, LIVE_OCR_TIMEOUT).await
}

/// [`perform_ocr_windows`] with a caller-chosen deadline.
#[cfg(target_os = "windows")]
pub async fn perform_ocr_windows_with_timeout(
    image: &DynamicImage,
    languages: &[Language],
    timeout: Duration,
) -> Result<(String, String, Option<f64>)> {
    if crate::ocr_availability::ocr_fault_injected() {
        crate::metrics::set_ocr_engine_available(false);
        return Err(anyhow::Error::new(OcrWorkerError::Unavailable(
            crate::ocr_availability::SIMULATED_REASON.to_string(),
        )));
    }
    let bgra = BgraImage::from_dynamic(image, caller_max_side(), OCR_PIXEL_BUDGET)
        .map_err(|e| anyhow::Error::new(OcrWorkerError::InvalidImage(e)))?;
    let job = WinOcrJob::Recognize {
        image: bgra,
        languages: languages.to_vec(),
    };
    match WINDOWS_OCR_WORKER.run(job, timeout).await {
        Ok(WinOcrReply::Text(result)) => result.map_err(anyhow::Error::new),
        Ok(WinOcrReply::Probe(_)) => Err(anyhow::Error::new(OcrWorkerError::Engine(
            "unexpected probe reply to a recognition request".to_string(),
        ))),
        Err(e) => Err(anyhow::Error::new(e)),
    }
}

/// Asks the OCR thread which recognizer languages are installed.
#[cfg(target_os = "windows")]
pub async fn probe_windows_ocr() -> std::result::Result<WindowsOcrProbe, OcrWorkerError> {
    match WINDOWS_OCR_WORKER
        .run(WinOcrJob::Probe, Duration::from_secs(30))
        .await?
    {
        WinOcrReply::Probe(probe) => Ok(probe),
        WinOcrReply::Text(_) => Err(OcrWorkerError::Engine(
            "unexpected recognition reply to a probe".to_string(),
        )),
    }
}

/// Counters of the Windows OCR thread.
#[cfg(target_os = "windows")]
pub fn windows_ocr_worker_stats() -> WorkerStats {
    WINDOWS_OCR_WORKER.stats()
}

#[cfg(target_os = "windows")]
fn create_windows_ocr_engine(languages: &[Language]) -> Result<(WindowsOcrEngine, String)> {
    if !languages.is_empty() {
        return create_windows_ocr_engine_for_requested_languages(languages);
    }

    match WindowsOcrEngine::TryCreateFromUserProfileLanguages() {
        Ok(engine) => {
            let tag = recognizer_language_tag(&engine);
            Ok((engine, tag))
        }
        Err(profile_err) => {
            let available_tags = available_windows_ocr_language_tags().unwrap_or_default();
            if available_tags.is_empty() {
                return Err(anyhow!(
                    "Windows OCR unavailable: no OCR recognizer languages are installed. \
                     Install a Windows OCR language pack in Settings > Time & language > Language & region. \
                     User profile engine creation failed: {}",
                    profile_err
                ));
            }

            for tag in &available_tags {
                if let Some(engine) = try_create_windows_ocr_engine_for_tag(tag)? {
                    tracing::debug!(
                        "windows OCR user profile languages did not create an engine; falling back to installed recognizer language: {}",
                        tag
                    );
                    return Ok((engine, tag.clone()));
                }
            }

            Err(anyhow!(
                "Windows OCR unavailable: user profile languages do not match any installed OCR recognizer. \
                 Available Windows OCR recognizer languages: {}. \
                 User profile engine creation failed: {}",
                format_available_tags(&available_tags),
                profile_err
            ))
        }
    }
}

#[cfg(target_os = "windows")]
fn create_windows_ocr_engine_for_requested_languages(
    languages: &[Language],
) -> Result<(WindowsOcrEngine, String)> {
    let available_tags = available_windows_ocr_language_tags().unwrap_or_default();
    let mut attempted_tags: Vec<&'static str> = Vec::new();

    for language in languages {
        for tag in windows_language_tags_for(language) {
            if attempted_tags.contains(&tag) {
                continue;
            }
            attempted_tags.push(tag);

            if let Some(engine) = try_create_windows_ocr_engine_for_tag(tag)? {
                return Ok((engine, tag.to_string()));
            }
        }
    }

    Err(anyhow!(
        "Windows OCR unavailable for requested language(s): {}. \
         Tried Windows language tag(s): {}. \
         Available Windows OCR recognizer languages: {}. \
         Install the matching Windows OCR language pack or pass a language that is available.",
        languages
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        attempted_tags.join(", "),
        format_available_tags(&available_tags)
    ))
}

#[cfg(target_os = "windows")]
fn try_create_windows_ocr_engine_for_tag(tag: &str) -> Result<Option<WindowsOcrEngine>> {
    let language_tag = HSTRING::from(tag);
    let language = WindowsLanguage::CreateLanguage(&language_tag)?;
    if !WindowsOcrEngine::IsLanguageSupported(&language)? {
        return Ok(None);
    }

    match WindowsOcrEngine::TryCreateFromLanguage(&language) {
        Ok(engine) => Ok(Some(engine)),
        Err(err) if is_null_ocr_engine_error(&err) => Ok(None),
        Err(err) => Err(anyhow!(
            "failed to create Windows OCR engine for language tag '{}': {}",
            tag,
            err
        )),
    }
}

#[cfg(target_os = "windows")]
fn is_null_ocr_engine_error(err: &windows::core::Error) -> bool {
    // windows-rs reports null WinRT interface results as an Error whose HRESULT
    // is success. Turn that confusing "operation completed successfully" state
    // into a normal "no engine for this language" branch.
    err.code().0 == 0
}

#[cfg(target_os = "windows")]
fn recognizer_language_tag(engine: &WindowsOcrEngine) -> String {
    engine
        .RecognizerLanguage()
        .and_then(|language| language.LanguageTag())
        .map(|tag| tag.to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(target_os = "windows")]
fn available_windows_ocr_language_tags() -> Result<Vec<String>> {
    let languages = WindowsOcrEngine::AvailableRecognizerLanguages()?;
    let mut tags = Vec::new();
    for language in languages {
        tags.push(language.LanguageTag()?.to_string());
    }
    Ok(tags)
}

#[cfg(any(target_os = "windows", test))]
fn windows_language_tags_for(language: &Language) -> Vec<&'static str> {
    match language {
        Language::English => vec!["en-US", "en"],
        Language::Chinese => vec!["zh-Hans", "zh-CN", "zh-Hans-CN", "zh"],
        Language::German => vec!["de-DE", "de"],
        Language::Spanish => vec!["es-ES", "es-MX", "es"],
        Language::Russian => vec!["ru-RU", "ru"],
        Language::Korean => vec!["ko-KR", "ko"],
        Language::French => vec!["fr-FR", "fr-CA", "fr"],
        Language::Japanese => vec!["ja-JP", "ja"],
        Language::Portuguese => vec!["pt-BR", "pt-PT", "pt"],
        Language::Turkish => vec!["tr-TR", "tr"],
        Language::Polish => vec!["pl-PL", "pl"],
        Language::Dutch => vec!["nl-NL", "nl"],
        Language::Arabic => vec!["ar-SA", "ar"],
        Language::Swedish => vec!["sv-SE", "sv"],
        Language::Italian => vec!["it-IT", "it"],
        Language::Hindi => vec!["hi-IN", "hi"],
        Language::Vietnamese => vec!["vi-VN", "vi"],
        Language::Finnish => vec!["fi-FI", "fi"],
        Language::Hebrew => vec!["he-IL", "he"],
        Language::Ukrainian => vec!["uk-UA", "uk"],
        Language::Greek => vec!["el-GR", "el"],
        Language::Czech => vec!["cs-CZ", "cs"],
        Language::Romanian => vec!["ro-RO", "ro"],
        Language::Danish => vec!["da-DK", "da"],
        Language::Hungarian => vec!["hu-HU", "hu"],
        Language::Norwegian => vec!["nb-NO", "nn-NO", "no"],
        Language::Thai => vec!["th-TH", "th"],
        Language::Bulgarian => vec!["bg-BG", "bg"],
        Language::Lithuanian => vec!["lt-LT", "lt"],
        Language::Latvian => vec!["lv-LV", "lv"],
        Language::Serbian => vec!["sr-Cyrl-RS", "sr-Latn-RS", "sr"],
        Language::Slovenian => vec!["sl-SI", "sl"],
        Language::Estonian => vec!["et-EE", "et"],
        Language::Croatian => vec!["hr-HR", "hr"],
        _ => vec![language.as_lang_code()],
    }
}

#[cfg(target_os = "windows")]
fn format_available_tags(tags: &[String]) -> String {
    if tags.is_empty() {
        "none".to_string()
    } else {
        tags.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_language_tags_include_chinese_simplified_candidates() {
        let tags = windows_language_tags_for(&Language::Chinese);
        assert_eq!(tags[0], "zh-Hans");
        assert!(tags.contains(&"zh-CN"));
        assert!(tags.contains(&"zh"));
    }

    #[test]
    fn windows_language_tags_include_english_fallback() {
        let tags = windows_language_tags_for(&Language::English);
        assert_eq!(tags, vec!["en-US", "en"]);
    }

    #[test]
    fn assembled_output_normalises_boxes() {
        let words = vec![
            RecognizedWord {
                text: "Hello".into(),
                x: 10.0,
                y: 20.0,
                width: 50.0,
                height: 10.0,
            },
            RecognizedWord {
                text: String::new(),
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            RecognizedWord {
                text: "world".into(),
                x: 70.0,
                y: 20.0,
                width: 30.0,
                height: 10.0,
            },
        ];
        let (text, json) = assemble_ocr_output(&words, "ignored", 100, 40);
        assert_eq!(text, "Hello world");
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["text"], "Hello");
        assert_eq!(parsed[0]["left"], "0.1");
        assert_eq!(parsed[0]["top"], "0.5");
        assert_eq!(parsed[0]["width"], "0.5");
        assert_eq!(parsed[0]["height"], "0.25");
        assert_eq!(parsed[0]["conf"], "1.0");
    }

    #[test]
    fn assembled_output_falls_back_to_the_page_text() {
        let (text, json) = assemble_ocr_output(&[], "page text", 0, 0);
        assert_eq!(text, "page text");
        assert_eq!(json, "[]");
    }

    #[test]
    fn engine_cache_keys_follow_the_language_list() {
        assert_eq!(language_key(&[]), "");
        assert_eq!(
            language_key(&[Language::English, Language::German]),
            "en+de"
        );
    }

    #[test]
    fn windows_language_tags_default_to_core_lang_code() {
        let tags = windows_language_tags_for(&Language::Catalan);
        assert_eq!(tags, vec!["ca"]);
    }
}
