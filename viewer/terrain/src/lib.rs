//! 共通SQLiteの地形を制御点からGPUで描画する。
pub mod snapshot;
pub use yamamuki_terrain::{
    natural_local_gradient, natural_regional_gpu, natural_regional_surface,
};
mod catalog_cache;
mod globe;
mod gpu_profile;
mod hit_test;
pub use hit_test::Hit;
pub mod direct_material;
mod landcover;
mod point_visibility;
#[path = "shared_selection.rs"]
mod selection;
mod shared_data;
mod shared_field;
mod shared_patches;
mod shared_render;
pub fn check(path: &std::path::Path) -> anyhow::Result<()> {
    shared_data::Database::check(path)
}
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
    Catalog(HashMap<Key, usize>, String, String),
    Packet(u64, Arc<Packet>, bool),
    Selected(u64, Vec<Key>),
    Ready(u64, usize, f64),
    Error(String),
}
fn same_meshes(a: &[Arc<shared_render::Tile>], b: &[Arc<shared_render::Tile>]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    if a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b)) {
        return true;
    }
    let mut a = a
        .iter()
        .map(|v| Arc::as_ptr(v) as usize)
        .collect::<Vec<_>>();
    let mut b = b
        .iter()
        .map(|v| Arc::as_ptr(v) as usize)
        .collect::<Vec<_>>();
    a.sort_unstable();
    b.sort_unstable();
    a == b
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
        send(Message::Catalog(
            db.catalog.clone(),
            db.generation.clone(),
            db.credits.clone(),
        ));
        let mut roots = db
            .catalog
            .keys()
            .filter(|k| k.0 == 6)
            .copied()
            .collect::<Vec<_>>();
        roots.sort();
        let mut roots: VecDeque<_> = roots.into();
        let parallelism = std::thread::available_parallelism()
            .map_or(2, usize::from)
            .clamp(1, 8);
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
                send(Message::Error(format!(
                    "固定z{}の表示範囲が広すぎます（{}タイル・{}制御点）。拡大するか自動LODを選んでください。",
                    request.camera.maximum,
                    keys.len(),
                    keys.iter().map(|k| db.catalog[k]).sum::<usize>()
                )));
                continue;
            }
            send(Message::Selected(request.version, keys.clone()));
            let mut interrupted = false;
            let mut pending = keys
                .iter()
                .copied()
                .filter(|k| !request.resident.contains(k))
                .collect::<Vec<_>>();
            pending.sort_by(|a, b| {
                request
                    .camera
                    .projected(*b)
                    .1
                    .total_cmp(&request.camera.projected(*a).1)
            });
            for batch in pending.chunks(parallelism) {
                match requests.try_recv() {
                    Ok(new) => {
                        next = Some(new);
                        interrupted = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                db.packet_batch(batch, &|packet| {
                    send(Message::Packet(request.version, packet, false))
                })?;
            }
            if interrupted {
                continue;
            }
            send(Message::Ready(
                request.version,
                keys.len(),
                started.elapsed().as_secs_f64(),
            ));
            // 全国の背景を先に作って現在の画面を待たせない。背景の準備も視点変更で中断する。
            while !roots.is_empty() {
                match requests.try_recv() {
                    Ok(new) => {
                        next = Some(new);
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                let batch = (0..parallelism)
                    .filter_map(|_| roots.pop_front())
                    .collect::<Vec<_>>();
                db.packet_batch(&batch, &|packet| send(Message::Packet(0, packet, true)))?;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        send(Message::Error(format!("{error:#}")));
    }
}
type MeshKey = (Key, u32, shared_field::Field);
type PlannedField = (Key, shared_field::Field, u32, f64, bool);
// 曲面の完成まで必要な局所メッシュと、穴を塞ぐ元タイルの背景を一緒に保護する。
fn preparing_meshes(
    plan: impl IntoIterator<Item = (Key, shared_field::Field, u32)>,
) -> HashSet<MeshKey> {
    let mut protected = HashSet::new();
    for (key, field, n) in plan {
        protected.insert((key, n, field));
        if field.depth > 0 {
            protected.insert((key, 32, shared_field::Field::default()));
        }
    }
    protected
}
struct RememberedScene {
    camera: selection::Camera,
    tiles: Vec<Arc<shared_render::Tile>>,
    background: Vec<Arc<shared_render::Tile>>,
}
fn same_observer_direction(a: &selection::Camera, b: &selection::Camera) -> bool {
    let ea = a.eye();
    let eb = b.eye();
    let distance = ((ea[0] - eb[0]).hypot(ea[1] - eb[1])) * a.scale();
    let horizontal = 2. * (a.tan_half_fov() * a.viewport[0] / a.viewport[1]).atan();
    let yaw = (a.yaw - b.yaw).sin().atan2((a.yaw - b.yaw).cos()).abs();
    distance < 50.
        && (ea[2] - eb[2]).abs() < 50.
        && yaw < horizontal * 0.1
        && (a.pitch - b.pitch).abs() < a.fov_y * 0.1
        && (a.fov_y - b.fov_y).abs() < 0.02
        && (a.distance / b.distance).ln().abs() < 0.05
        && a.viewport[0] <= b.viewport[0] * 1.05
        && a.viewport[1] <= b.viewport[1] * 1.05
}
struct App {
    frozen_morph: Option<f32>,
    embedded_rect: Option<egui::Rect>,
    renderer: shared_render::Renderer,
    send: mpsc::Sender<Request>,
    receive: mpsc::Receiver<Message>,
    camera: selection::Camera,
    catalog: HashMap<Key, usize>,
    packets: HashMap<Key, Arc<Packet>>,
    gpu: HashMap<(Key, u32), Arc<shared_render::Tile>>,
    local_gpu: HashMap<(Key, u32, shared_field::Field), Arc<shared_render::Tile>>,
    local_background: Vec<Arc<shared_render::Tile>>,
    mesh_used: HashMap<MeshKey, Instant>,
    remembered: VecDeque<RememberedScene>,
    remembered_until_selection: Option<u64>,
    roots: Vec<Arc<shared_render::Tile>>,
    shown: Vec<Arc<shared_render::Tile>>,
    previous: Vec<Arc<shared_render::Tile>>,
    transition: Instant,
    wanted: Vec<Key>,
    pending_roots: VecDeque<Key>,
    scene_pending: bool,
    preparation: Option<(u64, Arc<Vec<PlannedField>>)>,
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
    credits: String,
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
        ctx: &egui::Context,
        state: eframe::egui_wgpu::RenderState,
        path: PathBuf,
        smoke: Option<PathBuf>,
        region: &str,
        landcover_path: Option<PathBuf>,
    ) -> Self {
        eprintln!("GPU {:?}", state.adapter.get_info());
        let (send, requests) = mpsc::channel();
        let (messages, receive) = mpsc::channel();
        let context = ctx.clone();
        let worker_path = path.clone();
        if !path.as_os_str().is_empty() {
            std::thread::spawn(move || worker(worker_path, requests, messages, context));
        }
        let mut app = Self {
            frozen_morph: None,
            embedded_rect: None,
            renderer: shared_render::Renderer::new(state, ctx),
            send,
            receive,
            camera: selection::Camera::default(),
            catalog: HashMap::new(),
            packets: HashMap::new(),
            gpu: HashMap::new(),
            local_gpu: HashMap::new(),
            local_background: Vec::new(),
            mesh_used: HashMap::new(),
            remembered: VecDeque::new(),
            remembered_until_selection: None,
            roots: Vec::new(),
            shown: Vec::new(),
            previous: Vec::new(),
            transition: Instant::now() - Duration::from_secs(1),
            wanted: Vec::new(),
            pending_roots: VecDeque::new(),
            scene_pending: false,
            preparation: None,
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
            resolution: 512,
            status: "全国z6の制御点を読み込み中".into(),
            loading: true,
            path: path.to_string_lossy().into(),
            generation: String::new(),
            credits: String::new(),
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
            app.open_landcover(ctx);
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
    fn desired_resolution(&self, key: Key, field: shared_field::Field) -> (u32, f64) {
        let pixels = field.pixels(key, &self.camera).unwrap_or(0.);
        let n = if self.resolution == 0 {
            32
        } else {
            ((pixels / self.camera.mesh_pixels)
                .ceil()
                .clamp(16., self.resolution.max(16) as f64) as u32)
                .next_power_of_two()
                .min(self.resolution)
        };
        (n, pixels)
    }
    fn tile(&mut self, key: Key, n: u32) -> Result<Option<Arc<shared_render::Tile>>> {
        if let Some(tile) = self.gpu.get(&(key, n)) {
            self.mesh_used
                .insert((key, n, shared_field::Field::default()), Instant::now());
            return Ok(Some(tile.clone()));
        }
        let Some(packet) = self.packets.get(&key) else {
            return Ok(None);
        };
        if !self.renderer.can_upload(n) {
            return Ok(None);
        }
        let tile = self.renderer.upload(packet, n)?;
        self.mesh_used
            .insert((key, n, shared_field::Field::default()), Instant::now());
        self.gpu.insert((key, n), tile.clone());
        Ok(Some(tile))
    }
    fn fields(&self, key: Key) -> Vec<shared_field::Field> {
        if self.resolution == 0 || self.wire >= 3 {
            vec![shared_field::Field::default()]
        } else {
            shared_field::Field::visible(key, &self.camera)
        }
    }
    fn cached_local(
        &mut self,
        key: Key,
        n: u32,
        field: shared_field::Field,
        sample_budget: u64,
    ) -> Option<Arc<shared_render::Tile>> {
        // 同じ領域の候補は2の累乗の密度だけ。全キャッシュを毎区画走査しない。
        let mut size = n.max(1).next_power_of_two();
        let mut result = None;
        while size <= 512 && u64::from(size + 1).pow(2) <= sample_budget {
            if let Some(tile) = self.local_gpu.get(&(key, size, field)) {
                result = Some(tile.clone());
                break;
            }
            size *= 2;
        }
        if let Some(tile) = &result {
            self.mesh_used.insert((key, tile.n, field), Instant::now());
        }
        result
    }
    fn remember_scene(&mut self) {
        if self.shown.is_empty() {
            return;
        }
        if let Some(i) = self
            .remembered
            .iter()
            .position(|s| same_observer_direction(&self.camera, &s.camera))
        {
            self.remembered.remove(i);
        }
        self.remembered.push_back(RememberedScene {
            camera: self.camera.clone(),
            tiles: self.shown.clone(),
            background: self.local_background.clone(),
        });
        while self.remembered.len() > 32 {
            self.remembered.pop_front();
        }
    }
    fn restore_remembered_scene(&mut self) {
        let Some(i) = self
            .remembered
            .iter()
            .rposition(|s| same_observer_direction(&self.camera, &s.camera))
        else {
            return;
        };
        let scene = self.remembered.remove(i).unwrap();
        let changed = self.shown.len() != scene.tiles.len()
            || self
                .shown
                .iter()
                .zip(&scene.tiles)
                .any(|(a, b)| !Arc::ptr_eq(a, b));
        if changed {
            self.shown = scene.tiles.clone();
            self.local_background = scene.background.clone();
            self.previous.clear();
            self.transition = Instant::now() - Duration::from_secs(1);
            // 復帰直後に、まだ届いていない旧方向の選択結果で上書きしない。
            self.remembered_until_selection = Some(self.version + 1);
            self.scene_pending = true;
        }
        let used = scene
            .tiles
            .iter()
            .chain(&scene.background)
            .map(|t| Arc::as_ptr(t) as usize)
            .collect::<HashSet<_>>();
        let now = Instant::now();
        for ((key, n), tile) in &self.gpu {
            if used.contains(&(Arc::as_ptr(tile) as usize)) {
                self.mesh_used
                    .insert((*key, *n, shared_field::Field::default()), now);
            }
        }
        for (key, tile) in &self.local_gpu {
            if used.contains(&(Arc::as_ptr(tile) as usize)) {
                self.mesh_used.insert(*key, now);
            }
        }
        self.remembered.push_back(scene);
    }
    fn trim_gpu_cache(&mut self, protected: &HashSet<MeshKey>) {
        const BUDGET: usize = 512 * 1024 * 1024;
        let mut bytes = self
            .gpu
            .values()
            .chain(self.local_gpu.values())
            .map(|t| t.bytes)
            .sum::<usize>();
        if bytes + self.renderer.source_bytes() <= BUDGET {
            return;
        }
        let mut entries = self
            .gpu
            .iter()
            .map(|((k, n), t)| ((*k, *n, shared_field::Field::default()), t.bytes))
            .chain(self.local_gpu.iter().map(|(k, t)| (*k, t.bytes)))
            .collect::<Vec<_>>();
        entries.sort_by_key(|(k, _)| self.mesh_used.get(k).copied());
        // まず未使用の古いGPUデータを除き、なお不足するときだけ最古の方向記録を一件外す。
        loop {
            for &(key, size) in &entries {
                if bytes + self.renderer.source_bytes() <= BUDGET {
                    break;
                }
                if protected.contains(&key) || (key.0.0 == 6 && key.1 == 32 && key.2.depth == 0) {
                    continue;
                }
                let removable = if key.2.depth == 0 {
                    self.gpu.get(&(key.0, key.1))
                } else {
                    self.local_gpu.get(&key)
                }
                .is_some_and(|t| Arc::strong_count(t) == 1);
                if !removable {
                    continue;
                }
                if key.2.depth == 0 {
                    self.gpu.remove(&(key.0, key.1));
                } else {
                    self.local_gpu.remove(&key);
                }
                self.mesh_used.remove(&key);
                bytes = bytes.saturating_sub(size);
            }
            if bytes + self.renderer.source_bytes() <= BUDGET || self.remembered.len() <= 1 {
                break;
            }
            self.remembered.pop_front();
        }
        let keep = self
            .gpu
            .keys()
            .map(|(k, _)| *k)
            .chain(self.local_gpu.keys().map(|(k, _, _)| *k))
            .chain(self.wanted.iter().copied())
            .collect::<HashSet<_>>();
        self.packets.retain(|k, _| k.0 == 6 || keep.contains(k));
    }
    fn preparation_plan(&mut self) -> Arc<Vec<PlannedField>> {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        let c = &self.camera;
        for v in [
            c.center[0],
            c.center[1],
            c.height,
            c.distance,
            c.yaw,
            c.pitch,
            c.fov_y,
            c.viewport[0],
            c.viewport[1],
            c.mesh_pixels,
        ] {
            v.to_bits().hash(&mut hash);
        }
        (c.observer, c.globe, self.resolution, self.wire).hash(&mut hash);
        self.wanted.hash(&mut hash);
        let mut roots = self
            .packets
            .keys()
            .filter(|k| k.0 == 6)
            .copied()
            .collect::<Vec<_>>();
        roots.sort();
        roots.hash(&mut hash);
        let stamp = hash.finish();
        if let Some((id, plan)) = &self.preparation {
            if *id == stamp {
                return plan.clone();
            }
        }
        let sample_budget = self.renderer.sample_budget();
        let mut planned = Vec::new();

        for &key in &self.wanted {
            for field in self.fields(key) {
                let (n, pixels) = self.desired_resolution(key, field);
                planned.push((key, field, n, pixels, false));
            }
        }
        let covered = shared_field::Coverage::new(&self.wanted);
        for &key in self
            .catalog
            .keys()
            .filter(|k| k.0 == 6 && self.packets.contains_key(k))
        {
            for field in self.fields(key) {
                if covered.covers(key, field) {
                    continue;
                }
                let (n, pixels) = self.desired_resolution(key, field);
                planned.push((key, field, n, pixels, true));
            }
        }
        while planned
            .iter()
            .map(|(_, _, n, _, _)| u64::from(*n + 1).pow(2))
            .sum::<u64>()
            > sample_budget
        {
            let least = planned
                .iter()
                .enumerate()
                .filter(|(_, (_, _, n, _, _))| *n > 16)
                .min_by(|(_, a), (_, b)| (a.3 / a.2 as f64).total_cmp(&(b.3 / b.2 as f64)))
                .map(|(i, _)| i);
            let Some(i) = least else {
                break;
            };
            planned[i].2 /= 2;
        }
        let planned = Arc::new(planned);
        self.preparation = Some((stamp, planned.clone()));
        planned
    }
    fn scene(&mut self) -> Result<bool> {
        self.renderer.begin_upload_batch();
        let result = self.prepare_scene();
        // 途中で不足・エラーになった場合も、準備済みの曲面は投入して次回へつなぐ。
        self.renderer.flush_upload_batch();
        result
    }
    fn prepare_scene(&mut self) -> Result<bool> {
        let missing = self
            .wanted
            .iter()
            .any(|key| !self.packets.contains_key(key));
        if missing && !self.loading {
            self.queued = true;
            self.ready_since = None;
            self.status = "不足している地形タイルを再取得中".into();
        }
        if self.remembered_until_selection.is_some() {
            return Ok(false);
        }
        // 復号や既存の遷移を待つ間にも、届いた曲面からGPU準備を進める。
        let prepare_started = Instant::now();
        let mut uploaded = 0;
        let mut incomplete = missing;
        if self.transition.elapsed() >= Duration::from_millis(450) {
            self.previous.clear();
        }
        let mut next = Vec::new();
        let mut background = Vec::new();
        let sample_budget = self.renderer.sample_budget();
        let planned = self.preparation_plan();
        let protected = preparing_meshes(planned.iter().map(|(k, f, n, _, _)| (*k, *f, *n)));
        self.trim_gpu_cache(&protected);
        let mut background_keys = HashSet::new();
        let mut remaining_samples = sample_budget;
        let mut required_samples = planned
            .iter()
            .map(|(_, _, n, _, _)| u64::from(*n + 1).pow(2))
            .sum::<u64>();
        for (key, field, n, _, is_background) in planned.iter().copied() {
            let samples = u64::from(n + 1).pow(2);
            required_samples -= samples;
            let available_samples = remaining_samples.saturating_sub(required_samples);
            if !self.packets.contains_key(&key) {
                incomplete = true;
                continue;
            }
            if field.depth > 0 && background_keys.insert(key) {
                if !self.gpu.contains_key(&(key, 32)) {
                    if uploaded >= 16
                        || (uploaded > 0 && prepare_started.elapsed() >= Duration::from_millis(8))
                    {
                        incomplete = true;
                        continue;
                    }
                    uploaded += 1;
                }
                let Some(base) = self.tile(key, 32)? else {
                    incomplete = true;
                    continue;
                };
                background.push(base);
            }
            let cached = if field.depth > 0 {
                self.cached_local(key, n, field, available_samples)
            } else {
                self.gpu.get(&(key, n)).cloned()
            };
            if let Some(tile) = cached {
                self.mesh_used.insert((key, tile.n, field), Instant::now());
                remaining_samples = remaining_samples.saturating_sub(u64::from(tile.n + 1).pow(2));
                if is_background {
                    background.push(tile);
                } else {
                    next.push(tile);
                }
                continue;
            }
            if uploaded >= 16
                || (uploaded > 0 && prepare_started.elapsed() >= Duration::from_millis(8))
                || !self.renderer.can_upload(n)
            {
                incomplete = true;
                continue;
            }
            uploaded += 1;
            let tile = if field.depth > 0 {
                let packet = self.packets.get(&key).unwrap();
                let tile = self.renderer.upload_field(packet, n, field)?;
                self.mesh_used.insert((key, n, field), Instant::now());
                self.local_gpu.insert((key, n, field), tile.clone());
                tile
            } else {
                let Some(tile) = self.tile(key, n)? else {
                    incomplete = true;
                    continue;
                };
                tile
            };
            remaining_samples = remaining_samples.saturating_sub(samples);
            if is_background {
                background.push(tile);
            } else {
                next.push(tile);
            }
        }
        if incomplete {
            return Ok(false);
        }
        // 同じ評価区画の順番だけが変わっても、再遷移やその完了待ちを発生させない。
        let changed = !same_meshes(&self.shown, &next);
        if changed
            && !self.previous.is_empty()
            && self.transition.elapsed() < Duration::from_millis(450)
        {
            return Ok(false);
        }
        // 全領域が揃ってから背景と本体を同時に切り替え、準備途中の穴を作らない。
        self.local_background = background;
        if changed {
            self.previous = std::mem::replace(&mut self.shown, next);
            self.transition = Instant::now();
        }
        if self.transition.elapsed() >= Duration::from_millis(450) {
            self.previous.clear();
        }
        self.remember_scene();
        self.trim_gpu_cache(&HashSet::new());
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
                    landcover::Message::Extent {
                        bounds,
                        credits,
                        supports_blur,
                        layered,
                        has_hillshade,
                        split_land,
                        unified_land,
                        has_bare,
                        default_bare_on_top,
                        l3_zoom,
                        l4_zoom,
                        has_l2,
                        has_z7,
                    } => {
                        loader.extent = Some(bounds);
                        loader.credits = credits;
                        loader.supports_blur = supports_blur;
                        loader.has_hillshade = has_hillshade;
                        loader.layered = layered;
                        loader.split_land = split_land;
                        loader.unified_land = unified_land;
                        loader.has_bare = has_bare;
                        self.landcover_style.bare_on_top = default_bare_on_top;
                        self.landcover_style.l3_zoom = l3_zoom;
                        self.landcover_style.l4_zoom = l4_zoom;
                        self.landcover_style.has_l2 = has_l2;
                        self.landcover_style.has_z7 = has_z7;
                    }
                    landcover::Message::Overview(image) => {
                        self.renderer.upload_landcover(image, true, false);
                        if self.landcover_status == "地表SQLiteを読み込み中" {
                            self.landcover_status = "地表プレビュー".into();
                        }
                    }
                    landcover::Message::Detail(image) => detail = Some(image),
                    landcover::Message::Error(error) => self.landcover_status = error,
                }
            }
            if let Some(image) = detail {
                self.landcover_status = format!("{}", image.status());
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

    fn start_worker(&mut self, ctx: &egui::Context) {
        let (send, requests) = mpsc::channel();
        let (messages, receive) = mpsc::channel();
        let path = PathBuf::from(&self.path);
        let context = ctx.clone();
        std::thread::spawn(move || worker(path, requests, messages, context));
        self.send = send;
        self.receive = receive;
    }
    fn open(&mut self, ctx: &egui::Context) {
        self.start_worker(ctx);
        self.catalog.clear();
        self.packets.clear();
        self.gpu.clear();
        self.renderer.reset_sources();
        self.local_gpu.clear();
        self.mesh_used.clear();
        self.remembered.clear();
        self.remembered_until_selection = None;
        self.local_background.clear();
        self.roots.clear();
        self.shown.clear();
        self.previous.clear();
        self.wanted.clear();
        self.pending_roots.clear();
        self.scene_pending = false;
        self.preparation = None;
        self.queued = true;
        self.loading = true;
        self.ready_since = None;
    }
}
/// 停止直後・0.4秒後・1.2秒後の3段階。DPIに依存せず実画面のpxを基準にする。
fn refinement_spacing(idle: Duration, pixels_per_point: f32) -> f64 {
    let pixels = if idle < Duration::from_millis(400) {
        3.
    } else if idle < Duration::from_millis(1200) {
        1.5
    } else {
        0.75
    };
    pixels / f64::from(pixels_per_point.max(1.))
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if self.frozen_morph.is_none() {
            self.update_landcover();
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if dt > 0. {
            self.fps = self.fps * 0.9 + 0.1 / dt;
        }
        if self.frozen_morph.is_none() {
            let dropped = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));
            if let Some(path) = dropped.filter(|_| self.embedded_rect.is_none()) {
                self.path = path.to_string_lossy().into();
                self.open(ctx);
            }
            let idle = now.duration_since(self.last_interaction);
            let spacing = refinement_spacing(idle, ctx.pixels_per_point());
            // 先の段階を完成させてから細かくする。操作の再開時はすぐ軽い設定へ戻す。
            if !self.path.is_empty()
                && spacing != self.camera.mesh_pixels
                && (spacing > self.camera.mesh_pixels || !self.scene_pending)
            {
                self.camera.mesh_pixels = spacing;
                self.scene_pending = true;
            }
            let mut scene_changed = false;
            // 通知はArc参照だけなので全件取り込み、GPUへの転送件数は別に制限する。
            for message in drain_messages(&self.receive) {
                match message {
                    Message::Catalog(catalog, generation, credits) => {
                        let maximum = catalog.keys().map(|k| k.0).max().unwrap_or(6);
                        self.camera.maximum = if self.camera.auto {
                            maximum
                        } else {
                            self.camera.maximum.min(maximum)
                        };
                        if self.smoke.is_some() {
                            if let Some(z) = std::env::var("YAMAMUKI_TERRAIN_CHECK_LEVEL")
                                .ok()
                                .and_then(|v| v.parse::<u32>().ok())
                                .filter(|z| catalog.keys().any(|k| k.0 == *z))
                            {
                                self.camera.maximum = z;
                                self.camera.auto = false;
                            }
                        }
                        self.catalog = catalog;
                        self.generation = generation;
                        self.credits = credits;
                        self.queued = true;
                    }
                    Message::Packet(version, packet, root) => {
                        let key = packet.key;
                        self.packets.insert(key, packet);
                        if root {
                            scene_changed |= self.camera.projected(key).0;
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
                            if self
                                .remembered_until_selection
                                .is_some_and(|v| version >= v)
                            {
                                self.remembered_until_selection = None;
                            }
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
                        self.remembered_until_selection = None;
                        eprintln!("地形読込: {error}");
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
                    _ => {
                        self.pending_roots.push_front(key);
                        break;
                    }
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
            if self.embedded_rect.is_none() {
                egui::TopBottomPanel::top("settings").show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{
                ui.strong("山むき・データビューワ／地形");
                for (label,name) in [("全国","national"),("富士山","fuji"),("槍ヶ岳","yari"),("筑波山","tsukuba")] {
                    if ui.button(label).clicked(){self.region(name);}
                }
                ui.label(format!("{:0.0} fps",self.fps));
            });
            if let Some(loader) = self.landcover.as_ref().filter(|_| self.landcover_visible) {
                ui.horizontal_wrapped(|ui| self.landcover_style.level_picker(ui, loader.layered, loader.split_land, loader.unified_land, loader.has_bare, loader.has_hillshade));
            }
            egui::CollapsingHeader::new("表示・SQLite設定").default_open(true).show(ui,|ui|{
                ui.horizontal(|ui|{ui.text_edit_singleline(&mut self.path);if ui.button("地形SQLiteを開く").clicked(){self.open(ctx);}});
                ui.horizontal_wrapped(|ui|{
                    ui.checkbox(&mut self.landcover_visible,"地表分類");
                    ui.text_edit_singleline(&mut self.landcover_path);
                    if ui.button("地表SQLiteを開く").clicked(){self.open_landcover(ctx);}
                    ui.label(&self.landcover_status);
                    if let Some(loader) = self.landcover.as_ref().filter(|_| self.landcover_visible) {
                        self.landcover_style.controls(ui, loader.supports_blur, loader.layered, loader.has_hillshade);
                    }
                });
                ui.horizontal_wrapped(|ui|{
                    changed|=ui.checkbox(&mut self.camera.auto,"自動LOD").changed();
                    let old=self.camera.maximum;
                    egui::ComboBox::from_label("最大／固定").selected_text(format!("z{old}")).show_ui(ui,|ui|{
                        for z in self.catalog.keys().map(|k|k.0).collect::<std::collections::BTreeSet<_>>() {ui.selectable_value(&mut self.camera.maximum,z,format!("z{z}・{}m",ERRORS[(z-6) as usize]));}
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
            ui.label(&self.credits);
            if let Some(loader)=self.landcover.as_ref().filter(|_|self.landcover_visible){ui.label(&loader.credits);}
            let levels=self.shown.iter().map(|t|t.key.0).collect::<std::collections::BTreeSet<_>>();
            ui.label(format!("左ドラッグ 移動 ／ ホイール 拡大縮小 ／ 右ドラッグ 回転・傾き ／ 表示 {:?} ／ z6常駐 {}枚 ／ GPU {:0.1} MiB",
                levels,self.roots.len(),self.gpu.values().map(|t|t.bytes as f64).sum::<f64>()/1048576.));
            let (triangles,patches,extra_bytes)=self.renderer.metrics();
            ui.label(format!("制御点 {} ／ 描画三角形 {} ／ {}パッチ ／ GPU接合・モーフ {:0.1} MiB ／ 表示タイルの圧縮BLOB {:0.1} MiB",
                self.shown.iter().map(|t|t.controls).sum::<usize>(),
                triangles,patches,extra_bytes as f64/1048576.,
                self.shown.iter().map(|t|t.compressed_bytes as f64).sum::<f64>()/1048576.));
        });
            }
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
        }
        let time = ctx.input(|i| i.time).to_bits();
        let fresh_input =
            self.frozen_morph.is_none() && self.embedded_rect.is_none() && time != self.input_stamp;
        self.input_stamp = time;
        let mut draw = |ui: &mut egui::Ui| {
            let (rect, response) = ui.allocate_exact_size(
                ui.available_size().max(egui::vec2(1., 1.)),
                if self.embedded_rect.is_some() {
                    egui::Sense::hover()
                } else {
                    egui::Sense::drag()
                },
            );
            let viewport = [rect.width() as f64, rect.height() as f64];
            if self.frozen_morph.is_none() && self.camera.viewport != viewport {
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
                        self.camera.distance =
                            (self.camera.distance * (-wheel as f64 * 0.003).exp()).min(10_000_000.);
                        self.queued = true;
                    }
                }
            }
            let callback = self.renderer.frame(
                self.shown.clone(),
                self.local_background
                    .iter()
                    .chain(&self.roots)
                    .cloned()
                    .collect(),
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
                    self.frozen_morph.unwrap_or(t * t * (3. - 2. * t))
                },
                self.resolution == 0,
                rect,
                ctx.pixels_per_point(),
                self.landcover_visible,
                self.landcover_style.animate,
                self.landcover_style.bilinear,
            );
            ui.painter().add(callback);
            ui.painter().image(
                self.renderer.texture,
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                egui::Color32::WHITE,
            );
        };
        if let Some(rect) = self.embedded_rect {
            let mut ui = egui::Ui::new(
                ctx.clone(),
                egui::Id::new("terrain-map-canvas"),
                egui::UiBuilder::new().max_rect(rect),
            );
            ui.set_clip_rect(rect);
            draw(&mut ui);
        } else {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, draw);
        }

        if self.frozen_morph.is_some() {
            return;
        }
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
                    && self
                        .shown
                        .iter()
                        .map(|t| t.key)
                        .collect::<std::collections::BTreeSet<_>>()
                        == self
                            .wanted
                            .iter()
                            .copied()
                            .collect::<std::collections::BTreeSet<_>>();
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
            if self
                .send
                .send(Request {
                    version: self.version,
                    camera: self.camera.clone(),
                    prefer_finer: !self.settled,
                    resident: self.packets.keys().copied().collect(),
                })
                .is_err()
            {
                // 復号エラー等でワーカーが終了していても、次の操作を永久に待たせない。
                // 表示済みメッシュは保ち、元データを読み出すスレッドだけを再開する。
                self.start_worker(ctx);
                self.queued = true;
                self.status = "地形の読み込み処理を再開中".into();
            }
        }
        if self.smoke.is_some()
            && (self
                .ready_since
                .is_some_and(|t| now.duration_since(t) > Duration::from_secs(8))
                || now.duration_since(self.last_interaction) > Duration::from_secs(20))
            && !self.screenshot_requested
        {
            eprintln!(
                "描画検査: {} / loading={} / shown={} / wanted={}",
                self.status,
                self.loading,
                self.shown.len(),
                self.wanted.len()
            );
            for tile in &self.shown {
                if tile.local_field() {
                    eprintln!(
                        "局所評価 z{} / {:.1} m / {}分割 / 格子間隔 {:.3} m",
                        tile.key.0,
                        tile.side,
                        tile.n,
                        tile.side / tile.n as f64
                    );
                }
            }
            let (triangles, patches, bytes) = self.renderer.metrics();
            eprintln!(
                "描画量: {triangles} 三角形 / {patches} パッチ / 共有GPU情報 {:.2} MiB",
                bytes as f64 / 1048576.
            );
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
        if self.smoke.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if !self.path.is_empty()
            && (self.last_interaction.elapsed() < Duration::from_millis(1200)
                || self.camera.mesh_pixels
                    > refinement_spacing(Duration::from_millis(1200), ctx.pixels_per_point()))
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.loading
            || self.scene_pending
            || self.transition.elapsed() < Duration::from_millis(450)
            || self.renderer.landcover_blend(self.landcover_style.animate) < 1.
            || !self.pending_roots.is_empty()
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if !self.settled {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
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

/// 地形と同じ透視投影を地点マークとクリック座標にも使う。
pub struct Projection {
    camera: selection::Camera,
}
impl Projection {
    pub fn screen(&self, rect: egui::Rect, world: [f64; 2], height: f64) -> Option<egui::Pos2> {
        let c = &self.camera;
        let back = c.back();
        let right = glam::DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
        let up = back.cross(right);
        let d = if c.globe {
            globe::local(c.center, world.map(|v| v * shared_data::WORLD), height)
                - glam::DVec3::Y * c.height
        } else {
            glam::DVec3::new(
                (world[0] * shared_data::WORLD - c.center[0]) * c.scale(),
                height - c.height,
                (world[1] * shared_data::WORLD - c.center[1]) * c.scale(),
            )
        };
        if c.globe {
            let point = d + glam::DVec3::Y * c.height;
            let eye = back * c.distance + glam::DVec3::Y * c.height;
            if let Some(hit) = globe::intersect(eye, point - eye, 0.) {
                if hit.distance(eye) + 1. < point.distance(eye) {
                    return None;
                }
            }
        }
        let depth = c.distance - d.dot(back);
        if !depth.is_finite() || depth <= (c.distance / 10000.).max(0.2) || depth > c.far_depth() {
            return None;
        }
        let scale = rect.height() as f64 / (2. * c.tan_half_fov() * depth);
        Some(egui::pos2(
            rect.center().x + (d.dot(right) * scale) as f32,
            rect.center().y - (d.dot(up) * scale) as f32,
        ))
    }
    /// 詳細画像は、供給元の最大画素密度より狭い領域へ縮めない。
    pub fn near_material_bounds(&self, minimum_span: f64) -> [f64; 4] {
        let c = &self.camera;
        let half = (c.distance / c.scale()).max(minimum_span * shared_data::WORLD / 2.);
        [
            c.center[0] - half,
            c.center[1] - half,
            c.center[0] + half,
            c.center[1] + half,
        ]
        .map(|v| v / shared_data::WORLD)
    }
    /// 注視平面だけでなく、描画時の地形標高範囲への透視投影を含める。
    pub fn material_bounds(&self, rect: egui::Rect) -> [f64; 4] {
        let c = &self.camera;
        let back = c.back();
        let right = glam::DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
        let up = back.cross(right);
        let f = 2. * c.tan_half_fov() / rect.height() as f64;
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for height in [-500., c.height, 4500.] {
            let limit = 8. * (c.distance + (height - c.height).abs());
            for row in 0..=8 {
                for col in 0..=8 {
                    let x = rect.width() as f64 * (col as f64 / 8. - 0.5);
                    let y = rect.height() as f64 * (row as f64 / 8. - 0.5);
                    let ray = right * (x * f) - up * (y * f) - back;
                    if c.globe {
                        let origin = back * c.distance + glam::DVec3::Y * c.height;
                        // 視点を内包する高い標高球面の、地球の裏側の交点は視野に含めない。
                        if (origin + glam::DVec3::Y * globe::RADIUS).length()
                            <= globe::RADIUS + height
                        {
                            continue;
                        }
                        if let Some(p) = globe::intersect(
                            back * c.distance + glam::DVec3::Y * c.height,
                            ray,
                            height,
                        ) {
                            let mut world = globe::world(c.center, p);
                            // 範囲計算中は注視点から連続した経度に保ち、取得するときに分割する。
                            world[0] = c.center[0]
                                + (world[0] - c.center[0] + shared_data::WORLD / 2.)
                                    .rem_euclid(shared_data::WORLD)
                                - shared_data::WORLD / 2.;
                            for axis in 0..2 {
                                bounds[axis] = bounds[axis].min(world[axis]);
                                bounds[axis + 2] = bounds[axis + 2].max(world[axis]);
                            }
                        }
                        continue;
                    }
                    if ray.y.abs() < 1e-9 {
                        continue;
                    }
                    let t = (height - c.height - back.y * c.distance) / ray.y;
                    if t < 0. {
                        continue;
                    }
                    let local = back * c.distance + ray * t.min(limit);
                    let world = [
                        c.center[0] + local.x / c.scale(),
                        c.center[1] + local.z / c.scale(),
                    ];
                    for axis in 0..2 {
                        bounds[axis] = bounds[axis].min(world[axis]);
                        bounds[axis + 2] = bounds[axis + 2].max(world[axis]);
                    }
                }
            }
        }
        for axis in 0..2 {
            let pad = (bounds[axis + 2] - bounds[axis]) * 0.08;
            bounds[axis] -= pad;
            bounds[axis + 2] += pad;
        }
        bounds.map(|v| v / shared_data::WORLD)
    }
    pub fn world_at(&self, rect: egui::Rect, p: egui::Pos2) -> Option<[f64; 2]> {
        let c = &self.camera;
        let back = c.back();
        let right = glam::DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
        let up = back.cross(right);
        let f = 2. * c.tan_half_fov() / rect.height() as f64;
        let ray = right * ((p.x - rect.center().x) as f64 * f)
            - up * ((p.y - rect.center().y) as f64 * f)
            - back;
        if c.globe {
            let origin = back * c.distance + glam::DVec3::Y * c.height;
            let p = globe::intersect(origin, ray, c.height)?;
            return Some(globe::world(c.center, p).map(|v| v / shared_data::WORLD));
        }
        if ray.y >= -1e-9 {
            return None;
        }
        let local = back * c.distance + ray * (-back.y * c.distance / ray.y);
        Some([
            c.center[0] / shared_data::WORLD + local.x / (c.scale() * shared_data::WORLD),
            c.center[1] / shared_data::WORLD + local.z / (c.scale() * shared_data::WORLD),
        ])
    }
}
fn projected_material_bands(
    c: &selection::Camera,
    rect: egui::Rect,
    minimum_span: f64,
    regions: [[f64; 4]; 3],
) -> Vec<([f64; 4], f64)> {
    let eye = c.eye();
    let center = [eye[0] / shared_data::WORLD, eye[1] / shared_data::WORLD];
    let mut result = vec![(
        regions[0],
        (rect.width().max(rect.height()) as f64
            / (256. * (regions[0][2] - regions[0][0]).max(regions[0][3] - regions[0][1]) * 1.02))
            .log2(),
    )];
    let mut radius = (minimum_span / 2.).max(64. / (c.scale() * shared_data::WORLD));
    let limit = (0..2)
        .flat_map(|axis| {
            [
                (regions[2][axis] - center[axis]).abs(),
                (regions[2][axis + 2] - center[axis]).abs(),
            ]
        })
        .fold(0., f64::max)
        .max(radius);
    for _ in 0..20 {
        let bounds = [
            center[0] - radius,
            center[1] - radius,
            center[0] + radius,
            center[1] + radius,
        ];
        // 内側の境界（半径の半分）を基準に、横方向の2画素に必要な密度を確保する。
        let horizontal = radius * shared_data::WORLD * c.scale() / 2.;
        let vertical = eye[2] - eye[2].clamp(-500., 4500.);
        let distance = horizontal.hypot(vertical).max(1.);
        let zoom = (shared_data::WORLD * c.scale() * rect.height() as f64
            / (1024. * c.tan_half_fov() * distance))
            .log2();
        result.push((bounds, zoom));
        if radius >= limit {
            break;
        }
        radius *= 2.;
    }
    result
}
fn wire_control(ui: &mut egui::Ui, wire: &mut u32) {
    let labels = ["なし", "描画網", "描画網のみ", "制御点", "制御点のみ"];
    // 親メニューと同じ階層管理を使い、選択前に親だけ閉じることを防ぐ。
    ui.menu_button(labels[*wire as usize], |ui| {
        for (i, label) in labels.iter().enumerate() {
            if ui.selectable_value(wire, i as u32, *label).clicked() {
                ui.close_menu();
            }
        }
    })
    .response
    .on_hover_text("ワイヤーフレーム表示");
}
/// 通常ビューワーへ埋め込む3D地形表示。
pub struct TerrainView {
    label: String,
    app: App,
    automatic_maximum: Option<u32>,
}
impl TerrainView {
    pub fn flat(ctx: &egui::Context, state: &eframe::egui_wgpu::RenderState) -> Result<Self> {
        let mut app = App::new(ctx, state.clone(), PathBuf::new(), None, "national", None);
        app.shown = vec![app.renderer.upload(&Packet::flat()?, 1)?];
        app.camera.height = 0.;
        app.camera.globe = true;
        app.camera.auto = false;
        app.resolution = 0;
        app.loading = false;
        app.queued = false;
        app.settled = true;
        app.scene_pending = false;
        app.camera.mesh_pixels =
            refinement_spacing(Duration::from_millis(1200), ctx.pixels_per_point());
        Ok(Self {
            label: String::new(),
            app,
            automatic_maximum: None,
        })
    }
    pub fn new(
        ctx: &egui::Context,
        state: &eframe::egui_wgpu::RenderState,
        path: PathBuf,
    ) -> Result<Self> {
        let label = yamamuki_data_sqlite::open(&path)
            .and_then(|conn| yamamuki_data_sqlite::datasets(&conn))?
            .into_iter()
            .find(|d| d.kind == "terrain")
            .map(|d| d.title)
            .unwrap_or_else(|| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        let mut app = App::new(ctx, state.clone(), path, None, "national", None);
        app.camera.globe = true;
        Ok(Self {
            label,
            app,
            automatic_maximum: None,
        })
    }
    /// GPUへ最初の地形メッシュを描画したか。ファイルのメタデータ取得とは区別する。
    pub fn has_drawn_terrain(&self) -> bool {
        !self.app.path.is_empty() && self.app.renderer.has_drawn_mesh()
    }
    pub fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        hit_test::poll(
            &self.app.renderer.hit_state,
            &self.app.renderer.state.device,
            ctx,
        );
        eframe::App::update(&mut self.app, ctx, frame);
    }
    pub fn hit_test(
        &self,
        ctx: &egui::Context,
        rect: egui::Rect,
        position: egui::Pos2,
    ) -> mpsc::Receiver<Result<Option<Hit>>> {
        hit_test::request(
            &self.app.renderer.hit_state,
            ctx,
            [
                (position.x - rect.left()) / rect.width(),
                (position.y - rect.top()) / rect.height(),
            ],
        )
    }
    pub fn show(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame, rect: egui::Rect) {
        self.app.embedded_rect = Some(rect);
        self.update(ui.ctx(), frame);
    }
    pub fn set_map_camera(
        &mut self,
        center: [f64; 2],
        zoom: f64,
        yaw: f64,
        tilt: f64,
        rect: egui::Rect,
    ) {
        if self.app.camera.set_map_view(
            center,
            zoom,
            yaw,
            tilt,
            [rect.width() as f64, rect.height() as f64],
        ) {
            if !self.app.path.is_empty() {
                self.app.queued = true;
                self.app.settled = false;
            }
            self.app.last_interaction = Instant::now();
        }
    }
    pub fn elevation_revision(&self, world: [f64; 2]) -> Option<(u32, i32, i32, u32)> {
        let p = world.map(|v| v * shared_data::WORLD);
        self.app
            .shown
            .iter()
            .chain(&self.app.local_background)
            .chain(&self.app.roots)
            .filter(|tile| {
                let b = shared_data::bounds(tile.key);
                p[0] >= b[0] && p[0] < b[2] && p[1] >= b[1] && p[1] < b[3]
            })
            .map(|tile| tile.key)
            .max_by_key(|key| key.0)
            .or_else(|| {
                self.app
                    .catalog
                    .keys()
                    .filter(|key| {
                        let b = shared_data::bounds(**key);
                        p[0] >= b[0] && p[0] < b[2] && p[1] >= b[1] && p[1] < b[3]
                    })
                    .max_by_key(|key| key.0)
                    .copied()
            })
            .map(|key| (key.0, key.1, key.2, self.app.settings[0].to_bits()))
    }
    pub fn request_elevation(
        &self,
        ctx: &egui::Context,
        world: [f64; 2],
    ) -> mpsc::Receiver<Result<f64>> {
        let (send, receive) = mpsc::channel();
        let path = PathBuf::from(&self.app.path);
        let amount = f64::from(self.app.settings[0]);
        let ctx = ctx.clone();
        let key = self.elevation_revision(world).map(|(z, x, y, _)| (z, x, y));
        let packet = key.and_then(|key| self.app.packets.get(&key).cloned());
        std::thread::spawn(move || {
            let result = (|| -> Result<f64> {
                let meters = world.map(|v| v * shared_data::WORLD);
                let packet = if let Some(packet) = packet {
                    packet
                } else {
                    let mut db = Database::open(&path)?;
                    let key = key.or_else(|| {
                        db.catalog
                            .keys()
                            .filter(|key| {
                                let b = shared_data::bounds(**key);
                                meters[0] >= b[0]
                                    && meters[0] < b[2]
                                    && meters[1] >= b[1]
                                    && meters[1] < b[3]
                            })
                            .max_by_key(|key| key.0)
                            .copied()
                    });
                    let Some(key) = key else {
                        return Ok(0.);
                    };
                    db.packet(key)?
                };
                packet.elevation_at(meters, amount)
            })();
            let _ = send.send(result);
            ctx.request_repaint();
        });
        receive
    }
    pub fn point_projection(
        &self,
        ctx: &egui::Context,
        points: &[[f64; 3]],
    ) -> Option<Vec<[f32; 4]>> {
        use std::hash::{Hash, Hasher};
        let c = &self.app.camera;
        if points.is_empty() {
            return Some(Vec::new());
        }
        if self.app.shown.is_empty()
            && self.app.local_background.is_empty()
            && self.app.roots.is_empty()
        {
            let rect = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(c.viewport[0] as f32, c.viewport[1] as f32),
            );
            let projection = self.projection();
            return Some(
                points
                    .iter()
                    .map(|p| {
                        projection
                            .screen(rect, [p[0], p[1]], 2.)
                            .map_or([0.; 4], |v| {
                                [
                                    v.x / rect.width(),
                                    v.y / rect.height(),
                                    2.,
                                    if rect.contains(v) { 1. } else { 0. },
                                ]
                            })
                    })
                    .collect(),
            );
        }
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        ctx.pixels_per_point().to_bits().hash(&mut hash);
        for x in [
            c.center[0],
            c.center[1],
            c.height,
            c.distance,
            c.yaw,
            c.pitch,
            c.fov_y,
            c.viewport[0],
            c.viewport[1],
        ] {
            x.to_bits().hash(&mut hash);
        }
        self.app.settings[0].to_bits().hash(&mut hash);
        self.app.join.hash(&mut hash);
        for tile in self
            .app
            .shown
            .iter()
            .chain(&self.app.local_background)
            .chain(&self.app.roots)
        {
            (Arc::as_ptr(tile) as usize).hash(&mut hash);
        }
        if self.app.transition.elapsed() < Duration::from_millis(450) {
            (self.app.transition.elapsed().as_millis() / 100).hash(&mut hash);
        }
        for point in points {
            for x in point {
                x.to_bits().hash(&mut hash);
            }
        }
        point_visibility::query(
            &self.app.renderer.state.device,
            ctx,
            &self.app.renderer.point_state,
            c,
            hash.finish(),
            points,
        )
    }
    pub fn camera_height(&self) -> f64 {
        self.app.camera.height
    }
    pub fn restore_camera_height(&mut self, height: f64) {
        self.app.camera.height = height;
    }
    pub fn set_observer_camera(
        &mut self,
        world: [f64; 2],
        height: f64,
        yaw: f64,
        pitch: f64,
        fov: f64,
        rect: egui::Rect,
    ) -> f64 {
        let before = self.app.camera.clone();
        if self.app.camera.set_observer(
            world.map(|v| v * shared_data::WORLD),
            height,
            yaw,
            pitch,
            fov,
            [rect.width() as f64, rect.height() as f64],
        ) {
            // 方角だけなら全方位の準備を中断せず、そのまま描画方向だけ変える。
            if !self.app.path.is_empty() && !before.same_preparation(&self.app.camera) {
                self.app.restore_remembered_scene();
                self.app.queued = true;
                self.app.scene_pending = true;
                self.app.settled = false;
                self.app.last_interaction = Instant::now();
            }
        }
        let c = &self.app.camera;
        (shared_data::WORLD * c.scale() * c.viewport[1]
            / (256. * 2. * c.tan_half_fov() * c.distance))
            .log2()
    }
    pub fn label(&self) -> &str {
        &self.label
    }
    /// 主表示の実際の描画レベル。全国の常駐背景だけでは低レベル表示と判定しない。
    pub fn displayed_levels(&self) -> Vec<u32> {
        let previous = self
            .app
            .previous
            .iter()
            .filter(|_| self.app.transition.elapsed() < Duration::from_millis(450));
        let levels = self
            .app
            .shown
            .iter()
            .chain(previous)
            .filter(|tile| {
                self.app
                    .camera
                    .visible_depth([
                        tile.origin[0],
                        tile.origin[1],
                        tile.origin[0] + tile.side,
                        tile.origin[1] + tile.side,
                    ])
                    .is_some()
            })
            .map(|tile| tile.key.0)
            .collect::<std::collections::BTreeSet<_>>();
        levels.into_iter().collect()
    }
    pub fn level_label(&self) -> String {
        let levels = self.displayed_levels();
        let level = match (levels.first(), levels.last()) {
            (Some(first), Some(last)) if first != last => format!("z{first}〜z{last}"),
            (Some(z), _) => format!("z{z}"),
            _ => "読込中".into(),
        };
        if self.app.camera.auto {
            format!("自動 {level}")
        } else {
            level
        }
    }
    fn select_level(&mut self, level: Option<u32>) {
        match level {
            Some(z) => {
                if self.app.camera.auto {
                    self.automatic_maximum = Some(self.app.camera.maximum);
                }
                self.app.camera.auto = false;
                self.app.camera.maximum = z;
            }
            None => {
                if !self.app.camera.auto {
                    self.app.camera.maximum = self.automatic_maximum.take().unwrap_or_else(|| {
                        self.app.catalog.keys().map(|k| k.0).max().unwrap_or(13)
                    });
                }
                self.app.camera.auto = true;
            }
        }
        self.app.queued = true;
        self.app.settled = false;
        self.app.last_interaction = Instant::now();
    }
    pub fn layer_lod(&mut self, ui: &mut egui::Ui) {
        let before = (!self.app.camera.auto).then_some(self.app.camera.maximum);
        let mut selected = before;
        let label = if self.app.camera.auto {
            self.level_label()
        } else {
            format!("z{}", self.app.camera.maximum)
        };
        let levels = self
            .app
            .catalog
            .keys()
            .map(|k| k.0)
            .collect::<std::collections::BTreeSet<_>>();
        egui::ComboBox::from_id_salt("terrain-layer-lod")
            .width(106.)
            .selected_text(label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected, None, "自動");
                for z in levels {
                    ui.selectable_value(&mut selected, Some(z), format!("z{z}"));
                }
            })
            .response
            .on_hover_text(format!("現在の主表示: {}", self.level_label()));
        if selected != before {
            self.select_level(selected);
        }
    }
    pub fn lod_controls(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        if let (Some(min), Some(max)) = (
            self.app.catalog.keys().map(|k| k.0).min(),
            self.app.catalog.keys().map(|k| k.0).max(),
        ) {
            let mut maximum = if self.app.camera.auto {
                self.app.camera.maximum
            } else {
                self.automatic_maximum.unwrap_or(max)
            };
            if ui
                .add(
                    egui::Slider::new(&mut maximum, min..=max)
                        .prefix("z")
                        .text("レベル上限"),
                )
                .changed()
            {
                if self.app.camera.auto {
                    self.app.camera.maximum = maximum;
                } else {
                    self.automatic_maximum = Some(maximum);
                }
                changed = true;
            }
        }
        changed |= ui
            .add(egui::Slider::new(&mut self.app.camera.lod_quality, 0.5..=4.).text("精細度"))
            .changed();
        if changed && self.app.camera.auto {
            self.app.queued = true;
            self.app.settled = false;
            self.app.last_interaction = Instant::now();
        }
    }
    pub fn display_controls(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::Slider::new(&mut self.app.settings[0], 0.0..=1.).text("傾き調整"));
        ui.add(egui::Slider::new(&mut self.app.settings[1], 0.0..=3.).text("地形の陰影の強さ"));
        ui.checkbox(&mut self.app.join, "境界接合");
        ui.checkbox(&mut self.app.animate, "LODを滑らかに切替");
        ui.checkbox(&mut self.app.borders, "タイル境界");
        wire_control(ui, &mut self.app.wire);
        ui.small(&self.app.status);
    }
    /// 俯瞰・景色とも、一枚の広域範囲で中景全体のLODを落とさない。
    /// 距離が2倍になるごとに、投影画素密度に対応する一段粗い画像へ移る。
    pub fn projected_material_regions(
        &self,
        rect: egui::Rect,
        minimum_span: f64,
    ) -> Vec<([f64; 4], f64)> {
        let regions = self.material_regions(rect, minimum_span);
        if !self.app.camera.observer && self.app.camera.pitch >= std::f64::consts::FRAC_PI_3 {
            let c = &self.app.camera;
            let zoom = (shared_data::WORLD * c.scale() * f64::from(rect.height())
                / (512. * c.tan_half_fov() * c.distance))
                .log2();
            // 視野全体は同じLODにするが、視野外の常駐背景まで詳細画像で埋めない。
            let overview = (f64::from(rect.width().max(rect.height()))
                / (256.
                    * (regions[0][2] - regions[0][0]).max(regions[0][3] - regions[0][1])
                    * 1.02))
                .log2()
                .min(zoom);
            return vec![(regions[0], overview), (regions[2], zoom)];
        }
        projected_material_bands(&self.app.camera, rect, minimum_span, regions)
    }

    pub fn material_regions(&self, rect: egui::Rect, minimum_span: f64) -> [[f64; 4]; 3] {
        let camera = &self.app.camera;
        let near = self
            .projection()
            .near_material_bounds(minimum_span)
            .map(|v| v * shared_data::WORLD);
        let mut far = near;
        // 読込・切替中の粗タイルも必ず材質で覆う。
        for tile in self
            .app
            .roots
            .iter()
            .chain(&self.app.shown)
            .chain(&self.app.previous)
        {
            // 標高0mの一枚の平面は全世界を覆うが、全世界分の画像は要求しない。
            let b = shared_data::bounds(tile.key);
            if !self.app.path.is_empty() && camera.visible_depth(b).is_some() {
                for axis in 0..2 {
                    far[axis] = far[axis].min(b[axis]);
                    far[axis + 2] = far[axis + 2].max(b[axis + 2]);
                }
            }
        }
        let mut mid = self
            .projection()
            .material_bounds(rect)
            .map(|v| v * shared_data::WORLD);
        for axis in 0..2 {
            mid[axis] = mid[axis].min(near[axis]);
            mid[axis + 2] = mid[axis + 2].max(near[axis + 2]);
            far[axis] = far[axis].min(mid[axis]);
            far[axis + 2] = far[axis + 2].max(mid[axis + 2]);
        }
        [far, near, mid].map(|b| b.map(|v| v / shared_data::WORLD))
    }
    pub fn material_parts(&self, area: [f64; 4]) -> Vec<[f64; 4]> {
        if self.app.camera.globe {
            globe::texture_parts(area)
        } else {
            vec![area]
        }
    }
    pub fn set_direct_materials(&mut self, commands: Vec<direct_material::Quad>) {
        self.app.renderer.direct = Some(commands);
        self.app.renderer.clear_landcover();
    }
    pub fn set_materials(&mut self, views: [Option<(eframe::wgpu::TextureView, [f64; 4])>; 3]) {
        self.app.renderer.direct = None;
        self.app.landcover = None;
        self.app.landcover_visible = views.iter().any(Option::is_some);
        if self.app.landcover_visible {
            self.app.renderer.set_external_materials(views);
        } else {
            self.app.renderer.clear_landcover();
        }
    }
    pub fn projection(&self) -> Projection {
        Projection {
            camera: self.app.camera.clone(),
        }
    }
    pub fn capture(&mut self, output: PathBuf) {
        self.app.smoke = Some(output);
        self.app.region("fuji");
        if let Some(height) = std::env::var("YAMAMUKI_TERRAIN_CHECK_HEIGHT")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
        {
            self.app.camera.height = height;
        }
    }
    pub fn open_surface(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.app.landcover_path = path.to_string_lossy().into_owned();
        self.app.landcover_visible = true;
        self.app.open_landcover(ctx);
    }
}
/// SQLiteの内容で判別する。未知の地形版は別種類として読み替えない。
pub fn is_terrain(path: &std::path::Path) -> Result<bool> {
    let conn = yamamuki_data_sqlite::open(path)?;
    let sets = yamamuki_data_sqlite::datasets(&conn)?;
    if let Some(dataset) = sets.iter().find(|d| d.kind == "terrain") {
        anyhow::ensure!(dataset.format_version == 1, "未対応の地形形式です");
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn launch(path: PathBuf, surface: Option<PathBuf>) -> Result<()> {
    launch_internal(path, surface, None, "national")
}
/// 実ウィンドウのGPU描画をPNGへ保存して終了する、手元の表示検査用入口。
pub fn render_check(path: PathBuf, output: PathBuf) -> Result<()> {
    launch_internal(path, None, Some(output), "fuji")
}
fn launch_internal(
    path: PathBuf,
    surface: Option<PathBuf>,
    smoke: Option<PathBuf>,
    region: &'static str,
) -> Result<()> {
    Database::open(&path)?;
    let surface = surface.or_else(|| {
        let conn = yamamuki_data_sqlite::open(&path).ok()?;
        yamamuki_data_sqlite::datasets(&conn)
            .ok()?
            .iter()
            .any(|dataset| dataset.kind == "surface")
            .then(|| path.clone())
    });
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::default();
    #[cfg(target_os = "windows")]
    {
        setup.instance_descriptor.backends = wgpu::Backends::DX12;
    }
    #[cfg(target_os = "macos")]
    {
        setup.instance_descriptor.backends = wgpu::Backends::METAL;
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
        "山むき・データビューワ — 地形",
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new(
                &cc.egui_ctx,
                cc.wgpu_render_state.as_ref().expect("wgpu初期化").clone(),
                path,
                smoke,
                region,
                surface,
            )))
        }),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
}
#[cfg(test)]
mod notification_tests {
    use super::*;
    #[test]
    fn camera_idle_refines_in_stages_and_scales_for_dpi() {
        for ppp in [1., 1.5, 2.] {
            for (millis, pixels) in [
                (0, 3.),
                (399, 3.),
                (400, 1.5),
                (1199, 1.5),
                (1200, 0.75),
                (8000, 0.75),
            ] {
                assert_eq!(
                    refinement_spacing(Duration::from_millis(millis), ppp) * f64::from(ppp),
                    pixels
                );
            }
        }
    }
    #[test]
    #[ignore = "実データでズーム往復の計画を計測する"]
    fn benchmark_zoom_roundtrip_planning() -> Result<()> {
        let path = std::env::var("YAMAMUKI_TERRAIN_BENCH_DB")?;
        let db = Database::open(std::path::Path::new(&path))?;
        let mut camera = selection::Camera {
            height: 1800.,
            ..Default::default()
        };
        let center = selection::lonlat(138.7274, 35.3606).map(|v| v / shared_data::WORLD);
        for zoom in [8., 12., 16., 20., 24., 28., 24., 20., 16., 12., 8.] {
            camera.set_map_view(center, zoom, 0., 0.8, [1600., 1000.]);
            let start = Instant::now();
            let keys = selection::select(&db.catalog, &camera);
            let selected = start.elapsed();
            let mut fields = 0;
            let mut cells = 0u64;
            let coverage = shared_field::Coverage::new(&keys);
            for (key, background) in keys
                .iter()
                .map(|k| (*k, false))
                .chain(db.catalog.keys().filter(|k| k.0 == 6).map(|k| (*k, true)))
            {
                for field in shared_field::Field::visible(key, &camera) {
                    if background && coverage.covers(key, field) {
                        continue;
                    }
                    let pixels = field.pixels(key, &camera).unwrap_or(0.);
                    let n = ((pixels / 3.).ceil().clamp(16., 512.) as u32).next_power_of_two();
                    fields += 1;
                    cells += u64::from(n + 1).pow(2);
                }
            }
            eprintln!(
                "z{zoom}: {}タイル / {fields}区画 / {cells}候補点 / 選択 {:.2} ms / 全計画 {:.2} ms",
                keys.len(),
                selected.as_secs_f64() * 1000.,
                start.elapsed().as_secs_f64() * 1000.
            );
        }
        Ok(())
    }
    #[test]
    fn latest_visible_tiles_arrive_before_background_roots() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "terrain-worker-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let mut conn = yamamuki_data_sqlite::rusqlite::Connection::open(&path)?;
        yamamuki_data_sqlite::initialize(&conn)?;
        yamamuki_terrain::fixture::create(&mut conn)?;
        drop(conn);
        let (tx, requests) = mpsc::channel();
        let (messages, rx) = mpsc::channel();
        let camera = selection::Camera {
            center: selection::lonlat(138.7274, 35.3606),
            distance: 10000.,
            auto: false,
            maximum: 13,
            ..Default::default()
        };
        for version in [1, 2] {
            tx.send(Request {
                version,
                camera: camera.clone(),
                prefer_finer: false,
                resident: HashSet::new(),
            })?;
        }
        let source = path.clone();
        let handle = std::thread::spawn(move || {
            worker(source, requests, messages, egui::Context::default())
        });
        let mut selected = false;
        let mut delivered = false;
        loop {
            match rx.recv_timeout(Duration::from_secs(5))? {
                Message::Catalog(..) => {}
                Message::Selected(version, keys) => {
                    assert_eq!(version, 2);
                    assert!(!keys.is_empty());
                    selected = true;
                }
                Message::Packet(version, _, root) => {
                    assert!(selected && !root);
                    assert_eq!(version, 2);
                    delivered = true;
                }
                Message::Ready(version, ..) => {
                    assert!(delivered);
                    assert_eq!(version, 2);
                    break;
                }
                Message::Error(error) => anyhow::bail!(error),
            }
        }
        drop(tx);
        handle.join().unwrap();
        std::fs::remove_file(path)?;
        Ok(())
    }
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

#[cfg(test)]
mod projection_tests {
    use super::*;
    #[test]
    fn observer_images_use_gradual_distance_levels() {
        let mut camera = selection::Camera::default();
        camera.set_observer(
            selection::lonlat(138.7274, 35.3606),
            3778.,
            0.,
            0.,
            45f64.to_radians(),
            [1200., 800.],
        );
        let center = camera.eye();
        let cx = center[0] / shared_data::WORLD;
        let cy = center[1] / shared_data::WORLD;
        let area = |r: f64| [cx - r, cy - r, cx + r, cy + r];
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200., 800.));
        let bands = projected_material_bands(
            &camera,
            rect,
            0.00002,
            [area(0.1), area(0.00001), area(0.01)],
        );
        assert!(bands.len() > 8 && bands.len() <= 21);
        for pair in bands[1..].windows(2) {
            let r0 = pair[0].0[2] - pair[0].0[0];
            let r1 = pair[1].0[2] - pair[1].0[0];
            assert!((r1 / r0 - 2.).abs() < 1e-7);
            assert!((pair[0].1 - pair[1].1 - 1.).abs() < 1e-7);
        }
        // 近景の外に出ても、数km先にz8等の広域密度を使わない。
        assert!(bands[4].1 > 14.);
        assert!(bands.last().unwrap().0[0] <= area(0.01)[0]);
    }
    #[test]
    fn tilted_orbit_uses_projected_density_beyond_the_near_texture() {
        let camera = selection::Camera {
            center: [35231276.17590823, 15878191.672569184],
            height: 500.,
            distance: 4652.565323126368,
            yaw: 1.2048033475875854,
            pitch: 0.3329332629826407,
            viewport: [2686., 1562.8125],
            ..Default::default()
        };
        let projection = Projection {
            camera: camera.clone(),
        };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2686., 1562.8125));
        let near = projection.near_material_bounds(0.00004);
        let mid = projection.material_bounds(rect);
        let bands = projected_material_bands(&camera, rect, 0.00004, [mid, near, mid]);
        let eye = camera.eye();
        let back = camera.back();
        let p = [
            (eye[0] - back.x * 12000. / camera.scale()) / shared_data::WORLD,
            (eye[1] - back.z * 12000. / camera.scale()) / shared_data::WORLD,
        ];
        assert!(
            !(p[0] >= near[0] && p[0] <= near[2] && p[1] >= near[1] && p[1] <= near[3]),
            "中景の点を検証できていない"
        );
        let chosen = bands[1..]
            .iter()
            .find(|(b, _)| p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3])
            .unwrap()
            .1;
        let old = (2686. / (256. * (mid[2] - mid[0]).max(mid[3] - mid[1]) * 1.02)).log2();
        assert!(
            chosen >= old + 1.,
            "中景全体の面積に引きずられている: old={old}, selected={chosen}"
        );
        eprintln!("提供視点の中景: 旧z{old:.2} → 距離別z{chosen:.2}");
        // 高空の俯瞰では、近景の小さな範囲を理由に最高LODを要求しない。
        let mut high = camera;
        high.distance = 2_000_000.;
        high.pitch = 1.4;
        let high_bands = projected_material_bands(&high, rect, 0.00004, [mid, near, mid]);
        assert!(high_bands[1].1 < 10.);
    }
    #[test]
    fn provided_globe_view_keeps_texture_coverage_across_the_date_line() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1955.2188, 1326.8125));
        let mut c = selection::Camera {
            height: 0.,
            globe: true,
            ..Default::default()
        };
        c.set_map_view(
            [0.8569477284471069, 0.40443680425979317],
            3.444444410337342,
            0.,
            0.,
            [rect.width() as f64, rect.height() as f64],
        );
        let p = Projection { camera: c };
        let mut b = p.material_bounds(rect);
        let n = p.near_material_bounds(0.0001);
        for axis in 0..2 {
            b[axis] = b[axis].min(n[axis]);
            b[axis + 2] = b[axis + 2].max(n[axis + 2]);
        }
        let parts = globe::texture_parts(b);
        let mut wrapped = 0;
        for row in 0..64 {
            for col in 0..96 {
                let pixel = egui::pos2(
                    (col as f32 + 0.5) * rect.width() / 96.,
                    (row as f32 + 0.5) * rect.height() / 64.,
                );
                if let Some(w) = p.world_at(rect, pixel) {
                    if !(0. ..=1.).contains(&w[1]) {
                        continue;
                    }
                    if w[0] < 0.2 {
                        wrapped += 1;
                    }
                    assert!(
                        parts
                            .iter()
                            .any(|b| w[0] >= b[0] && w[0] <= b[2] && w[1] >= b[1] && w[1] <= b[3]),
                        "表示中の球面を画像取得範囲が覆っていない: {w:?} / {parts:?}"
                    );
                }
            }
        }
        assert!(wrapped > 0, "日付変更線の先を検証していない");
    }
    #[test]
    fn sea_level_plane_uses_the_same_camera_for_pan_zoom_orbit_and_picking() {
        let rect = egui::Rect::from_min_size(egui::pos2(40., 70.), egui::vec2(1600., 1000.));
        let center = selection::lonlat(138.7274, 35.3606).map(|v| v / shared_data::WORLD);
        let mut camera = selection::Camera {
            height: 0.,
            ..Default::default()
        };
        for zoom in [6., 12., 18., 24.] {
            for tilt in [0., 0.5, 1.2] {
                for yaw in [0., 1.4, 3.] {
                    camera.set_map_view(center, zoom, yaw, tilt, [1600., 1000.]);
                    let projection = Projection {
                        camera: camera.clone(),
                    };
                    assert!(
                        projection
                            .screen(rect, center, 0.)
                            .unwrap()
                            .distance(rect.center())
                            < 0.001
                    );
                    for pixel in [rect.center(), rect.center() + egui::vec2(150., 200.)] {
                        let world = projection.world_at(rect, pixel).unwrap();
                        assert!(projection.screen(rect, world, 0.).unwrap().distance(pixel) < 0.1);
                    }
                    assert_eq!(camera.height, 0.);
                }
            }
        }
    }
    #[test]
    fn observer_rotation_and_zoom_keep_the_eye_at_the_selected_point() {
        let world = selection::lonlat(138.7274, 35.3606);
        let mut camera = selection::Camera::default();
        for yaw in [0., 1., 2., 4., 6.] {
            for pitch in [-1., 0., 1.] {
                for degrees in [20f64, 45., 90.] {
                    camera.set_observer(
                        world,
                        3778.,
                        yaw,
                        pitch,
                        degrees.to_radians(),
                        [1600., 900.],
                    );
                    let eye = camera.eye();
                    assert!(
                        (eye[0] - world[0]).abs() < 0.0001 && (eye[1] - world[1]).abs() < 0.0001
                    );
                    assert!((eye[2] - 3778.).abs() < 0.0001);
                }
            }
        }
    }
    #[test]
    fn overzoom_does_not_collapse_texture_coverage() {
        let minimum = 1536. / (256. * 2f64.powi(18));
        let mut camera = selection::Camera {
            center: selection::lonlat(138.7274, 35.3606),
            height: 500.,
            pitch: 0.3,
            ..Default::default()
        };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200., 800.));
        let mut previous = None;
        for distance in [10., 1., 0.01, 0.000001] {
            camera.distance = distance;
            let projection = Projection {
                camera: camera.clone(),
            };
            let near = projection.near_material_bounds(minimum);
            assert!((near[2] - near[0] - minimum).abs() < 1e-12);
            if let Some(old) = previous {
                assert_eq!(near, old);
            }
            previous = Some(near);
            let mid = projection.material_bounds(rect);
            assert!(mid.iter().all(|v| v.is_finite()));
            // 起伏のある地形が見える範囲を、距離ゼロ付近でもゼロへ縮めない。
            assert!((mid[2] - mid[0]) * shared_data::WORLD > 1000.);
        }
    }
    #[test]
    fn tilted_material_covers_visible_ground_and_finite_horizon() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200., 800.));
        for pitch in [0.15, 0.5, 0.9, 1.55] {
            let p = Projection {
                camera: selection::Camera {
                    pitch,
                    ..Default::default()
                },
            };
            let b = p.material_bounds(rect);
            assert!(b.iter().all(|v| v.is_finite()));
            for point in [rect.center(), rect.left_bottom(), rect.right_bottom()] {
                let world = p.world_at(rect, point).unwrap();
                assert!(
                    world[0] >= b[0] && world[0] <= b[2] && world[1] >= b[1] && world[1] <= b[3]
                );
            }
        }
    }
    #[test]
    fn markers_and_clicks_share_the_terrain_camera() {
        let rect = egui::Rect::from_min_size(egui::pos2(100., 50.), egui::vec2(900., 600.));
        for yaw in [0., 1.4, 3.] {
            for pitch in [0.3, 0.8, 1.55] {
                let camera = selection::Camera {
                    center: selection::lonlat(138.7274, 35.3606),
                    height: 1200.,
                    distance: 10000.,
                    yaw,
                    pitch,
                    ..Default::default()
                };
                let center = camera.center.map(|v| v / shared_data::WORLD);
                let p = Projection { camera };
                assert!(
                    p.screen(rect, center, 1200.)
                        .unwrap()
                        .distance(rect.center())
                        < 0.001
                );
                for delta in [[0., 0.], [0.00001, -0.00001], [-0.00001, 0.00002]] {
                    let world = [center[0] + delta[0], center[1] + delta[1]];
                    let screen = p.screen(rect, world, 1200.).unwrap();
                    let restored = p.world_at(rect, screen).unwrap();
                    assert!(
                        (world[0] - restored[0]).abs() < 1e-9
                            && (world[1] - restored[1]).abs() < 1e-9
                    );
                }
                assert!(p.screen(rect, center, 1300.).unwrap().y < rect.center().y);
            }
        }
    }
}

