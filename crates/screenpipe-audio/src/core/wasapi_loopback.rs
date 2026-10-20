// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Event-driven WASAPI loopback engine for Windows output capture.
//!
//! Replaces the cpal loopback for output devices. Each stream runs one
//! capture thread (COM multithreaded apartment, MMCSS "Audio" class) that
//! waits on the client's event handle, drains packets with
//! `GetNextPacketSize` / `GetBuffer` / `ReleaseBuffer`, converts them to mono
//! `f32` at the stream's rate and feeds the `AudioStream` broadcast, so
//! chunking, VAD and live taps downstream are unchanged.
//!
//! What a stream captures is a [`LoopbackTarget`] (see
//! `process_tap/targets.rs` for the rules):
//!
//! * The output device that is the current default becomes the
//!   *system-audio stream*. It follows the default render endpoint (after a
//!   short grace, see [`DefaultEndpointFollow`]) and honours
//!   `audio-exclusions.json`: when an excluded executable runs, the stream
//!   moves to `exclude-process-tree:<root pid>` for the first running one.
//! * Any other output device is captured as that endpoint only.
//! * [`spawn_loopback_capture`] with [`CaptureRequest::ProcessTree`] gives a
//!   per-process include tap (for meeting capture).
//!
//! Every timeout and failure keeps audio flowing: a per-process activation
//! that fails or times out falls back to the plain default-endpoint loopback
//! (each untappable root is logged once), a broken client is rebuilt with
//! backoff, and a stream that cannot start in time makes the caller fall back
//! to the cpal loopback. After repeated failures on one device the engine
//! steps aside for that device for the rest of the session.

use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use windows::core::{implement, Interface, IUnknown, HRESULT, PCWSTR, PROPVARIANT};
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{
    CloseHandle, BOOL, HANDLE, RPC_E_CHANGED_MODE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Media::Audio::{
    eConsole, eRender, ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, DEVICE_STATE_ACTIVE,
    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, OpenProcess,
    WaitForSingleObject, PROCESS_SYNCHRONIZE,
};
use windows::Win32::System::Variant::VT_BLOB;

use super::process_tap::exclusions::{self, ExclusionFile};
use super::process_tap::pcm::{
    FormatDescription, LinearResampler, SampleLayout, FORMAT_TAG_EXTENSIBLE, FORMAT_TAG_PCM,
};
use super::process_tap::targets::{
    choose_exclusion, desired_system_target, initial_include_target, resolve_include_root,
    CaptureRole, DefaultEndpointFollow, ExclusionChoice, FailureWindow, IncludeRoot,
    LoopbackTarget, ProcessEntry, ProcessLoopbackSupport, ProcessSnapshot, TargetDebounce,
    UntappableRoots,
};
use super::stream::AudioStreamConfig;

/// Label used in logs for streams this engine opens.
pub const ENGINE_LABEL: &str = "wasapi-local";

/// How long `ActivateAudioInterfaceAsync` may take before the per-process
/// target is given up for the plain loopback.
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(4);
/// How long a caller waits for the capture thread to report a running
/// client. Covers one activation timeout plus the plain fallback.
const START_TIMEOUT: Duration = Duration::from_secs(8);
/// Longest wait on the capture event before the loop looks at its timers.
const EVENT_WAIT_MS: u32 = 100;
/// Requested shared-mode buffer, in 100 ns units (200 ms).
const BUFFER_DURATION_HNS: i64 = 2_000_000;
/// How often the default render endpoint is checked.
const DEFAULT_POLL: Duration = Duration::from_secs(1);
/// How often the exclusion file and process list are checked.
const EXCLUSION_POLL: Duration = Duration::from_secs(2);
/// Wait after a default-endpoint change before the stream moves itself (the
/// device monitor usually replaces it first when following the default).
const DEFAULT_SWITCH_GRACE: Duration = Duration::from_secs(4);
/// Minimum spacing of exclusion-driven rebuilds.
const EXCLUSION_MIN_INTERVAL: Duration = Duration::from_secs(5);
/// Consecutive failed rebuilds of a broken client before the stream is
/// handed back to the device monitor.
const MAX_REBUILD_FAILURES: u32 = 5;
/// Backoff between rebuild attempts (doubles, capped at 8 s).
const REBUILD_RETRY_BASE: Duration = Duration::from_millis(500);
/// Rate used for a per-process client when no stream rate is known yet.
const PROCESS_LOOPBACK_DEFAULT_RATE: u32 = 48_000;
/// Engine failures per device in [`FAILURE_WINDOW`] before the engine steps
/// aside for that device.
const MAX_ENGINE_FAILURES: usize = 3;
const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);

/// What the caller wants captured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureRequest {
    /// An output device by its friendly name (`"default"` means the default
    /// output). The engine decides on the capture thread whether this is the
    /// system-audio stream or a pinned endpoint.
    OutputDevice { name: String },
    /// A per-process include tap over these processes.
    ProcessTree { pids: Vec<u32> },
}

// ---------------------------------------------------------------------------
// Windows build and per-process support
// ---------------------------------------------------------------------------

static WINDOWS_BUILD: OnceLock<Option<u32>> = OnceLock::new();

/// The Windows build number from `RtlGetVersion` (not subject to the
/// compatibility shims of `GetVersionEx`). Cached.
pub fn windows_build_number() -> Option<u32> {
    *WINDOWS_BUILD.get_or_init(|| {
        // SAFETY: RtlGetVersion fills a caller-owned struct whose size field
        // is set; it has no other preconditions.
        let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
        info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
        let status = unsafe { RtlGetVersion(&mut info) };
        if status.0 >= 0 && info.dwBuildNumber > 0 {
            Some(info.dwBuildNumber)
        } else {
            None
        }
    })
}

/// Whether per-process loopback works on this build. Logged once.
pub fn process_loopback_support() -> ProcessLoopbackSupport {
    static LOGGED: OnceLock<()> = OnceLock::new();
    let support = ProcessLoopbackSupport::from_build(windows_build_number());
    LOGGED.get_or_init(|| match support {
        ProcessLoopbackSupport::Available { .. } => info!("{}", support.describe()),
        _ => warn!("{}", support.describe()),
    });
    support
}

