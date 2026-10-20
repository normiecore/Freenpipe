// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! DirectML for the ONNX Parakeet engine (audio-020, shell-030).
//!
//! Where the Parakeet ONNX sessions run is decided here, not in audiopipe:
//!
//! * `SCREENPIPE_DIRECTML` unset or empty: automatic. `1` or `true`: on (an
//!   adapter must still qualify). Any other value (`0`, `false`, `off`, ...):
//!   CPU.
//! * `DirectML.dll`: automatic mode uses DirectML only with a copy next to
//!   the executable; `1`/`true` also accepts the one in System32. Either copy
//!   must export `DMLCreateDevice1` at the ordinal ONNX Runtime imports (see
//!   [`directml_exports_verdict`]).
//! * Adapter: every DXGI adapter except software (WARP, Basic Render Driver)
//!   and remote ones is opened with `D3D12CreateDevice` at feature level
//!   11_0 and asked for its architecture. Of those that pass, a discrete
//!   adapter is preferred, then the one with the most dedicated video memory,
//!   then the lowest ordinal. Its DXGI ordinal is the DirectML device id.
//! * A DirectML provider that cannot be initialised means CPU sessions. An
//!   inference that fails under DirectML releases the DirectML sessions,
//!   loads CPU sessions and runs the same audio once more. Either failure
//!   keeps DirectML off for the rest of the process.
//! * DirectML sessions are rebuilt every [`DIRECTML_RECYCLE_EVERY`]
//!   inferences; a rebuild that fails moves to CPU.
//!
//! The policy and the recovery runner are plain Rust and run on every
//! platform (so Linux tests cover them); only the DXGI probe is Windows code.

use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{error, info, warn};

/// Environment switch (same name as screenpipe's; see `screenpipe_core::env`).
pub const DIRECTML_ENV: &str = "SCREENPIPE_DIRECTML";

/// DirectML sessions are rebuilt after this many inferences, which keeps
/// long sessions from slowing down.
pub const DIRECTML_RECYCLE_EVERY: u32 = 500;

/// Lowest Direct3D feature level an adapter must offer (DirectML's minimum).
pub const MIN_FEATURE_LEVEL: &str = "11_0";

/// How `SCREENPIPE_DIRECTML` was set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectMlSwitch {
    /// Unset or empty: use DirectML when an adapter qualifies.
    Auto,
    /// `1` or `true`.
    On,
    /// Any other value.
    Off,
}

pub fn parse_directml_switch(raw: Option<&str>) -> DirectMlSwitch {
    let Some(value) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return DirectMlSwitch::Auto;
    };
    if value == "1" || value.eq_ignore_ascii_case("true") {
        DirectMlSwitch::On
    } else {
        DirectMlSwitch::Off
    }
}

pub fn directml_switch_from_env() -> DirectMlSwitch {
    parse_directml_switch(std::env::var(DIRECTML_ENV).ok().as_deref())
}

/// Result of the Direct3D 12 checks on one adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum D3d12Check {
    /// A device was created at feature level 11_0 and answered the
    /// architecture query. `uma`: the GPU shares system memory (integrated).
    Capable { uma: bool },
    /// Why the adapter does not qualify.
    Failed(String),
}

/// One DXGI adapter as the probe saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterInfo {
    /// DXGI enumeration ordinal (`EnumAdapters1` index); the DirectML device id.
    pub ordinal: u32,
    pub name: String,
    pub vendor_id: u32,
    pub dedicated_video_memory: u64,
    pub software: bool,
    pub remote: bool,
    pub d3d12: D3d12Check,
}

impl AdapterInfo {
    pub fn dedicated_mib(&self) -> u64 {
        self.dedicated_video_memory / (1024 * 1024)
    }

    pub fn uma(&self) -> Option<bool> {
        match self.d3d12 {
            D3d12Check::Capable { uma } => Some(uma),
            D3d12Check::Failed(_) => None,
        }
    }

    /// Why this adapter cannot run DirectML, or `None` when it can.
    pub fn rejection(&self) -> Option<String> {
        if self.software {
            return Some("software adapter".to_string());
        }
        if self.remote {
            return Some("remote adapter".to_string());
        }
        match &self.d3d12 {
            D3d12Check::Capable { .. } => None,
            D3d12Check::Failed(reason) => Some(reason.clone()),
        }
    }
}

/// Pick the DirectML adapter: hardware, local and D3D12-capable; discrete
/// before integrated, then most dedicated memory, then lowest ordinal.
pub fn select_adapter(adapters: &[AdapterInfo]) -> Option<&AdapterInfo> {
    adapters
        .iter()
        .filter(|a| a.rejection().is_none())
        .min_by_key(|a| {
            (
                a.uma().unwrap_or(true),
                std::cmp::Reverse(a.dedicated_video_memory),
                a.ordinal,
            )
        })
}

/// Where the Parakeet ONNX sessions run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnnxTarget {
    Cpu,
    DirectMl { device_id: i32 },
}

impl OnnxTarget {
    pub fn is_directml(&self) -> bool {
        matches!(self, OnnxTarget::DirectMl { .. })
    }
}

/// The decision and the reason it was taken (logged once per engine).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDecision {
    pub target: OnnxTarget,
    pub reason: String,
}

/// Set after a DirectML failure; Parakeet stays on CPU until restart.
static DIRECTML_DISABLED: AtomicBool = AtomicBool::new(false);

/// The process-wide latch used outside tests.
pub fn directml_latch() -> &'static AtomicBool {
    &DIRECTML_DISABLED
}

