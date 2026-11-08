use crate::{
    config::{RecorderState, RecordingConfig},
    media::MediaSession,
    storage::{PendingRecording, Store},
};
use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, bounded};
use gstreamer::{self as gst, prelude::*};
use std::{
    path::PathBuf,
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub enum Command {
    Start(RecordingConfig),
    Preview(RecordingConfig),
    ReleasePreview,
    Pause,
    Resume,
    Stop,
    Recover(PendingRecording),
    Shutdown,
}

#[derive(Debug, Clone, Default)]
pub struct Statistics {
    pub elapsed: Duration,
    pub captured: u64,
    pub encoded: u64,
    pub queue_overruns: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub enum Event {
    State(RecorderState),
    Started {
        encoder: String,
        width: u32,
        height: u32,
    },
    Statistics(Statistics),
    Preview(crate::preview::Preview),
    Warning(String),
    Saved(PathBuf),
    Error(String),
}

pub struct Controller {
    pub commands: Sender<Command>,
    pub events: Receiver<Event>,
}

struct Active {
    media: MediaSession,
    pending: PendingRecording,
    since: Instant,
    elapsed: Duration,
    paused: bool,
}

impl Controller {
    pub fn spawn(store: Store) -> Self {
        let (commands, receiver) = bounded(8);
        let (sender, events) = bounded(64);
        thread::Builder::new()
            .name("recording-engine".into())
            .spawn(move || worker(store, receiver, sender))
            .expect("Cannot create recording thread");
        Self { commands, events }
    }
    pub fn send(&self, command: Command) -> Result<()> {
        self.commands
            .try_send(command)
            .map_err(|_| anyhow::anyhow!("Recording engine is busy or has stopped"))
    }
}

fn emit(events: &Sender<Event>, event: Event) {
    let _ = events.send(event);
}

fn worker(store: Store, commands: Receiver<Command>, events: Sender<Event>) {
    let mut active: Option<Active> = None;
    let mut prepared: Option<crate::preview::PreparedCapture> = None;
    let mut last_stats = Instant::now();
    let mut last_disk = Instant::now();
    loop {
        let command = match commands.recv_timeout(Duration::from_millis(25)) {
            Ok(command) => Some(command),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => None,
            Err(_) => Some(Command::Shutdown),
        };
        match command {
            Some(Command::Start(config)) if active.is_none() => {
                emit(&events, Event::State(RecorderState::Starting));
                let mut pending = None;
                let result = (|| -> Result<Active> {
                    config.validate()?;
                    let recording = store.begin(&config.settings.output_dir)?;
                    pending = Some(recording.clone());
                    let media = MediaSession::build_prepared(&config, &recording, prepared.take())?;
                    media.play()?;
                    emit(
                        &events,
                        Event::Started {
                            encoder: media.encoder.clone(),
                            width: media.dimensions.0,
                            height: media.dimensions.1,
                        },
                    );
                    Ok(Active {
                        media,
                        pending: recording,
                        since: Instant::now(),
                        elapsed: Duration::ZERO,
                        paused: false,
                    })
                })();
                match result {
                    Ok(recording) => {
                        active = Some(recording);
                        emit(&events, Event::State(RecorderState::Recording));
                    }
                    Err(error) => {
                        if let Some(pending) = pending {
                            let _ = store.forget_empty(&pending);
                        }
                        emit(&events, Event::Error(format!("{error:#}")));
                        emit(&events, Event::State(RecorderState::Error));
                    }
                }
            }
            Some(Command::Preview(config)) if active.is_none() => {
                emit(&events, Event::State(RecorderState::Starting));
                prepared = None;
                match crate::preview::PreparedCapture::new(&config).and_then(|mut capture| {
                    let preview = capture.preview()?;
                    Ok((capture, preview))
                }) {
                    Ok((capture, preview)) => {
                        prepared = Some(capture);
                        emit(&events, Event::Preview(preview));
                        emit(&events, Event::State(RecorderState::Idle));
                    }
                    Err(error) => {
                        emit(&events, Event::Error(format!("{error:#}")));
                        emit(&events, Event::State(RecorderState::Error));
                    }
                }
            }
            Some(Command::ReleasePreview) => {
                prepared = None;
            }
            Some(Command::Pause) => {
                if let Some(active) = active.as_mut().filter(|a| !a.paused) {
                    match active.media.pause() {
                        Ok(()) => {
                            active.elapsed += active.since.elapsed();
                            active.paused = true;
                            emit(&events, Event::State(RecorderState::Paused));
                        }
                        Err(error) => emit(&events, Event::Warning(error.to_string())),
                    }
                }
            }
            Some(Command::Resume) => {
                if let Some(active) = active.as_mut().filter(|a| a.paused) {
                    match active.media.resume() {
                        Ok(()) => {
                            active.since = Instant::now();
                            active.paused = false;
                            emit(&events, Event::State(RecorderState::Recording));
                        }
                        Err(error) => emit(&events, Event::Warning(error.to_string())),
                    }
                }
            }
            Some(Command::Stop) => {
                if let Some(active) = active.take() {
                    finish(active, &store, &events, false);
                }
            }
            Some(Command::Shutdown) => {
                if let Some(active) = active.take() {
                    finish(active, &store, &events, false);
                }
                break;
            }
            Some(Command::Recover(recording)) if active.is_none() => {
                emit(&events, Event::State(RecorderState::Finalizing));
                publish(&store, &recording, &events);
            }
            _ => {}
        }

        let mut stop = None;
        if let Some(recording) = active.as_mut() {
            let bus = recording.media.pipeline.bus().unwrap();
            for message in bus.iter() {
                match message.view() {
                    gst::MessageView::Error(error) => {
                        let source = message.src();
                        if let Some(source) = source {
                            match recording.media.detach_failed_audio(source) {
                                Ok(Some(warning)) => {
                                    emit(&events, Event::Warning(warning));
                                    continue;
                                }
                                Ok(None) => {}
                                Err(detach) => emit(
                                    &events,
                                    Event::Warning(format!("Audio cleanup: {detach:#}")),
                                ),
                            }
                        }
                        emit(
                            &events,
                            Event::Warning(format!(
                                "Capture stopped: {} ({})",
                                error.error(),
                                error.debug().unwrap_or_default()
                            )),
                        );
                        stop = Some(false);
                        break;
                    }
                    gst::MessageView::Eos(..) => {
                        stop = Some(true);
                        break;
                    }
                    gst::MessageView::Warning(warning) => {
                        emit(&events, Event::Warning(warning.error().to_string()))
                    }
                    _ => {}
                }
            }
            if last_stats.elapsed() >= Duration::from_millis(500) {
                let _ = events.try_send(Event::Statistics(Statistics {
                    elapsed: recording.elapsed
                        + if recording.paused {
                            Duration::ZERO
                        } else {
                            recording.since.elapsed()
                        },
                    captured: recording.media.counters.captured.load(Ordering::Relaxed),
                    encoded: recording.media.counters.encoded.load(Ordering::Relaxed),
                    queue_overruns: recording.media.counters.overruns.load(Ordering::Relaxed),
                    bytes: recording
                        .pending
                        .mkv
                        .metadata()
                        .map(|m| m.len())
                        .unwrap_or(0),
                }));
                last_stats = Instant::now();
            }
            if last_disk.elapsed() >= Duration::from_secs(2) {
                if let Ok(free) = fs2::available_space(recording.pending.mkv.parent().unwrap())
                    && free < 64 * 1024 * 1024
                {
                    emit(&events, Event::Warning("Low disk space; stopping. MKV will be kept if MP4 conversion cannot finish".into()));
                    stop = Some(false);
                }
                last_disk = Instant::now();
            }
        }
        if let Some(eos) = stop
            && let Some(recording) = active.take()
        {
            finish(recording, &store, &events, eos);
        }
    }
}

fn finish(active: Active, store: &Store, events: &Sender<Event>, already_eos: bool) {
    emit(events, Event::State(RecorderState::Finalizing));
    if !already_eos {
        let _ = active.media.eos();
        let bus = active.media.pipeline.bus().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(message) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) {
                match message.view() {
                    gst::MessageView::Eos(..) => break,
                    gst::MessageView::Error(error) => {
                        emit(events, Event::Warning(error.error().to_string()));
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    let pending = active.pending.clone();
    drop(active); // Close and flush files before starting FFmpeg.
    publish(store, &pending, events);
}

fn publish(store: &Store, pending: &PendingRecording, events: &Sender<Event>) {
    match store.finalize(pending) {
        Ok(path) => {
            emit(events, Event::Saved(path));
            emit(events, Event::State(RecorderState::Idle));
        }
        Err(error) => {
            emit(events, Event::Error(format!("{error:#}")));
            emit(events, Event::State(RecorderState::Error));
        }
    }
}
