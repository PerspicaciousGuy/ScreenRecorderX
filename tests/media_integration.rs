//! Run explicitly with GStreamer runtime plugins and ffmpeg/ffprobe installed:
//! cargo test --test media_integration -- --ignored --test-threads=1
use clearcast::{
    config::{CaptureSource, RecordingConfig, Settings},
    controller::{Command, Controller, Event},
    storage::Store,
};
use std::{
    process::{Command as Process, Stdio},
    time::{Duration, Instant},
};

fn probe(path: &std::path::Path) -> serde_json::Value {
    let result = Process::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

#[test]
#[ignore = "requires media runtime and software diagnostic encoder"]
fn synthetic_recording_pause_resume_and_mp4_are_valid() {
    clearcast::media::init().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().join("data")).unwrap();
    let controller = Controller::spawn(store.clone());
    let config = RecordingConfig {
        source: CaptureSource::TestPattern,
        region: None,
        diagnostic_software: true,
        settings: Settings {
            output_dir: temp.path().join("out"),
            system_audio: true,
            microphone: true,
            ..Default::default()
        },
    };
    controller.send(Command::Start(config)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut started = None;
    let mut paused = false;
    let mut resumed = false;
    let mut stopped = false;
    let saved = loop {
        assert!(Instant::now() < deadline, "Media worker timed out");
        if let Some(started) = started {
            let elapsed = Instant::now().duration_since(started);
            if !paused && elapsed.as_secs_f64() >= 1.0 {
                controller.send(Command::Pause).unwrap();
                paused = true;
            }
            if !resumed && elapsed.as_secs_f64() >= 2.0 {
                controller.send(Command::Resume).unwrap();
                resumed = true;
            }
            if !stopped && elapsed.as_secs_f64() >= 4.0 {
                controller.send(Command::Stop).unwrap();
                stopped = true;
            }
        }
        match controller.events.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Started { .. }) => started = Some(Instant::now()),
            Ok(Event::Saved(path)) => break path,
            Ok(Event::Error(error)) => panic!("{error}"),
            Ok(Event::Warning(warning)) => eprintln!("{warning}"),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => panic!("Worker crashed"),
            _ => {}
        }
    };
    let info = probe(&saved);
    let streams = info["streams"].as_array().unwrap();
    let video = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
    let audio = streams.iter().find(|s| s["codec_type"] == "audio").unwrap();
    assert_eq!(video["codec_name"], "h264");
    assert_eq!(video["width"], 1920);
    assert_eq!(video["height"], 1080);
    assert_eq!(video["r_frame_rate"], "60/1");
    assert_eq!(audio["codec_name"], "aac");
    assert_eq!(audio["sample_rate"], "48000");
    let video_duration: f64 = video["duration"].as_str().unwrap().parse().unwrap();
    let audio_duration: f64 = audio["duration"].as_str().unwrap().parse().unwrap();
    let video_start: f64 = video["start_time"].as_str().unwrap().parse().unwrap();
    let audio_start: f64 = audio["start_time"].as_str().unwrap().parse().unwrap();
    assert!(
        (2.5..3.8).contains(&video_duration),
        "Pause was not removed from timeline: {video_duration}"
    );
    assert!(
        ((video_start + video_duration) - (audio_start + audio_duration)).abs() < 0.05,
        "A/V end drift: video {video_start}+{video_duration}, audio {audio_start}+{audio_duration}"
    );
    assert!(store.pending().unwrap().is_empty());
    controller.send(Command::Shutdown).unwrap();
}

#[test]
#[ignore = "requires media runtime and ffmpeg"]
fn force_killed_process_recovers_recorded_clusters() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = Process::new(env!("CARGO_BIN_EXE_clearcast"))
        .args(["diagnose", "--software", "--seconds", "120", "--output"])
        .arg(temp.path().join("out"))
        .env("XDG_DATA_HOME", temp.path().join("data"))
        .env("LOCALAPPDATA", temp.path().join("data"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(4));
    assert!(
        child.try_wait().unwrap().is_none(),
        "Diagnostic exited before crash test"
    );
    child.kill().unwrap();
    child.wait().unwrap();
    let journals: Vec<_> = walk_json(&temp.path().join("data"));
    let journal = journals
        .iter()
        .find(|path| path.parent().unwrap().ends_with("pending"))
        .expect("Missing recovery journal");
    let pending: clearcast::storage::PendingRecording =
        serde_json::from_slice(&std::fs::read(journal).unwrap()).unwrap();
    let store = Store::new(journal.parent().unwrap().parent().unwrap().to_owned()).unwrap();
    let saved = store.finalize(&pending).unwrap();
    let info = probe(&saved);
    assert_eq!(info["streams"][0]["codec_name"], "h264");
    assert!(
        info["format"]["duration"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap()
            > 1.0
    );
}

fn walk_json(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walk_json(&path));
            } else if path.extension().is_some_and(|e| e == "json") {
                files.push(path);
            }
        }
    }
    files
}

#[test]
#[ignore = "requires a running X11 desktop (or Xvfb)"]
fn x11_capture_delivers_a_real_preview() {
    clearcast::media::init().unwrap();
    let source = clearcast::platform::sources()
        .unwrap()
        .into_iter()
        .find(|s| matches!(s.source, CaptureSource::Monitor { .. }))
        .unwrap()
        .source;
    let config = RecordingConfig {
        source,
        region: None,
        settings: Settings::default(),
        diagnostic_software: false,
    };
    let mut capture = clearcast::preview::PreparedCapture::new(&config).unwrap();
    let preview = capture.preview().unwrap();
    assert!(preview.width > 0 && preview.height > 0);
    assert_eq!(preview.rgba.len(), preview.width * preview.height * 4);
    assert!(capture.measured);
    let second = capture.preview().unwrap();
    assert_eq!(
        (preview.source_width, preview.source_height),
        (second.source_width, second.source_height)
    );
}

#[test]
#[ignore = "requires media runtime without a GPU encoder"]
fn absent_hardware_fails_without_leaving_empty_recovery_entries() {
    clearcast::media::init().unwrap();
    if !clearcast::media::hardware_encoders().is_empty() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().join("data")).unwrap();
    let controller = Controller::spawn(store.clone());
    controller
        .send(Command::Start(RecordingConfig {
            source: CaptureSource::TestPattern,
            region: None,
            diagnostic_software: false,
            settings: Settings {
                output_dir: temp.path().join("out"),
                ..Default::default()
            },
        }))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline);
        if let Ok(Event::Error(error)) = controller.events.recv_timeout(Duration::from_millis(100))
        {
            assert!(error.contains("No hardware H.264 encoder"));
            break;
        }
    }
    assert!(store.pending().unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(temp.path().join("out")).unwrap().count(),
        0
    );
    controller.send(Command::Shutdown).unwrap();
}