#[cfg(test)]
mod wire_menu_tests {
    use super::*;
    #[test]
    fn wire_mode_can_be_selected_from_toolbar_settings() {
        let ctx = egui::Context::default();
        let mut wire = 0;
        let mut time = 0.;
        let mut frame = |events: Vec<egui::Event>| {
            time += 1. / 60.;
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900., 600.),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
                        ui.menu_button("settings", |ui| wire_control(ui, &mut wire));
                    });
                },
            );
            out.shapes
                .into_iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(t) = shape.shape {
                        Some((t.galley.text().to_owned(), t.pos + t.galley.size() * 0.5))
                    } else {
                        None
                    }
                })
                .collect::<std::collections::HashMap<_, _>>()
        };
        let mut click = |position: egui::Pos2| {
            frame(vec![egui::Event::PointerMoved(position)]);
            frame(vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }]);
            frame(vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }])
        };
        // 初回のレイアウトを確定させてから、親メニューと子リストを操作する。
        let mut labels = click(egui::pos2(30., 12.));
        labels = click(*labels.get("settings").unwrap_or(&egui::pos2(30., 12.)));
        if !labels.contains_key("なし") {
            labels = click(*labels.get("settings").unwrap());
        }
        let labels = click(*labels.get("なし").expect("選択リスト"));
        let target = *labels
            .get("描画網のみ")
            .unwrap_or_else(|| panic!("描画網のみの選択肢: {:?}", labels.keys()));
        click(target);
        assert_eq!(wire, 2);
    }
}