// ---------------------------------------------------------------------------
// Session-wide failure bookkeeping
// ---------------------------------------------------------------------------

fn failure_registry() -> &'static Mutex<FailureWindow> {
    static REGISTRY: OnceLock<Mutex<FailureWindow>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(FailureWindow::new(FAILURE_WINDOW, MAX_ENGINE_FAILURES)))
}

fn record_engine_failure(device: &str) {
    if let Ok(mut reg) = failure_registry().lock() {
        reg.record(device, Instant::now());
    }
}

/// Whether the engine should open `device`. After repeated failures on a
/// device within the window, the cpal loopback is used for it instead.
pub fn engine_allowed_for(device: &str) -> bool {
    static LOGGED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let exhausted = failure_registry()
        .lock()
        .map(|reg| reg.is_exhausted(device, Instant::now()))
        .unwrap_or(false);
    if exhausted {
        let logged = LOGGED.get_or_init(|| Mutex::new(HashSet::new()));
        if logged
            .lock()
            .map(|mut set| set.insert(device.to_string()))
            .unwrap_or(false)
        {
            warn!(
                "{ENGINE_LABEL}: loopback engine failed {MAX_ENGINE_FAILURES} times for '{device}'; \
                 using the cpal loopback for it from now on"
            );
        }
    }
    !exhausted
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Starts a capture thread for `request` and waits until its first client
/// runs. Returns the stream config (rate of the first client; later clients
/// are resampled to it) and the thread handle. On error or start timeout the
/// caller falls back to another backend; the thread never leaves a client
/// running in that case.
pub async fn spawn_loopback_capture(
    request: CaptureRequest,
    device_label: String,
    tx: broadcast::Sender<Vec<f32>>,
    is_disconnected: Arc<AtomicBool>,
) -> Result<(AudioStreamConfig, tokio::task::JoinHandle<()>)> {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let abandoned = Arc::new(AtomicBool::new(false));
    let ctx = ThreadContext {
        request: request.clone(),
        label: device_label.clone(),
        tx,
        is_disconnected,
        abandoned: abandoned.clone(),
    };
    let handle = tokio::task::spawn_blocking(move || run_capture_thread(ctx, started_tx));

    let failure_key = match &request {
        CaptureRequest::OutputDevice { name } => Some(name.clone()),
        CaptureRequest::ProcessTree { .. } => None,
    };
    let outcome = match tokio::time::timeout(START_TIMEOUT, started_rx).await {
        Ok(Ok(Ok(config))) => return Ok((config, handle)),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(_)) => Err(anyhow!(
            "{ENGINE_LABEL}: capture thread for {device_label} ended before it started"
        )),
        Err(_) => {
            abandoned.store(true, Ordering::Relaxed);
            Err(anyhow!(
                "{ENGINE_LABEL}: timed out after {}s starting loopback capture for {device_label}",
                START_TIMEOUT.as_secs()
            ))
        }
    };
    if let Some(key) = failure_key {
        record_engine_failure(&key);
    }
    outcome
}

struct ThreadContext {
    request: CaptureRequest,
    label: String,
    tx: broadcast::Sender<Vec<f32>>,
    is_disconnected: Arc<AtomicBool>,
    abandoned: Arc<AtomicBool>,
}

fn run_capture_thread(
    ctx: ThreadContext,
    started: tokio::sync::oneshot::Sender<Result<AudioStreamConfig>>,
) {
    let _com = match ComApartment::enter() {
        Ok(com) => com,
        Err(e) => {
            let _ = started.send(Err(e));
            return;
        }
    };
    let _mmcss = MmcssGuard::enter();

    let mut engine = match Engine::start(&ctx) {
        Ok(engine) => engine,
        Err(e) => {
            let _ = started.send(Err(e));
            return;
        }
    };
    if ctx.abandoned.load(Ordering::Relaxed) || started.send(Ok(engine.config())).is_err() {
        // The caller gave up waiting; dropping the engine stops the client.
        debug!(
            "{ENGINE_LABEL}: start for {} was abandoned by the caller",
            ctx.label
        );
        return;
    }
    info!(
        "{ENGINE_LABEL}: capturing {} as {} at {} Hz",
        ctx.label,
        engine.client.target,
        engine.output_rate
    );
    engine.run(&ctx);
    debug!("{ENGINE_LABEL}: capture thread for {} exited", ctx.label);
}

// ---------------------------------------------------------------------------
// RAII helpers
// ---------------------------------------------------------------------------

/// COM multithreaded apartment for the capture thread.
struct ComApartment {
    owns_init: bool,
}

impl ComApartment {
    fn enter() -> Result<Self> {
        // SAFETY: plain COM initialisation of the current thread, undone in Drop.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_ok() {
            Ok(Self { owns_init: true })
        } else if hr == RPC_E_CHANGED_MODE {
            // The thread already joined another apartment; COM is usable.
            Ok(Self { owns_init: false })
        } else {
            Err(anyhow!(
                "{ENGINE_LABEL}: COM initialisation failed for the loopback thread: {hr:?}"
            ))
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.owns_init {
            // SAFETY: balances the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
        }
    }
}

/// Raises the capture thread to the MMCSS "Audio" class while it runs.
struct MmcssGuard(Option<HANDLE>);

impl MmcssGuard {
    fn enter() -> Self {
        let mut task_index = 0u32;
        // SAFETY: valid task name and out pointer.
        match unsafe { AvSetMmThreadCharacteristicsW(windows::core::w!("Audio"), &mut task_index) }
        {
            Ok(handle) => Self(Some(handle)),
            Err(e) => {
                debug!("{ENGINE_LABEL}: MMCSS registration failed: {e}");
                Self(None)
            }
        }
    }
}

impl Drop for MmcssGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            // SAFETY: handle came from AvSetMmThreadCharacteristicsW.
            let _ = unsafe { AvRevertMmThreadCharacteristics(handle) };
        }
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: the handle is owned and closed once.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

/// Watches a process for exit through a SYNCHRONIZE handle.
struct ProcessWatch {
    pid: u32,
    handle: OwnedHandle,
}

