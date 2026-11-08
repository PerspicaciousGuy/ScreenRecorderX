use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CaptureSource {
    Monitor {
        index: i32,
        width: u32,
        height: u32,
        x: i32,
        y: i32,
    },
    Window {
        id: u64,
        width: u32,
        height: u32,
    },
    Portal,
    /// Only used by the explicit diagnostic command, never as a fallback.
    TestPattern,
}

impl CaptureSource {
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        match self {
            Self::Monitor { width, height, .. } | Self::Window { width, height, .. } => {
                Some((*width, *height))
            }
            Self::Portal => None,
            Self::TestPattern => Some((1920, 1080)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceInfo {
    pub label: String,
    pub source: CaptureSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub fn validate(self, width: u32, height: u32) -> Result<()> {
        ensure!(
            self.width >= 64 && self.height >= 64,
            "Region must be at least 64 × 64 pixels"
        );
        ensure!(
            self.x.checked_add(self.width).is_some_and(|v| v <= width)
                && self.y.checked_add(self.height).is_some_and(|v| v <= height),
            "Region extends outside the selected source"
        );
        Ok(())
    }
}

/// Fit without upscaling; H.264 chroma subsampling requires even dimensions.
pub fn output_size(width: u32, height: u32) -> Result<(u32, u32)> {
    ensure!(
        width >= 64 && height >= 64 && width <= 32768 && height <= 32768,
        "Unsupported source dimensions: {width} × {height}"
    );
    let scale = (1920.0 / width as f64).min(1080.0 / height as f64).min(1.0);
    Ok((
        ((width as f64 * scale) as u32 / 2 * 2).max(2),
        ((height as f64 * scale) as u32 / 2 * 2).max(2),
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub fps: u32,
    pub bitrate_mbps: u32,
    pub cursor: bool,
    pub system_audio: bool,
    pub microphone: bool,
    pub system_volume: f64,
    pub microphone_volume: f64,
    pub system_device: Option<String>,
    pub microphone_device: Option<String>,
    pub output_dir: PathBuf,
    pub record_hotkey: String,
    pub pause_hotkey: String,
    pub use_dxgi: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let output_dir = directories::UserDirs::new()
            .and_then(|d| d.video_dir().map(|p| p.join("Clearcast")))
            .unwrap_or_else(|| PathBuf::from("recordings"));
        Self {
            fps: 60,
            bitrate_mbps: 16,
            cursor: true,
            system_audio: true,
            microphone: false,
            system_volume: 1.0,
            microphone_volume: 1.0,
            system_device: None,
            microphone_device: None,
            output_dir,
            record_hotkey: "Ctrl+Shift+F9".into(),
            pause_hotkey: "Ctrl+Shift+F10".into(),
            use_dxgi: false,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(matches!(self.fps, 30 | 60), "Choose 30 or 60 fps");
        ensure!(
            (1..=100).contains(&self.bitrate_mbps),
            "Bitrate must be between 1 and 100 Mbps"
        );
        ensure!(
            self.system_volume.is_finite() && (0.0..=2.0).contains(&self.system_volume),
            "Invalid system volume"
        );
        ensure!(
            self.microphone_volume.is_finite() && (0.0..=2.0).contains(&self.microphone_volume),
            "Invalid microphone volume"
        );
        ensure!(
            !self.output_dir.as_os_str().is_empty(),
            "Choose an output directory"
        );
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct RecordingConfig {
    pub source: CaptureSource,
    pub region: Option<Region>,
    pub settings: Settings,
    pub diagnostic_software: bool,
}

impl RecordingConfig {
    pub fn validate(&self) -> Result<()> {
        self.settings.validate()?;
        ensure!(
            !self.diagnostic_software || self.source == CaptureSource::TestPattern,
            "Software encoding is restricted to the synthetic diagnostic command"
        );
        if let Some((width, height)) = self.source.dimensions() {
            output_size(width, height)?;
            if let Some(region) = self.region {
                region.validate(width, height)?;
            }
        }
        ensure!(
            self.region.is_none() || !matches!(self.source, CaptureSource::Window { .. }),
            "Choose a monitor to record a region"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderState {
    Idle,
    Starting,
    Recording,
    Paused,
    Finalizing,
    Error,
}

impl RecorderState {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Recording | Self::Paused | Self::Finalizing
        )
    }
}