/// Pure decision step. `built_with_directml` is whether this binary has the
/// DirectML provider at all (the `directml` feature on Windows);
/// `library_check` names the usable `DirectML.dll` or says why there is none
/// (ONNX Runtime loads it lazily, the first time the provider is used, so an
/// unusable DLL has to be caught before choosing DirectML; see
/// [`choose_directml_library`]).
pub fn decide_target(
    switch: DirectMlSwitch,
    built_with_directml: bool,
    disabled_after_failure: bool,
    library_check: impl FnOnce() -> Result<String, String>,
    probe: impl FnOnce() -> Result<Vec<AdapterInfo>, String>,
) -> TargetDecision {
    let cpu = |reason: String| TargetDecision {
        target: OnnxTarget::Cpu,
        reason,
    };
    if switch == DirectMlSwitch::Off {
        return cpu(format!(
            "{DIRECTML_ENV} explicitly disabled DirectML; using CPU"
        ));
    }
    if !built_with_directml {
        return cpu("this build has no DirectML provider; using CPU".to_string());
    }
    if disabled_after_failure {
        return cpu("DirectML failed earlier in this process; using CPU".to_string());
    }
    let library = match library_check() {
        Ok(library) => library,
        Err(e) => return cpu(format!("{e}; using CPU")),
    };
    let adapters = match probe() {
        Ok(adapters) => adapters,
        Err(e) => return cpu(format!("DXGI adapter enumeration failed ({e}); using CPU")),
    };
    for adapter in &adapters {
        if let Some(why) = adapter.rejection() {
            info!(
                "parakeet: DirectML skips adapter ordinal {} ({}): {}",
                adapter.ordinal, adapter.name, why
            );
        }
    }
    match select_adapter(&adapters) {
        Some(adapter) => TargetDecision {
            target: OnnxTarget::DirectMl {
                device_id: adapter.ordinal as i32,
            },
            reason: format!(
                "DirectML adapter ordinal {}: {} ({} MiB dedicated video memory, UMA={}), {}{}",
                adapter.ordinal,
                adapter.name,
                adapter.dedicated_mib(),
                adapter.uma().unwrap_or(false),
                library,
                if switch == DirectMlSwitch::On {
                    format!(" [{DIRECTML_ENV} on]")
                } else {
                    String::new()
                }
            ),
        },
        None => cpu(
            "no hardware adapter that is non-software, non-remote and D3D12-capable; using CPU"
                .to_string(),
        ),
    }
}

/// Decide for this process: reads `SCREENPIPE_DIRECTML`, the build features
/// and (on Windows) the DXGI adapters.
pub fn decide_target_for_this_process() -> TargetDecision {
    let switch = directml_switch_from_env();
    decide_target(
        switch,
        cfg!(all(windows, feature = "directml")),
        directml_latch().load(Ordering::SeqCst),
        || directml_library_check(switch),
        probe_adapters,
    )
}

// ---------------------------------------------------------------------------
// DirectML.dll
// ---------------------------------------------------------------------------

/// The pinned ONNX Runtime 1.22.0 DirectML build (NuGet
/// `Microsoft.ML.OnnxRuntime.DirectML`) delay-imports `DirectML.dll` by this
/// ordinal, which is [`DIRECTML_REQUIRED_EXPORT`] in the DirectML 1.15.4
/// redistributable (ordinal 1 is `DMLCreateDevice`). The import is bound the
/// first time the provider runs; a DLL without it fails there as a loader
/// exception, not as an error the CPU fallback can catch.
pub const DIRECTML_REQUIRED_ORDINAL: u16 = 2;
/// The export that must sit at [`DIRECTML_REQUIRED_ORDINAL`].
pub const DIRECTML_REQUIRED_EXPORT: &str = "DMLCreateDevice1";

/// Pure check of the two lookups in a loaded `DirectML.dll`: the ordinal
/// ONNX Runtime imports must exist and be `DMLCreateDevice1`.
pub fn directml_exports_verdict(
    by_ordinal: Option<usize>,
    by_name: Option<usize>,
) -> Result<(), String> {
    match (by_ordinal, by_name) {
        (Some(ordinal), Some(name)) if ordinal == name => Ok(()),
        (_, None) => Err(format!(
            "has no {DIRECTML_REQUIRED_EXPORT} export (older than the DirectML the provider needs)"
        )),
        (None, Some(_)) => Err(format!(
            "does not export ordinal {DIRECTML_REQUIRED_ORDINAL}, which ONNX Runtime imports"
        )),
        (Some(_), Some(_)) => Err(format!(
            "exports something other than {DIRECTML_REQUIRED_EXPORT} at ordinal {DIRECTML_REQUIRED_ORDINAL}"
        )),
    }
}

/// Which `DirectML.dll` the provider may use, as a description for the log.
///
/// A copy next to the executable is found first by ONNX Runtime's lazy load
/// and is the one shipped or placed with the app. Automatic mode accepts only
/// that copy: the System32 one that Windows provides can be of any age, and
/// its fitness for the provider has not been checked on hardware.
/// `SCREENPIPE_DIRECTML=1` also accepts the System32 copy. Each candidate has
/// passed [`directml_exports_verdict`] when its closure returns `Ok`.
pub fn choose_directml_library(
    switch: DirectMlSwitch,
    app_dir_copy: impl FnOnce() -> Result<String, String>,
    system_copy: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    let app_dir_error = match app_dir_copy() {
        Ok(found) => return Ok(found),
        Err(e) => e,
    };
    if switch != DirectMlSwitch::On {
        return Err(format!(
            "{app_dir_error}; automatic mode does not use the System32 DirectML.dll \
             (set {DIRECTML_ENV}=1 to allow it)"
        ));
    }
    system_copy().map_err(|system_error| format!("{app_dir_error}; {system_error}"))
}

