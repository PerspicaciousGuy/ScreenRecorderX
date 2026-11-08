use crate::{
    config::{CaptureSource, RecordingConfig, output_size},
    platform,
    storage::PendingRecording,
};
use anyhow::{Context, Result, ensure};
use gstreamer::{self as gst, prelude::*};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
pub struct Counters {
    pub captured: AtomicU64,
    pub encoded: AtomicU64,
    pub overruns: AtomicU64,
}

pub struct MediaSession {
    pub pipeline: gst::Pipeline,
    pub counters: Arc<Counters>,
    pub encoder: String,
    pub dimensions: (u32, u32),
    audio_branches: Vec<gst::Bin>,
    _capture: crate::preview::PreparedCapture,
}

pub fn init() -> Result<()> {
    gst::init().context("Cannot initialize GStreamer")
}

fn numeric_property(element: &gst::Element, name: &str, value: u32) -> Result<()> {
    if let Some(spec) = element.find_property(name) {
        let value = value
            .to_value()
            .transform_with_type(spec.value_type())
            .with_context(|| format!("Cannot configure {name} on {}", element.name()))?;
        element.set_property_from_value(name, &value);
    }
    Ok(())
}

pub fn hardware_encoders() -> Vec<String> {
    let names: &[&str] = if cfg!(windows) {
        &["mfh264enc", "nvd3d11h264enc", "nvh264enc", "qsvh264enc"]
    } else {
        &["vah264enc", "vaapih264enc", "nvh264enc", "qsvh264enc"]
    };
    names
        .iter()
        .filter(|name| gst::ElementFactory::find(name).is_some())
        .map(|s| s.to_string())
        .collect()
}

fn queue(name: &str, video: bool) -> Result<gst::Element> {
    let queue = platform::element("queue", name)?;
    queue.set_property("max-size-buffers", if video { 3u32 } else { 128u32 });
    queue.set_property(
        "max-size-bytes",
        if video { 0u32 } else { 8 * 1024 * 1024u32 },
    );
    queue.set_property("max-size-time", if video { 0u64 } else { 250_000_000u64 });
    if video {
        queue.set_property_from_str("leaky", "downstream");
    }
    Ok(queue)
}

fn add_chain(pipeline: &gst::Pipeline, elements: &[gst::Element]) -> Result<()> {
    pipeline.add_many(elements)?;
    gst::Element::link_many(elements)
        .context("Cannot link media elements; check installed plugins and hardware support")?;
    Ok(())
}

fn filter(caps: gst::Caps, name: &str) -> Result<gst::Element> {
    Ok(gst::ElementFactory::make("capsfilter")
        .name(name)
        .property("caps", caps)
        .build()?)
}

impl MediaSession {
    pub fn build(config: &RecordingConfig, pending: &PendingRecording) -> Result<Self> {
        Self::build_prepared(config, pending, None)
    }

