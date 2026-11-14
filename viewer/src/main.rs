#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod appearance;
mod boundaries;
mod diagnostics;
mod lod_fade;
mod map;
mod markers;
mod point_detail;
mod point_list;
mod points;
mod surface;
mod surface_palette;
mod surface_shade;
mod terrain_info;
mod terrain_material;
mod tile_atlas;
mod tile_cache;
#[cfg(windows)]
mod window_move;

use anyhow::{Result, ensure};
use eframe::egui::{self, Color32, FontData, FontFamily, Pos2, Rect};
use points::{Mark, Points};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
};

#[cfg(target_os = "macos")]
const _: () = assert!(
    eframe::wgpu::Instance::enabled_backend_features().contains(eframe::wgpu::Backends::METAL),
    "macOSのビューワにはMetalが必要です"
);

const APP_ID: &str = "yamamuki-viewer";
const MAP_SOURCE_KEY: &str = "map_source";
const LOD_KEY: &str = "auto_lod";
const SURFACE_LOD_KEY: &str = "surface_auto_lod";
const POINT_NAMES_KEY: &str = "show_point_names";
const SURFACE_PALETTE_KEY: &str = "surface_palette";
const DEFAULT_WINDOW_SIZE: egui::Vec2 = egui::vec2(1150., 800.);
const NORMAL_WINDOW_SIZE_KEY: &str = "normal_window_size";
const NORMAL_WINDOW_POSITION_KEY: &str = "normal_window_position";

enum OpenedFile {
    Terrain(PathBuf),
    Points(Points),
    Surface(surface::Source),
}

fn read_file(path: PathBuf) -> Result<OpenedFile> {
    if points::is_sqlite(&path) {
        if yamamuki_terrain_view::is_terrain(&path)? {
            return Ok(OpenedFile::Terrain(path));
        }
        Points::try_open_sqlite(&path).and_then(|points| match points {
            Some(points) => Ok(OpenedFile::Points(points)),
            None => surface::Source::open(path).map(OpenedFile::Surface),
        })
    } else {
        Points::open(&path).map(OpenedFile::Points)
    }
}

fn lod_settings(ui: &mut egui::Ui, lod: &mut map::Lod, slider_max: u32) {
    let mut max = lod.max_level.unwrap_or(slider_max).min(slider_max);
    if ui
        .add(
            egui::Slider::new(&mut max, 0..=slider_max)
                .prefix("z")
                .text("レベル上限"),
        )
        .changed()
    {
        lod.max_level = (max < slider_max).then_some(max);
    }
    ui.add(
        egui::Slider::new(&mut lod.bias, map::Lod::BIAS_RANGE)
            .step_by(0.25)
            .text("選択基準（ズーム補正）"),
    )
    .on_hover_text("＋で細かめ、−で粗め。＋1で目標の画素密度が2倍になります。");
    if ui.button("LODを既定値に戻す").clicked() {
        *lod = map::Lod::default();
    }
}

fn observer_height(point: f64, terrain: Option<f64>, from_pin: bool, offset: f64) -> f64 {
    let terrain = terrain.unwrap_or(0.);
    let ground = if from_pin {
        terrain
    } else {
        point.max(terrain)
    };
    ground + offset
}

#[derive(Clone)]
struct Observer {
    from_pin: bool,
    point_index: Option<usize>,
    terrain_elevation: Option<f64>,
    height_revision: Option<(u32, i32, i32, u32)>,
    world: [f64; 2],
    elevation: f64,
    eye_offset: f64,
    yaw: f64,
    pitch: f64,
    fov: f64,
    saved_view: map::View,
    saved_height: f64,
}

struct Viewer {
    observer: Option<Observer>,
    pin_height_job: Option<([f64; 2], mpsc::Receiver<Result<f64>>)>,
    map_pin_elevation: Option<([f64; 2], f64)>,
    map_hit_job: Option<mpsc::Receiver<Result<Option<yamamuki_terrain_view::Hit>>>>,
    pin_observer_pending: bool,
    show_point_names: bool,
    observer_height_job: Option<(
        [f64; 2],
        Option<(u32, i32, i32, u32)>,
        mpsc::Receiver<Result<f64>>,
    )>,
    terrain: Option<yamamuki_terrain_view::TerrainView>,
    terrain_overlay: Option<PathBuf>,
    terrain_capture: Option<PathBuf>,
    show_terrain: bool,
    terrain_info: Option<terrain_info::Info>,
    flat_terrain: Option<yamamuki_terrain_view::TerrainView>,
    normal_window_size: egui::Vec2,
    normal_window_position: Option<Pos2>,
    startup_expanded_rect: Option<Rect>,
    restore_normal_size: Option<bool>,
    view: map::View,
    tiles: Vec<map::Tiles>,
    map_source: map::Source,
    lod: map::Lod,
    surface_lod: map::Lod,
    surface_palette: surface_palette::Palette,
    lod_renderer: Option<lod_fade::Renderer>,
    show_online_map: bool,
    points: Option<Arc<Points>>,
    show_point_markers: bool,
    client: reqwest::blocking::Client,
    boundaries: Arc<boundaries::BoundaryCache>,
    location_job: Option<mpsc::Receiver<Result<Vec<boundaries::Location>>>>,
    detail: point_detail::Detail,
    list: point_list::PointList,
    layer_list_height: Option<f32>,
    selected: Option<usize>,
    marker_popup: bool,
    map_pin: Option<[f64; 2]>,
    loading: Option<(PathBuf, mpsc::Receiver<Result<OpenedFile>>)>,
    show_point_file_info: bool,
    source: Option<PathBuf>,
    surfaces: Vec<surface::Surface>,
    loading_surface: Option<mpsc::Receiver<Result<surface::Source>>>,
    error: Option<String>,
    fit_pending: bool,
    marks: Vec<Mark>,
    marker_mesh: Option<Arc<egui::Mesh>>,
    marker_sprites: Option<markers::Sprites>,
    mark_view: Option<(map::View, Rect, Option<usize>)>,
}

impl Viewer {
    fn new(ctx: &egui::Context, client: reqwest::blocking::Client, file: Option<PathBuf>) -> Self {
        appearance::install(ctx);
        install_japanese_font(ctx);
        let mut viewer = Self {
            observer: None,
            pin_height_job: None,
            map_pin_elevation: None,
            map_hit_job: None,
            pin_observer_pending: false,
            show_point_names: false,
            observer_height_job: None,
            terrain: None,
            terrain_overlay: None,
            terrain_capture: None,
            show_terrain: true,
            terrain_info: None,
            flat_terrain: None,
            normal_window_size: DEFAULT_WINDOW_SIZE,
            normal_window_position: None,
            startup_expanded_rect: None,
            restore_normal_size: None,
            view: map::View {
                center: points::project(36.2, 138.3),
                zoom: 5.,
                orientation: map::Orientation::default(),
            },
            tiles: Vec::new(),
            map_source: map::Source::Gsi,
            lod: map::Lod::default(),
            surface_lod: map::Lod::default(),
            surface_palette: surface_palette::Palette::default(),
            lod_renderer: None,
            show_online_map: true,
            client,
            boundaries: Arc::default(),
            location_job: None,
            detail: point_detail::Detail::default(),
            points: None,
            show_point_markers: true,
            list: point_list::PointList::default(),
            layer_list_height: None,
            selected: None,
            marker_popup: false,
            map_pin: None,
            loading: None,
            show_point_file_info: false,
            source: None,
            surfaces: Vec::new(),
            loading_surface: None,
            error: None,
            fit_pending: false,
            marks: Vec::new(),
            marker_mesh: None,
            marker_sprites: None,
            mark_view: None,
        };
        if let Some(file) = file {
            viewer.open(ctx, file);
        }
        viewer
    }

    fn lod_slider_max(&self) -> u32 {
        self.map_source.max_tile_zoom()
    }
    fn surface_lod_slider_max(&self) -> u32 {
        self.surfaces
            .iter()
            .filter_map(surface::Surface::max_visible_level)
            .max()
            .unwrap_or(map::MAX_ZOOM as u32)
    }

