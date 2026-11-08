use crate::config::{CaptureSource, SourceInfo};
use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(all(target_os = "linux", feature = "portal"))]
pub mod portal;
pub mod shortcuts;
#[cfg(windows)]
mod windows;

pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland")
            || std::env::var_os("WAYLAND_DISPLAY").is_some())
}

pub fn sources() -> Result<Vec<SourceInfo>> {
    if is_wayland() {
        return Ok(vec![SourceInfo {
            label: "Choose a screen or window…".into(),
            source: CaptureSource::Portal,
        }]);
    }
    #[cfg(target_os = "linux")]
    {
        linux::sources()
    }
    #[cfg(windows)]
    {
        windows::sources()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    anyhow::bail!("Only Windows and Linux are supported")
}

pub fn element(factory: &str, name: &str) -> Result<gst::Element> {
    gst::ElementFactory::make(factory)
        .name(name)
        .build()
        .with_context(|| {
            format!("Missing GStreamer element '{factory}'. See docs/BUILD.md for runtime packages")
        })
}

#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub label: String,
    pub id: String,
}

pub fn audio_devices() -> Vec<AudioDevice> {
    let monitor = gst::DeviceMonitor::new();
    monitor.add_filter(Some("Audio/Source"), None);
    let _ = monitor.start();
    let devices = monitor
        .devices()
        .iter()
        .filter_map(|device| {
            let properties = device.properties()?;
            let id = ["device.id", "device.name", "device.str", "object.path"]
                .iter()
                .find_map(|key| properties.get::<String>(*key).ok())?;
            Some(AudioDevice {
                label: device.display_name().to_string(),
                id,
            })
        })
        .collect();
    monitor.stop();
    devices
}

pub fn audio_source(system: bool, device: Option<&str>, name: &str) -> Result<gst::Element> {
    #[cfg(windows)]
    {
        let source = element("wasapi2src", name)?;
        source.set_property("loopback", system);
        if let Some(device) = device {
            source.set_property("device", device);
        }
        Ok(source)
    }
    #[cfg(target_os = "linux")]
    {
        // PipeWire's PulseAudio compatibility server exposes sink monitors
        // without requiring users to install or run a second audio server.
        let source = element("pulsesrc", name)?;
        if let Some(device) = device {
            source.set_property("device", device);
        } else if system {
            // PulseAudio resolves this symbolic name against the default sink.
            source.set_property("device", "@DEFAULT_MONITOR@");
        }
        Ok(source)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    anyhow::bail!("Unsupported OS")
}

pub fn source(config: &crate::config::RecordingConfig) -> Result<gst::Element> {
    if config.source == CaptureSource::TestPattern {
        let source = element("videotestsrc", "capture")?;
        source.set_property("is-live", true);
        source.set_property_from_str("pattern", "ball");
        return Ok(source);
    }
    #[cfg(target_os = "linux")]
    {
        linux::source(config)
    }
    #[cfg(windows)]
    {
        windows::source(config)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    anyhow::bail!("Unsupported OS")
}
