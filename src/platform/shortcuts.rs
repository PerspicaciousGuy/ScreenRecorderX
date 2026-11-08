use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, bounded};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};

#[derive(Debug, Clone)]
pub enum ShortcutEvent {
    Record,
    Pause,
    Status(String),
}

pub struct Shortcuts {
    manager: Option<GlobalHotKeyManager>,
    record: Option<HotKey>,
    pause: Option<HotKey>,
    events: Receiver<ShortcutEvent>,
    stop: Sender<()>,
}

impl Shortcuts {
    pub fn bind(record: &str, pause: &str) -> Result<Self> {
        let (sender, events) = bounded(16);
        let (stop, _stopping) = bounded(1);
        if super::is_wayland() {
            #[cfg(all(target_os = "linux", feature = "portal"))]
            {
                let record = record.to_owned();
                let pause = pause.to_owned();
                std::thread::Builder::new().name("shortcut-portal".into()).spawn(move || {
                    let result = (|| -> Result<()> {
                        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
                        runtime.block_on(portal(record, pause, sender.clone(), _stopping))
                    })();
                    if let Err(error) = result {
                        let _ = sender.try_send(ShortcutEvent::Status(format!("Global shortcuts unavailable: {error:#}. Application shortcuts still work")));
                    }
                })?;
            }
            #[cfg(not(all(target_os = "linux", feature = "portal")))]
            anyhow::bail!("Build with portal support to enable Wayland global shortcuts");
            #[cfg(all(target_os = "linux", feature = "portal"))]
            return Ok(Self {
                manager: None,
                record: None,
                pause: None,
                events,
                stop,
            });
        }
        let record: HotKey = record
            .parse()
            .context("Invalid record shortcut; example: Ctrl+Shift+F9")?;
        let pause: HotKey = pause
            .parse()
            .context("Invalid pause shortcut; example: Ctrl+Shift+F10")?;
        anyhow::ensure!(
            record.id() != pause.id(),
            "Record and pause shortcuts must be different"
        );
        let manager = GlobalHotKeyManager::new()?;
        manager.register(record)?;
        if let Err(error) = manager.register(pause) {
            let _ = manager.unregister(record);
            return Err(error.into());
        }
        let _ = sender.try_send(ShortcutEvent::Status("Global shortcuts enabled".into()));
        Ok(Self {
            manager: Some(manager),
            record: Some(record),
            pause: Some(pause),
            events,
            stop,
        })
    }

    pub fn poll(&self) -> Vec<ShortcutEvent> {
        let mut events: Vec<_> = self.events.try_iter().collect();
        if self.manager.is_some() {
            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                if event.state != HotKeyState::Pressed {
                    continue;
                }
                if self.record.is_some_and(|key| key.id() == event.id) {
                    events.push(ShortcutEvent::Record);
                }
                if self.pause.is_some_and(|key| key.id() == event.id) {
                    events.push(ShortcutEvent::Pause);
                }
            }
        }
        events
    }
}

impl Drop for Shortcuts {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
        if let Some(manager) = &self.manager {
            if let Some(key) = self.record {
                let _ = manager.unregister(key);
            }
            if let Some(key) = self.pause {
                let _ = manager.unregister(key);
            }
        }
    }
}

#[cfg(all(target_os = "linux", feature = "portal"))]
async fn portal(
    record: String,
    pause: String,
    events: Sender<ShortcutEvent>,
    stop: Receiver<()>,
) -> Result<()> {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    let portal = GlobalShortcuts::new().await?;
    let session = portal.create_session().await?;
    let session_path = serde_json::to_value(&session)?
        .as_str()
        .context("Invalid shortcut session")?
        .to_owned();
    let mut activated = portal.receive_activated().await?;
    let record = record.to_ascii_uppercase();
    let pause = pause.to_ascii_uppercase();
    portal
        .bind_shortcuts(
            &session,
            &[
                NewShortcut::new("record", "Start or stop recording")
                    .preferred_trigger(record.as_str()),
                NewShortcut::new("pause", "Pause or resume recording")
                    .preferred_trigger(pause.as_str()),
            ],
            None,
        )
        .await?
        .response()?;
    let _ = events.try_send(ShortcutEvent::Status(
        "Desktop global shortcuts enabled".into(),
    ));
    loop {
        match stop.try_recv() {
            Ok(()) | Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            Err(crossbeam_channel::TryRecvError::Empty) => {}
        }
        if let Ok(Some(event)) =
            tokio::time::timeout(std::time::Duration::from_millis(250), activated.next()).await
        {
            if event.session_handle().as_str() != session_path {
                continue;
            }
            let event = match event.shortcut_id() {
                "record" => ShortcutEvent::Record,
                "pause" => ShortcutEvent::Pause,
                _ => continue,
            };
            let _ = events.try_send(event);
        }
    }
    session.close().await?;
    Ok(())
}