    fn track_window_geometry(&mut self, ctx: &egui::Context) {
        let (expanded, minimized, size, outer_rect, monitor_size) = ctx.input(|input| {
            let window = input.viewport();
            (
                window.maximized.unwrap_or(false) || window.fullscreen.unwrap_or(false),
                window.minimized.unwrap_or(false),
                window.inner_rect.map(|rect| rect.size()),
                window.outer_rect,
                window.monitor_size,
            )
        });
        // 最大化で起動したときは、最初の解除時に通常時の位置と大きさへ戻す。
        let restore = *self.restore_normal_size.get_or_insert(expanded);
        if restore && expanded && !minimized {
            self.startup_expanded_rect = outer_rect.or(self.startup_expanded_rect);
        }
        if expanded || minimized {
            return;
        }
        if restore {
            self.restore_normal_size = Some(false);
            let size = monitor_size.map_or(self.normal_window_size, |monitor| {
                self.normal_window_size.min(monitor)
            });
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            if let (Some(position), Some(work_area)) = (
                self.normal_window_position,
                self.startup_expanded_rect.take(),
            ) {
                // モニター構成が変わっていた場合は、タイトルバーが見える位置だけを復元する。
                let title_bar = Rect::from_min_size(position, egui::vec2(size.x, 24.));
                if work_area.expand(16.).intersects(title_bar) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
                }
            }
        } else {
            if let Some(size) = size.filter(|size| size.is_finite() && size.x > 0. && size.y > 0.) {
                self.normal_window_size = size;
            }
            if let Some(rect) = outer_rect.filter(|rect| rect.min.is_finite()) {
                self.normal_window_position = Some(rect.min);
            }
        }
    }

    fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        self.loading = Some((path.clone(), receive));
        self.error = None;
        std::thread::spawn(move || {
            let result = read_file(path);
            let _ = send.send(result);
            ctx.request_repaint();
        });
    }

    fn open_surface(&mut self, ctx: &egui::Context, path: PathBuf) {
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        self.loading_surface = Some(receive);
        self.error = None;
        std::thread::spawn(move || {
            let _ = send.send(surface::Source::open(path));
            ctx.request_repaint();
        });
    }

    fn poll_surface(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.loading_surface else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!("地表の読み込みが終了しました"))
            }
        };
        self.loading_surface = None;
        match result {
            Ok(source) => self.add_surface(ctx, source),
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }

    fn add_surface(&mut self, ctx: &egui::Context, source: surface::Source) {
        if self
            .surfaces
            .iter()
            .any(|surface| surface.same_file(&source))
        {
            return;
        }
        let mut surface = surface::Surface::new(ctx, source);
        surface.palette = self.surface_palette;
        surface.fit = self.surfaces.is_empty() && self.terrain.is_none();
        surface.reveal = true;
        if surface.fit {
            self.show_online_map = false;
        }
        self.surfaces.push(surface);
    }

    fn poll_file(&mut self, ctx: &egui::Context, state: Option<&eframe::egui_wgpu::RenderState>) {
        let Some((path, receiver)) = &self.loading else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!("地点の読み込みが終了しました"))
            }
        };
        let path = path.clone();
        self.loading = None;
        match result {
            Ok(OpenedFile::Terrain(path)) => {
                self.leave_observer();
                self.pin_height_job = None;
                let combined_surface = surface::Source::open(path.clone()).ok();
                let terrain_info = terrain_info::Info::new(ctx, path.clone());
                let result = state
                    .ok_or_else(|| anyhow::anyhow!("地形表示にはGPU描画が必要です"))
                    .and_then(|state| yamamuki_terrain_view::TerrainView::new(ctx, state, path));
                match result {
                    Ok(mut view) => {
                        self.terrain_info = Some(terrain_info);
                        if let Some(path) = self.terrain_overlay.take() {
                            self.open_surface(ctx, path);
                        }
                        if let Some(output) = self.terrain_capture.take() {
                            view.capture(output);
                            if std::env::var("YAMAMUKI_TERRAIN_CHECK_MAP").as_deref() == Ok("none")
                            {
                                self.show_online_map = false;
                            }
                            self.view.center = points::project(35.3606, 138.7274);
                            self.view.zoom = std::env::var("YAMAMUKI_TERRAIN_CHECK_ZOOM")
                                .ok()
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(10.5);
                            self.view.orientation = map::Orientation::default();
                            let tilt = std::env::var("YAMAMUKI_TERRAIN_CHECK_TILT")
                                .ok()
                                .and_then(|v| v.parse::<f32>().ok())
                                .unwrap_or(0.6);
                            self.view.orientation.orbit(egui::vec2(0., -tilt / 0.006));
                        }
                        self.terrain = Some(view);
                        if let Some(source) = combined_surface {
                            self.add_surface(ctx, source);
                            self.show_online_map = false;
                        }
                    }
                    Err(error) => self.error = Some(format!("{error:#}")),
                }
            }
            Ok(OpenedFile::Surface(source)) => {
                self.add_surface(ctx, source);
            }
            Ok(OpenedFile::Points(points)) => {
                self.source = Some(path);
                self.show_point_markers = true;
                self.show_point_file_info = false;
                self.list.reset(&points);
                let points = Arc::new(points);
                self.points = Some(Arc::clone(&points));
                self.detail = point_detail::Detail::default();
                let (sender, receiver) = mpsc::channel();
                self.location_job = Some(receiver);
                let cache = Arc::clone(&self.boundaries);
                let client = self.client.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let coordinates: Vec<_> = (0..points.len())
                        .map(|i| {
                            let point = points.point(i);
                            [point.longitude, point.latitude]
                        })
                        .collect();
                    let _ = sender.send(cache.lookup(&client, &coordinates));
                    ctx.request_repaint();
                });
                self.selected = None;
                self.marker_popup = false;
                self.map_pin = None;
                self.fit_pending = true;
                self.mark_view = None;
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }

    fn close_points(&mut self) {
        if self
            .observer
            .as_ref()
            .is_some_and(|observer| !observer.from_pin)
        {
            self.leave_observer();
        }
        self.points = None;
        self.source = None;
        self.location_job = None;
        self.show_point_file_info = false;
        self.fit_pending = false;
        self.selected = None;
        self.marker_popup = false;
        self.detail = point_detail::Detail::default();
        self.list = point_list::PointList::default();
        self.marks.clear();
        self.marker_mesh = None;
        self.marker_sprites = None;
        self.mark_view = None;
    }

    fn active_terrain(&self) -> Option<&yamamuki_terrain_view::TerrainView> {
        if self.show_terrain && self.terrain.is_some() {
            self.terrain.as_ref()
        } else if self.observer.is_some() {
            self.flat_terrain.as_ref()
        } else {
            None
        }
    }
    fn active_terrain_mut(&mut self) -> Option<&mut yamamuki_terrain_view::TerrainView> {
        if self.show_terrain && self.terrain.is_some() {
            self.terrain.as_mut()
        } else if self.observer.is_some() {
            self.flat_terrain.as_mut()
        } else {
            None
        }
    }
    fn select_map_pin(&mut self, ctx: &egui::Context, world: Option<[f64; 2]>) {
        self.map_pin = world;
        self.map_pin_elevation = None;
        self.pin_observer_pending = false;
        self.pin_height_job = world.and_then(|world| {
            self.terrain
                .as_ref()
                .map(|terrain| (world, terrain.request_elevation(ctx, world)))
        });
    }
    fn enter_pin_observer(&mut self, ctx: &egui::Context) {
        let Some(world) = self.map_pin else {
            return;
        };
        if self.terrain.is_some() && self.show_terrain {
            if let Some((_, height)) = self.map_pin_elevation.filter(|(p, _)| *p == world) {
                self.enter_observer_at(world, height, true);
            } else {
                self.pin_observer_pending = true;
                if self.pin_height_job.is_none() {
                    self.pin_height_job = Some((
                        world,
                        self.terrain.as_ref().unwrap().request_elevation(ctx, world),
                    ));
                }
            }
        } else {
            self.enter_observer_at(world, 0., true);
        }
    }
    fn receive_pin_elevation(&mut self) {
        let reply = self
            .pin_height_job
            .as_ref()
            .and_then(|(world, receiver)| receiver.try_recv().ok().map(|reply| (*world, reply)));
        if let Some((world, reply)) = reply {
            self.pin_height_job = None;
            if self.map_pin == Some(world) {
                match reply {
                    Ok(height) => {
                        self.map_pin_elevation = Some((world, height));
                        if self.pin_observer_pending {
                            self.pin_observer_pending = false;
                            self.enter_observer_at(world, height, true);
                        }
                    }
                    Err(error) => {
                        self.pin_observer_pending = false;
                        self.error = Some(format!("選択座標の標高を取得できません: {error:#}"));
                    }
                }
            }
        }
    }
    fn enter_observer(&mut self) {
        let Some((world, elevation)) =
            self.selected
                .zip(self.points.as_ref())
                .and_then(|(index, points)| {
                    let point = points.point(index);
                    point
                        .elevation_m
                        .filter(|v| v.is_finite())
                        .map(|z| (points::project(point.latitude, point.longitude), z))
                })
        else {
            return;
        };
        self.enter_observer_at(world, elevation, false);
    }
    fn enter_observer_at(&mut self, world: [f64; 2], elevation: f64, from_pin: bool) {
        let terrain = self.terrain.as_ref().filter(|_| self.show_terrain);
        self.observer = Some(Observer {
            from_pin,
            point_index: if from_pin { None } else { self.selected },
            terrain_elevation: if terrain.is_none() {
                Some(0.)
            } else {
                from_pin.then_some(elevation)
            },
            height_revision: if from_pin {
                terrain.and_then(|t| t.elevation_revision(world))
            } else {
                None
            },
            world,
            elevation,
            eye_offset: 2.,
            yaw: self.view.orientation.angles().0,
            pitch: 0.,
            fov: 45f64.to_radians(),
            saved_view: self.view,
            saved_height: terrain.map_or(0., |t| t.camera_height()),
        });
        self.fit_pending = false;
        self.map_pin = None;
        self.marker_popup = false;
        self.mark_view = None;
    }
    fn leave_observer(&mut self) {
        self.observer_height_job = None;
        if let Some(observer) = self.observer.take() {
            self.view = observer.saved_view;
            if let Some(terrain) = self.terrain.as_mut() {
                terrain.restore_camera_height(observer.saved_height);
            }
            self.mark_view = None;
            self.marker_mesh = None;
        }
    }
    fn refresh_observer_point(&mut self) {
        if self.observer.is_none() {
            return;
        }
        if self
            .observer
            .as_ref()
            .is_some_and(|observer| observer.from_pin)
        {
            return;
        }
        let point = self
            .selected
            .zip(self.points.as_ref())
            .and_then(|(index, points)| {
                let point = points.point(index);
                point
                    .elevation_m
                    .filter(|v| v.is_finite())
                    .map(|z| (points::project(point.latitude, point.longitude), z))
            });
        if point.is_none() {
            self.leave_observer();
            return;
        }
        let (world, elevation) = point.unwrap();
        let observer = self.observer.as_mut().unwrap();
        if observer.world != world || observer.point_index != self.selected {
            observer.eye_offset = 2.;
            observer.terrain_elevation = None;
            observer.height_revision = None;
        }
        observer.point_index = self.selected;
        observer.world = world;
        observer.elevation = elevation;
    }
    fn update_observer_height(&mut self, ctx: &egui::Context) {
        if !self.show_terrain || self.terrain.is_none() {
            self.observer_height_job = None;
            if let Some(observer) = self.observer.as_mut() {
                observer.terrain_elevation = Some(0.);
                observer.height_revision = None;
            }
            return;
        }
        let reply = self
            .observer_height_job
            .as_ref()
            .and_then(|(world, revision, receiver)| {
                receiver
                    .try_recv()
                    .ok()
                    .map(|reply| (*world, *revision, reply))
            });
        if let Some((world, revision, reply)) = reply {
            self.observer_height_job = None;
            if let Some(observer) = self
                .observer
                .as_mut()
                .filter(|observer| observer.world == world)
            {
                match reply {
                    Ok(height) => {
                        observer.terrain_elevation = Some(height);
                        observer.height_revision = revision;
                    }
                    Err(error) => {
                        self.error = Some(format!("地形標高を取得できません: {error:#}"));
                        self.leave_observer();
                    }
                }
            }
        }
        if let (Some(observer), Some(terrain)) = (self.observer.as_ref(), self.terrain.as_ref()) {
            let revision = terrain.elevation_revision(observer.world);
            if self.observer_height_job.is_none()
                && (observer.terrain_elevation.is_none() || observer.height_revision != revision)
            {
                self.observer_height_job = Some((
                    observer.world,
                    revision,
                    terrain.request_elevation(ctx, observer.world),
                ));
            }
        }
    }
    fn paint_base(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        view: map::View,
        dragging: bool,
        frame: &eframe::Frame,
        material_pass: usize,
        zoom_override: Option<f64>,
    ) -> bool {
        let ctx = ui.ctx();
        let lod_zoom = zoom_override.unwrap_or_else(|| {
            terrain_material::lod_zoom(self.view.zoom, view.zoom, material_pass)
        });
        let fade_backend = frame.wgpu_render_state().map(|state| {
            let renderer = self
                .lod_renderer
                .get_or_insert_with(|| lod_fade::Renderer::new(state));
            (state, &*renderer)
        });
        let failed = if self.show_online_map {
            if let Some(index) = self
                .tiles
                .iter()
                .position(|tiles| tiles.source == self.map_source)
            {
                self.tiles.swap(0, index);
            } else {
                self.tiles.truncate(1);
                self.tiles.insert(
                    0,
                    map::Tiles::new(ctx, self.client.clone(), self.map_source),
                );
            }
            self.tiles[0].select_material_pass(material_pass);
            self.tiles[0].paint(
                ui,
                painter,
                rect,
                &view,
                dragging,
                self.map_source.tile_zoom(lod_zoom, self.lod),
                lod_zoom,
                fade_backend,
            )
        } else {
            false
        };
        let mut has_base = self.show_online_map;
        for surface in self.surfaces.iter_mut().filter(|surface| surface.enabled) {
            surface.select_material_pass(material_pass);
            surface.paint(
                ui,
                painter,
                rect,
                &view,
                dragging,
                has_base,
                fade_backend,
                self.surface_lod,
                lod_zoom,
            );
            has_base |= surface.has_enabled_layers();
        }
        failed
    }
    fn terrain_base(
        &mut self,
        ui: &egui::Ui,
        frame: &eframe::Frame,
        rect: Rect,
        dragging: bool,
    ) -> bool {
        let Some(state) = frame.wgpu_render_state() else {
            return false;
        };
        let enabled = self.show_online_map
            || self
                .surfaces
                .iter()
                .any(|s| s.enabled && s.has_enabled_layers());
        if !enabled {
            self.active_terrain_mut()
                .unwrap()
                .set_materials([None, None, None]);
            return false;
        }
        // これはタイル取得の座標系だけで、画像へのラスタライズは行わない。
        let size = rect.width().max(rect.height()).max(1.);
        let material_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(size, size));
        let density = self
            .surfaces
            .iter()
            .filter(|s| s.enabled)
            .filter_map(|s| s.material_max_zoom(self.surface_lod))
            .chain(self.show_online_map.then(|| {
                self.map_source
                    .max_tile_zoom()
                    .min(self.lod.max_level.unwrap_or(u32::MAX)) as f64
            }))
            .max_by(f64::total_cmp)
            .unwrap_or(18.);
        let minimum_span = terrain_material::minimum_span(size as f64, density);
        let areas = self
            .active_terrain()
            .unwrap()
            .material_regions(rect, minimum_span);
        let observer_bands = self.observer.is_some().then(|| {
            self.active_terrain()
                .unwrap()
                .observer_material_regions(rect, minimum_span)
        });
        let areas: Vec<([f64; 4], Option<f64>)> = observer_bands.map_or_else(
            || areas.into_iter().map(|a| (a, None)).collect(),
            |v| v.into_iter().map(|(a, z)| (a, Some(z))).collect(),
        );
        let mut batches = vec![Vec::new(); areas.len()];
        let mut failed = false;
        // 近景を先に取得し、粗い背景から詳細へ重ねる。
        for i in (1..areas.len()).chain(std::iter::once(0)) {
            let (area, zoom) = areas[i];
            let view = terrain_material::map_view(area, size as f64);
            let layer = egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new(("terrain-material", i)),
            );
            let painter = egui::Painter::new(ui.ctx().clone(), layer, material_rect);
            failed |= self.paint_base(
                ui,
                &painter,
                material_rect,
                view,
                dragging,
                frame,
                i + 1,
                zoom,
            );
            let shapes = ui.ctx().graphics_mut(|graphics| {
                let list = graphics.entry(layer);
                let shapes = list.all_entries().cloned().collect::<Vec<_>>();
                *list = Default::default();
                shapes
            });
            let ready = (!self.show_online_map
                || self
                    .tiles
                    .first()
                    .is_some_and(|tiles| tiles.material_ready()))
                && self
                    .surfaces
                    .iter()
                    .filter(|surface| surface.enabled && surface.has_enabled_layers())
                    .all(|surface| surface.material_ready());
            let [a, b] = view.area(material_rect);
            // 未完了の近・中景で、取得済みの遠景を空白色へ戻さない。
            if i == 0 || ready {
                batches[i].push(yamamuki_terrain_view::direct_material::Quad {
                    bounds: [a[0], a[1], b[0], b[1]],
                    uv: [lod_fade::FULL_UV; 2],
                    textures: [None, None],
                    amount: 1.,
                    tint: appearance::CANVAS.to_array().map(|v| f32::from(v) / 255.),
                    shade: false,
                    gamma_shade: false,
                });
            }
            batches[i].extend(terrain_material::collect(
                ui.ctx(),
                state,
                self.lod_renderer.as_ref().unwrap(),
                shapes,
                &view,
                material_rect,
            ));
        }
        // 全範囲で使うタイルを同じフレームとして保護し、最後に一度だけ整理する。
        if self.show_online_map {
            if let Some(tiles) = self.tiles.first_mut() {
                tiles.finish_material_frame();
            }
        }
        for surface in self.surfaces.iter_mut().filter(|surface| surface.enabled) {
            surface.finish_material_frame();
        }
        let commands = std::iter::once(0)
            .chain((1..batches.len()).rev())
            .flat_map(|i| std::mem::take(&mut batches[i]))
            .collect();
        self.active_terrain_mut()
            .unwrap()
            .set_direct_materials(commands);
        failed
    }

    fn paint_markers(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        pointer: Option<Pos2>,
        clicked: bool,
    ) -> bool {
        if !self.show_point_markers {
            return false;
        }
        let Some(points) = &self.points else {
            return false;
        };
        let view = self.view;
        let projection = self.active_terrain().map(|t| t.projection());
        let screen = |mark: points::Mark| {
            projection.as_ref().map_or_else(
                || view.screen(rect, mark.world),
                |p| {
                    p.screen(rect, mark.world, 2.)
                        .unwrap_or(egui::pos2(f32::INFINITY, f32::INFINITY))
                },
            )
        };
        let current = (self.view, rect, self.selected);
        let view_changed = self.mark_view != Some(current);
        if view_changed {
            self.marker_mesh = None;
            // 地上視点では俯瞰地図の範囲外にある遠方の山も画面に入る。
            let area = if projection.is_some() {
                points.bounds
            } else {
                self.view.area(rect)
            };
            let mut marks = points.filtered_marks(area, self.view.scale(), self.list.prefix());
            if projection.is_none() && self.view.orientation != map::Orientation::default() {
                marks.retain(|mark| rect.contains(screen(*mark)));
            }
            self.marks = markers::displayed(marks, points.len(), self.selected);
            self.mark_view = Some(current);
        }
        let mut visible_marks = self.marks.clone();
        let mut projected = std::collections::HashMap::new();
        if self.show_terrain {
            if let Some(terrain) = self.terrain.as_ref() {
                let queries = self
                    .marks
                    .iter()
                    .map(|mark| [mark.world[0], mark.world[1], 0.])
                    .collect::<Vec<_>>();
                if let Some(result) = terrain.point_projection(painter.ctx(), &queries) {
                    visible_marks = self
                        .marks
                        .iter()
                        .zip(result)
                        .filter_map(|(mark, value)| {
                            if value[3] < 0.5 {
                                return None;
                            }
                            let position = egui::pos2(
                                rect.left() + value[0] * rect.width(),
                                rect.top() + value[1] * rect.height(),
                            );
                            if !position.is_finite() || !rect.contains(position) {
                                return None;
                            }
                            projected.insert(mark.index, position);
                            Some(*mark)
                        })
                        .collect();
                    visible_marks = markers::displayed(visible_marks, points.len(), self.selected);
                } else {
                    visible_marks.clear();
                }
                self.marker_mesh = None;
            }
        }
        let marks = visible_marks.as_slice();
        let screen = |mark: points::Mark| {
            projected
                .get(&mark.index)
                .copied()
                .unwrap_or_else(|| screen(mark))
        };
        let count = self.list.visible_count();
        if !marks.is_empty() {
            let sprites = markers::Sprites::update(
                &mut self.marker_sprites,
                painter.ctx(),
                self.view.zoom,
                count,
            );
            if self
                .marker_mesh
                .as_ref()
                .is_none_or(|mesh| mesh.texture_id != sprites.texture_id())
            {
                let positions = markers::in_paint_order(marks, self.selected).filter_map(|mark| {
                    let position = screen(*mark);
                    (position.is_finite() && rect.expand(32.).contains(position)).then_some((
                        position,
                        mark.matched,
                        self.selected == Some(mark.index),
                    ))
                });
                self.marker_mesh = Some(Arc::new(sprites.mesh(positions, marks.len())));
            }
            if let Some(mesh) = &self.marker_mesh {
                painter.add(egui::Shape::mesh(Arc::clone(mesh)));
            }
        }

        if self.show_point_names {
            let mut occupied = Vec::<Rect>::new();
            for mark in markers::in_paint_order(marks, self.selected).rev() {
                if occupied.len() >= 400 {
                    break;
                }
                let position = screen(*mark);
                if !rect.contains(position) {
                    continue;
                }
                let galley = painter.layout_no_wrap(
                    points.point(mark.index).name.clone(),
                    egui::FontId::proportional(13.),
                    appearance::INK,
                );
                let origin = position + egui::vec2(7., -galley.size().y / 2.);
                let label = Rect::from_min_size(origin, galley.size()).expand(2.);
                if !rect.intersects(label) || occupied.iter().any(|other| other.intersects(label)) {
                    continue;
                }
                occupied.push(label);
                painter.rect_filled(label, 2., Color32::from_white_alpha(210));
                painter.galley(origin, galley, appearance::INK);
            }
        }
        let hovered = pointer.and_then(|pointer| {
            markers::in_paint_order(marks, self.selected)
                .rev()
                .find(|mark| {
                    let position = screen(**mark);
                    let (radius, _, stroke) = markers::style(
                        self.view.zoom,
                        mark.matched,
                        count,
                        self.selected == Some(mark.index),
                    );
                    pointer.distance_sq(position) <= (radius + stroke.width / 2.).powi(2)
                })
                .copied()
        });
        if clicked {
            self.marker_popup = hovered.is_some();
            if let Some(mark) = hovered {
                // 同じ地点の再クリックでも、手動で変更した高さをリセットする。
                if let Some(observer) = &mut self.observer {
                    observer.eye_offset = 2.;
                }
                self.selected = Some(mark.index);
                self.list.reveal(mark.index);
            }
            painter.ctx().request_repaint();
        }
        if self.marker_popup
            && let Some(index) = self
                .selected
                .filter(|index| marks.iter().any(|mark| mark.index == *index))
        {
            let point = points.point(index);
            let world = points::project(point.latitude, point.longitude);
            let position = projected
                .get(&index)
                .copied()
                .unwrap_or_else(|| view.screen(rect, world));
            if rect.contains(position) {
                let elevation = point
                    .elevation_m
                    .map_or("不明".into(), |value| format!("{value} m"));
                Self::paint_marker_label(
                    painter,
                    rect,
                    position,
                    format!(
                        "{}\n{}\n標高 {}　ID {}",
                        point.name,
                        coordinate_label([point.latitude, point.longitude]),
                        elevation,
                        point.id
                    ),
                    true,
                    None,
                );
            }
        }
        if let Some(mark) = hovered.filter(|m| !self.marker_popup || Some(m.index) != self.selected)
        {
            Self::paint_marker_label(
                painter,
                rect,
                screen(mark),
                points.point(mark.index).name.clone(),
                false,
                None,
            );
        }
        clicked && hovered.is_some()
    }

    fn paint_marker_label(
        painter: &egui::Painter,
        rect: Rect,
        position: Pos2,
        text: String,
        popup: bool,
        pin_controls: Option<(&egui::Ui, [f64; 2])>,
    ) -> bool {
        const CLOSE_WIDTH: f32 = 24.;
        const COPY_WIDTH: f32 = 46.;
        let controls_width = if pin_controls.is_some() {
            CLOSE_WIDTH + COPY_WIDTH
        } else {
            0.
        };
        let label = painter.layout(
            text,
            egui::FontId::proportional(14.),
            appearance::INK,
            (rect.width() - 16. - controls_width).max(1.),
        );
        let size = label.size() + egui::vec2(controls_width, 0.);
        let x = if popup {
            position.x - size.x / 2.
        } else {
            position.x + 7.
        };
        let y = position.y - label.size().y - if popup { 17. } else { 6. };
        let box_pos = egui::pos2(
            x.min(rect.right() - size.x - 5.).max(rect.left() + 5.),
            y.max(rect.top() + 5.),
        );
        let card = Rect::from_min_size(box_pos, size).expand(4.);
        let controls = if let Some((ui, [latitude, longitude])) = pin_controls {
            let button = |left, right, id, hint| {
                let rect = Rect::from_min_max(
                    egui::pos2(left, card.top()),
                    egui::pos2(right, card.bottom()),
                );
                let response = ui
                    .interact(rect, ui.id().with(id), egui::Sense::click())
                    .on_hover_text(hint);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, hint)
                });
                response
            };
            let split = card.right() - CLOSE_WIDTH;
            let close = button(split, card.right(), "map-pin-close", "座標を閉じる");
            if close.clicked() {
                return true;
            }
            let copy = button(
                split - COPY_WIDTH,
                split,
                "map-pin-copy",
                "緯度・経度をコピー",
            );
            if copy.clicked() {
                ui.ctx().copy_text(format!("{latitude:.6}, {longitude:.6}"));
            }
            Some([(copy, "コピー", 12.), (close, "×", 16.)])
        } else {
            None
        };
        painter.rect_filled(card, 6, appearance::PAPER);
        painter.rect_stroke(
            card,
            6,
            egui::Stroke::new(1_f32, appearance::LINE),
            egui::StrokeKind::Inside,
        );
        painter.galley(box_pos, label, appearance::INK);
        if let Some(controls) = controls {
            for (response, text, font_size) in controls {
                let color = if response.hovered() || response.has_focus() {
                    appearance::ACCENT
                } else {
                    appearance::MUTED
                };
                painter.text(
                    response.rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(font_size),
                    color,
                );
            }
        }
        false
    }
}