/// The usable `DirectML.dll` for `switch` (see [`choose_directml_library`]).
#[cfg(windows)]
pub fn directml_library_check(switch: DirectMlSwitch) -> Result<String, String> {
    choose_directml_library(switch, directml_app_dir_copy, directml_system_copy)
}

#[cfg(not(windows))]
pub fn directml_library_check(_switch: DirectMlSwitch) -> Result<String, String> {
    Err("DirectML needs Windows".to_string())
}

/// `DirectML.dll` in the executable's directory, checked.
#[cfg(windows)]
pub fn directml_app_dir_copy() -> Result<String, String> {
    use windows::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };
    let exe = std::env::current_exe().map_err(|e| format!("executable path unknown ({e})"))?;
    let path = exe
        .parent()
        .map(|dir| dir.join("DirectML.dll"))
        .ok_or_else(|| "the executable has no parent directory".to_string())?;
    if !path.is_file() {
        return Err(format!(
            "no DirectML.dll next to the executable ({})",
            path.display()
        ));
    }
    let wide: Vec<u16> = {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    };
    check_directml_module(
        &wide,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
    )
    .map(|()| format!("DirectML.dll next to the executable ({})", path.display()))
    .map_err(|e| {
        format!(
            "DirectML.dll next to the executable ({}) {e}",
            path.display()
        )
    })
}

/// The System32 `DirectML.dll`, checked.
#[cfg(windows)]
pub fn directml_system_copy() -> Result<String, String> {
    use windows::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_SYSTEM32;
    let wide: Vec<u16> = "DirectML.dll".encode_utf16().chain(Some(0)).collect();
    check_directml_module(&wide, LOAD_LIBRARY_SEARCH_SYSTEM32)
        .map(|()| "System32 DirectML.dll".to_string())
        .map_err(|e| format!("System32 DirectML.dll {e}"))
}

/// Load `library` (NUL-terminated UTF-16 name or path) with `flags`, look up
/// the export ONNX Runtime binds, and release it again.
#[cfg(windows)]
pub fn check_directml_module(
    library: &[u16],
    flags: windows::Win32::System::LibraryLoader::LOAD_LIBRARY_FLAGS,
) -> Result<(), String> {
    use windows::core::{s, PCSTR, PCWSTR};
    use windows::Win32::Foundation::{FreeLibrary, HANDLE};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW};

    debug_assert_eq!(
        library.last(),
        Some(&0),
        "library name must be NUL-terminated"
    );
    // SAFETY: `library` is a NUL-terminated UTF-16 string that outlives the
    // call. The module is only queried for two export addresses, which are
    // compared and never called, and is released before returning. An
    // ordinal is passed to GetProcAddress as a pointer-sized integer below
    // 0x10000, which is how the API takes ordinals.
    unsafe {
        let module = LoadLibraryExW(PCWSTR(library.as_ptr()), HANDLE::default(), flags)
            .map_err(|e| format!("could not be loaded ({e})"))?;
        let by_ordinal = GetProcAddress(
            module,
            PCSTR(DIRECTML_REQUIRED_ORDINAL as usize as *const u8),
        )
        .map(|f| f as usize);
        let by_name = GetProcAddress(module, s!("DMLCreateDevice1")).map(|f| f as usize);
        let _ = FreeLibrary(module);
        directml_exports_verdict(by_ordinal, by_name)
    }
}

/// The Microsoft Basic Render Driver (WARP) by its PCI ids, counted as a
/// software adapter even when DXGI does not flag it as one.
pub fn is_basic_render_driver(vendor_id: u32, device_id: u32) -> bool {
    vendor_id == 0x1414 && device_id == 0x8c
}

