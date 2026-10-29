// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Win32 monitor enumeration for the Windows capture path.
//!
//! Monitors come from `EnumDisplayMonitors`, with the device name from
//! `GetMonitorInfoW` and the position and pixel size from the current display
//! mode (`EnumDisplaySettingsW`). That is the same source xcap reads, so a
//! monitor keeps the id (`HMONITOR` truncated to `u32`) and the
//! `stable_id()` (`"{name}_{w}x{h}_{x},{y}"`) it had when monitors were listed
//! through xcap, and stored `monitorIds` keep matching.
//!
//! The helpers that turn raw Win32 values into monitor data are plain
//! functions compiled on every OS so they are unit-tested everywhere; only the
//! system calls are `cfg(windows)`.

use super::MonitorData;

/// One display as reported by Win32, with the handle kept as an integer so the
/// value is `Send` (an `HMONITOR` wraps a raw pointer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeMonitor {
    /// The `HMONITOR` value.
    pub hmonitor: isize,
    /// Monitor id as xcap and earlier screenpipe builds report it.
    pub id: u32,
    /// GDI device name, for example `\\.\DISPLAY1`.
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
}

impl NativeMonitor {
    pub(crate) fn monitor_data(&self) -> MonitorData {
        MonitorData {
            width: self.width,
            height: self.height,
            x: self.x,
            y: self.y,
            name: self.name.clone(),
            is_primary: self.is_primary,
        }
    }
}

/// Monitor id from an `HMONITOR` value: the low 32 bits, exactly as xcap
/// derives it.
pub(crate) fn monitor_id_from_handle(hmonitor: isize) -> u32 {
    hmonitor as u32
}

/// A NUL-terminated UTF-16 buffer (such as `szDevice`) as a `String`.
pub(crate) fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Current display mode of a monitor: position on the virtual desktop and
/// size in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DisplayMode {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// `rcMonitor` from `MONITORINFO`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MonitorRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Geometry for a monitor. The display mode is what xcap reports, so it wins;
/// the monitor rectangle is only used when the mode query fails (base builds
/// reported 0x0 at 0,0 in that case, which helps nobody).
pub(crate) fn monitor_geometry(mode: Option<DisplayMode>, rect: MonitorRect) -> DisplayMode {
    match mode {
        Some(mode) => mode,
        None => DisplayMode {
            x: rect.left,
            y: rect.top,
            width: rect.right.saturating_sub(rect.left).max(0) as u32,
            height: rect.bottom.saturating_sub(rect.top).max(0) as u32,
        },
    }
}

/// `MONITORINFOF_PRIMARY`.
const MONITOR_FLAG_PRIMARY: u32 = 1;

/// Assemble a [`NativeMonitor`] from the raw Win32 values.
pub(crate) fn native_monitor(
    hmonitor: isize,
    device_name: &[u16],
    flags: u32,
    rect: MonitorRect,
    mode: Option<DisplayMode>,
) -> NativeMonitor {
    let geometry = monitor_geometry(mode, rect);
    NativeMonitor {
        hmonitor,
        id: monitor_id_from_handle(hmonitor),
        name: wide_to_string(device_name),
        x: geometry.x,
        y: geometry.y,
        width: geometry.width,
        height: geometry.height,
        is_primary: flags & MONITOR_FLAG_PRIMARY != 0,
    }
}

/// Pick the monitor with `id` from an enumeration.
pub(crate) fn select_monitor(monitors: Vec<NativeMonitor>, id: u32) -> Option<NativeMonitor> {
    monitors.into_iter().find(|m| m.id == id)
}

#[cfg(target_os = "windows")]
pub(crate) use system::{enumerate_monitors, find_monitor};

