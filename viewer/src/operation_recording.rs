//! F8押下中のアプリ入力を、開始状態・時刻・操作後の視点と一緒に記録する。
use super::*;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Recording {
    version: u32,
    executable: Option<PathBuf>,
    executable_modified_ms: Option<u128>,
    process_id: u32,
    pub initial: Box<snapshot::Snapshot>,
    initial_frozen: bool,
    initial_input: egui::RawInput,
    ui_memory: Option<String>,
    initial_focus: Option<egui::Id>,
    end_capture_error: Option<String>,
    settings_open: bool,
    settings_section: SettingsSection,
    layer_list_height: Option<f32>,
    previous_trace_summary: Option<String>,
    pub frames: Vec<InputFrame>,
    duration_ms: f64,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct InputFrame {
    elapsed_ms: f64,
    input: egui::RawInput,
    after: Option<CameraState>,
}
#[derive(Clone, Serialize, Deserialize)]
struct CameraState {
    view: map::View,
    observer: Option<Observer>,
    frozen: bool,
    canvas: [f32; 2],
}
pub(crate) struct Active {
    started: Instant,
    recording: Recording,
    stopping: bool,
    ready: bool,
}
pub(crate) struct Playback {
    recording: Arc<Recording>,
    started: Option<Instant>,
    next: usize,
    modifiers: egui::Modifiers,
}
#[derive(Default)]
struct MemoryStorage(Option<String>);
impl eframe::Storage for MemoryStorage {
    fn get_string(&self, _: &str) -> Option<String> {
        self.0.clone()
    }
    fn set_string(&mut self, _: &str, value: String) {
        self.0 = Some(value);
    }
    fn flush(&mut self) {}
}
fn f8(event: &egui::Event, pressed: bool) -> bool {
    matches!(event, egui::Event::Key {key:egui::Key::F8,pressed:p,..} if *p==pressed)
}
fn bootstrap(ctx: &egui::Context, raw: &egui::RawInput) -> egui::RawInput {
    let mut input = raw.clone();
    input.events.clear();
    input.dropped_files.clear();
    input.hovered_files.clear();
    ctx.input(|i| {
        if let Some(pos) = i.pointer.latest_pos() {
            input.events.push(egui::Event::PointerMoved(pos));
            for button in [
                egui::PointerButton::Primary,
                egui::PointerButton::Secondary,
                egui::PointerButton::Middle,
                egui::PointerButton::Extra1,
                egui::PointerButton::Extra2,
            ] {
                if i.pointer.button_down(button) {
                    input.events.push(egui::Event::PointerButton {
                        pos,
                        button,
                        pressed: true,
                        modifiers: i.modifiers,
                    });
                }
            }
        }
        for &key in &i.keys_down {
            if key != egui::Key::F8 {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: i.modifiers,
                });
            }
        }
    });
    input
}
fn release_input(ctx: &egui::Context, raw: &mut egui::RawInput) {
    let mut release = bootstrap(ctx, raw);
    for event in &mut release.events {
        if let egui::Event::PointerButton { pressed, .. } | egui::Event::Key { pressed, .. } = event
        {
            *pressed = false;
        }
    }
    raw.events.splice(0..0, release.events);
}
impl Recording {
    pub(crate) fn summary(&self) -> String {
        format!(
            "操作記録：{:.1}秒・{}フレーム",
            self.duration_ms / 1000.,
            self.frames.len()
        )
    }
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && self.initial.recording.is_none(),
            "未対応の操作記録です"
        );
        ensure!(
            self.duration_ms.is_finite()
                && self.duration_ms >= 0.
                && Duration::try_from_secs_f64(self.duration_ms / 1000.).is_ok(),
            "記録時間が不正です"
        );
        let mut previous = 0.;
        for frame in &self.frames {
            ensure!(
                frame.elapsed_ms.is_finite()
                    && frame.elapsed_ms >= previous
                    && frame.elapsed_ms <= self.duration_ms,
                "入力時刻の順序が不正です"
            );
            ensure!(
                frame.input.viewports.contains_key(&frame.input.viewport_id),
                "入力のウィンドウ情報がありません"
            );
            previous = frame.elapsed_ms;
        }
        Ok(())
    }
}
impl Viewer {
    pub(crate) fn operation_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // 解除フレームを描き終えてから確保し、視点とGPU表示を同じ時点で保存する。
        if self.operation_recording.as_ref().is_some_and(|r| r.ready) {
            self.finish_operation_recording(ctx);
        }
        if self.operation_release_pending {
            release_input(ctx, raw);
            self.operation_release_pending = false;
        }
        if self.operation_playback.is_some() {
            self.play_operation_input(ctx, raw);
            return;
        }
        let start = raw.events.iter().any(|e| f8(e, true));
        if start && self.operation_recording.is_none() {
            match self.capture_snapshot() {
                Ok(initial) => {
                    let mut storage = MemoryStorage::default();
                    ctx.memory(|m| eframe::set_value(&mut storage, "ui", m));
                    self.operation_recording = Some(Active {
                        started: Instant::now(),
                        recording: Recording {
                            version: 1,
                            executable: std::env::current_exe().ok(),
                            executable_modified_ms: std::env::current_exe()
                                .ok()
                                .and_then(|p| std::fs::metadata(p).ok())
                                .and_then(|m| m.modified().ok())
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_millis()),
                            process_id: std::process::id(),
                            initial: Box::new(initial),
                            initial_frozen: self.snapshot_frozen,
                            initial_input: bootstrap(ctx, raw),
                            ui_memory: storage.0,
                            initial_focus: ctx.memory(|m| m.focused()),
                            end_capture_error: None,
                            settings_open: self.settings_open,
                            settings_section: self.settings_section,
                            layer_list_height: self.layer_list_height,
                            previous_trace_summary: self
                                .operation_trace
                                .as_ref()
                                .map(|r| r.summary()),
                            frames: Vec::new(),
                            duration_ms: 0.,
                        },
                        stopping: false,
                        ready: false,
                    });
                    self.snapshot_notice = None;
                }
                Err(error) => {
                    self.snapshot_notice =
                        Some((format!("記録を開始できません: {error:#}"), Instant::now()))
                }
            }
        }
        if let Some(active) = &mut self.operation_recording {
            let mut input = raw.clone();
            // GPUからの画像応答は操作ではない。画像本体を記録へ複製しない。
            input
                .events
                .retain(|e| !matches!(e, egui::Event::Screenshot { .. }));
            let elapsed_ms = active.started.elapsed().as_secs_f64() * 1000.;
            active.recording.frames.push(InputFrame {
                elapsed_ms,
                input,
                after: None,
            });
            if raw.events.iter().any(|e| f8(e, false)) || !raw.focused {
                active.stopping = true;
                active.recording.duration_ms = elapsed_ms;
            }
        }
    }
    pub(crate) fn operation_frame_finished(&mut self, ctx: &egui::Context) {
        if let Some(active) = &mut self.operation_recording {
            if let Some(frame) = active.recording.frames.last_mut() {
                frame.after = Some(CameraState {
                    view: self.view,
                    observer: self.observer.clone(),
                    frozen: self.snapshot_frozen,
                    canvas: self.last_canvas_size,
                });
            }
            if active.stopping {
                active.ready = true;
                ctx.request_repaint();
            }
        }
        if self
            .operation_playback
            .as_ref()
            .is_some_and(|p| p.next >= p.recording.frames.len())
        {
            self.operation_playback = None;
            self.operation_release_pending = true;
            self.snapshot_status = "操作の再生が完了しました".into();
            ctx.request_repaint();
        }
    }
    fn take_operation_snapshot(&mut self) -> Result<Option<snapshot::Snapshot>> {
        let Some(mut active) = self.operation_recording.take() else {
            return Ok(None);
        };
        active.recording.duration_ms = active
            .recording
            .duration_ms
            .max(active.recording.frames.last().map_or(0., |f| f.elapsed_ms));
        let mut snapshot = match self.capture_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                // 終了時にGPUを準備中でも、取得済みの入力記録を失わない。
                active.recording.end_capture_error = Some(format!("{error:#}"));
                (*active.recording.initial).clone()
            }
        };
        snapshot.recording = Some(Box::new(active.recording));
        Ok(Some(snapshot))
    }
    fn finish_operation_recording(&mut self, ctx: &egui::Context) {
        match self.take_operation_snapshot() {
            Ok(Some(snapshot)) => {
                let summary = snapshot.recording.as_ref().unwrap().summary();
                self.snapshot_jobs.push(snapshot::save_in_background(
                    snapshot,
                    snapshot::quick_directory(),
                    ctx.clone(),
                ));
                self.snapshot_notice = Some((format!("{summary}を保存中…"), Instant::now()));
            }
            Ok(None) => {}
            Err(error) => {
                self.snapshot_notice = Some((
                    format!("操作記録を保存できません: {error:#}"),
                    Instant::now(),
                ))
            }
        }
    }
    pub(crate) fn operation_menu_summary(&self) -> Option<String> {
        if let Some(playback) = &self.operation_playback {
            playback.recording.previous_trace_summary.clone()
        } else {
            self.operation_trace.as_ref().map(|r| r.summary())
        }
    }
    pub(crate) fn flush_operation_recording(&mut self, dir: &std::path::Path) {
        if self.operation_recording.is_some() {
            let result = (|| -> Result<()> {
                if let Some(snapshot) = self.take_operation_snapshot()? {
                    std::fs::create_dir_all(dir)?;
                    let path = snapshot::quick_path(dir);
                    snapshot.write(&path)?;
                    diagnostics::record(&format!("終了時の操作記録保存: {}", path.display()));
                }
                Ok(())
            })();
            if let Err(error) = result {
                diagnostics::record(&format!("操作記録の保存失敗: {error:#}"));
            }
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        for job in self.snapshot_jobs.drain(..) {
            if Instant::now() >= deadline {
                break;
            }
            if let Err(error) = job.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                diagnostics::record(&format!("スナップショット保存の完了待ち: {error}"));
            }
        }
    }
    pub(crate) fn operation_notice(&self, ctx: &egui::Context) {
        let message = if let Some(active) = &self.operation_recording {
            Some(format!(
                "● 操作を記録中 {:.1}秒（F8を離すと保存）",
                active.started.elapsed().as_secs_f64()
            ))
        } else {
            self.operation_playback.as_ref().map(|p| {
                format!(
                    "操作を再生中 {}/{}（Escで中止）",
                    p.next,
                    p.recording.frames.len()
                )
            })
        };
        if let Some(message) = message {
            egui::Area::new(egui::Id::new("operation-recording-notice"))
                .order(egui::Order::Foreground)
                .interactable(false)
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16., -80.))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.colored_label(appearance::ACCENT, message);
                    });
                });
        }
    }
    pub(crate) fn replay_operations(&mut self, ctx: &egui::Context) {
        let Some(recording) = self.operation_trace.clone() else {
            return;
        };
        if self.operation_recording.is_some() || self.operation_playback.is_some() {
            return;
        }
        let initial = (*recording.initial).clone();
        let (send, receive) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = snapshot::prepare_value(initial).map(|mut p| {
                p.replay = Some(recording);
                OpenedFile::Snapshot(Box::new(p))
            });
            let _ = send.send(result);
            wake.request_repaint();
        });
        self.loading = Some((PathBuf::from("記録した操作"), receive));
    }
    pub(crate) fn begin_operation_playback(
        &mut self,
        ctx: &egui::Context,
        recording: Arc<Recording>,
    ) {
        if let Some(memory) =
            eframe::get_value::<egui::Memory>(&MemoryStorage(recording.ui_memory.clone()), "ui")
        {
            ctx.memory_mut(|m| *m = memory);
        }
        if let Some(id) = recording.initial_focus {
            ctx.memory_mut(|m| m.request_focus(id));
        }
        self.settings_open = recording.settings_open;
        self.settings_section = recording.settings_section;
        self.layer_list_height = recording.layer_list_height;
        if !recording.initial_frozen {
            self.resume_snapshot();
        }
        self.operation_trace = Some(recording.clone());
        self.operation_playback = Some(Playback {
            recording,
            started: None,
            next: 0,
            modifiers: egui::Modifiers::default(),
        });
        self.snapshot_status = "記録した操作を再生中".into();
        ctx.request_repaint();
    }
    fn play_operation_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if raw.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                }
            )
        }) {
            self.operation_playback = None;
            self.snapshot_status = "操作の再生を中止しました".into();
            raw.events.retain(|e| {
                !matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        ..
                    }
                )
            });
            release_input(ctx, raw);
            return;
        }
        let playback = self.operation_playback.as_mut().unwrap();
        let started = *playback.started.get_or_insert_with(Instant::now);
        let Some(frame) = playback.recording.frames.get(playback.next) else {
            return;
        };
        let due = Duration::from_secs_f64(frame.elapsed_ms / 1000.);
        if started.elapsed() < due {
            raw.events.clear();
            raw.dropped_files.clear();
            raw.modifiers = playback.modifiers;
            ctx.request_repaint_after(due - started.elapsed().min(due));
            return;
        }
        let mut input = frame.input.clone();
        if playback.next == 0 {
            input
                .events
                .splice(0..0, playback.recording.initial_input.events.clone());
        }
        input.events.retain(|e| !f8(e, true) && !f8(e, false));
        // 入力時刻は現在の単調増加時計を使い、終了後に時計を逆行させない。
        input.time = raw.time;
        input.max_texture_side = raw.max_texture_side;
        let info = input
            .viewports
            .remove(&input.viewport_id)
            .unwrap_or_default();
        input.viewport_id = raw.viewport_id;
        input.viewports = raw.viewports.clone();
        input.viewports.insert(input.viewport_id, info);
        playback.modifiers = input.modifiers;
        playback.next += 1;
        *raw = input;
        if let Some(next) = playback.recording.frames.get(playback.next) {
            ctx.request_repaint_after(
                Duration::from_secs_f64(next.elapsed_ms / 1000.).saturating_sub(started.elapsed()),
            );
        } else {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(pressed: bool, repeat: bool) -> egui::Event {
        egui::Event::Key {
            key: egui::Key::F8,
            physical_key: Some(egui::Key::F8),
            pressed,
            repeat,
            modifiers: egui::Modifiers::NONE,
        }
    }
    fn raw(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200., 800.))),
            events,
            ..Default::default()
        }
    }
    fn viewer(ctx: &egui::Context) -> Viewer {
        let mut v = Viewer::new(ctx, reqwest::blocking::Client::new(), None);
        v.show_online_map = false;
        v
    }
    #[test]
    fn f8_records_all_frames_and_finishes_only_after_release() -> Result<()> {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.operation_input(&ctx, &mut raw(vec![key(true, false)]));
        assert!(v.operation_recording.is_some());
        assert!(v.snapshot_jobs.is_empty());
        v.operation_frame_finished(&ctx);
        let events = vec![
            key(true, true),
            egui::Event::PointerMoved(egui::pos2(700., 400.)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0., 1.),
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Text("日本語".into()),
        ];
        v.operation_input(&ctx, &mut raw(events.clone()));
        v.view.zoom = 14.5;
        v.operation_frame_finished(&ctx);
        assert_eq!(
            v.operation_recording
                .as_ref()
                .unwrap()
                .recording
                .frames
                .len(),
            2
        );
        assert!(v.snapshot_jobs.is_empty());
        v.operation_input(&ctx, &mut raw(vec![key(false, false)]));
        assert!(v.snapshot_jobs.is_empty());
        assert!(!v.operation_recording.as_ref().unwrap().ready);
        v.operation_frame_finished(&ctx);
        assert!(v.operation_recording.as_ref().unwrap().ready);
        let snapshot = v.take_operation_snapshot()?.unwrap();
        let record = snapshot.recording.as_ref().unwrap();
        assert_eq!(record.frames.len(), 3);
        assert_eq!(record.frames[1].input.events, events);
        assert_eq!(record.frames[1].after.as_ref().unwrap().view.zoom, 14.5);
        record.validate()?;
        let dir = tempfile::tempdir()?;
        let result = snapshot::save_in_background(snapshot, dir.path().to_path_buf(), ctx);
        let path = result.recv_timeout(Duration::from_secs(5))??;
        snapshot::prepare(&path)?;
        let saved: snapshot::Snapshot =
            serde_json::from_reader(flate2::read::GzDecoder::new(std::fs::File::open(path)?))?;
        assert_eq!(saved.recording.unwrap().frames.len(), 3);
        Ok(())
    }
    #[test]
    fn loss_of_focus_finishes_recording_without_a_key_up() -> Result<()> {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.operation_input(&ctx, &mut raw(vec![key(true, false)]));
        v.operation_frame_finished(&ctx);
        let mut lost = raw(vec![egui::Event::WindowFocused(false)]);
        lost.focused = false;
        v.operation_input(&ctx, &mut lost);
        v.operation_frame_finished(&ctx);
        assert!(v.operation_recording.as_ref().unwrap().ready);
        let snapshot = v.take_operation_snapshot()?.unwrap();
        assert!(
            !snapshot
                .recording
                .unwrap()
                .frames
                .last()
                .unwrap()
                .input
                .focused
        );
        Ok(())
    }
    #[test]
    fn initial_mouse_hold_is_bootstrapped_and_released_after_replay() -> Result<()> {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let p = egui::pos2(850., 400.);
        let _ = ctx.run(
            raw(vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]),
            |_| {},
        );
        v.operation_input(&ctx, &mut raw(vec![key(true, false)]));
        v.operation_frame_finished(&ctx);
        v.operation_input(&ctx, &mut raw(vec![key(false, false)]));
        v.operation_frame_finished(&ctx);
        let snapshot = v.take_operation_snapshot()?.unwrap();
        let record = Arc::new(*snapshot.recording.unwrap());
        assert!(
            record
                .initial_input
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. }))
        );
        let replay_ctx = egui::Context::default();
        let mut replay = viewer(&replay_ctx);
        replay.begin_operation_playback(&replay_ctx, record.clone());
        replay.operation_playback.as_mut().unwrap().started =
            Some(Instant::now() - Duration::from_secs(1));
        for _ in &record.frames {
            let mut input = raw(vec![]);
            replay.operation_input(&replay_ctx, &mut input);
            let _ = replay_ctx.run(input, |_| {});
            replay.operation_frame_finished(&replay_ctx);
        }
        assert!(replay.operation_release_pending);
        let mut input = raw(vec![]);
        replay.operation_input(&replay_ctx, &mut input);
        assert!(
            input
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::PointerButton { pressed: false, .. }))
        );
        let _ = replay_ctx.run(input, |_| {});
        assert!(!replay_ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary)));
        Ok(())
    }
    #[test]
    fn closing_the_app_flushes_the_recording() -> Result<()> {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.operation_input(&ctx, &mut raw(vec![key(true, false)]));
        v.operation_frame_finished(&ctx);
        let dir = tempfile::tempdir()?;
        v.flush_operation_recording(dir.path());
        assert!(v.operation_recording.is_none());
        let files = std::fs::read_dir(dir.path())?.collect::<std::io::Result<Vec<_>>>()?;
        assert_eq!(files.len(), 1);
        snapshot::prepare(&files[0].path())?;
        Ok(())
    }
    fn run_frame(v: &mut Viewer, ctx: &egui::Context, mut input: egui::RawInput, time: f64) {
        input.time = Some(time);
        v.operation_input(ctx, &mut input);
        let mut frame = eframe::Frame::_new_kittest();
        let _ = ctx.run(input, |ctx| eframe::App::update(v, ctx, &mut frame));
    }
    #[test]
    fn replay_uses_the_recorded_input_order_and_reproduces_map_operations() -> Result<()> {
        let ctx = egui::Context::default();
        let mut original = viewer(&ctx);
        original.view.zoom = 10.;
        let p = egui::pos2(850., 400.);
        let p2 = p + egui::vec2(45., 20.);
        run_frame(
            &mut original,
            &ctx,
            raw(vec![egui::Event::PointerMoved(p)]),
            0.,
        );
        run_frame(&mut original, &ctx, raw(vec![key(true, false)]), 0.02);
        run_frame(
            &mut original,
            &ctx,
            raw(vec![egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }]),
            0.04,
        );
        run_frame(
            &mut original,
            &ctx,
            raw(vec![egui::Event::PointerMoved(p2)]),
            0.06,
        );
        run_frame(
            &mut original,
            &ctx,
            raw(vec![
                egui::Event::PointerButton {
                    pos: p2,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0., 1.),
                    modifiers: egui::Modifiers::NONE,
                },
            ]),
            0.08,
        );
        run_frame(&mut original, &ctx, raw(vec![key(false, false)]), 0.10);
        let expected = original.view;
        assert_ne!(expected.zoom, 10., "実際の地図操作を記録できていない");
        let snapshot = original.take_operation_snapshot()?.unwrap();
        let record = Arc::new(*snapshot.recording.unwrap());
        let replay_ctx = egui::Context::default();
        let mut replay = viewer(&replay_ctx);
        let mut prepared = snapshot::prepare_value((*record.initial).clone())?;
        prepared.replay = Some(record.clone());
        replay.restore_snapshot(&replay_ctx, None, prepared)?;
        assert!(
            replay.operation_menu_summary().is_none(),
            "再生時のメニュー追加で入力座標がずれる"
        );
        replay.operation_playback.as_mut().unwrap().started =
            Some(Instant::now() - Duration::from_secs(1));
        for (i, _) in record.frames.iter().enumerate() {
            run_frame(&mut replay, &replay_ctx, raw(vec![]), 1. + i as f64 * 0.02);
        }
        assert!(replay.operation_playback.is_none());
        assert!(
            (replay.view.zoom - expected.zoom).abs() < 0.00001,
            "再生でズームが変わった: {} / {}",
            replay.view.zoom,
            expected.zoom
        );
        for axis in 0..2 {
            assert!(
                (replay.view.center[axis] - expected.center[axis]).abs() < 1e-9,
                "再生で中心座標が変わった"
            );
        }
        assert!(
            replay.operation_recording.is_none(),
            "記録されたF8で再記録している"
        );
        Ok(())
    }
}
