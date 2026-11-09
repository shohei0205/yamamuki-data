//! 共通SQLiteの地表を読み、可視タイルだけを背景で合成する。
use crate::map::{Source as MapSource, Tile, View};
use crate::surface_shade;
use anyhow::{Result, bail, ensure};
use eframe::{
    egui::{self, ColorImage, Pos2, Rect, TextureHandle},
    egui_wgpu::{RenderState, wgpu},
};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};
use yamamuki_data_sqlite::{self as db, SurfaceLevel, rusqlite::Connection};

const PENDING: usize = 12;
const CACHE_BYTES: usize = 96 * 1024 * 1024;
const ESA_WORK_ZOOM: u32 = 8;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    color: Option<[u8; 4]>,
    background: Option<[u8; 4]>,
    neutral: Option<u8>,
    strength: Option<f32>,
    complete_tile_extent: Option<bool>,
}

enum Role {
    Rgb,
    Mask,
    Hillshade,
}

struct Layer {
    key: i64,
    id: String,
    name: String,
    role: Role,
    settings: Settings,
    levels: Vec<SurfaceLevel>,
    credits: Vec<String>,
}

struct Dataset {
    key: i64,
    label: String,
    info: db::Dataset,
    metadata_json: String,
    work_areas: Option<HashSet<(u32, u32)>>,
    complete_tile_extent: bool,
    layers: Vec<Layer>,
    bounds: [[f64; 2]; 2],
}

pub struct Source {
    path: PathBuf,
    identity: PathBuf,
    size_bytes: Option<u64>,
    datasets: Vec<Arc<Dataset>>,
}