impl eframe::App for Viewer {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, MAP_SOURCE_KEY, &self.map_source);
        eframe::set_value(storage, LOD_KEY, &self.lod);
        eframe::set_value(storage, SURFACE_LOD_KEY, &self.surface_lod);
        eframe::set_value(storage, SURFACE_PALETTE_KEY, &self.surface_palette);
        eframe::set_value(storage, POINT_NAMES_KEY, &self.show_point_names);
        eframe::set_value(storage, NORMAL_WINDOW_SIZE_KEY, &self.normal_window_size);
        if let Some(position) = self.normal_window_position {
            eframe::set_value(storage, NORMAL_WINDOW_POSITION_KEY, &position);
        }
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.track_window_geometry(ctx);
        self.poll_file(ctx, frame.wgpu_render_state());
        self.poll_surface(ctx);
        if let Some(receiver) = &self.location_job {
            match receiver.try_recv() {
                Ok(Ok(locations)) => {
                    if let Some(points) = &self.points {
                        self.list.set_locations(points, locations);
                    }
                    self.location_job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                _ => self.location_job = None,
            }
        }
        if self.loading.is_none() && self.loading_surface.is_none() {
            if let Some(path) =
                ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()))
            {
                self.open(ctx, path);
            }
        }
        self.receive_pin_elevation();
        if let Some(reply) = self.map_hit_job.as_ref().and_then(|r| r.try_recv().ok()) {
            self.map_hit_job = None;
            match reply {
                Ok(Some(hit)) => {
                    self.map_pin = Some(hit.world);
                    self.map_pin_elevation = Some((hit.world, hit.height));
                }
                Ok(None) => {}
                Err(error) => self.error = Some(format!("クリック位置を取得できません: {error:#}")),
            }
        }
        egui::TopBottomPanel::top("tools")
            .frame(appearance::panel(appearance::PAPER, 16, 10))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("山むき")
                            .color(appearance::ACCENT)
                            .strong()
                            .size(22.),
                    );
                    ui.label(egui::RichText::new("データビューワ").color(appearance::MUTED));
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.loading_surface.is_none(),
                            egui::Button::new(
                                egui::RichText::new("地表SQLiteを開く")
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(appearance::ACCENT),
                        )
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("地表SQLite", &["sqlite3", "sqlite", "db"])
                            .pick_file()
                    {
                        self.open_surface(ctx, path);
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("地形SQLiteを開く")
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(appearance::ACCENT),
                        )
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("地形SQLite", &["sqlite3", "sqlite", "db"])
                            .pick_file()
                    {
                        self.open(ctx, path);
                    }
                    if self.loading_surface.is_some() {
                        ui.spinner();
                        ui.label("地表を読み込み中");
                    }
                    if ui
                        .add_enabled(
                            self.loading.is_none(),
                            egui::Button::new(
                                egui::RichText::new("地点ファイルを開く")
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(appearance::ACCENT),
                        )
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("地点データ", &["json", "gz", "sqlite3", "sqlite", "db"])
                            .pick_file()
                        {
                            self.open(ctx, path);
                        }
                    }
                    if self.loading.is_some() {
                        ui.spinner();
                        ui.label("データを読み込み中");
                    }
                    ui.menu_button("オンライン地図の表示設定", |ui| {
                        ui.strong("自動LOD");
                        let maximum = self.lod_slider_max();
                        lod_settings(ui, &mut self.lod, maximum);
                    });
                    ui.menu_button("地表の表示設定", |ui| {
                        ui.strong("自動LOD");
                        let maximum = self.surface_lod_slider_max();
                        lod_settings(ui, &mut self.surface_lod, maximum);
                        ui.separator();
                        ui.strong("配色");
                        if self.surface_palette.menu(ui) {
                            for surface in &mut self.surfaces {
                                surface.palette = self.surface_palette;
                            }
                        }
                    });
                    if let Some(terrain) = &mut self.terrain {
                        ui.menu_button("地形の表示設定", |ui| {
                            ui.strong("自動LOD");
                            terrain.lod_controls(ui);
                            ui.separator();
                            terrain.display_controls(ui);
                        });
                    }
                    ui.menu_button("地点の表示設定", |ui| {
                        ui.checkbox(&mut self.show_point_names, "地点名を常時表示")
                            .on_hover_text("丸印の横に名前を表示します。重なる名前は省きます。");
                    });
                    if self.observer.is_some() {
                        if ui.button("俯瞰視点に戻る").clicked() {
                            self.leave_observer();
                        }
                    } else {
                        let available = self.selected.zip(self.points.as_ref()).is_some_and(
                            |(index, points)| {
                                points.point(index).elevation_m.is_some_and(f64::is_finite)
                            },
                        );
                        if ui
                            .add_enabled(available, egui::Button::new("選択地点から見る"))
                            .on_hover_text("標高のある地点を選択してください")
                            .clicked()
                        {
                            self.enter_observer();
                        }
                    }
                    if self.observer.is_none() {
                        if ui
                            .add_enabled(
                                self.map_pin.is_some() && self.pin_height_job.is_none(),
                                egui::Button::new("選択座標から見る"),
                            )
                            .on_hover_text("地図をクリックして座標を選択してください")
                            .clicked()
                        {
                            self.enter_pin_observer(ctx);
                        }
                        if self.pin_height_job.is_some() {
                            ui.spinner();
                            ui.label("標高を取得中");
                        }
                    }
                    if let Some(observer) = self.observer.as_mut() {
                        ui.menu_button("地点視点の設定", |ui| {
                            if !observer.from_pin {
                                ui.label(format!("地点標高 {:.1} m", observer.elevation));
                            }
                            if let Some(height) = observer.terrain_elevation {
                                ui.label(format!("表示地形の標高 {height:.1} m"));
                            } else {
                                ui.label("地形標高を取得中");
                            }
                            ui.add(
                                egui::Slider::new(&mut observer.eye_offset, 0.5..=200.)
                                    .clamping(egui::SliderClamping::Never)
                                    .text("加算する高さ（m）"),
                            );
                            let mut degrees = observer.fov.to_degrees();
                            if ui
                                .add(
                                    egui::Slider::new(&mut degrees, 15.0..=100.0)
                                        .text("視野角（度）"),
                                )
                                .changed()
                            {
                                observer.fov = degrees.to_radians();
                            }
                            ui.label("左ドラッグ：見回す ／ 右ドラッグ：左右で向き・上下で高さ ／ ホイール：視野角");
                        });
                    }
                });
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            });
        egui::TopBottomPanel::bottom("credits")
            .frame(appearance::panel(appearance::CANVAS, 14, 5))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if self.show_online_map {
                        self.map_source.attribution(ui, self.map_source.tile_zoom(self.view.zoom, self.lod));
                    }
                    let mut shown = std::collections::HashSet::new();
                    for surface in self.surfaces.iter().filter(|surface| surface.enabled) {
                        surface.credits(ui, &mut shown);
                    }
                    if self.points.is_some() {
                        ui.hyperlink_to(
                            "行政区域：国土数値情報（共同通信社加工）",
                            boundaries::SOURCE,
                        );
                    }
                    ui.separator();
                    ui.label(
                        egui::RichText::new(
                            if self.observer.is_some() {"地点視点：左ドラッグで見回す　／　右ドラッグは左右で向き・上下で高さ　／　ホイールで視野角"} else {"左ドラッグで移動　／　右ドラッグで回転・傾き　／　ホイールで拡大縮小　／　クリックで座標"},
                        )
                        .color(appearance::MUTED)
                        .small(),
                    );
                });
            });
        let has_points = self.points.is_some();
        let (panel_id, default_width, max_width) = if has_points {
            ("points-list", 510., 1000.)
        } else {
            ("layers-list", 370., 600.)
        };
        let mut close_points = false;
        let mut close_terrain = false;
        egui::SidePanel::left(panel_id)
            .resizable(true)
            .default_width(default_width)
            .width_range(240. ..=max_width)
            .frame(appearance::panel(appearance::PAPER, 12, 10))
            .show(ctx, |ui| {
                const MIN_LAYER_HEIGHT: f32 = 52.;
                const POINT_LIST_RESERVE: f32 = 180.;
                let available_height = ui.available_height();
                let max_layer_height =
                    (available_height - POINT_LIST_RESERVE).max(MIN_LAYER_HEIGHT);
                let layer_height = self
                    .layer_list_height
                    .unwrap_or_else(|| (available_height * 0.4).min(200.))
                    .clamp(MIN_LAYER_HEIGHT, max_layer_height);
                let layers = egui::CollapsingHeader::new("レイヤー一覧")
                    .default_open(true)
                    .show(ui, |ui| {
                        let point_layer = self.source.as_deref().zip(self.points.as_deref()).map(
                            |(path, points)| surface::PointLayer {
                                path,
                                count: points.len(),
                                enabled: &mut self.show_point_markers,
                                close: &mut close_points,
                                show_info: &mut self.show_point_file_info,
                            },
                        );
                        let close_surface = surface::layer_list(
                            ui,
                            &mut self.surfaces,
                            point_layer,
                            &mut self.map_source,
                            &mut self.show_online_map,
                            self.view.zoom,
                            if has_points {
                                layer_height
                            } else {
                                f32::INFINITY
                            },
                            self.lod,
                            self.surface_lod,
                        );
                        if let Some(terrain) = &mut self.terrain {
                            ui.separator();
                            // 他のデータセットと同じ三列。LODだけ名前の下段へ置く。
                            egui_extras::TableBuilder::new(ui)
                                .id_salt("terrain-layer-row")
                                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                                .column(egui_extras::Column::exact(38.))
                                .column(egui_extras::Column::remainder().clip(true))
                                .column(egui_extras::Column::exact(146.))
                                .body(|mut body| {
                                    body.row(24., |mut row| {
                                        row.col(|ui| {
                                            if ui.checkbox(&mut self.show_terrain, "").changed() {
                                                self.mark_view = None;
                                                self.marker_mesh = None;
                                            }
                                        });
                                        row.col(|ui| {
                                            ui.add(
                                                egui::Label::new(terrain.label())
                                                    .truncate()
                                                    .selectable(false),
                                            )
                                            .on_hover_text(terrain.label());
                                        });
                                        row.col(|ui| {
                                            if ui
                                                .button("i")
                                                .on_hover_text("地形ファイルの情報")
                                                .clicked()
                                            {
                                                if let Some(info) = &mut self.terrain_info {
                                                    info.open = true;
                                                }
                                            }
                                            close_terrain = ui
                                                .button("閉じる")
                                                .on_hover_text("地形データを閉じる")
                                                .clicked();
                                        });
                                    });
                                    body.row(24., |mut row| {
                                        row.col(|_| {});
                                        row.col(|ui| {
                                            ui.add_enabled_ui(self.show_terrain, |ui| {
                                                terrain.layer_lod(ui)
                                            });
                                        });
                                        row.col(|_| {});
                                    });
                                });
                        }

                        if has_points {
                            let remaining = layer_height - ui.min_rect().height();
                            if remaining > 0. {
                                ui.add_space(remaining);
                            }
                        }
                        close_surface
                    });
                if let Some(index) = layers.body_returned.flatten() {
                    self.surfaces.remove(index);
                    if self.surfaces.is_empty() {
                        self.show_online_map = true;
                    }
                    ctx.request_repaint();
                }
                if close_points {
                    self.close_points();
                }
                if close_terrain {
                    self.leave_observer();
                    self.terrain = None;
                    self.terrain_info = None;
                    self.terrain_overlay = None;
                    self.terrain_capture = None;
                    self.mark_view = None;
                    self.marker_mesh = None;
                    ctx.request_repaint();
                }
                if let Some(points) = &self.points {
                    if layers.fully_open() {
                        let (_, rect) = ui.allocate_space(egui::vec2(ui.available_width(), 8.));
                        let splitter = ui
                            .interact(
                                rect,
                                egui::Id::new("layer-point-splitter"),
                                egui::Sense::drag(),
                            )
                            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
                        let delta = splitter.drag_delta().y;
                        if delta != 0. {
                            self.layer_list_height = Some(
                                (layer_height + delta).clamp(MIN_LAYER_HEIGHT, max_layer_height),
                            );
                            ctx.request_repaint();
                        }
                        let color = if splitter.hovered() || splitter.dragged() {
                            appearance::ACCENT
                        } else {
                            appearance::LINE
                        };
                        ui.painter()
                            .hline(rect.x_range(), rect.center().y, (1., color));
                    } else {
                        ui.separator();
                    }
                    let (selected, changed) = self.list.show(ui, points, self.selected);
                    if changed {
                        self.mark_view = None;
                    }
                    if let Some(index) = selected {
                        self.selected = Some(index);
                        self.fit_pending = false;
                        let point = points.point(index);
                        self.view.center = points::project(point.latitude, point.longitude);
                    }
                }
            });
        if let Some(info) = &mut self.terrain_info {
            info.show(ctx);
        }
        for surface in &mut self.surfaces {
            surface.show_file_info(ctx);
        }
        if self.show_point_file_info
            && let (Some(points), Some(path)) = (&self.points, &self.source)
        {
            let info = &points.info;
            egui::Window::new("地点ファイルの情報")
                .open(&mut self.show_point_file_info)
                .default_size([550., 400.])
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if let Some(dataset) = &info.dataset {
                            ui.strong(&dataset.title);
                            ui.label(format!("データセットID {}", dataset.id));
                            ui.label(format!("データ版 {}", dataset.data_version));
                            ui.label(format!("地点の格納形式 版{}", dataset.format_version));
                        }
                        ui.label(path.display().to_string());
                        ui.label(format!("形式 {}", info.format));
                        ui.label(format!("収録地点数 {}", points.len()));
                        if let Some(bytes) = info.size_bytes {
                            ui.label(format!("ファイルサイズ {bytes} B"));
                        }
                        ui.separator();
                        ui.strong("Metadata JSON");
                        if let Some(json) = &info.metadata_json {
                            appearance::json_view(ui, json);
                        } else {
                            ui.label("メタデータの収録なし");
                        }
                    });
                });
        }
        if let (Some(index), Some(points)) = (self.selected, &self.points) {
            match self.detail.show(ctx, points, &self.list, index) {
                Some(point_detail::Action::Close) => {
                    self.selected = None;
                    self.marker_popup = false;
                    self.detail = point_detail::Detail::default();
                }
                Some(point_detail::Action::Select(target)) => {
                    if self.list.clear_filter_to_reveal(points, target) {
                        self.mark_view = None;
                    }
                    self.list.reveal(target);
                    self.selected = Some(target);
                    self.fit_pending = false;
                    let point = points.point(target);
                    self.view.center = points::project(point.latitude, point.longitude);
                    ctx.request_repaint();
                }
                None => {}
            }
        }
        self.refresh_observer_point();
        self.update_observer_height(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(appearance::CANVAS).inner_margin(8))
            .show(ctx, |ui| {
                let (response, painter) =
                    ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
                let rect = response.rect;
                if self.fit_pending
                    && let Some(points) = &self.points
                {
                    self.view.fit(rect, points.bounds);
                    self.fit_pending = false;
                }
                for surface in &mut self.surfaces {
                    surface.fit_view(&mut self.view, rect);
                }
                let compass_rect = Rect::from_center_size(
                    rect.right_top() + egui::vec2(-32., 32.),
                    egui::vec2(48., 48.),
                );
                let compass = ui
                    .interact(
                        compass_rect,
                        ui.id().with("map-compass"),
                        egui::Sense::click(),
                    )
                    .on_hover_text(if self.observer.is_some() {
                        "北向き・水平に戻す"
                    } else {
                        "北を上にして真上からの表示に戻す"
                    });
                if compass.clicked() {
                    if let Some(observer) = self.observer.as_mut() {
                        observer.yaw = 0.;
                        observer.pitch = 0.;
                    } else {
                        self.view.orientation = map::Orientation::default();
                    }
                }
                let dragging = response.dragged();
                let rotating = response.dragged_by(egui::PointerButton::Secondary);
                if dragging {
                    let d = ctx.input(|input| {
                        if response.drag_started() {
                            input
                                .pointer
                                .interact_pos()
                                .zip(input.pointer.press_origin())
                                .map_or(input.pointer.delta(), |(position, origin)| {
                                    position - origin
                                })
                        } else {
                            input.pointer.delta()
                        }
                    });
                    if let Some(observer) = self.observer.as_mut() {
                        observer.yaw -= f64::from(d.x) * 0.006;
                        if rotating {
                            // 上へドラッグすると視点を上げる。見上げ・見下ろしは左ドラッグで行う。
                            observer.eye_offset -= f64::from(d.y);
                        } else {
                            observer.pitch =
                                (observer.pitch + f64::from(d.y) * 0.006).clamp(-1.5, 1.5);
                        }
                    } else if rotating {
                        self.view.orientation.orbit(d);
                    } else if response.dragged_by(egui::PointerButton::Primary) {
                        self.view.pan(d);
                    }
                }
                let pointer = response.hover_pos();
                if let Some(pointer) = pointer {
                    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0. {
                        if let Some(observer) = self.observer.as_mut() {
                            observer.fov = (observer.fov * (-f64::from(scroll) * 0.002).exp())
                                .clamp(15f64.to_radians(), 100f64.to_radians());
                        } else if self.show_terrain && self.terrain.is_some() {
                            let terrain = self.terrain.as_mut().unwrap();
                            let (yaw, tilt) = self.view.orientation.angles();
                            terrain.set_map_camera(
                                self.view.center,
                                self.view.zoom,
                                yaw,
                                tilt,
                                rect,
                            );
                            let anchor = terrain.projection().world_at(rect, pointer);
                            // 2Dの逆投影を混ぜず、3Dの同じ注視平面上で拡縮する。
                            self.view.zoom_at_terrain(
                                rect,
                                rect.center(),
                                f64::from(scroll) / 180.,
                            );
                            terrain.set_map_camera(
                                self.view.center,
                                self.view.zoom,
                                yaw,
                                tilt,
                                rect,
                            );
                            if let (Some(anchor), Some(after)) =
                                (anchor, terrain.projection().world_at(rect, pointer))
                            {
                                for axis in 0..2 {
                                    self.view.center[axis] =
                                        (self.view.center[axis] + anchor[axis] - after[axis])
                                            .clamp(0., 1.);
                                }
                            }
                        } else {
                            self.view.zoom_at(rect, pointer, f64::from(scroll) / 180.);
                        }
                    }
                }
                painter.rect_filled(rect, 0., appearance::CANVAS);
                if self.observer.is_some()
                    && (!self.show_terrain || self.terrain.is_none())
                    && self.flat_terrain.is_none()
                {
                    if let Some(state) = frame.wgpu_render_state() {
                        match yamamuki_terrain_view::TerrainView::flat(ctx, state) {
                            Ok(flat) => self.flat_terrain = Some(flat),
                            Err(error) => {
                                self.error = Some(format!("景色の平面を準備できません: {error:#}"));
                                self.leave_observer();
                            }
                        }
                    }
                }
                let failed = if self.active_terrain().is_some() {
                    if let Some(observer) = self
                        .observer
                        .as_ref()
                        .filter(|observer| observer.terrain_elevation.is_some())
                        .cloned()
                    {
                        self.view.center = observer.world;
                        self.view.zoom = self.active_terrain_mut().unwrap().set_observer_camera(
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
                        self.terrain.as_mut().unwrap().set_map_camera(
                            self.view.center,
                            self.view.zoom,
                            yaw,
                            tilt,
                            rect,
                        );
                    }
                    let failed = self.terrain_base(ui, frame, rect, dragging);
                    self.active_terrain_mut().unwrap().show(ui, frame, rect);
                    self.mark_view = None;
                    failed
                } else {
                    self.paint_base(ui, &painter, rect, self.view, dragging, frame, 0, None)
                };
                let click_position = response
                    .interact_pointer_pos()
                    .filter(|pos| response.clicked() && !compass_rect.contains(*pos));
                let marker_pointer = click_position
                    .or_else(|| pointer.filter(|pos| !dragging && !compass_rect.contains(*pos)));
                let marker_picked =
                    self.paint_markers(&painter, rect, marker_pointer, click_position.is_some());
                if let Some(position) = click_position {
                    self.map_hit_job = None;
                    if marker_picked {
                        self.select_map_pin(ctx, None);
                    } else if self.active_terrain().is_some() {
                        self.select_map_pin(ctx, None);
                        self.map_hit_job =
                            Some(self.active_terrain().unwrap().hit_test(ctx, rect, position));
                    } else {
                        let world = self.view.world_at(rect, position);
                        self.select_map_pin(
                            ctx,
                            world
                                .iter()
                                .all(|v| (0. ..=1.).contains(v))
                                .then_some(world),
                        );
                    }
                }
                if let Some(world) = self.map_pin {
                    let height = self
                        .map_pin_elevation
                        .filter(|(p, _)| *p == world)
                        .map(|(_, h)| h);
                    let position = self
                        .active_terrain()
                        .and_then(|t| t.projection().screen(rect, world, height.unwrap_or(0.)))
                        .unwrap_or_else(|| self.view.screen(rect, world));
                    if rect.contains(position) {
                        let coordinates = points::unproject(world);
                        if Self::paint_marker_label(
                            &painter,
                            rect,
                            position,
                            height.map_or_else(
                                || coordinate_label(coordinates),
                                |h| format!("{}　標高 {h:.1} m", coordinate_label(coordinates)),
                            ),
                            true,
                            Some((ui, coordinates)),
                        ) {
                            self.select_map_pin(ctx, None);
                        } else {
                            painter.circle_filled(position, 5., appearance::ACCENT);
                            painter.circle_stroke(
                                position,
                                5.,
                                egui::Stroke::new(1_f32, Color32::WHITE),
                            );
                        }
                    }
                }
                if failed {
                    painter.text(
                        rect.left_top() + egui::vec2(8., 8.),
                        egui::Align2::LEFT_TOP,
                        "一部の地図タイルを取得できません。再試行します。",
                        egui::FontId::proportional(13.),
                        Color32::from_rgb(122, 54, 40),
                    );
                }
                painter.rect_stroke(
                    rect,
                    0,
                    egui::Stroke::new(1_f32, appearance::LINE),
                    egui::StrokeKind::Inside,
                );
                let zoom = painter.layout_no_wrap(
                    {
                        let zoom = self.view.zoom;
                        let tile_zoom = self.map_source.tile_zoom(zoom, self.lod);
                        if self.show_online_map && tile_zoom != zoom as u32 {
                            format!("Z {zoom:.1} / z{tile_zoom}")
                        } else {
                            format!("Z {zoom:.1}")
                        }
                    },
                    egui::FontId::monospace(12.),
                    appearance::INK,
                );
                let position = rect.right_bottom() - zoom.size() - egui::vec2(17., 14.);
                let badge = Rect::from_min_size(position, zoom.size()).expand2(egui::vec2(7., 4.));
                painter.rect_filled(badge, 7, appearance::PAPER);
                painter.rect_stroke(
                    badge,
                    7,
                    egui::Stroke::new(1_f32, appearance::LINE),
                    egui::StrokeKind::Inside,
                );
                let zoom_height = zoom.size().y;
                painter.galley(position, zoom, appearance::INK);
                if let Some(observer) = &self.observer {
                    let height = observer_height(
                        observer.elevation,
                        observer.terrain_elevation,
                        observer.from_pin,
                        observer.eye_offset,
                    );
                    let label = painter.layout_no_wrap(
                        format!("視点標高 {height:.1} m"),
                        egui::FontId::proportional(13.),
                        appearance::INK,
                    );
                    let position = rect.right_bottom()
                        - label.size()
                        - egui::vec2(17., 14. + zoom_height + 16.);
                    let badge =
                        Rect::from_min_size(position, label.size()).expand2(egui::vec2(7., 4.));
                    painter.rect_filled(badge, 7, appearance::PAPER);
                    painter.rect_stroke(
                        badge,
                        7,
                        egui::Stroke::new(1_f32, appearance::LINE),
                        egui::StrokeKind::Inside,
                    );
                    painter.galley(position, label, appearance::INK);
                }
                paint_compass(
                    &painter,
                    compass_rect.center(),
                    self.observer.as_ref().map_or_else(
                        || self.view.orientation.north(),
                        |observer| {
                            egui::vec2(observer.yaw.sin() as f32, -observer.yaw.cos() as f32)
                        },
                    ),
                );
            });
    }
}