impl ProcessWatch {
    fn open(pid: u32) -> Result<Self> {
        // SAFETY: OpenProcess with a plain access mask.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, BOOL(0), pid) }.map_err(|e| {
            anyhow!("{ENGINE_LABEL}: cannot open target pid {pid} to watch for its exit: {e}")
        })?;
        Ok(Self {
            pid,
            handle: OwnedHandle(handle),
        })
    }

    fn has_exited(&self) -> Result<bool> {
        // SAFETY: the handle stays open for the life of self.
        let wait = unsafe { WaitForSingleObject(self.handle.0, 0) };
        if wait == WAIT_OBJECT_0 {
            Ok(true)
        } else if wait == WAIT_TIMEOUT {
            Ok(false)
        } else {
            Err(anyhow!(
                "{ENGINE_LABEL}: waiting on target pid {} failed ({:?})",
                self.pid,
                wait
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Process list
// ---------------------------------------------------------------------------

/// Snapshot of running processes (pid, parent pid, executable name) from
/// the ToolHelp API, which needs no per-process handles.
pub fn snapshot_processes() -> Result<ProcessSnapshot> {
    // SAFETY: standard ToolHelp enumeration over an owned snapshot handle.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
            .map_err(|e| anyhow!("{ENGINE_LABEL}: process snapshot failed: {e}"))?;
        let snap = OwnedHandle(snap);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut rows = Vec::new();
        if Process32FirstW(snap.0, &mut entry).is_ok() {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                rows.push(ProcessEntry {
                    pid: entry.th32ProcessID,
                    parent_pid: entry.th32ParentProcessID,
                    exe_name: String::from_utf16_lossy(&entry.szExeFile[..len]),
                });
                if Process32NextW(snap.0, &mut entry).is_err() {
                    break;
                }
            }
        }
        Ok(ProcessSnapshot::new(rows))
    }
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

fn create_enumerator() -> Result<IMMDeviceEnumerator> {
    // SAFETY: COM is initialised on this thread (ComApartment).
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
        .map_err(|e| anyhow!("{ENGINE_LABEL}: creating MMDeviceEnumerator failed: {e}"))
}

fn default_render_endpoint(enumerator: &IMMDeviceEnumerator) -> Result<IMMDevice> {
    // SAFETY: valid enumerator.
    unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
        .map_err(|e| anyhow!("{ENGINE_LABEL}: getting the default render endpoint failed: {e}"))
}

fn endpoint_id(device: &IMMDevice) -> Option<String> {
    // SAFETY: GetId returns a CoTaskMem string that is freed here.
    unsafe {
        let id = device.GetId().ok()?;
        let text = id.to_string().ok();
        CoTaskMemFree(Some(id.as_ptr() as *const c_void));
        text
    }
}

fn friendly_name(device: &IMMDevice) -> Option<String> {
    // SAFETY: read-only property store access (STGM_READ = 0).
    unsafe {
        let store = device.OpenPropertyStore(STGM(0)).ok()?;
        let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let name = value.to_string();
        (!name.is_empty()).then_some(name)
    }
}

fn default_render_endpoint_id(enumerator: &IMMDeviceEnumerator) -> Option<String> {
    default_render_endpoint(enumerator)
        .ok()
        .and_then(|d| endpoint_id(&d))
}

fn find_render_endpoint(enumerator: &IMMDeviceEnumerator, name: &str) -> Result<IMMDevice> {
    // SAFETY: valid enumerator and collection.
    unsafe {
        let devices = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .map_err(|e| anyhow!("{ENGINE_LABEL}: listing render endpoints failed: {e}"))?;
        let count = devices.GetCount().unwrap_or(0);
        for i in 0..count {
            if let Ok(device) = devices.Item(i) {
                if friendly_name(&device).as_deref() == Some(name) {
                    return Ok(device);
                }
            }
        }
    }
    Err(anyhow!(
        "{ENGINE_LABEL}: output device '{name}' is not an active render endpoint"
    ))
}

/// Reads the fields of a WAVEFORMATEX (or WAVEFORMATEXTENSIBLE) the decoder
/// needs. The structs are packed, so they are copied out unaligned.
///
/// # Safety
/// `format` must point at a valid WAVEFORMATEX whose `cbSize` extra bytes
/// are readable.
unsafe fn describe_format(format: *const WAVEFORMATEX) -> FormatDescription {
    let base: WAVEFORMATEX = std::ptr::read_unaligned(format);
    let tag = base.wFormatTag;
    let extra = base.cbSize;
    let subformat_tag = if tag == FORMAT_TAG_EXTENSIBLE
        && extra as usize
            >= std::mem::size_of::<WAVEFORMATEXTENSIBLE>() - std::mem::size_of::<WAVEFORMATEX>()
    {
        let ext: WAVEFORMATEXTENSIBLE =
            std::ptr::read_unaligned(format as *const WAVEFORMATEXTENSIBLE);
        let sub = ext.SubFormat;
        Some(sub.data1)
    } else {
        None
    };
    FormatDescription {
        format_tag: tag,
        channels: base.nChannels,
        sample_rate: base.nSamplesPerSec,
        block_align: base.nBlockAlign,
        bits_per_sample: base.wBitsPerSample,
        subformat_tag,
    }
}

// ---------------------------------------------------------------------------
// Per-process activation
// ---------------------------------------------------------------------------

/// A PROPVARIANT holding a VT_BLOB, laid out by hand so no PROPVARIANT value
/// with a destructor ever owns the blob pointer (`PropVariantClear` would
/// try to free it).
#[repr(C)]
struct BlobPropVariant {
    vt: u16,
    reserved: [u16; 3],
    blob_size: u32,
    blob_data: *const c_void,
}

/// Activation parameters plus the blob that points at them. Boxed and owned
/// by the completion handler, so they outlive an activation the caller
/// stopped waiting for.
struct ActivationPayload {
    params: AUDIOCLIENT_ACTIVATION_PARAMS,
    blob: BlobPropVariant,
}

// SAFETY: the payload is written once before activation and only read by
// the audio service afterwards; the raw pointer targets the same box.
unsafe impl Send for ActivationPayload {}
unsafe impl Sync for ActivationPayload {}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ActivationHandler {
    done: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    _payload: Box<ActivationPayload>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationHandler_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Option<&IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        if let Ok(mut done) = self.done.lock() {
            if let Some(tx) = done.take() {
                let _ = tx.send(());
            }
        }
        Ok(())
    }
}

/// Activates an `IAudioClient` on the virtual process-loopback device for
/// `pid`'s process tree, in include or exclude mode.
fn activate_process_loopback(pid: u32, exclude: bool) -> Result<IAudioClient> {
    let mode = if exclude {
        PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE
    } else {
        PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
    };
    let mut payload = Box::new(ActivationPayload {
        params: AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: mode,
                },
            },
        },
        blob: BlobPropVariant {
            vt: VT_BLOB.0,
            reserved: [0; 3],
            blob_size: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            blob_data: std::ptr::null(),
        },
    });
    payload.blob.blob_data = &payload.params as *const AUDIOCLIENT_ACTIVATION_PARAMS as *const c_void;
    let blob_ptr = &payload.blob as *const BlobPropVariant as *const PROPVARIANT;

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let handler: IActivateAudioInterfaceCompletionHandler = ActivationHandler {
        done: Mutex::new(Some(done_tx)),
        _payload: payload,
    }
    .into();

    // SAFETY: the blob and its parameters live in the handler, which the
    // operation keeps alive until it completes.
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(blob_ptr),
            &handler,
        )
    }
    .map_err(|e| anyhow!("{ENGINE_LABEL}: ActivateAudioInterfaceAsync failed for pid {pid}: {e}"))?;

    if done_rx.recv_timeout(ACTIVATION_TIMEOUT).is_err() {
        return Err(anyhow!(
            "{ENGINE_LABEL}: process-loopback activation for pid {pid} timed out after {}s",
            ACTIVATION_TIMEOUT.as_secs()
        ));
    }

    let mut activate_hr = HRESULT(0);
    let mut activated: Option<IUnknown> = None;
    // SAFETY: the operation completed (handler fired).
    unsafe { operation.GetActivateResult(&mut activate_hr, &mut activated) }
        .map_err(|e| anyhow!("{ENGINE_LABEL}: GetActivateResult failed for pid {pid}: {e}"))?;
    activate_hr.ok().map_err(|e| {
        anyhow!("{ENGINE_LABEL}: process-loopback activation failed for pid {pid}: {e}")
    })?;
    let activated = activated.ok_or_else(|| {
        anyhow!("{ENGINE_LABEL}: process-loopback activation for pid {pid} returned no IAudioClient")
    })?;
    activated.cast::<IAudioClient>().map_err(|e| {
        anyhow!("{ENGINE_LABEL}: activated interface for pid {pid} is not an IAudioClient: {e}")
    })
}

