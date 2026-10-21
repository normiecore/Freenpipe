// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! The three exports WER looks up in a runtime exception module, and the
//! minidump writer behind the first one. Everything here runs inside
//! WerFault.exe while the crashed process is suspended; it must not panic
//! across the FFI boundary and must not take long.

use std::ffi::{c_void, OsString};
use std::fs::{self, File};
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use windows_sys::core::{HRESULT, PCWSTR, PWSTR};
use windows_sys::Win32::Foundation::{BOOL, E_NOTIMPL, FALSE, HANDLE, HMODULE, S_OK};
use windows_sys::Win32::System::Diagnostics::Debug::{
    MiniDumpWithHandleData, MiniDumpWithIndirectlyReferencedMemory, MiniDumpWithThreadInfo,
    MiniDumpWithUnloadedModules, MiniDumpWriteDump, ReadProcessMemory, CONTEXT, EXCEPTION_POINTERS,
    EXCEPTION_RECORD, MINIDUMP_EXCEPTION_INFORMATION, MINIDUMP_TYPE,
};
use windows_sys::Win32::System::ErrorReporting::WER_RUNTIME_EXCEPTION_INFORMATION;
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows_sys::Win32::System::Threading::{
    GetProcessId, GetThreadId, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
};

use crate::layout::{
    dump_file_name, is_finished_dump, may_write_dump, parse_context, CONTEXT_BLOCK_BYTES,
    CRASH_DUMPS_DIR, PARTIAL_SUFFIX,
};

/// Threads, their stacks and the memory those stacks point at, plus handles
/// and unloaded modules: enough to read a native stack trace, a few MB.
const DUMP_TYPE: MINIDUMP_TYPE = MiniDumpWithIndirectlyReferencedMemory
    | MiniDumpWithThreadInfo
    | MiniDumpWithUnloadedModules
    | MiniDumpWithHandleData;

/// `CONTEXT` must be 16-byte aligned on x64 for the debugging APIs.
#[repr(C, align(16))]
struct AlignedContext(CONTEXT);

/// Called by WER for a crash of a process that registered this module.
/// Writes the dump and leaves the crash unclaimed, so WER goes on exactly as
/// it would without this module.
///
/// # Safety
/// Called by WerFault.exe with the pointers its contract describes.
#[no_mangle]
pub unsafe extern "system" fn OutOfProcessExceptionEventCallback(
    pcontext: *const c_void,
    pexceptioninformation: *const WER_RUNTIME_EXCEPTION_INFORMATION,
    pbownershipclaimed: *mut BOOL,
    _pwszeventname: PWSTR,
    _pchsize: *mut u32,
    _pdwsignaturecount: *mut u32,
) -> HRESULT {
    if !pbownershipclaimed.is_null() {
        *pbownershipclaimed = FALSE;
    }
    if pexceptioninformation.is_null() {
        return S_OK;
    }
    let info = std::panic::AssertUnwindSafe(&*pexceptioninformation);
    let context = pcontext as usize;
    let _ = std::panic::catch_unwind(move || {
        let info = info;
        write_dump(context as *const c_void, info.0)
    });
    S_OK
}

/// Only called for a crash this module claimed, which never happens.
///
/// # Safety
/// Called by WerFault.exe; the arguments are not used.
#[no_mangle]
pub unsafe extern "system" fn OutOfProcessExceptionEventSignatureCallback(
    _pcontext: *const c_void,
    _pexceptioninformation: *const WER_RUNTIME_EXCEPTION_INFORMATION,
    _dwindex: u32,
    _pwszname: PWSTR,
    _pchname: *mut u32,
    _pwszvalue: PWSTR,
    _pchvalue: *mut u32,
) -> HRESULT {
    E_NOTIMPL
}

/// No custom debugger: WER keeps its own choice.
///
/// # Safety
/// Called by WerFault.exe with the pointers its contract describes.
#[no_mangle]
pub unsafe extern "system" fn OutOfProcessExceptionEventDebuggerLaunchCallback(
    _pcontext: *const c_void,
    _pexceptioninformation: *const WER_RUNTIME_EXCEPTION_INFORMATION,
    pbiscustomdebugger: *mut BOOL,
    _pwszdebuggerlaunch: PWSTR,
    _pchdebuggerlaunch: *mut u32,
    pbisdebuggerautolaunch: *mut BOOL,
) -> HRESULT {
    if !pbiscustomdebugger.is_null() {
        *pbiscustomdebugger = FALSE;
    }
    if !pbisdebuggerautolaunch.is_null() {
        *pbisdebuggerautolaunch = FALSE;
    }
    S_OK
}

