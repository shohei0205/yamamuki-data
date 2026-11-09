#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod appearance;
mod boundaries;
mod map;
mod markers;
mod point_detail;
mod point_list;
mod points;
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
const DEFAULT_WINDOW_SIZE: egui::Vec2 = egui::vec2(1150., 800.);
const NORMAL_WINDOW_SIZE_KEY: &str = "normal_window_size";
const NORMAL_WINDOW_POSITION_KEY: &str = "normal_window_position";

struct Viewer {
    normal_window_size: egui::Vec2,
    normal_window_position: Option<Pos2>,
    startup_expanded_rect: Option<Rect>,
    restore_normal_size: Option<bool>,
    view: map::View,
    tiles: Vec<map::Tiles>,
    map_source: map::Source,
    points: Option<Arc<Points>>,
    client: reqwest::blocking::Client,
    boundaries: Arc<boundaries::BoundaryCache>,
    location_job: Option<mpsc::Receiver<Result<Vec<boundaries::Location>>>>,
    detail: point_detail::Detail,
    list: point_list::PointList,
    selected: Option<usize>,
    marker_popup: bool,
    map_pin: Option<[f64; 2]>,
    loading: Option<(PathBuf, mpsc::Receiver<Result<Points>>)>,
    source: Option<PathBuf>,
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
            client,
            boundaries: Arc::default(),
            location_job: None,
            detail: point_detail::Detail::default(),
            points: None,
            list: point_list::PointList::default(),
            selected: None,
            marker_popup: false,
            map_pin: None,
            loading: None,
            source: None,
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
            let _ = send.send(Points::open(&path));
            ctx.request_repaint();
        });
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
            Ok(points) => {
                self.source = Some(path);
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

    fn paint_markers(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        pointer: Option<Pos2>,
        clicked: bool,
    ) -> bool {
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
    ) {
        let label = painter.layout(
            text,
            egui::FontId::proportional(14.),
            appearance::INK,
            (rect.width() - 16.).max(1.),
        );
        let x = if popup {
            position.x - label.size().x / 2.
        } else {
            position.x + 7.
        };
        let y = position.y - label.size().y - if popup { 17. } else { 6. };
        let box_pos = egui::pos2(
            x.min(rect.right() - label.size().x - 5.)
                .max(rect.left() + 5.),
            y.max(rect.top() + 5.),
        );
        let card = Rect::from_min_size(box_pos, label.size()).expand(4.);
        painter.rect_filled(card, 6, appearance::PAPER);
        painter.rect_stroke(
            card,
            6,
            egui::Stroke::new(1_f32, appearance::LINE),
            egui::StrokeKind::Inside,
        );
        painter.galley(box_pos, label, appearance::INK);
    }
}

impl eframe::App for Viewer {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, MAP_SOURCE_KEY, &self.map_source);
        eframe::set_value(storage, NORMAL_WINDOW_SIZE_KEY, &self.normal_window_size);
        if let Some(position) = self.normal_window_position {
            eframe::set_value(storage, NORMAL_WINDOW_POSITION_KEY, &position);
        }
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.track_window_geometry(ctx);
        self.poll_file(ctx);
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
        if self.loading.is_none() {
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
                    let map_group = if self.map_source.is_osm() {
                        "OSM"
                    } else {
                        "地理院"
                    };
                    ui.menu_button(
                        format!("{map_group}：{} ▾", self.map_source.label()),
                        |ui| {
                            for (group, layers) in [
                                ("地理院地図", map::Source::GSI_LAYERS.as_slice()),
                                ("OpenStreetMap系", map::Source::OSM_LAYERS.as_slice()),
                            ] {
                                ui.strong(group);
                                ui.indent(group, |ui| {
                                    for &source in layers {
                                        if ui
                                            .selectable_value(
                                                &mut self.map_source,
                                                source,
                                                source.label(),
                                            )
                                            .clicked()
                                        {
                                            ui.close_menu();
                                        }
                                    }
                                });
                                if group == "地理院地図" {
                                    ui.separator();
                                }
                            }
                        },
                    );
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
                            .add_filter("地点データ", &["json", "gz"])
                            .pick_file()
                        {
                            self.open(ctx, path);
                        }
                    }
                    if ui
                        .add_enabled(self.points.is_some(), egui::Button::new("地点全体"))
                        .clicked()
                    {
                        self.fit_pending = true;
                    }
                    if self.loading.is_some() {
                        ui.spinner();
                        ui.label("地点を読み込み中");
                    } else if let Some(points) = &self.points {
                        appearance::badge(ui, format!("{} 地点", points.len()));
                    }
                    if let Some(source) = self
                        .loading
                        .as_ref()
                        .map(|(path, _)| path)
                        .or(self.source.as_ref())
                    {
                        let name = source
                            .file_name()
                            .unwrap_or(source.as_os_str())
                            .to_string_lossy();
                        ui.label(egui::RichText::new(name).color(appearance::MUTED).small())
                            .on_hover_text(source.display().to_string());
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
                    self.map_source.attribution(ui, self.view.zoom);
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
        if let Some(points) = &self.points {
            egui::SidePanel::left("points-list")
                .resizable(true)
                .default_width(510.)
                .width_range(240. ..=1000.)
                .frame(appearance::panel(appearance::PAPER, 12, 10))
                .show(ctx, |ui| {
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
                let failed = self.tiles[0].paint(ui, &painter, rect, &self.view, dragging);
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
                        painter.circle_filled(position, 5., appearance::ACCENT);
                        painter.circle_stroke(
                            position,
                            5.,
                            egui::Stroke::new(1_f32, Color32::WHITE),
                        );
                        Self::paint_marker_label(
                            &painter,
                            rect,
                            position,
                            coordinate_label(points::unproject(world)),
                            true,
                        );
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
                        let tile_zoom = self.map_source.tile_zoom(zoom);
                        if tile_zoom < zoom as u32 {
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
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .push("日本語".into());
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
    ensure!(args.next().is_none(), "地点ファイルは1つ指定してください");
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
            .is_some_and(|a| a == "--check-map" || a == "--check-points")
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
    fn clicking_the_map_displays_coordinates_without_a_points_file() {
        use eframe::App as _;

        let ctx = egui::Context::default();
        let client = reqwest::blocking::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
            .build()
            .unwrap();
        let mut viewer = Viewer::new(&ctx, client, None);
        let mut app_frame = eframe::Frame::_new_kittest();
        let pos = egui::pos2(430., 310.);
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
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
        frame(vec![egui::Event::PointerMoved(pos)]);
        frame(vec![button(true)]);
        let output = frame(vec![button(false)]);

        let label = coordinate_label(points::unproject(viewer.map_pin.expect("地図にピンを表示")));
        assert!(output.shapes.iter().any(|clipped| {
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
