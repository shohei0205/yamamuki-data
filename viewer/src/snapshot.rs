//! 表示状態をgzip JSONで保存する。再生中は地形LODとカメラを進めない。
use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Serialize, Deserialize)]
pub(crate) struct SurfaceState {
    pub path: PathBuf,
    pub dataset: usize,
    pub enabled: bool,
    pub options: Vec<(bool, Option<u32>)>,
    pub palette: surface_palette::Palette,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Snapshot {
    format: String,
    version: u32,
    viewer_version: String,
    canvas_size: [f32; 2],
    view: map::View,
    observer: Option<Observer>,
    map_source: map::Source,
    lod: map::Lod,
    surface_lod: map::Lod,
    #[serde(default)]
    map_borders: bool,
    #[serde(default)]
    map_edge_filter: bool,
    #[serde(default)]
    surface_borders: bool,
    palette: surface_palette::Palette,
    show_online_map: bool,
    show_terrain: bool,
    show_names: bool,
    show_markers: bool,
    points: Option<PathBuf>,
    selected: Option<usize>,
    pin: Option<[f64; 2]>,
    pin_elevation: Option<([f64; 2], f64)>,
    surfaces: Vec<SurfaceState>,
    terrain: Option<yamamuki_terrain_view::snapshot::Snapshot>,
}
pub(crate) struct Prepared {
    snapshot: Snapshot,
    terrain: Option<yamamuki_terrain_view::snapshot::Prepared>,
    points: Option<Points>,
    surfaces: Vec<surface::Source>,
}
pub(crate) fn prepare(path: &Path) -> Result<Prepared> {
    let input = std::fs::File::open(path)?;
    let reader: Box<dyn std::io::Read> = if path.extension().is_some_and(|v| v == "gz") {
        Box::new(flate2::read::GzDecoder::new(input))
    } else {
        Box::new(input)
    };
    let mut snapshot: Snapshot = serde_json::from_reader(std::io::BufReader::new(
        std::io::Read::take(reader, 64 * 1024 * 1024),
    ))?;
    ensure!(
        snapshot.format == "yamamuki-viewer-snapshot" && snapshot.version == 1,
        "未対応の表示スナップショットです"
    );
    ensure!(
        snapshot.view.center.iter().all(|v| v.is_finite()) && snapshot.view.zoom.is_finite(),
        "視点情報が不正です"
    );
    ensure!(
        snapshot
            .canvas_size
            .iter()
            .all(|v| v.is_finite() && *v >= 1. && *v <= 32768.),
        "描画領域が不正です"
    );
    let terrain = snapshot.terrain.take().map(|s| s.prepare()).transpose()?;
    let points = snapshot
        .points
        .as_ref()
        .map(|p| match read_file(p.clone())? {
            OpenedFile::Points(v) => Ok(v),
            _ => Err(anyhow::anyhow!("地点データではありません")),
        })
        .transpose()?;
    let surfaces = snapshot
        .surfaces
        .iter()
        .map(|s| surface::Source::open(s.path.clone()))
        .collect::<Result<Vec<_>>>()?;
    Ok(Prepared {
        snapshot,
        terrain,
        points,
        surfaces,
    })
}
impl Viewer {
    pub(crate) fn resume_snapshot(&mut self) {
        self.snapshot_frozen = false;
        self.snapshot_canvas = None;
        self.snapshot_status.clear();
        if let Some(t) = &mut self.terrain {
            t.unfreeze();
        }
        if let Some(t) = &mut self.flat_terrain {
            t.unfreeze();
        }
    }
    fn capture_snapshot(&self) -> Result<Snapshot> {
        let terrain = self
            .terrain
            .as_ref()
            .filter(|_| self.show_terrain)
            .map(|t| t.snapshot())
            .transpose()?;
        let snapshot = Snapshot {
            format: "yamamuki-viewer-snapshot".into(),
            version: 1,
            viewer_version: env!("CARGO_PKG_VERSION").into(),
            canvas_size: self.last_canvas_size,
            view: self.view,
            observer: self.observer.clone(),
            map_source: self.map_source,
            lod: self.lod,
            surface_lod: self.surface_lod,
            map_borders: self.map_borders,
            map_edge_filter: self.map_edge_filter,
            surface_borders: self.surface_borders,
            palette: self.surface_palette,
            show_online_map: self.show_online_map,
            show_terrain: self.show_terrain,
            show_names: self.show_point_names,
            show_markers: self.show_point_markers,
            points: self.source.as_ref().map(|p| p.canonicalize()).transpose()?,
            selected: self.selected,
            pin: self.map_pin,
            pin_elevation: self.map_pin_elevation,
            surfaces: self
                .surfaces
                .iter()
                .map(|surface| {
                    let mut state = surface.snapshot();
                    state.path = state.path.canonicalize()?;
                    Ok(state)
                })
                .collect::<Result<Vec<_>>>()?,
            terrain,
        };
        Ok(snapshot)
    }
    pub(crate) fn save_snapshot(&self, path: &Path) -> Result<()> {
        self.capture_snapshot()?.write(path)
    }
    pub(crate) fn quick_snapshot(&mut self, ctx: &egui::Context) {
        // ファイル処理を待つ前に、そのキー入力時点の描画状態を確保する。
        match self.capture_snapshot() {
            Ok(snapshot) => {
                self.snapshot_jobs.push(save_in_background(
                    snapshot,
                    quick_directory(),
                    ctx.clone(),
                ));
                self.snapshot_notice = Some((
                    "スナップショットを保存中…".into(),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => {
                self.snapshot_notice = Some((
                    format!("保存できません: {error:#}"),
                    std::time::Instant::now(),
                ))
            }
        }
    }
    pub(crate) fn snapshot_shortcut(&mut self, ctx: &egui::Context) {
        let pressed = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::F8,
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            })
        });
        if pressed {
            self.quick_snapshot(ctx);
        }
        self.snapshot_jobs
            .retain(|receive| match receive.try_recv() {
                Ok(result) => {
                    let message = match result {
                        Ok(path) => format!("保存しました: {}", path.display()),
                        Err(error) => format!("保存できません: {error:#}"),
                    };
                    self.snapshot_status = message.clone();
                    self.snapshot_notice = Some((message, std::time::Instant::now()));
                    false
                }
                Err(mpsc::TryRecvError::Empty) => true,
                Err(_) => false,
            });
        if let Some((message, time)) = &self.snapshot_notice {
            let duration = std::time::Duration::from_secs(10);
            if time.elapsed() < duration {
                egui::Area::new(egui::Id::new("snapshot-notice"))
                    .order(egui::Order::Foreground)
                    .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16., -44.))
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_max_width(550.);
                            ui.label(message);
                        });
                    });
                ctx.request_repaint_after(duration - time.elapsed().min(duration));
            }
        }
    }
}
fn save_in_background(
    snapshot: Snapshot,
    dir: PathBuf,
    ctx: egui::Context,
) -> mpsc::Receiver<Result<PathBuf>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> Result<PathBuf> {
            std::fs::create_dir_all(&dir)?;
            let path = quick_path(&dir);
            snapshot.write(&path)?;
            Ok(path)
        })();
        let _ = send.send(result);
        ctx.request_repaint();
    });
    receive
}
fn quick_directory() -> PathBuf {
    eframe::storage_dir(APP_ID)
        .unwrap_or_else(std::env::temp_dir)
        .join("snapshots")
}
fn quick_path(dir: &Path) -> PathBuf {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let id = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dir.join(format!(
        "viewer-{time}-{}-{id}.snapshot.json.gz",
        std::process::id()
    ))
}
impl Snapshot {
    fn write(&self, path: &Path) -> Result<()> {
        let snapshot = self;
        let pending = tempfile::NamedTempFile::new_in(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let file = pending.reopen()?;
        use std::io::Write;
        let mut output = std::io::BufWriter::new(file);
        if path.extension().is_some_and(|v| v == "gz") {
            let mut compressed =
                flate2::write::GzEncoder::new(&mut output, flate2::Compression::default());
            serde_json::to_writer_pretty(&mut compressed, &snapshot)?;
            compressed.finish()?;
        } else {
            serde_json::to_writer_pretty(&mut output, &snapshot)?;
        }
        output.flush()?;
        pending.as_file().sync_all()?;
        pending.persist(path).map_err(|e| e.error)?;
        Ok(())
    }
}
impl Viewer {
    pub(crate) fn snapshot_load(
        &self,
        ctx: &egui::Context,
        path: PathBuf,
    ) -> (PathBuf, mpsc::Receiver<Result<OpenedFile>>) {
        let (send, receive) = mpsc::channel();
        let input = path.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = prepare(&input).map(|p| OpenedFile::Snapshot(Box::new(p)));
            let _ = send.send(result);
            ctx.request_repaint();
        });
        (path, receive)
    }
    pub(crate) fn restore_snapshot(
        &mut self,
        ctx: &egui::Context,
        state: Option<&eframe::egui_wgpu::RenderState>,
        prepared: Prepared,
    ) -> Result<()> {
        let Prepared {
            snapshot: s,
            terrain,
            points,
            surfaces,
        } = prepared;
        let terrain = terrain
            .map(|p| {
                yamamuki_terrain_view::TerrainView::from_snapshot(
                    ctx,
                    state.ok_or_else(|| anyhow::anyhow!("GPUが必要です"))?,
                    p,
                )
            })
            .transpose()?;
        self.terrain = terrain;
        self.terrain_info = None;
        self.flat_terrain = None;
        self.surfaces.clear();
        for (source, options) in surfaces.into_iter().zip(&s.surfaces) {
            let mut surface = surface::Surface::new(ctx, source);
            surface.restore_options(options)?;
            self.surfaces.push(surface);
        }
        self.points = points.map(Arc::new);
        self.source = s.points;
        if let Some(points) = &self.points {
            self.list.reset(points);
        }
        self.selected = s
            .selected
            .filter(|i| self.points.as_ref().is_some_and(|p| *i < p.len()));
        self.view = s.view;
        self.observer = s.observer;
        self.map_source = s.map_source;
        self.lod = s.lod;
        self.surface_lod = s.surface_lod;
        self.map_borders = s.map_borders;
        self.map_edge_filter = s.map_edge_filter;
        self.surface_borders = s.surface_borders;
        self.surface_palette = s.palette;
        self.show_online_map = s.show_online_map;
        self.show_terrain = s.show_terrain;
        self.show_point_names = s.show_names;
        self.show_point_markers = s.show_markers;
        self.map_pin = s.pin;
        self.map_pin_elevation = s.pin_elevation;
        self.pin_height_job = None;
        self.map_hit_job = None;
        self.observer_height_job = None;
        self.location_job = None;
        self.loading_surface = None;
        self.mark_view = None;
        self.snapshot_canvas = Some(s.canvas_size);
        if self.terrain.is_none()
            && let Some(state) = state
        {
            let mut flat = yamamuki_terrain_view::TerrainView::flat(ctx, state)?;
            let rect =
                Rect::from_min_size(Pos2::ZERO, egui::vec2(s.canvas_size[0], s.canvas_size[1]));
            if let Some(observer) = &self.observer {
                flat.set_observer_camera(
                    observer.world,
                    observer_height(
                        observer.elevation,
                        observer.terrain_elevation,
                        observer.from_pin,
                        observer.eye_offset,
                    ),
                    observer.yaw,
                    observer.pitch,
                    observer.fov,
                    rect,
                );
            } else {
                let (yaw, tilt) = self.view.orientation.angles();
                flat.set_map_camera(self.view.center, self.view.zoom, yaw, tilt, rect);
            }
            flat.freeze_current();
            self.flat_terrain = Some(flat);
        }
        self.snapshot_frozen = true;
        self.snapshot_status = "スナップショットを固定表示中（解除で通常操作へ戻ります）".into();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immediate_save_keeps_captured_state_and_never_overwrites() -> Result<()> {
        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(&ctx, reqwest::blocking::Client::new(), None);
        viewer.view.zoom = 11.25;
        let captured = viewer.capture_snapshot()?;
        viewer.view.zoom = 18.5;
        let dir = tempfile::tempdir()?;
        let first = save_in_background(captured, dir.path().to_path_buf(), ctx.clone());
        let second = save_in_background(viewer.capture_snapshot()?, dir.path().to_path_buf(), ctx);
        let a = first.recv_timeout(std::time::Duration::from_secs(5))??;
        let b = second.recv_timeout(std::time::Duration::from_secs(5))??;
        assert_ne!(a, b);
        assert_eq!(prepare(&a)?.snapshot.view.zoom, 11.25);
        assert_eq!(prepare(&b)?.snapshot.view.zoom, 18.5);
        Ok(())
    }
    #[test]
    fn gzip_snapshot_restores_view_and_layer_settings_without_moving() -> Result<()> {
        let ctx = egui::Context::default();
        let mut v = Viewer::new(&ctx, reqwest::blocking::Client::new(), None);
        v.view.zoom = 12.75;
        v.view.center = [0.875, 0.35];
        v.view.orientation.orbit(egui::vec2(14., -80.));
        v.show_online_map = false;
        v.map_borders = true;
        v.map_edge_filter = true;
        v.surface_borders = true;
        v.lod.bias = 1.25;
        v.surface_lod.max_level = Some(9);
        v.map_pin = Some([0.875, 0.35]);
        v.map_pin_elevation = Some(([0.875, 0.35], 1234.5));
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.snapshot.json.gz");
        v.save_snapshot(&path)?;
        let prepared = prepare(&path)?;
        let mut replay = Viewer::new(&ctx, reqwest::blocking::Client::new(), None);
        replay.restore_snapshot(&ctx, None, prepared)?;
        assert!(replay.view == v.view);
        assert_eq!(replay.map_source, v.map_source);
        assert_eq!(replay.lod, v.lod);
        assert_eq!(replay.surface_lod, v.surface_lod);
        assert!(replay.map_borders && replay.surface_borders);
        assert!(replay.map_edge_filter);
        assert_eq!(replay.map_pin_elevation, v.map_pin_elevation);
        assert!(!replay.show_online_map);
        assert!(replay.snapshot_frozen);
        Ok(())
    }
}
