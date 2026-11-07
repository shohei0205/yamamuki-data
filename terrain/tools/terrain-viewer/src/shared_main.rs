#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod landcover;
#[path = "../../terrain-rs/src/natural_codec.rs"]
#[allow(dead_code)]
mod natural_codec;
#[path = "../../terrain-rs/src/natural_local_gradient.rs"]
mod natural_local_gradient;
#[path = "../../terrain-rs/src/natural_regional_gpu.rs"]
#[allow(dead_code)]
mod natural_regional_gpu;
#[path = "../../terrain-rs/src/natural_regional_surface.rs"]
#[allow(dead_code)]
mod natural_regional_surface;
#[path = "../../terrain-rs/src/natural_surface.rs"]
#[allow(dead_code)]
mod natural_surface;
#[path = "shared_selection.rs"]
mod selection;
mod shared_data;
mod shared_patches;
mod shared_render;

use anyhow::Result;
use eframe::egui;
use shared_data::{Database, ERRORS, Key, Packet};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
#[derive(Clone)]
struct Request {
    version: u64,
    camera: selection::Camera,
    prefer_finer: bool,
    resident: HashSet<Key>,
}
enum Message {
    Catalog(HashMap<Key, usize>, String),
    Packet(u64, Arc<Packet>, bool),
    Selected(u64, Vec<Key>),
    Ready(u64, usize, f64),
    Error(String),
}
fn drain_messages(receiver: &mpsc::Receiver<Message>) -> Vec<Message> {
    receiver.try_iter().collect()
}
fn worker(
    path: PathBuf,
    requests: mpsc::Receiver<Request>,
    messages: mpsc::Sender<Message>,
    ctx: egui::Context,
) {
    let send = |message| {
        let _ = messages.send(message);
        ctx.request_repaint();
    };
    let result = (|| -> Result<()> {
        let mut db = Database::open(&path)?;
        send(Message::Catalog(db.catalog.clone(), db.generation.clone()));
        let mut roots = db
            .catalog
            .keys()
            .filter(|k| k.0 == 6)
            .copied()
            .collect::<Vec<_>>();
        roots.sort();
        for key in roots {
            send(Message::Packet(0, db.packet(key)?, true));
        }
        let mut next = None;
        loop {
            let mut request = if let Some(request) = next.take() {
                request
            } else {
                match requests.recv() {
                    Ok(v) => v,
                    Err(_) => break,
                }
            };
            while let Ok(new) = requests.try_recv() {
                request = new;
            }
            let started = Instant::now();
            let mut keys = selection::select(&db.catalog, &request.camera);
            if request.camera.auto && request.prefer_finer {
                keys = selection::prefer_cached(
                    keys,
                    &db.cached_keys(),
                    &db.catalog,
                    request.camera.maximum,
                );
            }
            if !request.camera.auto
                && (keys.len() > 256 || keys.iter().map(|k| db.catalog[k]).sum::<usize>() > 500_000)
            {
                send(Message::Error(
                    "固定レベルの表示範囲が広すぎます。拡大するか自動LODを選んでください。".into(),
                ));
                continue;
            }
            send(Message::Selected(request.version, keys.clone()));
            let mut interrupted = false;
            for key in &keys {
                if let Ok(new) = requests.try_recv() {
                    next = Some(new);
                    interrupted = true;
                    break;
                }
                if !request.resident.contains(key) {
                    send(Message::Packet(request.version, db.packet(*key)?, false));
                }
            }
            if !interrupted {
                send(Message::Ready(
                    request.version,
                    keys.len(),
                    started.elapsed().as_secs_f64(),
                ));
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        send(Message::Error(format!("{error:#}")));
    }
}
struct App {
    renderer: shared_render::Renderer,
    send: mpsc::Sender<Request>,
    receive: mpsc::Receiver<Message>,
    camera: selection::Camera,
    catalog: HashMap<Key, usize>,
    packets: HashMap<Key, Arc<Packet>>,
    gpu: HashMap<(Key, u32), Arc<shared_render::Tile>>,
    roots: Vec<Arc<shared_render::Tile>>,
    shown: Vec<Arc<shared_render::Tile>>,
    previous: Vec<Arc<shared_render::Tile>>,
    transition: Instant,
    wanted: Vec<Key>,
    pending_roots: VecDeque<Key>,
    scene_pending: bool,
    version: u64,
    queued: bool,
    last_request: Instant,
    last_frame: Instant,
    fps: f32,
    settings: [f32; 3],
    smooth: bool,
    wire: u32,
    borders: bool,
    colors: bool,
    join: bool,
    animate: bool,
    resolution: u32,
    status: String,
    loading: bool,
    path: String,
    generation: String,
    smoke: Option<PathBuf>,
    screenshot_requested: bool,
    ready_since: Option<Instant>,
    input_stamp: u64,
    last_interaction: Instant,
    settled: bool,
    zoom_probe: Option<(PathBuf, Option<Instant>)>,
    landcover: Option<landcover::Loader>,
    landcover_path: String,
    landcover_visible: bool,
    landcover_status: String,
    landcover_style: landcover::Style,
    last_landcover_request: Instant,
}
impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        path: PathBuf,
        smoke: Option<PathBuf>,
        region: &str,
        landcover_path: Option<PathBuf>,
    ) -> Self {
        install_fonts(&cc.egui_ctx);
        let state = cc.wgpu_render_state.as_ref().expect("wgpu初期化").clone();
        eprintln!("GPU {:?}", state.adapter.get_info());
        let (send, requests) = mpsc::channel();
        let (messages, receive) = mpsc::channel();
        let context = cc.egui_ctx.clone();
        let worker_path = path.clone();
        std::thread::spawn(move || worker(worker_path, requests, messages, context));
        let mut app = Self {
            renderer: shared_render::Renderer::new(state),
            send,
            receive,
            camera: selection::Camera::default(),
            catalog: HashMap::new(),
            packets: HashMap::new(),
            gpu: HashMap::new(),
            roots: Vec::new(),
            shown: Vec::new(),
            previous: Vec::new(),
            transition: Instant::now() - Duration::from_secs(1),
            wanted: Vec::new(),
            pending_roots: VecDeque::new(),
            scene_pending: false,
            version: 0,
            queued: true,
            last_request: Instant::now() - Duration::from_secs(1),
            last_frame: Instant::now(),
            fps: 0.,
            settings: [0.25, 1., 0.],
            smooth: true,
            wire: 0,
            borders: false,
            colors: false,
            join: true,
            animate: true,
            resolution: 128,
            status: "全国z6の制御点を読み込み中".into(),
            loading: true,
            path: path.to_string_lossy().into(),
            generation: String::new(),
            smoke,
            screenshot_requested: false,
            ready_since: None,
            input_stamp: u64::MAX,
            last_interaction: Instant::now(),
            settled: false,
            zoom_probe: None,
            landcover: None,
            landcover_path: landcover_path
                .as_ref()
                .map_or_else(String::new, |p| p.to_string_lossy().into()),
            landcover_visible: landcover_path.is_some(),
            landcover_status: String::new(),
            landcover_style: landcover::Style::default(),
            last_landcover_request: Instant::now() - Duration::from_secs(1),
        };
        if landcover_path.is_some() {
            app.open_landcover(&cc.egui_ctx);
        }
        if region != "national" {
            app.region(region);
        }
        let args = std::env::args().collect::<Vec<_>>();
        if let Some(z) = args
            .iter()
            .position(|v| v == "--level")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u32>().ok())
        {
            app.camera.maximum = z.clamp(6, 13);
            app.camera.auto = false;
        }
        let option = |name: &str| {
            args.iter()
                .position(|v| v == name)
                .and_then(|i| args.get(i + 1))
        };
        if let Some(n) = option("--resolution").and_then(|s| s.parse::<u32>().ok()) {
            if [0, 32, 64, 128, 256, 512].contains(&n) {
                app.resolution = n;
            }
        }
        if let Some(n) = option("--wire-mode").and_then(|s| s.parse::<u32>().ok()) {
            app.wire = n.min(4);
        }
        if let Some(path) = option("--zoom-test-output") {
            app.zoom_probe = Some((PathBuf::from(path), None));
            app.camera.distance = 200000.;
        }
        app.borders = args.iter().any(|s| s == "--borders");
        app.colors = args.iter().any(|s| s == "--lod-colors");
        if args.iter().any(|v| v == "--wire") {
            app.wire = 1;
        }
        app
    }
    fn region(&mut self, name: &str) {
        let (lon, lat, height, distance) = match name {
            "fuji" => (138.727, 35.36, 1800., 28000.),
            "yari" => (137.647, 36.342, 2200., 18000.),
            "tsukuba" => (140.106, 36.225, 400., 18000.),
            _ => (138., 36., 500., 3600000.),
        };
        self.camera.center = selection::lonlat(lon, lat);
        self.camera.height = height;
        self.camera.distance = distance;
        self.camera.pitch = 0.9;
        self.camera.yaw = 0.;
        self.queued = true;
    }
    fn desired_resolution(&self, key: Key) -> u32 {
        if self.resolution == 0 {
            return 32;
        }
        let side = shared_data::bounds(key)[2] - shared_data::bounds(key)[0];
        let depth = self
            .camera
            .visible_depth(shared_data::bounds(key))
            .unwrap_or(self.camera.distance);
        let pixels = side * self.camera.scale() * self.camera.viewport[1]
            / (2. * (std::f64::consts::PI / 8.).tan() * depth);
        ((pixels / 3.).ceil().max(16.) as u32)
            .next_power_of_two()
            .min(self.resolution)
    }
    fn tile(&mut self, key: Key, n: u32) -> Result<Option<Arc<shared_render::Tile>>> {
        if let Some(tile) = self.gpu.get(&(key, n)) {
            return Ok(Some(tile.clone()));
        }
        let Some(packet) = self.packets.get(&key) else {
            return Ok(None);
        };
        let tile = self.renderer.upload(packet, n)?;
        self.gpu.insert((key, n), tile.clone());
        Ok(Some(tile))
    }
    fn scene(&mut self) -> Result<bool> {
        if !self.previous.is_empty() && self.transition.elapsed() < Duration::from_millis(450) {
            return Ok(false);
        }
        let mut uploaded = 0;
        if self.transition.elapsed() >= Duration::from_millis(450) {
            self.previous.clear();
        }
        let mut next = Vec::new();
        let mut planned = self
            .wanted
            .iter()
            .map(|&k| (k, self.desired_resolution(k)))
            .collect::<Vec<_>>();
        while planned
            .iter()
            .map(|(_, n)| u64::from(*n + 1).pow(2))
            .sum::<u64>()
            > 2_000_000
        {
            let least = planned
                .iter()
                .enumerate()
                .filter(|(_, (_, n))| *n > 16)
                .min_by(|(_, a), (_, b)| {
                    self.camera
                        .projected(a.0)
                        .1
                        .total_cmp(&self.camera.projected(b.0).1)
                })
                .map(|(i, _)| i);
            let Some(i) = least else { break };
            planned[i].1 /= 2;
        }
        for (key, n) in planned {
            if !self.gpu.contains_key(&(key, n)) {
                if uploaded >= 4 {
                    return Ok(false);
                }
                uploaded += 1;
            }
            if let Some(tile) = self.tile(key, n)? {
                next.push(tile);
            } else {
                return Ok(false);
            }
        }
        let changed = self.shown.len() != next.len()
            || self
                .shown
                .iter()
                .zip(&next)
                .any(|(a, b)| !Arc::ptr_eq(a, b));
        if changed {
            if !self.previous.is_empty() && self.transition.elapsed() < Duration::from_millis(450) {
                return Ok(false);
            }
            self.previous = std::mem::replace(&mut self.shown, next);
            self.transition = Instant::now();
        }
        if self.transition.elapsed() >= Duration::from_millis(450) {
            self.previous.clear();
        }
        if self.gpu.values().map(|v| v.bytes).sum::<usize>() > 512 * 1024 * 1024 {
            self.gpu
                .retain(|(k, _), v| k.0 == 6 || Arc::strong_count(v) > 1);
            self.packets
                .retain(|k, _| k.0 == 6 || self.wanted.contains(k));
        }
        Ok(true)
    }
    fn open_landcover(&mut self, ctx: &egui::Context) {
        self.renderer.clear_landcover();
        self.landcover = None;
        self.landcover_status.clear();
        if !self.landcover_path.trim().is_empty() {
            self.landcover = Some(landcover::Loader::start(
                PathBuf::from(&self.landcover_path),
                ctx.clone(),
                true,
            ));
            self.landcover_visible = true;
            self.landcover_status = "地表SQLiteを読み込み中".into();
        }
    }

    fn update_landcover(&mut self) {
        let Some(loader) = &mut self.landcover else {
            return;
        };
        if self.renderer.landcover_blend(self.landcover_style.animate) >= 1. {
            self.renderer.finish_landcover_transition();
            let mut detail = None;
            for message in loader.receive.try_iter() {
                match message {
                    landcover::Message::Extent(extent) => loader.extent = Some(extent),
                    landcover::Message::Overview(image) => {
                        self.renderer.upload_landcover(image, true, false);
                        if self.landcover_status == "地表SQLiteを読み込み中" {
                            self.landcover_status = "地表 z6 ／ ESA WorldCover 2021".into();
                        }
                    }
                    landcover::Message::Detail(image) => detail = Some(image),
                    landcover::Message::Error(error) => self.landcover_status = error,
                }
            }
            if let Some(image) = detail {
                self.landcover_status = format!("地表 z{} ／ ESA WorldCover 2021", image.z);
                self.renderer
                    .upload_landcover(image, false, self.landcover_style.animate);
            }
        }
        if self.landcover_visible
            && loader.extent.is_some()
            && self.last_landcover_request.elapsed() > Duration::from_millis(160)
        {
            let span = 2f64.powf((self.camera.distance.max(100.).log2() * 2.).ceil() / 2.) * 4.;
            let step = span / 16.;
            let center = self.camera.center.map(|v| (v / step).round() * step);
            let size = [1536, 1536];
            loader.request(
                self.landcover_style
                    .request(center, size, span / size[0] as f64),
            );
            self.last_landcover_request = Instant::now();
        }
    }

    fn open(&mut self, ctx: &egui::Context) {
        let (send, requests) = mpsc::channel();
        let (messages, receive) = mpsc::channel();
        let path = PathBuf::from(&self.path);
        let context = ctx.clone();
        std::thread::spawn(move || worker(path, requests, messages, context));
        self.send = send;
        self.receive = receive;
        self.catalog.clear();
        self.packets.clear();
        self.gpu.clear();
        self.roots.clear();
        self.shown.clear();
        self.previous.clear();
        self.wanted.clear();
        self.pending_roots.clear();
        self.scene_pending = false;
        self.queued = true;
        self.loading = true;
        self.ready_since = None;
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.update_landcover();
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if dt > 0. {
            self.fps = self.fps * 0.9 + 0.1 / dt;
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));
        if let Some(path) = dropped {
            self.path = path.to_string_lossy().into();
            self.open(ctx);
        }
        let mut scene_changed = false;
        // Messages only carry Arc handles. Drain them all; throttle actual GPU uploads separately.
        for message in drain_messages(&self.receive) {
            match message {
                Message::Catalog(catalog, generation) => {
                    self.catalog = catalog;
                    self.generation = generation;
                    self.queued = true;
                }
                Message::Packet(version, packet, root) => {
                    let key = packet.key;
                    self.packets.insert(key, packet);
                    if root {
                        if !self.roots.iter().any(|t| t.key == key)
                            && !self.pending_roots.contains(&key)
                        {
                            self.pending_roots.push_back(key);
                        }
                    } else if version == self.version || self.wanted.contains(&key) {
                        scene_changed = true;
                    }
                }
                Message::Selected(version, keys) => {
                    if version == self.version {
                        self.wanted = keys;
                        scene_changed = true;
                    }
                }
                Message::Ready(version, n, seconds) => {
                    if version == self.version {
                        self.loading = false;
                        self.status =
                            format!("読込完了 {n}タイル ／ 復号・補助情報 {:0.2}秒", seconds);
                        self.ready_since = Some(now);
                        if self.smoke.is_some() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        }
                        scene_changed = true;
                    }
                }
                Message::Error(error) => {
                    self.loading = false;
                    self.status = error;
                }
            }
        }
        for _ in 0..4 {
            let Some(key) = self.pending_roots.pop_front() else {
                break;
            };
            match self.tile(key, 32) {
                Ok(Some(tile)) => self.roots.push(tile),
                Err(e) => self.status = format!("{e:#}"),
                _ => {}
            }
        }
        if scene_changed {
            self.scene_pending = true;
        }
        if self.scene_pending
            || (!self.previous.is_empty()
                && self.transition.elapsed() >= Duration::from_millis(450))
        {
            match self.scene() {
                Ok(done) => self.scene_pending = !done,
                Err(e) => {
                    self.status = format!("{e:#}");
                    self.loading = false;
                    self.scene_pending = false;
                }
            }
        }
        let mut changed = false;
        egui::TopBottomPanel::top("settings").show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{
                ui.strong("全国共有曲面・ネイティブ");
                for (label,name) in [("全国","national"),("富士山","fuji"),("槍ヶ岳","yari"),("筑波山","tsukuba")] {
                    if ui.button(label).clicked(){self.region(name);}
                }
                ui.label(format!("{:0.0} fps",self.fps));
            });
            egui::CollapsingHeader::new("表示・SQLite設定").default_open(true).show(ui,|ui|{
                ui.horizontal(|ui|{ui.text_edit_singleline(&mut self.path);if ui.button("地形SQLiteを開く").clicked(){self.open(ctx);}});
                ui.horizontal_wrapped(|ui|{
                    ui.checkbox(&mut self.landcover_visible,"地表分類");
                    ui.text_edit_singleline(&mut self.landcover_path);
                    if ui.button("地表SQLiteを開く").clicked(){self.open_landcover(ctx);}
                    ui.label(&self.landcover_status);
                    if self.landcover_visible && self.landcover.is_some() { self.landcover_style.controls(ui); }
                });
                ui.horizontal_wrapped(|ui|{
                    changed|=ui.checkbox(&mut self.camera.auto,"自動LOD").changed();
                    let old=self.camera.maximum;
                    egui::ComboBox::from_label("最大／固定").selected_text(format!("z{old}")).show_ui(ui,|ui|{
                        for z in 6..=13 {ui.selectable_value(&mut self.camera.maximum,z,format!("z{z}・{}m",ERRORS[(z-6) as usize]));}
                    });
                    changed|=old!=self.camera.maximum;
                    changed|=ui.add(egui::Slider::new(&mut self.camera.lod_quality,0.5..=4.).text("LOD精細度")).changed();
                    let old=self.resolution;
                    egui::ComboBox::from_label("GPU描画分割の上限").selected_text(old.to_string()).show_ui(ui,|ui|{
                        for n in [0,32,64,128,256,512] {ui.selectable_value(&mut self.resolution,n,if n==0{"なし・制御点TIN".into()}else{n.to_string()});}
                    });
                    changed|=old!=self.resolution;
                    ui.add(egui::Slider::new(&mut self.settings[0],0.0..=1.).text("全域傾き調整"));
                    ui.add(egui::Slider::new(&mut self.settings[1],0.0..=6.).text("陰影の強さ"));
                    ui.checkbox(&mut self.smooth,"滑らかな陰影");
                    egui::ComboBox::from_id_salt("wire").selected_text(["ワイヤーなし","描画網＋陰影","描画網のみ","制御点TIN＋陰影","制御点TINのみ"][self.wire as usize]).show_ui(ui,|ui|{
                        for (i,label) in ["ワイヤーなし","描画網＋陰影","描画網のみ","制御点TIN＋陰影","制御点TINのみ"].iter().enumerate(){ui.selectable_value(&mut self.wire,i as u32,*label);}
                    });
                    ui.checkbox(&mut self.borders,"タイル境界");ui.checkbox(&mut self.colors,"LOD色分け");ui.checkbox(&mut self.join,"異LOD接合");ui.checkbox(&mut self.animate,"LODを滑らかに切替");
                });
            });
            if self.scene_pending&&!self.loading{ui.label("GPU曲面を準備中（直前の地形を維持）");}else{ui.label(&self.status);}
            let levels=self.shown.iter().map(|t|t.key.0).collect::<std::collections::BTreeSet<_>>();
            ui.label(format!("左ドラッグ 移動 ／ ホイール 拡大縮小 ／ 右ドラッグ 回転・傾き ／ 表示 {:?} ／ z6常駐 {}枚 ／ GPU {:0.1} MiB",
                levels,self.roots.len(),self.gpu.values().map(|t|t.bytes as f64).sum::<f64>()/1048576.));
            let (triangles,patches,extra_bytes)=self.renderer.metrics();
            ui.label(format!("制御点 {} ／ 描画三角形 {} ／ {}パッチ ／ GPU接合・モーフ {:0.1} MiB ／ 表示タイルの圧縮BLOB {:0.1} MiB",
                self.shown.iter().map(|t|t.controls).sum::<usize>(),
                triangles,patches,extra_bytes as f64/1048576.,
                self.shown.iter().map(|t|t.compressed_bytes as f64).sum::<f64>()/1048576.));
        });
        if changed {
            self.last_interaction = now;
            self.settled = false;
            self.queued = true;
        }
        if self.camera.auto
            && !self.settled
            && self.last_interaction.elapsed() >= Duration::from_secs(3)
        {
            self.settled = true;
            self.queued = true;
        }
        let time = ctx.input(|i| i.time).to_bits();
        let fresh_input = time != self.input_stamp;
        self.input_stamp = time;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    ui.available_size().max(egui::vec2(1., 1.)),
                    egui::Sense::drag(),
                );
                let viewport = [rect.width() as f64, rect.height() as f64];
                if self.camera.viewport != viewport {
                    self.camera.viewport = viewport;
                    self.queued = true;
                }
                if fresh_input {
                    let delta = ctx.input(|i| i.pointer.delta());
                    if response.dragged_by(egui::PointerButton::Secondary) {
                        self.camera.yaw -= delta.x as f64 * 0.006;
                        self.camera.pitch =
                            (self.camera.pitch + delta.y as f64 * 0.006).clamp(0.05, 1.56);
                        self.queued = true;
                    } else if response.dragged_by(egui::PointerButton::Primary) {
                        self.camera.pan(delta.x as f64, delta.y as f64);
                        self.queued = true;
                    }
                    if response.dragged() {
                        self.last_interaction = now;
                        self.settled = false;
                    }
                    if response.hovered() {
                        let wheel = ctx.input(|i| i.smooth_scroll_delta.y);
                        if wheel != 0. {
                            self.last_interaction = now;
                            self.settled = false;
                            self.camera.distance = (self.camera.distance
                                * (-wheel as f64 * 0.003).exp())
                            .clamp(100., 10_000_000.);
                            self.queued = true;
                        }
                    }
                }
                let callback = self.renderer.frame(
                    self.shown.clone(),
                    self.roots.clone(),
                    &self.camera,
                    self.settings,
                    self.smooth,
                    self.wire,
                    self.borders,
                    self.colors,
                    self.join,
                    self.previous.clone(),
                    {
                        let t = if self.animate {
                            (self.transition.elapsed().as_secs_f32() / 0.45).min(1.)
                        } else {
                            1.
                        };
                        t * t * (3. - 2. * t)
                    },
                    self.resolution == 0,
                    rect,
                    ctx.pixels_per_point(),
                    self.landcover_visible,
                    self.landcover_style.animate,
                );
                ui.painter().add(callback);
                ui.painter().image(
                    self.renderer.texture,
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
            });
        if self.zoom_probe.is_some() {
            let begin = self.zoom_probe.as_ref().unwrap().1;
            if begin.is_none() && !self.loading && !self.scene_pending && !self.shown.is_empty() {
                self.zoom_probe.as_mut().unwrap().1 = Some(now);
            }
            if let Some(start) = self.zoom_probe.as_ref().unwrap().1 {
                let seconds = start.elapsed().as_secs_f64();
                let distance = 200000. * (500f64 / 200000.).powf((seconds / 4.).min(1.));
                if (self.camera.distance - distance).abs() > 0.001 {
                    self.camera.distance = distance;
                    self.queued = true;
                    self.last_interaction = now;
                    self.settled = false;
                }
                let done = seconds > 4.5
                    && !self.loading
                    && !self.scene_pending
                    && !self.queued
                    && self.shown.iter().map(|t| t.key).collect::<Vec<_>>() == self.wanted;
                if done || seconds > 60. {
                    let center_level = self
                        .shown
                        .iter()
                        .filter(|t| {
                            let b = shared_data::bounds(t.key);
                            self.camera.center[0] >= b[0]
                                && self.camera.center[0] < b[2]
                                && self.camera.center[1] >= b[1]
                                && self.camera.center[1] < b[3]
                        })
                        .map(|t| t.key.0)
                        .max();
                    let result = serde_json::json!({"complete":done,"seconds":seconds,"zoom_seconds":4.,"distance":self.camera.distance,"center_level":center_level,"maximum":self.camera.maximum,"shown":self.shown.len(),"wanted":self.wanted.len(),"gpu_pending":self.scene_pending,"loading":self.loading,"status":self.status});
                    let path = self.zoom_probe.take().unwrap().0;
                    let _ = std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap());
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if self.queued
            && !self.catalog.is_empty()
            && now.duration_since(self.last_request) > Duration::from_millis(200)
        {
            self.version += 1;
            self.loading = true;
            self.queued = false;
            self.last_request = now;
            self.ready_since = None;
            let _ = self.send.send(Request {
                version: self.version,
                camera: self.camera.clone(),
                prefer_finer: !self.settled,
                resident: self.packets.keys().copied().collect(),
            });
        }
        if self.smoke.is_some()
            && self
                .ready_since
                .is_some_and(|t| now.duration_since(t) > Duration::from_secs(8))
            && !self.screenshot_requested
        {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if let Some(path) = &self.smoke {
                    if let Err(e) = landcover::write_screenshot(path, &image) {
                        self.status = format!("{e:#}");
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        ctx.request_repaint();
    }
}
fn install_fonts(ctx: &egui::Context) {
    for path in [
        "C:/Windows/Fonts/meiryo.ttc",
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.otf",
    ] {
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
            ctx.set_fonts(fonts);
            break;
        }
    }
}

fn run() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let option = |name: &str| {
        args.iter()
            .position(|v| v == name)
            .and_then(|i| args.get(i + 1))
    };
    let landcover_path = option("--landcover").map(PathBuf::from);
    if args.iter().any(|a| a == "--check-landcover") {
        let source = landcover_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("--check-landcover requires --landcover"))?;
        return landcover::check(source);
    }
    let explicit_path = option("--database").map(PathBuf::from);
    let map_path = if args.iter().any(|a| a == "--landcover-only") {
        landcover_path.clone().or_else(|| explicit_path.clone())
    } else if explicit_path.is_none() {
        landcover_path.clone()
    } else {
        explicit_path
            .as_ref()
            .filter(|path| landcover::Database::open(path).is_ok())
            .cloned()
    };
    if let Some(path) = map_path {
        let smoke = option("--smoke-output").map(PathBuf::from);
        return eframe::run_native(
            "山むき・全国地表",
            eframe::NativeOptions {
                renderer: eframe::Renderer::Wgpu,
                viewport: egui::ViewportBuilder::default().with_inner_size([1280., 900.]),
                ..Default::default()
            },
            Box::new(move |cc| {
                install_fonts(&cc.egui_ctx);
                Ok(Box::new(landcover::Map::new(&cc.egui_ctx, path, smoke)))
            }),
        )
        .map_err(|e| anyhow::anyhow!("{e}"));
    }
    if args.iter().any(|a| a == "--landcover-only") {
        anyhow::bail!("--landcover-only requires a SQLite path");
    }
    let path = explicit_path.unwrap_or_else(|| {
        let mut choices = vec![
            PathBuf::from("terrain-shared.sqlite3"),
            PathBuf::from("tools/terrain-viewer/terrain-shared.sqlite3"),
            PathBuf::from("build/national-shared-z6-z13-v1/terrain-shared.sqlite3"),
        ];
        if let Ok(exe) = std::env::current_exe() {
            if let Some(folder) = exe.parent() {
                choices.push(folder.join("terrain-shared.sqlite3"));
            }
        }
        choices
            .into_iter()
            .find(|p| p.is_file())
            .unwrap_or_else(|| PathBuf::from("terrain-shared.sqlite3"))
    });
    if let Some(output) = option("--lod-report") {
        let db = Database::open(&path)?;
        let mut rows = Vec::new();
        for (region, lon, lat, h) in [
            ("fuji", 138.727, 35.36, 1800.),
            ("yari", 137.647, 36.342, 2200.),
            ("tsukuba", 140.106, 36.225, 400.),
        ] {
            for distance in [
                200000., 100000., 50000., 28000., 18000., 10000., 5000., 2000., 500.,
            ] {
                let c = selection::Camera {
                    center: selection::lonlat(lon, lat),
                    height: h,
                    distance,
                    viewport: [1400., 800.],
                    pitch: 0.9,
                    ..Default::default()
                };
                let keys = selection::select(&db.catalog, &c);
                let center = keys
                    .iter()
                    .filter(|&&k| {
                        let b = shared_data::bounds(k);
                        c.center[0] >= b[0]
                            && c.center[0] < b[2]
                            && c.center[1] >= b[1]
                            && c.center[1] < b[3]
                    })
                    .map(|k| k.0)
                    .max();
                rows.push(serde_json::json!({"region":region,"distance":distance,"center_level":center,"tiles":keys.len(),"controls":keys.iter().map(|k|db.catalog[k]).sum::<usize>(),"levels":keys.iter().map(|k|k.0).collect::<std::collections::BTreeSet<_>>()}));
            }
        }
        std::fs::write(output, serde_json::to_vec_pretty(&rows)?)?;
        return Ok(());
    }
    if args.iter().any(|a| a == "--check-database") {
        return Database::check(&path);
    }
    let smoke = option("--smoke-output").map(PathBuf::from);
    let region = option("--region")
        .cloned()
        .unwrap_or_else(|| "national".into());
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::default();
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
        "山むき・全国共有曲面",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, path, smoke, &region, landcover_path)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
fn main() {
    if let Err(e) = run() {
        let _ = std::fs::write("terrain-shared-viewer-error.log", format!("{e:#}\n"));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod notification_tests {
    use super::*;
    #[test]
    fn cached_notification_backlog_does_not_hide_latest_selection() {
        let (tx, rx) = mpsc::channel();
        for version in 0..100 {
            for _ in 0..96 {
                tx.send(Message::Ready(version, 96, 0.)).unwrap();
            }
        }
        tx.send(Message::Selected(100, vec![(13, 1, 2)])).unwrap();
        tx.send(Message::Ready(100, 1, 0.)).unwrap();
        let messages = drain_messages(&rx);
        assert_eq!(messages.len(), 9602);
        assert!(
            matches!(&messages[messages.len()-2],Message::Selected(100,keys)if keys==&vec![(13,1,2)])
        );
        assert!(rx.try_recv().is_err());
    }
}
