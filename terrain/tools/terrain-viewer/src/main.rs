#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod adaptive_curve;
mod curve_budget;
mod data;
mod geometry;
mod gpu_loop;
mod lod;
mod render;
mod selection;

use anyhow::Result;
use data::{Database, Key};
use eframe::egui;
use geometry::Mesh;
use render::{GpuMesh, Renderer};
use selection::Camera;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

// 複数回のGUIレイアウトで同じマウス入力を再適用しない。
fn consume_input(previous: &mut u64, time: f64) -> bool {
    let stamp = time.to_bits();
    if *previous == stamp {
        return false;
    }
    *previous = stamp;
    true
}
struct Request {
    version: u64,
    epoch: u64,
    camera: Camera,
    divisions: u32,
}
enum Message {
    Roots(Arc<Mesh>, [f64; 2]),
    Scene {
        version: u64,
        epoch: u64,
        view: Camera,
        mesh: Arc<Mesh>,
        origin: [f64; 2],
        key: (Vec<Key>, u32, Arc<Vec<u8>>),
        seconds: f64,
        visible: usize,
    },
    Retry(u64, u64, f64),
    Error(String),
}
fn worker(
    path: PathBuf,
    input: mpsc::Receiver<Request>,
    output: mpsc::Sender<Message>,
    ctx: egui::Context,
) {
    let send = |v| {
        let _ = output.send(v);
        ctx.request_repaint();
    };
    let result = (|| -> Result<()> {
        let mut db = Database::open(&path)?;
        let origin = selection::lonlat(138., 36.);
        send(Message::Roots(
            Arc::new(geometry::build(&db.roots, origin, 1)?),
            origin,
        ));
        let mut cache: VecDeque<((Vec<Key>, u32), Arc<Mesh>, [f64; 2])> = VecDeque::new();
        while let Ok(mut request) = input.recv() {
            while let Ok(new) = input.try_recv() {
                request = new;
            }
            let start = Instant::now();
            let keys = selection::select(&db.catalog, &request.camera);
            let key = (keys.clone(), request.divisions);
            if !request.camera.auto
                && (keys.len() > 256 || keys.iter().map(|k| db.catalog[k]).sum::<usize>() > 500000)
            {
                send(Message::Retry(
                    request.version,
                    request.epoch,
                    request.camera.distance * 0.65,
                ));
                continue;
            }
            if let Some(i) = cache.iter().position(|v| v.0 == key) {
                let item = cache.remove(i).unwrap();
                let adjusted =
                    adaptive_curve::for_view(&item.1, item.2, &request.camera, request.divisions);
                let plan = adjusted.adaptive.as_ref().unwrap();
                let visible = plan.visible_faces;
                send(Message::Scene {
                    version: request.version,
                    epoch: request.epoch,
                    view: request.camera.clone(),
                    mesh: adjusted.clone(),
                    origin: item.2,
                    key: (keys.clone(), adjusted.divisions, plan.levels.clone()),
                    seconds: start.elapsed().as_secs_f64(),
                    visible,
                });
                cache.push_back(item);
                continue;
            }
            let tiles = keys
                .iter()
                .map(|&k| db.tile(k).map(Arc::new))
                .collect::<Result<Vec<_>>>()?;
            let origin = request.camera.center;
            let mesh = Arc::new(geometry::build(&tiles, origin, request.divisions)?);
            let adjusted =
                adaptive_curve::for_view(&mesh, origin, &request.camera, request.divisions);
            let plan = adjusted.adaptive.as_ref().unwrap();
            let visible = plan.visible_faces;
            send(Message::Scene {
                version: request.version,
                epoch: request.epoch,
                view: request.camera.clone(),
                mesh: adjusted.clone(),
                origin,
                key: (keys.clone(), adjusted.divisions, plan.levels.clone()),
                seconds: start.elapsed().as_secs_f64(),
                visible,
            });
            cache.push_back((key, mesh, origin));
            while cache.len() > 6
                || cache
                    .iter()
                    .map(|v| {
                        v.1.vertices.len() * 24
                            + v.1.indices.len() * 4
                            + v.1.control.words.len() * 4
                    })
                    .sum::<usize>()
                    > 192 * 1024 * 1024
            {
                cache.pop_front();
            }
            db.trim();
        }
        Ok(())
    })();
    if let Err(e) = result {
        send(Message::Error(format!("{e:#}")));
    }
}
struct Scene {
    mesh: Arc<Mesh>,
    gpu: Arc<GpuMesh>,
    origin: [f64; 2],
    key: (Vec<Key>, u32, Arc<Vec<u8>>),
}
struct Benchmark {
    output: PathBuf,
    frames: usize,
    frame_number: u64,
    intervals: Vec<f64>,
    cpu: Vec<f64>,
}
impl Benchmark {
    fn tick(&mut self, stats: render::PaintStats, upload: f64) -> bool {
        let number = stats.count;
        let cpu = stats.cpu_ms;
        if number == self.frame_number {
            return false;
        }
        self.frame_number = number;
        let interval = stats.interval_ms;
        self.frames += 1;
        if self.frames > 20 {
            self.intervals.push(interval);
            self.cpu.push(cpu);
        }
        if self.frames < 140 {
            return false;
        }
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let mut sorted = self.intervals.clone();
        sorted.sort_by(f64::total_cmp);
        let report = format!(
            "{{\"measurement\":\"terrain_paint_callback\",\"frames\":{},\"frame_ms\":{:.3},\"frame_p95_ms\":{:.3},\"draw_cpu_ms\":{:.3},\"upload_cpu_ms\":{:.3}}}\n",
            self.intervals.len(),
            mean(&self.intervals),
            sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)],
            mean(&self.cpu),
            upload
        );
        println!("{report}");
        if let Err(e) = std::fs::write(&self.output, report) {
            eprintln!("計測結果の保存失敗: {e}");
        }
        true
    }
}
struct DragTrial {
    output: PathBuf,
    start: Option<Instant>,
    moves: u64,
    requests: u64,
    applied: u64,
    geometries: u64,
}
impl DragTrial {
    fn active(&self) -> bool {
        self.start.is_some_and(|s| s.elapsed().as_secs_f64() < 4.)
    }
}
struct UnderlayTest {
    folder: PathBuf,
    stage: u8,
}
fn save_screenshot(path: &std::path::Path, image: &egui::ColorImage) -> Result<()> {
    let mut encoder = png::Encoder::new(
        std::fs::File::create(path)?,
        image.size[0] as u32,
        image.size[1] as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(
        &image
            .pixels
            .iter()
            .flat_map(|p| p.to_array())
            .collect::<Vec<_>>(),
    )?;
    Ok(())
}
struct App {
    renderer: Renderer,
    send: mpsc::Sender<Request>,
    receive: mpsc::Receiver<Message>,
    camera: Camera,
    roots: Option<Scene>,
    scenes: VecDeque<Scene>,
    active: Option<(Vec<Key>, u32, Arc<Vec<u8>>)>,
    lod: lod::Scheduler,
    loading: bool,
    status: String,
    divisions: u32,
    retention: f32,
    smooth: bool,
    strength: f32,
    smoke: Option<PathBuf>,
    screenshot_requested: bool,
    loaded: bool,
    benchmark: Option<Benchmark>,
    upload_ms: f64,
    input_stamp: u64,
    drag_trial: Option<DragTrial>,
    underlay_test: Option<UnderlayTest>,
    hide_underlay: bool,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>, path: PathBuf, smoke: Option<PathBuf>) -> Self {
        let fonts = [
            "C:/Windows/Fonts/meiryo.ttc",
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.otf",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        ];
        for path in fonts {
            if let Ok(bytes) = std::fs::read(path) {
                let mut fonts = egui::FontDefinitions::default();
                fonts
                    .font_data
                    .insert("日本語".into(), Arc::new(egui::FontData::from_owned(bytes)));
                fonts
                    .families
                    .get_mut(&egui::FontFamily::Proportional)
                    .unwrap()
                    .push("日本語".into());
                cc.egui_ctx.set_fonts(fonts);
                break;
            }
        }
        println!(
            "GPU: {:?}",
            cc.wgpu_render_state
                .as_ref()
                .expect("wgpu初期化")
                .adapter
                .get_info()
        );
        let renderer = Renderer::new(cc.wgpu_render_state.as_ref().expect("wgpu初期化").clone());
        let (send, input) = mpsc::channel();
        let (output, receive) = mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || worker(path, input, output, ctx));
        let mut app = Self {
            renderer,
            send,
            receive,
            camera: Camera::default(),
            roots: None,
            scenes: VecDeque::new(),
            active: None,
            lod: lod::Scheduler::new(Instant::now()),
            loading: true,
            status: "全国 z6 を読み込み中…".into(),
            divisions: 1,
            retention: 0.5,
            smooth: true,
            strength: 1.,
            smoke,
            screenshot_requested: false,
            loaded: false,
            benchmark: None,
            upload_ms: 0.,
            input_stamp: u64::MAX,
            drag_trial: None,
            underlay_test: None,
            hide_underlay: false,
        };
        let args: Vec<String> = std::env::args().collect();
        if let Some(i) = args.iter().position(|a| a == "--region") {
            match args.get(i + 1).map(String::as_str) {
                Some("fuji") => app.region(138.727, 35.36, 1800., 28000.),
                Some("yari") => app.region(137.647, 36.342, 2200., 28000.),
                Some("tsukuba") => app.region(140.106, 36.225, 400., 20000.),
                _ => {}
            }
        }
        if let Some(i) = args.iter().position(|a| a == "--divisions") {
            app.divisions = args
                .get(i + 1)
                .and_then(|v| v.parse().ok())
                .filter(|v| [1, 2, 4, 8].contains(v))
                .unwrap_or(1);
        }
        if let Some(i) = args.iter().position(|a| a == "--benchmark-output") {
            if let Some(path) = args.get(i + 1) {
                app.benchmark = Some(Benchmark {
                    output: path.into(),
                    frames: 0,
                    frame_number: u64::MAX,
                    intervals: vec![],
                    cpu: vec![],
                });
            }
        }
        if let Some(i) = args.iter().position(|a| a == "--drag-test-output") {
            if let Some(path) = args.get(i + 1) {
                app.drag_trial = Some(DragTrial {
                    output: path.into(),
                    start: None,
                    moves: 0,
                    requests: 0,
                    applied: 0,
                    geometries: 0,
                });
            }
        }
        if let Some(i) = args.iter().position(|a| a == "--underlay-test-output") {
            if let Some(path) = args.get(i + 1) {
                let folder = PathBuf::from(path);
                std::fs::create_dir_all(&folder).expect("背景試験の出力フォルダー");
                app.underlay_test = Some(UnderlayTest { folder, stage: 0 });
            }
        }
        app
    }
    fn change(&mut self) {
        self.lod.invalidate(Instant::now());
        self.loading = true;
    }
    fn motion(&mut self) {
        self.lod.moved(Instant::now());
        self.loading = true;
    }
    fn region(&mut self, lon: f64, lat: f64, height: f64, distance: f64) {
        self.camera.center = selection::lonlat(lon, lat);
        self.camera.height = height;
        self.camera.distance = distance;
        self.active = None;
        self.change();
    }
    fn messages(&mut self) {
        while let Ok(message) = self.receive.try_recv() {
            match message {
                Message::Roots(mesh, origin) => {
                    let gpu = Arc::new(self.renderer.upload(&mesh));
                    self.roots = Some(Scene {
                        mesh,
                        gpu,
                        origin,
                        key: (vec![], 1, Arc::new(vec![])),
                    });
                }
                Message::Scene {
                    version,
                    epoch,
                    view,
                    mesh,
                    origin,
                    key,
                    seconds,
                    visible,
                } => {
                    if !self.lod.complete(version, epoch) {
                        continue;
                    }
                    if version != self.lod.desired {
                        if let Some(previous) = self
                            .active
                            .as_ref()
                            .and_then(|key| self.scenes.iter().find(|s| &s.key == key))
                        {
                            if selection::stale_zoom_would_coarsen(
                                &self.camera,
                                &view,
                                &previous.key.0,
                                &key.0,
                                previous.gpu.divisions,
                                mesh.divisions,
                            ) {
                                self.loading = self.lod.pending();
                                continue;
                            }
                        }
                    }
                    if let Some(t) = &mut self.drag_trial {
                        if t.active() {
                            t.applied += 1;
                        }
                    }
                    let index = self.scenes.iter().position(|s| s.key == key);
                    if index.is_none() {
                        if let Some(t) = &mut self.drag_trial {
                            if t.active() {
                                t.geometries += 1;
                            }
                        }
                    }
                    let reused = index.is_some();
                    let scene = if let Some(i) = index {
                        self.scenes.remove(i).unwrap()
                    } else {
                        let started = Instant::now();
                        let gpu = Arc::new(self.renderer.upload(&mesh));
                        self.upload_ms = started.elapsed().as_secs_f64() * 1000.;
                        Scene {
                            mesh,
                            gpu,
                            origin,
                            key: key.clone(),
                        }
                    };
                    self.status = format!(
                        "z{:?} ／ {} タイル ／ {} 元三角形 → {} 描画三角形 ／ {} 頂点 ／ 曲面 最大指定{}→最大実際{} 分割・GPU ／ 画面内推定 {} 面{} ／ CPU組立 {:.2}s{}",
                        scene.mesh.levels,
                        scene.mesh.tiles,
                        scene.mesh.source_faces,
                        scene.gpu.triangles,
                        scene.gpu.vertex_count,
                        self.divisions,
                        scene.gpu.divisions,
                        visible,
                        scene
                            .mesh
                            .adaptive
                            .as_ref()
                            .map(|p| format!(
                                "・1/2/4/8分割の元面数 {:?}{}",
                                p.counts,
                                if p.limited {
                                    "・GPU生成メモリの上限"
                                } else {
                                    ""
                                }
                            ))
                            .unwrap_or_default(),
                        seconds,
                        if reused { "・GPU再利用" } else { "" }
                    );
                    self.active = Some(key);
                    self.scenes.push_back(scene);
                    while self.scenes.len() > 6
                        || self.scenes.iter().map(|s| s.gpu.bytes).sum::<usize>()
                            > 384 * 1024 * 1024
                    {
                        if self.scenes.len() <= 1 {
                            break;
                        }
                        self.scenes.pop_front();
                    }
                    self.loading = self.lod.pending();
                    self.loaded = true;
                }
                Message::Retry(version, epoch, d) => {
                    if self.lod.complete(version, epoch) && version == self.lod.desired {
                        self.camera.distance = d;
                        self.change();
                    }
                }
                Message::Error(e) => {
                    self.status = format!("エラー: {e}");
                    self.loading = false;
                    eprintln!("{}", self.status);
                }
            }
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.messages();
        if let Some(t) = &mut self.underlay_test {
            if t.stage == 0 && self.loaded && !self.loading && !self.lod.pending() {
                self.camera.distance *= 4.;
                self.hide_underlay = true;
                t.stage = 1;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                ctx.request_repaint();
            } else if t.stage == 2 {
                t.stage = 3;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                ctx.request_repaint();
            }
        }
        let first_input = consume_input(&mut self.input_stamp, ctx.input(|i| i.time));
        if self.loaded {
            let mut moving = false;
            if let Some(t) = &mut self.drag_trial {
                t.start.get_or_insert_with(Instant::now);
                if t.active() {
                    moving = first_input;
                    if moving {
                        t.moves += 1;
                    }
                    ctx.request_repaint();
                } else if !self.lod.pending() && !self.loading {
                    let report = format!(
                        "{{\"moves\":{},\"requests_during_drag\":{},\"applied_during_drag\":{},\"new_geometries_during_drag\":{}}}\n",
                        t.moves, t.requests, t.applied, t.geometries
                    );
                    println!("{report}");
                    let _ = std::fs::write(&t.output, report);
                    self.drag_trial = None;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
            }
            if moving {
                self.camera.pan(2., 0.5);
                self.motion();
            }
        }
        if self.loaded && !self.loading && !self.lod.pending() {
            if let Some(bench) = &mut self.benchmark {
                let stats = self.renderer.stats();
                let previous = bench.frame_number;
                if bench.tick(stats, self.upload_ms) {
                    self.benchmark = None;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                if stats.count != previous {
                    self.camera.yaw += 0.003;
                }
                ctx.request_repaint();
            }
        }
        let mut changed = false;
        egui::TopBottomPanel::top("設定").show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{ui.strong("全国地形・ネイティブ");for (label,lon,lat,h,d) in [("全国",138.,36.,500.,3600000.),("富士山",138.727,35.36,1800.,28000.),("槍ヶ岳",137.647,36.342,2200.,28000.),("筑波山",140.106,36.225,400.,20000.)]{if ui.button(label).clicked(){self.region(lon,lat,h,d);}}});
            egui::CollapsingHeader::new("表示設定").default_open(true).show(ui,|ui|{ui.horizontal_wrapped(|ui|{
                changed|=ui.checkbox(&mut self.camera.auto,"自動 LOD").changed();let old=self.camera.maximum;egui::ComboBox::from_label("最大／固定レベル").selected_text(format!("z{}",old)).show_ui(ui,|ui|{for z in 6..=14{ui.selectable_value(&mut self.camera.maximum,z,format!("z{z}・誤差 {} m",12.5*2f64.powi(14-z as i32)));}});changed|=old!=self.camera.maximum;
                ui.add_enabled_ui(self.camera.auto, |ui| {
                    changed |= ui.add(egui::Slider::new(&mut self.camera.lod_quality, 0.5..=4.0)
                        .logarithmic(true).text("自動LOD品質")
                        .custom_formatter(|n,_|format!("{n:.1}倍")))
                        .on_hover_text("右ほど高精細。1倍は従来、標準は2倍。面数・タイル数の上限は維持します。")
                        .changed();
                });
                let old=self.divisions;egui::ComboBox::from_label("Loop曲面（GPU・最大）").selected_text(format!("{}分割",old)).show_ui(ui,|ui|{for d in [1,2,4,8]{ui.selectable_value(&mut self.divisions,d,if d==1{"なし".into()}else{format!("{d}分割")});}});changed|=old!=self.divisions;
                ui.checkbox(&mut self.smooth,"滑らかな陰影");ui.add(egui::Slider::new(&mut self.retention,0.0..=1.0).text("形状保持"));ui.add(egui::Slider::new(&mut self.strength,0.0..=6.0).text("陰影の強さ"));
            });});

            ui.label(&self.status);ui.horizontal(|ui|{if self.loading {ui.spinner();} else {ui.allocate_space(egui::vec2(18.,18.));}ui.label(if self.loading {"必要なタイルを読み込み・接合中。保持済みの地形を表示しています。"} else {"読み込み完了"});});
            ui.label(format!("左ドラッグ: 移動 ／ ホイール: 拡大縮小 ／ 右ドラッグ: 回転・傾き ／ z6 全国 {} タイル常駐 ／ GPU {:.1} MiB",self.roots.as_ref().map_or(0,|s|s.mesh.tiles),self.scenes.iter().chain(self.roots.iter()).map(|s|s.gpu.bytes as f64).sum::<f64>()/1048576.));
        });
        if changed {
            self.change();
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let available = ui.available_size().max(egui::vec2(1., 1.));
                let (rect, response) = ui.allocate_exact_size(available, egui::Sense::drag());
                let viewport = [available.x as f64, available.y as f64];
                if self.camera.viewport != viewport {
                    self.camera.viewport = viewport;
                    self.change();
                }
                let delta = if first_input {
                    ctx.input(|i| i.pointer.delta())
                } else {
                    egui::Vec2::ZERO
                };
                let mut moved = false;
                if response.dragged_by(egui::PointerButton::Secondary) {
                    self.camera.yaw -= delta.x as f64 * 0.007;
                    self.camera.pitch =
                        (self.camera.pitch + delta.y as f64 * 0.007).clamp(0.02, 1.56);
                    moved = delta != egui::Vec2::ZERO;
                } else if response.dragged_by(egui::PointerButton::Primary) {
                    self.camera.pan(delta.x as f64, delta.y as f64);
                    moved = delta != egui::Vec2::ZERO;
                }
                if response.hovered() {
                    let scroll = if first_input {
                        ctx.input(|i| i.raw_scroll_delta.y)
                    } else {
                        0.
                    };
                    if scroll != 0. {
                        self.camera.distance = (self.camera.distance
                            * (-scroll as f64 * 0.002).exp())
                        .clamp(100., 8000000.);
                        moved = true;
                    }
                }
                if moved {
                    self.motion();
                }
                let active = self
                    .active
                    .as_ref()
                    .and_then(|key| self.scenes.iter().find(|s| &s.key == key))
                    .or(self.roots.as_ref());
                if let Some(scene) = active {
                    let size = [
                        (available.x * ctx.pixels_per_point())
                            .round()
                            .clamp(1., 4096.) as u32,
                        (available.y * ctx.pixels_per_point())
                            .round()
                            .clamp(1., 4096.) as u32,
                    ];
                    let callback = self.renderer.frame(
                        scene.gpu.clone(),
                        &self.camera,
                        scene.origin,
                        size,
                        self.retention,
                        self.smooth,
                        self.strength,
                        rect,
                        self.roots
                            .as_ref()
                            .filter(|root| {
                                !self.hide_underlay && !Arc::ptr_eq(&root.gpu, &scene.gpu)
                            })
                            .map(|root| (root.gpu.clone(), root.origin)),
                    );
                    ui.painter().add(callback);
                    ui.painter().image(
                        self.renderer.texture,
                        rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                        egui::Color32::WHITE,
                    );
                }
                let stats = self.renderer.stats();
                let label = if stats.fps > 0. {
                    format!("FPS: {:.1}\np95: {:.1} ms", stats.fps, stats.p95_ms)
                } else {
                    "FPS: —（停止中）".into()
                };
                let text = ui.painter().layout_no_wrap(
                    label,
                    egui::FontId::proportional(20.),
                    egui::Color32::WHITE,
                );
                let position = egui::pos2(
                    (rect.right() - text.size().x - 16.).max(rect.left() + 8.),
                    rect.top() + 12.,
                );
                let background = egui::Rect::from_min_size(position, text.size()).expand(6.);
                ui.painter()
                    .rect_filled(background, 6., egui::Color32::from_black_alpha(190));
                ui.painter().galley(position, text, egui::Color32::WHITE);
            });
        if let Some((version, epoch)) = self.lod.take(Instant::now()) {
            if let Some(t) = &mut self.drag_trial {
                if t.active() {
                    t.requests += 1;
                }
            }
            let _ = self.send.send(Request {
                version,
                epoch,
                camera: self.camera.clone(),
                divisions: self.divisions,
            });
        }
        if ctx.input(|i| i.pointer.any_down() || i.raw_scroll_delta.y != 0.) {
            ctx.request_repaint();
        }
        if self.renderer.stats().fps > 0. {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.lod.pending() || self.loading {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        if self.smoke.is_some() && self.loaded && !self.loading && !self.screenshot_requested {
            self.screenshot_requested = true;
            println!("{}", self.status);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            ctx.request_repaint();
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if let Some(t) = &mut self.underlay_test {
                    let path = t.folder.join(if t.stage == 1 {
                        "without-z6.png"
                    } else {
                        "with-z6.png"
                    });
                    save_screenshot(&path, &image).expect("背景試験の画像保存");
                    if t.stage == 1 {
                        self.hide_underlay = false;
                        t.stage = 2;
                        ctx.request_repaint();
                    } else {
                        self.underlay_test = None;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    continue;
                }
                if let Some(path) = &self.smoke {
                    let result = (|| -> Result<()> {
                        let file = std::fs::File::create(path)?;
                        let mut encoder =
                            png::Encoder::new(file, image.size[0] as u32, image.size[1] as u32);
                        encoder.set_color(png::ColorType::Rgba);
                        encoder.set_depth(png::BitDepth::Eight);
                        encoder.write_header()?.write_image_data(
                            &image
                                .pixels
                                .iter()
                                .flat_map(|p| p.to_array())
                                .collect::<Vec<_>>(),
                        )?;
                        Ok(())
                    })();
                    if let Err(e) = result {
                        eprintln!("画像保存失敗: {e}");
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let path = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("build/terrain-national-xyz-20261024/terrain.sqlite3"));
    ensure_exists(&path)?;
    if args.iter().any(|a| a == "--check-db") {
        let mut db = Database::open(&path)?;
        println!(
            "{} タイル、z6 {} タイル常駐",
            db.catalog.len(),
            db.roots.len()
        );
        for (name, lon, lat) in [
            ("富士山", 138.727, 35.36),
            ("槍ヶ岳", 137.647, 36.342),
            ("筑波山", 140.106, 36.225),
        ] {
            let mut c = Camera::default();
            c.center = selection::lonlat(lon, lat);
            c.distance = 28000.;
            for d in [1, 4, 8] {
                let now = Instant::now();
                let keys = selection::select(&db.catalog, &c);
                let tiles = keys
                    .into_iter()
                    .map(|k| db.tile(k).map(Arc::new))
                    .collect::<Result<Vec<_>>>()?;
                let base = Arc::new(geometry::build(&tiles, c.center, d)?);
                let m = adaptive_curve::for_view(&base, c.center, &c, d);
                let plan = m.adaptive.as_ref().unwrap();
                let mut close = c.clone();
                close.distance *= 0.15;
                let closer = adaptive_curve::for_view(&base, c.center, &close, d);
                let next = closer.adaptive.as_ref().unwrap();
                println!(
                    "適応分割 1/2/4/8の元面数 {:?}・画面内{}面・最大{}分割 / 接近 {:?}・{}面",
                    plan.counts, plan.visible_faces, m.divisions, next.counts, next.visible_faces
                );
                println!(
                    "{name}・GPU{d}分割予定: {} タイル / {} 制御頂点 / {} 制御面 / CPU {:.2}s",
                    m.tiles,
                    m.vertices.len(),
                    m.indices.len() / 3,
                    now.elapsed().as_secs_f64()
                );
            }
        }
        return Ok(());
    }
    let smoke = args
        .iter()
        .position(|a| a == "--smoke-output")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::default();
    // WindowsのVulkanでは描画間隔が詰まるため、既定はDirectX 12。
    // 明示されたバックエンド指定は比較試験などに使えるように残す。
    #[cfg(target_os = "windows")]
    if std::env::var_os("WGPU_BACKEND").is_none() {
        setup.instance_descriptor.backends = wgpu::Backends::DX12;
    }
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            desired_maximum_frame_latency: Some(1),
            wgpu_setup: setup.into(),
            ..Default::default()
        },
        viewport: egui::ViewportBuilder::default().with_inner_size([1400., 950.]),
        ..Default::default()
    };
    eframe::run_native(
        "山むき・地形ビューワー",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, path, smoke)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
fn ensure_exists(path: &std::path::Path) -> Result<()> {
    anyhow::ensure!(path.is_file(), "SQLite がありません: {}", path.display());
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e:#}");
        let _ = std::fs::write("terrain-viewer-error.log", format!("{e:#}\n"));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;
    #[test]
    fn multiple_layout_passes_move_camera_once() {
        let mut stamp = u64::MAX;
        let mut camera = Camera::default();
        let mut expected = camera.clone();
        expected.pan(10., 10.);
        for time in [1., 1., 2., 2.] {
            if consume_input(&mut stamp, time) {
                camera.pan(5., 5.);
            }
        }
        assert_eq!(camera.center, expected.center);
    }
}