/// Write one minidump. Errors are swallowed by the caller: a failed dump
/// must never change how WER handles the crash.
fn write_dump(context: *const c_void, info: &WER_RUNTIME_EXCEPTION_INFORMATION) -> Option<PathBuf> {
    let process = info.hProcess;
    if process.is_null() {
        return None;
    }
    let dir = directory_from_context(process, context).or_else(fallback_directory)?;
    fs::create_dir_all(&dir).ok()?;
    if !may_write_dump(finished_dumps_in(&dir)) {
        return None;
    }

    // SAFETY: plain queries on handles WER gave us.
    let pid = unsafe { GetProcessId(process) };
    let thread_id = if info.hThread.is_null() {
        0
    } else {
        unsafe { GetThreadId(info.hThread) }
    };
    let exe = image_path(process).unwrap_or_default();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = dump_file_name(&exe, pid, now);
    let target = dir.join(&name);
    let partial = dir.join(format!("{name}{PARTIAL_SUFFIX}"));

    let file = File::create_new(&partial).ok()?;
    // The full structure carries the faulting record and register state; an
    // older, shorter one does not, and the dump is written without them.
    let has_exception = info.dwSize as usize
        >= std::mem::size_of::<WER_RUNTIME_EXCEPTION_INFORMATION>()
        && thread_id != 0;
    let mut record: EXCEPTION_RECORD = info.exceptionRecord;
    let mut registers = AlignedContext(info.context);
    let mut pointers = EXCEPTION_POINTERS {
        ExceptionRecord: &mut record,
        ContextRecord: &mut registers.0,
    };
    let exception = MINIDUMP_EXCEPTION_INFORMATION {
        ThreadId: thread_id,
        ExceptionPointers: &mut pointers,
        // The pointers above live in this process, not the crashed one.
        ClientPointers: FALSE,
    };
    let exception_param: *const MINIDUMP_EXCEPTION_INFORMATION = if has_exception {
        &exception
    } else {
        std::ptr::null()
    };
    // SAFETY: the process handle comes from WER, the file handle stays open
    // for the call, and the exception pointers outlive it.
    let written = unsafe {
        MiniDumpWriteDump(
            process,
            pid,
            file.as_raw_handle() as HANDLE,
            DUMP_TYPE,
            exception_param,
            std::ptr::null(),
            std::ptr::null(),
        )
    } != 0;
    let synced = written && file.sync_all().is_ok();
    drop(file);
    if !synced || fs::rename(&partial, &target).is_err() {
        let _ = fs::remove_file(&partial);
        return None;
    }
    Some(target)
}

/// Directory named in the app's context block, read out of the crashed
/// process.
fn directory_from_context(process: HANDLE, context: *const c_void) -> Option<PathBuf> {
    if context.is_null() {
        return None;
    }
    let mut bytes = vec![0u8; CONTEXT_BLOCK_BYTES];
    let mut read = 0usize;
    // SAFETY: reads into a buffer of exactly the requested size; the remote
    // address is only dereferenced by the kernel, which checks it.
    let ok = unsafe {
        ReadProcessMemory(
            process,
            context,
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            &mut read,
        )
    } != 0;
    if !ok || read != bytes.len() {
        return None;
    }
    let units = parse_context(&bytes)?;
    let dir = PathBuf::from(OsString::from_wide(&units));
    dir.is_absolute().then_some(dir)
}

/// `crash-dumps` next to this DLL, for a crash whose context block could
/// not be read.
fn fallback_directory() -> Option<PathBuf> {
    let mut module: HMODULE = std::ptr::null_mut();
    let anchor = OutOfProcessExceptionEventCallback as *const () as PCWSTR;
    // SAFETY: looks up the module that contains one of our own functions,
    // without taking a reference on it.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            anchor,
            &mut module,
        )
    } != 0;
    if !found {
        return None;
    }
    let mut buf = vec![0u16; 32_768];
    // SAFETY: the buffer length is passed alongside it.
    let len = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if len == 0 || len >= buf.len() {
        return None;
    }
    let dll = PathBuf::from(OsString::from_wide(&buf[..len]));
    Some(dll.parent()?.join(CRASH_DUMPS_DIR))
}

/// Full path of the crashed executable.
fn image_path(process: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; 32_768];
    let mut len = buf.len() as u32;
    // SAFETY: `len` holds the buffer capacity and receives the used length.
    let ok = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
    } != 0;
    if !ok {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

/// Finished dumps already waiting in `dir`.
fn finished_dumps_in(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| is_finished_dump(&e.file_name().to_string_lossy()))
                .count()
        })
        .unwrap_or(0)
}
