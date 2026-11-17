//! 共通SQLiteの地表を読み、可視タイルだけを背景で合成する。
use crate::map::{Lod, Source as MapSource, TILE_SIZE, Tile, View};
use crate::{lod_fade, surface_palette::Palette, surface_shade};
use anyhow::{Result, bail, ensure};
use eframe::{
    egui::{self, ColorImage, Pos2, Rect},
    egui_wgpu::wgpu,
};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};
use yamamuki_data_sqlite::{self as db, SurfaceLevel, rusqlite::Connection};
use yamamuki_terrain_view::direct_material::Interpolation;

const PENDING: usize = 12;
const CACHE_BYTES: usize = 192 * 1024 * 1024;
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

fn equivalent_zoom(z: u32, pixels: u32) -> f64 {
    f64::from(z) + (f64::from(pixels) / TILE_SIZE as f64).log2()
}

fn auto_level(levels: &[SurfaceLevel], zoom: f64, lod: Lod) -> Option<u32> {
    let target = lod.target_zoom(zoom);
    let by_density =
        |a: &(u32, f64), b: &(u32, f64)| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0));
    let allowed = levels
        .iter()
        .filter(|level| lod.allows(level.z))
        .map(|level| (level.z, equivalent_zoom(level.z, level.pixels)));
    allowed
        .clone()
        .filter(|&(_, equivalent)| equivalent <= target)
        .max_by(by_density)
        .or_else(|| allowed.min_by(by_density))
        .map(|(z, _)| z)
        .or_else(|| levels.iter().map(|level| level.z).min())
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

fn samples_with_gutter(
    index: u32,
    dst_z: u32,
    src_z: u32,
    pixels: usize,
    gutter: usize,
) -> Vec<Sample> {
    let scale = 2f64.powi(src_z as i32 - dst_z as i32);
    let edge = 0.5 / pixels as f64 * scale;
    (-(gutter as i32)..pixels as i32 + gutter as i32)
        .map(|i| {
            let position = ((f64::from(index) + (f64::from(i) + 0.5) / pixels as f64) * scale)
                .clamp(edge, f64::from(1u32 << src_z) - edge);
            Sample {
                tile: position.floor() as u32,
                local: position.fract(),
            }
        })
        .collect()
}

fn surface_uv_with_gutter(pixels: usize, gutter: usize) -> Rect {
    let inset = gutter as f32 / (pixels + gutter * 2) as f32;
    Rect::from_min_max(egui::pos2(inset, inset), egui::pos2(1. - inset, 1. - inset))
}

// 描画タイルの1画素の余白で共有する、復号済み元タイルの作業キャッシュ。
#[derive(Default)]
struct DecodedTiles {
    gutter: usize,
    palette: Palette,
    images: HashMap<(i64, i64, Tile), Option<Arc<image::RgbaImage>>>,
    order: VecDeque<(i64, i64, Tile)>,
}

impl DecodedTiles {
    fn get(
        &mut self,
        conn: &Connection,
        dataset: i64,
        layer: i64,
        tile: Tile,
    ) -> Result<Option<Arc<image::RgbaImage>>> {
        let key = (dataset, layer, tile);
        if let Some(image) = self.images.get(&key) {
            return Ok(image.clone());
        }
        let image = db::surface_tile(conn, dataset, layer, tile.z, tile.x, tile.y)?
            .map(|image| Arc::new(image.into_rgba8()));
        // 大きな汎用タイルの復号結果は作業キャッシュに保持しない。
        if image.as_ref().is_none_or(|image| image.width() <= 512) {
            if self.order.len() == 32 {
                if let Some(oldest) = self.order.pop_front() {
                    self.images.remove(&oldest);
                }
            }
            self.images.insert(key, image.clone());
            self.order.push_back(key);
        }
        Ok(image)
    }
}

struct ShadeImage {
    layer: usize,
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
    composite_cached(
        conn,
        set,
        tile,
        pixels,
        levels,
        overlay,
        auto_levels,
        &mut DecodedTiles::default(),
    )
}