// ---------------------------------------------------------------------------
// One running loopback client
// ---------------------------------------------------------------------------

struct LoopbackClient {
    target: LoopbackTarget,
    client: IAudioClient,
    capture: IAudioCaptureClient,
    event: OwnedHandle,
    layout: SampleLayout,
    watch: Option<ProcessWatch>,
}

impl LoopbackClient {
    /// Whole-endpoint loopback on `device`, in the endpoint's mix format.
    fn open_endpoint(target: LoopbackTarget, device: &IMMDevice) -> Result<Self> {
        // SAFETY: COM calls on a valid device; the mix format is freed after
        // Initialize copied it.
        unsafe {
            let client: IAudioClient = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| anyhow!("{ENGINE_LABEL}: activating IAudioClient on the endpoint failed: {e}"))?;
            let mix = client
                .GetMixFormat()
                .map_err(|e| anyhow!("{ENGINE_LABEL}: GetMixFormat failed: {e}"))?;
            let desc = describe_format(mix);
            let init = client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                BUFFER_DURATION_HNS,
                0,
                mix,
                None,
            );
            CoTaskMemFree(Some(mix as *const c_void));
            init.map_err(|e| anyhow!("{ENGINE_LABEL}: initialising the loopback client failed: {e}"))?;
            let layout = SampleLayout::from_format(&desc).map_err(|e| anyhow!("{ENGINE_LABEL}: {e}"))?;
            Self::finish(target, client, layout, None)
        }
    }

    /// Per-process loopback (include or exclude `pid`'s tree) at `rate`.
    fn open_process(target: LoopbackTarget, rate: u32) -> Result<Self> {
        let (pid, exclude) = match &target {
            LoopbackTarget::ProcessTree { pid } => (*pid, false),
            LoopbackTarget::ExcludeProcessTree { pid } => (*pid, true),
            other => return Err(anyhow!("{ENGINE_LABEL}: {other} is not a per-process target")),
        };
        let client = activate_process_loopback(pid, exclude)?;
        // The virtual device has no mix format; ask for 16-bit stereo PCM at
        // the stream rate and let the engine convert.
        let channels: u16 = 2;
        let bits: u16 = 16;
        let block_align = channels * bits / 8;
        let format = WAVEFORMATEX {
            wFormatTag: FORMAT_TAG_PCM,
            nChannels: channels,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * block_align as u32,
            nBlockAlign: block_align,
            wBitsPerSample: bits,
            cbSize: 0,
        };
        // SAFETY: valid client and format.
        unsafe {
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                        | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    BUFFER_DURATION_HNS,
                    0,
                    &format,
                    None,
                )
                .map_err(|e| {
                    anyhow!("{ENGINE_LABEL}: initialising the process-loopback client for pid {pid} failed: {e}")
                })?;
        }
        let layout = SampleLayout::from_format(&FormatDescription {
            format_tag: FORMAT_TAG_PCM,
            channels,
            sample_rate: rate,
            block_align,
            bits_per_sample: bits,
            subformat_tag: None,
        })
        .map_err(|e| anyhow!("{ENGINE_LABEL}: {e}"))?;
        let watch = match ProcessWatch::open(pid) {
            Ok(w) => Some(w),
            Err(e) => {
                warn!("{e}; exit of {target} will be noticed by the process poll only");
                None
            }
        };
        Self::finish(target, client, layout, watch)
    }

    fn finish(
        target: LoopbackTarget,
        client: IAudioClient,
        layout: SampleLayout,
        watch: Option<ProcessWatch>,
    ) -> Result<Self> {
        // SAFETY: COM calls on an initialised client; the event handle is
        // owned by the returned value.
        unsafe {
            let event = CreateEventW(None, BOOL(0), BOOL(0), PCWSTR::null())
                .map_err(|e| anyhow!("{ENGINE_LABEL}: CreateEventW failed: {e}"))?;
            let event = OwnedHandle(event);
            client
                .SetEventHandle(event.0)
                .map_err(|e| anyhow!("{ENGINE_LABEL}: setting the loopback event handle failed: {e}"))?;
            let capture: IAudioCaptureClient = client
                .GetService()
                .map_err(|e| anyhow!("{ENGINE_LABEL}: getting IAudioCaptureClient failed: {e}"))?;
            client
                .Start()
                .map_err(|e| anyhow!("{ENGINE_LABEL}: starting the loopback client failed: {e}"))?;
            Ok(Self {
                target,
                client,
                capture,
                event,
                layout,
                watch,
            })
        }
    }

    /// Waits up to `timeout_ms` for the next packet event.
    fn wait(&self, timeout_ms: u32) -> Result<()> {
        // SAFETY: the event handle is open for the life of self.
        let wait = unsafe { WaitForSingleObject(self.event.0, timeout_ms) };
        if wait == WAIT_OBJECT_0 || wait == WAIT_TIMEOUT {
            Ok(())
        } else {
            Err(anyhow!("waiting on the loopback event failed ({wait:?})"))
        }
    }

    /// Drains every available packet into `out` as mono samples.
    fn drain(&self, out: &mut Vec<f32>) -> Result<()> {
        let block = self.layout.block_align as usize;
        loop {
            // SAFETY: COM calls on a started capture client; the buffer is
            // only read between GetBuffer and ReleaseBuffer.
            unsafe {
                let next = self
                    .capture
                    .GetNextPacketSize()
                    .map_err(|e| describe_client_error("GetNextPacketSize", e))?;
                if next == 0 {
                    return Ok(());
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .map_err(|e| describe_client_error("GetBuffer", e))?;
                let frames_usize = frames as usize;
                if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                    out.extend(std::iter::repeat_n(0.0f32, frames_usize));
                } else {
                    let bytes = std::slice::from_raw_parts(data, frames_usize * block);
                    out.extend(self.layout.decode_mono(bytes, frames_usize));
                }
                if flags & (AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32) != 0 {
                    debug!("{ENGINE_LABEL}: data discontinuity on {}", self.target);
                }
                self.capture
                    .ReleaseBuffer(frames)
                    .map_err(|e| describe_client_error("ReleaseBuffer", e))?;
            }
        }
    }
}