/// Enumerate DXGI adapters and run the Direct3D 12 checks on each.
#[cfg(windows)]
pub fn probe_adapters() -> Result<Vec<AdapterInfo>, String> {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
    use windows::Win32::Graphics::Direct3D12::{
        D3D12CreateDevice, ID3D12Device, D3D12_FEATURE_ARCHITECTURE,
        D3D12_FEATURE_DATA_ARCHITECTURE,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_REMOTE, DXGI_ADAPTER_FLAG_SOFTWARE,
        DXGI_ERROR_NOT_FOUND,
    };

    // SAFETY: plain COM calls on objects this function owns; every out
    // pointer is a local of the right type and size.
    unsafe {
        let factory: IDXGIFactory1 =
            CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
        let mut adapters = Vec::new();
        for ordinal in 0u32.. {
            let adapter = match factory.EnumAdapters1(ordinal) {
                Ok(adapter) => adapter,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(format!("EnumAdapters1({ordinal}): {e}")),
            };
            let desc = adapter
                .GetDesc1()
                .map_err(|e| format!("GetDesc1 for adapter ordinal {ordinal}: {e}"))?;
            let name_len = desc
                .Description
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(desc.Description.len());
            let name = String::from_utf16_lossy(&desc.Description[..name_len]);
            let flags = desc.Flags;
            let software = flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0
                || is_basic_render_driver(desc.VendorId, desc.DeviceId);
            let remote = flags & (DXGI_ADAPTER_FLAG_REMOTE.0 as u32) != 0;

            let d3d12 = if software || remote {
                D3d12Check::Failed("not checked".to_string())
            } else {
                let mut device: Option<ID3D12Device> = None;
                match D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device) {
                    Err(e) => D3d12Check::Failed(format!(
                        "D3D12CreateDevice at feature level {MIN_FEATURE_LEVEL} failed (HRESULT {:#010x})",
                        e.code().0 as u32
                    )),
                    Ok(()) => match device {
                        None => D3d12Check::Failed(
                            "D3D12CreateDevice succeeded without returning a device".to_string(),
                        ),
                        Some(device) => {
                            let mut arch = D3D12_FEATURE_DATA_ARCHITECTURE {
                                NodeIndex: 0,
                                TileBasedRenderer: BOOL(0),
                                UMA: BOOL(0),
                                CacheCoherentUMA: BOOL(0),
                            };
                            match device.CheckFeatureSupport(
                                D3D12_FEATURE_ARCHITECTURE,
                                &mut arch as *mut _ as *mut core::ffi::c_void,
                                std::mem::size_of::<D3D12_FEATURE_DATA_ARCHITECTURE>() as u32,
                            ) {
                                Ok(()) => D3d12Check::Capable {
                                    uma: arch.UMA.as_bool(),
                                },
                                Err(e) => D3d12Check::Failed(format!(
                                    "capability check failed (HRESULT {:#010x})",
                                    e.code().0 as u32
                                )),
                            }
                        }
                    },
                }
            };
            adapters.push(AdapterInfo {
                ordinal,
                name,
                vendor_id: desc.VendorId,
                dedicated_video_memory: desc.DedicatedVideoMemory as u64,
                software,
                remote,
                d3d12,
            });
        }
        Ok(adapters)
    }
}

#[cfg(not(windows))]
pub fn probe_adapters() -> Result<Vec<AdapterInfo>, String> {
    Err("DirectML needs Windows".to_string())
}

// ---------------------------------------------------------------------------
// Recovering runner
// ---------------------------------------------------------------------------

/// Loads a model on a target (DirectML or CPU sessions).
pub type LoadFn<M> = Box<dyn Fn(OnnxTarget) -> anyhow::Result<M> + Send>;
/// Runs one inference.
pub type RunFn<M> = fn(&mut M, &[f32], u32) -> anyhow::Result<String>;

/// A model plus the DirectML policy: recycling, CPU fallback on provider
/// init failure, and one CPU retry after a failed DirectML inference.
pub struct DirectMlRuntime<M> {
    label: &'static str,
    model: Option<M>,
    target: OnnxTarget,
    dml_inferences: u32,
    recycle_every: u32,
    load: LoadFn<M>,
    run: RunFn<M>,
    latch: &'static AtomicBool,
}

impl<M> DirectMlRuntime<M> {
    /// Load the model on `target`. When DirectML sessions cannot be built
    /// the model is loaded on CPU instead. `is_missing_model` recognises the
    /// "weights not downloaded yet" error, which is returned as is so the
    /// caller can start the download.
    pub fn start(
        label: &'static str,
        target: OnnxTarget,
        recycle_every: u32,
        load: LoadFn<M>,
        run: RunFn<M>,
        is_missing_model: fn(&anyhow::Error) -> bool,
        latch: &'static AtomicBool,
    ) -> anyhow::Result<Self> {
        let (model, target) = match target {
            OnnxTarget::Cpu => (load(OnnxTarget::Cpu)?, OnnxTarget::Cpu),
            dml @ OnnxTarget::DirectMl { .. } => match load(dml) {
                Ok(model) => (model, dml),
                Err(e) if is_missing_model(&e) => return Err(e),
                Err(e) => {
                    warn!("{label}: DirectML init failed ({e:#}); falling back to CPU");
                    latch.store(true, Ordering::SeqCst);
                    (load(OnnxTarget::Cpu)?, OnnxTarget::Cpu)
                }
            },
        };
        Ok(Self {
            label,
            model: Some(model),
            target,
            dml_inferences: 0,
            recycle_every,
            load,
            run,
            latch,
        })
    }

    pub fn target(&self) -> OnnxTarget {
        self.target
    }

    /// Release everything and continue on CPU for good.
    fn switch_to_cpu(&mut self) {
        self.model = None;
        self.target = OnnxTarget::Cpu;
        self.dml_inferences = 0;
        self.latch.store(true, Ordering::SeqCst);
    }

    fn recycle(&mut self) {
        info!(
            "{}: recycling DirectML sessions after {} inferences to prevent slowdown",
            self.label, self.dml_inferences
        );
        // Release first: the point is to give the device memory back.
        self.model = None;
        self.dml_inferences = 0;
        match (self.load)(self.target) {
            Ok(model) => self.model = Some(model),
            Err(e) => {
                warn!(
                    "{}: DirectML session recycle failed ({e:#}); falling back to CPU",
                    self.label
                );
                self.switch_to_cpu();
            }
        }
    }

    fn ensure_model(&mut self) -> anyhow::Result<&mut M> {
        if self.model.is_none() {
            self.model = Some((self.load)(self.target)?);
        }
        Ok(self.model.as_mut().expect("model loaded above"))
    }