impl Source {
    pub fn open(path: PathBuf) -> Result<Self> {
        let conn = db::open(&path)?;
        let mut datasets = Vec::new();
        for dataset in db::datasets(&conn)? {
            if dataset.kind != "surface" || dataset.format_version != 1 {
                continue;
            }
            let mut layers = Vec::new();
            for layer in db::surface_layers(&conn, dataset.key)? {
                let (role, name) = match (layer.id.as_str(), layer.role.as_str()) {
                    ("forest", "mask") => (Role::Mask, "森林"),
                    ("water", "mask") => (Role::Mask, "水域"),
                    ("hillshade", "hillshade") => (Role::Hillshade, "陰影"),
                    // 予約IDとroleが食い違う行と、旧版の補助レイヤーは読み飛ばす。
                    ("forest" | "water" | "hillshade", _) | (_, "coverage") => continue,
                    (id, "rgb") => (Role::Rgb, id),
                    (id, "mask") => (Role::Mask, id),
                    (id, "hillshade") => (Role::Hillshade, id),
                    (_, other) => bail!("未対応の地表レイヤー: {other}"),
                };
                let settings: Settings = serde_json::from_value(layer.settings)?;
                if matches!(role, Role::Mask) {
                    ensure!(settings.color.is_some(), "地表の着色設定がありません");
                }
                if matches!(role, Role::Hillshade) {
                    ensure!(
                        settings.neutral.is_some_and(|v| v > 0)
                            && settings.strength.is_some_and(|v| v.is_finite()),
                        "陰影の設定が不正です"
                    );
                }
                let credits = dataset.metadata["sources"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|source| {
                        source["name"].as_str().is_some_and(|name| {
                            layer.source_names.iter().any(|value| value == name)
                        })
                    })
                    .filter_map(|source| {
                        source["attribution"]
                            .as_str()
                            .or_else(|| source["name"].as_str())
                    })
                    .map(str::to_owned)
                    .collect();
                layers.push(Layer {
                    key: layer.key,
                    name: name.to_owned(),
                    id: layer.id,
                    role,
                    settings,
                    levels: db::surface_levels(&conn, dataset.key, layer.key)?,
                    credits,
                });
            }
            let mut bounds = [[1.0f64; 2], [0.; 2]];
            for layer in &layers {
                for level in &layer.levels {
                    let extent = conn.query_row(
                        "SELECT min(x),min(y),max(x),max(y) FROM surface_tiles
                         WHERE dataset_key=?1 AND layer_key=?2 AND z=?3",
                        (dataset.key, layer.key, level.z),
                        |row| {
                            Ok([
                                row.get::<_, Option<u32>>(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                            ])
                        },
                    )?;
                    if let [Some(left), Some(top), Some(right), Some(bottom)] = extent {
                        let side = f64::from(1u32 << level.z);
                        for (axis, (lo, hi)) in
                            [(left, right), (top, bottom)].into_iter().enumerate()
                        {
                            bounds[0][axis] = bounds[0][axis].min(f64::from(lo) / side);
                            bounds[1][axis] = bounds[1][axis].max(f64::from(hi + 1) / side);
                        }
                    }
                }
            }
            if bounds[0][0] < bounds[1][0] && bounds[0][1] < bounds[1][1] {
                // ESAの粗いタイルには、未生成のz8作業区画もゼロで詰めてある。
                let complete_tile_extent = layers
                    .iter()
                    .any(|layer| layer.settings.complete_tile_extent == Some(true));
                let work_areas = if dataset.id == "esa-worldcover-japan" && !complete_tile_extent {
                    let mut query = conn.prepare(
                        "SELECT t.x,t.y FROM surface_tiles t
                         JOIN surface_layers l USING (dataset_key,layer_key)
                         WHERE t.dataset_key=?1 AND t.z=?2 AND l.id IN ('forest','water') AND l.role='mask'",
                    )?;
                    let areas = query
                        .query_map((dataset.key, ESA_WORK_ZOOM), |row| {
                            Ok((row.get(0)?, row.get(1)?))
                        })?
                        .collect::<db::rusqlite::Result<HashSet<_>>>()?;
                    (!areas.is_empty()).then_some(areas)
                } else {
                    None
                };
                datasets.push(Arc::new(Dataset {
                    work_areas,
                    complete_tile_extent,
                    key: dataset.key,
                    label: format!("{} ({})", dataset.title, dataset.data_version),
                    metadata_json: serde_json::to_string_pretty(&dataset.metadata)?,
                    info: dataset,
                    layers,
                    bounds,
                }));
            }
        }
        ensure!(
            !datasets.is_empty(),
            "表示できる地表データセットがありません"
        );
        let identity = path.canonicalize().unwrap_or_else(|_| path.clone());
        Ok(Self {
            size_bytes: std::fs::metadata(&path).ok().map(|metadata| metadata.len()),
            path,
            identity,
            datasets,
        })
    }
}

fn auto_level(levels: &[SurfaceLevel], scale: f64) -> Option<u32> {
    let density = |level: &&SurfaceLevel| (u64::from(level.pixels)) << level.z;
    levels
        .iter()
        .filter(|level| density(level) as f64 >= scale)
        .min_by_key(density)
        .or_else(|| levels.iter().max_by_key(density))
        .map(|level| level.z)
}

type Pixel = [f32; 4];

fn premultiply(color: [u8; 4], alpha: f32) -> Pixel {
    let a = f32::from(color[3]) / 255. * alpha;
    [
        f32::from(color[0]) * a,
        f32::from(color[1]) * a,
        f32::from(color[2]) * a,
        a,
    ]
}

fn over(front: Pixel, back: Pixel) -> Pixel {
    std::array::from_fn(|i| front[i] + (1. - front[3]) * back[i])
}

#[derive(Clone, Copy)]
struct Sample {
    tile: u32,
    local: f64,
}

fn samples(index: u32, dst_z: u32, src_z: u32, pixels: usize) -> Vec<Sample> {
    let scale = 2f64.powi(src_z as i32 - dst_z as i32);
    (0..pixels)
        .map(|i| {
            let position = (f64::from(index) + (i as f64 + 0.5) / pixels as f64) * scale;
            Sample {
                tile: position.floor() as u32,
                local: position.fract(),
            }
        })
        .collect()
}

struct ShadeImage {
    image: ColorImage,
    neutral: u8,
    strength: f32,
}

#[derive(Default)]
struct ComposedTile {
    image: Option<ColorImage>,
    shades: Vec<ShadeImage>,
}

fn composite(
    conn: &Connection,
    set: &Dataset,
    tile: Tile,
    pixels: usize,
    levels: &[Option<u32>],
    overlay: bool,
    auto_levels: &[bool],
) -> Result<ComposedTile> {
    let mut tile_images = ComposedTile::default();
    let work_areas = set.work_areas.as_ref();
    if let Some(areas) = work_areas
        && tile.z >= ESA_WORK_ZOOM
    {
        let shift = tile.z - ESA_WORK_ZOOM;
        if !areas.contains(&(tile.x >> shift, tile.y >> shift)) {
            return Ok(tile_images);
        }
    }
    let mut colors = vec![[0.; 4]; pixels * pixels];
    let mut has_base = false;
    for (index, (layer, &z)) in set.layers.iter().zip(levels).enumerate() {
        let Some(z) = z else { continue };
        let fallback = set.complete_tile_extent && auto_levels.get(index) == Some(&true);
        let is_shade = matches!(layer.role, Role::Hillshade);
        // オンライン地図に掛ける陰影と、ローカルの地表画像を分けておく。
        let shade_online = overlay && is_shade;
        let apply_shade = has_base && is_shade;
        has_base |= !shade_online;
        let mut shade_image = None;
        let background = if overlay {
            [0.; 4]
        } else {
            layer
                .settings
                .background
                .map_or([0.; 4], |color| premultiply(color, 1.))
        };
        let xs = samples(tile.x, tile.z, z, pixels);
        let ys = samples(tile.y, tile.z, z, pixels);
        let mut row = 0;
        for ys in ys.chunk_by(|a, b| a.tile == b.tile) {
            let mut col = 0;
            for xs in xs.chunk_by(|a, b| a.tile == b.tile) {
                let x = xs[0].tile;
                let y = ys[0].tile;
                let mut source_z = z;
                let mut image = db::surface_tile(conn, set.key, layer.key, z, x, y)?;
                if fallback && image.is_none() {
                    for level in layer.levels.iter().rev().filter(|level| level.z < z) {
                        let shift = z - level.z;
                        image = db::surface_tile(
                            conn,
                            set.key,
                            layer.key,
                            level.z,
                            x >> shift,
                            y >> shift,
                        )?;
                        if image.is_some() {
                            source_z = level.z;
                            break;
                        }
                    }
                }
                if let Some(image) = image {
                    let mut shade_pixels = shade_online.then(|| {
                        &mut shade_image
                            .get_or_insert_with(|| ShadeImage {
                                image: ColorImage::new(
                                    [pixels, pixels],
                                    egui::Color32::TRANSPARENT,
                                ),
                                neutral: layer.settings.neutral.expect("hillshade neutral"),
                                strength: layer.settings.strength.expect("hillshade strength"),
                            })
                            .image
                            .pixels
                    });
                    let rgba = image.into_rgba8();
                    let size = rgba.width();
                    let sample = |ix, iy| {
                        let v = rgba.get_pixel(ix, iy).0;
                        match layer.role {
                            Role::Rgb => premultiply(v, 1.),
                            Role::Mask => over(
                                premultiply(
                                    layer.settings.color.expect("mask color"),
                                    f32::from(v[1]) / 255.,
                                ),
                                background,
                            ),
                            Role::Hillshade => {
                                let gray = f32::from(v[1]);
                                [gray, gray, gray, 1.]
                            }
                        }
                    };
                    let factor = 1u32 << (z - source_z);
                    let index = |s: &Sample| {
                        let local = (f64::from(s.tile % factor) + s.local) / f64::from(factor);
                        let coord = (local * f64::from(size) - 0.5).clamp(0., f64::from(size - 1));
                        let i = coord as u32;
                        (i, (i + 1).min(size - 1), coord.fract() as f32)
                    };
                    for (j, sy) in ys.iter().enumerate() {
                        let (y0, y1, fy) = index(sy);
                        for (i, sx) in xs.iter().enumerate() {
                            let (x0, x1, fx) = index(sx);
                            let v =
                                if source_z == tile.z && z == source_z && size as usize == pixels {
                                    sample(i as u32, j as u32)
                                } else {
                                    let [a, b, c, d] = [
                                        sample(x0, y0),
                                        sample(x1, y0),
                                        sample(x0, y1),
                                        sample(x1, y1),
                                    ];
                                    std::array::from_fn(|k| {
                                        let u = a[k] * (1. - fx) + b[k] * fx;
                                        let l = c[k] * (1. - fx) + d[k] * fx;
                                        u * (1. - fy) + l * fy
                                    })
                                };
                            let index = (row + j) * pixels + col + i;
                            if let Some(shade_pixels) = shade_pixels.as_deref_mut() {
                                shade_pixels[index] = egui::Color32::from_gray(v[0].round() as u8);
                            }
                            let dest = &mut colors[index];
                            if apply_shade {
                                let neutral =
                                    f32::from(layer.settings.neutral.expect("hillshade neutral"));
                                let gain = 1.
                                    + layer.settings.strength.expect("hillshade strength")
                                        * (v[0] - neutral * v[3])
                                        / neutral;
                                let max = 255. * dest[3];
                                for channel in &mut dest[..3] {
                                    *channel = (*channel * gain).clamp(0., max);
                                }
                            } else if !shade_online {
                                *dest = over(v, *dest);
                            }
                        }
                    }
                }
                col += xs.len();
            }
            row += ys.len();
        }
        tile_images.shades.extend(shade_image);
    }
    if let Some(areas) = work_areas
        && tile.z < ESA_WORK_ZOOM
    {
        let xs = samples(tile.x, tile.z, ESA_WORK_ZOOM, pixels);
        let ys = samples(tile.y, tile.z, ESA_WORK_ZOOM, pixels);
        let mut row = 0;
        for ys in ys.chunk_by(|a, b| a.tile == b.tile) {
            let mut col = 0;
            for xs in xs.chunk_by(|a, b| a.tile == b.tile) {
                if !areas.contains(&(xs[0].tile, ys[0].tile)) {
                    for y in row..row + ys.len() {
                        let start = y * pixels + col;
                        let span = start..start + xs.len();
                        colors[span.clone()].fill([0.; 4]);
                        for shade in &mut tile_images.shades {
                            shade.image.pixels[span.clone()].fill(egui::Color32::TRANSPARENT);
                        }
                    }
                }
                col += xs.len();
            }
            row += ys.len();
        }
    }
    if colors.iter().all(|v| v[3] == 0.) {
        return Ok(tile_images);
    }
    let rgba: Vec<_> = colors
        .into_iter()
        .flat_map(|[r, g, b, a]| {
            let rgb = if a > 0. {
                [r, g, b].map(|v| (v / a).round() as u8)
            } else {
                [0; 3]
            };
            [rgb[0], rgb[1], rgb[2], (a * 255.).round() as u8]
        })
        .collect();
    tile_images.image = Some(ColorImage::from_rgba_unmultiplied([pixels, pixels], &rgba));
    Ok(tile_images)
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Key {
    dataset: usize,
    tile: Tile,
    pixels: usize,
    levels: Vec<Option<u32>>,
    auto_levels: Vec<bool>,
    overlay: bool,
}

impl Key {
    fn same_grid(&self, other: &Self) -> bool {
        self.dataset == other.dataset
            && self.tile.z == other.tile.z
            && self.pixels == other.pixels
            && self.levels == other.levels
            && self.auto_levels == other.auto_levels
            && self.overlay == other.overlay
    }
}

type Reply = (Key, Result<ComposedTile>);

struct Cached {
    image: Option<TextureHandle>,
    shades: Vec<wgpu::BindGroup>,
    bytes: usize,
    touched: u64,
    failed: bool,
}

impl Cached {
    fn collect(
        &self,
        view: &View,
        rect: Rect,
        tile: Tile,
        images: &mut Vec<(Tile, egui::TextureId)>,
        shades: &mut Vec<([Pos2; 4], wgpu::BindGroup)>,
    ) -> bool {
        if let Some(image) = &self.image {
            images.push((tile, image.id()));
        }
        if !self.shades.is_empty() {
            let corners = view.tile_corners(rect, tile);
            shades.extend(self.shades.iter().map(|shade| (corners, shade.clone())));
        }
        self.image.is_some() || !self.shades.is_empty()
    }
}

struct LevelStats {
    z: u32,
    pixels: u32,
    tiles: u64,
    bytes: u64,
}

fn read_layer_stats(path: &Path, dataset: i64, layer: i64) -> Result<Vec<LevelStats>> {
    let conn = db::open(path)?;
    let mut statement = conn.prepare(
        "SELECT l.z, l.tile_pixels, COUNT(t.z), COALESCE(SUM(length(t.payload)), 0)
         FROM surface_levels AS l
         LEFT JOIN surface_tiles AS t USING (dataset_key, layer_key, z)
         WHERE l.dataset_key=?1 AND l.layer_key=?2
         GROUP BY l.z, l.tile_pixels
         ORDER BY l.z DESC",
    )?;
    let rows = statement.query_map([dataset, layer], |row| {
        Ok(LevelStats {
            z: row.get(0)?,
            pixels: row.get(1)?,
            tiles: row.get(2)?,
            bytes: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<db::rusqlite::Result<_>>()?)
}

enum LayerStats {
    Loading(mpsc::Receiver<Result<Vec<LevelStats>>>),
    Ready(Vec<LevelStats>),
    Failed(String),
}

impl LayerStats {
    fn load(ctx: egui::Context, path: PathBuf, dataset: i64, layer: i64) -> Self {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(read_layer_stats(&path, dataset, layer));
            ctx.request_repaint();
        });
        Self::Loading(receiver)
    }

    fn show(&mut self, ui: &mut egui::Ui) {
        if let Self::Loading(receiver) = self {
            match receiver.try_recv() {
                Ok(Ok(stats)) => *self = Self::Ready(stats),
                Ok(Err(error)) => *self = Self::Failed(format!("{error:#}")),
                Err(mpsc::TryRecvError::Disconnected) => {
                    *self = Self::Failed("集計が中断されました".to_owned());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        match self {
            Self::Loading(_) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("収録タイルを集計中");
                });
            }
            Self::Failed(error) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("集計できません: {error}"),
                );
            }
            Self::Ready(levels) => {
                let size = |ui: &mut egui::Ui, bytes| {
                    let label = if bytes >= 1024 * 1024 {
                        format!("{:.2} MiB", bytes as f64 / (1024. * 1024.))
                    } else if bytes >= 1024 {
                        format!("{:.2} KiB", bytes as f64 / 1024.)
                    } else {
                        format!("{bytes} B")
                    };
                    ui.label(label).on_hover_text(format!("{bytes} バイト"));
                };
                egui::Grid::new("surface-layer-stats")
                    .striped(true)
                    .show(ui, |ui| {
                        for title in ["レベル", "タイル寸法", "タイル数", "圧縮データ"]
                        {
                            ui.strong(title);
                        }
                        ui.end_row();
                        for level in levels.iter() {
                            ui.label(format!("z{}", level.z));
                            ui.label(format!("{} × {}", level.pixels, level.pixels));
                            ui.label(level.tiles.to_string());
                            size(ui, level.bytes);
                            ui.end_row();
                        }
                        ui.strong("合計");
                        ui.label("—");
                        ui.strong(
                            levels
                                .iter()
                                .map(|level| level.tiles)
                                .sum::<u64>()
                                .to_string(),
                        );
                        size(ui, levels.iter().map(|level| level.bytes).sum::<u64>());
                        ui.end_row();
                    });
                ui.small("容量は画像BLOBの合計です。");
            }
        }
    }
}

pub struct Surface {
    source: Source,
    layer_stats: HashMap<(i64, i64), LayerStats>,
    dataset: usize,
    show_file_info: bool,
    options: Vec<(bool, Option<u32>)>,
    pub enabled: bool,
    pub fit: bool,
    pub reveal: bool,
    jobs: mpsc::Sender<Key>,
    replies: mpsc::Receiver<Reply>,
    pending: HashSet<Key>,
    cache: HashMap<Key, Cached>,
    cache_bytes: usize,
    frame: u64,
    displayed: Option<Key>,
    shade_renderer: Option<surface_shade::Renderer>,
    error: Option<String>,
}

impl Surface {
    pub fn new(ctx: &egui::Context, source: Source) -> Self {
        let (jobs, incoming) = mpsc::channel::<Key>();
        let (replies, outgoing) = mpsc::channel();
        let incoming = Arc::new(Mutex::new(incoming));
        for _ in 0..2 {
            let (incoming, replies, ctx) = (incoming.clone(), replies.clone(), ctx.clone());
            let datasets = source.datasets.clone();
            let path = source.path.clone();
            std::thread::spawn(move || {
                let conn = db::open(&path);
                loop {
                    let job = incoming.lock().expect("surface queue").recv();
                    let Ok(job) = job else { break };
                    let result = match &conn {
                        Ok(conn) => composite(
                            conn,
                            &datasets[job.dataset],
                            job.tile,
                            job.pixels,
                            &job.levels,
                            job.overlay,
                            &job.auto_levels,
                        ),
                        Err(error) => Err(anyhow::anyhow!("{error:#}")),
                    };
                    if replies.send((job, result)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        }
        let options = Self::default_options(&source.datasets[0]);
        Self {
            source,
            layer_stats: HashMap::new(),
            dataset: 0,
            show_file_info: false,
            options,
            enabled: true,
            fit: true,
            reveal: false,
            jobs,
            replies: outgoing,
            pending: HashSet::new(),
            cache: HashMap::new(),
            cache_bytes: 0,
            frame: 0,
            displayed: None,
            shade_renderer: None,
            error: None,
        }
    }

    fn default_options(set: &Dataset) -> Vec<(bool, Option<u32>)> {
        vec![(true, None); set.layers.len()]
    }

    pub fn same_file(&self, source: &Source) -> bool {
        self.source.identity == source.identity
    }

    pub fn has_enabled_layers(&self) -> bool {
        self.options.iter().any(|(shown, _)| *shown)
    }

    fn dataset_name(&self, ui: &mut egui::Ui) -> Option<usize> {
        if self.source.datasets.len() > 1 {
            let mut selected = self.dataset;
            egui::ComboBox::from_id_salt(("surface-dataset", &self.source.identity))
                .selected_text(&self.source.datasets[selected].label)
                .show_ui(ui, |ui| {
                    for (i, set) in self.source.datasets.iter().enumerate() {
                        ui.selectable_value(&mut selected, i, &set.label);
                    }
                });
            (selected != self.dataset).then_some(selected)
        } else {
            let label = &self.source.datasets[self.dataset].label;
            ui.add(
                egui::Label::new(egui::RichText::new(label).strong())
                    .truncate()
                    .selectable(false),
            )
            .on_hover_text(label);
            None
        }
    }

    pub fn show_file_info(&mut self, ctx: &egui::Context) {
        if !self.show_file_info {
            return;
        }
        let set = &self.source.datasets[self.dataset];
        let filename = self
            .source
            .path
            .file_name()
            .unwrap_or(self.source.path.as_os_str());
        egui::Window::new("地表ファイルの情報")
            .id(egui::Id::new(("surface-file-info", &self.source.identity)))
            .open(&mut self.show_file_info)
            .default_size([550., 400.])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.strong(filename.to_string_lossy());
                    ui.label(self.source.identity.display().to_string());
                    ui.strong(&set.info.title);
                    ui.label(format!("データセットID {}", set.info.id));
                    ui.label(format!("データ版 {}", set.info.data_version));
                    ui.label("形式 SQLite コンテナ版1");
                    ui.label(format!("地表の格納形式 版{}", set.info.format_version));
                    if let Some(bytes) = self.source.size_bytes {
                        ui.label(format!("ファイルサイズ {bytes} B"));
                    }
                    if ui.button("全体を表示").clicked() {
                        self.fit = true;
                    }
                    ui.separator();
                    ui.strong(format!("収録レイヤー（{}）", set.layers.len()));
                    egui::Grid::new("surface-file-layers")
                        .striped(true)
                        .show(ui, |ui| {
                            for title in ["レイヤー", "ID", "Role", "収録レベル"] {
                                ui.strong(title);
                            }
                            ui.end_row();
                            for layer in &set.layers {
                                ui.label(&layer.name);
                                ui.label(&layer.id);
                                ui.label(match layer.role {
                                    Role::Rgb => "rgb",
                                    Role::Mask => "mask",
                                    Role::Hillshade => "hillshade",
                                });
                                ui.label(
                                    layer
                                        .levels
                                        .iter()
                                        .map(|level| format!("z{}", level.z))
                                        .collect::<Vec<_>>()
                                        .join(" / "),
                                );
                                ui.end_row();
                            }
                        });
                    ui.separator();
                    ui.strong("Metadata JSON");
                    crate::appearance::json_view(ui, &set.metadata_json);
                });
            });
    }