fn composite_cached(
    conn: &Connection,
    set: &Dataset,
    tile: Tile,
    pixels: usize,
    levels: &[Option<u32>],
    overlay: bool,
    auto_levels: &[bool],
    decoded: &mut DecodedTiles,
) -> Result<ComposedTile> {
    let gutter = decoded.gutter.max(1);
    let side = pixels + gutter * 2;
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
    let mut colors = vec![[0.; 4]; side * side];
    let mut has_base = false;
    for (index, (layer, &z)) in set.layers.iter().zip(levels).enumerate() {
        let Some(z) = z else { continue };
        let fallback = set.complete_tile_extent && auto_levels.get(index) == Some(&true);
        let is_shade = matches!(layer.role, Role::Hillshade);
        let mask_color = decoded.palette.mask(&layer.id).or(layer.settings.color);
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
                .map(|color| decoded.palette.background(&layer.id).unwrap_or(color))
                .map_or([0.; 4], |color| premultiply(color, 1.))
        };
        let xs = samples_with_gutter(tile.x, tile.z, z, pixels, gutter);
        let ys = samples_with_gutter(tile.y, tile.z, z, pixels, gutter);
        let mut row = 0;
        for ys in ys.chunk_by(|a, b| a.tile == b.tile) {
            let mut col = 0;
            for xs in xs.chunk_by(|a, b| a.tile == b.tile) {
                let x = xs[0].tile;
                let y = ys[0].tile;
                let mut source_z = z;
                let mut image = decoded.get(conn, set.key, layer.key, Tile { z, x, y })?;
                if fallback && image.is_none() {
                    for level in layer.levels.iter().rev().filter(|level| level.z < z) {
                        let shift = z - level.z;
                        image = decoded.get(
                            conn,
                            set.key,
                            layer.key,
                            Tile {
                                z: level.z,
                                x: x >> shift,
                                y: y >> shift,
                            },
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
                                layer: index,
                                image: ColorImage::new([side, side], egui::Color32::TRANSPARENT),
                                neutral: layer.settings.neutral.expect("hillshade neutral"),
                                strength: layer.settings.strength.expect("hillshade strength"),
                            })
                            .image
                            .pixels
                    });
                    let size = image.width() as i32;
                    let factor = 1u32 << (z - source_z);
                    let world_side = 1 << source_z;
                    let aligned = source_z == tile.z && z == source_z && size as usize == pixels;
                    let index = |s: &Sample| {
                        let local = (f64::from(s.tile % factor) + s.local) / f64::from(factor);
                        let coord = local * f64::from(size) - 0.5;
                        let source_tile = s.tile / factor;
                        let coord = if source_tile == 0 && coord < 0. {
                            0.
                        } else if source_tile + 1 == world_side {
                            coord.min(f64::from(size - 1))
                        } else {
                            coord
                        };
                        if aligned {
                            (coord.round() as i32, 0.)
                        } else {
                            let first = coord.floor() as i32;
                            (first, (coord - f64::from(first)) as f32)
                        }
                    };
                    let xs: Vec<_> = xs.iter().map(index).collect();
                    let ys: Vec<_> = ys.iter().map(index).collect();
                    let offsets = |axis: &[(i32, f32)]| {
                        let (last, fraction) = axis[axis.len() - 1];
                        [axis[0].0 < 0, true, last >= size - 1 && fraction > 0.]
                    };
                    let mut neighbors: [Option<Arc<image::RgbaImage>>; 9] = Default::default();
                    if !aligned {
                        let offsets_x = offsets(&xs);
                        let offsets_y = offsets(&ys);
                        for (iy, enabled_y) in offsets_y.into_iter().enumerate() {
                            for (ix, enabled_x) in offsets_x.into_iter().enumerate() {
                                if !(enabled_x && enabled_y) || (ix == 1 && iy == 1) {
                                    continue;
                                }
                                let nx = (x / factor).checked_add_signed(ix as i32 - 1);
                                let ny = (y / factor).checked_add_signed(iy as i32 - 1);
                                if let (Some(nx), Some(ny)) = (nx, ny)
                                    && nx < world_side
                                    && ny < world_side
                                {
                                    neighbors[iy * 3 + ix] = decoded.get(
                                        conn,
                                        set.key,
                                        layer.key,
                                        Tile {
                                            z: source_z,
                                            x: nx,
                                            y: ny,
                                        },
                                    )?;
                                }
                            }
                        }
                    }
                    let sample = |ix: i32, iy: i32| {
                        let v = if (0..size).contains(&ix) && (0..size).contains(&iy) {
                            image.get_pixel(ix as u32, iy as u32).0
                        } else {
                            let nx = (ix.div_euclid(size) + 1) as usize;
                            let ny = (iy.div_euclid(size) + 1) as usize;
                            if let Some(neighbor) = &neighbors[ny * 3 + nx] {
                                neighbor
                                    .get_pixel(
                                        ix.rem_euclid(size) as u32,
                                        iy.rem_euclid(size) as u32,
                                    )
                                    .0
                            } else {
                                image
                                    .get_pixel(
                                        ix.clamp(0, size - 1) as u32,
                                        iy.clamp(0, size - 1) as u32,
                                    )
                                    .0
                            }
                        };
                        match layer.role {
                            Role::Rgb => premultiply(v, 1.),
                            Role::Mask => over(
                                premultiply(
                                    mask_color.expect("mask color"),
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
                    for (j, &(y0, fy)) in ys.iter().enumerate() {
                        for (i, &(x0, fx)) in xs.iter().enumerate() {
                            let v = if aligned {
                                sample(x0, y0)
                            } else {
                                let x1 = x0 + i32::from(fx > 0.);
                                let y1 = y0 + i32::from(fy > 0.);
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
                            let index = (row + j) * side + col + i;
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
    if let Some(areas) = work_areas {
        let xs = samples_with_gutter(tile.x, tile.z, ESA_WORK_ZOOM, pixels, gutter);
        let ys = samples_with_gutter(tile.y, tile.z, ESA_WORK_ZOOM, pixels, gutter);
        let mut row = 0;
        for ys in ys.chunk_by(|a, b| a.tile == b.tile) {
            let mut col = 0;
            for xs in xs.chunk_by(|a, b| a.tile == b.tile) {
                if !areas.contains(&(xs[0].tile, ys[0].tile)) {
                    for y in row..row + ys.len() {
                        let start = y * side + col;
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
    tile_images.shades.retain(|shade| {
        shade
            .image
            .pixels
            .chunks_exact(side)
            .skip(gutter)
            .take(pixels)
            .any(|row| {
                row[gutter..gutter + pixels]
                    .iter()
                    .any(|pixel| pixel.a() != 0)
            })
    });
    if colors
        .chunks_exact(side)
        .skip(gutter)
        .take(pixels)
        .all(|row| row[gutter..gutter + pixels].iter().all(|v| v[3] == 0.))
    {
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
    tile_images.image = Some(ColorImage::from_rgba_unmultiplied([side, side], &rgba));
    Ok(tile_images)
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Key {
    gutter: usize,
    dataset: usize,
    tile: Tile,
    pixels: usize,
    levels: Vec<Option<u32>>,
    auto_levels: Vec<bool>,
    overlay: bool,
    palette: Palette,
}

impl Key {
    // 同じレイヤー・配色・合成方法の旧画像だけを切替元に使う。
    fn same_layer(&self, other: &Self) -> bool {
        self.dataset == other.dataset
            && self.overlay == other.overlay
            && self.palette == other.palette
            && self
                .levels
                .iter()
                .map(Option::is_some)
                .eq(other.levels.iter().map(Option::is_some))
    }

    fn layer_keys(&self) -> Vec<(usize, Self)> {
        let mut base = self.overlay;
        self.levels
            .iter()
            .enumerate()
            .filter_map(|(layer, level)| {
                level.as_ref()?;
                let mut key = self.clone();
                for index in 0..key.levels.len() {
                    if index != layer {
                        key.levels[index] = None;
                        key.auto_levels[index] = false;
                    }
                }
                key.overlay = base;
                base = true;
                Some((layer, key))
            })
            .collect()
    }

    fn covers(&self, tile: Tile) -> bool {
        self.tile.z <= tile.z
            && lod_fade::sample(tile, self.tile.z, lod_fade::FULL_UV).0 == self.tile
    }
}

type Reply = (Key, Result<ComposedTile>);

struct Cached {
    image: Option<crate::tile_atlas::Image>,
    shades: Vec<(usize, surface_shade::ShadeTexture)>,
    bytes: usize,
    touched: u64,
    failed: bool,
}

impl Cached {
    fn collect(
        &self,
        view: &View,
        rect: Rect,
        key: &Key,
        region: Tile,
        images: &mut Vec<(Tile, Rect, egui::TextureId)>,
        shades: &mut Vec<([Pos2; 4], Rect, wgpu::BindGroup, wgpu::TextureView)>,
    ) -> bool {
        let uv = lod_fade::sample(
            region,
            key.tile.z,
            surface_uv_with_gutter(key.pixels, key.gutter),
        )
        .1;
        if let Some(image) = &self.image {
            images.push((region, image.uv(uv), image.id()));
        }
        if !self.shades.is_empty() {
            let corners = view.tile_corners(rect, region);
            shades.extend(
                self.shades
                    .iter()
                    .map(|(_, shade)| (corners, uv, shade.bind_group.clone(), shade.view.clone())),
            );
        }
        self.image.is_some() || !self.shades.is_empty()
    }
}

// 同じ地表タイルのレイヤーをまとめて残し、占有量の多いレベルから整理する。
fn trim_surface_cache(
    cache: &mut HashMap<Key, Cached>,
    bytes: &mut usize,
    byte_limit: usize,
    count_limit: usize,
    frame: u64,
) {
    let mut groups = HashMap::<Key, (Vec<Key>, u64)>::new();
    for (key, entry) in cache.iter() {
        let mut group = key.clone();
        group.levels.clear();
        group.auto_levels.clear();
        group.overlay = false;
        let group = groups.entry(group).or_default();
        group.0.push(key.clone());
        group.1 = group.1.max(entry.touched);
    }
    let mut candidates = groups
        .into_iter()
        .filter(|(_, (_, touched))| *touched != frame)
        .collect::<Vec<_>>();
    // 表示中の作業領域と、ズームアウト用に残す画像の保持枠を分ける。
    // 作業領域だけで上限を超えても、保持画像を全削除してはならない。
    let mut sizes = HashMap::<u32, usize>::new();
    let mut counts = HashMap::<u32, usize>::new();
    let mut retained_bytes = 0;
    let mut retained_count = 0;
    for (group, (keys, _)) in &candidates {
        let size: usize = keys.iter().map(|key| cache[key].bytes).sum();
        *sizes.entry(group.tile.z).or_default() += size;
        *counts.entry(group.tile.z).or_default() += keys.len();
        retained_bytes += size;
        retained_count += keys.len();
    }
    while retained_bytes > byte_limit || retained_count > count_limit {
        let Some((index, _)) =
            candidates
                .iter()
                .enumerate()
                .max_by_key(|(_, (key, (_, touched)))| {
                    (
                        sizes[&key.tile.z],
                        counts[&key.tile.z],
                        std::cmp::Reverse(*touched),
                        key.tile.z,
                    )
                })
        else {
            break;
        };
        let (group, (keys, _)) = candidates.swap_remove(index);
        for key in keys {
            let removed = cache.remove(&key).unwrap();
            *bytes -= removed.bytes;
            retained_bytes -= removed.bytes;
            retained_count -= 1;
            *sizes.get_mut(&group.tile.z).unwrap() -= removed.bytes;
            *counts.get_mut(&group.tile.z).unwrap() -= 1;
        }
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
                        for title in ["レベル", "タイル寸法", "地図換算", "タイル数", "圧縮データ"]
                        {
                            ui.strong(title);
                        }
                        ui.end_row();
                        for level in levels.iter() {
                            ui.label(format!("z{}", level.z));
                            ui.label(format!("{} × {}", level.pixels, level.pixels));
                            ui.label(format!("z{:.2}", equivalent_zoom(level.z, level.pixels)));
                            ui.label(level.tiles.to_string());
                            size(ui, level.bytes);
                            ui.end_row();
                        }
                        ui.strong("合計");
                        ui.label("—");
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
                ui.small("地図換算は256画素タイルで同じ画素密度になるズームです。容量は画像BLOBの合計です。");
            }
        }
    }
}

pub struct Surface {
    pub animate_lod: bool,
    pub edge_filter: Interpolation,
    tile_lods: HashMap<usize, HashMap<Tile, lod_fade::State<Key>>>,
    pub borders: bool,
    source: Source,
    layer_stats: HashMap<(i64, i64), LayerStats>,
    dataset: usize,
    show_file_info: bool,
    options: Vec<(bool, Option<u32>)>,
    pub enabled: bool,
    pub palette: Palette,
    pub fit: bool,
    pub reveal: bool,
    jobs: mpsc::Sender<Key>,
    replies: mpsc::Receiver<Reply>,
    pending: HashSet<Key>,
    cache: HashMap<Key, Cached>,
    cached_tiles: HashMap<Key, u64>,
    cache_bytes: usize,
    frame: u64,
    material_pass: usize,
    shade_renderer: Option<surface_shade::Renderer>,
    error: Option<String>,
}

impl Surface {
    pub(crate) fn snapshot(&self) -> crate::snapshot::SurfaceState {
        crate::snapshot::SurfaceState {
            path: self.source.path.clone(),
            dataset: self.dataset,
            enabled: self.enabled,
            options: self.options.clone(),
            palette: self.palette,
        }
    }
    pub(crate) fn restore_options(&mut self, s: &crate::snapshot::SurfaceState) -> Result<()> {
        anyhow::ensure!(
            s.dataset < self.source.datasets.len(),
            "地表データセット構成が変わっています"
        );
        anyhow::ensure!(
            s.options.len() == self.source.datasets[s.dataset].layers.len(),
            "地表レイヤー構成が変わっています"
        );
        self.dataset = s.dataset;
        self.enabled = s.enabled;
        self.options = s.options.clone();
        self.palette = s.palette;
        self.fit = false;
        Ok(())
    }
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
                let mut decoded = DecodedTiles::default();
                loop {
                    let job = incoming.lock().expect("surface queue").recv();
                    let Ok(job) = job else { break };
                    decoded.palette = job.palette;
                    decoded.gutter = job.gutter;
                    let result = match &conn {
                        Ok(conn) => composite_cached(
                            conn,
                            &datasets[job.dataset],
                            job.tile,
                            job.pixels,
                            &job.levels,
                            job.overlay,
                            &job.auto_levels,
                            &mut decoded,
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
            edge_filter: Interpolation::Bilinear,
            animate_lod: true,
            tile_lods: HashMap::new(),
            borders: false,
            source,
            layer_stats: HashMap::new(),
            dataset: 0,
            show_file_info: false,
            options,
            enabled: true,
            palette: Palette::default(),
            fit: true,
            reveal: false,
            jobs,
            replies: outgoing,
            pending: HashSet::new(),
            cache: HashMap::new(),
            cached_tiles: HashMap::new(),
            cache_bytes: 0,
            frame: 0,
            material_pass: 0,
            shade_renderer: None,
            error: None,
        }
    }

    fn default_options(set: &Dataset) -> Vec<(bool, Option<u32>)> {
        vec![(true, None); set.layers.len()]
    }

    /// 実際に使える最も細かい画像の画素密度。過拡大で材質の範囲を縮め続けないために使う。
    pub fn material_max_zoom(&self, lod: Lod) -> Option<f64> {
        self.source.datasets[self.dataset]
            .layers
            .iter()
            .zip(&self.options)
            .filter(|(_, option)| option.0)
            .filter_map(|(layer, &(_, fixed))| {
                let selected = layer
                    .levels
                    .iter()
                    .filter(|level| fixed.map_or_else(|| lod.allows(level.z), |z| level.z == z))
                    .map(|level| equivalent_zoom(level.z, level.pixels))
                    .max_by(f64::total_cmp);
                selected.or_else(|| {
                    layer
                        .levels
                        .iter()
                        .min_by_key(|l| l.z)
                        .map(|l| equivalent_zoom(l.z, l.pixels))
                })
            })
            .max_by(f64::total_cmp)
    }
    pub fn same_file(&self, source: &Source) -> bool {
        self.source.identity == source.identity
    }

    pub fn max_visible_level(&self) -> Option<u32> {
        if !self.enabled {
            return None;
        }
        self.source.datasets[self.dataset]
            .layers
            .iter()
            .zip(&self.options)
            .filter(|(_, (shown, _))| *shown)
            .flat_map(|(layer, _)| layer.levels.iter().map(|level| level.z))
            .max()
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

    // 描画履歴にない距離帯・拡大前の画像も、キャッシュから直ちに再利用する。
    fn cached_bundle(&self, region: Tile, grid: &Key, exclude: Option<&Key>) -> Option<Key> {
        self.cached_tiles
            .iter()
            .filter(|(key, _)| {
                exclude != Some(*key)
                    && key.same_layer(grid)
                    && key.covers(region)
                    && key
                        .layer_keys()
                        .iter()
                        .all(|(_, key)| self.cache.get(key).is_some_and(|entry| !entry.failed))
            })
            .max_by_key(|(key, used)| (key.tile.z, **used))
            .map(|(key, _)| key.clone())
    }

    pub fn cache_stats(&self) -> (usize, usize, usize) {
        (
            self.cache
                .values()
                .filter(|entry| entry.image.is_some() || !entry.shades.is_empty())
                .count(),
            self.cache_bytes,
            self.pending.len(),
        )
    }

    pub fn finish_material_frame(&mut self) {
        trim_surface_cache(
            &mut self.cache,
            &mut self.cache_bytes,
            CACHE_BYTES,
            1024,
            self.frame,
        );
        self.cached_tiles.retain(|key, _| {
            key.layer_keys()
                .iter()
                .all(|(_, key)| self.cache.get(key).is_some_and(|entry| !entry.failed))
        });
    }
    pub fn select_material_pass(&mut self, pass: usize) {
        self.material_pass = pass;
    }
    pub fn paint(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        view: &View,
        dragging: bool,
        overlay: bool,
        fade_backend: lod_fade::Backend<'_>,
        lod: Lod,
        lod_zoom: f64,
    ) {
        // 各レイヤーの画像・補間は分離し、切り替え時刻だけをタイル単位で共有する。
        let set = &self.source.datasets[self.dataset];
        self.frame = ui.ctx().cumulative_pass_nr();
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
                        entry.image = Some(crate::tile_atlas::Image::load(
                            ui.ctx(),
                            "surface tile",
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                    if let Some((state, _)) = fade_backend.filter(|_| !shades.is_empty()) {
                        let renderer = self
                            .shade_renderer
                            .get_or_insert_with(|| surface_shade::Renderer::new(state));
                        for shade in shades {
                            entry.bytes += shade.image.pixels.len() * 2;
                            entry.shades.push((
                                shade.layer,
                                renderer.upload(&shade.image, shade.neutral, shade.strength),
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
        // 3Dの収集用座標系には画素数の意味がない。通常の画面と同じ密度で元タイルを保持する。
        let scale = if self.material_pass == 0 {
            view.scale()
        } else {
            256. * lod_zoom.exp2()
        } * f64::from(ui.ctx().pixels_per_point());
        let levels: Vec<_> = set
            .layers
            .iter()
            .zip(&self.options)
            .map(|(l, &(show, z))| {
                if show {
                    z.or_else(|| auto_level(&l.levels, lod_zoom, lod))
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
        if let Some(finest) = levels.iter().flatten().max().copied() {
            let draw_zoom = (scale / 384.).log2().ceil().clamp(0., 30.) as u32;
            let z = finest.min(draw_zoom);
            let pixels = set
                .layers
                .iter()
                .zip(&levels)
                .filter_map(|(layer, z)| {
                    layer
                        .levels
                        .iter()
                        .find(|level| Some(level.z) == *z)
                        .map(|level| level.pixels as usize)
                })
                .max()
                .unwrap_or(256)
                .clamp(256, 512);
            let grid = Key {
                gutter: 3,
                dataset: self.dataset,
                tile: Tile { z, x: 0, y: 0 },
                pixels,
                levels,
                auto_levels,
                palette: self.palette,
                overlay,
            };
            let visible = view.tiles_at(rect, z);
            // レイヤーを一つずつ全域要求せず、同じタイルに必要な画像をまとめて要求する。
            for &tile in &visible {
                let bundle = Key {
                    tile,
                    ..grid.clone()
                };
                for (_, key) in bundle.layer_keys() {
                    if let Some(entry) = self.cache.get_mut(&key) {
                        entry.touched = self.frame;
                        if entry.failed && self.error.is_none() {
                            self.error = Some("地表タイルを読み込めません".into());
                        }
                    } else if self.pending.len() < PENDING
                        && !self.pending.contains(&key)
                        && self.jobs.send(key.clone()).is_ok()
                    {
                        self.pending.insert(key);
                    }
                }
                if bundle
                    .layer_keys()
                    .iter()
                    .all(|(_, key)| self.cache.get(key).is_some_and(|entry| !entry.failed))
                {
                    self.cached_tiles.insert(bundle, self.frame);
                }
            }
            let now = ui.input(|input| input.time);
            let mut previous = self
                .tile_lods
                .remove(&self.material_pass)
                .unwrap_or_default();
            previous.retain(|_, state| {
                state
                    .current
                    .as_ref()
                    .is_some_and(|key| key.same_layer(&grid))
            });
            let details = previous
                .iter()
                .filter_map(|(&tile, state)| {
                    (state.transitioning(now)
                        || state.current.as_ref().is_some_and(|key| key.tile.z > z))
                    .then_some(tile)
                })
                .filter(|tile| tile.z > z);
            let regions = crate::map::transition_regions(visible, details);
            let mut next = HashMap::new();
            let mut colors: Vec<Vec<lod_fade::Quad>> =
                (0..set.layers.len()).map(|_| Vec::new()).collect();
            let mut shades: Vec<Vec<lod_fade::Quad>> =
                (0..set.layers.len()).map(|_| Vec::new()).collect();
            let mut cpu_images = (0..set.layers.len())
                .map(|_| Vec::new())
                .collect::<Vec<_>>();
            let mut cpu_shades = (0..set.layers.len())
                .map(|_| Vec::new())
                .collect::<Vec<_>>();
            let mut borders = Vec::new();
            for region in regions {
                let target = Key {
                    tile: lod_fade::sample(region, z, lod_fade::FULL_UV).0,
                    ..grid.clone()
                };
                let mut transition = (0..=region.z)
                    .rev()
                    .find_map(|level| {
                        previous
                            .get(&lod_fade::sample(region, level, lod_fade::FULL_UV).0)
                            .cloned()
                    })
                    .unwrap_or_default();
                let available = |key: &Key| {
                    key.layer_keys()
                        .iter()
                        .all(|(_, key)| self.cache.get(key).is_some_and(|entry| !entry.failed))
                };
                let desired = if available(&target) {
                    Some(target)
                } else {
                    transition
                        .current
                        .as_ref()
                        .filter(|key| key.covers(region) && available(key))
                        .cloned()
                        .or_else(|| {
                            previous
                                .values()
                                .filter_map(|state| state.current.as_ref())
                                .filter(|key| key.covers(region) && available(key))
                                .max_by_key(|key| key.tile.z)
                                .cloned()
                        })
                        .or_else(|| self.cached_bundle(region, &grid, None))
                };
                let Some(desired) = desired else { continue };
                let initial_fallback = if transition.current.is_none() {
                    self.cached_bundle(region, &grid, Some(&desired))
                } else {
                    None
                };
                if let Some((state, renderer)) = fade_backend {
                    let textures = state.renderer.read();
                    let mut fetch = |key: &Key| {
                        let entry = self.cache.get_mut(key).filter(|entry| !entry.failed)?;
                        entry.touched = self.frame;
                        let uv = lod_fade::sample(
                            region,
                            key.tile.z,
                            surface_uv_with_gutter(key.pixels, key.gutter),
                        )
                        .1;
                        let color = entry.image.as_ref().and_then(|image| {
                            renderer.color_view(&textures, image.id(), self.frame)
                        });
                        let ready = entry.image.is_none() || color.is_some();
                        Some((
                            color,
                            ready,
                            entry
                                .shades
                                .iter()
                                .map(|(layer, shade)| (*layer, shade.view.clone()))
                                .collect::<Vec<_>>(),
                            entry.image.as_ref().map_or(uv, |image| image.uv(uv)),
                            uv,
                        ))
                    };
                    // 空・透明タイルも含め、有効な全レイヤーのGPU転送を待つ。
                    let ready = desired
                        .layer_keys()
                        .iter()
                        .all(|(_, key)| fetch(key).is_some_and(|entry| entry.1));
                    // 描画区画をまとめ直しただけなら、表示していない粗画像から再フェードしない。
                    // ターゲットをまだGPUへ渡せない場合だけ、実際に表示する代替画像を切替元にする。
                    if !ready
                        && let Some(fallback) = initial_fallback
                        && fallback
                            .layer_keys()
                            .iter()
                            .all(|(_, key)| fetch(key).is_some_and(|entry| entry.1))
                    {
                        transition.current = Some(fallback);
                    }
                    transition.update(desired.clone(), ready, self.animate_lod, now);
                    if !ready {
                        ui.ctx().request_repaint();
                    }
                    if transition.current.is_none() {
                        next.insert(region, transition);
                        continue;
                    }
                    let current = transition.current.as_ref().unwrap_or(&desired);
                    let (old, amount) = transition
                        .blend(ui.ctx(), now)
                        .map_or((current, 1.), |(old, t)| (old, t));
                    for ((layer, old), (_, current)) in
                        old.layer_keys().into_iter().zip(current.layer_keys())
                    {
                        let mut entries = [fetch(&old), fetch(&current)];
                        let color_ready = entries
                            .each_ref()
                            .map(|entry| entry.as_ref().is_some_and(|entry| entry.1));
                        let uv = entries
                            .each_ref()
                            .map(|entry| entry.as_ref().map_or(lod_fade::FULL_UV, |entry| entry.3));
                        let image = entries
                            .each_mut()
                            .map(|entry| entry.as_mut().and_then(|entry| entry.0.take()));
                        if let Some(mut quad) = lod_fade::Quad::new(region, uv, image, color_ready)
                        {
                            quad.edge_filter = self.edge_filter;
                            quad.amount = Some(amount);
                            colors[layer].push(quad);
                        }
                        let ready = entries.each_ref().map(Option::is_some);
                        let uv = entries
                            .each_ref()
                            .map(|entry| entry.as_ref().map_or(lod_fade::FULL_UV, |entry| entry.4));
                        let shade = entries.each_ref().map(|entry| {
                            entry
                                .as_ref()?
                                .2
                                .iter()
                                .find(|(index, _)| *index == layer)
                                .map(|(_, texture)| texture.clone())
                        });
                        if let Some(mut quad) = lod_fade::Quad::new(region, uv, shade, ready) {
                            quad.edge_filter = self.edge_filter;
                            quad.amount = Some(amount);
                            shades[layer].push(quad);
                        }
                    }
                } else {
                    transition.update(desired.clone(), true, false, now);
                    for (layer, key) in desired.layer_keys() {
                        if let Some(entry) = self.cache.get_mut(&key) {
                            entry.touched = self.frame;
                            entry.collect(
                                view,
                                rect,
                                &key,
                                region,
                                &mut cpu_images[layer],
                                &mut cpu_shades[layer],
                            );
                        }
                    }
                }
                if self.borders {
                    borders.push(region);
                }
                next.insert(region, transition);
            }
            self.tile_lods.insert(self.material_pass, next);
            for layer in 0..set.layers.len() {
                if let Some((_, renderer)) = fade_backend {
                    renderer.paint(
                        painter,
                        rect,
                        view,
                        std::mem::take(&mut colors[layer]),
                        std::mem::take(&mut shades[layer]),
                        1.,
                        self.material_pass != 0,
                    );
                } else {
                    if let Some(renderer) = &self.shade_renderer {
                        renderer.paint(
                            painter,
                            rect,
                            std::mem::take(&mut cpu_shades[layer]),
                            self.material_pass != 0,
                        );
                    }
                    for (tile, uv, image) in std::mem::take(&mut cpu_images[layer]) {
                        view.paint_tile_uv(painter, rect, tile, image, uv);
                    }
                }
            }
            for tile in borders {
                crate::terrain_material::tile_border(
                    painter,
                    rect,
                    view,
                    tile,
                    self.material_pass != 0,
                    egui::Color32::from_rgb(0, 210, 235),
                );
            }
        }
        if self.material_pass == 0 {
            self.finish_material_frame();
        }
    }
}

pub struct PointLayer<'a> {
    pub path: &'a Path,
    pub count: usize,
    pub enabled: &'a mut bool,
    pub close: &'a mut bool,
    pub show_info: &'a mut bool,
}

pub fn layer_list(
    ui: &mut egui::Ui,
    surfaces: &mut [Surface],
    mut point_layer: Option<PointLayer<'_>>,
    map_source: &mut MapSource,
    online_enabled: &mut bool,
    zoom: f64,
    max_height: f32,
    lod: Lod,
    surface_lod: Lod,
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
                            .menu_button("i", |ui| {
                                source.show_info(ui, source.tile_zoom(zoom, lod))
                            })
                            .response
                            .on_hover_text("オンライン地図の情報")
                            .clicked();
                        if !*online_enabled {
                            ui.disable();
                        }
                        ui.add(
                            egui::Label::new(format!("自動 (z{})", source.tile_zoom(zoom, lod)))
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
                        let selected_z =
                            level.or_else(|| auto_level(&layer.levels, zoom, surface_lod));
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
                    if before.0 != *shown {
                        surface.tile_lods.clear();
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
        surface.tile_lods.clear();
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
                .map(|layer| {
                    layer
                        .levels
                        .iter()
                        .find(|level| level.z == tile.z)
                        .map(|level| level.z)
                })
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
    #[ignore = "指定した表示スナップショットで地表LODの往復を診断する"]
    fn trace_snapshot_surface_zoom_roundtrip() -> Result<()> {
        let path = std::env::var("YAMAMUKI_ZOOM_SNAPSHOT")?;
        let input = flate2::read::GzDecoder::new(std::fs::File::open(path)?);
        let snapshot: serde_json::Value = serde_json::from_reader(input)?;
        let original: View = serde_json::from_value(snapshot["view"].clone())?;
        let size: [f32; 2] = serde_json::from_value(snapshot["canvas_size"].clone())?;
        let lod: Lod = serde_json::from_value(snapshot["surface_lod"].clone())?;
        let ctx = egui::Context::default();
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None))?;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let renderer = eframe::egui_wgpu::Renderer::new(&device, format, None, 1, false);
        let state = eframe::egui_wgpu::RenderState {
            adapter,
            available_adapters: vec![],
            device,
            queue,
            target_format: format,
            renderer: Arc::new(eframe::epaint::mutex::RwLock::new(renderer)),
        };
        let capture = lod_fade::Renderer::new(&state);
        let mut flat = yamamuki_terrain_view::TerrainView::flat(&ctx, &state)?;
        let surface_state = &snapshot["surfaces"][0];
        let mut surface = Surface::new(
            &ctx,
            Source::open(PathBuf::from(surface_state["path"].as_str().unwrap()))?,
        );
        surface.options = serde_json::from_value(surface_state["options"].clone())?;
        surface.edge_filter = serde_json::from_value(snapshot["surface_edge_filter"].clone())?;
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(size[0], size[1]));
        let material_rect =
            Rect::from_min_size(Pos2::ZERO, egui::Vec2::splat(size[0].max(size[1])));
        let density = surface.material_max_zoom(lod).unwrap();
        let span = crate::terrain_material::minimum_span(f64::from(material_rect.width()), density);
        let (yaw, tilt) = original.orientation.angles();
        let mut frame = 0;
        let fast = std::env::var_os("YAMAMUKI_ZOOM_TRACE_FAST").is_some();
        let zooms = if let Ok(steps) = std::env::var("YAMAMUKI_ZOOM_TRACE_STEPS") {
            serde_json::from_str::<Vec<f64>>(&steps)?
        } else if fast {
            vec![
                19., 18., 17., 16., 15., 14., 13., 12.8, 12.6, 12.58, 12.5, 12.3, 12., 11.6, 11.58,
                11.5, 11., 10.5, 10., 11., 12., 13., 14., 15., 16., 17., 18., 19.,
            ]
        } else {
            vec![
                18.5,
                19.,
                original.zoom,
                19.5,
                20.,
                19.5,
                original.zoom,
                19.,
                18.5,
            ]
        };
        for z in zooms {
            flat.set_map_camera(original.center, z, yaw, tilt, rect);
            let regions = flat.projected_material_regions(rect, span);
            eprintln!(
                "view={z:.6} regions={:?}",
                regions.iter().map(|(r, z)| (*z, *r)).collect::<Vec<_>>()
            );
            let frames = std::env::var("YAMAMUKI_ZOOM_TRACE_FRAMES")
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(if fast { 1 } else { 12 });
            let mut last_active = None;
            for local in 0..if fast { frames } else { frames * 10 } {
                let output = ctx.run(
                    egui::RawInput {
                        time: Some(f64::from(frame) * 0.03),
                        screen_rect: Some(rect),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            for i in (1..regions.len()).chain(std::iter::once(0)) {
                                let (area, zoom) = regions[i];
                                let view = crate::terrain_material::map_view(
                                    area,
                                    f64::from(material_rect.width()),
                                );
                                surface.select_material_pass((i + 1) * 3);
                                surface.paint(
                                    ui,
                                    ui.painter(),
                                    material_rect,
                                    &view,
                                    false,
                                    false,
                                    Some((&state, &capture)),
                                    lod,
                                    zoom,
                                );
                            }
                        });
                    },
                );
                let mut renderer = state.renderer.write();
                for (id, delta) in &output.textures_delta.set {
                    renderer.update_texture(&state.device, &state.queue, *id, delta);
                }
                drop(renderer);
                surface.finish_material_frame();
                let active = surface
                    .tile_lods
                    .values()
                    .flat_map(|states| states.values())
                    .filter(|s| s.transitioning(f64::from(frame) * 0.03))
                    .count();
                if last_active != Some(active) {
                    eprintln!(
                        "  frame={local} active={active} cache={} pending={}",
                        surface.cache.len(),
                        surface.pending.len()
                    );
                    last_active = Some(active);
                }
                frame += 1;
                if !fast && local >= frames && surface.pending.is_empty() && active == 0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            for (pass, lod) in &surface.tile_lods {
                eprintln!(
                    "  {pass:?}: {:?}",
                    lod.values()
                        .filter_map(|state| state.current.as_ref())
                        .map(|k| (k.tile.z, k.pixels, k.levels.clone()))
                        .collect::<Vec<_>>()
                );
            }
            eprintln!(
                "  cache={} pending={}",
                surface.cache.len(),
                surface.pending.len()
            );
        }
        Ok(())
    }
    #[test]
    fn material_density_uses_image_pixels_and_fixed_lod() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("density.sqlite3");
        let conn = Connection::open(&path)?;
        db::initialize(&conn)?;
        conn.execute_batch(
            r#"
            INSERT INTO datasets VALUES(1,'test','surface','地表','1',1,'{}');
            INSERT INTO surface_layers VALUES(1,1,'forest','mask',0,'[]','{"color":[111,148,88,255]}');
            INSERT INTO surface_levels VALUES(1,1,9,'z9',512),(1,1,12,'z12',384);
        "#,
        )?;
        for (z, pixels) in [(9, 512), (12, 384)] {
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, pixels, pixels);
                encoder.set_color(png::ColorType::Grayscale);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()?
                    .write_image_data(&vec![255; (pixels * pixels) as usize])?;
            }
            conn.execute(
                "INSERT INTO surface_tiles VALUES(1,1,?1,0,0,1,?2)",
                db::rusqlite::params![z, bytes],
            )?;
        }
        drop(conn);
        let mut surface = Surface::new(&egui::Context::default(), Source::open(path)?);
        assert_eq!(
            surface.material_max_zoom(Lod::default()),
            Some(12. + 1.5f64.log2())
        );
        surface.options[0].1 = Some(9);
        assert_eq!(surface.material_max_zoom(Lod::default()), Some(10.));
        surface.options[0].0 = false;
        assert_eq!(surface.material_max_zoom(Lod::default()), None);
        Ok(())
    }
    #[test]
    fn terrain_toggle_keeps_surface_lod_at_the_same_screen_zoom() {
        let levels = [(8, 512), (10, 384), (12, 384)].map(|(z, pixels)| SurfaceLevel {
            z,
            lod: format!("z{z}"),
            pixels,
        });
        for screen in [8., 11.7, 12.6, 25.] {
            for material in [6.3, 10.4, 18.] {
                let flat = crate::terrain_material::lod_zoom(screen, material, 0);
                let terrain = crate::terrain_material::lod_zoom(screen, material, 2);
                assert_eq!(
                    auto_level(&levels, flat, Lod::default()),
                    auto_level(&levels, terrain, Lod::default())
                );
            }
        }
    }
    #[test]
    fn surface_auto_lod_switches_at_the_equivalent_pixel_density() {
        let levels =
            [(8, 512), (9, 512), (10, 384), (11, 384), (12, 384)].map(|(z, pixels)| SurfaceLevel {
                z,
                lod: format!("z{z}"),
                pixels,
            });
        let offset = 1.5_f64.log2();
        for (zoom, expected) in [
            (5., 8),
            (9.99, 8),
            (10., 9),
            (10. + offset - 0.001, 9),
            (10. + offset, 10),
            (11., 10),
            (11. + offset, 11),
            (12. + offset, 12),
            (18., 12),
        ] {
            assert_eq!(auto_level(&levels, zoom, Lod::default()), Some(expected));
        }
        assert_eq!(auto_level(&[], 9., Lod::default()), None);
    }

    #[test]
    fn surface_auto_lod_uses_density_even_when_it_decreases_with_xyz_zoom() {
        let levels = [(8, 512), (9, 128), (10, 128)].map(|(z, pixels)| SurfaceLevel {
            z,
            lod: format!("z{z}"),
            pixels,
        });
        assert_eq!(auto_level(&levels, 7., Lod::default()), Some(9));
        assert_eq!(auto_level(&levels, 8.99, Lod::default()), Some(9));
        assert_eq!(auto_level(&levels, 9., Lod::default()), Some(8));
    }

    #[test]
    fn surface_auto_lod_applies_bias_in_map_zoom_and_caps_xyz_level() {
        let levels = [(8, 512), (9, 512), (10, 384)].map(|(z, pixels)| SurfaceLevel {
            z,
            lod: format!("z{z}"),
            pixels,
        });
        let mut lod = Lod {
            max_level: Some(9),
            bias: 1.,
        };
        assert_eq!(auto_level(&levels, 8.99, lod), Some(8));
        assert_eq!(auto_level(&levels, 9., lod), Some(9));
        assert_eq!(auto_level(&levels, 18., lod), Some(9));
        lod.max_level = Some(4);
        assert_eq!(auto_level(&levels, 18., lod), Some(8));
    }

    #[test]
    fn drawing_keeps_layers_separate_with_independent_lods_and_original_order() -> Result<()> {
        check_separate_layer_drawing(false)
    }
    #[test]
    #[ignore = "実GPUで地形へ渡す補間・陰影・色のレイヤー順を確認する"]
    fn gpu_layer_interpolation_precedes_forest_shade_water_composition() -> Result<()> {
        check_separate_layer_drawing(true)
    }
    fn check_separate_layer_drawing(gpu: bool) -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("separate.sqlite3");
        let conn = Connection::open(&path)?;
        db::initialize(&conn)?;
        conn.execute_batch(r#"
            INSERT INTO datasets VALUES(1,'test','surface','地表','1',1,'{}');
            INSERT INTO surface_layers VALUES
                (1,1,'forest','mask',0,'[]','{"color":[0,140,0,255],"background":[200,200,200,255]}'),
                (1,2,'hillshade','hillshade',10,'[]','{"neutral":136,"strength":0.65}'),
                (1,3,'water','mask',20,'[]','{"color":[30,90,150,255]}');
            INSERT INTO surface_levels VALUES(1,1,0,'z0',2),(1,1,1,'z1',2),(1,2,0,'z0',2),(1,2,1,'z1',2),(1,3,0,'z0',2),(1,3,1,'z1',2);
        "#)?;
        for (layer, gray) in [(1, 255), (2, 68), (3, 128)] {
            let mut png = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut png, 2, 2);
                encoder.set_color(png::ColorType::Grayscale);
                encoder.write_header()?.write_image_data(&[gray; 4])?;
            }
            conn.execute(
                "INSERT INTO surface_tiles VALUES(1,?1,0,0,0,1,?2)",
                db::rusqlite::params![layer, png],
            )?;
        }
        drop(conn);
        let ctx = egui::Context::default();
        let mut surface = Surface::new(&ctx, Source::open(path)?);
        for option in &mut surface.options {
            option.1 = Some(0);
        }
        let original_options = surface.options.clone();
        let state = gpu.then(|| {
            let instance = wgpu::Instance::default();
            let adapter =
                pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU");
            let (device, queue) =
                pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
            let format = wgpu::TextureFormat::Rgba8Unorm;
            let renderer = eframe::egui_wgpu::Renderer::new(&device, format, None, 1, false);
            eframe::egui_wgpu::RenderState {
                adapter,
                available_adapters: Vec::new(),
                device,
                queue,
                target_format: format,
                renderer: Arc::new(eframe::epaint::mutex::RwLock::new(renderer)),
            }
        });
        let renderer = state.as_ref().map(lod_fade::Renderer::new);
        surface.edge_filter = gpu.into();
        surface.select_material_pass(usize::from(gpu));
        let view = View {
            center: [0.5, 0.5],
            zoom: 0.,
            orientation: Default::default(),
        };
        let mut frame = || {
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(256., 256.));
                    surface.paint(
                        ui,
                        ui.painter(),
                        rect,
                        &view,
                        false,
                        false,
                        state.as_ref().zip(renderer.as_ref()),
                        Lod::default(),
                        0.,
                    );
                });
            });
            if let Some(state) = &state {
                let mut renderer = state.renderer.write();
                for (id, delta) in &output.textures_delta.set {
                    renderer.update_texture(&state.device, &state.queue, *id, delta);
                }
            }
            let captured = output.shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Callback(c) if c.callback.downcast_ref::<yamamuki_terrain_view::direct_material::Capture>().is_some())).count();
            let ready =
                surface.pending.is_empty() && surface.cache.len() >= 3 && (!gpu || captured >= 3);
            (output, ready)
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let output = loop {
            let (output, ready) = frame();
            if ready {
                break output;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "地表レイヤーの読み込みが完了しない"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        drop(frame);
        assert_eq!(surface.options, original_options);
        assert!(surface.cache.len() >= 3);
        for key in surface.cache.keys() {
            assert_eq!(
                key.levels.iter().flatten().count(),
                1,
                "合成済みの画像を作っている"
            );
            assert_eq!(key.gutter, 3);
        }
        if gpu {
            let captures: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|s| {
                    let egui::Shape::Callback(c) = &s.shape else {
                        return None;
                    };
                    c.callback
                        .downcast_ref::<yamamuki_terrain_view::direct_material::Capture>()
                })
                .collect();
            assert_eq!(captures.len(), 3);
            assert_eq!(
                captures.iter().map(|c| c.shade).collect::<Vec<_>>(),
                vec![false, true, false]
            );
            assert!(
                captures
                    .iter()
                    .all(|c| c.edge_filter == Interpolation::Edge),
                "合成前のレイヤーに補間が適用されていない"
            );
            assert!(
                captures
                    .iter()
                    .all(|c| c.textures.iter().all(Option::is_some))
            );
            check_synchronized_surface_tile_fades(
                &ctx,
                &mut surface,
                state.as_ref().unwrap(),
                renderer.as_ref().unwrap(),
            )?;
            for layer in 0..3 {
                check_independent_surface_tile_fades(
                    &ctx,
                    &mut surface,
                    state.as_ref().unwrap(),
                    renderer.as_ref().unwrap(),
                    layer,
                )?;
            }
            return Ok(());
        }
        let images: Vec<_> = [0, 2]
            .into_iter()
            .map(|layer| {
                let (key, entry) = surface
                    .cache
                    .iter()
                    .find(|(key, _)| key.levels[layer].is_some())
                    .unwrap();
                let image = entry.image.as_ref().unwrap();
                (
                    image.id(),
                    image.uv(surface_uv_with_gutter(key.pixels, key.gutter)),
                )
            })
            .collect();
        let drawn: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                let egui::Shape::Mesh(mesh) = &shape.shape else {
                    return None;
                };
                images.iter().position(|(id, uv)| {
                    *id == mesh.texture_id
                        && mesh.vertices.first().is_some_and(|v| uv.contains(v.uv))
                })
            })
            .collect();
        // 同じアトラスページに格納されても、UVで識別して実際の描画順を確認する。
        assert_eq!(drawn, vec![0, 1]);
        assert!(surface.tile_lods.contains_key(&0));
        Ok(())
    }

    fn check_synchronized_surface_tile_fades(
        ctx: &egui::Context,
        surface: &mut Surface,
        state: &eframe::egui_wgpu::RenderState,
        renderer: &lod_fade::Renderer,
    ) -> Result<()> {
        let (jobs, requests) = mpsc::channel();
        let (send, replies) = mpsc::channel();
        surface.jobs = jobs;
        surface.replies = replies;
        surface.pending.clear();
        for option in &mut surface.options {
            *option = (true, Some(1));
        }
        let view = View {
            center: [0.5, 0.25],
            zoom: 1.,
            orientation: Default::default(),
        };
        let frame = |surface: &mut Surface, time| {
            let output = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        surface.paint(
                            ui,
                            ui.painter(),
                            Rect::from_min_size(Pos2::ZERO, egui::vec2(256., 128.)),
                            &view,
                            false,
                            false,
                            Some((state, renderer)),
                            Lod::default(),
                            1.,
                        );
                    });
                },
            );
            for (id, delta) in &output.textures_delta.set {
                state
                    .renderer
                    .write()
                    .update_texture(&state.device, &state.queue, *id, delta);
            }
            output
                .shapes
                .iter()
                .filter_map(|s| {
                    let egui::Shape::Callback(c) = &s.shape else {
                        return None;
                    };
                    let c = c
                        .callback
                        .downcast_ref::<yamamuki_terrain_view::direct_material::Capture>()?;
                    Some((c.rect.center().x, c.amount))
                })
                .collect::<Vec<_>>()
        };
        // 描画履歴がなくても、GPUに残る粗い画像で未取得範囲を埋める。
        surface.tile_lods.clear();
        for entry in surface.cache.values() {
            if let Some(image) = &entry.image {
                assert!(
                    state.renderer.read().texture(&image.id()).is_some(),
                    "粗画像がGPUから消えている"
                );
            }
        }
        let initial = frame(surface, 20.);
        assert_eq!(initial.len(), 6, "GPUにある粗い地表を選べず白抜けした");
        let keys: Vec<_> = requests.try_iter().collect();
        let image = |layer| match layer {
            1 => ComposedTile {
                image: None,
                shades: vec![ShadeImage {
                    layer: 1,
                    image: ColorImage::new([262, 262], egui::Color32::WHITE),
                    neutral: 136,
                    strength: 0.65,
                }],
            },
            2 => ComposedTile::default(),
            _ => ComposedTile {
                image: Some(ColorImage::new([262, 262], egui::Color32::WHITE)),
                shades: Vec::new(),
            },
        };
        for (layer, time) in [(0, 20.01), (1, 20.03)] {
            let key = keys
                .iter()
                .find(|key| key.tile.x == 0 && key.levels[layer].is_some())
                .unwrap()
                .clone();
            send.send((key, Ok(image(layer))))?;
            frame(surface, time);
            let captured = frame(surface, time + 0.01);
            assert_eq!(captured.len(), 6);
            assert!(
                captured.iter().all(|(_, amount)| *amount == 1.),
                "未到着のレイヤーを待たずに切り替わった: {captured:?}"
            );
        }
        let key = keys
            .iter()
            .find(|key| key.tile.x == 0 && key.levels[2].is_some())
            .unwrap()
            .clone();
        send.send((key, Ok(image(2))))?;
        frame(surface, 20.05);
        let captured = frame(surface, 20.06);
        let left: Vec<_> = captured
            .iter()
            .filter(|(x, _)| *x < 128.)
            .map(|(_, amount)| *amount)
            .collect();
        assert_eq!(left.len(), 3);
        assert!(
            left[0] < 1. && left.iter().all(|amount| *amount == left[0]),
            "左タイルのレイヤーが同期していない: {captured:?}"
        );
        assert!(
            captured
                .iter()
                .filter(|(x, _)| *x > 128.)
                .all(|(_, amount)| *amount == 1.)
        );
        for layer in 0..3 {
            let key = keys
                .iter()
                .find(|key| key.tile.x == 1 && key.levels[layer].is_some())
                .unwrap()
                .clone();
            send.send((key, Ok(image(layer))))?;
        }
        frame(surface, 20.1);
        let captured = frame(surface, 20.11);
        let right: Vec<_> = captured
            .iter()
            .filter(|(x, _)| *x > 128.)
            .map(|(_, amount)| *amount)
            .collect();
        let left: Vec<_> = captured
            .iter()
            .filter(|(x, _)| *x < 128.)
            .map(|(_, amount)| *amount)
            .collect();
        assert_eq!(right.len(), 3);
        assert!(
            right[0] < left[0] && right.iter().all(|amount| *amount == right[0]),
            "タイル間の独立性とレイヤー間の同期が両立していない: {captured:?}"
        );
        frame(surface, 20.5);
        // 完了済みタイルの描画区画を再構成しても、未表示の粗画像からフェードし直さない。
        surface.tile_lods.clear();
        let repeated = frame(surface, 20.6);
        assert!(
            repeated.iter().all(|(_, amount)| *amount == 1.),
            "完了済み画像へ不要なフェードが再発した: {repeated:?}"
        );
        requests.try_iter().for_each(drop);
        for option in &mut surface.options {
            option.1 = Some(0);
        }
        frame(surface, 21.);
        assert!(
            requests.try_iter().next().is_none(),
            "ズームアウトで保持済みの地表タイルを再要求した"
        );
        Ok(())
    }

    fn check_independent_surface_tile_fades(
        ctx: &egui::Context,
        surface: &mut Surface,
        state: &eframe::egui_wgpu::RenderState,
        renderer: &lod_fade::Renderer,
        layer: usize,
    ) -> Result<()> {
        let (jobs, requests) = mpsc::channel();
        let (send, replies) = mpsc::channel();
        surface.jobs = jobs;
        surface.replies = replies;
        surface.pending.clear();
        surface.tile_lods.clear();
        let remove: Vec<_> = surface
            .cache
            .keys()
            .filter(|key| key.tile.z > 0)
            .cloned()
            .collect();
        for key in remove {
            surface.cache_bytes -= surface.cache.remove(&key).unwrap().bytes;
        }
        for (index, option) in surface.options.iter_mut().enumerate() {
            *option = (index == layer, Some(0));
        }
        let view = View {
            center: [0.5, 0.25],
            zoom: 1.,
            orientation: Default::default(),
        };
        let frame = |surface: &mut Surface, time: f64| {
            let time = time + layer as f64;
            let output = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        surface.paint(
                            ui,
                            ui.painter(),
                            Rect::from_min_size(Pos2::ZERO, egui::vec2(256., 128.)),
                            &view,
                            false,
                            layer != 0,
                            Some((state, renderer)),
                            Lod::default(),
                            1.,
                        );
                    });
                },
            );
            for (id, delta) in &output.textures_delta.set {
                state
                    .renderer
                    .write()
                    .update_texture(&state.device, &state.queue, *id, delta);
            }
            output
                .shapes
                .iter()
                .filter_map(|s| {
                    let egui::Shape::Callback(c) = &s.shape else {
                        return None;
                    };
                    let c = c
                        .callback
                        .downcast_ref::<yamamuki_terrain_view::direct_material::Capture>()?;
                    Some((
                        c.rect.center().x,
                        c.amount,
                        c.textures[0] != c.textures[1] || c.uv[0] != c.uv[1],
                    ))
                })
                .collect::<Vec<_>>()
        };
        frame(surface, 49.8);
        for option in &mut surface.options {
            option.1 = Some(1);
        }
        frame(surface, 50.);
        let keys: Vec<_> = requests.try_iter().collect();
        let left = keys
            .iter()
            .find(|key| key.tile.x == 0)
            .expect("左の要求")
            .clone();
        let right = keys
            .iter()
            .find(|key| key.tile.x == 1)
            .expect("右の要求")
            .clone();
        let image = || match layer {
            0 => ComposedTile {
                image: Some(ColorImage::new([262, 262], egui::Color32::WHITE)),
                shades: Vec::new(),
            },
            1 => ComposedTile {
                image: None,
                shades: vec![ShadeImage {
                    layer: 1,
                    image: ColorImage::new([262, 262], egui::Color32::WHITE),
                    neutral: 136,
                    strength: 0.65,
                }],
            },
            // 完全に透明な水域タイルも準備済みとしてフェードする。
            _ => ComposedTile::default(),
        };
        send.send((left, Ok(image())))?;
        frame(surface, 50.01);
        let first = frame(surface, 50.02);
        assert!(
            first.iter().any(|(x, t, d)| *x < 128. && *t < 1. && *d),
            "隣が未到着のため左の地表タイルがフェードしない: layer={layer} {first:?}"
        );
        assert!(
            first.iter().any(|(x, t, d)| *x > 128. && *t == 1. && !d),
            "未到着の右側が旧地表を維持していない: {first:?}"
        );
        send.send((right, Ok(image())))?;
        frame(surface, 50.1);
        let second = frame(surface, 50.11);
        let left = second.iter().find(|(x, _, _)| *x < 128.).unwrap();
        let right = second.iter().find(|(x, _, _)| *x > 128.).unwrap();
        assert!(
            left.1 > right.1 && right.1 < 1. && right.2,
            "地表タイルが個別の時刻で開始していない: {second:?}"
        );
        assert!(frame(surface, 50.5).iter().all(|(_, t, _)| *t == 1.));
        Ok(())
    }

    #[test]
    fn cache_keeps_coarse_surface_bundles_when_fine_tiles_arrive() {
        let mut cache = HashMap::new();
        for (z, tiles, touched) in [(5, 2, 1), (12, 10, 2)] {
            for x in 0..tiles {
                for layer in 0..3 {
                    let mut levels = vec![None; 3];
                    levels[layer] = Some(z);
                    let key = Key {
                        gutter: 3,
                        dataset: 0,
                        tile: Tile { z, x, y: 0 },
                        pixels: 512,
                        levels,
                        auto_levels: vec![true; 3],
                        overlay: layer > 0,
                        palette: Palette::default(),
                    };
                    cache.insert(
                        key,
                        Cached {
                            image: None,
                            shades: Vec::new(),
                            bytes: 1,
                            touched,
                            failed: false,
                        },
                    );
                }
            }
        }
        let mut bytes = cache.len();
        trim_surface_cache(&mut cache, &mut bytes, 18, 100, 3);
        assert_eq!(bytes, cache.len());
        assert!(bytes <= 18);
        assert_eq!(
            cache.keys().filter(|key| key.tile.z == 5).count(),
            6,
            "粗い地表が細タイルに追い出された"
        );
        for x in 0..10 {
            let count = cache
                .keys()
                .filter(|key| key.tile.z == 12 && key.tile.x == x)
                .count();
            assert!(count == 0 || count == 3, "一部レイヤーだけ破棄された");
        }
        for entry in cache.values_mut() {
            entry.touched = 3;
        }
        trim_surface_cache(&mut cache, &mut bytes, 0, 0, 3);
        assert!(!cache.is_empty(), "表示中のタイルを破棄した");
    }

    #[test]
    fn visible_surface_does_not_consume_the_zoom_out_reserve() {
        let mut cache = HashMap::new();
        for (z, tiles, bytes, touched) in [(5, 2, 1, 1), (12, 1, 100, 3)] {
            for x in 0..tiles {
                for layer in 0..3 {
                    let mut levels = vec![None; 3];
                    levels[layer] = Some(z);
                    let key = Key {
                        gutter: 3,
                        dataset: 0,
                        tile: Tile { z, x, y: 0 },
                        pixels: 512,
                        levels,
                        auto_levels: vec![true; 3],
                        overlay: layer > 0,
                        palette: Palette::default(),
                    };
                    cache.insert(
                        key,
                        Cached {
                            image: None,
                            shades: Vec::new(),
                            bytes,
                            touched,
                            failed: false,
                        },
                    );
                }
            }
        }
        let mut bytes = 306;
        trim_surface_cache(&mut cache, &mut bytes, 6, 6, 3);
        assert_eq!(
            cache.len(),
            9,
            "表示中の画像が保持枠を超えただけで粗タイルが全滅した"
        );
        assert_eq!(bytes, 306);
        // 細画像が表示対象でなくなったら、保持枠内へ整理する。
        trim_surface_cache(&mut cache, &mut bytes, 6, 6, 4);
        assert_eq!(bytes, 6);
        assert!(cache.keys().all(|key| key.tile.z == 5));
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

    pub(super) fn interior(image: &ColorImage) -> Vec<egui::Color32> {
        image
            .pixels
            .chunks_exact(image.size[0])
            .skip(1)
            .take(image.size[1] - 2)
            .flat_map(|row| row[1..row.len() - 1].iter().copied())
            .collect()
    }

    #[test]
    fn ground_and_hillshade_sample_across_tile_boundaries() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("edges.sqlite3");
        let conn = Connection::open(&path)?;
        db::initialize(&conn)?;
        conn.execute_batch(r#"
            INSERT INTO datasets VALUES (1,'edges','surface','地表','1',1,'{}');
            INSERT INTO surface_layers VALUES (1,1,'hillshade','hillshade',0,'[]','{"neutral":136,"strength":1}');
            INSERT INTO surface_levels VALUES (1,1,1,'z1',2);
        "#)?;
        for (x, gray) in [(0, 40), (1, 200)] {
            let mut png = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut png, 2, 2);
                encoder.set_color(png::ColorType::Grayscale);
                encoder.write_header()?.write_image_data(&[gray; 4])?;
            }
            conn.execute(
                "INSERT INTO surface_tiles VALUES(1,1,1,?1,0,1,?2)",
                (x, png),
            )?;
        }
        let source = Source::open(path)?;
        let dataset = &source.datasets[0];
        for overlay in [false, true] {
            let render = |z, x| -> Result<ColorImage> {
                let mut result = composite(
                    &conn,
                    dataset,
                    Tile { z, x, y: 0 },
                    2,
                    &[Some(1)],
                    overlay,
                    &[],
                )?;
                if overlay {
                    Ok(result.shades.remove(0).image)
                } else {
                    result.image.context("陰影画像がありません")
                }
            };
            let gray = egui::Color32::from_gray;
            assert_eq!(
                &render(1, 0)?.pixels[4..8],
                &[gray(40), gray(40), gray(40), gray(200)]
            );
            assert_eq!(
                &render(1, 1)?.pixels[4..8],
                &[gray(40), gray(200), gray(200), gray(200)]
            );
            assert_eq!(&interior(&render(2, 1)?)[..2], &[gray(40), gray(80)]);
            assert_eq!(&interior(&render(2, 2)?)[..2], &[gray(160), gray(200)]);
        }
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
            interior(&result.image.context("描画タイル")?),
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
            interior(&result.image.context("生成済みの粗いタイル")?),
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
            interior(
                &at(outside, &[true, true])?
                    .image
                    .context("粗いタイルにフォールバック")?
            ),
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
            interior(&image),
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
            interior(&overlay.shades[0].image),
            [68, 68, 136, 136].map(egui::Color32::from_gray)
        );
        assert_eq!(
            interior(&overlay.image.context("地表の重ね表示が透明です")?),
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
            interior(&water_overlay.image.context("水域が透明です")?),
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
            interior(&shade_only),
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
                                f32::INFINITY,
                                Lod::default(),
                                Lod::default(),
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

#[cfg(test)]
mod palette_checks {
    use super::*;
    use anyhow::Context;
    #[test]
    fn palette_recolors_masks_and_keeps_transparency_and_decoded_pixels() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("palette.sqlite3");
        let conn = Connection::open(&path)?;
        db::initialize(&conn)?;
        conn.execute_batch(
            r#"
            INSERT INTO datasets VALUES(1,'palette','surface','配色','1',1,'{}');
            INSERT INTO surface_layers VALUES
              (1,1,'forest','mask',0,'[]','{"color":[10,20,30,255],"background":[70,80,90,255]}'),
              (1,2,'hillshade','hillshade',10,'[]','{"neutral":136,"strength":0.65}'),
              (1,3,'water','mask',20,'[]','{"color":[40,50,60,255]}');
            INSERT INTO surface_levels VALUES(1,1,0,'z0',2),(1,2,0,'z0',2),(1,3,0,'z0',2);
        "#,
        )?;
        for (layer, pixels) in [(1, [0, 255, 0, 0]), (2, [136; 4]), (3, [0, 0, 255, 0])] {
            let mut png = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut png, 2, 2);
                encoder.set_color(png::ColorType::Grayscale);
                encoder.write_header()?.write_image_data(&pixels)?;
            }
            conn.execute(
                "INSERT INTO surface_tiles VALUES(1,?1,0,0,0,1,?2)",
                (layer, png),
            )?;
        }
        let source = Source::open(path)?;
        let set = &source.datasets[0];
        let tile = Tile { z: 0, x: 0, y: 0 };
        let mut decoded = DecodedTiles::default();
        let original = decoded.get(&conn, 1, 1, tile)?.unwrap();
        for palette in Palette::ALL {
            decoded.palette = palette;
            for overlay in [false, true] {
                let result = composite_cached(
                    &conn,
                    set,
                    tile,
                    2,
                    &[Some(0); 3],
                    overlay,
                    &[],
                    &mut decoded,
                )?;
                let colors = if palette == Palette::Source {
                    [[70, 80, 90, 255], [10, 20, 30, 255], [40, 50, 60, 255]]
                } else {
                    palette.colors()
                };
                let color =
                    |c: [u8; 4]| egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
                let other = if overlay {
                    egui::Color32::TRANSPARENT
                } else {
                    color(colors[0])
                };
                assert_eq!(
                    tests::interior(&result.image.context("地表画像がありません")?),
                    vec![other, color(colors[1]), color(colors[2]), other]
                );
                assert_eq!(result.shades.len(), usize::from(overlay));
            }
            assert!(Arc::ptr_eq(
                &original,
                &decoded.get(&conn, 1, 1, tile)?.unwrap()
            ));
        }
        Ok(())
    }
}