    pub fn build_prepared(
        config: &RecordingConfig,
        pending: &PendingRecording,
        prepared: Option<crate::preview::PreparedCapture>,
    ) -> Result<Self> {
        config.validate()?;
        let encoder_name = if config.diagnostic_software {
            ["openh264enc", "x264enc"]
                .into_iter()
                .find(|name| gst::ElementFactory::find(name).is_some())
                .context("Diagnostic requires openh264enc (or x264enc); no encoder installed")?
                .to_owned()
        } else {
            hardware_encoders().into_iter().next().context("No hardware H.264 encoder found. Install GPU drivers and GStreamer hardware plugins; run clearcast doctor")?
        };

        let mut capture = match prepared.filter(|p| {
            p.key == config.source
                && p.cursor == config.settings.cursor
                && p.dxgi == config.settings.use_dxgi
        }) {
            Some(prepared) => prepared,
            None => crate::preview::PreparedCapture::new(config)?,
        };
        if capture.key == CaptureSource::Portal && !capture.measured {
            capture.preview()?;
        }
        let (width, height) = capture.dimensions;
        if let Some(region) = config.region {
            region.validate(width, height)?;
        }
        let source_size = config
            .region
            .map(|r| (r.width, r.height))
            .unwrap_or((width, height));
        let output = output_size(source_size.0, source_size.1)?;
        let source = capture.source.clone();

        let pipeline = gst::Pipeline::with_name("clearcast-recording");
        // Fully construct before Playing. GStreamer supplies one clock and
        // adjusts base-time on pause/resume for all audio/video branches.
        let counters = Arc::new(Counters::default());
        let video_queue = queue("capture-queue", true)?;
        let count = counters.clone();
        video_queue.connect("overrun", false, move |_| {
            count.overruns.fetch_add(1, Ordering::Relaxed);
            None
        });
        let count = counters.clone();
        source
            .static_pad("src")
            .context("Capture has no source pad")?
            .add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                count.captured.fetch_add(1, Ordering::Relaxed);
                gst::PadProbeReturn::Ok
            });
        let mut chain = vec![source.clone(), video_queue];
        let windows_gpu = cfg!(windows)
            && config.source != CaptureSource::TestPattern
            && encoder_name == "mfh264enc";
        if let Some(region) = config.region {
            if windows_gpu {
                source.set_property("crop-x", region.x);
                source.set_property("crop-y", region.y);
                source.set_property("crop-width", region.width);
                source.set_property("crop-height", region.height);
            } else {
                let crop = platform::element("videocrop", "region")?;
                crop.set_property("left", region.x as i32);
                crop.set_property("top", region.y as i32);
                crop.set_property("right", (width - region.x - region.width) as i32);
                crop.set_property("bottom", (height - region.y - region.height) as i32);
                chain.push(crop);
            }
        }
        let rate = platform::element("videorate", "frame-pacer")?;
        rate.set_property("skip-to-first", true);
        chain.push(rate);
        let mut raw_caps = gst::Caps::builder("video/x-raw")
            .field(
                "format",
                if config.diagnostic_software {
                    "I420"
                } else {
                    "NV12"
                },
            )
            .field("width", output.0 as i32)
            .field("height", output.1 as i32)
            .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
            .field(
                "framerate",
                gst::Fraction::new(config.settings.fps as i32, 1),
            )
            .build();
        if windows_gpu {
            chain.push(platform::element("d3d11convert", "gpu-convert")?);
            raw_caps
                .get_mut()
                .unwrap()
                .set_features(0, Some(gst::CapsFeatures::new(["memory:D3D11Memory"])));
        } else if encoder_name == "vah264enc"
            && config.region.is_none()
            && gst::ElementFactory::find("vapostproc").is_some()
        {
            chain.push(platform::element("vapostproc", "gpu-convert")?);
            raw_caps
                .get_mut()
                .unwrap()
                .set_features(0, Some(gst::CapsFeatures::new(["memory:VAMemory"])));
        } else {
            // X11 and incompatible DMA-BUF paths need bounded staging; do not
            // describe this fallback as zero-copy.
            if cfg!(windows) && config.source != CaptureSource::TestPattern {
                chain.push(platform::element("d3d11download", "gpu-download")?);
            }
            chain.push(platform::element("videoconvert", "pixel-convert")?);
            let scale = platform::element("videoscale", "resize")?;
            scale.set_property("add-borders", true);
            chain.push(scale);
        }
        chain.push(filter(raw_caps, "output-format")?);
        let encoder = platform::element(&encoder_name, "video-encoder")?;
        let bitrate = config.settings.bitrate_mbps
            * if encoder_name == "openh264enc" {
                1_000_000
            } else {
                1000
            };
        numeric_property(&encoder, "bitrate", bitrate)?;
        numeric_property(&encoder, "gop-size", config.settings.fps * 2)?;
        if encoder.find_property("low-latency").is_some() {
            encoder.set_property("low-latency", true);
        }
        if encoder_name == "x264enc" {
            encoder.set_property_from_str("tune", "zerolatency");
            encoder.set_property_from_str("speed-preset", "ultrafast");
        }
        let count = counters.clone();
        encoder
            .static_pad("src")
            .context("Encoder has no output pad")?
            .add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                count.encoded.fetch_add(1, Ordering::Relaxed);
                gst::PadProbeReturn::Ok
            });
        chain.push(encoder);
        chain.push(platform::element("h264parse", "h264-parser")?);
        // Encoded packets must never be dropped: doing so corrupts H.264 GOPs.
        chain.push(queue("encoded-video", false)?);
        add_chain(&pipeline, &chain)?;
        let mux = platform::element("matroskamux", "recoverable-container")?;
        mux.set_property("streamable", true);
        mux.set_property("max-cluster-duration", 1_000_000_000i64);
        let sink = platform::element("filesink", "recording-file")?;
        sink.set_property(
            "location",
            pending
                .mkv
                .to_str()
                .context("Output path must be valid UTF-8")?,
        );
        sink.set_property("sync", false);
        sink.set_property_from_str("buffer-mode", "unbuffered");
        pipeline.add_many([&mux, &sink])?;
        mux.link(&sink)?;
        chain.last().unwrap().link(&mux)?;

        let mut audio_branches = Vec::new();
        if config.settings.system_audio || config.settings.microphone {
            let mixer = platform::element("audiomixer", "audio-mixer")?;
            mixer.set_property("ignore-inactive-pads", true);
            let convert = platform::element("audioconvert", "mix-convert")?;
            let resample = platform::element("audioresample", "mix-resample")?;
            let caps = filter(
                gst::Caps::builder("audio/x-raw")
                    .field("rate", 48000i32)
                    .field("channels", 2i32)
                    .build(),
                "mix-format",
            )?;
            let aac_names = if cfg!(windows) {
                ["mfaacenc", "voaacenc", "avenc_aac"]
            } else {
                ["voaacenc", "avenc_aac", "mfaacenc"]
            };
            let aac_name = aac_names
                .into_iter()
                .find(|name| gst::ElementFactory::find(name).is_some())
                .context("Missing AAC encoder: install GStreamer libav or voaacenc")?;
            let aac = platform::element(aac_name, "audio-encoder")?;
            numeric_property(&aac, "bitrate", 192000)?;
            let aac_parse = platform::element("aacparse", "aac-parser")?;
            let encoded = queue("encoded-audio", false)?;
            add_chain(
                &pipeline,
                &[
                    mixer.clone(),
                    convert,
                    resample,
                    caps,
                    aac,
                    aac_parse,
                    encoded.clone(),
                ],
            )?;
            encoded.link(&mux)?;
            for (system, enabled, device, volume) in [
                (
                    true,
                    config.settings.system_audio,
                    config.settings.system_device.as_deref(),
                    config.settings.system_volume,
                ),
                (
                    false,
                    config.settings.microphone,
                    config.settings.microphone_device.as_deref(),
                    config.settings.microphone_volume,
                ),
            ] {
                if !enabled {
                    continue;
                }
                let name = if system { "system-audio" } else { "microphone" };
                let input = if config.source == CaptureSource::TestPattern {
                    let input = platform::element("audiotestsrc", name)?;
                    input.set_property("is-live", true);
                    input.set_property("freq", if system { 440.0 } else { 880.0 });
                    input
                } else {
                    platform::audio_source(system, device, name)?
                };
                let bin = gst::Bin::with_name(&format!("{name}-branch"));
                let convert = platform::element("audioconvert", &format!("{name}-convert"))?;
                let resample = platform::element("audioresample", &format!("{name}-resample"))?;
                let level = platform::element("volume", &format!("{name}-volume"))?;
                level.set_property("volume", volume);
                let queue = queue(&format!("{name}-queue"), false)?;
                bin.add_many([&input, &convert, &resample, &level, &queue])?;
                gst::Element::link_many([&input, &convert, &resample, &level, &queue])?;
                let ghost = gst::GhostPad::with_target(&queue.static_pad("src").unwrap())?;
                bin.add_pad(&ghost)?;
                pipeline.add(&bin)?;
                bin.link(&mixer)?;
                audio_branches.push(bin);
            }
        }
        Ok(Self {
            pipeline,
            counters,
            encoder: encoder_name,
            dimensions: output,
            audio_branches,
            _capture: capture,
        })
    }

    /// Detach a failed audio branch; keep video and other audio inputs running.
    pub fn detach_failed_audio(&mut self, source: &gst::Object) -> Result<Option<String>> {
        let Some(index) = self.audio_branches.iter().position(|branch| {
            source == branch.upcast_ref::<gst::Object>() || source.has_as_ancestor(branch)
        }) else {
            return Ok(None);
        };
        let branch = self.audio_branches.remove(index);
        let name = branch.name().to_string();
        let pad = branch.static_pad("src").unwrap();
        if let Some(peer) = pad.peer() {
            // EOS only this mixer input so the mix and finalization can proceed.
            peer.send_event(gst::event::Eos::new());
            let _ = pad.unlink(&peer);
            if let Some(mixer) = peer.parent_element() {
                mixer.release_request_pad(&peer);
            }
        }
        branch.set_state(gst::State::Null)?;
        self.pipeline.remove(&branch)?;
        Ok(Some(format!(
            "{name} disconnected; recording continues with remaining audio"
        )))
    }

    pub fn play(&self) -> Result<()> {
        self.pipeline
            .set_state(gst::State::Playing)
            .context("Cannot start capture/encoder")?;
        Ok(())
    }
    pub fn pause(&self) -> Result<()> {
        self.pipeline.set_state(gst::State::Paused)?;
        Ok(())
    }
    pub fn resume(&self) -> Result<()> {
        self.play()
    }
    pub fn eos(&self) -> Result<()> {
        // A paused pipeline must resume so queued data and EOS can reach muxer.
        self.pipeline.set_state(gst::State::Playing)?;
        ensure!(
            self.pipeline.send_event(gst::event::Eos::new()),
            "Cannot send end-of-stream"
        );
        Ok(())
    }
}

impl Drop for MediaSession {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