impl Drop for LoopbackClient {
    fn drop(&mut self) {
        // SAFETY: stopping a started client; errors are irrelevant on teardown.
        let _ = unsafe { self.client.Stop() };
    }
}

fn describe_client_error(call: &str, e: windows::core::Error) -> anyhow::Error {
    if e.code() == AUDCLNT_E_DEVICE_INVALIDATED {
        anyhow!("{call} failed: the audio device was removed or reconfigured")
    } else {
        anyhow!("{call} failed: {e}")
    }
}

// ---------------------------------------------------------------------------
// Engine: target choice, rebuilds, the capture loop
// ---------------------------------------------------------------------------

struct ExclusionState {
    file: ExclusionFile,
    untappable: UntappableRoots,
    choice: Option<ExclusionChoice>,
    logged_unsupported: bool,
    logged_uncovered: HashSet<u32>,
}

impl ExclusionState {
    fn new() -> Self {
        Self {
            file: ExclusionFile::new(exclusions::config_path()),
            untappable: UntappableRoots::default(),
            choice: None,
            logged_unsupported: false,
            logged_uncovered: HashSet::new(),
        }
    }
}

struct Engine {
    enumerator: IMMDeviceEnumerator,
    role: CaptureRole,
    label: String,
    support: ProcessLoopbackSupport,
    output_rate: u32,
    client: LoopbackClient,
    resampler: LinearResampler,
    follow: DefaultEndpointFollow,
    exclusions: Option<ExclusionState>,
    debounce: TargetDebounce,
    last_default_poll: Instant,
    last_exclusion_poll: Instant,
    /// Set when the running client is unusable and must be replaced.
    broken: Option<String>,
    rebuild_failures: u32,
    next_retry: Instant,
    logged_named_exclusions: bool,
}

impl Engine {
    fn start(ctx: &ThreadContext) -> Result<Self> {
        let enumerator = create_enumerator()?;
        let support = process_loopback_support();
        let role = resolve_role(&enumerator, &ctx.request);
        let mut exclusions = match role {
            CaptureRole::SystemDefault => Some(ExclusionState::new()),
            _ => None,
        };

        // Pick the first target, then open it; a per-process target that
        // fails falls back to the plain default loopback right here.
        let first = match &role {
            CaptureRole::SystemDefault => {
                let state = exclusions.as_mut().expect("system stream has exclusion state");
                evaluate_exclusions(state, support, &ctx.label);
                desired_system_target(support, state.choice.as_ref(), state.untappable.set())
            }
            CaptureRole::NamedEndpoint(name) => LoopbackTarget::Endpoint { name: name.clone() },
            CaptureRole::ProcessTree(pids) => initial_process_tree_target(support, pids),
        };
        let client = match open_target(&enumerator, &first, None) {
            Ok(client) => client,
            Err(e) if first.is_per_process() => {
                if let (Some(state), Some(pid)) = (exclusions.as_mut(), first.watched_pid()) {
                    state.untappable.mark(pid);
                }
                warn!(
                    "{ENGINE_LABEL}: {first} could not be tapped for {} ({e:#}); \
                     capturing the whole default output instead",
                    ctx.label
                );
                open_target(&enumerator, &LoopbackTarget::DefaultRenderEndpoint, None)?
            }
            Err(e) => return Err(e),
        };

        let output_rate = client.layout.sample_rate;
        let now = Instant::now();
        let follow = DefaultEndpointFollow::new(
            DEFAULT_SWITCH_GRACE,
            default_render_endpoint_id(&enumerator),
        );
        Ok(Self {
            enumerator,
            role,
            label: ctx.label.clone(),
            support,
            output_rate,
            resampler: LinearResampler::new(output_rate, output_rate),
            client,
            follow,
            exclusions,
            debounce: TargetDebounce::new(EXCLUSION_MIN_INTERVAL),
            last_default_poll: now,
            last_exclusion_poll: now,
            broken: None,
            rebuild_failures: 0,
            next_retry: now,
            logged_named_exclusions: false,
        })
    }

