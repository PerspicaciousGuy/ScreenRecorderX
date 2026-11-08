use super::*;
use ::windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO},
        UI::WindowsAndMessaging::{EnumWindows, GetWindowRect, GetWindowTextW, IsWindowVisible},
    },
    core::BOOL,
};

pub fn sources() -> Result<Vec<SourceInfo>> {
    let mut sources = Vec::<SourceInfo>::new();
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(monitor),
            LPARAM(&mut sources as *mut _ as isize),
        )
        .ok()?;
        EnumWindows(Some(window), LPARAM(&mut sources as *mut _ as isize))?;
    }
    Ok(sources)
}

unsafe extern "system" fn monitor(handle: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let sources = unsafe { &mut *(data.0 as *mut Vec<SourceInfo>) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool() {
        let width = (info.rcMonitor.right - info.rcMonitor.left) as u32;
        let height = (info.rcMonitor.bottom - info.rcMonitor.top) as u32;
        sources.push(SourceInfo {
            label: format!("Monitor {} · {width} × {height}", sources.len() + 1),
            source: CaptureSource::Monitor {
                index: sources.len() as i32,
                width,
                height,
                x: info.rcMonitor.left,
                y: info.rcMonitor.top,
            },
        });
    }
    BOOL(1)
}

unsafe extern "system" fn window(handle: HWND, data: LPARAM) -> BOOL {
    if !unsafe { IsWindowVisible(handle) }.as_bool() {
        return BOOL(1);
    }
    let mut text = [0u16; 1024];
    let length = unsafe { GetWindowTextW(handle, &mut text) };
    let mut rect = RECT::default();
    if length > 0 && unsafe { GetWindowRect(handle, &mut rect) }.is_ok() {
        let width = (rect.right - rect.left).max(0) as u32;
        let height = (rect.bottom - rect.top).max(0) as u32;
        if width >= 64 && height >= 64 {
            let sources = unsafe { &mut *(data.0 as *mut Vec<SourceInfo>) };
            sources.push(SourceInfo {
                label: format!(
                    "Window · {}",
                    String::from_utf16_lossy(&text[..length as usize])
                ),
                source: CaptureSource::Window {
                    id: handle.0 as u64,
                    width,
                    height,
                },
            });
        }
    }
    BOOL(1)
}

pub fn source(config: &crate::config::RecordingConfig) -> Result<gst::Element> {
    let source = element("d3d11screencapturesrc", "capture")?;
    source.set_property("show-cursor", config.settings.cursor);
    match config.source {
        CaptureSource::Monitor { index, .. } => {
            source.set_property("monitor-index", index);
            source.set_property_from_str(
                "capture-api",
                if config.settings.use_dxgi {
                    "dxgi"
                } else {
                    "wgc"
                },
            );
        }
        CaptureSource::Window { id, .. } => {
            source.set_property("window-handle", id);
            source.set_property_from_str("capture-api", "wgc");
        }
        _ => anyhow::bail!("Unsupported Windows capture source"),
    }
    Ok(source)
}
