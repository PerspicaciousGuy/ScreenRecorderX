use clearcast::{
    config::{CaptureSource, RecordingConfig, Region, Settings, output_size},
    storage::Store,
};
use std::fs;

#[test]
fn output_preserves_portrait_and_never_upscales() {
    assert_eq!(output_size(3840, 2160).unwrap(), (1920, 1080));
    assert_eq!(output_size(1080, 1920).unwrap(), (606, 1080));
    assert_eq!(output_size(801, 601).unwrap(), (800, 600));
    assert!(output_size(0, 1080).is_err());
    assert!(output_size(u32::MAX, 1080).is_err());
}

#[test]
fn crop_rejects_overflow_and_cross_monitor_regions() {
    assert!(
        Region {
            x: u32::MAX,
            y: 0,
            width: 100,
            height: 100
        }
        .validate(1920, 1080)
        .is_err()
    );
    assert!(
        Region {
            x: 1900,
            y: 0,
            width: 100,
            height: 100
        }
        .validate(1920, 1080)
        .is_err()
    );
    assert!(
        Region {
            x: 20,
            y: 20,
            width: 640,
            height: 480
        }
        .validate(1920, 1080)
        .is_ok()
    );
}

#[test]
fn invalid_audio_settings_and_software_fallback_are_rejected() {
    let settings = Settings {
        system_volume: f64::NAN,
        ..Default::default()
    };
    assert!(settings.validate().is_err());
    let config = RecordingConfig {
        source: CaptureSource::Monitor {
            index: 0,
            width: 1920,
            height: 1080,
            x: 0,
            y: 0,
        },
        region: None,
        settings: Settings::default(),
        diagnostic_software: true,
    };
    assert!(config.validate().is_err());
}

#[test]
fn recovery_survives_restart_and_malformed_neighbor() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let store = Store::new(root.clone()).unwrap();
    let pending = store.begin(&temp.path().join("recordings")).unwrap();
    fs::write(&pending.mkv, b"unfinished recording").unwrap();
    fs::write(root.join("pending/broken.json"), b"{").unwrap();
    drop(store);
    let reopened = Store::new(root).unwrap();
    assert_eq!(reopened.pending().unwrap()[0].id, pending.id);
    reopened.forget_empty(&pending).unwrap();
    assert!(
        pending.mkv.exists(),
        "Nonempty recordings must never be discarded"
    );
}

#[test]
fn recovery_does_not_overwrite_existing_destination() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().join("data")).unwrap();
    let pending = store.begin(&temp.path().join("out")).unwrap();
    fs::write(&pending.mkv, b"source").unwrap();
    fs::write(&pending.mp4, b"existing user file").unwrap();
    assert!(store.finalize(&pending).is_err());
    assert_eq!(fs::read(&pending.mp4).unwrap(), b"existing user file");
    assert_eq!(fs::read(&pending.mkv).unwrap(), b"source");
    assert!(store.journal(pending.id).exists());
}

#[test]
fn instance_lock_prevents_concurrent_journal_writers() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().to_owned()).unwrap();
    let guard = store.lock().unwrap();
    assert!(store.lock().is_err());
    drop(guard);
    assert!(store.lock().is_ok());
}

#[test]
fn settings_replace_is_atomic_and_round_trips() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().to_owned()).unwrap();
    store.save_settings(&Settings::default()).unwrap();
    let changed = Settings {
        fps: 30,
        microphone: true,
        ..Settings::default()
    };
    store.save_settings(&changed).unwrap();
    let restored = store.load_settings().unwrap();
    assert_eq!(restored.fps, 30);
    assert!(restored.microphone);
}