    fn config(&self) -> AudioStreamConfig {
        AudioStreamConfig::new(self.output_rate, self.client.layout.channels)
    }

    fn run(&mut self, ctx: &ThreadContext) {
        let mut mono: Vec<f32> = Vec::with_capacity(8192);
        while !ctx.is_disconnected.load(Ordering::Relaxed) {
            if self.broken.is_some() {
                // Nothing to wait on; pace the rebuild attempts.
                std::thread::sleep(Duration::from_millis(EVENT_WAIT_MS as u64));
            } else {
                if let Err(e) = self.client.wait(EVENT_WAIT_MS) {
                    self.mark_broken(format!("{e:#}"));
                } else if let Err(e) = self.client.drain(&mut mono) {
                    self.mark_broken(format!("draining packets failed: {e:#}"));
                }
                if !mono.is_empty() {
                    let samples = self.resampler.process(&mono);
                    mono.clear();
                    let _ = ctx.tx.send(samples);
                }
            }

            if !self.check_target_exit(ctx) {
                break;
            }
            let now = Instant::now();
            if now.duration_since(self.last_default_poll) >= DEFAULT_POLL {
                self.last_default_poll = now;
                self.poll_default_endpoint(now);
            }
            if now.duration_since(self.last_exclusion_poll) >= EXCLUSION_POLL {
                self.last_exclusion_poll = now;
                self.poll_exclusions(now);
            }
            if self.broken.is_some() && now >= self.next_retry && !self.repair(ctx) {
                break;
            }
        }
    }

    fn mark_broken(&mut self, reason: String) {
        if self.broken.is_none() {
            warn!(
                "{ENGINE_LABEL}: loopback {} for {} stopped working: {reason}; rebuilding",
                self.client.target, self.label
            );
            self.broken = Some(reason);
            self.next_retry = Instant::now();
        }
    }

    /// Handles the exit of a watched target pid. Returns `false` when the
    /// stream should end.
    fn check_target_exit(&mut self, ctx: &ThreadContext) -> bool {
        let Some(watch) = &self.client.watch else {
            return true;
        };
        let exited = match watch.has_exited() {
            Ok(exited) => exited,
            Err(e) => {
                warn!("{e}");
                true
            }
        };
        if !exited {
            return true;
        }
        let target = self.client.target.clone();
        self.client.watch = None;
        match (&self.role, &target) {
            (CaptureRole::ProcessTree(_), LoopbackTarget::ProcessTree { .. }) => {
                info!(
                    "{ENGINE_LABEL}: loopback target {target} for {} exited; stopping the tap",
                    self.label
                );
                ctx.is_disconnected.store(true, Ordering::Relaxed);
                false
            }
            _ => {
                info!(
                    "{ENGINE_LABEL}: excluded process tree {target} exited; re-checking exclusions"
                );
                // Look again on the next pass instead of waiting for the poll.
                self.last_exclusion_poll = Instant::now() - EXCLUSION_POLL;
                true
            }
        }
    }

    fn poll_default_endpoint(&mut self, now: Instant) {
        if !self.client.target.follows_default_endpoint() {
            return;
        }
        let latest = default_render_endpoint_id(&self.enumerator);
        let was_pending = self.follow.pending_id().map(str::to_string);
        let ready = self.follow.observe(latest, now);
        let pending = self.follow.pending_id().map(str::to_string);
        if pending.is_some() && pending != was_pending {
            info!(
                "{ENGINE_LABEL}: default render endpoint changed ({} -> {}); the loopback for {} \
                 moves in {}s unless the device monitor replaces this stream first",
                self.follow.current_id().unwrap_or("none"),
                pending.as_deref().unwrap_or("none"),
                self.label,
                DEFAULT_SWITCH_GRACE.as_secs()
            );
        }
        if let Some(new_id) = ready {
            info!(
                "{ENGINE_LABEL}: rebuilding loopback {} for {} on the new default render endpoint {new_id}",
                self.client.target, self.label
            );
            let target = self.client.target.clone();
            self.debounce.note_switch(now);
            self.switch_to(target, "default render endpoint changed");
        }
    }

    fn poll_exclusions(&mut self, now: Instant) {
        let Some(state) = self.exclusions.as_mut() else {
            if !self.logged_named_exclusions {
                if let CaptureRole::NamedEndpoint(name) = &self.role {
                    let mut file = ExclusionFile::new(exclusions::config_path());
                    file.poll();
                    if !file.exe_names().is_empty() {
                        info!(
                            "{ENGINE_LABEL}: audio exclusions apply to the default output stream only; \
                             '{name}' is captured in full"
                        );
                    }
                }
                self.logged_named_exclusions = true;
            }
            return;
        };
        evaluate_exclusions(state, self.support, &self.label);
        let desired =
            desired_system_target(self.support, state.choice.as_ref(), state.untappable.set());
        if self.broken.is_some() {
            // repair() picks the desired target itself.
            return;
        }
        if let Some(next) = self.debounce.observe(&self.client.target, desired, now) {
            info!(
                "{ENGINE_LABEL}: exclusion target changed ({} -> {next}) for {}; rebuilding loopback",
                self.client.target, self.label
            );
            self.switch_to(next, "exclusion target changed");
        }
    }

    /// Opens `target` and swaps it in. The running client keeps capturing
    /// until the new one works, and a per-process target that fails falls
    /// back to the plain default loopback, so the stream never goes quiet
    /// because of a rebuild.
    fn switch_to(&mut self, target: LoopbackTarget, reason: &str) {
        match open_target(&self.enumerator, &target, Some(self.output_rate)) {
            Ok(client) => self.install(client),
            Err(e) if target.is_per_process() => {
                self.note_untappable(&target, &e);
                if !self.client.target.is_per_process() && self.broken.is_none() {
                    // Already on a plain loopback that works: stay on it.
                    return;
                }
                match open_target(
                    &self.enumerator,
                    &LoopbackTarget::DefaultRenderEndpoint,
                    Some(self.output_rate),
                ) {
                    Ok(client) => self.install(client),
                    Err(e) => self.note_rebuild_failure(reason, &e),
                }
            }
            Err(e) => self.note_rebuild_failure(reason, &e),
        }
    }