    pub fn credits<'a>(&'a self, ui: &mut egui::Ui, shown: &mut HashSet<&'a str>) {
        let set = &self.source.datasets[self.dataset];
        for (layer, _) in set.layers.iter().zip(&self.options).filter(|(_, o)| o.0) {
            for credit in &layer.credits {
                if shown.insert(credit.as_str()) {
                    ui.label(egui::RichText::new(credit).small());
                }
            }
        }
    }

    pub fn fit_view(&mut self, view: &mut View, rect: Rect) {
        if self.fit {
            view.fit(rect, self.source.datasets[self.dataset].bounds);
            self.fit = false;
        }
    }

    pub fn paint(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        view: &View,
        dragging: bool,
        overlay: bool,
        render_state: Option<&RenderState>,
    ) {
        let set = &self.source.datasets[self.dataset];
        self.frame += 1;
        let limit = if dragging { 1 } else { PENDING };
        let mut received = 0;
        for (key, result) in self.replies.try_iter().take(limit) {
            received += 1;
            self.pending.remove(&key);
            let mut entry = Cached {
                image: None,
                shades: Vec::new(),
                bytes: 0,
                touched: self.frame,
                failed: false,
            };
            match result {
                Ok(ComposedTile { image, shades }) => {
                    if let Some(image) = image {
                        entry.bytes += image.pixels.len() * 4;
                        entry.image = Some(ui.ctx().load_texture(
                            "surface tile",
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                    if let Some(state) = render_state.filter(|_| !shades.is_empty()) {
                        let renderer = self
                            .shade_renderer
                            .get_or_insert_with(|| surface_shade::Renderer::new(state));
                        for shade in shades {
                            entry.bytes += shade.image.pixels.len() * 2;
                            entry.shades.push(renderer.upload(
                                &shade.image,
                                shade.neutral,
                                shade.strength,
                            ));
                        }
                    }
                }
                Err(error) => {
                    self.error = Some(format!("地表タイルを読み込めません: {error:#}"));
                    entry.failed = true;
                }
            }
            self.cache_bytes += entry.bytes;
            self.cache.insert(key, entry);
        }
        if received == limit && !self.pending.is_empty() {
            ui.ctx().request_repaint();
        }
        let scale = view.scale() * f64::from(ui.ctx().pixels_per_point());
        let levels: Vec<_> = set
            .layers
            .iter()
            .zip(&self.options)
            .map(|(l, &(show, z))| {
                if show {
                    z.or_else(|| auto_level(&l.levels, scale))
                } else {
                    None
                }
            })
            .collect();
        let auto_levels: Vec<_> = self
            .options
            .iter()
            .map(|&(shown, z)| shown && z.is_none())
            .collect();
        let mut shade_tiles = Vec::new();
        let mut images = Vec::new();
        if let Some(finest) = levels.iter().flatten().max().copied() {
            let draw_zoom = (scale / 384.).log2().ceil().clamp(0., 30.) as u32;
            let z = finest.min(draw_zoom);
            let pixels = set
                .layers
                .iter()
                .zip(&levels)
                .filter_map(|(l, z)| {
                    l.levels
                        .iter()
                        .find(|level| Some(level.z) == *z)
                        .map(|level| level.pixels as usize)
                })
                .max()
                .unwrap_or(256)
                .clamp(256, 512);
            let visible: Vec<_> = view
                .tiles_at(rect, z)
                .into_iter()
                .map(|tile| Key {
                    dataset: self.dataset,
                    tile,
                    pixels,
                    levels: levels.clone(),
                    auto_levels: auto_levels.clone(),
                    overlay,
                })
                .collect();
            let mut waiting = false;
            for key in &visible {
                if let Some(entry) = self.cache.get_mut(key) {
                    entry.touched = self.frame;
                    if entry.failed && self.error.is_none() {
                        self.error = Some("地表タイルを読み込めません".into());
                    }
                    continue;
                }
                waiting = true;
                if self.pending.len() < PENDING
                    && !self.pending.contains(key)
                    && self.jobs.send(key.clone()).is_ok()
                {
                    self.pending.insert(key.clone());
                }
            }

            // 自動LODの読み込み中は、直前のテクスチャで覆える範囲を先に描く。
            let mut used_previous = false;
            if let Some((current, previous)) = visible.first().zip(self.displayed.as_ref())
                && waiting
                && current.overlay == previous.overlay
                && !current.same_grid(previous)
            {
                for (key, entry) in &mut self.cache {
                    if key.same_grid(previous)
                        && Rect::from_points(&view.tile_corners(rect, key.tile)).intersects(rect)
                        && entry.collect(view, rect, key.tile, &mut images, &mut shade_tiles)
                    {
                        entry.touched = self.frame;
                        used_previous = true;
                    }
                }
            }
            if !used_previous {
                for key in &visible {
                    if let Some(entry) = self.cache.get(key) {
                        entry.collect(view, rect, key.tile, &mut images, &mut shade_tiles);
                    }
                }
            }
            if !waiting {
                self.displayed = visible.into_iter().next();
            }
        }
        if let Some(renderer) = &self.shade_renderer {
            renderer.paint(painter, rect, shade_tiles);
        }
        for (tile, image) in images {
            view.paint_tile(painter, rect, tile, image);
        }
        while self.cache_bytes > CACHE_BYTES || self.cache.len() > 512 {
            let Some(key) = self
                .cache
                .iter()
                .filter(|(_, e)| e.touched != self.frame)
                .min_by_key(|(_, e)| e.touched)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.cache_bytes -= self.cache.remove(&key).expect("cached tile").bytes;
        }
    }
}

pub struct PointLayer<'a> {
    pub path: &'a Path,
    pub count: usize,
    pub enabled: &'a mut bool,
    pub close: &'a mut bool,
    pub show_info: &'a mut bool,
    pub fit: &'a mut bool,
}

pub fn layer_list(
    ui: &mut egui::Ui,
    surfaces: &mut [Surface],
    mut point_layer: Option<PointLayer<'_>>,
    map_source: &mut MapSource,
    online_enabled: &mut bool,
    zoom: f64,
    scale: f64,
    max_height: f32,
) -> Option<usize> {
    const HEADER_HEIGHT: f32 = 20.;
    const LAYER_INDENT: f32 = 14.;
    const LOD_WIDTH: f32 = 110.;
    const INFO_WIDTH: f32 = 36.;

    enum Row {
        PointsGroup,
        Points,
        Online,
        Provider(&'static str),
        Map(MapSource),
        File(usize),
        Surface(usize, usize),
        Error(usize),
    }

    let mut rows = Vec::new();
    if point_layer.is_some() {
        rows.extend([Row::PointsGroup, Row::Points]);
    }
    rows.push(Row::Online);
    for (name, maps) in [
        ("国土地理院", MapSource::GSI_LAYERS.as_slice()),
        ("OpenStreetMap系", MapSource::OSM_LAYERS.as_slice()),
    ] {
        rows.push(Row::Provider(name));
        rows.extend(maps.iter().copied().map(Row::Map));
    }
    let mut reveal = None;
    for (file, surface) in surfaces.iter_mut().enumerate() {
        if std::mem::take(&mut surface.reveal) {
            reveal = Some(rows.len());
        }
        rows.push(Row::File(file));
        rows.extend((0..surface.options.len()).map(|layer| Row::Surface(file, layer)));
        if surface.error.is_some() {
            rows.push(Row::Error(file));
        }
    }
    let mut close = None;
    let mut change_dataset = None;
    let previous_online = (*map_source, *online_enabled);
    let max_scroll_height = (max_height - HEADER_HEIGHT - ui.spacing().item_spacing.y).max(0.);
    let mut table = egui_extras::TableBuilder::new(ui)
        .id_salt("layer-list")
        .striped(true)
        .sense(egui::Sense::click())
        .min_scrolled_height(0.)
        .max_scroll_height(max_scroll_height)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(egui_extras::Column::exact(38.))
        .column(egui_extras::Column::remainder().clip(true))
        .column(egui_extras::Column::exact(INFO_WIDTH + LOD_WIDTH));
    if let Some(row) = reveal {
        table = table
            .animate_scrolling(false)
            .scroll_to_row(row, Some(egui::Align::TOP));
    }
    table
        .header(HEADER_HEIGHT, |mut header| {
            for title in ["表示", "レイヤー"] {
                header.col(|ui| {
                    ui.strong(title);
                });
            }
            header.col(|ui| {
                ui.add_space(INFO_WIDTH);
                ui.strong("LOD");
            });
        })
        .body(|body| {
            body.rows(24., rows.len(), |mut row| match rows[row.index()] {
                Row::PointsGroup => {
                    let layer = point_layer.as_mut().expect("地点データ");
                    let before = *layer.enabled;
                    row.col(|ui| {
                        ui.checkbox(layer.enabled, "")
                            .on_hover_text("地点マークの表示");
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new("地点データ").strong())
                                .selectable(false),
                        );
                    });
                    row.col(|ui| {
                        ui.label(format!("{} 地点", layer.count));
                    });
                    if row.response().clicked() && *layer.enabled == before {
                        *layer.enabled = !before;
                    }
                }
                Row::Points => {
                    let layer = point_layer.as_mut().expect("地点ファイル");
                    let name = layer.path.file_name().unwrap_or(layer.path.as_os_str());
                    row.col(|_| {});
                    row.col(|ui| {
                        if !*layer.enabled {
                            ui.disable();
                        }
                        ui.add_space(LAYER_INDENT);
                        ui.add(
                            egui::Label::new(name.to_string_lossy())
                                .truncate()
                                .selectable(false),
                        )
                        .on_hover_text(layer.path.display().to_string());
                    });
                    row.col(|ui| {
                        if ui.button("i").on_hover_text("地点ファイルの情報").clicked() {
                            *layer.show_info = true;
                        }
                        if ui.small_button("地点全体").clicked() {
                            *layer.fit = true;
                        }
                        if ui
                            .small_button("閉じる")
                            .on_hover_text("地点ファイルを閉じる")
                            .clicked()
                        {
                            *layer.close = true;
                        }
                    });
                }
                Row::Online => {
                    let before = *online_enabled;
                    row.col(|ui| {
                        ui.checkbox(online_enabled, "")
                            .on_hover_text("オンライン地図");
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new("オンライン地図").strong())
                                .selectable(false),
                        );
                    });
                    row.col(|_| {});
                    if row.response().clicked() && *online_enabled == before {
                        *online_enabled = !before;
                    }
                }
                Row::Provider(name) => {
                    row.col(|_| {});
                    row.col(|ui| {
                        if !*online_enabled {
                            ui.disable();
                        }
                        ui.add_space(LAYER_INDENT);
                        ui.strong(name);
                    });
                    row.col(|_| {});
                }
                Row::Map(source) => {
                    let name = source.label();
                    let mut clicked = false;
                    let mut info_clicked = false;
                    row.col(|ui| {
                        if !*online_enabled {
                            ui.disable();
                        }
                        clicked = ui
                            .radio(*map_source == source, "")
                            .on_hover_text(name)
                            .clicked();
                    });
                    row.col(|ui| {
                        if !*online_enabled {
                            ui.disable();
                        }
                        ui.add_space(2. * LAYER_INDENT);
                        ui.add(egui::Label::new(name).truncate().selectable(false))
                            .on_hover_text(name);
                    });
                    row.col(|ui| {
                        info_clicked = ui
                            .menu_button("i", |ui| source.show_info(ui, zoom))
                            .response
                            .on_hover_text("オンライン地図の情報")
                            .clicked();
                        if !*online_enabled {
                            ui.disable();
                        }
                        ui.add(
                            egui::Label::new(format!("自動 (z{})", source.tile_zoom(zoom)))
                                .selectable(false),
                        );
                    });
                    if *online_enabled && !info_clicked && (clicked || row.response().clicked()) {
                        *map_source = source;
                    }
                }
                Row::File(file) => {
                    let surface = &mut surfaces[file];
                    row.col(|ui| {
                        ui.checkbox(&mut surface.enabled, "")
                            .on_hover_text("データセットの表示");
                    });
                    let (_, name) = row.col(|ui| {
                        if let Some(dataset) = surface.dataset_name(ui) {
                            change_dataset = Some((file, dataset));
                        }
                    });
                    if name.clicked() && surface.source.datasets.len() == 1 {
                        surface.enabled = !surface.enabled;
                    }
                    row.col(|ui| {
                        if ui.button("i").on_hover_text("地表ファイルの情報").clicked() {
                            surface.show_file_info = true;
                        }
                        if ui.button("閉じる").clicked() {
                            close = Some(file);
                        }
                    });
                }
                Row::Surface(file, index) => {
                    let surface = &mut surfaces[file];
                    let set = &surface.source.datasets[surface.dataset];
                    let layer = &set.layers[index];
                    let (shown, level) = &mut surface.options[index];
                    let before = (*shown, *level);
                    let mut info_clicked = false;
                    row.col(|ui| {
                        if !surface.enabled {
                            ui.disable();
                        }
                        ui.checkbox(shown, "").on_hover_text(&layer.name);
                    });
                    row.col(|ui| {
                        if !surface.enabled {
                            ui.disable();
                        }
                        ui.add_space(LAYER_INDENT);
                        ui.add(egui::Label::new(&layer.name).truncate().selectable(false))
                            .on_hover_text(&layer.name);
                    });
                    let mut lod_clicked = false;
                    row.col(|ui| {
                        info_clicked = ui
                            .menu_button("i", |ui| {
                                ui.strong(&layer.name);
                                surface
                                    .layer_stats
                                    .entry((set.key, layer.key))
                                    .or_insert_with(|| {
                                        LayerStats::load(
                                            ui.ctx().clone(),
                                            surface.source.path.clone(),
                                            set.key,
                                            layer.key,
                                        )
                                    })
                                    .show(ui);
                            })
                            .response
                            .on_hover_text("レイヤーの収録情報")
                            .clicked();
                        if !surface.enabled {
                            ui.disable();
                        }
                        let selected_z = level.or_else(|| auto_level(&layer.levels, scale));
                        let label = selected_z.map_or_else(|| "—".to_owned(), |z| format!("z{z}"));
                        let current = if level.is_some() {
                            label
                        } else {
                            format!("自動 ({label})")
                        };
                        lod_clicked = egui::ComboBox::from_id_salt((
                            "surface-lod",
                            &surface.source.identity,
                            set.key,
                            layer.key,
                        ))
                        .width(LOD_WIDTH - 4.)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(level, None, "自動");
                            for l in &layer.levels {
                                ui.selectable_value(level, Some(l.z), format!("z{}", l.z));
                            }
                        })
                        .response
                        .clicked();
                    });
                    if surface.enabled
                        && row.response().clicked()
                        && !lod_clicked
                        && !info_clicked
                        && before == (*shown, *level)
                    {
                        *shown = !*shown;
                    }
                    if before != (*shown, *level) {
                        surface.displayed = None;
                    }
                }
                Row::Error(file) => {
                    let error = surfaces[file]
                        .error
                        .as_deref()
                        .expect("地表ファイルのエラー");
                    row.col(|_| {});
                    row.col(|ui| {
                        if !surfaces[file].enabled {
                            ui.disable();
                        }
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(error).color(ui.visuals().error_fg_color),
                            )
                            .truncate(),
                        )
                        .on_hover_text(error);
                    });
                    row.col(|_| {});
                }
            });
        });
    if let Some((file, dataset)) = change_dataset {
        let surface = &mut surfaces[file];
        surface.dataset = dataset;
        surface.options = Surface::default_options(&surface.source.datasets[dataset]);
        surface.displayed = None;
        surface.fit = true;
        surface.error = None;
    }
    if previous_online != (*map_source, *online_enabled) {
        ui.ctx().request_repaint();
    }
    close
}

