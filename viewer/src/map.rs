use crate::{lod_fade, tile_cache};
use anyhow::{Context, Result, ensure};
use eframe::egui::{self, ColorImage, Pos2, Rect};
use image::ImageDecoder;
use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use yamamuki_terrain_view::direct_material::Interpolation;

pub const MIN_ZOOM: f64 = 0.;
pub const MAX_ZOOM: f64 = 18.;
pub const TILE_SIZE: usize = 256;
const MAX_PENDING: usize = 6;
const MAX_CACHE: usize = 160;
const USER_AGENT: &str = concat!(
    "yamamuki-viewer/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/shohei0205/yamamuki-data)"
);

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Lod {
    pub max_level: Option<u32>,
    pub bias: f64,
}

impl Lod {
    pub const BIAS_RANGE: std::ops::RangeInclusive<f64> = -4. ..=2.;

    pub fn target_zoom(self, view_zoom: f64) -> f64 {
        view_zoom
            + self
                .bias
                .clamp(*Self::BIAS_RANGE.start(), *Self::BIAS_RANGE.end())
    }

    pub fn allows(self, z: u32) -> bool {
        self.max_level.is_none_or(|max| z <= max)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Source {
    #[default]
    Gsi,
    Pale,
    Blank,
    Photo,
    Relief,
    Hillshade,
    Slope,
    OpenStreetMap,
    OpenTopoMap,
    CyclOsm,
    Humanitarian,
}

impl Source {
    pub const GSI_LAYERS: [Self; 7] = [
        Self::Gsi,
        Self::Pale,
        Self::Blank,
        Self::Photo,
        Self::Relief,
        Self::Hillshade,
        Self::Slope,
    ];

    pub const OSM_LAYERS: [Self; 4] = [
        Self::OpenStreetMap,
        Self::OpenTopoMap,
        Self::CyclOsm,
        Self::Humanitarian,
    ];

    pub fn is_osm(self) -> bool {
        Self::OSM_LAYERS.contains(&self)
    }

    fn cache_directory(self) -> &'static str {
        match self {
            Self::Gsi => "gsi-std-tiles",
            Self::Pale => "gsi-pale-tiles",
            Self::Blank => "gsi-blank-tiles",
            Self::Photo => "gsi-photo-tiles",
            Self::Relief => "gsi-relief-tiles",
            Self::Hillshade => "gsi-hillshade-tiles",
            Self::Slope => "gsi-slope-tiles",
            Self::OpenStreetMap => "osm-tiles",
            Self::OpenTopoMap => "opentopomap-tiles",
            Self::CyclOsm => "cyclosm-tiles",
            Self::Humanitarian => "osm-humanitarian-tiles",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Gsi => "標準地図",
            Self::Pale => "淡色地図",
            Self::Blank => "白地図",
            Self::Photo => "写真",
            Self::Relief => "色別標高図",
            Self::Hillshade => "陰影起伏図",
            Self::Slope => "傾斜量図",
            Self::OpenStreetMap => "標準地図",
            Self::OpenTopoMap => "地形図（OpenTopoMap）",
            Self::CyclOsm => "自転車地図（CyclOSM）",
            Self::Humanitarian => "人道支援地図（HOT）",
        }
    }

    /// 提供元に存在しない粗いズームは要求しない。
    pub fn min_tile_zoom(self) -> u32 {
        match self {
            Self::Blank | Self::Relief => 5,
            Self::Slope => 3,
            Self::Gsi | Self::Pale | Self::Photo | Self::Hillshade => 2,
            _ => 0,
        }
    }
    pub fn max_tile_zoom(self) -> u32 {
        match self {
            Self::Blank => 14,
            Self::Relief | Self::Slope => 15,
            Self::Hillshade => 16,
            Self::OpenTopoMap => 17,
            _ => MAX_ZOOM as u32,
        }
    }

    pub fn tile_zoom(self, zoom: f64, lod: Lod) -> u32 {
        let target = lod.target_zoom(zoom).floor() as u32;
        target
            .min(self.max_tile_zoom())
            .min(lod.max_level.unwrap_or(u32::MAX))
            .max(self.min_tile_zoom())
    }

    fn tile_endpoint(self) -> (&'static str, &'static str) {
        let endpoint = match self {
            Self::Gsi => "https://cyberjapandata.gsi.go.jp/xyz/std",
            Self::Pale => "https://cyberjapandata.gsi.go.jp/xyz/pale",
            Self::Blank => "https://cyberjapandata.gsi.go.jp/xyz/blank",
            Self::Photo => "https://cyberjapandata.gsi.go.jp/xyz/seamlessphoto",
            Self::Relief => "https://cyberjapandata.gsi.go.jp/xyz/relief",
            Self::Hillshade => "https://cyberjapandata.gsi.go.jp/xyz/hillshademap",
            Self::Slope => "https://cyberjapandata.gsi.go.jp/xyz/slopemap",
            Self::OpenStreetMap => "https://tile.openstreetmap.org",
            Self::OpenTopoMap => "https://a.tile.opentopomap.org",
            Self::CyclOsm => "https://a.tile-cyclosm.openstreetmap.fr/cyclosm",
            Self::Humanitarian => "https://tile-a.openstreetmap.fr/hot",
        };
        (endpoint, if self == Self::Photo { "jpg" } else { "png" })
    }

    pub fn show_info(self, ui: &mut egui::Ui, tile_zoom: u32) {
        let (_, extension) = self.tile_endpoint();
        let format = if extension == "jpg" { "JPEG" } else { "PNG" };
        ui.set_max_width(360.);
        ui.strong(self.label());
        ui.label(format!(
            "取得範囲 z{}〜z{}",
            self.min_tile_zoom(),
            self.max_tile_zoom()
        ));
        ui.label(format!("画像形式 {format}"));
        ui.separator();
        self.attribution(ui, tile_zoom);
    }

    pub fn attribution(self, ui: &mut egui::Ui, tile_zoom: u32) {
        if self.is_osm() {
            ui.hyperlink_to(
                "© OpenStreetMap contributors",
                "https://www.openstreetmap.org/copyright",
            );
            match self {
                Self::OpenTopoMap => {
                    ui.hyperlink_to(
                        "標高：SRTM・Sonny ／ © OpenTopoMap（CC BY-SA）",
                        "https://opentopomap.org/about",
                    );
                }
                Self::CyclOsm | Self::Humanitarian => {
                    let (style, url) = match self {
                        Self::CyclOsm => ("CyclOSM", "https://www.cyclosm.org/"),
                        _ => ("Humanitarian OpenStreetMap Team", "https://www.hotosm.org/"),
                    };
                    ui.hyperlink_to(style, url);
                    ui.hyperlink_to(
                        "配信：OpenStreetMap France",
                        "https://www.openstreetmap.fr/usage/",
                    );
                }
                _ => {}
            }
            return;
        }
        ui.hyperlink_to(
            format!("出典：国土地理院（{}）", self.label()),
            "https://maps.gsi.go.jp/development/ichiran.html",
        );
        let detail = match (self, tile_zoom) {
            (Self::Pale, 0..=8) => {
                "Shoreline data: U.S. National Imagery and Mapping Agency, Vector Map Level 0 (VMAP0), USGS Information Services, 1997"
            }
            (Self::Photo, 0..=8) => {
                "世界衛星モザイク画像：NASA LP DAAC、USGS EROS Center, Sioux Falls, South Dakota（2015年公開）"
            }
            (Self::Photo, 9..=13) => {
                "Landsat8画像（GSI, TSIC, GEO Grid/AIST・U.S. Geological Survey）、海底地形（GEBCO）"
            }
            (Self::Photo, _) => "GRUS画像（© Axelspace）",
            (Self::Relief, _) => "海域部は海上保安庁海洋情報部の資料を使用して作成",
            _ => return,
        };
        ui.label(egui::RichText::new(detail).small());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tile {
    pub z: u32,
    pub x: u32,
    pub y: u32,
}

impl Tile {
    fn neighbor(self, dx: i32, dy: i32) -> Option<Self> {
        let side = 1i64 << self.z;
        let y = i64::from(self.y) + i64::from(dy);
        (0..side).contains(&y).then(|| Self {
            z: self.z,
            x: (i64::from(self.x) + i64::from(dx)).rem_euclid(side) as u32,
            y: y as u32,
        })
    }

    fn url(self, source: Source) -> String {
        let (base, extension) = source.tile_endpoint();
        format!("{base}/{}/{}/{}.{extension}", self.z, self.x, self.y)
    }

    fn cache_path(self, directory: &str) -> Option<PathBuf> {
        Some(
            eframe::storage_dir(crate::APP_ID)?
                .join(directory)
                .join(self.z.to_string())
                .join(self.x.to_string())
                .join(format!("{}.tile", self.y)),
        )
    }
}

pub fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .build()?)
}

type LoadedTile = (ColorImage, Option<Instant>);

fn fetch(
    client: &reqwest::blocking::Client,
    tile: Tile,
    source: Source,
) -> Result<Option<LoadedTile>> {
    let url = tile.url(source);
    let path = tile
        .cache_path(source.cache_directory())
        .context("地図のタイルキャッシュを利用できません")?;
    Ok(tile_cache::load(client, &url, &path, USER_AGENT, |bytes| {
        decode_tile(source, bytes)
    })?
    .map(|(image, ttl)| {
        (
            image,
            Instant::now().checked_add(ttl.max(Duration::from_secs(1))),
        )
    }))
}

fn decode_tile(source: Source, bytes: &[u8]) -> Result<ColorImage> {
    if source == Source::Photo {
        decode_jpeg(bytes)
    } else {
        decode_png(bytes)
    }
}

fn decode_jpeg(bytes: &[u8]) -> Result<ColorImage> {
    let decoder = image::codecs::jpeg::JpegDecoder::new(Cursor::new(bytes))?;
    ensure!(
        decoder.dimensions() == (TILE_SIZE as u32, TILE_SIZE as u32),
        "地図タイルの寸法が異なります"
    );
    let rgb = image::DynamicImage::from_decoder(decoder)?.into_rgb8();
    Ok(ColorImage::from_rgb([TILE_SIZE; 2], rgb.as_raw()))
}

fn decode_png(bytes: &[u8]) -> Result<ColorImage> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    ensure!(
        info.width as usize == TILE_SIZE && info.height as usize == TILE_SIZE,
        "地図タイルの寸法が異なります"
    );
    let mut pixels = vec![
        0;
        reader
            .output_buffer_size()
            .context("地図タイルの容量が不正です")?
    ];
    let frame = reader.next_frame(&mut pixels)?;
    let size = [TILE_SIZE; 2];
    let image = match frame.color_type {
        png::ColorType::Rgb => ColorImage::from_rgb(size, &pixels),
        png::ColorType::Rgba => ColorImage::from_rgba_unmultiplied(size, &pixels),
        png::ColorType::Grayscale => ColorImage::from_gray(size, &pixels),
        png::ColorType::GrayscaleAlpha => ColorImage::from_rgba_unmultiplied(
            size,
            &pixels
                .chunks_exact(2)
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect::<Vec<_>>(),
        ),
        png::ColorType::Indexed => anyhow::bail!("地図タイルの色形式に対応していません"),
    };
    Ok(image)
}

