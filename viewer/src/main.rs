#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod appearance;
mod boundaries;
mod lod_fade;
mod map;
mod markers;
mod point_detail;
mod point_list;
mod points;
mod surface;
mod surface_shade;
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
const DEFAULT_WINDOW_SIZE: egui::Vec2 = egui::vec2(1150., 800.);
const NORMAL_WINDOW_SIZE_KEY: &str = "normal_window_size";
const NORMAL_WINDOW_POSITION_KEY: &str = "normal_window_position";

enum OpenedFile {
    Points(Points),
    Surface(surface::Source),
}

struct Viewer {
    normal_window_size: egui::Vec2,
    normal_window_position: Option<Pos2>,
    startup_expanded_rect: Option<Rect>,
    restore_normal_size: Option<bool>,
    view: map::View,
    tiles: Vec<map::Tiles>,
    map_source: map::Source,
    lod: map::Lod,
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
        self.surfaces
            .iter()
            .filter_map(surface::Surface::max_visible_level)
            .chain(
                self.show_online_map
                    .then(|| self.map_source.max_tile_zoom()),
            )
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
            let result = if points::is_sqlite(&path) {
                Points::try_open_sqlite(&path).and_then(|points| match points {
                    Some(points) => Ok(OpenedFile::Points(points)),
                    None => surface::Source::open(path).map(OpenedFile::Surface),
                })
            } else {
                Points::open(&path).map(OpenedFile::Points)
            };
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
        surface.fit = self.surfaces.is_empty();
        surface.reveal = true;
        if surface.fit {
            self.show_online_map = false;
        }
        self.surfaces.push(surface);
    }

    fn poll_file(&mut self, ctx: &egui::Context) {
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
            Ok(OpenedFile::Surface(source)) => self.add_surface(ctx, source),
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
        let current = (self.view, rect, self.selected);
        let view_changed = self.mark_view != Some(current);
        if view_changed {
            self.marker_mesh = None;
            let mut marks =
                points.filtered_marks(self.view.area(rect), self.view.scale(), self.list.prefix());
            if self.view.orientation != map::Orientation::default() {
                marks.retain(|mark| rect.contains(self.view.screen(rect, mark.world)));
            }
            self.marks = markers::displayed(marks, points.len(), self.selected);
            self.mark_view = Some(current);
        }
        let count = self.list.visible_count();
        if !self.marks.is_empty() {
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
                let positions = markers::in_paint_order(&self.marks, self.selected).map(|mark| {
                    (
                        self.view.screen(rect, mark.world),
                        mark.matched,
                        self.selected == Some(mark.index),
                    )
                });
                self.marker_mesh = Some(Arc::new(sprites.mesh(positions, self.marks.len())));
            }
            if let Some(mesh) = &self.marker_mesh {
                painter.add(egui::Shape::mesh(Arc::clone(mesh)));
            }
        }

        let hovered = pointer.and_then(|pointer| {
            markers::in_paint_order(&self.marks, self.selected)
                .rev()
                .find(|mark| {
                    let position = self.view.screen(rect, mark.world);
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
                self.selected = Some(mark.index);
                self.list.reveal(mark.index);
            }
            painter.ctx().request_repaint();
        }
        if self.marker_popup
            && let Some(index) = self.selected
        {
            let point = points.point(index);
            let position = self
                .view
                .screen(rect, points::project(point.latitude, point.longitude));
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
                self.view.screen(rect, mark.world),
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
        self.poll_file(ctx);
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
                            egui::Button::new("地表SQLiteを開く"),
                        )
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("地表SQLite", &["sqlite3", "sqlite", "db"])
                            .pick_file()
                    {
                        self.open_surface(ctx, path);
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
                    ui.menu_button("自動LOD", |ui| {
                        let slider_max = self.lod_slider_max();
                        let mut max = self.lod.max_level.unwrap_or(slider_max).min(slider_max);
                        if ui
                            .add(
                                egui::Slider::new(&mut max, 0..=slider_max)
                                    .prefix("z")
                                    .text("レベル上限"),
                            )
                            .on_hover_text("取得するタイルの実XYZレベルの上限。数値をクリックして直接入力できます。")
                            .changed()
                        {
                            self.lod.max_level = (max < slider_max).then_some(max);
                        }
                        ui.add(
                            egui::Slider::new(&mut self.lod.bias, map::Lod::BIAS_RANGE)
                                .step_by(0.25)
                                .text("選択基準（ズーム補正）"),
                        )
                        .on_hover_text("＋で細かめ、−で粗め。＋1で目標の画素密度が2倍になります。");
                        if ui.button("既定値に戻す").clicked() {
                            self.lod = map::Lod::default();
                        }
                    });
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
                            "左ドラッグで移動　／　右ドラッグで回転・傾き　／　ホイールで拡大縮小　／　クリックで座標",
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
                                fit: &mut self.fit_pending,
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
                        );
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
                    .on_hover_text("北を上にして真上からの表示に戻す");
                if compass.clicked() {
                    self.view.orientation = map::Orientation::default();
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
                    if rotating {
                        self.view.orientation.orbit(d);
                    } else if response.dragged_by(egui::PointerButton::Primary) {
                        self.view.pan(d);
                    }
                }
                let pointer = response.hover_pos();
                if let Some(pointer) = pointer {
                    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0. {
                        self.view.zoom_at(rect, pointer, f64::from(scroll) / 180.);
                    }
                }
                painter.rect_filled(rect, 0., appearance::CANVAS);
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
                    self.tiles[0].paint(
                        ui,
                        &painter,
                        rect,
                        &self.view,
                        dragging,
                        self.map_source.tile_zoom(self.view.zoom, self.lod),
                        fade_backend,
                    )
                } else {
                    false
                };
                let mut has_base = self.show_online_map;
                for surface in self.surfaces.iter_mut().filter(|surface| surface.enabled) {
                    surface.paint(
                        ui,
                        &painter,
                        rect,
                        &self.view,
                        dragging,
                        has_base,
                        fade_backend,
                        self.lod,
                    );
                    has_base |= surface.has_enabled_layers();
                }
                let click_position = response
                    .interact_pointer_pos()
                    .filter(|pos| response.clicked() && !compass_rect.contains(*pos));
                let marker_pointer = click_position
                    .or_else(|| pointer.filter(|pos| !dragging && !compass_rect.contains(*pos)));
                let marker_picked =
                    self.paint_markers(&painter, rect, marker_pointer, click_position.is_some());
                if let Some(position) = click_position {
                    let world = self.view.world_at(rect, position);
                    self.map_pin = (!marker_picked
                        && world.iter().all(|value| (0. ..=1.).contains(value)))
                    .then_some(world);
                }
                if let Some(world) = self.map_pin {
                    let position = self.view.screen(rect, world);
                    if rect.contains(position) {
                        let coordinates = points::unproject(world);
                        if Self::paint_marker_label(
                            &painter,
                            rect,
                            position,
                            coordinate_label(coordinates),
                            true,
                            Some((ui, coordinates)),
                        ) {
                            self.map_pin = None;
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
                painter.galley(position, zoom, appearance::INK);
                paint_compass(
                    &painter,
                    compass_rect.center(),
                    self.view.orientation.north(),
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
    let file = match args.next() {
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
            viewer.map_source = cc
                .storage
                .and_then(|storage| eframe::get_value(storage, MAP_SOURCE_KEY))
                .unwrap_or_default();
            viewer.lod = cc
                .storage
                .and_then(|storage| eframe::get_value(storage, LOD_KEY))
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
    let result = run();
    if let Err(error) = &result {
        #[cfg(target_os = "windows")]
        if !std::env::args_os()
            .nth(1)
            .is_some_and(|a| a == "--check-map" || a == "--check-points" || a == "--check-surface")
        {
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
        viewer.show_online_map = false;
        assert_eq!(viewer.lod_slider_max(), 12);
        viewer.surfaces[0].enabled = false;
        assert_eq!(viewer.lod_slider_max(), 18);
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
        let fit_positions: Vec<_> = texts("地点全体").collect();
        assert_eq!(fit_positions.len(), 1);
        assert!((fit_positions[0].y - file_row.y).abs() < 5.);
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