pub fn check(path: &Path) -> Result<()> {
    let source = Source::open(path.to_owned())?;
    let conn = db::open(path)?;
    for set in source.datasets {
        let mut rendered = 0;
        let mut statement = conn.prepare(
            "SELECT z,x,y FROM surface_tiles WHERE dataset_key=?1 GROUP BY z ORDER BY z",
        )?;
        for tile in statement.query_map([set.key], |row| {
            Ok(Tile {
                z: row.get(0)?,
                x: row.get(1)?,
                y: row.get(2)?,
            })
        })? {
            let tile = tile?;
            let levels: Vec<_> = set
                .layers
                .iter()
                .map(|layer| auto_level(&layer.levels, f64::from(1u32 << tile.z) * 384.))
                .collect();
            let auto_levels = vec![true; levels.len()];
            if composite(&conn, &set, tile, 384, &levels, false, &auto_levels)?
                .image
                .is_some()
            {
                rendered += 1;
            }
        }
        ensure!(rendered > 0, "描画する地表タイルがありません");
        println!(
            "{}: {rendered} レベルの画像を読み込み・合成しました",
            set.label
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    #[test]
    fn lod_accounts_for_the_stored_pixel_dimensions() {
        let levels = [
            SurfaceLevel {
                z: 8,
                lod: "L4".into(),
                pixels: 512,
            },
            SurfaceLevel {
                z: 9,
                lod: "L3".into(),
                pixels: 384,
            },
        ];
        assert_eq!(auto_level(&levels, 130000.), Some(8));
        assert_eq!(auto_level(&levels, 140000.), Some(9));
    }

    #[test]
    fn partial_water_is_composited_over_shaded_land_once() {
        let land = premultiply([100, 100, 100, 255], 1.);
        let water = premultiply([60, 100, 140, 255], 0.5);
        assert_eq!(over(water, land), [80., 100., 120., 1.]);
    }

    #[test]
    fn skips_wrong_roles_for_reserved_ids_and_keeps_generic_layers() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("layers.sqlite3");
        let conn = Connection::open(&path)?;
        db::initialize(&conn)?;
        conn.execute_batch(
            r#"
            INSERT INTO datasets VALUES (1,'test','surface','地表','1',1,'{}');
            INSERT INTO surface_layers VALUES
                (1,1,'forest','rgb',0,'[]','{}'),
                (1,2,'water','hillshade',10,'[]','{}'),
                (1,3,'hillshade','mask',20,'[]','{}'),
                (1,4,'Forest','mask',30,'[]','{"color":[0,140,0,255]}');
            INSERT INTO surface_levels VALUES (1,4,0,'L0',1);
        "#,
        )?;
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.write_header()?.write_image_data(&[255])?;
        }
        conn.execute("INSERT INTO surface_tiles VALUES (1,4,0,0,0,1,?1)", [png])?;

        let source = Source::open(path)?;
        let [layer] = source.datasets[0].layers.as_slice() else {
            panic!("汎用のマスクだけを読み込む");
        };
        assert_eq!(layer.name, "Forest");
        assert!(matches!(layer.role, Role::Mask));
        Ok(())
    }

    #[test]
    fn esa_ground_color_stays_inside_generated_work_areas() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("esa.sqlite3");
        let conn = Connection::open(&path)?;
        db::initialize(&conn)?;
        conn.execute_batch(
            r#"INSERT INTO datasets VALUES (1,'esa-worldcover-japan','surface','地表','1',1,'{}');
               INSERT INTO surface_layers VALUES
                 (1,1,'forest','mask',0,'[]','{"color":[0,140,0,255],"background":[200,200,200,255]}'),
                 (1,2,'water','mask',20,'[]','{"color":[30,90,150,255]}');
               INSERT INTO surface_levels VALUES
                 (1,1,7,'z7',2), (1,1,8,'z8',2),
                 (1,2,7,'z7',2), (1,2,8,'z8',2);"#,
        )?;
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 2, 2);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.write_header()?.write_image_data(&[0; 4])?;
        }
        for layer in [1, 2] {
            for z in [7, 8] {
                conn.execute(
                    "INSERT INTO surface_tiles VALUES(1,?1,?2,0,0,1,?3)",
                    (layer, z, &png),
                )?;
            }
        }
        let source = Source::open(path)?;
        let dataset = &source.datasets[0];
        let tile = Tile { z: 7, x: 0, y: 0 };
        let result = composite(&conn, dataset, tile, 2, &[Some(7), Some(7)], false, &[])?;
        assert_eq!(
            result.image.context("描画タイル")?.pixels,
            [
                egui::Color32::from_gray(200),
                egui::Color32::TRANSPARENT,
                egui::Color32::TRANSPARENT,
                egui::Color32::TRANSPARENT,
            ]
        );
        let outside = Tile { z: 8, x: 1, y: 0 };
        assert!(
            composite(&conn, dataset, outside, 2, &[Some(7), Some(8)], false, &[])?
                .image
                .is_none()
        );
        conn.execute(
            "UPDATE surface_layers SET settings_json=json_set(settings_json,'$.completeTileExtent',json('true')) WHERE layer_key=1",
            [],
        )?;
        let complete = Source::open(source.path.clone())?;
        let result = composite(
            &conn,
            &complete.datasets[0],
            outside,
            2,
            &[Some(7), Some(8)],
            false,
            &[],
        )?;
        assert_eq!(
            result.image.context("生成済みの粗いタイル")?.pixels,
            vec![egui::Color32::from_gray(200); 4]
        );
        let complete = &complete.datasets[0];
        let at = |tile, auto_levels| {
            composite(
                &conn,
                complete,
                tile,
                2,
                &[Some(8), Some(8)],
                false,
                auto_levels,
            )
        };
        assert!(at(outside, &[])?.image.is_none());
        assert_eq!(
            at(outside, &[true, true])?
                .image
                .context("粗いタイルにフォールバック")?
                .pixels,
            vec![egui::Color32::from_gray(200); 4],
        );
        let beyond = Tile { z: 8, x: 2, y: 0 };
        assert!(at(beyond, &[true, true])?.image.is_none());
        Ok(())
    }

    #[test]
    fn composites_mixed_lods_and_ignores_legacy_coverage() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ground.sqlite3");
        let conn = Connection::open(&path)?;
        db::initialize(&conn)?;
        conn.execute_batch(r#"
            INSERT INTO datasets VALUES (1,'test','surface','地表','1',1,'{"authors":[{"name":"テスト作成者"}]}');
            INSERT INTO surface_layers VALUES
                (1,1,'forest','mask',0,'[]','{"color":[0,140,0,255],"background":[200,200,200,255],"coverageLayer":"missing"}'),
                (1,2,'hillshade','hillshade',10,'[]','{"neutral":136,"strength":1}'),
                (1,3,'water','mask',20,'[]','{"color":[30,90,150,255]}'),
                (1,4,'missing','coverage',0,'[]','{"appliesTo":"forest","whiteMeansMissing":true}');
            INSERT INTO surface_levels VALUES (1,1,0,'L0',2),(1,2,0,'L0',2),(1,3,1,'L1',2),(1,3,0,'L0',2),(1,4,0,'L0',2);
        "#)?;
        let mut water_bytes = 0;
        for (key, z, values) in [
            (1, 0, [0, 0, 255, 255]),
            (2, 0, [68, 68, 136, 136]),
            (3, 1, [255; 4]),
            (4, 0, [0, 255, 0, 0]),
        ] {
            let mut png = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut png, 2, 2);
                encoder.set_color(png::ColorType::Grayscale);
                encoder.write_header()?.write_image_data(&values)?;
            }
            if key == 3 {
                water_bytes = png.len() as u64;
            }
            conn.execute(
                "INSERT INTO surface_tiles VALUES (1,?1,?2,0,0,1,?3)",
                (key, z, png),
            )?;
        }
        drop(conn);
        let stats = read_layer_stats(&path, 1, 3)?;
        assert_eq!(
            stats
                .iter()
                .map(|l| (l.z, l.pixels, l.tiles, l.bytes))
                .collect::<Vec<_>>(),
            [(1, 2, 1, water_bytes), (0, 2, 0, 0)]
        );
        let source = Source::open(path.clone())?;
        let set = &source.datasets[0];
        assert_eq!(
            set.layers
                .iter()
                .map(|layer| layer.name.as_str())
                .collect::<Vec<_>>(),
            ["森林", "陰影", "水域"]
        );
        let conn = db::open(&path)?;
        let image = composite(
            &conn,
            set,
            Tile { z: 0, x: 0, y: 0 },
            2,
            &[Some(0), Some(0), Some(1)],
            false,
            &[],
        )?
        .image
        .context("地表が透明です")?;
        assert_eq!(
            image.pixels,
            [
                egui::Color32::from_rgb(30, 90, 150),
                egui::Color32::from_rgb(100, 100, 100),
                egui::Color32::from_rgb(0, 140, 0),
                egui::Color32::from_rgb(0, 140, 0),
            ]
        );
        let overlay = composite(
            &conn,
            set,
            Tile { z: 0, x: 0, y: 0 },
            2,
            &[Some(0), Some(0), Some(1)],
            true,
            &[],
        )?;
        assert_eq!(overlay.shades.len(), 1);
        assert_eq!(
            overlay.shades[0].image.pixels,
            [68, 68, 136, 136].map(egui::Color32::from_gray)
        );
        assert_eq!(
            overlay.image.context("地表の重ね表示が透明です")?.pixels,
            [
                egui::Color32::from_rgb(30, 90, 150),
                egui::Color32::TRANSPARENT,
                egui::Color32::from_rgb(0, 140, 0),
                egui::Color32::from_rgb(0, 140, 0),
            ]
        );
        let water_overlay = composite(
            &conn,
            set,
            Tile { z: 0, x: 0, y: 0 },
            2,
            &[None, Some(0), Some(1)],
            true,
            &[],
        )?;
        assert_eq!(water_overlay.shades.len(), 1);
        assert_eq!(
            water_overlay.image.context("水域が透明です")?.pixels,
            [
                egui::Color32::from_rgb(30, 90, 150),
                egui::Color32::TRANSPARENT,
                egui::Color32::TRANSPARENT,
                egui::Color32::TRANSPARENT,
            ]
        );
        let shade_only = composite(
            &conn,
            set,
            Tile { z: 0, x: 0, y: 0 },
            2,
            &[None, Some(0), None],
            false,
            &[],
        )?
        .image
        .context("陰影単独表示が透明です")?;
        assert_eq!(
            shade_only.pixels,
            [68, 68, 136, 136].map(egui::Color32::from_gray)
        );

        let ctx = egui::Context::default();
        let second_path = temp.path().join("another.sqlite3");
        std::fs::copy(&path, &second_path)?;
        Connection::open(&second_path)?
            .execute("UPDATE datasets SET title = ?1", ["追加の地表"])?;
        let client = reqwest::blocking::Client::builder().build()?;
        let mut viewer = crate::Viewer::new(&ctx, client, None);
        for input in [&path, &second_path, &path] {
            viewer.open_surface(&ctx, input.clone());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while viewer.loading_surface.is_some() {
                viewer.poll_surface(&ctx);
                assert!(
                    std::time::Instant::now() < deadline,
                    "SQLiteの読み込みが終わらない"
                );
                std::thread::yield_now();
            }
            assert!(viewer.error.is_none());
        }
        assert_eq!(viewer.surfaces.len(), 2);
        let mut surfaces = viewer.surfaces;
        assert!(surfaces[0].fit);
        assert!(!surfaces[1].fit);
        for surface in &mut surfaces {
            surface.reveal = false;
        }
        let mut map = MapSource::Gsi;
        let mut online_enabled = true;
        let mut frame = |events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 850.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::SidePanel::left("layers")
                        .default_width(370.)
                        .show(ctx, |ui| {
                            layer_list(
                                ui,
                                &mut surfaces,
                                None,
                                &mut map,
                                &mut online_enabled,
                                8.,
                                65536.,
                                f32::INFINITY,
                            );
                        });
                    for surface in &mut surfaces {
                        surface.show_file_info(ctx);
                    }
                },
            )
        };
        frame(Vec::new());
        let output = frame(Vec::new());
        assert!(output.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text)
                if text.galley.text() == "自動 (z1)")
        }));
        let mut previous_y = f32::NEG_INFINITY;
        let mut category_x = None;
        for (label, is_category) in [
            ("オンライン地図", true),
            ("国土地理院", true),
            ("傾斜量図", false),
            ("OpenStreetMap系", true),
            ("地形図（OpenTopoMap）", false),
            ("地表 (1)", true),
            ("森林", false),
            ("陰影", false),
            ("水域", false),
            ("追加の地表 (1)", true),
        ] {
            let pos = output
                .shapes
                .iter()
                .find_map(|clipped| {
                    let egui::Shape::Text(text) = &clipped.shape else {
                        return None;
                    };
                    let center = text.visual_bounding_rect().center();
                    (text.galley.text() == label && clipped.clip_rect.contains(center))
                        .then_some(egui::pos2(text.pos.x, center.y))
                })
                .unwrap_or_else(|| panic!("{label} が一覧の表示範囲にない"));
            assert!(previous_y < pos.y, "{label} の表示順が違う");
            previous_y = pos.y;
            if is_category {
                category_x = Some(pos.x);
            } else if let Some(x) = category_x {
                assert!(pos.x - x >= 12., "{label} が字下げされていない");
            }
        }
        let forest_row = output
            .shapes
            .iter()
            .find_map(|clipped| {
                let egui::Shape::Text(text) = &clipped.shape else {
                    return None;
                };
                (text.galley.text() == "森林").then(|| {
                    egui::pos2(
                        clipped.clip_rect.right() - 12.,
                        text.visual_bounding_rect().center().y,
                    )
                })
            })
            .expect("森林の行");
        let find_text = |output: &egui::FullOutput, label| {
            output.shapes.iter().find_map(|clipped| {
                let egui::Shape::Text(text) = &clipped.shape else {
                    return None;
                };
                let center = text.visual_bounding_rect().center();
                (text.galley.text() == label && clipped.clip_rect.contains(center))
                    .then_some(center)
            })
        };
        assert!(find_text(&output, "ground.sqlite3").is_none());
        assert!(find_text(&output, "地表SQLite").is_none());
        let file_name_row = find_text(&output, "地表 (1)").expect("データセット名");
        let info_at = |row_y: f32| {
            output.shapes.iter().find_map(|clipped| {
                let egui::Shape::Text(text) = &clipped.shape else {
                    return None;
                };
                let center = text.visual_bounding_rect().center();
                (text.galley.text() == "i"
                    && (center.y - row_y).abs() < 5.
                    && clipped.clip_rect.contains(center))
                .then_some(center)
            })
        };
        let info_button = info_at(file_name_row.y).expect("ファイルの情報ボタン");
        let layer_info = info_at(forest_row.y).expect("森林レイヤーの情報ボタン");
        assert!(
            output.shapes.iter().any(|clipped| {
                let egui::Shape::Text(text) = &clipped.shape else {
                    return false;
                };
                let center = text.visual_bounding_rect().center();
                text.galley.text() == "自動 (z0)"
                    && (center.y - layer_info.y).abs() < 5.
                    && text.pos.x > layer_info.x
            }),
            "情報ボタンの右にLODがある"
        );
        for pos in [forest_row, file_name_row, layer_info] {
            frame(vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while find_text(&frame(Vec::new()), "圧縮データ").is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "レイヤーの集計が表示されない"
            );
            std::thread::yield_now();
        }
        frame(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        for pos in [forest_row, info_button] {
            frame(vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
        }
        let popup = frame(Vec::new());
        assert!(find_text(&popup, "ground.sqlite3").is_some());
        assert!(find_text(&popup, "データセットID test").is_some());
        assert!(find_text(&popup, "another.sqlite3").is_none());
        assert!(popup.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text().contains("テスト作成者"))
        }));
        let forest_color = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .find_map(|clipped| {
                    let egui::Shape::Text(text) = &clipped.shape else {
                        return None;
                    };
                    (text.galley.text() == "森林").then_some(text.fallback_color)
                })
                .expect("森林の文字色")
        };
        assert_ne!(forest_color(&output), forest_color(&popup));
        assert!(!surfaces[0].enabled && surfaces[1].enabled);
        assert!(
            !surfaces[0].options[0].0,
            "先に開いたSQLiteの森林を非表示にできる"
        );
        assert!(surfaces[1].options[0].0, "別ファイルの森林は表示したまま");
        Ok(())
    }
}