fn coordinate_label([latitude, longitude]: [f64; 2]) -> String {
    format!("緯度 {latitude:.6}°　経度 {longitude:.6}°")
}

fn paint_compass(painter: &egui::Painter, center: Pos2, north: egui::Vec2) {
    painter.circle(
        center,
        24.,
        appearance::PAPER,
        egui::Stroke::new(1_f32, appearance::LINE),
    );
    let side = egui::vec2(-north.y, north.x);
    for (direction, color) in [
        (north, Color32::from_rgb(177, 57, 47)),
        (-north, appearance::MUTED),
    ] {
        painter.add(egui::Shape::convex_polygon(
            vec![
                center + direction * 11.,
                center + side * 4.,
                center - side * 4.,
            ],
            color,
            egui::Stroke::NONE,
        ));
    }
    painter.text(
        center + north * 17.,
        egui::Align2::CENTER_CENTER,
        "N",
        egui::FontId::proportional(11.),
        appearance::INK,
    );
}

fn install_japanese_font(ctx: &egui::Context) {
    for path in [
        "C:/Windows/Fonts/meiryo.ttc",
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("日本語".into(), FontData::from_owned(bytes).into());
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("日本語".into());
            }
            ctx.set_fonts(fonts);
            break;
        }
    }
}

#[cfg(windows)]
fn windows_graphics() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::{
        egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew},
        wgpu,
    };

    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(WgpuSetupCreateNew {
            // 垂直同期を保ったまま、描画間隔が安定していた Direct3D 12 を優先する。
            native_adapter_selector: Some(Arc::new(|adapters, surface| {
                adapters
                    .iter()
                    .filter(|adapter| {
                        surface.is_none_or(|surface| adapter.is_surface_supported(surface))
                    })
                    .min_by_key(|adapter| {
                        let info = adapter.get_info();
                        (
                            info.device_type == wgpu::DeviceType::Cpu,
                            info.backend != wgpu::Backend::Dx12,
                            info.device_type != wgpu::DeviceType::DiscreteGpu,
                        )
                    })
                    .cloned()
                    .ok_or_else(|| "利用できる描画デバイスがありません".to_owned())
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mut initial_surface = None;
    let mut terrain_capture = None;
    let file = match args.next() {
        Some(arg) if arg == "--terrain" => {
            let path = PathBuf::from(
                args.next()
                    .ok_or_else(|| anyhow::anyhow!("地形SQLiteを指定してください"))?,
            );
            let surface = args.next().map(PathBuf::from);
            ensure!(
                args.next().is_none(),
                "地形SQLiteと任意の地表SQLiteを指定してください"
            );
            initial_surface = surface;
            Some(path)
        }
        Some(arg) if arg == "--check-terrain-render" => {
            let path = PathBuf::from(
                args.next()
                    .ok_or_else(|| anyhow::anyhow!("地形SQLiteを指定してください"))?,
            );
            let output = PathBuf::from(
                args.next()
                    .ok_or_else(|| anyhow::anyhow!("出力PNGを指定してください"))?,
            );
            initial_surface = args.next().map(PathBuf::from);
            ensure!(
                args.next().is_none(),
                "地形SQLite・出力PNG・任意の地表SQLiteを指定してください"
            );
            terrain_capture = Some(output);
            Some(path)
        }
        Some(arg) if arg == "--check-terrain" => {
            let path = PathBuf::from(
                args.next()
                    .ok_or_else(|| anyhow::anyhow!("地形SQLiteを指定してください"))?,
            );
            ensure!(args.next().is_none(), "地形SQLiteは1つ指定してください");
            return yamamuki_terrain_view::check(&path);
        }
        Some(arg) if arg == "--check-map" => {
            ensure!(args.next().is_none(), "--check-map は引数を取りません");
            return map::check();
        }
        Some(arg) if arg == "--check-surface" => {
            let path = PathBuf::from(
                args.next()
                    .ok_or_else(|| anyhow::anyhow!("地表SQLiteを指定してください"))?,
            );
            ensure!(args.next().is_none(), "地表SQLiteは1つ指定してください");
            return surface::check(&path);
        }
        Some(arg) if arg == "--check-points" => {
            let path = PathBuf::from(
                args.next()
                    .ok_or_else(|| anyhow::anyhow!("地点ファイルを指定してください"))?,
            );
            ensure!(args.next().is_none(), "地点ファイルは1つ指定してください");
            println!("{} 地点を読み込みました", Points::open(&path)?.len());
            return Ok(());
        }
        Some(path) => Some(PathBuf::from(path)),
        None => None,
    };
    ensure!(args.next().is_none(), "ファイルは1つ指定してください");
    let client = map::client()?;
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        #[cfg(windows)]
        wgpu_options: windows_graphics(),
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_inner_size(DEFAULT_WINDOW_SIZE),
        ..Default::default()
    };
    eframe::run_native(
        "山むき・データビューワ",
        options,
        Box::new(move |cc| {
            #[cfg(windows)]
            window_move::install(cc);
            let mut viewer = Viewer::new(&cc.egui_ctx, client, file);
            if initial_surface.is_some() {
                viewer.show_online_map = false;
            }
            viewer.terrain_overlay = initial_surface;
            viewer.terrain_capture = terrain_capture;
            viewer.show_point_names = cc
                .storage
                .and_then(|storage| eframe::get_value(storage, POINT_NAMES_KEY))
                .unwrap_or(false);
            viewer.map_source = cc
                .storage
                .and_then(|storage| eframe::get_value(storage, MAP_SOURCE_KEY))
                .unwrap_or_default();
            viewer.lod = cc
                .storage
                .and_then(|storage| eframe::get_value(storage, LOD_KEY))
                .unwrap_or_default();
            viewer.surface_lod = cc
                .storage
                .and_then(|storage| eframe::get_value(storage, SURFACE_LOD_KEY))
                .unwrap_or(viewer.lod);
            viewer.surface_palette = cc
                .storage
                .and_then(|storage| eframe::get_value(storage, SURFACE_PALETTE_KEY))
                .unwrap_or_default();
            if let Some(size) = cc
                .storage
                .and_then(|storage| {
                    eframe::get_value::<egui::Vec2>(storage, NORMAL_WINDOW_SIZE_KEY)
                })
                .filter(|size| size.is_finite() && size.x > 0. && size.y > 0.)
            {
                viewer.normal_window_size = size;
            }
            viewer.normal_window_position = cc
                .storage
                .and_then(|storage| eframe::get_value::<Pos2>(storage, NORMAL_WINDOW_POSITION_KEY))
                .filter(|position| position.is_finite());
            Ok(Box::new(viewer))
        }),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
}

fn main() -> Result<()> {
    diagnostics::install();
    let result = run();
    if let Err(error) = &result {
        diagnostics::record(&format!("終了エラー: {error:#}"));
        #[cfg(target_os = "windows")]
        if !std::env::args_os().nth(1).is_some_and(|a| {
            a == "--check-map"
                || a == "--check-points"
                || a == "--check-surface"
                || a == "--check-terrain"
                || a == "--check-terrain-render"
        }) {
            rfd::MessageDialog::new()
                .set_title("データビューワ")
                .set_description(format!("{error:#}"))
                .show();
        }
        #[cfg(not(target_os = "windows"))]
        let _ = error;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_can_start_without_terrain_or_with_terrain_hidden() {
        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(&ctx, map::client().unwrap(), None);
        for shown in [false, true] {
            viewer.show_terrain = shown;
            viewer.enter_observer_at([0.8, 0.4], 3776., false);
            let observer = viewer
                .observer
                .as_ref()
                .expect("地形なしでも景色モードに入る");
            assert_eq!(observer.terrain_elevation, Some(0.));
            assert_eq!(
                observer_height(
                    observer.elevation,
                    observer.terrain_elevation,
                    false,
                    observer.eye_offset
                ),
                3778.
            );
            viewer.update_observer_height(&ctx);
            viewer.leave_observer();
            assert!(viewer.observer.is_none());
        }
    }
    #[test]
    fn observer_stands_above_both_point_and_current_terrain() {
        assert_eq!(observer_height(100., Some(120.), false, 2.), 122.);
        assert_eq!(observer_height(140., None, true, 2.), 2.);
        assert_eq!(observer_height(140., None, false, 2.), 142.);
        assert_eq!(observer_height(-10., None, false, 2.), 2.);
        assert_eq!(observer_height(140., Some(120.), false, 2.), 142.);
        assert_eq!(observer_height(140., Some(90.), true, 3.), 93.);
        assert_eq!(observer_height(140., Some(200.), true, 3.), 203.);
    }
    #[test]
    fn coordinate_observer_can_start_with_no_terrain() {
        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(&ctx, map::client().unwrap(), None);
        let world = [0.8, 0.4];
        viewer.map_pin = Some(world);
        viewer.enter_pin_observer(&ctx);
        let observer = viewer.observer.as_ref().unwrap();
        assert!(observer.from_pin);
        assert_eq!(observer.world, world);
        assert_eq!(observer.terrain_elevation, Some(0.));
        assert_eq!(
            observer_height(
                observer.elevation,
                observer.terrain_elevation,
                true,
                observer.eye_offset
            ),
            2.
        );
    }
    #[test]
    fn clicking_a_coordinate_fetches_height_without_entering_observer() {
        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(&ctx, map::client().unwrap(), None);
        let world = [0.8, 0.4];
        viewer.select_map_pin(&ctx, Some(world));
        let (send, receive) = mpsc::channel();
        viewer.pin_height_job = Some((world, receive));
        send.send(Ok(3776.)).unwrap();
        viewer.receive_pin_elevation();
        assert_eq!(viewer.map_pin_elevation, Some((world, 3776.)));
        assert!(viewer.observer.is_none());
        assert!(viewer.pin_height_job.is_none());
        let (send, receive) = mpsc::channel();
        viewer.pin_height_job = Some((world, receive));
        viewer.map_pin = Some([0.7, 0.3]);
        viewer.map_pin_elevation = None;
        send.send(Ok(100.)).unwrap();
        viewer.receive_pin_elevation();
        assert!(viewer.map_pin_elevation.is_none());
        assert!(viewer.observer.is_none());
    }
    #[test]
    fn lod_slider_follows_visible_map_and_surface_levels() {
        use yamamuki_data_sqlite as db;

        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(&ctx, map::client().unwrap(), None);
        assert_eq!(viewer.lod_slider_max(), 18);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("surface.sqlite3");
        let conn = db::rusqlite::Connection::open(&path).unwrap();
        db::initialize(&conn).unwrap();
        conn.execute_batch(
            r#"
            INSERT INTO datasets VALUES (1,'test','surface','地表','1',1,'{}');
            INSERT INTO surface_layers VALUES (1,1,'photo','rgb',0,'[]','{}');
            INSERT INTO surface_levels VALUES (1,1,12,'z12',256);
            INSERT INTO surface_tiles VALUES (1,1,12,0,0,1,X'00');
        "#,
        )
        .unwrap();
        let source = surface::Source::open(path).unwrap();
        viewer.surfaces.push(surface::Surface::new(&ctx, source));
        assert_eq!(viewer.lod_slider_max(), 18);

        viewer.map_source = map::Source::Blank;
        assert_eq!(viewer.lod_slider_max(), 14);
        assert_eq!(viewer.surface_lod_slider_max(), 12);
        viewer.show_online_map = false;
        assert_eq!(viewer.lod_slider_max(), 14);
        assert_eq!(viewer.surface_lod_slider_max(), 12);
        viewer.surfaces[0].enabled = false;
        assert_eq!(viewer.lod_slider_max(), 14);
        assert_eq!(viewer.surface_lod_slider_max(), 18);
    }

    #[test]
    fn restores_normal_window_geometry_after_starting_maximized() {
        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(&ctx, map::client().unwrap(), None);
        let normal = egui::vec2(920., 650.);
        let expanded = egui::vec2(1600., 1000.);
        let position = egui::pos2(150., 120.);
        viewer.normal_window_size = normal;
        viewer.normal_window_position = Some(position);

        let mut frame = |maximized, size| {
            let mut input = egui::RawInput::default();
            let window = input.viewports.entry(egui::ViewportId::ROOT).or_default();
            window.maximized = Some(maximized);
            window.inner_rect = Some(Rect::from_min_size(Pos2::ZERO, size));
            window.outer_rect = window.inner_rect;
            let mut output = ctx.run(input, |ctx| viewer.track_window_geometry(ctx));
            let commands = output
                .viewport_output
                .remove(&egui::ViewportId::ROOT)
                .unwrap()
                .commands;
            (viewer.normal_window_size, commands)
        };

        let (saved, commands) = frame(true, expanded);
        assert_eq!(saved, normal);
        assert!(commands.is_empty());
        let (saved, commands) = frame(false, expanded);
        assert_eq!(saved, normal);
        assert!(
            matches!(commands.as_slice(), [egui::ViewportCommand::InnerSize(size), egui::ViewportCommand::OuterPosition(pos)] if *size == normal && *pos == position)
        );
        assert!(frame(false, normal).1.is_empty());
    }

    #[test]
    fn layer_list_controls_the_online_map_in_the_left_pane() {
        use eframe::App as _;

        let ctx = egui::Context::default();
        let client = reqwest::blocking::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
            .build()
            .unwrap();
        let mut viewer = Viewer::new(&ctx, client, None);
        let mut app_frame = eframe::Frame::_new_kittest();
        let mut frame = |events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.))),
                    events,
                    ..Default::default()
                },
                |ctx| viewer.update(ctx, &mut app_frame),
            )
        };
        for _ in 0..3 {
            frame(Vec::new());
        }
        for (label, online_expected) in [
            ("オンライン地図", false),
            ("淡色地図", false),
            ("オンライン地図", true),
            ("オンライン地図", false),
        ] {
            let output = frame(Vec::new());
            let pos = output
                .shapes
                .iter()
                .find_map(|clipped| {
                    let egui::Shape::Text(text) = &clipped.shape else {
                        return None;
                    };
                    let center = text.visual_bounding_rect().center();
                    (text.galley.text() == label
                        && clipped.clip_rect.right() <= 390.
                        && clipped.clip_rect.contains(center))
                    .then_some(center)
                })
                .unwrap_or_else(|| panic!("{label} が左ペインにない"));
            frame(vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
            let output = frame(Vec::new());
            assert_eq!(
                output.shapes.iter().any(|clipped| {
                    matches!(&clipped.shape, egui::Shape::Text(text)
                    if text.galley.text().starts_with("出典：国土地理院"))
                }),
                online_expected
            );
        }
        let output = frame(Vec::new());
        let text_positions = |label| {
            output.shapes.iter().filter_map(move |clipped| {
                let egui::Shape::Text(text) = &clipped.shape else {
                    return None;
                };
                let center = text.visual_bounding_rect().center();
                (text.galley.text() == label && clipped.clip_rect.contains(center))
                    .then_some(center)
            })
        };
        let pale = text_positions("淡色地図").next().expect("淡色地図の行");
        let info = text_positions("i")
            .find(|pos| (pos.y - pale.y).abs() < 5. && pos.x > pale.x)
            .expect("淡色地図の情報ボタン");
        frame(vec![egui::Event::PointerMoved(info)]);
        for pressed in [true, false] {
            frame(vec![egui::Event::PointerButton {
                pos: info,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        let popup = frame(Vec::new());
        assert!(popup.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text() == "画像形式 PNG")
        }));
        assert!(!viewer.show_online_map);
        assert_eq!(viewer.map_source, map::Source::Gsi);
    }

    #[test]
    fn japanese_text_uses_the_same_fallback_in_json() {
        let ctx = egui::Context::default();
        install_japanese_font(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let text = "配布データの編集・加工";
            ctx.fonts(|fonts| {
                if fonts.has_glyphs(&egui::FontId::proportional(13.), text) {
                    assert!(fonts.has_glyphs(&egui::FontId::monospace(13.), text));
                }
            });
        });
    }

    #[test]
    fn point_layer_can_toggle_markers_show_info_and_close() {
        use eframe::App as _;
        use yamamuki_data_sqlite as sqlite;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peaks.sqlite3");
        let db = sqlite::rusqlite::Connection::open(&path).unwrap();
        sqlite::initialize(&db).unwrap();
        db.execute_batch(
            r#"INSERT INTO datasets VALUES(1,'sample-peaks','points','山頂テスト','0.1.0',6,'{"pointCount":2}');
               INSERT INTO point_records VALUES
               (1,1,'east','{"id":"east","name":"東","latitude":36.1,"longitude":138.3}'),
               (1,0,'west','{"id":"west","name":"西","latitude":36.1,"longitude":138.2}');"#,
        )
        .unwrap();
        drop(db);
        let points = Points::open(&path).unwrap();
        assert_eq!(points.len(), 2);
        let first = (0..points.len())
            .find(|&i| points.source_index(i) == 0)
            .unwrap();
        assert_eq!(points.point(first).id, "west");

        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(&ctx, map::client().unwrap(), None);
        viewer.show_online_map = false;
        viewer.list.reset(&points);
        viewer.points = Some(Arc::new(points));
        viewer.source = Some(path);
        let mut app_frame = eframe::Frame::_new_kittest();
        let mut frame = |viewer: &mut Viewer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1100., 720.))),
                    events,
                    ..Default::default()
                },
                |ctx| viewer.update(ctx, &mut app_frame),
            )
        };
        let output = frame(&mut viewer, Vec::new());
        let texts = |label| {
            output.shapes.iter().filter_map(move |clipped| {
                let egui::Shape::Text(text) = &clipped.shape else {
                    return None;
                };
                let center = text.visual_bounding_rect().center();
                (text.galley.text() == label && clipped.clip_rect.contains(center))
                    .then_some(center)
            })
        };
        let file_row = texts("peaks.sqlite3").next().expect("地点ファイル行");
        let category = texts("地点データ").next().expect("地点データの分類");
        assert!(category.y < file_row.y);
        assert!(texts("2 地点").any(|pos| (pos.y - category.y).abs() < 5.));
        assert_eq!(texts("地点全体").count(), 0);
        let button_pos = texts("i")
            .find(|pos| (pos.y - file_row.y).abs() < 5.)
            .expect("地点ファイルの情報ボタン");
        let close_pos = texts("閉じる")
            .find(|pos| (pos.y - file_row.y).abs() < 5.)
            .expect("地点ファイルを閉じるボタン");
        let shows_markers = |output: &egui::FullOutput, viewer: &Viewer| {
            output.shapes.iter().any(|clipped| {
                matches!(&clipped.shape, egui::Shape::Mesh(mesh)
                    if viewer.marker_mesh.as_ref().is_some_and(|markers| Arc::ptr_eq(mesh, markers)))
            })
        };
        assert!(shows_markers(&output, &viewer));
        let mut click = |viewer: &mut Viewer, pos| {
            frame(viewer, vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(
                    viewer,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
            frame(viewer, Vec::new())
        };
        for expected in [false, true, false] {
            let output = click(&mut viewer, category);
            assert_eq!(shows_markers(&output, &viewer), expected);
            assert!(viewer.points.is_some());
        }
        let output = click(&mut viewer, button_pos);
        for expected in [
            "データセットID sample-peaks",
            "データ版 0.1.0",
            "収録地点数 2",
            "pointCount",
        ] {
            assert!(output.shapes.iter().any(|clipped| {
                matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text().contains(expected))
            }), "{expected}");
        }
        viewer.show_point_file_info = false;
        click(&mut viewer, close_pos);
        assert!(viewer.points.is_none() && viewer.source.is_none());
        assert!(viewer.marker_mesh.is_none());
    }

    #[test]
    fn dragging_the_splitter_resizes_the_layer_list() {
        use eframe::App as _;

        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(&ctx, map::client().unwrap(), None);
        viewer.show_online_map = false;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test-peaks.json");
        std::fs::write(
            &path,
            br#"[{"id":"gsi:1","name":"Peak","latitude":36.2,"longitude":138.3}]"#,
        )
        .unwrap();
        let points = Points::open(&path).unwrap();
        viewer.list.reset(&points);
        viewer.points = Some(Arc::new(points));
        viewer.source = Some(path);
        let mut app_frame = eframe::Frame::_new_kittest();
        let mut frame = |events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1100., 720.))),
                    events,
                    ..Default::default()
                },
                |ctx| viewer.update(ctx, &mut app_frame),
            )
        };
        let splitter = || {
            ctx.read_response(egui::Id::new("layer-point-splitter"))
                .expect("一覧の境界")
                .rect
                .center()
        };
        let output = frame(Vec::new());
        assert!(output.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text)
                if text.galley.text() == "test-peaks.json"
                    && text.pos.y > 60.
                    && clipped.clip_rect.right() < 550.)
        }));
        for delta in [70., -110.] {
            let start = splitter();
            let end = start + egui::vec2(0., delta);
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(vec![egui::Event::PointerMoved(start)]);
            frame(vec![button(start, true)]);
            frame(vec![egui::Event::PointerMoved(end)]);
            frame(vec![button(end, false)]);
            frame(Vec::new());
            assert!(
                (splitter().y - end.y).abs() <= 1.,
                "start={start:?}, end={end:?}, actual={:?}",
                splitter()
            );
        }
    }

    #[test]
    fn map_coordinates_can_be_copied_and_cleared_without_a_points_file() {
        use eframe::App as _;

        let ctx = egui::Context::default();
        let client = reqwest::blocking::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
            .build()
            .unwrap();
        let mut viewer = Viewer::new(&ctx, client, None);
        let mut app_frame = eframe::Frame::_new_kittest();
        let pos = egui::pos2(430., 310.);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut frame = |viewer: &mut Viewer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.))),
                    events,
                    ..Default::default()
                },
                |ctx| viewer.update(ctx, &mut app_frame),
            )
        };
        frame(&mut viewer, vec![egui::Event::PointerMoved(pos)]);
        frame(&mut viewer, vec![button(pos, true)]);
        let output = frame(&mut viewer, vec![button(pos, false)]);

        let pin = viewer.map_pin.expect("地図にピンを表示");
        let [latitude, longitude] = points::unproject(pin);
        let label = coordinate_label([latitude, longitude]);
        assert!(output.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text() == label)
        }));
        let copy_button = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == "コピー" => {
                    Some(text.visual_bounding_rect().center())
                }
                _ => None,
            })
            .expect("座標ラベルのコピーボタン");
        frame(&mut viewer, vec![egui::Event::PointerMoved(copy_button)]);
        frame(&mut viewer, vec![button(copy_button, true)]);
        let copied = frame(&mut viewer, vec![button(copy_button, false)]);
        assert!(copied.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if text == &format!("{latitude:.6}, {longitude:.6}"))
        }));
        assert_eq!(viewer.map_pin, Some(pin));
        let clear_button = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == "×" => {
                    Some(text.visual_bounding_rect().center())
                }
                _ => None,
            })
            .expect("座標ラベルの閉じるボタン");
        frame(&mut viewer, vec![egui::Event::PointerMoved(clear_button)]);
        frame(&mut viewer, vec![button(clear_button, true)]);
        let output = frame(&mut viewer, vec![button(clear_button, false)]);
        assert!(viewer.map_pin.is_none());
        assert!(!output.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text() == label)
        }));
    }

    #[test]
    fn selected_marker_does_not_schedule_idle_frames() {
        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(&ctx, map::client().unwrap(), None);
        let points = Points::read(
            br#"[{"id":"gsi:1","name":"Peak","latitude":36.2,"longitude":138.3}]"#.as_slice(),
        )
        .unwrap();
        viewer.list.reset(&points);
        viewer.points = Some(Arc::new(points));
        viewer.selected = Some(0);
        viewer.marker_popup = true;
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));

        let mut delay = std::time::Duration::ZERO;
        let mut previous_mesh = None;
        for pass in 0..4 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    ..Default::default()
                },
                |ctx| {
                    let painter = ctx.layer_painter(egui::LayerId::background());
                    viewer.paint_markers(&painter, rect, None, false);
                },
            );
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            if pass > 0 {
                let mesh = viewer.marker_mesh.as_ref().unwrap();
                assert_eq!(mesh.vertices.len(), 4 * viewer.marks.len());
                assert_eq!(mesh.indices.len(), 6 * viewer.marks.len());
                if let Some(previous) = &previous_mesh {
                    assert!(Arc::ptr_eq(previous, mesh));
                }
                previous_mesh = Some(Arc::clone(mesh));
            }
        }
        assert_eq!(viewer.marks.len(), 1);
        assert_eq!(delay, std::time::Duration::MAX);
    }
}

