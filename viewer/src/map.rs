use crate::tile_cache;
use anyhow::{Context, Result, ensure};
use eframe::egui::{self, ColorImage, Pos2, Rect, TextureHandle};
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

pub const MIN_ZOOM: f64 = 5.;
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
    pub const MAX_LEVEL: u32 = 30;

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
        self.osm_cache_directory().is_some()
    }

    fn osm_cache_directory(self) -> Option<&'static str> {
        match self {
            Self::OpenStreetMap => Some("osm-tiles"),
            Self::OpenTopoMap => Some("opentopomap-tiles"),
            Self::CyclOsm => Some("cyclosm-tiles"),
            Self::Humanitarian => Some("osm-humanitarian-tiles"),
            _ => None,
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

    fn max_tile_zoom(self) -> u32 {
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
            .min(lod.max_level.unwrap_or(Lod::MAX_LEVEL))
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
        ui.label(format!("ビューワでの取得上限 z{}", self.max_tile_zoom()));
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
    if let Some(directory) = source.osm_cache_directory() {
        let path = tile
            .cache_path(directory)
            .context("OpenStreetMapのタイルキャッシュを利用できません")?;
        return Ok(
            tile_cache::load(client, &url, &path, USER_AGENT, decode_png)?.map(|(image, ttl)| {
                (
                    image,
                    Instant::now().checked_add(ttl.max(Duration::from_secs(1))),
                )
            }),
        );
    }
    let response = client.get(url).send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let body = tile_cache::read_body(response.error_for_status()?)?;
    let image = if source == Source::Photo {
        decode_jpeg(&body)?
    } else {
        decode_png(&body)?
    };
    Ok(Some((image, None)))
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

#[derive(Clone, Copy, Debug, PartialEq)]
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

    pub fn north(self) -> egui::Vec2 {
        self.project(-egui::Vec2::Y).normalized()
    }
}

#[derive(Clone, Copy, PartialEq)]
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
        let anchor = self.world_at(rect, pointer);
        self.zoom = (self.zoom + delta).clamp(MIN_ZOOM, MAX_ZOOM);
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

    pub fn paint_tile(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        tile: Tile,
        texture: egui::TextureId,
    ) {
        self.paint_tile_uv(
            painter,
            rect,
            tile,
            texture,
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
        );
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
    Texture(TextureHandle, Option<Instant>),
    Missing,
    Failed(Instant),
}

type Reply = (Tile, Result<Option<LoadedTile>>);

pub struct Tiles {
    pub source: Source,
    cancelled: Arc<AtomicBool>,
    queue: mpsc::Sender<Tile>,
    replies: mpsc::Receiver<Reply>,
    pending: HashSet<Tile>,
    cache: HashMap<Tile, (Cached, u64)>,
    frame: u64,
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
            source,
            cancelled,
            queue,
            replies,
            pending: HashSet::new(),
            cache: HashMap::new(),
            frame: 0,
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
    ) -> bool {
        self.frame += 1;
        let limit = if dragging { 1 } else { MAX_PENDING };
        let mut processed = 0;
        // 取得とPNGの展開はワーカーで行う。ドラッグ中はGPUへ渡すタイル数を抑える。
        for (tile, result) in self.replies.try_iter().take(limit) {
            processed += 1;
            self.pending.remove(&tile);
            let entry = match result {
                Ok(Some((image, expiry))) => Cached::Texture(
                    ui.ctx().load_texture(
                        tile.url(self.source),
                        image,
                        egui::TextureOptions::LINEAR,
                    ),
                    expiry,
                ),
                Ok(None) => Cached::Missing,
                Err(_) => Cached::Failed(Instant::now()),
            };
            self.cache.insert(tile, (entry, self.frame));
        }
        if processed == limit && !self.pending.is_empty() {
            ui.ctx().request_repaint();
        }
        let tiles = view.tiles_at(rect, tile_zoom);
        // 傾斜で可視範囲が広がっても、表示中のタイルはキャッシュに収める。
        let cache_limit = MAX_CACHE.max(tiles.len() + MAX_PENDING);
        let mut failed = false;
        for tile in tiles {
            if let Some((entry, touched)) = self.cache.get_mut(&tile) {
                *touched = self.frame;
                match entry {
                    Cached::Texture(texture, expiry)
                        if expiry.is_none_or(|until| Instant::now() < until) =>
                    {
                        view.paint_tile(painter, rect, tile, texture.id());
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
        while self.cache.len() > cache_limit {
            if let Some(key) = self
                .cache
                .iter()
                .min_by_key(|(_, (_, touched))| *touched)
                .map(|(&key, _)| key)
            {
                self.cache.remove(&key);
            }
        }
        failed
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
        assert_eq!(Source::Gsi.tile_zoom(2., lod), 0);
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
                .join(source.osm_cache_directory().unwrap())
                .with_extension("tile");
            let (image, _) =
                tile_cache::load(&client, &tile.url(source), &path, USER_AGENT, decode_png)
                    .unwrap_or_else(|error| panic!("{source:?}: {error:#}"))
                    .unwrap_or_else(|| panic!("{source:?}"));
            assert_eq!(image.size, [TILE_SIZE; 2], "{source:?}");
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
            .map(|&source| source.osm_cache_directory().unwrap())
            .collect();
        assert_eq!(urls.len(), Source::OSM_LAYERS.len());
        assert_eq!(directories.len(), Source::OSM_LAYERS.len());
        assert_eq!(
            Source::OpenStreetMap.osm_cache_directory(),
            Some("osm-tiles")
        );
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
            source: Source::Gsi,
            cancelled: Arc::default(),
            queue,
            replies,
            pending,
            cache: HashMap::new(),
            frame: 0,
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
}