    /// Replaces a broken client. Returns `false` when the stream gives up.
    fn repair(&mut self, ctx: &ThreadContext) -> bool {
        let target = match &self.role {
            CaptureRole::SystemDefault => self
                .exclusions
                .as_ref()
                .map(|s| desired_system_target(self.support, s.choice.as_ref(), s.untappable.set()))
                .unwrap_or(LoopbackTarget::DefaultRenderEndpoint),
            _ => self.client.target.clone(),
        };
        let reason = self.broken.clone().unwrap_or_default();
        self.switch_to(target, &reason);
        if self.broken.is_none() {
            return true;
        }
        if self.rebuild_failures >= MAX_REBUILD_FAILURES {
            error!(
                "{ENGINE_LABEL}: giving up on {} after {} failed rebuilds ({reason}); \
                 handing the device back to the device monitor",
                self.label, self.rebuild_failures
            );
            record_engine_failure(device_key(&self.label));
            ctx.is_disconnected.store(true, Ordering::Relaxed);
            return false;
        }
        true
    }

    fn install(&mut self, client: LoopbackClient) {
        let rate = client.layout.sample_rate;
        if rate != self.output_rate {
            info!(
                "{ENGINE_LABEL}: {} runs at {rate} Hz; resampling to the stream rate {} Hz",
                client.target, self.output_rate
            );
        }
        self.resampler = LinearResampler::new(rate, self.output_rate);
        if client.target.follows_default_endpoint() {
            self.follow
                .set_current(default_render_endpoint_id(&self.enumerator));
        }
        info!(
            "{ENGINE_LABEL}: {} now captures {} (was {})",
            self.label, client.target, self.client.target
        );
        // The old client stops when it is dropped here.
        self.client = client;
        self.broken = None;
        self.rebuild_failures = 0;
    }

    fn note_untappable(&mut self, target: &LoopbackTarget, e: &anyhow::Error) {
        let Some(pid) = target.watched_pid() else {
            return;
        };
        match self.exclusions.as_mut() {
            Some(state) => {
                if state.untappable.mark(pid) {
                    let exe = state
                        .choice
                        .as_ref()
                        .filter(|c| c.root_pid == pid)
                        .map(|c| c.exe_name.clone())
                        .unwrap_or_else(|| "excluded app".to_string());
                    warn!(
                        "{ENGINE_LABEL}: {exe} (process tree {pid}) cannot be excluded: {e:#}; \
                         capturing the whole system mix while it runs"
                    );
                }
            }
            None => warn!(
                "{ENGINE_LABEL}: {target} could not be tapped for {} ({e:#}); \
                 capturing the whole default output instead",
                self.label
            ),
        }
    }

    fn note_rebuild_failure(&mut self, reason: &str, e: &anyhow::Error) {
        if self.broken.is_none() {
            // The old client still runs, so nothing is lost: keep it. A
            // default-endpoint move is retried after the next grace period.
            warn!(
                "{ENGINE_LABEL}: rebuilding loopback for {} failed ({reason}): {e:#}; \
                 keeping {}",
                self.label, self.client.target
            );
            return;
        }
        self.rebuild_failures += 1;
        let backoff = REBUILD_RETRY_BASE
            .saturating_mul(1u32 << self.rebuild_failures.min(4))
            .min(Duration::from_secs(8));
        self.next_retry = Instant::now() + backoff;
        warn!(
            "{ENGINE_LABEL}: rebuilding loopback for {} failed ({reason}): {e:#}; \
             retrying in {}ms ({} of {MAX_REBUILD_FAILURES})",
            self.label,
            backoff.as_millis(),
            self.rebuild_failures
        );
    }
}

/// The failure-registry key is the bare device name; labels carry the
/// `" (output)"` suffix of the device's display form.
fn device_key(label: &str) -> &str {
    label.strip_suffix(" (output)").unwrap_or(label)
}

/// Decides the role of an output-device request: the device that is the
/// current default (or the `default` sentinel) is the system-audio stream.
fn resolve_role(enumerator: &IMMDeviceEnumerator, request: &CaptureRequest) -> CaptureRole {
    match request {
        CaptureRequest::ProcessTree { pids } => CaptureRole::ProcessTree(pids.clone()),
        CaptureRequest::OutputDevice { name } => {
            if name.trim().eq_ignore_ascii_case("default") {
                return CaptureRole::SystemDefault;
            }
            let default_name = default_render_endpoint(enumerator)
                .ok()
                .and_then(|d| friendly_name(&d));
            if default_name.as_deref() == Some(name.as_str()) {
                CaptureRole::SystemDefault
            } else {
                CaptureRole::NamedEndpoint(name.clone())
            }
        }
    }
}

/// The starting target of a per-process include request, with the
/// multi-root compatibility fallback.
fn initial_process_tree_target(support: ProcessLoopbackSupport, pids: &[u32]) -> LoopbackTarget {
    if pids.is_empty() {
        warn!("{ENGINE_LABEL}: process tap requested without a target pid; capturing the whole default output");
        return LoopbackTarget::DefaultRenderEndpoint;
    }
    if !support.is_available() {
        warn!(
            "{ENGINE_LABEL}: per-process taps need Windows build 20348 or later; older builds \
             cannot isolate a process tree without recording the whole system mix ({})",
            support.describe()
        );
        return LoopbackTarget::DefaultRenderEndpoint;
    }
    let snapshot = match snapshot_processes() {
        Ok(s) => s,
        Err(e) => {
            warn!("{e:#}; capturing the whole default output");
            return LoopbackTarget::DefaultRenderEndpoint;
        }
    };
    let resolution = resolve_include_root(&snapshot, pids);
    match &resolution {
        IncludeRoot::MultipleRoots { roots } => warn!(
            "{ENGINE_LABEL}: requested pids {pids:?} belong to {} independent process trees \
             ({roots:?}); compatibility fallback to the whole default output",
            roots.len()
        ),
        IncludeRoot::NotRunning => warn!(
            "{ENGINE_LABEL}: none of the requested pids {pids:?} is running; capturing the whole default output"
        ),
        IncludeRoot::Single { root } => {
            let teams_root = snapshot
                .get(*root)
                .map(|e| e.exe_name == super::process_tap::targets::TEAMS_DESKTOP_EXE)
                .unwrap_or(false);
            if teams_root && !pids.contains(root) {
                info!(
                    "{ENGINE_LABEL}: requested pids {pids:?} resolve through the Teams desktop app; \
                     tapping its tree root {root}"
                );
            }
        }
    }
    initial_include_target(support, &resolution)
}