#[cfg(test)]
mod surface_palette_checks {
    use super::*;
    use eframe::App as _;
    #[derive(Default)]
    struct Storage(std::collections::HashMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn flush(&mut self) {}
    }
    #[test]
    fn palette_menu_selects_and_saves_the_display_colors() {
        let ctx = egui::Context::default();
        let client = reqwest::blocking::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
            .build()
            .unwrap();
        let mut viewer = Viewer::new(&ctx, client, None);
        let mut app_frame = eframe::Frame::_new_kittest();
        let mut frame = |viewer: &mut Viewer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1100., 700.))),
                    events,
                    ..Default::default()
                },
                |ctx| viewer.update(ctx, &mut app_frame),
            )
        };
        let find = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        Some(t.pos + t.galley.rect.center().to_vec2())
                    }
                    _ => None,
                })
                .expect(label)
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut viewer, vec![]);
        let output = frame(&mut viewer, vec![]);
        let at = find(&output, "地表の表示設定");
        frame(
            &mut viewer,
            vec![egui::Event::PointerMoved(at), button(at, true)],
        );
        frame(&mut viewer, vec![button(at, false)]);
        let output = frame(&mut viewer, vec![]);
        let at = find(&output, "高コントラスト");
        frame(
            &mut viewer,
            vec![egui::Event::PointerMoved(at), button(at, true)],
        );
        frame(&mut viewer, vec![button(at, false)]);
        assert_eq!(viewer.surface_palette, surface_palette::Palette::Contrast);
        let mut storage = Storage::default();
        viewer.save(&mut storage);
        let saved: surface_palette::Palette =
            eframe::get_value(&storage, SURFACE_PALETTE_KEY).unwrap();
        assert_eq!(saved, surface_palette::Palette::Contrast);
    }
}

