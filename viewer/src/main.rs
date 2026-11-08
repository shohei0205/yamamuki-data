#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod boundaries;
mod map;
mod markers;
mod point_detail;
mod point_list;
mod points;

use anyhow::{Result, ensure};
use eframe::egui::{self, Color32, FontData, FontFamily, Pos2, Rect};
use points::{Mark, Points};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
};

struct Viewer {
    view: map::View,
    tiles: map::Tiles,
    points: Option<Arc<Points>>,
    client: reqwest::blocking::Client,
    boundaries: Arc<boundaries::BoundaryCache>,
    location_job: Option<mpsc::Receiver<Result<Vec<boundaries::Location>>>>,
    detail: point_detail::Detail,
    list: point_list::PointList,
    selected: Option<usize>,
    marker_popup: bool,
    loading: Option<(PathBuf, mpsc::Receiver<Result<Points>>)>,
    source: Option<PathBuf>,
    error: Option<String>,
    fit_pending: bool,
    marks: Vec<Mark>,
    mark_view: Option<([f64; 2], f64, Rect, Option<usize>)>,
}

impl Viewer {
    fn new(ctx: &egui::Context, client: reqwest::blocking::Client, file: Option<PathBuf>) -> Self {
        install_japanese_font(ctx);
        let mut viewer = Self {
            view: map::View {
                center: points::project(36.2, 138.3),
                zoom: 5.,
            },
            tiles: map::Tiles::new(ctx, client.clone()),
            client,
            boundaries: Arc::default(),
            location_job: None,
            detail: point_detail::Detail::default(),
            points: None,
            list: point_list::PointList::default(),
            selected: None,
            marker_popup: false,
            loading: None,
            source: None,
            error: None,
            fit_pending: false,
            marks: Vec::new(),
            mark_view: None,
        };
        if let Some(file) = file {
            viewer.open(ctx, file);
        }
        viewer
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
    ) {
        let Some(points) = &self.points else { return };
        let current = (self.view.center, self.view.zoom, rect, self.selected);
        if self.mark_view != Some(current) {
            self.marks = markers::displayed(
                points.filtered_marks(self.view.area(rect), self.view.scale(), self.list.prefix()),
                points.len(),
                self.selected,
            );
            self.mark_view = Some(current);
        }
        let mut hovered = None;
        let count = self.list.visible_count();
        for &mark in self
            .marks
            .iter()
            .filter(|m| !m.matched)
            .chain(self.marks.iter().filter(|m| m.matched))
            .filter(|m| self.selected != Some(m.index))
            .chain(self.marks.iter().filter(|m| self.selected == Some(m.index)))
        {
            let position = self.view.screen(rect, mark.world);
            let active = self.selected == Some(mark.index);
            let (radius, fill, stroke) =
                markers::style(self.view.zoom, mark.matched, count, active);
            painter.circle(position, radius, fill, stroke);
            if pointer
                .is_some_and(|pointer| pointer.distance(position) <= radius + stroke.width / 2.)
            {
                hovered = Some(mark);
            }
        }
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
                    format!("{}\n標高 {}　ID {}", point.name, elevation, point.id),
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
            Color32::BLACK,
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
        painter.rect_filled(
            Rect::from_min_size(box_pos, label.size()).expand(4.),
            3.,
            Color32::WHITE,
        );
        painter.galley(box_pos, label, Color32::BLACK);
    }
}

impl eframe::App for Viewer {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
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
        egui::TopBottomPanel::top("tools").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.loading.is_none(),
                        egui::Button::new("地点ファイルを開く"),
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
                    ui.label(format!("{} 地点", points.len()));
                }
                if let Some(source) = self
                    .loading
                    .as_ref()
                    .map(|(path, _)| path)
                    .or(self.source.as_ref())
                {
                    ui.label(
                        source
                            .file_name()
                            .unwrap_or(source.as_os_str())
                            .to_string_lossy(),
                    );
                }
            });
            if let Some(error) = &self.error {
                ui.label(error);
            }
        });
        egui::TopBottomPanel::bottom("credits").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                map::Tiles::attribution(ui);
                if self.points.is_some() {
                    ui.hyperlink_to(
                        "行政区域：国土数値情報（共同通信社加工）",
                        boundaries::SOURCE,
                    );
                }
                ui.separator();
                ui.label("左ドラッグで移動・ホイールで拡大縮小・地点をクリック");
            });
        });
        if let Some(points) = &self.points {
            egui::SidePanel::left("points-list")
                .resizable(true)
                .default_width(510.)
                .width_range(240. ..=1000.)
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
        egui::CentralPanel::default().show(ctx, |ui| {
            let (response, painter) =
                ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
            let rect = response.rect;
            if self.fit_pending
                && let Some(points) = &self.points
            {
                self.view.fit(rect, points.bounds);
                self.fit_pending = false;
            }
            if response.dragged_by(egui::PointerButton::Primary) {
                let d = ctx.input(|i| i.pointer.delta());
                for (axis, delta) in [d.x, d.y].into_iter().enumerate() {
                    self.view.center[axis] = (self.view.center[axis]
                        - f64::from(delta) / self.view.scale())
                    .clamp(0., 1.);
                }
            }
            let pointer = response.hover_pos();
            if let Some(pointer) = pointer {
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0. {
                    self.view.zoom_at(rect, pointer, f64::from(scroll) / 180.);
                }
            }
            painter.rect_filled(rect, 0., Color32::from_rgb(227, 235, 239));
            let failed = self.tiles.paint(ui, &painter, rect, &self.view);
            self.paint_markers(&painter, rect, pointer, response.clicked());
            if failed {
                painter.text(
                    rect.left_top() + egui::vec2(8., 8.),
                    egui::Align2::LEFT_TOP,
                    "一部の地図タイルを取得できません。再試行します。",
                    egui::FontId::proportional(13.),
                    Color32::from_rgb(122, 54, 40),
                );
            }
            painter.text(
                rect.right_bottom() - egui::vec2(8., 8.),
                egui::Align2::RIGHT_BOTTOM,
                format!("z{:.1}", self.view.zoom),
                egui::FontId::proportional(13.),
                Color32::DARK_GRAY,
            );
        });
    }
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
        viewport: egui::ViewportBuilder::default().with_inner_size([1150., 800.]),
        ..Default::default()
    };
    eframe::run_native(
        "山むき・地点ビューワ",
        options,
        Box::new(move |cc| Ok(Box::new(Viewer::new(&cc.egui_ctx, client, file)))),
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
                .set_title("地点ビューワ")
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
        for _ in 0..4 {
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
        }
        assert_eq!(viewer.marks.len(), 1);
        assert_eq!(delay, std::time::Duration::MAX);
    }
}
