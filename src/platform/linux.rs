use super::*;
use anyhow::ensure;
use x11rb::{
    connection::Connection,
    protocol::{
        randr::ConnectionExt as _,
        xproto::{AtomEnum, ConnectionExt as _},
    },
};

pub fn sources() -> Result<Vec<SourceInfo>> {
    let (connection, screen) = x11rb::connect(None)
        .context("Cannot connect to X11. Under Wayland use the portal capture source")?;
    let root = &connection.setup().roots[screen];
    let mut sources = Vec::new();
    if let Ok(reply) = connection.randr_get_monitors(root.root, true)?.reply() {
        for (index, monitor) in reply.monitors.iter().enumerate() {
            let label = connection
                .get_atom_name(monitor.name)?
                .reply()
                .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                .unwrap_or_else(|_| format!("Monitor {}", index + 1));
            sources.push(SourceInfo {
                label: format!("{label} · {} × {}", monitor.width, monitor.height),
                source: CaptureSource::Monitor {
                    index: index as i32,
                    width: monitor.width.into(),
                    height: monitor.height.into(),
                    x: monitor.x.into(),
                    y: monitor.y.into(),
                },
            });
        }
    }
    if sources.is_empty() {
        sources.push(SourceInfo {
            label: "Desktop".into(),
            source: CaptureSource::Monitor {
                index: 0,
                width: root.width_in_pixels.into(),
                height: root.height_in_pixels.into(),
                x: 0,
                y: 0,
            },
        });
    }
    let client_list = connection
        .intern_atom(false, b"_NET_CLIENT_LIST")?
        .reply()?
        .atom;
    let name_atom = connection
        .intern_atom(false, b"_NET_WM_NAME")?
        .reply()?
        .atom;
    let utf8 = connection.intern_atom(false, b"UTF8_STRING")?.reply()?.atom;
    if let Ok(reply) = connection
        .get_property(false, root.root, client_list, AtomEnum::WINDOW, 0, 4096)?
        .reply()
        && let Some(windows) = reply.value32()
    {
        for window in windows {
            let Ok(geometry) = connection.get_geometry(window)?.reply() else {
                continue;
            };
            if geometry.width < 64 || geometry.height < 64 {
                continue;
            }
            let name = connection
                .get_property(false, window, name_atom, utf8, 0, 1024)?
                .reply()
                .map(|r| String::from_utf8_lossy(&r.value).into_owned())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            sources.push(SourceInfo {
                label: format!("Window · {name}"),
                source: CaptureSource::Window {
                    id: window.into(),
                    width: geometry.width.into(),
                    height: geometry.height.into(),
                },
            });
        }
    }
    Ok(sources)
}

pub fn source(config: &crate::config::RecordingConfig) -> Result<gst::Element> {
    ensure!(
        config.source != CaptureSource::Portal,
        "Wayland capture must be selected through the screen-sharing portal"
    );
    let source = element("ximagesrc", "capture")?;
    source.set_property("use-damage", false);
    source.set_property("show-pointer", config.settings.cursor);
    match config.source {
        CaptureSource::Monitor {
            x,
            y,
            width,
            height,
            ..
        } => {
            ensure!(
                x >= 0 && y >= 0,
                "This X11 layout has negative coordinates; choose a window or rearrange monitors"
            );
            source.set_property("startx", x as u32);
            source.set_property("starty", y as u32);
            source.set_property("endx", x as u32 + width - 1);
            source.set_property("endy", y as u32 + height - 1);
        }
        CaptureSource::Window { id, .. } => source.set_property("xid", id),
        _ => anyhow::bail!("Unsupported X11 capture source"),
    }
    Ok(source)
}