/// Opens a client for `target`. `rate` is the stream rate a per-process
/// client should run at (endpoint clients use their mix rate).
fn open_target(
    enumerator: &IMMDeviceEnumerator,
    target: &LoopbackTarget,
    rate: Option<u32>,
) -> Result<LoopbackClient> {
    match target {
        LoopbackTarget::DefaultRenderEndpoint => {
            let device = default_render_endpoint(enumerator)?;
            LoopbackClient::open_endpoint(target.clone(), &device)
        }
        LoopbackTarget::Endpoint { name } => {
            let device = find_render_endpoint(enumerator, name)?;
            LoopbackClient::open_endpoint(target.clone(), &device)
        }
        LoopbackTarget::ProcessTree { .. } | LoopbackTarget::ExcludeProcessTree { .. } => {
            let rate = rate.unwrap_or_else(|| {
                default_render_endpoint(enumerator)
                    .ok()
                    .and_then(|d| mix_rate(&d))
                    .unwrap_or(PROCESS_LOOPBACK_DEFAULT_RATE)
            });
            LoopbackClient::open_process(target.clone(), rate)
        }
    }
}

/// The shared-mode mix rate of an endpoint, so a per-process client can
/// match it.
fn mix_rate(device: &IMMDevice) -> Option<u32> {
    // SAFETY: COM calls on a valid device; the format is freed here.
    unsafe {
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).ok()?;
        let mix = client.GetMixFormat().ok()?;
        let desc = describe_format(mix);
        CoTaskMemFree(Some(mix as *const c_void));
        (desc.sample_rate > 0).then_some(desc.sample_rate)
    }
}

/// Re-reads the exclusion file and process list and records the chosen
/// exclusion in `state`.
fn evaluate_exclusions(state: &mut ExclusionState, support: ProcessLoopbackSupport, label: &str) {
    if state.file.poll() {
        if state.file.exe_names().is_empty() {
            info!("{ENGINE_LABEL}: audio exclusion list is empty");
        } else {
            info!(
                "{ENGINE_LABEL}: audio exclusion list from {}: {:?}",
                state.file.path().display(),
                state.file.exe_names()
            );
        }
    }
    let names = state.file.exe_names().to_vec();
    if names.is_empty() {
        state.choice = None;
        return;
    }
    if !support.is_available() {
        if !state.logged_unsupported {
            warn!(
                "{ENGINE_LABEL}: audio exclusions need per-process loopback ({}); {label} records \
                 the whole system mix",
                support.describe()
            );
            state.logged_unsupported = true;
        }
        state.choice = None;
        return;
    }
    let snapshot = match snapshot_processes() {
        Ok(s) => s,
        Err(e) => {
            debug!("{e:#}; keeping the previous exclusion choice");
            return;
        }
    };
    state.untappable.retain_running(&snapshot);
    state.logged_uncovered.retain(|pid| snapshot.contains(*pid));
    state.choice = choose_exclusion(&snapshot, &names);
    if let Some(choice) = &state.choice {
        for root in &choice.uncovered_roots {
            if state.logged_uncovered.insert(*root) {
                info!(
                    "{ENGINE_LABEL}: {} also runs as process tree {root}; only one tree can be \
                     excluded, so that one stays audible",
                    choice.exe_name
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_propvariant_matches_propvariant_layout() {
        assert_eq!(
            std::mem::size_of::<BlobPropVariant>(),
            std::mem::size_of::<PROPVARIANT>()
        );
        assert_eq!(std::mem::offset_of!(BlobPropVariant, blob_size), 8);
    }

    #[test]
    fn build_number_is_known_on_windows() {
        let build = windows_build_number().expect("RtlGetVersion should report a build");
        assert!(build > 7600, "unexpected build {build}");
        let support = process_loopback_support();
        assert_eq!(
            support.is_available(),
            build >= super::super::process_tap::targets::MIN_PROCESS_LOOPBACK_BUILD
        );
        assert_eq!(
            super::super::process_tap::is_process_tap_available(),
            support.is_available()
        );
    }

    #[test]
    fn process_snapshot_contains_this_process() {
        let snapshot = snapshot_processes().expect("ToolHelp snapshot");
        let me = std::process::id();
        let entry = snapshot.get(me).expect("own pid in the snapshot");
        assert!(entry.exe_name.ends_with(".exe"), "{}", entry.exe_name);
        assert!(snapshot.same_exe_root(me).is_some());
    }

    #[test]
    fn device_key_strips_the_output_suffix() {
        assert_eq!(device_key("Speakers (Realtek(R) Audio) (output)"), "Speakers (Realtek(R) Audio)");
        assert_eq!(device_key("Meeting Tap"), "Meeting Tap");
    }

    #[test]
    fn engine_failures_exhaust_per_device() {
        let name = "test-device-that-does-not-exist";
        assert!(engine_allowed_for(name));
        for _ in 0..MAX_ENGINE_FAILURES {
            record_engine_failure(name);
        }
        assert!(!engine_allowed_for(name));
        assert!(engine_allowed_for("another-test-device"));
    }

    /// Needs an active render endpoint; CI runners usually have none.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs an audio output device"]
    async fn default_endpoint_loopback_starts_and_stops() {
        let (tx, _rx) = broadcast::channel(16);
        let disconnected = Arc::new(AtomicBool::new(false));
        let (config, handle) = spawn_loopback_capture(
            CaptureRequest::OutputDevice {
                name: "default".to_string(),
            },
            "default (output)".to_string(),
            tx,
            disconnected.clone(),
        )
        .await
        .expect("loopback start");
        assert!(config.sample_rate().0 > 0);
        disconnected.store(true, Ordering::Relaxed);
        handle.await.unwrap();
    }
}