pub fn check() -> Result<()> {
    let tile = Tile {
        z: 8,
        x: 226,
        y: 101,
    };
    let (image, _) =
        fetch(&client()?, tile, Source::Gsi)?.context("確認用の地理院タイルが見つかりません")?;
    println!(
        "地理院タイル {}×{} を取得しました",
        image.size[0], image.size[1]
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Orientation {
    rotation: egui::emath::Rot2,
    vertical_scale: f32,
}

impl Default for Orientation {
    fn default() -> Self {
        Self {
            rotation: egui::emath::Rot2::IDENTITY,
            vertical_scale: 1.,
        }
    }
}

impl Orientation {
    pub fn orbit(&mut self, delta: egui::Vec2) {
        const DRAG_SPEED: f32 = 0.006;
        const MAX_TILT: f32 = 80_f32.to_radians();
        self.rotation = egui::emath::Rot2::from_angle(self.rotation.angle() - delta.x * DRAG_SPEED);
        let tilt = (self.vertical_scale.acos() - delta.y * DRAG_SPEED).clamp(0., MAX_TILT);
        self.vertical_scale = tilt.cos();
    }

    fn project(self, offset: egui::Vec2) -> egui::Vec2 {
        let rotated = self.rotation * offset;
        egui::vec2(rotated.x, rotated.y * self.vertical_scale)
    }

    fn unproject(self, offset: egui::Vec2) -> egui::Vec2 {
        self.rotation.inverse() * egui::vec2(offset.x, offset.y / self.vertical_scale)
    }

    pub fn angles(self) -> (f64, f64) {
        (
            self.rotation.angle() as f64,
            self.vertical_scale.acos() as f64,
        )
    }

    pub fn north(self) -> egui::Vec2 {
        self.project(-egui::Vec2::Y).normalized()
    }
}

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct View {
    pub center: [f64; 2],
    pub zoom: f64,
    pub orientation: Orientation,
}

impl View {
    pub fn scale(&self) -> f64 {
        TILE_SIZE as f64 * self.zoom.exp2()
    }

    pub fn world_at(&self, rect: Rect, pos: Pos2) -> [f64; 2] {
        let d = self.orientation.unproject(pos - rect.center());
        [
            self.center[0] + f64::from(d.x) / self.scale(),
            self.center[1] + f64::from(d.y) / self.scale(),
        ]
    }

    pub fn screen(&self, rect: Rect, [x, y]: [f64; 2]) -> Pos2 {
        rect.center()
            + self.orientation.project(egui::vec2(
                ((x - self.center[0]) * self.scale()) as f32,
                ((y - self.center[1]) * self.scale()) as f32,
            ))
    }

    pub fn area(&self, rect: Rect) -> [[f64; 2]; 2] {
        let corners = [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ];
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for corner in corners {
            let world = self.world_at(rect, corner);
            for axis in 0..2 {
                min[axis] = min[axis].min(world[axis]);
                max[axis] = max[axis].max(world[axis]);
            }
        }
        [min, max]
    }

    pub fn pan(&mut self, delta: egui::Vec2) {
        let delta = self.orientation.unproject(delta);
        for (axis, delta) in [delta.x, delta.y].into_iter().enumerate() {
            self.center[axis] = (self.center[axis] - f64::from(delta) / self.scale()).clamp(0., 1.);
        }
    }

    pub fn zoom_at(&mut self, rect: Rect, pointer: Pos2, delta: f64) {
        self.zoom_at_limit(rect, pointer, delta, MAX_ZOOM);
    }
    /// 3Dはオンライン地図の取得上限でカメラの接近を止めない。
    pub fn zoom_at_terrain(&mut self, rect: Rect, pointer: Pos2, delta: f64) {
        self.zoom_at_limit(rect, pointer, delta, f64::INFINITY);
    }
    fn zoom_at_limit(&mut self, rect: Rect, pointer: Pos2, delta: f64, maximum: f64) {
        let anchor = self.world_at(rect, pointer);
        self.zoom = (self.zoom + delta).clamp(MIN_ZOOM, maximum);
        let after = self.world_at(rect, pointer);
        for axis in 0..2 {
            self.center[axis] = (self.center[axis] + anchor[axis] - after[axis]).clamp(0., 1.);
        }
    }

    pub fn fit(&mut self, rect: Rect, [min, max]: [[f64; 2]; 2]) {
        self.center = std::array::from_fn(|i| (min[i] + max[i]) * 0.5);
        let dx = (max[0] - min[0]).max(1e-8);
        let dy = (max[1] - min[1]).max(1e-8);
        let width = self.orientation.project(egui::vec2(dx as f32, 0.)).abs();
        let height = self.orientation.project(egui::vec2(0., dy as f32)).abs();
        let size = width + height;
        let scale = f64::from((rect.width() / size.x).min(rect.height() / size.y)) * 0.8;
        self.zoom = (scale / TILE_SIZE as f64).log2().clamp(MIN_ZOOM, 14.);
    }

    pub fn tiles_at(&self, rect: Rect, z: u32) -> Vec<Tile> {
        let side = 1u32 << z;
        let index = |x: f64| (x * side as f64).floor().clamp(0., (side - 1) as f64) as u32;
        let [min, max] = self.area(rect);
        let mut tiles = Vec::new();
        for y in index(min[1])..=index(max[1]) {
            for x in index(min[0])..=index(max[0]) {
                let tile = Tile { z, x, y };
                if self.orientation == Orientation::default()
                    || Rect::from_points(&self.tile_corners(rect, tile)).intersects(rect)
                {
                    tiles.push(tile);
                }
            }
        }
        tiles.sort_by(|a, b| {
            let d = |tile: &Tile| {
                ((tile.x as f64 + 0.5) / side as f64 - self.center[0]).powi(2)
                    + ((tile.y as f64 + 0.5) / side as f64 - self.center[1]).powi(2)
            };
            d(a).total_cmp(&d(b))
        });
        tiles
    }

    pub fn tile_corners(&self, rect: Rect, tile: Tile) -> [Pos2; 4] {
        let side = (1u32 << tile.z) as f64;
        let [left, right] = [tile.x, tile.x + 1].map(|x| f64::from(x) / side);
        let [top, bottom] = [tile.y, tile.y + 1].map(|y| f64::from(y) / side);
        [[left, top], [right, top], [right, bottom], [left, bottom]]
            .map(|world| self.screen(rect, world))
    }

    pub fn paint_tile_uv(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        tile: Tile,
        texture: egui::TextureId,
        uv: Rect,
    ) {
        let corners = self.tile_corners(rect, tile);
        if self.orientation == Orientation::default() {
            painter.image(
                texture,
                Rect::from_min_max(corners[0], corners[2]),
                uv,
                egui::Color32::WHITE,
            );
        } else {
            let mut mesh = egui::Mesh::with_texture(texture);
            for (pos, uv) in corners.into_iter().zip([
                uv.left_top(),
                uv.right_top(),
                uv.right_bottom(),
                uv.left_bottom(),
            ]) {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos,
                    uv,
                    color: egui::Color32::WHITE,
                });
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}

enum Cached {
    Texture(crate::tile_atlas::Image, Option<Instant>),
    Missing,
    Failed(Instant),
}

type Reply = (Tile, Result<Option<LoadedTile>>);

#[derive(Default)]
struct DrawHistory {
    previous: HashMap<Tile, (Tile, u64)>,
    fading: HashMap<Tile, (Tile, u64)>,
    order: u64,
}
impl DrawHistory {
    fn record(&mut self, region: Tile, source: Tile) {
        self.order += 1;
        self.previous.insert(region, (source, self.order));
    }
}
// 最も細かい下地ではなく、最後に上へ描いた画像をフェード元に使う。
fn displayed_sample(history: &HashMap<Tile, (Tile, u64)>, tile: Tile) -> Option<(Tile, Rect)> {
    (0..=tile.z)
        .filter_map(|z| {
            let region = lod_fade::sample(tile, z, lod_fade::FULL_UV).0;
            history
                .get(&region)
                .filter(|(source, _)| source.z <= tile.z)
        })
        .max_by_key(|(_, order)| *order)
        .map(|(source, _)| lod_fade::sample(tile, source.z, lod_fade::FULL_UV))
}
pub struct Tiles {
    pub edge_filter: Interpolation,
    pub borders: bool,
    pub source: Source,
    cancelled: Arc<AtomicBool>,
    queue: mpsc::Sender<Tile>,
    replies: mpsc::Receiver<Reply>,
    pending: HashSet<Tile>,
    cache: HashMap<Tile, (Cached, u64)>,
    frame: u64,
    lod: lod_fade::State<u32>,
    material_lods: Vec<lod_fade::State<u32>>,
    material_pass: usize,
    draw_history: HashMap<usize, DrawHistory>,
}

impl Tiles {
    pub fn new(ctx: &egui::Context, client: reqwest::blocking::Client, source: Source) -> Self {
        let (queue, jobs) = mpsc::channel();
        let (send, replies) = mpsc::channel();
        let jobs = Arc::new(Mutex::new(jobs));
        let cancelled = Arc::new(AtomicBool::new(false));
        for _ in 0..4 {
            let (jobs, send, ctx, client) =
                (jobs.clone(), send.clone(), ctx.clone(), client.clone());
            let cancelled = Arc::clone(&cancelled);
            std::thread::spawn(move || {
                loop {
                    let job = jobs.lock().expect("tile queue").recv();
                    let Ok(tile) = job else { break };
                    if cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                    if send.send((tile, fetch(&client, tile, source))).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        }
        Self {
            edge_filter: Interpolation::Bilinear,
            borders: false,
            source,
            cancelled,
            queue,
            replies,
            pending: HashSet::new(),
            cache: HashMap::new(),
            frame: 0,
            lod: lod_fade::State::default(),
            material_lods: (0..4).map(|_| Default::default()).collect(),
            material_pass: 0,
            draw_history: HashMap::new(),
        }
    }

    // LODの全域切り替え待ちでも、各範囲で最も細かい読込済み祖先を使う。
    fn cached_ancestor(&self, tile: Tile) -> Option<(Tile, Rect)> {
        self.cached_ancestor_up_to(tile, tile.z)
    }
    fn cached_ancestor_up_to(&self, tile: Tile, maximum: u32) -> Option<(Tile, Rect)> {
        (0..=maximum.min(tile.z)).rev().find_map(|z| {
            let (source, uv) = lod_fade::sample(tile, z, lod_fade::FULL_UV);
            matches!(self.cache.get(&source), Some((Cached::Texture(_, _), _)))
                .then_some((source, uv))
        })
    }

    fn stitch_neighbors(&self, tile: Tile) {
        let Some((Cached::Texture(image, _), _)) = self.cache.get(&tile) else {
            return;
        };
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                if let Some(other) = tile.neighbor(dx, dy)
                    && let Some((Cached::Texture(neighbor, _), _)) = self.cache.get(&other)
                {
                    image.set_neighbor(neighbor, dx, dy);
                    neighbor.set_neighbor(image, -dx, -dy);
                }
            }
        }
    }

    pub fn finish_material_frame(&mut self) {
        trim_cache(&mut self.cache, MAX_CACHE, self.frame);
    }
    pub fn select_material_pass(&mut self, pass: usize) {
        if self.material_lods.len() <= pass {
            self.material_lods.resize_with(pass + 1, Default::default);
        }

        if self.material_pass != pass {
            std::mem::swap(&mut self.lod, &mut self.material_lods[self.material_pass]);
            std::mem::swap(&mut self.lod, &mut self.material_lods[pass]);
            self.material_pass = pass;
        }
    }
    pub fn paint(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        view: &View,
        dragging: bool,
        tile_zoom: u32,
        lod_zoom: f64,
        fade_backend: lod_fade::Backend<'_>,
    ) -> bool {
        self.frame = ui.ctx().cumulative_pass_nr();
        let limit = if dragging { 1 } else { MAX_PENDING };
        let mut processed = 0;
        // 取得とPNGの展開はワーカーで行う。ドラッグ中はGPUへ渡すタイル数を抑える。
        for (tile, result) in self.replies.try_iter().take(limit) {
            processed += 1;
            self.pending.remove(&tile);
            let entry = match result {
                Ok(Some((image, expiry))) => {
                    Cached::Texture(crate::tile_atlas::Image::load_map(ui.ctx(), image), expiry)
                }
                Ok(None) => Cached::Missing,
                Err(_) => Cached::Failed(Instant::now()),
            };
            self.cache.insert(tile, (entry, self.frame));
            self.stitch_neighbors(tile);
        }
        if processed == limit && !self.pending.is_empty() {
            ui.ctx().request_repaint();
        }
        let tiles = view.tiles_at(rect, tile_zoom);
        // 傾斜で可視範囲が広がっても、表示中のタイルはキャッシュに収める。
        let requested = tiles_with_halo(tiles.clone());
        let cache_limit = MAX_CACHE.max(requested.len() + MAX_PENDING);
        let mut failed = false;
        for &tile in &requested {
            if let Some((entry, touched)) = self.cache.get_mut(&tile) {
                *touched = self.frame;
                match entry {
                    Cached::Texture(_, expiry)
                        if expiry.is_none_or(|until| Instant::now() < until) =>
                    {
                        continue;
                    }
                    Cached::Texture(_, _) => {}
                    Cached::Missing => continue,
                    Cached::Failed(at) => {
                        failed = true;
                        let retry = Duration::from_secs(15).saturating_sub(at.elapsed());
                        if !retry.is_zero() {
                            ui.ctx().request_repaint_after(retry);
                            continue;
                        }
                    }
                }
            }
            if self.pending.len() < MAX_PENDING
                && !self.pending.contains(&tile)
                && self.queue.send(tile).is_ok()
            {
                self.pending.insert(tile);
            }
        }
        let now = ui.input(|input| input.time);
        let ready = tiles.iter().all(|tile| self.cache.contains_key(tile));
        if ready && processed != 0 {
            ui.ctx().request_repaint();
        }
        let fade = fade_backend.is_some()
            && self
                .lod
                .current
                .is_some_and(|old| lod_fade::can_blend(old, tile_zoom, lod_zoom));
        let previous_sources = std::mem::take(
            &mut self
                .draw_history
                .entry(self.material_pass)
                .or_default()
                .previous,
        );
        self.draw_history
            .entry(self.material_pass)
            .or_default()
            .order = 0;
        let previous_level = self.lod.current;
        self.lod
            .update(tile_zoom, ready && processed == 0, fade, now);
        if previous_level != self.lod.current {
            self.draw_history
                .entry(self.material_pass)
                .or_default()
                .fading = previous_sources;
        }
        let current = self.lod.current.unwrap_or(tile_zoom);
        if let (Some((state, renderer)), Some((&old, amount))) =
            (fade_backend, self.lod.blend(ui.ctx(), now))
            && lod_fade::can_blend(old, current, lod_zoom)
            && current == tile_zoom
        {
            let textures = state.renderer.read();
            let old_sources = self
                .draw_history
                .entry(self.material_pass)
                .or_default()
                .fading
                .clone();
            let quads = view
                .tiles_at(rect, old.max(current))
                .into_iter()
                .filter_map(|tile| {
                    let [mut old_sample, new_sample] =
                        [old, current].map(|z| lod_fade::sample(tile, z, lod_fade::FULL_UV));
                    // 中間レベルを表示済みなら、フェード開始時にも粗い旧状態へ戻さない。
                    if old < current
                        && let Some(cached) = self.cached_ancestor_up_to(tile, current - 1)
                    {
                        old_sample = cached;
                    }
                    if let Some(displayed) = displayed_sample(&old_sources, tile) {
                        old_sample = displayed;
                    }
                    let (old_tile, old_uv) = old_sample;
                    let (new_tile, new_uv) = new_sample;
                    let [(old, old_ready, old_uv), (new, new_ready, new_uv)] =
                        [(old_tile, old_uv), (new_tile, new_uv)].map(|(tile, uv)| {
                            let Some((cached, touched)) = self.cache.get_mut(&tile) else {
                                return (None, false, uv);
                            };
                            *touched = self.frame;
                            match cached {
                                Cached::Texture(handle, _) => {
                                    let texture =
                                        renderer.color_view(&textures, handle.id(), self.frame);
                                    let ready = texture.is_some();
                                    (texture, ready, handle.uv(uv))
                                }
                                Cached::Missing => (None, true, uv),
                                Cached::Failed(_) => (None, false, uv),
                            }
                        });
                    let displayed = if new_ready && (amount >= 0.5 || !old_ready) {
                        new_tile
                    } else {
                        old_tile
                    };
                    self.draw_history
                        .entry(self.material_pass)
                        .or_default()
                        .record(tile, displayed);
                    lod_fade::Quad::new(tile, [old_uv, new_uv], [old, new], [old_ready, new_ready])
                        .map(|mut quad| {
                            quad.edge_filter = self.edge_filter;
                            quad
                        })
                })
                .collect();
            renderer.paint(
                painter,
                rect,
                view,
                quads,
                Vec::new(),
                amount,
                self.material_pass != 0,
            );
        } else {
            let mut filtered = Vec::new();
            let mut paint = |tile: Tile, texture: &crate::tile_atlas::Image, uv: Rect| {
                if self.edge_filter != Interpolation::Bilinear
                    && let Some((state, renderer)) = fade_backend
                {
                    let textures = state.renderer.read();
                    if let Some(view) = renderer.color_view(&textures, texture.id(), self.frame) {
                        let mut quad = lod_fade::Quad::new(
                            tile,
                            [texture.uv(uv); 2],
                            [Some(view.clone()), Some(view)],
                            [true; 2],
                        )
                        .unwrap();
                        quad.edge_filter = self.edge_filter;
                        filtered.push(quad);
                    } else {
                        ui.ctx().request_repaint();
                    }
                } else {
                    view.paint_tile_uv(painter, rect, tile, texture.id(), texture.uv(uv));
                }
            };
            // ズームアウト中は以前の細タイルも下地として残す。
            if current > tile_zoom {
                for (&tile, (entry, touched)) in &mut self.cache {
                    if tile.z == current
                        && let Cached::Texture(texture, _) = entry
                        && Rect::from_points(&view.tile_corners(rect, tile)).intersects(rect)
                    {
                        *touched = self.frame;
                        self.draw_history
                            .entry(self.material_pass)
                            .or_default()
                            .record(tile, tile);
                        paint(tile, texture, lod_fade::FULL_UV);
                    }
                }
            }
            for tile in tiles {
                if let Some((source, uv)) = self.cached_ancestor(tile)
                    && let Some((Cached::Texture(texture, _), touched)) =
                        self.cache.get_mut(&source)
                {
                    *touched = self.frame;
                    self.draw_history
                        .entry(self.material_pass)
                        .or_default()
                        .record(tile, source);
                    paint(tile, texture, uv);
                }
            }
            if !filtered.is_empty()
                && let Some((_, renderer)) = fade_backend
            {
                renderer.paint(
                    painter,
                    rect,
                    view,
                    filtered,
                    Vec::new(),
                    1.,
                    self.material_pass != 0,
                );
            }
        }
        if self.borders {
            for tile in view.tiles_at(rect, current) {
                if matches!(self.cache.get(&tile), Some((Cached::Texture(_, _), _))) {
                    crate::terrain_material::tile_border(
                        painter,
                        rect,
                        view,
                        tile,
                        self.material_pass != 0,
                        egui::Color32::from_rgb(240, 95, 35),
                    );
                }
            }
        }
        if self.material_pass == 0 {
            trim_cache(&mut self.cache, cache_limit, self.frame);
        }
        failed
    }
}

fn tiles_with_halo(tiles: Vec<Tile>) -> Vec<Tile> {
    let mut requested = tiles.clone();
    let mut included: HashSet<_> = tiles.iter().copied().collect();
    // 表示端も補間できるよう、同ズームの一周分だけ余分に取得する。
    for tile in tiles {
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(neighbor) = tile.neighbor(dx, dy)
                    && included.insert(neighbor)
                {
                    requested.push(neighbor);
                }
            }
        }
    }
    requested
}

fn trim_cache(cache: &mut HashMap<Tile, (Cached, u64)>, limit: usize, frame: u64) {
    // 切替前のタイルも表示中なら一時的な超過を許す。削除候補なしで巡回し続けない。
    let excess = cache.len().saturating_sub(limit);
    let mut candidates = cache
        .iter()
        .filter(|(_, (_, touched))| *touched != frame)
        .map(|(&key, (_, touched))| (key, *touched))
        .collect::<Vec<_>>();
    candidates.sort_unstable_by_key(|(_, touched)| *touched);
    for (key, _) in candidates.into_iter().take(excess) {
        cache.remove(&key);
    }
}

impl Drop for Tiles {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distant_materials_and_low_lod_limits_use_the_first_available_source_zoom() {
        for (source, minimum) in [
            (Source::Relief, 5),
            (Source::Blank, 5),
            (Source::Slope, 3),
            (Source::Hillshade, 2),
            (Source::Gsi, 2),
            (Source::Pale, 2),
            (Source::Photo, 2),
            (Source::OpenStreetMap, 0),
        ] {
            assert_eq!(source.tile_zoom(1., Lod::default()), minimum.max(1));
            assert_eq!(
                source.tile_zoom(
                    6.37,
                    Lod {
                        max_level: Some(1),
                        bias: -4.
                    }
                ),
                minimum.max(1)
            );
        }
        assert_eq!(Source::Relief.tile_zoom(4.9, Lod::default()), 5);
        assert_eq!(
            Source::Relief.tile_zoom(
                6.37,
                Lod {
                    max_level: None,
                    bias: -0.5
                }
            ),
            5
        );
    }
    #[test]
    fn material_ranges_keep_each_others_tiles_across_zoom_frames() {
        let ctx = egui::Context::default();
        let (queue, _jobs) = mpsc::channel();
        let (_send, replies) = mpsc::channel();
        let mut tiles = Tiles {
            edge_filter: Interpolation::Bilinear,
            borders: false,
            source: Source::Gsi,
            cancelled: Arc::default(),
            queue,
            replies,
            pending: HashSet::new(),
            cache: HashMap::new(),
            frame: 0,
            lod: Default::default(),
            material_lods: (0..4).map(|_| Default::default()).collect(),
            material_pass: 0,
            draw_history: HashMap::new(),
        };
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(2048., 2048.));
        let views = [0.25, 0.5, 0.75].map(|x| View {
            center: [x, 0.5],
            zoom: 10.,
            orientation: Default::default(),
        });
        let required = views
            .iter()
            .flat_map(|view| tiles_with_halo(view.tiles_at(rect, 10)))
            .collect::<HashSet<_>>();
        assert!(required.len() > MAX_CACHE);
        for &tile in &required {
            tiles.cache.insert(tile, (Cached::Missing, 0));
        }
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for (i, view) in views.iter().enumerate() {
                        tiles.select_material_pass(i + 1);
                        tiles.paint(ui, ui.painter(), rect, view, false, 10, 10., None);
                        assert!(
                            view.tiles_at(rect, 10)
                                .iter()
                                .all(|tile| tiles.cache.contains_key(tile))
                        );
                    }
                    tiles.finish_material_frame();
                });
            });
            assert!(required.iter().all(|key| tiles.cache.contains_key(key)));
            assert!(
                tiles.pending.is_empty(),
                "別範囲のタイルを削除して再取得している"
            );
        }
    }

    #[test]
    fn zoom_out_preserves_visible_fallbacks_without_spinning_at_cache_limit() {
        let frame = 42;
        let mut cache = (0..MAX_CACHE as u32 + 20)
            .map(|x| (Tile { z: 12, x, y: 0 }, (Cached::Missing, frame)))
            .collect::<HashMap<_, _>>();
        // 拡大前後の表示中タイルだけで上限超過しても終了し、表示を保持する。
        trim_cache(&mut cache, MAX_CACHE, frame);
        assert_eq!(cache.len(), MAX_CACHE + 20);
        for (_, touched) in cache.values_mut() {
            *touched = frame - 1;
        }
        let active = Tile { z: 12, x: 0, y: 0 };
        cache.get_mut(&active).unwrap().1 = frame;
        trim_cache(&mut cache, MAX_CACHE, frame);
        assert_eq!(cache.len(), MAX_CACHE);
        assert!(cache.contains_key(&active));
    }

    #[test]
    fn overzoom_uses_the_highest_available_tile_without_changing_the_view() {
        let view = View {
            center: crate::points::project(35.36, 138.73),
            zoom: 18.,
            orientation: Orientation::default(),
        };
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
        for (source, max) in [
            (Source::Gsi, 18),
            (Source::Pale, 18),
            (Source::Blank, 14),
            (Source::Photo, 18),
            (Source::Relief, 15),
            (Source::Hillshade, 16),
            (Source::Slope, 15),
            (Source::OpenStreetMap, 18),
            (Source::OpenTopoMap, 17),
            (Source::CyclOsm, 18),
            (Source::Humanitarian, 18),
        ] {
            let tiles = view.tiles_at(rect, source.tile_zoom(view.zoom, Lod::default()));
            assert!(!tiles.is_empty());
            assert!(tiles.iter().all(|tile| tile.z == max));
            assert!(Rect::from_points(&view.tile_corners(rect, tiles[0])).contains(rect.center()));
        }
    }

    #[test]
    fn terrain_zoom_is_not_limited_by_online_map_zoom() {
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 600.));
        let mut view = View {
            center: [0.5; 2],
            zoom: 18.,
            orientation: Orientation::default(),
        };
        view.zoom_at_terrain(rect, rect.center(), 6.);
        assert_eq!(view.zoom, 24.);
        assert_eq!(view.center, [0.5; 2]);
        assert_eq!(Source::Gsi.tile_zoom(view.zoom, Lod::default()), 18);
    }
    #[test]
    fn terrain_distances_keep_independent_lod_transitions() {
        let mut tiles = Tiles::new(
            &egui::Context::default(),
            reqwest::blocking::Client::new(),
            Source::Gsi,
        );
        tiles.lod.current = Some(10);
        tiles.select_material_pass(1);
        assert_eq!(tiles.lod.current, None);
        tiles.lod.current = Some(15);
        tiles.select_material_pass(2);
        tiles.lod.current = Some(12);
        tiles.select_material_pass(3);
        tiles.lod.current = Some(6);
        for (pass, z) in [(1, 15), (2, 12), (3, 6), (0, 10)] {
            tiles.select_material_pass(pass);
            assert_eq!(tiles.lod.current, Some(z));
        }
    }
    #[test]
    fn online_lod_respects_bias_and_both_zoom_limits() {
        let mut lod = Lod {
            max_level: None,
            bias: 0.75,
        };
        assert_eq!(Source::Gsi.tile_zoom(9.24, lod), 9);
        assert_eq!(Source::Gsi.tile_zoom(9.25, lod), 10);
        lod.max_level = Some(9);
        assert_eq!(Source::Gsi.tile_zoom(15., lod), 9);
        lod.max_level = Some(17);
        assert_eq!(Source::Blank.tile_zoom(16., lod), 14);
        lod.bias = -4.;
        assert_eq!(Source::Gsi.tile_zoom(2., lod), 2);
    }

    #[test]
    #[ignore = "requires one public tile from each GSI layer"]
    fn reads_public_gsi_layers() {
        let client = client().unwrap();
        for source in Source::GSI_LAYERS {
            let tile = Tile {
                z: 8,
                x: 226,
                y: 101,
            };
            let (image, _) = fetch(&client, tile, source)
                .unwrap_or_else(|error| panic!("{source:?}: {error:#}"))
                .unwrap_or_else(|| panic!("{source:?}"));
            assert_eq!(image.size, [TILE_SIZE; 2], "{source:?}");
        }
    }

    #[test]
    #[ignore = "requires one public tile from each OSM layer"]
    fn reads_public_osm_layers() {
        let tile = Tile {
            z: 8,
            x: 226,
            y: 101,
        };
        let cache = tempfile::tempdir().unwrap();
        let client = client().unwrap();
        for source in Source::OSM_LAYERS {
            let path = cache
                .path()
                .join(source.cache_directory())
                .with_extension("tile");
            let (image, _) =
                tile_cache::load(&client, &tile.url(source), &path, USER_AGENT, decode_png)
                    .unwrap_or_else(|error| panic!("{source:?}: {error:#}"))
                    .unwrap_or_else(|| panic!("{source:?}"));
            assert_eq!(image.size, [TILE_SIZE; 2], "{source:?}");
        }
    }

    #[test]
    fn all_online_layers_have_separate_disk_caches() {
        let sources = Source::GSI_LAYERS
            .into_iter()
            .chain(Source::OSM_LAYERS)
            .collect::<Vec<_>>();
        let directories = sources
            .iter()
            .map(|source| source.cache_directory())
            .collect::<HashSet<_>>();
        assert_eq!(directories.len(), sources.len());
        assert_eq!(Source::Gsi.cache_directory(), "gsi-std-tiles");
    }
    #[test]
    fn gsi_png_and_photo_are_reused_from_disk_without_the_server() {
        use std::{
            io::{BufRead, BufReader, Write},
            net::TcpListener,
        };
        for source in [Source::Gsi, Source::Photo] {
            let rgb = vec![128u8; TILE_SIZE * TILE_SIZE * 3];
            let mut body = Vec::new();
            if source == Source::Photo {
                image::codecs::jpeg::JpegEncoder::new(&mut body)
                    .encode(
                        &rgb,
                        TILE_SIZE as u32,
                        TILE_SIZE as u32,
                        image::ExtendedColorType::Rgb8,
                    )
                    .unwrap();
            } else {
                let mut encoder = png::Encoder::new(&mut body, TILE_SIZE as u32, TILE_SIZE as u32);
                encoder.set_color(png::ColorType::Rgb);
                let mut writer = encoder.write_header().unwrap();
                writer.write_image_data(&rgb).unwrap();
            }
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/tile", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                {
                    let mut reader = BufReader::new(&socket);
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                            break;
                        }
                    }
                }
                write!(socket,"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\nCache-Control: max-age=86400\r\n\r\n",body.len()).unwrap();
                socket.write_all(&body).unwrap();
            });
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(source.cache_directory()).join("tile");
            let client = reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();
            let load = || {
                tile_cache::load(&client, &url, &path, USER_AGENT, |bytes| {
                    decode_tile(source, bytes)
                })
                .unwrap()
                .unwrap()
            };
            let first = load().0;
            server.join().unwrap();
            let second = load().0;
            assert_eq!(first.size, [TILE_SIZE; 2]);
            assert_eq!(first.pixels, second.pixels);
        }
    }

    #[test]
    fn osm_layers_have_separate_urls_and_caches() {
        let tile = Tile {
            z: 8,
            x: 226,
            y: 101,
        };
        let urls: HashSet<_> = Source::OSM_LAYERS
            .iter()
            .map(|&source| tile.url(source))
            .collect();
        let directories: HashSet<_> = Source::OSM_LAYERS
            .iter()
            .map(|&source| source.cache_directory())
            .collect();
        assert_eq!(urls.len(), Source::OSM_LAYERS.len());
        assert_eq!(directories.len(), Source::OSM_LAYERS.len());
        assert_eq!(Source::OpenStreetMap.cache_directory(), "osm-tiles");
        assert!(Source::GSI_LAYERS.iter().all(|source| !source.is_osm()));
    }

    #[test]
    fn drag_spreads_ready_tile_uploads_over_frames() {
        let ctx = egui::Context::default();
        let (queue, _jobs) = mpsc::channel();
        let (send, replies) = mpsc::channel();
        let pending: HashSet<_> = (0..MAX_PENDING as u32)
            .map(|x| Tile { z: 5, x, y: 0 })
            .collect();
        for &tile in &pending {
            send.send((
                tile,
                Ok(Some((
                    ColorImage::new([TILE_SIZE; 2], egui::Color32::WHITE),
                    None,
                ))),
            ))
            .unwrap();
        }
        let mut tiles = Tiles {
            edge_filter: Interpolation::Bilinear,
            borders: false,
            source: Source::Gsi,
            cancelled: Arc::default(),
            queue,
            replies,
            pending,
            cache: HashMap::new(),
            frame: 0,
            lod: lod_fade::State::default(),
            material_lods: (0..4).map(|_| Default::default()).collect(),
            material_pass: 0,
            draw_history: HashMap::new(),
        };
        let view = View {
            center: [0.5; 2],
            zoom: MIN_ZOOM,
            orientation: Orientation::default(),
        };
        let mut frame = |dragging| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    tiles.paint(
                        ui,
                        ui.painter(),
                        ui.available_rect_before_wrap(),
                        &view,
                        dragging,
                        Source::Gsi.tile_zoom(view.zoom, Lod::default()),
                        view.zoom,
                        None,
                    );
                });
            });
            tiles.cache.len()
        };

        assert_eq!(frame(true), 1);
        assert_eq!(frame(true), 2);
        assert_eq!(frame(false), MAX_PENDING);
    }

    #[test]
    fn tilted_view_keeps_the_zoom_and_drag_anchors() {
        let mut view = View {
            center: [0.5; 2],
            zoom: 8.,
            orientation: Orientation::default(),
        };
        view.orientation.orbit(egui::vec2(100., -140.));
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
        let pointer = egui::pos2(621., 405.);
        let world = view.world_at(rect, pointer);
        assert!(view.screen(rect, world).distance(pointer) < 0.001);
        view.zoom_at(rect, pointer, 2.);
        assert!(view.screen(rect, world).distance(pointer) < 0.001);
        let delta = egui::vec2(31., -70.);
        view.pan(delta);
        assert!(view.screen(rect, world).distance(pointer + delta) < 0.001);

        let fuji = [35.36, 138.73];
        let world = crate::points::project(fuji[0], fuji[1]);
        view.center = world;
        let position = view.screen(rect, world);
        let coordinates = crate::points::unproject(view.world_at(rect, position));
        for (actual, expected) in coordinates.into_iter().zip(fuji) {
            assert!((actual - expected).abs() < 1e-9);
        }

        let projected_north = view.screen(rect, [view.center[0], view.center[1] - 0.0001]);
        let direction = (projected_north - rect.center()).normalized();
        assert!((view.orientation.north() - direction).length() < 0.001);
    }

    #[test]
    fn rotated_tiles_cover_the_viewport_and_share_edges() {
        let mut view = View {
            center: [0.5; 2],
            zoom: 8.,
            orientation: Orientation::default(),
        };
        view.orientation.orbit(egui::vec2(380., -400.));
        assert!((view.orientation.vertical_scale.acos().to_degrees() - 80.).abs() < 0.001);
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(820., 620.));
        let tiles = view.tiles_at(rect, Source::Gsi.tile_zoom(view.zoom, Lod::default()));
        for pos in [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ] {
            let [x, y] = view.world_at(rect, pos);
            assert!(tiles.contains(&Tile {
                z: 8,
                x: (x * 256.).floor() as u32,
                y: (y * 256.).floor() as u32,
            }));
        }
        let a = view.tile_corners(
            rect,
            Tile {
                z: 8,
                x: 127,
                y: 128,
            },
        );
        let b = view.tile_corners(
            rect,
            Tile {
                z: 8,
                x: 128,
                y: 128,
            },
        );
        assert_eq!([a[1], a[2]], [b[0], b[3]]);
    }

    #[test]
    fn cursor_anchor_survives_zoom_and_fuji_uses_expected_xyz() {
        let mut view = View {
            center: crate::points::project(35.36, 138.73),
            zoom: 8.,
            orientation: Orientation::default(),
        };
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
        assert_eq!(
            view.tiles_at(rect, Source::Gsi.tile_zoom(view.zoom, Lod::default()))[0],
            Tile {
                z: 8,
                x: 226,
                y: 101
            }
        );
        let p = egui::pos2(600., 340.);
        let world = view.world_at(rect, p);
        view.zoom_at(rect, p, 2.);
        assert_eq!(view.world_at(rect, p), world);
    }
    #[test]
    fn tile_neighbors_wrap_longitude_but_not_poles() {
        let tile = Tile { z: 2, x: 0, y: 0 };
        assert_eq!(tile.neighbor(-1, 0), Some(Tile { z: 2, x: 3, y: 0 }));
        assert_eq!(tile.neighbor(0, -1), None);
        assert_eq!(
            Tile { z: 2, x: 3, y: 3 }.neighbor(1, 0),
            Some(Tile { z: 2, x: 0, y: 3 })
        );
        assert_eq!(Tile { z: 2, x: 3, y: 3 }.neighbor(0, 1), None);
        assert_eq!(tiles_with_halo(vec![tile]).len(), 6);
    }
    #[test]
    #[ignore = "実GPUで通常描画から地形へ渡す補間モードを確認する"]
    fn gpu_settled_map_keeps_selected_interpolation() {
        let ctx = egui::Context::default();
        let instance = eframe::wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU");
        let (device, gpu_queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let format = eframe::wgpu::TextureFormat::Rgba8Unorm;
        let gpu_renderer = eframe::egui_wgpu::Renderer::new(&device, format, None, 1, false);
        let state = eframe::egui_wgpu::RenderState {
            adapter,
            available_adapters: Vec::new(),
            device,
            queue: gpu_queue,
            target_format: format,
            renderer: Arc::new(eframe::epaint::mutex::RwLock::new(gpu_renderer)),
        };
        let renderer = lod_fade::Renderer::new(&state);
        let (queue, _jobs) = mpsc::channel();
        let (send, replies) = mpsc::channel();
        let tile = Tile { z: 0, x: 0, y: 0 };
        let image =
            crate::tile_atlas::Image::load_map(&ctx, ColorImage::new([2, 2], egui::Color32::WHITE));
        let mut tiles = Tiles {
            edge_filter: Interpolation::Edge,
            borders: false,
            source: Source::Gsi,
            cancelled: Arc::default(),
            queue,
            replies,
            pending: HashSet::new(),
            cache: HashMap::from([(tile, (Cached::Texture(image, None), 0))]),
            frame: 0,
            lod: Default::default(),
            material_lods: (0..4).map(|_| Default::default()).collect(),
            material_pass: 1,
            draw_history: HashMap::new(),
        };
        let view = View {
            center: [0.5, 0.5],
            zoom: 1.,
            orientation: Default::default(),
        };
        for mode in [Interpolation::Edge, Interpolation::Nearest] {
            tiles.edge_filter = mode;
            let mut captured = Vec::new();
            for _ in 0..3 {
                let output = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(256., 256.));
                        tiles.paint(
                            ui,
                            ui.painter(),
                            rect,
                            &view,
                            false,
                            0,
                            1.,
                            Some((&state, &renderer)),
                        );
                    });
                });
                for (id, delta) in &output.textures_delta.set {
                    state
                        .renderer
                        .write()
                        .update_texture(&state.device, &state.queue, *id, delta);
                }
                captured = output
                    .shapes
                    .iter()
                    .filter_map(|s| {
                        if let egui::Shape::Callback(c) = &s.shape {
                            c.callback
                                .downcast_ref::<yamamuki_terrain_view::direct_material::Capture>()
                                .map(|c| c.edge_filter)
                        } else {
                            None
                        }
                    })
                    .collect();
            }
            assert!(!captured.is_empty(), "通常描画の命令が作られていない");
            assert!(
                captured.iter().all(|actual| *actual == mode),
                "選択した{mode:?}が通常描画で上書きされた: {captured:?}"
            );
        }
        // 子タイル到着フレームでは先にその画像が表示され、次のフレームから全体のフェードが始まる。
        let child = Tile { z: 1, x: 0, y: 0 };
        tiles.edge_filter = Interpolation::Bilinear;
        tiles.lod = lod_fade::State::default();
        tiles.lod.current = Some(0);
        send.send((
            child,
            Ok(Some((ColorImage::new([2, 2], egui::Color32::BLACK), None))),
        ))
        .unwrap();
        let close_view = View {
            center: [0.25, 0.25],
            zoom: 1.,
            orientation: Default::default(),
        };
        let mut frame = || {
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    tiles.paint(
                        ui,
                        ui.painter(),
                        Rect::from_min_size(Pos2::ZERO, egui::vec2(128., 128.)),
                        &close_view,
                        false,
                        1,
                        1.,
                        Some((&state, &renderer)),
                    );
                });
            });
            for (id, delta) in &output.textures_delta.set {
                state
                    .renderer
                    .write()
                    .update_texture(&state.device, &state.queue, *id, delta);
            }
            output
        };
        frame();
        let transition = frame();
        let captures = transition
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::Callback(c) = &s.shape {
                    c.callback
                        .downcast_ref::<yamamuki_terrain_view::direct_material::Capture>()
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert!(!captures.is_empty(), "LOD切替の描画経路を通っていない");
        assert!(
            captures
                .iter()
                .all(|c| c.textures[0] == c.textures[1] && c.uv[0] == c.uv[1]),
            "表示済みの細タイルがフェード開始時に粗タイルへ戻った"
        );
        drop(frame);
        tiles.lod = lod_fade::State::default();
        tiles.lod.current = Some(1);
        send.send((
            tile,
            Ok(Some((ColorImage::new([2, 2], egui::Color32::WHITE), None))),
        ))
        .unwrap();
        let wide_view = View {
            center: [0.25, 0.25],
            zoom: 0.,
            orientation: Default::default(),
        };
        let mut wide_frame = || {
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    tiles.paint(
                        ui,
                        ui.painter(),
                        Rect::from_min_size(Pos2::ZERO, egui::vec2(128., 128.)),
                        &wide_view,
                        false,
                        0,
                        0.,
                        Some((&state, &renderer)),
                    );
                });
            });
            for (id, delta) in &output.textures_delta.set {
                state
                    .renderer
                    .write()
                    .update_texture(&state.device, &state.queue, *id, delta);
            }
            output
        };
        wide_frame();
        let transition = wide_frame();
        let captures = transition
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::Callback(c) = &s.shape {
                    c.callback
                        .downcast_ref::<yamamuki_terrain_view::direct_material::Capture>()
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert!(!captures.is_empty());
        assert!(
            captures
                .iter()
                .all(|c| c.textures[0] == c.textures[1] && c.uv[0] == c.uv[1]),
            "粗タイルを表示した後に細い下地がフェード元として復活した"
        );
    }
    #[test]
    fn fade_origin_uses_foreground_instead_of_finer_hidden_underlay() {
        let fine = Tile {
            z: 12,
            x: 3626,
            y: 1617,
        };
        let coarse = lod_fade::sample(fine, 11, lod_fade::FULL_UV).0;
        let mut history = DrawHistory::default();
        history.record(fine, fine);
        history.record(coarse, coarse);
        assert_eq!(displayed_sample(&history.previous, fine).unwrap().0, coarse);
        history.record(fine, fine);
        assert_eq!(displayed_sample(&history.previous, fine).unwrap().0, fine);
    }
    #[test]
    fn pending_zoom_uses_finest_cached_parent_for_each_range_and_fade() {
        let ctx = egui::Context::default();
        let (queue, _jobs) = mpsc::channel();
        let (_send, replies) = mpsc::channel();
        let mut tiles = Tiles {
            edge_filter: Interpolation::Bilinear,
            borders: false,
            source: Source::Gsi,
            cancelled: Arc::default(),
            queue,
            replies,
            pending: HashSet::new(),
            cache: HashMap::new(),
            frame: 0,
            lod: Default::default(),
            material_lods: (0..4).map(|_| Default::default()).collect(),
            material_pass: 0,
            draw_history: HashMap::new(),
        };
        let target = Tile {
            z: 12,
            x: 2000,
            y: 1500,
        };
        let parent = lod_fade::sample(target, 11, lod_fade::FULL_UV).0;
        let grandparent = lod_fade::sample(target, 10, lod_fade::FULL_UV).0;
        let make = |name| {
            crate::tile_atlas::Image::load(
                &ctx,
                name,
                ColorImage::new([2, 2], egui::Color32::WHITE),
                egui::TextureOptions::LINEAR,
            )
        };
        tiles
            .cache
            .insert(grandparent, (Cached::Texture(make("z10"), None), 0));
        tiles.lod.current = Some(10);
        assert_eq!(tiles.cached_ancestor(target).unwrap().0, grandparent);
        tiles
            .cache
            .insert(parent, (Cached::Texture(make("z11"), None), 0));
        // 全域のLODがz10のままでも、部分的に届いたz11を表示する。
        assert_eq!(
            tiles.cached_ancestor(target).unwrap(),
            lod_fade::sample(target, 11, lod_fade::FULL_UV)
        );
        tiles
            .cache
            .insert(target, (Cached::Texture(make("z12"), None), 0));
        assert_eq!(tiles.cached_ancestor(target).unwrap().0, target);
        assert_eq!(tiles.cached_ancestor_up_to(target, 11).unwrap().0, parent);
        // 別範囲はz11が未到着ならz10を使い、空白にはしない。
        let other = Tile {
            x: target.x + 2,
            ..target
        };
        assert_eq!(tiles.cached_ancestor(other).unwrap().0, grandparent);
        for pass in [0, 1, 2, 3] {
            tiles.select_material_pass(pass);
            assert_eq!(tiles.cached_ancestor_up_to(target, 11).unwrap().0, parent);
        }
    }
}
