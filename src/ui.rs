use crate::{
    config::{CaptureSource, RecorderState, RecordingConfig, Region, Settings, SourceInfo},
    controller::{Command, Controller, Event, Statistics},
    platform::{
        self,
        shortcuts::{ShortcutEvent, Shortcuts},
    },
    storage::{PendingRecording, Store},
};
use anyhow::Result;
use eframe::egui::{self, Color32, RichText};
use std::time::Duration;

const ACCENT: Color32 = Color32::from_rgb(80, 222, 182);
const MUTED: Color32 = Color32::from_rgb(145, 156, 174);

pub fn run(store: Store) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Clearcast")
            .with_inner_size([860.0, 760.0])
            .with_min_inner_size([660.0, 600.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Clearcast",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, store)))),
    )
    .map_err(|error| anyhow::anyhow!("Cannot open desktop interface: {error}"))
}

struct App {
    store: Store,
    controller: Controller,
    settings: Settings,
    sources: Vec<SourceInfo>,
    source: usize,
    devices: Vec<platform::AudioDevice>,
    region_enabled: bool,
    region: Region,
    state: RecorderState,
    stats: Statistics,
    encoder: String,
    message: String,
    error: String,
    pending: Vec<PendingRecording>,
    shortcuts: Option<Shortcuts>,
    output: String,
    tab: usize,
    close_after_stop: bool,
    preview: Option<(egui::TextureHandle, crate::preview::Preview)>,
    crop_anchor: Option<egui::Pos2>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, store: Store) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.style_mut(|style| {
            style.spacing.item_spacing = egui::vec2(12.0, 10.0);
            style.spacing.button_padding = egui::vec2(16.0, 10.0);
            style.visuals.panel_fill = Color32::from_rgb(17, 22, 30);
            style.visuals.window_fill = Color32::from_rgb(24, 30, 40);
            style.visuals.selection.bg_fill = Color32::from_rgb(35, 100, 85);
        });
        let mut error = String::new();
        let settings = store.load_settings().unwrap_or_else(|e| {
            error = format!("{e:#}");
            Settings::default()
        });
        let sources = platform::sources().unwrap_or_else(|e| {
            error = format!("{e:#}");
            Vec::new()
        });
        let pending = store.pending().unwrap_or_default();
        Self {
            output: settings.output_dir.display().to_string(),
            settings,
            sources,
            controller: Controller::spawn(store.clone()),
            store,
            source: 0,
            devices: platform::audio_devices(),
            region_enabled: false,
            region: Region {
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
            },
            state: RecorderState::Idle,
            stats: Statistics::default(),
            encoder: String::new(),
            message: "Choose a source to get started.".into(),
            error,
            pending,
            shortcuts: None,
            tab: 0,
            close_after_stop: false,
            preview: None,
            crop_anchor: None,
        }
    }

    fn command(&mut self, command: Command) {
        if let Err(error) = self.controller.send(command) {
            self.error = error.to_string();
        }
    }

    fn start_stop(&mut self) {
        match self.state {
            RecorderState::Idle | RecorderState::Error => {
                let Some(source) = self.sources.get(self.source) else {
                    self.error = "Choose an available source first".into();
                    return;
                };
                self.settings.output_dir = self.output.clone().into();
                let config = RecordingConfig {
                    source: source.source.clone(),
                    region: self.region_enabled.then_some(self.region),
                    settings: self.settings.clone(),
                    diagnostic_software: false,
                };
                if let Err(error) = config
                    .validate()
                    .and_then(|_| self.store.save_settings(&self.settings))
                {
                    self.error = format!("{error:#}");
                    return;
                }
                self.error.clear();
                self.stats = Statistics::default();
                self.state = RecorderState::Starting;
                self.message = if platform::is_wayland() {
                    "Complete the desktop screen-sharing dialog."
                } else {
                    "Starting capture and hardware encoder…"
                }
                .into();
                self.command(Command::Start(config));
            }
            RecorderState::Recording | RecorderState::Paused => {
                self.state = RecorderState::Finalizing;
                self.command(Command::Stop);
            }
            _ => {}
        }
    }

    fn pause_resume(&mut self) {
        match self.state {
            RecorderState::Recording => self.command(Command::Pause),
            RecorderState::Paused => self.command(Command::Resume),
            _ => {}
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let events: Vec<_> = self.controller.events.try_iter().collect();
        for event in events {
            match event {
                Event::State(state) => {
                    self.state = state;
                    if state == RecorderState::Recording && self.close_after_stop {
                        self.command(Command::Stop);
                    }
                    if !state.active() {
                        self.pending = self.store.pending().unwrap_or_default();
                        if self.close_after_stop {
                            if state == RecorderState::Idle {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            } else {
                                self.close_after_stop = false;
                            }
                        }
                    }
                }
                Event::Statistics(stats) => self.stats = stats,
                Event::Started {
                    encoder,
                    width,
                    height,
                } => {
                    self.encoder = encoder;
                    self.message =
                        format!("Recording {width} × {height} at {} fps", self.settings.fps);
                }
                Event::Saved(path) => {
                    self.message = format!("Saved {}", path.display());
                    self.error.clear();
                }
                Event::Warning(warning) => self.message = warning,
                Event::Error(error) => self.error = error,
                Event::Preview(preview) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [preview.width, preview.height],
                        &preview.rgba,
                    );
                    let texture =
                        ctx.load_texture("capture-preview", image, egui::TextureOptions::LINEAR);
                    self.message =
                        "Drag on the preview to choose a region, or record the full source.".into();
                    self.preview = Some((texture, preview));
                }
            }
        }
        let shortcuts = self
            .shortcuts
            .as_ref()
            .map(|s| s.poll())
            .unwrap_or_default();
        for event in shortcuts {
            match event {
                ShortcutEvent::Record => self.start_stop(),
                ShortcutEvent::Pause => self.pause_resume(),
                ShortcutEvent::Status(status) => self.message = status,
            }
        }
        if self.shortcuts.is_none() && !ctx.wants_keyboard_input() {
            for (key, pause) in [
                (self.settings.record_hotkey.clone(), false),
                (self.settings.pause_hotkey.clone(), true),
            ] {
                if let Ok(shortcut) = key.parse::<global_hotkey::hotkey::HotKey>() {
                    // Local F9/F10 defaults remain available when a compositor
                    // has no global shortcut portal.
                    let key = if shortcut.key == global_hotkey::hotkey::Code::F9 {
                        egui::Key::F9
                    } else if shortcut.key == global_hotkey::hotkey::Code::F10 {
                        egui::Key::F10
                    } else {
                        continue;
                    };
                    if ctx.input_mut(|input| {
                        input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, key)
                    }) {
                        if pause {
                            self.pause_resume();
                        } else {
                            self.start_stop();
                        }
                        break;
                    }
                }
            }
        }
    }

    fn record_tab(&mut self, ui: &mut egui::Ui) {
        let active = self.state.active();
        let previous_source = self.sources.get(self.source).map(|s| s.source.clone());
        let previous_cursor = self.settings.cursor;
        let previous_dxgi = self.settings.use_dxgi;
        card(ui, |ui| {
            ui.label(RichText::new("CAPTURE SOURCE").small().color(MUTED));
            ui.add_enabled_ui(!active, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("source")
                        .width(470.0)
                        .selected_text(
                            self.sources
                                .get(self.source)
                                .map(|s| s.label.as_str())
                                .unwrap_or("No source available"),
                        )
                        .show_ui(ui, |ui| {
                            for (index, source) in self.sources.iter().enumerate() {
                                ui.selectable_value(&mut self.source, index, &source.label);
                            }
                        });
                    if ui.button("Refresh").clicked() {
                        match platform::sources() {
                            Ok(sources) => {
                                self.sources = sources;
                                self.source = 0;
                            }
                            Err(e) => self.error = format!("{e:#}"),
                        }
                    }
                });
                if ui
                    .button(if platform::is_wayland() {
                        "Choose source & show snapshot"
                    } else {
                        "Show snapshot / select region"
                    })
                    .clicked()
                    && let Some(source) = self.sources.get(self.source)
                {
                    let config = RecordingConfig {
                        source: source.source.clone(),
                        region: None,
                        settings: self.settings.clone(),
                        diagnostic_software: false,
                    };
                    self.state = RecorderState::Starting;
                    self.error.clear();
                    self.command(Command::Preview(config));
                }
                if self.preview.is_some()
                    && ui.small_button("Clear snapshot & release source").clicked()
                {
                    self.preview = None;
                    self.command(Command::ReleasePreview);
                }
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.settings.cursor, "Include cursor");
                    let window = self
                        .sources
                        .get(self.source)
                        .is_some_and(|s| matches!(s.source, CaptureSource::Window { .. }));
                    if window {
                        self.region_enabled = false;
                    }
                    ui.add_enabled_ui(!window, |ui| {
                        ui.checkbox(&mut self.region_enabled, "Record a region");
                    });
                });
                if self.region_enabled {
                    ui.horizontal(|ui| {
                        for (label, value) in [
                            ("X", &mut self.region.x),
                            ("Y", &mut self.region.y),
                            ("Width", &mut self.region.width),
                            ("Height", &mut self.region.height),
                        ] {
                            ui.label(label);
                            ui.add(egui::DragValue::new(value).range(0..=32768));
                        }
                    });
                    ui.label(
                        RichText::new(
                            "Region coordinates are pixels relative to the selected monitor.",
                        )
                        .small()
                        .color(MUTED),
                    );
                }
                #[cfg(windows)]
                ui.checkbox(
                    &mut self.settings.use_dxgi,
                    "Use DXGI monitor capture for exclusive-fullscreen games",
                );
            });
        });
        if previous_source != self.sources.get(self.source).map(|s| s.source.clone())
            || previous_cursor != self.settings.cursor
            || previous_dxgi != self.settings.use_dxgi
        {
            self.preview = None;
            self.region_enabled = false;
            self.command(Command::ReleasePreview);
        }
        if let Some((texture, preview)) = &self.preview {
            let width = ui.available_width().min(preview.width as f32);
            let response = ui.add(
                egui::Image::new(texture)
                    .fit_to_exact_size(egui::vec2(
                        width,
                        width * preview.height as f32 / preview.width as f32,
                    ))
                    .sense(if active {
                        egui::Sense::hover()
                    } else {
                        egui::Sense::drag()
                    }),
            );
            if response.drag_started() {
                self.crop_anchor = response.interact_pointer_pos();
            }
            if response.dragged()
                && let (Some(anchor), Some(pointer)) =
                    (self.crop_anchor, response.interact_pointer_pos())
            {
                let rect = egui::Rect::from_two_pos(anchor, pointer).intersect(response.rect);
                let sx = preview.source_width as f32 / response.rect.width();
                let sy = preview.source_height as f32 / response.rect.height();
                self.region = Region {
                    x: ((rect.left() - response.rect.left()) * sx) as u32,
                    y: ((rect.top() - response.rect.top()) * sy) as u32,
                    width: (rect.width() * sx) as u32,
                    height: (rect.height() * sy) as u32,
                };
                self.region_enabled = true;
            }
            if self.region_enabled {
                let sx = response.rect.width() / preview.source_width as f32;
                let sy = response.rect.height() / preview.source_height as f32;
                let rect = egui::Rect::from_min_size(
                    response.rect.min
                        + egui::vec2(self.region.x as f32 * sx, self.region.y as f32 * sy),
                    egui::vec2(
                        self.region.width as f32 * sx,
                        self.region.height as f32 * sy,
                    ),
                )
                .intersect(response.rect);
                ui.painter().rect_stroke(
                    rect,
                    0.0_f32,
                    egui::Stroke::new(2.0_f32, ACCENT),
                    egui::StrokeKind::Inside,
                );
            }
            ui.label(
                RichText::new("Snapshot only · drag to crop · clear to release the capture source")
                    .small()
                    .color(MUTED),
            );
        }
        ui.add_space(6.0);
        card(ui, |ui| {
            ui.label(RichText::new("VIDEO & AUDIO").small().color(MUTED));
            ui.add_enabled_ui(!active, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Frame rate");
                    ui.selectable_value(&mut self.settings.fps, 30, "30 fps");
                    ui.selectable_value(&mut self.settings.fps, 60, "60 fps");
                    ui.separator();
                    ui.label("Bitrate");
                    ui.add(
                        egui::Slider::new(&mut self.settings.bitrate_mbps, 4..=40).suffix(" Mbps"),
                    );
                });
                audio_row(
                    ui,
                    "System sound",
                    &mut self.settings.system_audio,
                    &mut self.settings.system_volume,
                    &mut self.settings.system_device,
                    &self.devices,
                );
                audio_row(
                    ui,
                    "Microphone",
                    &mut self.settings.microphone,
                    &mut self.settings.microphone_volume,
                    &mut self.settings.microphone_device,
                    &self.devices,
                );
                ui.label(
                    RichText::new("H.264 · AAC · up to 1080p · hardware encoding")
                        .small()
                        .color(MUTED),
                );
            });
        });
        ui.add_space(6.0);
        card(ui, |ui| {
            ui.label(RichText::new("SAVE TO").small().color(MUTED));
            ui.add_enabled(
                !active,
                egui::TextEdit::singleline(&mut self.output).desired_width(f32::INFINITY),
            );
            ui.label(
                RichText::new("Recoverable MKV while recording; MP4 when you stop.")
                    .small()
                    .color(MUTED),
            );
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let seconds = self.stats.elapsed.as_secs();
            ui.label(
                RichText::new(format!(
                    "{:02}:{:02}:{:02}",
                    seconds / 3600,
                    seconds / 60 % 60,
                    seconds % 60
                ))
                .size(32.0)
                .monospace(),
            );
            ui.label(
                RichText::new(format!(
                    "{:.1} MiB\n{} frames · {} queue overruns",
                    self.stats.bytes as f64 / 1_048_576.0,
                    self.stats.encoded,
                    self.stats.queue_overruns
                ))
                .small()
                .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_stop =
                    matches!(self.state, RecorderState::Recording | RecorderState::Paused);
                let enabled = matches!(
                    self.state,
                    RecorderState::Idle
                        | RecorderState::Error
                        | RecorderState::Recording
                        | RecorderState::Paused
                );
                let text = if can_stop {
                    "Stop & save"
                } else if self.state == RecorderState::Finalizing {
                    "Saving…"
                } else {
                    "Start recording"
                };
                if ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(RichText::new(text).color(Color32::BLACK))
                            .fill(if can_stop {
                                Color32::from_rgb(255, 126, 126)
                            } else {
                                ACCENT
                            })
                            .min_size(egui::vec2(170.0, 45.0)),
                    )
                    .clicked()
                {
                    self.start_stop();
                }
                if can_stop
                    && ui
                        .button(if self.state == RecorderState::Paused {
                            "Resume"
                        } else {
                            "Pause"
                        })
                        .clicked()
                {
                    self.pause_resume();
                }
            });
        });
    }

    fn recovery_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Unfinished recordings");
        ui.label("Recover available video and audio into an MP4. The last unwritten seconds may be lost.");
        if self.pending.is_empty() {
            ui.add_space(30.0);
            ui.label(RichText::new("No unfinished recordings.").color(MUTED));
        }
        for recording in self.pending.clone() {
            card(ui, |ui| {
                ui.label(&recording.started);
                ui.label(recording.mkv.display().to_string());
                if ui
                    .add_enabled(!self.state.active(), egui::Button::new("Recover to MP4"))
                    .clicked()
                {
                    self.state = RecorderState::Finalizing;
                    self.command(Command::Recover(recording.clone()));
                }
            });
        }
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Shortcuts & diagnostics");
        ui.label("Record shortcut starts or stops. Pause shortcut pauses or resumes.");
        ui.add_enabled_ui(!self.state.active(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Record");
                ui.text_edit_singleline(&mut self.settings.record_hotkey);
            });
            ui.horizontal(|ui| {
                ui.label("Pause");
                ui.text_edit_singleline(&mut self.settings.pause_hotkey);
            });
            if ui.button("Apply global shortcuts").clicked() {
                self.shortcuts = None;
                match Shortcuts::bind(&self.settings.record_hotkey, &self.settings.pause_hotkey) {
                    Ok(shortcuts) => {
                        self.shortcuts = Some(shortcuts);
                        self.message = "Registering shortcuts…".into();
                    }
                    Err(error) => self.error = format!("{error:#}"),
                }
            }
            if ui.button("Save settings").clicked() {
                self.settings.output_dir = self.output.clone().into();
                match self.store.save_settings(&self.settings) {
                    Ok(()) => self.message = "Settings saved".into(),
                    Err(error) => self.error = format!("{error:#}"),
                }
            }
        });
        ui.separator();
        ui.label(format!("GStreamer {}", gstreamer::version_string()));
        let encoders = crate::media::hardware_encoders();
        ui.label(format!(
            "Hardware H.264 encoders: {}",
            if encoders.is_empty() {
                "None detected".into()
            } else {
                encoders.join(", ")
            }
        ));
        ui.label(format!(
            "Session: {}",
            if platform::is_wayland() {
                "Wayland / screen-sharing portal"
            } else if cfg!(windows) {
                "Windows / Direct3D 11"
            } else {
                "X11"
            }
        ));
        ui.label(format!("Application data: {}", self.store.root.display()));
        ui.label(
            RichText::new("For more detail, run: clearcast doctor")
                .monospace()
                .color(MUTED),
        );
        ui.add_space(12.0);
        ui.label("Global shortcuts on Wayland depend on your desktop portal. Capture of protected content and some fullscreen games is restricted by the OS.");
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll(ctx);
        ctx.request_repaint_after(if self.state.active() {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(1)
        });
        if ctx.input(|input| input.viewport().close_requested()) && self.state.active() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_after_stop = true;
            if matches!(self.state, RecorderState::Recording | RecorderState::Paused) {
                self.start_stop();
            }
            self.message = "Finishing your recording before closing…".into();
        }
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(30.0, 40.0), egui::Sense::hover());
                ui.painter()
                    .circle_stroke(rect.center(), 11.0, egui::Stroke::new(2.0_f32, ACCENT));
                ui.painter().circle_filled(rect.center(), 5.0, ACCENT);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Clearcast").size(25.0).strong());
                    ui.label(RichText::new("SCREEN RECORDER").small().color(MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let color = if self.state == RecorderState::Recording {
                        Color32::LIGHT_RED
                    } else {
                        ACCENT
                    };
                    ui.label(RichText::new(format!("{:?}", self.state)).color(color));
                });
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, 0, "Recorder");
                ui.selectable_value(
                    &mut self.tab,
                    1,
                    format!("Recovery ({})", self.pending.len()),
                );
                ui.selectable_value(&mut self.tab, 2, "Settings");
            });
            ui.add_space(8.0);
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new(&self.message).small().color(MUTED));
            if !self.error.is_empty() {
                ui.colored_label(Color32::LIGHT_RED, &self.error);
            }
            ui.add_space(6.0);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(12.0);
                match self.tab {
                    0 => self.record_tab(ui),
                    1 => self.recovery_tab(ui),
                    _ => self.settings_tab(ui),
                }
            });
        });
    }
}

fn card(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::from_rgb(26, 33, 44))
        .corner_radius(10)
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            contents(ui);
        });
}

fn audio_row(
    ui: &mut egui::Ui,
    label: &str,
    enabled: &mut bool,
    volume: &mut f64,
    device: &mut Option<String>,
    devices: &[platform::AudioDevice],
) {
    ui.horizontal(|ui| {
        ui.checkbox(enabled, label);
        ui.add_enabled(
            *enabled,
            egui::Slider::new(volume, 0.0..=2.0).show_value(false),
        );
        ui.add_enabled_ui(*enabled, |ui| {
            egui::ComboBox::from_id_salt(label)
                .width(220.0)
                .selected_text(
                    device
                        .as_ref()
                        .and_then(|id| {
                            devices
                                .iter()
                                .find(|d| &d.id == id)
                                .map(|d| d.label.as_str())
                        })
                        .unwrap_or("System default"),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(device, None, "System default");
                    for source in devices {
                        ui.selectable_value(device, Some(source.id.clone()), &source.label);
                    }
                });
        });
    });
}