#[cfg(test)]
mod terrain_file_tests {
    use super::*;
    use yamamuki_data_sqlite as db;
    fn fixture() -> tempfile::NamedTempFile {
        let file = tempfile::Builder::new()
            .suffix(".sqlite3")
            .tempfile()
            .unwrap();
        let mut conn = db::rusqlite::Connection::open(file.path()).unwrap();
        db::initialize(&conn).unwrap();
        yamamuki_terrain::fixture::create(&mut conn).unwrap();
        file
    }
    #[test]
    fn ordinary_file_loading_detects_terrain() {
        let file = fixture();
        assert!(
            matches!(read_file(file.path().to_owned()).unwrap(),OpenedFile::Terrain(path) if path==file.path())
        );
    }
    #[test]
    fn mixed_sqlite_opens_as_terrain() {
        let file = fixture();
        let conn = db::rusqlite::Connection::open(file.path()).unwrap();
        conn.execute("INSERT INTO datasets(id,kind,title,data_version,format_version,metadata_json) VALUES('surface','surface','地表','test',1,'{}')",[]).unwrap();
        assert!(matches!(
            read_file(file.path().to_owned()).unwrap(),
            OpenedFile::Terrain(_)
        ));
    }
    #[test]
    fn unknown_terrain_version_is_rejected() {
        let file = fixture();
        let conn = db::rusqlite::Connection::open(file.path()).unwrap();
        conn.execute("UPDATE datasets SET format_version=99", [])
            .unwrap();
        assert!(read_file(file.path().to_owned()).is_err());
    }
}