#[cfg(target_os = "windows")]
mod system {
    use super::{native_monitor, select_monitor, DisplayMode, MonitorRect, NativeMonitor};
    use anyhow::{anyhow, Result};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{BOOL, LPARAM, RECT, TRUE};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW,
        ENUM_CURRENT_SETTINGS, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    };

    unsafe extern "system" fn collect_handle(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _clip: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` is the `&mut Vec<isize>` passed by `enumerate_monitors`,
        // alive for the whole synchronous EnumDisplayMonitors call.
        let handles = &mut *(data.0 as *mut Vec<isize>);
        handles.push(hmonitor.0 as isize);
        TRUE
    }

    fn display_mode(device: &[u16; 32]) -> Option<DisplayMode> {
        let mut mode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        let ok = unsafe {
            EnumDisplaySettingsW(PCWSTR(device.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode)
        };
        if !ok.as_bool() {
            return None;
        }
        // SAFETY: for display devices the position variant of the union is
        // the one EnumDisplaySettingsW fills in.
        let position = unsafe { mode.Anonymous1.Anonymous2.dmPosition };
        Some(DisplayMode {
            x: position.x,
            y: position.y,
            width: mode.dmPelsWidth,
            height: mode.dmPelsHeight,
        })
    }

    fn describe(hmonitor: isize) -> Option<NativeMonitor> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        let ok = unsafe {
            GetMonitorInfoW(
                HMONITOR(hmonitor as *mut core::ffi::c_void),
                &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
            )
        };
        if !ok.as_bool() {
            tracing::debug!("GetMonitorInfoW failed for monitor handle {:#x}", hmonitor);
            return None;
        }
        let r = info.monitorInfo.rcMonitor;
        let rect = MonitorRect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        };
        Some(native_monitor(
            hmonitor,
            &info.szDevice,
            info.monitorInfo.dwFlags,
            rect,
            display_mode(&info.szDevice),
        ))
    }

    /// All monitors in `EnumDisplayMonitors` order (the order xcap returns).
    /// A monitor whose details cannot be read (it vanished mid-enumeration)
    /// is skipped.
    pub(crate) fn enumerate_monitors() -> Result<Vec<NativeMonitor>> {
        let mut handles: Vec<isize> = Vec::new();
        unsafe {
            EnumDisplayMonitors(
                HDC::default(),
                None,
                Some(collect_handle),
                LPARAM(&mut handles as *mut Vec<isize> as isize),
            )
        }
        .ok()
        .map_err(|e| anyhow!("EnumDisplayMonitors failed: {}", e))?;
        Ok(handles.into_iter().filter_map(describe).collect())
    }

    /// The monitor with `id`, for opening a persistent capture session.
    pub(crate) fn find_monitor(id: u32) -> Result<NativeMonitor> {
        select_monitor(enumerate_monitors()?, id).ok_or_else(|| {
            anyhow!(
                "monitor {} requested for persistent capture was not found",
                id
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> [u16; 32] {
        let mut buf = [0u16; 32];
        for (slot, unit) in buf.iter_mut().zip(s.encode_utf16()) {
            *slot = unit;
        }
        buf
    }

    const RECT: MonitorRect = MonitorRect {
        left: -1920,
        top: 0,
        right: 0,
        bottom: 1080,
    };

    #[test]
    fn id_is_the_low_32_bits_of_the_handle() {
        assert_eq!(monitor_id_from_handle(0x0001_0001), 0x0001_0001);
        assert_eq!(monitor_id_from_handle(0x7_0001_0003), 0x0001_0003);
        assert_eq!(monitor_id_from_handle(-1), u32::MAX);
    }

    #[test]
    fn device_name_stops_at_the_terminator() {
        assert_eq!(wide_to_string(&wide(r"\\.\DISPLAY1")), r"\\.\DISPLAY1");
        assert_eq!(wide_to_string(&[0x44, 0x31]), "D1", "no terminator");
        assert_eq!(wide_to_string(&[0, 0x44]), "");
    }

    #[test]
    fn display_mode_wins_over_the_monitor_rect() {
        let mode = DisplayMode {
            x: -3840,
            y: 0,
            width: 3840,
            height: 2160,
        };
        assert_eq!(monitor_geometry(Some(mode), RECT), mode);
    }

    #[test]
    fn monitor_rect_is_the_fallback() {
        assert_eq!(
            monitor_geometry(None, RECT),
            DisplayMode {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080
            }
        );
        let inverted = MonitorRect {
            left: 10,
            top: 10,
            right: 0,
            bottom: 0,
        };
        let g = monitor_geometry(None, inverted);
        assert_eq!((g.width, g.height), (0, 0));
    }

    #[test]
    fn native_monitor_matches_the_xcap_derived_fields() {
        let mode = DisplayMode {
            x: 2560,
            y: -200,
            width: 2560,
            height: 1440,
        };
        let m = native_monitor(0x0002_0005, &wide(r"\\.\DISPLAY2"), 0, RECT, Some(mode));
        assert_eq!(m.id, 0x0002_0005);
        assert_eq!(m.name, r"\\.\DISPLAY2");
        assert_eq!((m.x, m.y, m.width, m.height), (2560, -200, 2560, 1440));
        assert!(!m.is_primary);

        let primary = native_monitor(1, &wide(r"\\.\DISPLAY1"), 1, RECT, Some(mode));
        assert!(primary.is_primary);
    }

    #[test]
    fn monitor_data_keeps_the_stable_id_inputs() {
        let m = native_monitor(
            7,
            &wide(r"\\.\DISPLAY3"),
            1,
            RECT,
            Some(DisplayMode {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            }),
        );
        let data = m.monitor_data();
        assert_eq!(data.name, r"\\.\DISPLAY3");
        assert_eq!(
            (data.width, data.height, data.x, data.y),
            (1920, 1080, 0, 0)
        );
        assert!(data.is_primary);
    }

    #[test]
    fn select_monitor_by_id() {
        let a = native_monitor(11, &wide("A"), 1, RECT, None);
        let b = native_monitor(22, &wide("B"), 0, RECT, None);
        let found = select_monitor(vec![a, b.clone()], 22);
        assert_eq!(found, Some(b));
        assert_eq!(select_monitor(Vec::new(), 22), None);
    }
}
