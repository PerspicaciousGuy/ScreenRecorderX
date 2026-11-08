#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use clearcast::{
    config::{CaptureSource, RecordingConfig, Settings},
    controller::{Command, Controller, Event},
    media,
    storage::Store,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(
    version,
    about = "Hardware-accelerated screen recording for Windows and Linux"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Action>,
}

#[derive(Subcommand)]
enum Action {
    /// Inspect capture, encoder, and runtime availability without recording.
    Doctor,
    /// List recoverable recordings; with --all, recover them to MP4.
    Recover {
        #[arg(long)]
        all: bool,
    },
    /// Record a real display using hardware encoding.
    Record {
        #[arg(long, default_value_t = 0)]
        monitor: usize,
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=86400))]
        seconds: u64,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        no_audio: bool,
        #[arg(long)]
        dxgi: bool,
    },
    /// Explicit synthetic test. --software never applies to desktop recording.
    Diagnose {
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=3600))]
        seconds: u64,
        #[arg(long)]
        software: bool,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        pause: bool,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();
    media::init()?;
    let store = Store::system()?;
    match args.command {
        Some(Action::Doctor) => doctor(),
        Some(Action::Recover { all }) => {
            let _lock = store.lock()?;
            for recording in store.pending()? {
                println!(
                    "{}  {}  {}",
                    recording.id,
                    recording.started,
                    recording.mkv.display()
                );
                if all {
                    println!("Saved {}", store.finalize(&recording)?.display());
                }
            }
            Ok(())
        }
        Some(Action::Diagnose {
            seconds,
            software,
            output,
            pause,
        }) => {
            let _lock = store.lock()?;
            let mut settings = Settings {
                system_audio: true,
                microphone: true,
                ..Settings::default()
            };
            if let Some(output) = output {
                settings.output_dir = output;
            }
            timed_record(
                store,
                RecordingConfig {
                    source: CaptureSource::TestPattern,
                    region: None,
                    settings,
                    diagnostic_software: software,
                },
                seconds,
                pause,
            )
        }
        Some(Action::Record {
            monitor,
            seconds,
            output,
            no_audio,
            dxgi,
        }) => {
            let _lock = store.lock()?;
            let source = clearcast::platform::sources()?
                .into_iter()
                .filter(|s| {
                    matches!(
                        s.source,
                        CaptureSource::Monitor { .. } | CaptureSource::Portal
                    )
                })
                .nth(monitor)
                .context("Monitor index not found; run clearcast doctor")?
                .source;
            let mut settings = store.load_settings()?;
            if let Some(output) = output {
                settings.output_dir = output;
            }
            settings.system_audio = !no_audio;
            settings.use_dxgi = dxgi;
            timed_record(
                store,
                RecordingConfig {
                    source,
                    region: None,
                    settings,
                    diagnostic_software: false,
                },
                seconds,
                false,
            )
        }
        None => {
            let _lock = store.lock()?;
            clearcast::ui::run(store)
        }
    }
}

fn doctor() -> Result<()> {
    println!(
        "Clearcast {} | {} {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("GStreamer: {}", gstreamer::version_string());
    println!("Wayland session: {}", clearcast::platform::is_wayland());
    println!("Hardware H.264 encoders: {:?}", media::hardware_encoders());
    for name in [
        "d3d11screencapturesrc",
        "pipewiresrc",
        "ximagesrc",
        "pulsesrc",
        "wasapi2src",
        "matroskamux",
        "h264parse",
        "avenc_aac",
        "voaacenc",
        "mfaacenc",
    ] {
        println!(
            "  {name}: {}",
            if gstreamer::ElementFactory::find(name).is_some() {
                "available"
            } else {
                "missing"
            }
        );
    }
    match clearcast::platform::sources() {
        Ok(sources) => {
            for source in sources {
                println!("Source: {}", source.label);
            }
        }
        Err(error) => println!("Capture: {error:#}"),
    }
    let ffmpeg = clearcast::storage::ffmpeg_path();
    let available = std::process::Command::new(&ffmpeg)
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    println!(
        "FFmpeg: {} ({})",
        ffmpeg.display(),
        if available { "available" } else { "missing" }
    );
    Ok(())
}

fn timed_record(
    store: Store,
    config: RecordingConfig,
    seconds: u64,
    test_pause: bool,
) -> Result<()> {
    let controller = Controller::spawn(store);
    controller.send(Command::Start(config))?;
    let mut started = None;
    let mut stopped = false;
    let mut paused = false;
    let mut resumed = false;
    let deadline = Instant::now() + Duration::from_secs(seconds + 240);
    loop {
        anyhow::ensure!(
            Instant::now() < deadline,
            "Recording timed out; check recovery journal"
        );
        if let Some(started) = started {
            let elapsed = Instant::now().duration_since(started);
            if test_pause && !paused && elapsed >= Duration::from_secs(seconds / 2) {
                controller.send(Command::Pause)?;
                paused = true;
            }
            if paused && !resumed && elapsed >= Duration::from_secs(seconds / 2 + 1) {
                controller.send(Command::Resume)?;
                resumed = true;
            }
            if !stopped && elapsed >= Duration::from_secs(seconds + if test_pause { 1 } else { 0 })
            {
                controller.send(Command::Stop)?;
                stopped = true;
            }
        }
        match controller.events.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Started {
                encoder,
                width,
                height,
            }) => {
                println!("Recording {width} × {height} with {encoder}");
                started = Some(Instant::now());
            }
            Ok(Event::Saved(path)) => {
                println!("Saved {}", path.display());
                return Ok(());
            }
            Ok(Event::Error(error)) => anyhow::bail!(error),
            Ok(Event::Warning(warning)) => eprintln!("Warning: {warning}"),
            Ok(Event::Statistics(stats)) => println!(
                "{:.1}s | {} frames encoded | {} queue overruns | {:.1} MiB",
                stats.elapsed.as_secs_f64(),
                stats.encoded,
                stats.queue_overruns,
                stats.bytes as f64 / 1_048_576.0
            ),
            Ok(Event::State(state)) => println!("{state:?}"),
            Ok(Event::Preview(_)) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("Recording engine exited")
            }
            Err(_) => {}
        }
    }
}