#[cfg(test)]
mod remembered_scene_tests {
    use super::*;
    pub(super) fn app() -> Option<App> {
        app_with_backend(eframe::wgpu::Backends::all())
    }
    pub(super) fn app_with_backend(backends: eframe::wgpu::Backends) -> Option<App> {
        use eframe::wgpu;
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let ui = eframe::egui_wgpu::Renderer::new(&device, format, None, 1, false);
        let state = eframe::egui_wgpu::RenderState {
            adapter,
            available_adapters: Vec::new(),
            device,
            queue,
            target_format: format,
            renderer: Arc::new(egui::epaint::mutex::RwLock::new(ui)),
        };
        Some(App::new(
            &egui::Context::default(),
            state,
            PathBuf::new(),
            None,
            "national",
            None,
        ))
    }
    #[test]
    fn turning_back_restores_the_same_gpu_mesh_without_uploading() {
        let Some(mut app) = app() else {
            eprintln!("GPUがないためGPU試験を省略します");
            return;
        };
        let packet = Packet::constant_plane((13, 7220, 3200), 1500.).unwrap();
        let a = app.renderer.upload(&packet, 1).unwrap();
        let b = app.renderer.upload(&packet, 2).unwrap();
        let world = [
            packet.origin[0] + packet.side / 2.,
            packet.origin[1] + packet.side / 2.,
        ];
        app.camera
            .set_observer(world, 1502., 0., 0., 0.8, [1200., 800.]);
        app.shown = vec![a.clone()];
        app.remember_scene();
        app.camera
            .set_observer(world, 1502., std::f64::consts::PI, 0., 0.8, [1200., 800.]);
        app.shown = vec![b.clone()];
        app.remember_scene();
        app.camera
            .set_observer(world, 1502., std::f64::consts::TAU, 0., 0.8, [1200., 800.]);
        let sources = app.renderer.source_bytes();
        app.restore_remembered_scene();
        assert!(Arc::ptr_eq(&app.shown[0], &a));
        assert_eq!(app.renderer.source_bytes(), sources);
        assert_eq!(app.remembered_until_selection, Some(app.version + 1));
        assert!(!app.scene().unwrap());
        assert!(Arc::ptr_eq(&app.shown[0], &a));
        for i in 0..20 {
            let (yaw, expected) = if i % 2 == 0 {
                (std::f64::consts::PI, &b)
            } else {
                (std::f64::consts::TAU, &a)
            };
            app.camera
                .set_observer(world, 1502., yaw, 0., 0.8, [1200., 800.]);
            app.restore_remembered_scene();
            assert!(Arc::ptr_eq(&app.shown[0], expected));
            assert_eq!(app.renderer.source_bytes(), sources);
        }
        assert_eq!(app.remembered.len(), 2);
        let _ = app.renderer.state.device.poll(eframe::wgpu::Maintain::Wait);
    }
    #[test]
    fn preparation_plan_is_reused_until_the_camera_or_targets_change() {
        let Some(mut app) = app() else {
            return;
        };
        app.resolution = 0;
        app.wanted = vec![(13, 7220, 3200)];
        let first = app.preparation_plan();
        assert!(
            Arc::ptr_eq(&first, &app.preparation_plan()),
            "転送を小分けに進める間に同じ計画を作り直している"
        );
        app.camera.distance *= 2.;
        let moved = app.preparation_plan();
        assert!(!Arc::ptr_eq(&first, &moved));
        app.wanted.push((13, 7221, 3200));
        let changed = app.preparation_plan();
        assert!(!Arc::ptr_eq(&moved, &changed));
        app.camera.mesh_pixels *= 0.5;
        let refined = app.preparation_plan();
        assert!(!Arc::ptr_eq(&changed, &refined));
    }
    #[test]
    fn resident_evicted_after_request_is_requested_again_and_scene_recovers() -> Result<()> {
        let Some(mut app) = app() else {
            return Ok(());
        };
        let old = Packet::constant_plane((13, 7220, 3200), 1500.)?;
        app.shown = vec![app.renderer.upload(&old, 1)?];
        let key = (13, 7221, 3200);
        app.wanted = vec![key];
        app.loading = false;
        app.queued = false;
        app.ready_since = Some(Instant::now());
        app.resolution = 0;
        assert!(!app.scene()?);
        assert!(app.queued && app.ready_since.is_none());
        assert_eq!(app.shown[0].key, old.key, "復帰まで直前の地形を維持");
        app.loading = true;
        app.queued = false;
        assert!(!app.scene()?);
        assert!(!app.queued, "取得中の要求を再送して中断させない");
        app.packets
            .insert(key, Arc::new(Packet::constant_plane(key, 1500.)?));
        let _ = app.renderer.state.device.poll(eframe::wgpu::Maintain::Wait);
        assert!(app.scene()?);
        assert_eq!(app.shown[0].key, key, "不足分の到着後に新しい地形へ進む");
        Ok(())
    }
    #[test]
    fn reordered_cached_fields_do_not_restart_the_transition() -> Result<()> {
        let Some(mut app) = app() else {
            return Ok(());
        };
        let mut tiles = Vec::new();
        for key in [(13, 7220, 3200), (13, 7221, 3200)] {
            let packet = Arc::new(Packet::constant_plane(key, 1500.)?);
            let tile = app.renderer.upload(&packet, 32)?;
            app.packets.insert(key, packet);
            app.gpu.insert((key, 32), tile.clone());
            app.wanted.push(key);
            tiles.push(tile);
        }
        app.shown = tiles.iter().rev().cloned().collect();
        app.resolution = 0;
        app.transition = Instant::now() - Duration::from_secs(2);
        let before = app.transition;
        assert!(app.scene()?);
        assert!(app.previous.is_empty());
        assert_eq!(app.transition, before);
        assert!(Arc::ptr_eq(&app.shown[0], &tiles[1]));
        Ok(())
    }
    #[test]
    fn arrived_tiles_prepare_during_missing_data_and_active_transition() -> Result<()> {
        let Some(mut app) = app() else {
            return Ok(());
        };
        let missing = (13, 7220, 3200);
        let arrived = (13, 7221, 3200);
        let old = app
            .renderer
            .upload(&Packet::constant_plane((13, 7219, 3200), 1500.)?, 1)?;
        app.shown = vec![old.clone()];
        app.previous = vec![old.clone()];
        app.local_background = vec![old.clone()];
        app.transition = Instant::now();
        app.loading = true;
        app.resolution = 0;
        app.wanted = vec![missing, arrived];
        app.packets
            .insert(arrived, Arc::new(Packet::constant_plane(arrived, 1500.)?));
        assert!(!app.scene()?);
        assert!(
            app.gpu.contains_key(&(arrived, 32)),
            "未着の先頭タイルや遷移完了を待っている"
        );
        assert!(Arc::ptr_eq(&app.shown[0], &old));
        assert!(
            Arc::ptr_eq(&app.local_background[0], &old),
            "準備途中で背景だけが切り替わった"
        );
        Ok(())
    }
    #[test]
    fn cached_fields_choose_the_smallest_sufficient_grid() -> Result<()> {
        let Some(mut app) = app() else {
            return Ok(());
        };
        let key = (13, 7220, 3200);
        let field = shared_field::Field {
            depth: 1,
            x: 0,
            y: 0,
        };
        let packet = Packet::constant_plane(key, 1500.)?;
        let a = app.renderer.upload_field(&packet, 32, field)?;
        let b = app.renderer.upload_field(&packet, 64, field)?;
        app.local_gpu.insert((key, 32, field), a.clone());
        app.local_gpu.insert((key, 64, field), b.clone());
        assert!(Arc::ptr_eq(
            &app.cached_local(key, 16, field, 1000000).unwrap(),
            &a
        ));
        assert!(Arc::ptr_eq(
            &app.cached_local(key, 64, field, 1000000).unwrap(),
            &b
        ));
        Ok(())
    }
    #[test]
    fn memory_pressure_keeps_background_needed_by_the_incomplete_scene() -> Result<()> {
        let Some(mut app) = app() else {
            return Ok(());
        };
        let field = shared_field::Field {
            depth: 1,
            x: 0,
            y: 0,
        };
        let key = (13, 7220, 3200);
        let other = (13, 7221, 3200);
        for (k, n) in [(key, 32), (key, 16), (other, 16)] {
            let mut tile = app.renderer.upload(&Packet::constant_plane(k, 1500.)?, n)?;
            Arc::get_mut(&mut tile).unwrap().bytes = 200 * 1024 * 1024;
            app.gpu.insert((k, n), tile);
            app.mesh_used
                .insert((k, n, shared_field::Field::default()), Instant::now());
        }
        let _ = app.renderer.state.device.poll(eframe::wgpu::Maintain::Wait);
        app.trim_gpu_cache(&preparing_meshes([(key, field, 128)]));
        assert!(
            app.gpu.contains_key(&(key, 32)),
            "準備途中の背景が毎回消されるとGPU準備が完了しない"
        );
        assert!(app.gpu.len() < 3, "未使用のメッシュは整理する");
        Ok(())
    }

    #[test]
    fn memory_pressure_evicts_oldest_unused_mesh_only() {
        let Some(mut app) = app() else {
            eprintln!("GPUがないためGPU試験を省略します");
            return;
        };
        let now = Instant::now();
        for i in 0..3 {
            let key = (13, 7220 + i, 3200);
            let mut tile = app
                .renderer
                .upload(&Packet::constant_plane(key, 1500.).unwrap(), 1)
                .unwrap();
            Arc::get_mut(&mut tile).unwrap().bytes = 200 * 1024 * 1024;
            app.gpu.insert((key, 1), tile);
            app.mesh_used.insert(
                (key, 1, shared_field::Field::default()),
                now - Duration::from_secs(30 - i as u64 * 10),
            );
        }
        let _ = app.renderer.state.device.poll(eframe::wgpu::Maintain::Wait);
        app.trim_gpu_cache(&HashSet::new());
        assert!(!app.gpu.contains_key(&((13, 7220, 3200), 1)));
        assert!(app.gpu.contains_key(&((13, 7221, 3200), 1)));
        assert!(app.gpu.contains_key(&((13, 7222, 3200), 1)));
    }
}