    /// Transcribe `audio`. Under DirectML a failure is retried once on CPU
    /// sessions with the same audio.
    pub fn transcribe(&mut self, audio: &[f32], sample_rate: u32) -> anyhow::Result<String> {
        if self.target.is_directml()
            && self.recycle_every > 0
            && self.dml_inferences >= self.recycle_every
        {
            self.recycle();
        }
        let on_dml = self.target.is_directml();
        let run = self.run;
        let first = run(self.ensure_model()?, audio, sample_rate);
        let err = match first {
            Ok(text) => {
                if on_dml {
                    self.dml_inferences += 1;
                }
                return Ok(text);
            }
            Err(e) if !on_dml => return Err(e),
            Err(e) => e,
        };
        warn!(
            "{}: DirectML inference failed ({err:#}); releasing the DirectML sessions and retrying on CPU",
            self.label
        );
        self.switch_to_cpu();
        let model = match self.ensure_model() {
            Ok(model) => model,
            Err(load_err) => {
                error!(
                    "{}: CPU sessions could not be loaded after the DirectML failure: {load_err:#}",
                    self.label
                );
                return Err(err);
            }
        };
        match run(model, audio, sample_rate) {
            Ok(text) => {
                info!(
                    "{}: CPU inference completed after recovering from a DirectML failure",
                    self.label
                );
                Ok(text)
            }
            Err(retry_err) => Err(retry_err.context(format!(
                "CPU retry after a DirectML failure also failed (DirectML error: {err})"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Parakeet on audiopipe
// ---------------------------------------------------------------------------

#[cfg(feature = "parakeet")]
mod parakeet_model {
    use super::*;

    pub type ParakeetRuntime = DirectMlRuntime<audiopipe::Model>;

    fn accelerator(target: OnnxTarget) -> audiopipe::Accelerator {
        match target {
            OnnxTarget::Cpu => audiopipe::Accelerator::Cpu,
            OnnxTarget::DirectMl { device_id } => audiopipe::Accelerator::DirectMl { device_id },
        }
    }

    fn run(
        model: &mut audiopipe::Model,
        audio: &[f32],
        sample_rate: u32,
    ) -> anyhow::Result<String> {
        model
            .transcribe_with_sample_rate(
                audio,
                sample_rate,
                audiopipe::TranscribeOptions::default(),
            )
            .map(|r| r.text)
            .map_err(|e| anyhow::Error::new(e))
    }

    /// The weights are not in the Hugging Face cache yet.
    pub fn is_model_not_cached(err: &anyhow::Error) -> bool {
        err.chain().any(|cause| {
            cause
                .downcast_ref::<audiopipe::Error>()
                .is_some_and(|e| e.is_model_not_cached())
        })
    }

    /// Decide the target for this process, log it, and load `model_name`
    /// from the local cache (blocking).
    pub fn load_parakeet(model_name: &'static str) -> anyhow::Result<ParakeetRuntime> {
        let decision = decide_target_for_this_process();
        info!("parakeet: {}", decision.reason);
        DirectMlRuntime::start(
            "parakeet",
            decision.target,
            DIRECTML_RECYCLE_EVERY,
            Box::new(move |target| {
                audiopipe::Model::from_pretrained_cache_only_with(model_name, accelerator(target))
                    .map_err(anyhow::Error::new)
            }),
            run,
            is_model_not_cached,
            directml_latch(),
        )
    }
}

#[cfg(feature = "parakeet")]
pub use parakeet_model::{is_model_not_cached, load_parakeet, ParakeetRuntime};

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    fn adapter(ordinal: u32, name: &str, mib: u64, check: D3d12Check) -> AdapterInfo {
        AdapterInfo {
            ordinal,
            name: name.to_string(),
            vendor_id: 0x10de,
            dedicated_video_memory: mib * 1024 * 1024,
            software: false,
            remote: false,
            d3d12: check,
        }
    }

    fn capable(uma: bool) -> D3d12Check {
        D3d12Check::Capable { uma }
    }

    #[test]
    fn env_switch_semantics() {
        assert_eq!(parse_directml_switch(None), DirectMlSwitch::Auto);
        assert_eq!(parse_directml_switch(Some("")), DirectMlSwitch::Auto);
        assert_eq!(parse_directml_switch(Some("  ")), DirectMlSwitch::Auto);
        assert_eq!(parse_directml_switch(Some("1")), DirectMlSwitch::On);
        assert_eq!(parse_directml_switch(Some("TRUE")), DirectMlSwitch::On);
        assert_eq!(parse_directml_switch(Some(" true ")), DirectMlSwitch::On);
        for off in ["0", "false", "off", "no", "2", "yes"] {
            assert_eq!(
                parse_directml_switch(Some(off)),
                DirectMlSwitch::Off,
                "{off}"
            );
        }
    }

    #[test]
    fn env_name_matches_the_shared_constant() {
        assert_eq!(DIRECTML_ENV, screenpipe_core::env::SCREENPIPE_DIRECTML);
    }

    #[test]
    fn software_remote_and_failed_adapters_are_skipped() {
        let mut warp = adapter(0, "Microsoft Basic Render Driver", 0, capable(true));
        warp.software = true;
        let mut rdp = adapter(1, "Remote Display Adapter", 0, capable(false));
        rdp.remote = true;
        let broken = adapter(
            2,
            "Old GPU",
            2048,
            D3d12Check::Failed("D3D12CreateDevice failed".into()),
        );
        assert_eq!(
            select_adapter(&[warp.clone(), rdp.clone(), broken.clone()]),
            None
        );
        let good = adapter(3, "Intel Iris Xe", 128, capable(true));
        assert_eq!(
            select_adapter(&[warp, rdp, broken, good.clone()]),
            Some(&good)
        );
    }

    #[test]
    fn basic_render_driver_is_recognised_by_its_ids() {
        assert!(is_basic_render_driver(0x1414, 0x8c));
        // Other Microsoft adapters (for example Hyper-V video) and real GPUs.
        assert!(!is_basic_render_driver(0x1414, 0x5353));
        assert!(!is_basic_render_driver(0x10de, 0x8c));
    }

    #[test]
    fn discrete_and_larger_adapters_win() {
        let igpu = adapter(0, "Intel UHD", 128, capable(true));
        let small = adapter(1, "GTX 1050", 2048, capable(false));
        let big = adapter(2, "RTX 4070", 12288, capable(false));
        let twin = adapter(3, "RTX 4070", 12288, capable(false));
        let all = [igpu.clone(), small, big.clone(), twin];
        assert_eq!(
            select_adapter(&all).unwrap().ordinal,
            2,
            "most VRAM, lowest ordinal"
        );
        assert_eq!(select_adapter(&[igpu.clone()]), Some(&igpu));
    }

    fn lib_ok() -> Result<String, String> {
        Ok("DirectML.dll next to the executable (C:\\app\\DirectML.dll)".to_string())
    }

    fn found(what: &str) -> impl FnOnce() -> Result<String, String> + '_ {
        move || Ok(what.to_string())
    }

    fn missing(what: &str) -> impl FnOnce() -> Result<String, String> + '_ {
        move || Err(what.to_string())
    }

    #[test]
    fn automatic_mode_needs_a_directml_dll_next_to_the_executable() {
        let system32_not_consulted = || -> Result<String, String> {
            panic!("the app copy wins without looking at System32")
        };
        assert_eq!(
            choose_directml_library(
                DirectMlSwitch::Auto,
                found("app copy"),
                system32_not_consulted
            ),
            Ok("app copy".to_string())
        );
        let err = choose_directml_library(
            DirectMlSwitch::Auto,
            missing("no DirectML.dll next to the executable"),
            || panic!("automatic mode never accepts the System32 copy"),
        )
        .unwrap_err();
        assert!(
            err.contains("no DirectML.dll next to the executable"),
            "{err}"
        );
        assert!(err.contains("SCREENPIPE_DIRECTML=1"), "{err}");
    }

    #[test]
    fn switch_on_also_accepts_the_system32_copy() {
        assert_eq!(
            choose_directml_library(
                DirectMlSwitch::On,
                missing("no DirectML.dll next to the executable"),
                found("System32 DirectML.dll"),
            ),
            Ok("System32 DirectML.dll".to_string())
        );
        let err = choose_directml_library(
            DirectMlSwitch::On,
            missing("no DirectML.dll next to the executable"),
            missing("System32 DirectML.dll has no DMLCreateDevice1 export"),
        )
        .unwrap_err();
        assert!(err.contains("next to the executable"), "{err}");
        assert!(err.contains("has no DMLCreateDevice1 export"), "{err}");
    }

    #[test]
    fn the_imported_ordinal_must_be_dmlcreatedevice1() {
        assert_eq!(directml_exports_verdict(Some(0x1000), Some(0x1000)), Ok(()));
        let old = directml_exports_verdict(Some(0x1000), None).unwrap_err();
        assert!(old.contains("has no DMLCreateDevice1 export"), "{old}");
        let no_ordinal = directml_exports_verdict(None, Some(0x1000)).unwrap_err();
        assert!(no_ordinal.contains("ordinal 2"), "{no_ordinal}");
        let other = directml_exports_verdict(Some(0x2000), Some(0x1000)).unwrap_err();
        assert!(
            other.contains("something other than DMLCreateDevice1"),
            "{other}"
        );
    }

    #[test]
    fn automatic_mode_without_an_app_copy_decides_cpu() {
        let decision = decide_target(
            DirectMlSwitch::Auto,
            true,
            false,
            || {
                choose_directml_library(
                    DirectMlSwitch::Auto,
                    missing("no DirectML.dll next to the executable"),
                    || panic!("not consulted in automatic mode"),
                )
            },
            || panic!("no adapter probe without a usable DirectML.dll"),
        );
        assert_eq!(decision.target, OnnxTarget::Cpu);
        assert!(
            decision.reason.ends_with("; using CPU"),
            "{}",
            decision.reason
        );
    }

    /// Runs the real DXGI / Direct3D 12 probe and the DLL checks (CI's
    /// windows job; hosted runners expose the Microsoft Basic Render Driver).
    #[cfg(windows)]
    #[test]
    fn windows_probe_runs_and_never_selects_a_software_adapter() {
        let adapters = probe_adapters().expect("DXGI adapter enumeration");
        for adapter in &adapters {
            eprintln!(
                "adapter {}: {} software={} remote={} {:?}",
                adapter.ordinal, adapter.name, adapter.software, adapter.remote, adapter.d3d12
            );
            if adapter.software || adapter.name.contains("Microsoft Basic Render Driver") {
                assert!(adapter.rejection().is_some(), "{adapter:?}");
            }
        }
        if let Some(chosen) = select_adapter(&adapters) {
            assert!(!chosen.software && !chosen.remote, "{chosen:?}");
            assert!(
                matches!(chosen.d3d12, D3d12Check::Capable { .. }),
                "{chosen:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_directml_dll_checks_run() {
        use windows::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_SYSTEM32;
        // A system DLL without DirectML's exports is refused by the export check.
        let kernel32: Vec<u16> = "kernel32.dll".encode_utf16().chain(Some(0)).collect();
        let err = check_directml_module(&kernel32, LOAD_LIBRARY_SEARCH_SYSTEM32).unwrap_err();
        assert!(err.contains("has no DMLCreateDevice1 export"), "{err}");
        // The test binary has no DirectML.dll beside it.
        let exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        if !exe_dir.join("DirectML.dll").exists() {
            let err = directml_app_dir_copy().unwrap_err();
            assert!(
                err.contains("no DirectML.dll next to the executable"),
                "{err}"
            );
            assert!(directml_library_check(DirectMlSwitch::Auto).is_err());
        }
        // Whatever the runner's System32 holds, the check answers without faulting.
        eprintln!("System32 DirectML.dll: {:?}", directml_system_copy());
        eprintln!(
            "switch on: {:?}",
            directml_library_check(DirectMlSwitch::On)
        );
    }

    #[test]
    fn missing_directml_library_means_cpu_without_probing() {
        let decision = decide_target(
            DirectMlSwitch::On,
            true,
            false,
            || Err("DirectML.dll could not be loaded (not found)".to_string()),
            || panic!("no adapter probe without DirectML.dll"),
        );
        assert_eq!(decision.target, OnnxTarget::Cpu);
        assert!(decision.reason.contains("DirectML.dll could not be loaded"));
    }

    #[test]
    fn disabled_switch_forces_cpu_without_probing() {
        let decision = decide_target(DirectMlSwitch::Off, true, false, lib_ok, || {
            panic!("no probe when SCREENPIPE_DIRECTML disables DirectML")
        });
        assert_eq!(decision.target, OnnxTarget::Cpu);
        assert!(decision
            .reason
            .contains("SCREENPIPE_DIRECTML explicitly disabled"));
    }

    #[test]
    fn decision_paths() {
        let gpu = || Ok(vec![adapter(1, "Radeon RX 7600", 8192, capable(false))]);
        let auto = decide_target(DirectMlSwitch::Auto, true, false, lib_ok, gpu);
        assert_eq!(auto.target, OnnxTarget::DirectMl { device_id: 1 });
        assert!(auto.reason.contains("ordinal 1"));
        assert!(auto.reason.contains("8192 MiB"));
        assert!(auto.reason.contains("UMA=false"));

        let on = decide_target(DirectMlSwitch::On, true, false, lib_ok, gpu);
        assert_eq!(on.target, OnnxTarget::DirectMl { device_id: 1 });

        let not_built = decide_target(DirectMlSwitch::On, false, false, lib_ok, gpu);
        assert_eq!(not_built.target, OnnxTarget::Cpu);

        let latched = decide_target(DirectMlSwitch::Auto, true, true, lib_ok, gpu);
        assert_eq!(latched.target, OnnxTarget::Cpu);

        let enum_failed = decide_target(DirectMlSwitch::Auto, true, false, lib_ok, || {
            Err("CreateDXGIFactory1: E_FAIL".to_string())
        });
        assert_eq!(enum_failed.target, OnnxTarget::Cpu);
        assert!(enum_failed
            .reason
            .contains("DXGI adapter enumeration failed"));

        let none = decide_target(DirectMlSwitch::Auto, true, false, lib_ok, || Ok(Vec::new()));
        assert_eq!(none.target, OnnxTarget::Cpu);
        assert!(none.reason.contains("no hardware adapter"));
    }

    #[cfg(feature = "parakeet")]
    #[test]
    fn missing_parakeet_weights_are_recognised() {
        let err = anyhow::Error::new(audiopipe::Error::ModelNotCached("parakeet".into()))
            .context("loading parakeet");
        assert!(is_model_not_cached(&err));
        assert!(!is_model_not_cached(&anyhow!("some other failure")));
    }

    #[test]
    fn this_process_decision_on_linux_is_cpu() {
        if cfg!(not(windows)) {
            assert_eq!(decide_target_for_this_process().target, OnnxTarget::Cpu);
        }
    }

    // --- runner ---------------------------------------------------------

    /// Fake model: remembers its target; DirectML inference fails when
    /// `dml_fails` is set.
    struct FakeModel {
        target: OnnxTarget,
        dml_fails: bool,
    }

    fn fake_run(model: &mut FakeModel, audio: &[f32], _sr: u32) -> anyhow::Result<String> {
        if model.target.is_directml() && model.dml_fails {
            Err(anyhow!("DML device removed"))
        } else {
            Ok(format!("{} samples on {:?}", audio.len(), model.target))
        }
    }

    fn not_cached(err: &anyhow::Error) -> bool {
        err.to_string().contains("not cached")
    }

    type Loads = std::sync::Arc<std::sync::Mutex<Vec<OnnxTarget>>>;

    fn loader(loads: Loads, dml_init_fails: bool, dml_fails: bool) -> LoadFn<FakeModel> {
        Box::new(move |target| {
            loads.lock().unwrap().push(target);
            if target.is_directml() && dml_init_fails {
                return Err(anyhow!("DmlExecutionProvider registration failed"));
            }
            Ok(FakeModel { target, dml_fails })
        })
    }

    const DML: OnnxTarget = OnnxTarget::DirectMl { device_id: 0 };

    #[test]
    fn provider_init_failure_loads_cpu_sessions() {
        static LATCH: AtomicBool = AtomicBool::new(false);
        let loads = Loads::default();
        let mut rt = DirectMlRuntime::start(
            "test",
            DML,
            500,
            loader(loads.clone(), true, false),
            fake_run,
            not_cached,
            &LATCH,
        )
        .unwrap();
        assert_eq!(rt.target(), OnnxTarget::Cpu);
        assert_eq!(*loads.lock().unwrap(), vec![DML, OnnxTarget::Cpu]);
        assert!(LATCH.load(Ordering::SeqCst));
        assert!(rt.transcribe(&[0.0; 4], 16_000).unwrap().contains("Cpu"));
    }

    #[test]
    fn missing_weights_are_reported_not_hidden_by_a_cpu_attempt() {
        static LATCH: AtomicBool = AtomicBool::new(false);
        let loads = Loads::default();
        let loads_in = loads.clone();
        let err = DirectMlRuntime::<FakeModel>::start(
            "test",
            DML,
            500,
            Box::new(move |t| {
                loads_in.lock().unwrap().push(t);
                Err(anyhow!("model not cached yet"))
            }),
            fake_run,
            not_cached,
            &LATCH,
        )
        .err()
        .unwrap();
        assert!(not_cached(&err));
        assert_eq!(*loads.lock().unwrap(), vec![DML]);
        assert!(!LATCH.load(Ordering::SeqCst));
    }

    #[test]
    fn dml_inference_failure_retries_the_same_audio_on_cpu_once() {
        static LATCH: AtomicBool = AtomicBool::new(false);
        let loads = Loads::default();
        let mut rt = DirectMlRuntime::start(
            "test",
            DML,
            500,
            loader(loads.clone(), false, true),
            fake_run,
            not_cached,
            &LATCH,
        )
        .unwrap();
        assert_eq!(rt.target(), DML);
        let text = rt.transcribe(&[0.0; 7], 16_000).unwrap();
        assert_eq!(text, "7 samples on Cpu");
        assert_eq!(rt.target(), OnnxTarget::Cpu);
        assert_eq!(*loads.lock().unwrap(), vec![DML, OnnxTarget::Cpu]);
        assert!(
            LATCH.load(Ordering::SeqCst),
            "DirectML stays off afterwards"
        );
        // Later calls stay on CPU without reloading.
        rt.transcribe(&[0.0; 3], 16_000).unwrap();
        assert_eq!(loads.lock().unwrap().len(), 2);
    }

    #[test]
    fn cpu_failures_are_returned_without_retry() {
        static LATCH: AtomicBool = AtomicBool::new(false);
        fn failing(_: &mut FakeModel, _: &[f32], _: u32) -> anyhow::Result<String> {
            Err(anyhow!("bad audio"))
        }
        let loads = Loads::default();
        let mut rt = DirectMlRuntime::start(
            "test",
            OnnxTarget::Cpu,
            500,
            loader(loads.clone(), false, false),
            failing,
            not_cached,
            &LATCH,
        )
        .unwrap();
        assert!(rt.transcribe(&[0.0], 16_000).is_err());
        assert_eq!(loads.lock().unwrap().len(), 1);
        assert!(!LATCH.load(Ordering::SeqCst));
    }

    #[test]
    fn dml_sessions_are_recycled_every_n_inferences() {
        static LATCH: AtomicBool = AtomicBool::new(false);
        let loads = Loads::default();
        let mut rt = DirectMlRuntime::start(
            "test",
            DML,
            3,
            loader(loads.clone(), false, false),
            fake_run,
            not_cached,
            &LATCH,
        )
        .unwrap();
        for _ in 0..7 {
            rt.transcribe(&[0.0; 2], 16_000).unwrap();
        }
        // Initial load, then a rebuild before the 4th and the 7th call.
        assert_eq!(*loads.lock().unwrap(), vec![DML, DML, DML]);
        assert_eq!(rt.target(), DML);
        assert!(!LATCH.load(Ordering::SeqCst));
    }

    #[test]
    fn a_failed_recycle_moves_to_cpu() {
        static LATCH: AtomicBool = AtomicBool::new(false);
        let loads = Loads::default();
        let loads_in = loads.clone();
        let mut rt = DirectMlRuntime::start(
            "test",
            DML,
            1,
            Box::new(move |target| {
                let mut seen = loads_in.lock().unwrap();
                seen.push(target);
                if target.is_directml() && seen.len() > 1 {
                    return Err(anyhow!("out of video memory"));
                }
                Ok(FakeModel {
                    target,
                    dml_fails: false,
                })
            }),
            fake_run,
            not_cached,
            &LATCH,
        )
        .unwrap();
        rt.transcribe(&[0.0], 16_000).unwrap();
        let text = rt.transcribe(&[0.0], 16_000).unwrap();
        assert!(text.contains("Cpu"), "{text}");
        assert_eq!(*loads.lock().unwrap(), vec![DML, DML, OnnxTarget::Cpu]);
        assert!(LATCH.load(Ordering::SeqCst));
    }

    #[test]
    fn cpu_load_failure_after_dml_failure_returns_the_dml_error() {
        static LATCH: AtomicBool = AtomicBool::new(false);
        let mut rt = DirectMlRuntime::start(
            "test",
            DML,
            500,
            Box::new(|target| {
                if target.is_directml() {
                    Ok(FakeModel {
                        target,
                        dml_fails: true,
                    })
                } else {
                    Err(anyhow!("cpu sessions failed"))
                }
            }),
            fake_run,
            not_cached,
            &LATCH,
        )
        .unwrap();
        let err = rt.transcribe(&[0.0], 16_000).unwrap_err();
        assert!(err.to_string().contains("DML device removed"), "{err}");
    }
}
