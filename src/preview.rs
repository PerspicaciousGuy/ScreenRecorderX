use crate::{
    config::{CaptureSource, RecordingConfig},
    platform,
};
use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};

pub struct PreparedCapture {
    pub source: gst::Element,
    pub dimensions: (u32, u32),
    pub key: CaptureSource,
    pub cursor: bool,
    pub dxgi: bool,
    pub measured: bool,
    #[cfg(all(target_os = "linux", feature = "portal"))]
    portal: Option<platform::portal::PortalCapture>,
    #[cfg(all(target_os = "linux", feature = "portal"))]
    runtime: Option<tokio::runtime::Runtime>,
}

#[derive(Debug, Clone)]
pub struct Preview {
    pub width: usize,
    pub height: usize,
    pub source_width: u32,
    pub source_height: u32,
    pub rgba: Vec<u8>,
}

impl PreparedCapture {
    pub fn new(config: &RecordingConfig) -> Result<Self> {
        config.validate()?;
        #[cfg(all(target_os = "linux", feature = "portal"))]
        let (portal, runtime) = if config.source == CaptureSource::Portal {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()?;
            let portal = runtime
                .block_on(async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(180),
                        platform::portal::PortalCapture::select(
                            config.settings.cursor,
                            config.region.is_some(),
                        ),
                    )
                    .await
                })
                .context("Screen selection timed out")??;
            (Some(portal), Some(runtime))
        } else {
            (None, None)
        };
        let dimensions = config.source.dimensions();
        #[cfg(all(target_os = "linux", feature = "portal"))]
        let dimensions = dimensions.or_else(|| portal.as_ref().map(|p| p.size));
        let dimensions = dimensions
            .context("Source dimensions unavailable; rebuild with portal support on Wayland")?;
        let source;
        #[cfg(all(target_os = "linux", feature = "portal"))]
        {
            source = if let Some(portal) = &portal {
                portal.source()?
            } else {
                platform::source(config)?
            };
        }
        #[cfg(not(all(target_os = "linux", feature = "portal")))]
        {
            source = platform::source(config)?;
        }
        Ok(Self {
            source,
            dimensions,
            key: config.source.clone(),
            cursor: config.settings.cursor,
            dxgi: config.settings.use_dxgi,
            measured: false,
            #[cfg(all(target_os = "linux", feature = "portal"))]
            portal,
            #[cfg(all(target_os = "linux", feature = "portal"))]
            runtime,
        })
    }

    pub fn preview(&mut self) -> Result<Preview> {
        let pipeline = gst::Pipeline::with_name("source-preview");
        let result = (|| -> Result<Preview> {
            let mut chain = vec![self.source.clone()];
            if cfg!(windows) && self.key != CaptureSource::TestPattern {
                chain.push(platform::element("d3d11download", "preview-download")?);
            }
            chain.push(platform::element("videoconvert", "preview-convert")?);
            chain.push(platform::element("videoscale", "preview-scale")?);
            let scale = (960.0 / self.dimensions.0 as f64)
                .min(540.0 / self.dimensions.1 as f64)
                .min(1.0);
            let width = (self.dimensions.0 as f64 * scale) as i32;
            let height = (self.dimensions.1 as f64 * scale) as i32;
            let caps = gst::ElementFactory::make("capsfilter")
                .property(
                    "caps",
                    gst::Caps::builder("video/x-raw")
                        .field("format", "RGBA")
                        .field("width", width)
                        .field("height", height)
                        .build(),
                )
                .build()?;
            chain.push(caps);
            let sink = platform::element("fakesink", "preview-sink")?;
            sink.set_property("sync", false);
            sink.set_property("enable-last-sample", true);
            chain.push(sink.clone());
            pipeline.add_many(&chain)?;
            gst::Element::link_many(&chain)?;
            pipeline.set_state(gst::State::Playing)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let bus = pipeline.bus().unwrap();
            while std::time::Instant::now() < deadline {
                if let Some(sample) = sink.property::<Option<gst::Sample>>("last-sample") {
                    // Portal geometry is in compositor coordinates. Negotiate
                    // actual pixel dimensions before translating crop positions.
                    if let Some(caps) = self
                        .source
                        .static_pad("src")
                        .and_then(|pad| pad.current_caps())
                        && let Some(structure) = caps.structure(0)
                        && let (Ok(width), Ok(height)) = (
                            structure.get::<i32>("width"),
                            structure.get::<i32>("height"),
                        )
                        && width > 0
                        && height > 0
                    {
                        self.dimensions = (width as u32, height as u32);
                        self.measured = true;
                    }
                    let info = gstreamer_video::VideoInfo::from_caps(
                        sample.caps().context("Preview has no format")?,
                    )?;
                    let map = sample
                        .buffer()
                        .context("Preview is empty")?
                        .map_readable()?;
                    let stride = info.stride()[0] as usize;
                    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
                    for row in 0..height as usize {
                        let offset = row * stride;
                        rgba.extend_from_slice(
                            map.get(offset..offset + width as usize * 4)
                                .context("Invalid preview stride")?,
                        );
                    }
                    return Ok(Preview {
                        width: width as usize,
                        height: height as usize,
                        source_width: self.dimensions.0,
                        source_height: self.dimensions.1,
                        rgba,
                    });
                }
                if let Some(message) = bus.timed_pop(gst::ClockTime::from_mseconds(50))
                    && let gst::MessageView::Error(error) = message.view()
                {
                    anyhow::bail!("Preview failed: {}", error.error());
                }
            }
            anyhow::bail!("No preview frames received within 10 seconds")
        })();
        let _ = pipeline.set_state(gst::State::Null);
        if self.source.parent().is_some() {
            let _ = pipeline.remove(&self.source);
        }
        result
    }
}

#[cfg(all(target_os = "linux", feature = "portal"))]
impl Drop for PreparedCapture {
    fn drop(&mut self) {
        if let (Some(portal), Some(runtime)) = (&self.portal, &self.runtime) {
            runtime.block_on(portal.close());
        }
    }
}
