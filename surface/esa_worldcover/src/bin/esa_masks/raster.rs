//! ESA WorldCoverから全国地表画像を生成する。
use super::{classify, gdal};
#[path = "urban_relief.rs"]
mod urban_relief;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use rayon::prelude::*;
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    process::Command,
};

const WORLD: f64 = 40075016.68557849;
const TILE: usize = 256;
const CORE: usize = TILE * 16;
const LPF_HALO: usize = 192;
const PAYLOAD: usize = TILE * TILE / 4;
const LAND_MIX_ENCODING: &str = "mix21-index8";
const SOURCE_WATER_GUARD: usize = 448;
const MIX_PALETTE: [[u8; 4]; 5] = [
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [0; 4],
    [0; 4],
];
const PALETTE: [[u8; 4]; 5] = [
    [224, 216, 196, 255],
    [111, 148, 88, 255],
    [185, 152, 113, 255],
    [114, 172, 204, 255],
    [0, 0, 0, 0],
];

#[derive(Parser)]
struct Args {
    /// ESA WorldCover 2021 v200のGeoTIFF原本フォルダー。
    #[arg(long)]
    source: PathBuf,
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    source_gaps: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    astcenc: PathBuf,
    /// Store lossless PNG tiles instead of ASTC blocks.
    #[arg(long)]
    png: bool,
    #[arg(long, default_value_t = 8)]
    jobs: usize,
    #[arg(long)]
    only_area: Option<String>,
    /// Average the class colors by area at every level.
    #[arg(long, conflicts_with = "lpf")]
    area_average: bool,
    #[arg(long)]
    lpf: bool,
    #[arg(long, value_parser = ["land", "water"])]
    layer: Option<String>,
    /// Fill sea colors in the source underlay before generating land LODs.
    #[arg(long)]
    source_water_fill: bool,
    /// Store one-byte land mixture indices and fill fully hidden source water.
    #[arg(long, conflicts_with = "source_water_fill")]
    indexed_land: bool,
    /// Reuse the water tiles of a compatible combined SQLite.
    #[arg(long, requires = "indexed_land")]
    reuse_water: Option<PathBuf>,
    /// Add a source-derived forest underlay to an existing split land SQLite.
    #[arg(long, requires = "indexed_land", conflicts_with = "reuse_water")]
    forest_underlay_from: Option<PathBuf>,
    /// Fill water with the locally dominant ESA land class before resampling.
    #[arg(long, requires = "indexed_land")]
    categorical_water_fill: bool,
    /// Keep original ESA class coverage for comparison without land underlays.
    #[arg(long, requires = "indexed_land", conflicts_with_all = ["categorical_water_fill", "forest_underlay_from"])]
    no_land_underlay: bool,
    /// Generate forest, water, and bare masks in their source compositing order.
    #[arg(long, requires = "indexed_land", conflicts_with_all = ["categorical_water_fill", "no_land_underlay", "forest_underlay_from", "reuse_water"])]
    bare_over_water: bool,
    /// Generate forest, bare, and water masks with water as the top layer.
    #[arg(long, requires = "indexed_land", conflicts_with_all = ["bare_over_water", "categorical_water_fill", "no_land_underlay", "forest_underlay_from", "reuse_water"])]
    water_over_bare: bool,
    /// Replace single isolated source pixels before any underlay or LPF.
    #[arg(long, requires = "water_over_bare")]
    remove_isolated_source_pixels: bool,
    /// Suppress small tree-cover components in built-up neighborhoods.
    #[arg(long, requires = "water_over_bare")]
    suppress_urban_forest: bool,
    /// Use z11 elevation from a terrain LOD SQLite to soften flat urban forest.
    #[arg(long, requires = "suppress_urban_forest")]
    urban_forest_relief_dem: Option<PathBuf>,
    /// Fill ocean pixels with a majority of forest in the specified source pixel radius.
    #[arg(long, requires = "water_over_bare", value_parser = clap::value_parser!(u32).range(1..=64))]
    coastal_forest_median_radius: Option<u32>,
    /// Experimental full forest underlay for the inland water body containing LAT LON.
    #[arg(long, num_args = 2, value_names = ["LAT", "LON"], requires_all = ["only_area", "coastal_forest_median_radius"])]
    full_inland_forest_at: Option<Vec<f64>>,
    /// Fill entire inland water bodies with at least 85% forested shore.
    #[arg(
        long,
        requires = "coastal_forest_median_radius",
        conflicts_with = "full_inland_forest_at"
    )]
    full_forested_inland_water: bool,
    /// Generate the combined raster levels at z10, z9, z8, z7, and z6.
    #[arg(long, requires = "water_over_bare")]
    compact_zooms: bool,
    /// Add source z11 to the compact levels.
    #[arg(long, requires_all = ["compact_zooms", "lpf"])]
    include_l2: bool,
    /// Classify WorldCover NoData as water unless the DEM confirms land.
    #[arg(long, requires = "water_over_bare")]
    esa_zero_land_dem: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CompositeOrder {
    BareOverWater,
    WaterOverBare,
}

impl Args {
    fn zooms(&self) -> &'static [u32] {
        if self.include_l2 {
            &[11, 10, 9, 8, 7, 6]
        } else if self.compact_zooms {
            &[10, 9, 8, 7, 6]
        } else if self.layer.is_some() {
            &[11, 10, 8, 6]
        } else {
            &[11, 9, 8, 6]
        }
    }

    fn composite_order(&self) -> Option<CompositeOrder> {
        if self.water_over_bare {
            Some(CompositeOrder::WaterOverBare)
        } else {
            self.bare_over_water
                .then_some(CompositeOrder::BareOverWater)
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
struct Area {
    x: u32,
    y: u32,
    #[serde(rename = "maxZoom")]
    max_zoom: Option<u32>,
    #[serde(rename = "workZoom", default = "base_work_zoom")]
    work_zoom: u32,
}

const fn base_work_zoom() -> u32 {
    8
}

struct Tile {
    z: u32,
    x: u32,
    y: u32,
    rgba: Vec<u8>,
}

fn hidden_command(path: &PathBuf) -> Command {
    let mut command = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

fn palette_pixel(counts: [u32; 5], area_average: bool) -> [u8; 4] {
    let known: u32 = counts[..4].iter().sum();
    if known == 0 || counts[4] > known {
        return PALETTE[4];
    }
    if area_average {
        let rgb: [u8; 3] = std::array::from_fn(|channel| {
            let sum: u32 = counts[..4]
                .iter()
                .zip(PALETTE)
                .map(|(count, color)| count * u32::from(color[channel]))
                .sum();
            ((sum + known / 2) / known) as u8
        });
        return [rgb[0], rgb[1], rgb[2], 255];
    }
    let index = (0..4).max_by_key(|&class| (counts[class], class)).unwrap();
    PALETTE[index]
}

fn downsample(classes: &[u8], side: usize, area_average: bool) -> Vec<u8> {
    let factor = CORE / side;
    let mut rgba = Vec::with_capacity(side * side * 4);
    for y in 0..side {
        for x in 0..side {
            let mut counts = [0; 5];
            for yy in y * factor..(y + 1) * factor {
                for &class in &classes[yy * CORE + x * factor..yy * CORE + (x + 1) * factor] {
                    counts[usize::from(class)] += 1;
                }
            }
            rgba.extend_from_slice(&palette_pixel(counts, area_average));
        }
    }
    rgba
}

#[cfg(test)]
#[test]
fn area_average_keeps_minority_color_without_mixing_unknown() {
    let counts = [0, 3, 0, 1, 2];
    assert_eq!(palette_pixel(counts, false), PALETTE[1]);
    assert_eq!(palette_pixel(counts, true), [112, 154, 117, 255]);
    assert_eq!(palette_pixel([0, 3, 0, 1, 5], true), PALETTE[4]);
}

// Symmetric binomial low-pass filter centered between two source texels.
const LPF_WEIGHTS: [u32; 6] = [1, 5, 10, 10, 5, 1];

fn filter_pixel<const N: usize>(taps: [usize; 6], sample: impl Fn(usize) -> [u8; N]) -> [u8; N] {
    let mut sum = [0; N];
    for (tap, weight) in taps.into_iter().zip(LPF_WEIGHTS) {
        for (total, value) in sum.iter_mut().zip(sample(tap)) {
            *total += u32::from(value) * weight;
        }
    }
    sum.map(|total| ((total + 16) / 32) as u8)
}

fn lpf_half<const N: usize>(src: &[[u8; N]], n: usize) -> Vec<[u8; N]> {
    let half = n / 2;
    let indices: Vec<[usize; 6]> = (0..half)
        .map(|i| std::array::from_fn(|tap| (2 * i + tap).saturating_sub(2).min(n - 1)))
        .collect();
    let mut horizontal = vec![[0; N]; n * half];
    horizontal
        .par_chunks_mut(half)
        .zip(src.par_chunks(n))
        .for_each(|(out, row)| {
            for (pixel, &taps) in out.iter_mut().zip(&indices) {
                *pixel = filter_pixel(taps, |x| row[x]);
            }
        });
    let mut output = vec![[0; N]; half * half];
    output
        .par_chunks_mut(half)
        .zip(indices.par_iter())
        .for_each(|(row, &taps)| {
            for (x, pixel) in row.iter_mut().enumerate() {
                *pixel = filter_pixel(taps, |y| horizontal[y * half + x]);
            }
        });
    output
}

fn sample_underlay(pixels: &[[u8; 4]], side: usize, x: f32, y: f32) -> [u8; 4] {
    let taps = |v: f32| {
        let v = v.clamp(0., (side - 1) as f32);
        let base = v as usize;
        [
            (base, 1. - v.fract()),
            ((base + 1).min(side - 1), v.fract()),
        ]
    };
    let mut rgba = [0.; 4];
    for (y, wy) in taps(y) {
        for (x, wx) in taps(x) {
            for (sum, value) in rgba.iter_mut().zip(pixels[y * side + x]) {
                *sum += f32::from(value) * wx * wy;
            }
        }
    }
    rgba.map(|v| v.round() as u8)
}

fn fill_water_underlay(
    src: &mut [[u8; 4]],
    classes: &[u8],
    land: &[u8],
    n: usize,
    fill_sea: bool,
    background: [u8; 4],
) {
    if !classes
        .iter()
        .zip(land)
        .any(|(&class, &land)| class == 3 && (fill_sea || land != 0))
    {
        return;
    }
    // Keep the finest estimate at eight source pixels, then use broader
    // estimates where a lake has too little shoreline in that neighborhood.
    let mut side = n;
    let mut reduced = lpf_half(src, side);
    side /= 2;
    for _ in 0..2 {
        reduced = lpf_half(&reduced, side);
        side /= 2;
    }
    let shore = fill_sea.then(|| reduced.clone());
    let mut pyramid = vec![(side, reduced)];
    while side > n / 128 {
        let reduced = lpf_half(&pyramid.last().unwrap().1, side);
        side /= 2;
        pyramid.push((side, reduced));
    }
    let mut totals = [0u64; 4];
    for pixel in &pyramid.last().unwrap().1 {
        for (total, &value) in totals.iter_mut().zip(pixel) {
            *total += u64::from(value);
        }
    }
    let mut regional_color = background;
    if totals[3] != 0 {
        for (channel, &total) in regional_color[..3].iter_mut().zip(&totals) {
            *channel = ((total * 255 + totals[3] / 2) / totals[3]).min(255) as u8;
        }
    }
    let mut parent = None;
    for (side, mut level) in pyramid.into_iter().rev() {
        level.par_chunks_mut(side).enumerate().for_each(|(y, row)| {
            for (x, pixel) in row.iter_mut().enumerate() {
                let fallback = parent.as_ref().map_or(
                    regional_color,
                    |(parent_side, pixels): &(usize, Vec<[u8; 4]>)| {
                        sample_underlay(
                            pixels,
                            *parent_side,
                            x as f32 * 0.5 - 0.25,
                            y as f32 * 0.5 - 0.25,
                        )
                    },
                );
                let coverage = u32::from(pixel[3]);
                let local_weight = coverage.min(64);
                for (channel, &base) in pixel[..3].iter_mut().zip(&fallback) {
                    let local = (u32::from(*channel) * 255 + coverage / 2) / coverage.max(1);
                    *channel = ((local.min(255) * local_weight
                        + u32::from(base) * (64 - local_weight)
                        + 32)
                        / 64) as u8;
                }
                pixel[3] = 255;
            }
        });
        parent = Some((side, level));
    }
    let (side, colors) = parent.unwrap();
    src.par_iter_mut().enumerate().for_each(|(i, pixel)| {
        if classes[i] != 3 || (!fill_sea && land[i] == 0) {
            return;
        }
        let x = ((i % n) as f32 + 0.5) / 8. - 0.5;
        let y = ((i / n) as f32 + 0.5) / 8. - 0.5;
        if land[i] != 0 {
            *pixel = sample_underlay(&colors, side, x, y);
        } else if let Some(shore) = &shore {
            let weight = u32::from(sample_underlay(shore, side, x, y)[3]).min(64);
            *pixel = background;
            if weight != 0 {
                let near = sample_underlay(&colors, side, x, y);
                for (base, nearby) in pixel[..3].iter_mut().zip(near) {
                    *base = ((u32::from(*base) * (64 - weight) + u32::from(nearby) * weight + 32)
                        / 64) as u8;
                }
            }
        }
    });
}

// Six LPF halvings reach at most 160 source texels from an output center.
// The two bilinear footprints can each reach 64 source texels at z6.
fn fill_deep_source_water(src: &mut [[u8; 4]], classes: &[u8], n: usize, guard: usize) {
    if n <= 2 * guard {
        return;
    }
    let mut horizontal = vec![false; n * n];
    horizontal
        .par_chunks_mut(n)
        .zip(classes.par_chunks(n))
        .for_each(|(out, row)| {
            let mut run = 0;
            for (x, &class) in row.iter().enumerate() {
                run = if class == 3 { run + 1 } else { 0 };
                if run > 2 * guard {
                    out[x - guard] = true;
                }
            }
        });
    let mut vertical = vec![0; n];
    for (y, row) in horizontal.chunks_exact(n).enumerate() {
        for (run, &water) in vertical.iter_mut().zip(row) {
            *run = if water { *run + 1 } else { 0 };
        }
        if y >= 2 * guard {
            for (pixel, &run) in src[(y - guard) * n..][..n].iter_mut().zip(&vertical) {
                if run > 2 * guard {
                    *pixel = MIX_PALETTE[0];
                }
            }
        }
    }
}

fn fill_categorical_underlay(src: &mut [[u8; 4]], classes: &[u8], side: usize, hole: u8) {
    if !classes.contains(&hole) {
        return;
    }
    let known: Vec<_> = classes
        .par_iter()
        .map(|&class| {
            if class < hole {
                MIX_PALETTE[class as usize]
            } else {
                [0; 4]
            }
        })
        .collect();
    let half = lpf_half(&known, side);
    let quarter = lpf_half(&half, side / 2);
    let fine = lpf_half(&quarter, side / 4);
    let middle = lpf_half(&fine, side / 8);
    let coarse = lpf_half(&middle, side / 16);
    src.par_iter_mut().enumerate().for_each(|(i, pixel)| {
        if classes[i] != hole {
            return;
        }
        let x = (i % side) as f32 + 0.5;
        let y = (i / side) as f32 + 0.5;
        let class = [(&fine, 8), (&middle, 16), (&coarse, 32)]
            .into_iter()
            .find_map(|(image, scale)| {
                let s = scale as f32;
                let [other, forest, bare, coverage] =
                    sample_underlay(image, side / scale, x / s - 0.5, y / s - 0.5);
                let votes = [other, forest, bare];
                (coverage >= 64).then(|| {
                    (0..usize::from(hole))
                        .rev()
                        .max_by_key(|&class| votes[class])
                        .unwrap()
                })
            })
            .unwrap_or(0);
        *pixel = MIX_PALETTE[class];
    });
}

fn fill_adjacent_forest_under_bare(src: &mut [[u8; 4]], classes: &[u8], side: usize) {
    src.par_chunks_mut(side).enumerate().for_each(|(y, row)| {
        for (x, pixel) in row.iter_mut().enumerate() {
            let i = y * side + x;
            if classes[i] != 2 && !(classes[i] == 3 && *pixel == MIX_PALETTE[2]) {
                continue;
            }
            let neighbors = [
                (x > 0).then(|| classes[i - 1]),
                (x + 1 < side).then(|| classes[i + 1]),
                (y > 0).then(|| classes[i - side]),
                (y + 1 < side).then(|| classes[i + side]),
            ];
            let mut votes = [0; 2];
            for class in neighbors.into_iter().flatten().filter(|&class| class < 2) {
                votes[class as usize] += 1;
            }
            *pixel = MIX_PALETTE[usize::from(votes[1] > votes[0])];
        }
    });
}

#[test]
fn forest_underlay_extends_the_neighboring_class_into_bare_ground() {
    const SIDE: usize = 128;
    let gravel = 64 * SIDE + 64;
    for (class, expected) in [(0, MIX_PALETTE[0]), (1, MIX_PALETTE[1])] {
        let mut classes = vec![class; SIDE * SIDE];
        classes[gravel] = 2;
        let mut source: Vec<_> = classes.iter().map(|&c| MIX_PALETTE[c as usize]).collect();
        fill_adjacent_forest_under_bare(&mut source, &classes, SIDE);
        assert!(source.into_iter().all(|pixel| pixel == expected));
    }
}

#[test]
fn forest_underlay_chooses_categories_in_mixed_surroundings() {
    const SIDE: usize = 128;
    let mut classes: Vec<_> = (0..SIDE * SIDE)
        .map(|i| u8::from(i % SIDE < SIDE / 2))
        .collect();
    for x in 48..80 {
        classes[64 * SIDE + x] = 2;
    }
    let mut source: Vec<_> = classes.iter().map(|&c| MIX_PALETTE[c as usize]).collect();
    fill_adjacent_forest_under_bare(&mut source, &classes, SIDE);
    assert!(
        source
            .iter()
            .all(|p| *p == MIX_PALETTE[0] || *p == MIX_PALETTE[1])
    );
    assert_eq!(source[64 * SIDE + 54], MIX_PALETTE[1]);
    assert_eq!(source[64 * SIDE + 74], MIX_PALETTE[0]);
}

#[test]
fn forest_underlay_stops_after_one_pixel_and_cannot_jump_over_other() {
    let classes = [
        1, 2, 2, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    let mut source: Vec<_> = classes.iter().map(|&c| MIX_PALETTE[c as usize]).collect();
    fill_adjacent_forest_under_bare(&mut source, &classes, 5);
    assert_eq!(source[1], MIX_PALETTE[0]); // The forest vote is tied by other.
    let classes = [
        1, 2, 2, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    ];
    let mut source: Vec<_> = classes.iter().map(|&c| MIX_PALETTE[c as usize]).collect();
    fill_adjacent_forest_under_bare(&mut source, &classes, 5);
    assert_eq!(source[1], MIX_PALETTE[1]);
    assert_eq!(source[2], MIX_PALETTE[0]);
    assert_eq!(source[4], MIX_PALETTE[0]);
    assert_eq!(source[6], MIX_PALETTE[0]); // Diagonal contact cannot seed forest.
}

#[test]
fn categorical_water_underlay_uses_all_three_land_classes() {
    const SIDE: usize = 128;
    let water = 64 * SIDE + 64;
    for class in 0..3 {
        let mut classes = vec![class; SIDE * SIDE];
        classes[water] = 3;
        let mut source: Vec<_> = classes.iter().map(|&c| MIX_PALETTE[c as usize]).collect();
        fill_categorical_underlay(&mut source, &classes, SIDE, 3);
        assert!(
            source
                .into_iter()
                .all(|pixel| pixel == MIX_PALETTE[class as usize])
        );
    }
    let mut source = vec![[0; 4]; SIDE * SIDE];
    fill_categorical_underlay(&mut source, &vec![3; SIDE * SIDE], SIDE, 3);
    assert!(source.into_iter().all(|pixel| pixel == MIX_PALETTE[0]));
}

fn ordered_source_pixel(
    order: CompositeOrder,
    class: u8,
    neighbors: [Option<u8>; 4],
    is_ocean: bool,
) -> [u8; 4] {
    if class == 4 {
        return [0; 4];
    }
    let mut votes = [0; 4];
    for neighbor in neighbors
        .into_iter()
        .flatten()
        .filter(|&neighbor| neighbor < 4)
    {
        votes[neighbor as usize] += 1;
    }
    let forest = !is_ocean && (class == 1 || (class >= 2 && votes[1] > votes[0]));
    let water = class == 3
        || (order == CompositeOrder::BareOverWater && class == 2 && votes[3] > votes[0] + votes[1]);
    let bare = class == 2
        || (order == CompositeOrder::WaterOverBare && class == 3 && votes[2] > votes[0] + votes[1]);
    [
        255 * u8::from(forest),
        255 * u8::from(water),
        255 * u8::from(bare),
        255,
    ]
}

#[test]
fn ordered_underlays_respect_source_adjacency_and_coverage() {
    let order = CompositeOrder::BareOverWater;
    let pixel = |class, neighbors| ordered_source_pixel(order, class, neighbors, false);
    assert_eq!(pixel(2, [Some(3), None, Some(1), None]), [255, 0, 255, 255]);
    assert_eq!(
        pixel(2, [Some(3), Some(3), Some(1), None]),
        [255, 255, 255, 255]
    );
    assert_eq!(pixel(2, [Some(2); 4]), [0, 0, 255, 255]);
    assert_eq!(pixel(3, [Some(1); 4]), [255, 255, 0, 255]);
    assert_eq!(pixel(4, [Some(1); 4]), [0; 4]);
}

#[test]
fn water_top_keeps_water_mask_and_fills_adjacent_bare_below_water() {
    let pixel = |class, neighbors| {
        ordered_source_pixel(CompositeOrder::WaterOverBare, class, neighbors, false)
    };
    assert_eq!(
        pixel(3, [Some(2), Some(2), Some(1), None]),
        [255, 255, 255, 255]
    );
    assert_eq!(pixel(3, [Some(2), Some(0), None, None]), [0, 255, 0, 255]);
    assert_eq!(pixel(3, [Some(3); 4]), [0, 255, 0, 255]);
    assert_eq!(pixel(2, [Some(3); 4]), [0, 0, 255, 255]);
}

const COAST_FOREST_CLOSE_RADIUS: usize = 8;
const COAST_FILL_CANDIDATE: u8 = 1;

// 元の森林に接する候補から、候補上だけを上下左右にたどる。
// 到達距離を制限することで、区画の余白内だけで同じ結果を決められる。
fn keep_connected_coastal_fill(
    src: &mut [[u8; 4]],
    classes: &[u8],
    side: usize,
    max_distance: usize,
) {
    let mut frontier = Vec::new();
    for (i, pixel) in src.iter_mut().enumerate() {
        if pixel[0] == COAST_FILL_CANDIDATE
            && cardinal_neighbors(i, side)
                .into_iter()
                .flatten()
                .any(|j| classes[j] == 1)
        {
            pixel[0] = 255;
            frontier.push(i);
        }
    }
    let mut next = Vec::new();
    for _ in 1..max_distance {
        if frontier.is_empty() {
            break;
        }
        for i in frontier.drain(..) {
            for j in cardinal_neighbors(i, side).into_iter().flatten() {
                if src[j][0] == COAST_FILL_CANDIDATE {
                    src[j][0] = 255;
                    next.push(j);
                }
            }
        }
        std::mem::swap(&mut frontier, &mut next);
    }
    src.par_iter_mut().for_each(|pixel| {
        if pixel[0] == COAST_FILL_CANDIDATE {
            pixel[0] = 0;
        }
    });
}

#[test]
fn coastal_fill_rejects_detached_and_diagonal_only_candidates() {
    let side = 8;
    let mut classes = vec![3; side * side];
    let mut src = vec![[0, 255, 0, 255]; side * side];
    classes[side + 1] = 1;
    src[side + 1] = [255, 0, 0, 255];
    let attached = [side + 2, side + 3];
    let detached = [2 * side, 4 * side + 3, 4 * side + 4];
    for i in attached.into_iter().chain(detached) {
        src[i][0] = COAST_FILL_CANDIDATE;
    }
    keep_connected_coastal_fill(&mut src, &classes, side, 8);
    for i in attached {
        assert_eq!(src[i], [255, 255, 0, 255]);
    }
    for i in detached {
        assert_eq!(src[i], [0, 255, 0, 255]);
    }
}

// 直交・斜めをそれぞれ 3・4 として、前後2回の走査で近傍距離を求める。
// 区画外の値が必要な外周は、書き出し範囲の外側の余白にある。
fn coastal_mask_distances(mask: &mut [u8], side: usize) {
    for y in 1..side - 1 {
        for x in 1..side - 1 {
            let i = y * side + x;
            if mask[i] != 0 {
                mask[i] = mask[i]
                    .min(mask[i - 1].min(mask[i - side]) + 3)
                    .min(mask[i - side - 1].min(mask[i - side + 1]) + 4);
            }
        }
    }
    for y in (1..side - 1).rev() {
        for x in (1..side - 1).rev() {
            let i = y * side + x;
            if mask[i] != 0 {
                mask[i] = mask[i]
                    .min(mask[i + 1].min(mask[i + side]) + 3)
                    .min(mask[i + side - 1].min(mask[i + side + 1]) + 4);
            }
        }
    }
}

fn close_coastal_forest(src: &mut [[u8; 4]], classes: &[u8], land: &[u8], side: usize) {
    if !land.contains(&0) || !classes.contains(&1) {
        return;
    }
    let radius = (3 * COAST_FOREST_CLOSE_RADIUS) as u8;
    let outside = radius + 1;
    let mut distance: Vec<_> = classes
        .par_iter()
        .map(|&class| if class == 1 { 0 } else { outside })
        .collect();
    coastal_mask_distances(&mut distance, side);
    distance.par_iter_mut().for_each(|d| {
        *d = if *d <= radius { outside } else { 0 };
    });
    coastal_mask_distances(&mut distance, side);
    // 膨張と収縮で残った凹みのうち、水域で覆う海側の画素だけを補う。
    src.par_iter_mut()
        .zip(land)
        .zip(distance)
        .for_each(|((pixel, &land), distance)| {
            if land == 0 && distance > radius {
                pixel[0] = COAST_FILL_CANDIDATE;
            }
        });
    keep_connected_coastal_fill(src, classes, side, 2 * COAST_FOREST_CLOSE_RADIUS);
}

#[test]
fn coastal_forest_closes_a_narrow_inlet_without_extending_the_open_coast() {
    let side = 80;
    let mut classes = vec![3; side * side];
    let mut land = vec![0; side * side];
    let mut src = vec![[0, 255, 0, 255]; side * side];
    for y in 0..side {
        for x in 40..side {
            if (33..39).contains(&y) && x < 52 {
                continue;
            }
            let i = y * side + x;
            classes[i] = 1;
            land[i] = 1;
            src[i] = [255, 0, 0, 255];
        }
    }
    close_coastal_forest(&mut src, &classes, &land, side);
    assert_eq!(src[36 * side + 46], [255, 255, 0, 255]);
    assert_eq!(src[36 * side + 39], [0, 255, 0, 255]);
    assert_eq!(src[19 * side + 39], [0, 255, 0, 255]);
    assert_eq!(src[19 * side + 40], [255, 0, 0, 255]);
}

fn cardinal_neighbors(i: usize, side: usize) -> [Option<usize>; 4] {
    let (x, y) = (i % side, i / side);
    [
        (x > 0).then(|| i - 1),
        (x + 1 < side).then(|| i + 1),
        (y > 0).then(|| i - side),
        (y + 1 < side).then(|| i + side),
    ]
}

fn remove_isolated_classes(classes: &[u8], land: &[u8], side: usize, output: &mut [u8]) {
    output
        .par_chunks_mut(side)
        .enumerate()
        .for_each(|(y, row)| {
            let input = &classes[y * side..][..side];
            row.copy_from_slice(input);
            if y == 0 || y + 1 == side {
                return;
            }
            for x in 1..side - 1 {
                let class = input[x];
                let i = y * side + x;
                if !(1..=3).contains(&class) || land[i] == 0 {
                    continue;
                }
                let neighbors = [
                    i - side - 1,
                    i - side,
                    i - side + 1,
                    i - 1,
                    i + 1,
                    i + side - 1,
                    i + side,
                    i + side + 1,
                ];
                if neighbors.iter().any(|&j| classes[j] == class) {
                    continue;
                }
                let mut votes = [0; 4];
                for j in neighbors {
                    if land[j] != 0 && classes[j] < 4 {
                        votes[classes[j] as usize] += 1;
                    }
                }
                let winner = (0..votes.len()).max_by_key(|&j| votes[j]).unwrap();
                if votes[winner] >= 2
                    && (0..votes.len()).all(|j| j == winner || votes[j] < votes[winner])
                {
                    row[x] = winner as u8;
                }
            }
        });
}

#[test]
fn isolated_filter_removes_singletons_and_keeps_diagonal_water_and_ocean() {
    let side = 9;
    let mut classes = vec![0; side * side];
    let mut land = vec![1; side * side];
    let [forest, bare, water, stream_a, stream_b, sea] = [10, 13, 16, 46, 56, 61];
    for (i, class) in [
        (forest, 1),
        (bare, 2),
        (water, 3),
        (stream_a, 3),
        (stream_b, 3),
        (sea, 3),
    ] {
        classes[i] = class;
    }
    land[sea] = 0;
    let mut out = vec![0; classes.len()];
    remove_isolated_classes(&classes, &land, side, &mut out);
    assert_eq!([out[forest], out[bare], out[water]], [0; 3]);
    assert_eq!([out[stream_a], out[stream_b], out[sea]], [3; 3]);
}

const URBAN_FOREST_MAX_PIXELS: usize = 256;
const URBAN_FOREST_RADIUS: usize = 24;
const URBAN_FOREST_MIN_BUILT_PERCENT: usize = 15;

fn suppress_urban_forest(classes: &mut [u8], built: &[u8], side: usize) -> usize {
    use std::collections::VecDeque;

    if !built.contains(&1) {
        return 0;
    }
    let stride = side + 1;
    let mut integral = vec![0u32; stride * stride];
    for y in 0..side {
        let mut sum = 0;
        for x in 0..side {
            sum += u32::from(built[y * side + x]);
            integral[(y + 1) * stride + x + 1] = integral[y * stride + x + 1] + sum;
        }
    }
    let urban = |i: usize| {
        let (x, y) = (i % side, i / side);
        let r = URBAN_FOREST_RADIUS;
        let (x0, y0) = (x.saturating_sub(r), y.saturating_sub(r));
        let (x1, y1) = ((x + r + 1).min(side), (y + r + 1).min(side));
        let at = |x, y| integral[y * stride + x];
        let count = at(x1, y1) + at(x0, y0) - at(x1, y0) - at(x0, y1);
        count as usize * 100 >= (x1 - x0) * (y1 - y0) * URBAN_FOREST_MIN_BUILT_PERCENT
    };

    let mut seen = vec![false; classes.len()];
    let mut queue = VecDeque::new();
    let mut component = Vec::with_capacity(URBAN_FOREST_MAX_PIXELS + 1);
    let mut removed = 0;
    for start in 0..classes.len() {
        if classes[start] != 1 || seen[start] {
            continue;
        }
        component.clear();
        seen[start] = true;
        queue.push_back(start);
        let mut count = 0;
        let mut border = false;
        while let Some(i) = queue.pop_front() {
            count += 1;
            if count <= URBAN_FOREST_MAX_PIXELS {
                component.push(i);
            }
            let (x, y) = (i % side, i / side);
            border |= x == 0 || y == 0 || x + 1 == side || y + 1 == side;
            for row in y.saturating_sub(1)..=(y + 1).min(side - 1) {
                for col in x.saturating_sub(1)..=(x + 1).min(side - 1) {
                    let j = row * side + col;
                    if classes[j] == 1 && !seen[j] {
                        seen[j] = true;
                        queue.push_back(j);
                    }
                }
            }
        }
        if !border
            && count <= URBAN_FOREST_MAX_PIXELS
            && component.iter().filter(|&&i| urban(i)).count() * 2 > count
        {
            for &i in &component {
                classes[i] = 0;
            }
            removed += count;
        }
    }
    removed
}

#[test]
fn urban_forest_filter_removes_small_patches_and_preserves_large_or_rural_patches() {
    let side = 64;
    let mut classes = vec![0; side * side];
    let mut built = vec![0; side * side];
    for y in 0..side {
        built[y * side..][..27].fill(1);
    }
    for y in 20..38 {
        classes[y * side + 10..][..18].fill(1);
    }
    classes[6 * side + 8..][..2].fill(1);
    classes[6 * side + 55..][..2].fill(1);
    assert_eq!(suppress_urban_forest(&mut classes, &built, side), 2);
    assert_eq!(&classes[6 * side + 8..][..2], &[0, 0]);
    assert_eq!(classes[25 * side + 14], 1);
    assert_eq!(classes[6 * side + 55], 1);
}

const INLAND_FOREST_MAX_REACH: usize = 8;

fn median_water_forest(
    src: &mut [[u8; 4]],
    classes: &[u8],
    land: &[u8],
    side: usize,
    radius: usize,
) {
    if !classes.contains(&3) || !classes.contains(&1) {
        return;
    }
    let stride = side + 1;
    let mut integral = vec![[0u32; 3]; stride * stride];
    for (y, row) in classes.chunks_exact(side).enumerate() {
        let mut sum = [0; 3];
        for (x, &class) in row.iter().enumerate() {
            sum[0] += u32::from(class == 1);
            sum[1] += u32::from(land[y * side + x] != 0);
            sum[2] += u32::from(class < 3);
            let above = integral[y * stride + x + 1];
            integral[(y + 1) * stride + x + 1] =
                std::array::from_fn(|channel| sum[channel] + above[channel]);
        }
    }
    for (inland, reach) in [
        (false, 2 * radius),
        (true, INLAND_FOREST_MAX_REACH.min(radius)),
    ] {
        src.par_chunks_mut(side).enumerate().for_each(|(y, row)| {
            if y < radius || y + radius >= side {
                return;
            }
            let above = (y - radius) * stride;
            let below = (y + radius + 1) * stride;
            for (x, pixel) in row.iter_mut().enumerate().take(side - radius).skip(radius) {
                let i = y * side + x;
                if classes[i] != 3 || (land[i] != 0) != inland || pixel[0] != 0 {
                    continue;
                }
                let left = x - radius;
                let right = x + radius + 1;
                let count = |channel: usize| {
                    integral[below + right][channel] + integral[above + left][channel]
                        - integral[below + left][channel]
                        - integral[above + right][channel]
                };
                // 河川では両岸の乾いた陸地だけを票にする。
                if 2 * count(0) > count(if inland { 2 } else { 1 }) {
                    pixel[0] = COAST_FILL_CANDIDATE;
                }
            }
        });
        keep_connected_coastal_fill(src, classes, side, reach);
    }
}

#[test]
fn median_forest_fills_water_but_preserves_land_classes() {
    let side = 80;
    let mut classes = vec![3; side * side];
    let mut land = vec![0; side * side];
    let mut src = vec![[0, 255, 0, 255]; side * side];
    for y in 0..side {
        for x in 40..side {
            let i = y * side + x;
            if (33..39).contains(&y) && x < 52 {
                continue;
            }
            classes[i] = 1;
            land[i] = 1;
            src[i] = [255, 0, 0, 255];
        }
    }
    let field = 37 * side + 45;
    classes[field] = 0;
    land[field] = 1;
    src[field] = [0, 0, 0, 255];
    median_water_forest(&mut src, &classes, &land, side, 8);
    assert_eq!(src[36 * side + 46], [255, 255, 0, 255]);
    assert_eq!(src[19 * side + 39], [255, 255, 0, 255]);
    assert_eq!(src[19 * side + 32], [255, 255, 0, 255]);
    assert_eq!(src[19 * side + 31], [0, 255, 0, 255]);
    assert_eq!(src[field], [0, 0, 0, 255]);
}

#[test]
fn median_inland_forest_extends_from_the_bank_without_filling_distant_water() {
    let side = 96;
    let classes: Vec<_> = (0..side * side)
        .map(|i| if i % side < 40 { 1 } else { 3 })
        .collect();
    let mut src: Vec<_> = classes
        .iter()
        .map(|&c| {
            if c == 1 {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 255]
            }
        })
        .collect();
    median_water_forest(&mut src, &classes, &vec![1; classes.len()], side, 32);
    assert_eq!(src[48 * side + 47], [255, 255, 0, 255]);
    assert_eq!(src[48 * side + 48], [0, 255, 0, 255]);
}

// 穴埋め前の森林を起点に、4ビットの濃さごとに候補を伝播させる。
fn connected_forest_levels(
    original: &mut [u8],
    filled: &[u8],
    solid_only: impl Fn(usize) -> bool,
    side: usize,
) {
    let mut pending: [Vec<usize>; 16] = std::array::from_fn(|_| Vec::new());
    for (i, (&before, &after)) in original.iter().zip(filled).enumerate() {
        if after > before {
            let adjacent = cardinal_neighbors(i, side)
                .into_iter()
                .flatten()
                .map(|j| original[j])
                .max()
                .unwrap_or(0);
            let level = after.min(adjacent);
            if level > before {
                pending[level as usize].push(i);
            }
        }
    }
    for level in (1..16).rev() {
        while let Some(i) = pending[level].pop() {
            if original[i] >= level as u8 || (level != 15 && solid_only(i)) {
                continue;
            }
            original[i] = level as u8;
            for j in cardinal_neighbors(i, side).into_iter().flatten() {
                let next = filled[j].min(level as u8);
                if next > original[j] {
                    pending[next as usize].push(j);
                }
            }
        }
    }
}

#[test]
fn quantized_forest_fill_cannot_be_darker_than_its_connection() {
    let mut original = [12, 0, 0, 0, 0, 0, 0, 0, 0];
    let filled = [12, 3, 10, 0, 0, 0, 0, 0, 14];
    connected_forest_levels(&mut original, &filled, |_| false, 3);
    assert_eq!(original, [12, 3, 3, 0, 0, 0, 0, 0, 0]);
    let mut original = [15, 0, 0, 0, 0, 0, 0, 0, 0];
    let filled = [15, 3, 15, 15, 0, 0, 0, 0, 0];
    connected_forest_levels(&mut original, &filled, |i| i == 2 || i == 3, 3);
    assert_eq!(original, [15, 3, 0, 15, 0, 0, 0, 0, 0]);
}

// 完全に水域で覆われる下地では、半径3画素の円内の多数決で細かな凹凸を落とす。
// 追加する色は不透明な森林色だけとし、元の森林濃度はそのまま残す。
fn compact_forest_under_water(quantized: &[[u8; 3]], side: usize) -> Vec<u8> {
    quantized
        .par_iter()
        .enumerate()
        .map(|(i, &[before, after, water])| {
            if water != 15 || after <= before {
                return after;
            }
            let (x, y) = (i % side, i / side);
            if x < 3 || x + 3 >= side || y < 3 || y + 3 >= side {
                return before;
            }
            let mut votes = 0;
            for (row, radius) in (y - 3..=y + 3).zip([0, 2, 2, 3, 2, 2, 0]) {
                votes += quantized[row * side + x - radius..=row * side + x + radius]
                    .iter()
                    .filter(|pixel| pixel[1] >= 8)
                    .count();
            }
            if votes >= 15 { 15 } else { before }
        })
        .collect()
}

#[test]
fn compact_forest_under_water_drops_isolated_faint_pixels_and_keeps_solid_color() {
    let side = 11;
    let mut pixels = vec![[0, 0, 15]; side * side];
    let i = 5 * side + 5;
    pixels[i] = [0, 10, 15];
    assert_eq!(compact_forest_under_water(&pixels, side)[i], 0);
    for y in 3..=7 {
        for x in 3..=7 {
            pixels[y * side + x][1] = 10;
        }
    }
    assert_eq!(compact_forest_under_water(&pixels, side)[i], 15);
    pixels[i][2] = 14;
    assert_eq!(compact_forest_under_water(&pixels, side)[i], 10);
}

fn forest_unpremultiply(value: u8, alpha: u8) -> u8 {
    ((u32::from(value) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255) as u8
}

fn forest_gray4(value: u8) -> u8 {
    ((u16::from(value) + 8) / 17) as u8
}

const FULL_INLAND_MIN_FOREST_PERCENT: usize = 85;
const FULL_INLAND_MAX_SIDE: usize = 320;

struct InlandWater {
    shore: [usize; 2],
    complete: bool,
    span: [usize; 2],
}

fn scan_inland_water(
    classes: &[u8],
    land: &[u8],
    side: usize,
    seed: usize,
    seen: &mut [bool],
    water: &mut Vec<usize>,
) -> InlandWater {
    seen[seed] = true;
    water.clear();
    water.push(seed);
    let mut shore = [0; 2];
    let mut complete = true;
    let mut min = [seed % side, seed / side];
    let mut max = min;
    let mut head = 0;
    while head < water.len() {
        let i = water[head];
        for (axis, value) in [i % side, i / side].into_iter().enumerate() {
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
        head += 1;
        for neighbor in cardinal_neighbors(i, side) {
            let Some(j) = neighbor else {
                complete = false;
                continue;
            };
            if land[j] == 0 {
                complete = false;
            } else if classes[j] == 3 {
                if !seen[j] {
                    seen[j] = true;
                    water.push(j);
                }
            } else {
                shore[usize::from(classes[j] == 1)] += 1;
            }
        }
    }
    InlandWater {
        shore,
        complete,
        span: std::array::from_fn(|axis| max[axis] - min[axis] + 1),
    }
}

fn inland_water_component(
    classes: &[u8],
    land: &[u8],
    side: usize,
    seed: usize,
) -> Result<(Vec<usize>, [usize; 2])> {
    ensure!(
        classes[seed] == 3 && land[seed] != 0,
        "指定点は内陸水域にありません"
    );
    let mut seen = vec![false; classes.len()];
    let mut water = Vec::new();
    let body = scan_inland_water(classes, land, side, seed, &mut seen, &mut water);
    ensure!(
        body.complete,
        "指定した水域が海か作業区画の外周につながっています"
    );
    ensure!(body.shore[1] > 0, "指定した水域に森林が接していません");
    Ok((water, body.shore))
}

fn forested_inland_water(classes: &[u8], land: &[u8], side: usize) -> (Vec<usize>, usize) {
    let mut seen = vec![false; classes.len()];
    let mut body = Vec::new();
    let mut full = Vec::new();
    let mut count = 0;
    for (i, (&class, &land_pixel)) in classes.iter().zip(land).enumerate() {
        if class != 3 || land_pixel == 0 || seen[i] {
            continue;
        }
        let water = scan_inland_water(classes, land, side, i, &mut seen, &mut body);
        let [other, forest] = water.shore;
        if water.complete
            && water
                .span
                .into_iter()
                .all(|size| size <= FULL_INLAND_MAX_SIDE)
            && forest > 0
            && 100 * forest >= FULL_INLAND_MIN_FOREST_PERCENT * (other + forest)
        {
            count += 1;
            full.extend_from_slice(&body);
        }
    }
    (full, count)
}

#[test]
fn forested_inland_water_requires_a_forest_dominated_non_marine_shore() {
    let mut classes = [1; 49];
    classes[16] = 3;
    classes[18] = 3;
    classes[19] = 0;
    let mut land = [1; 49];
    assert_eq!(forested_inland_water(&classes, &land, 7), (vec![16], 1));
    land[15] = 0;
    assert_eq!(forested_inland_water(&classes, &land, 7), (vec![], 0));
}

#[test]
fn full_inland_underlay_stays_in_the_selected_water_body() {
    let mut classes = [1; 25];
    for i in [6, 7, 18] {
        classes[i] = 3;
    }
    let mut land = [1; 25];
    assert_eq!(
        inland_water_component(&classes, &land, 5, 6).unwrap().0,
        [6, 7]
    );
    land[7] = 0;
    assert!(inland_water_component(&classes, &land, 5, 6).is_err());
}

fn lpf_levels(
    classes: &[u8],
    land: &[u8],
    mut n: usize,
    layer: Option<&str>,
    source_water_fill: bool,
    indexed_land: bool,
    forest_underlay: bool,
    categorical_water_fill: bool,
    no_land_underlay: bool,
    composite_order: Option<CompositeOrder>,
    forest_weights: Option<&[u8]>,
    coastal_forest_median_radius: Option<u32>,
    full_inland_water: &[usize],
    zooms: &[u32],
    work_zoom: u32,
) -> Vec<(u32, usize, Vec<u8>)> {
    let source_side = n;
    let halo = (n - CORE) / 2;
    let palette = if indexed_land { &MIX_PALETTE } else { &PALETTE };
    // The palette uses binary alpha and zero RGB for unknown samples, so it
    // already contains premultiplied colors.
    let mut src: Vec<_> = classes
        .par_iter()
        .enumerate()
        .map(|(i, &class)| {
            if let Some(order) = composite_order {
                return ordered_source_pixel(
                    order,
                    class,
                    cardinal_neighbors(i, n).map(|j| j.map(|j| classes[j])),
                    land[i] == 0,
                );
            }
            match layer {
                Some("land") if class == 3 && no_land_underlay => MIX_PALETTE[0],
                Some("land") if class == 3 => [0; 4],
                Some("water") if class != 3 => [0; 4],
                _ => palette[class as usize],
            }
        })
        .collect();
    if layer == Some("land") && !no_land_underlay && composite_order.is_none() {
        if categorical_water_fill {
            fill_categorical_underlay(&mut src, classes, n, 3);
        } else {
            fill_water_underlay(&mut src, classes, land, n, source_water_fill, palette[0]);
            if indexed_land {
                fill_deep_source_water(&mut src, classes, n, SOURCE_WATER_GUARD);
            }
        }
        if forest_underlay {
            fill_adjacent_forest_under_bare(&mut src, classes, n);
        }
    }
    let mut unfilled =
        coastal_forest_median_radius.map(|_| src.par_iter().map(|p| [p[0]]).collect::<Vec<_>>());
    if let Some(radius) = coastal_forest_median_radius {
        median_water_forest(&mut src, classes, land, n, radius as usize);
    } else if composite_order.is_some() {
        close_coastal_forest(&mut src, classes, land, n);
    }
    for &i in full_inland_water {
        src[i][0] = 255;
    }
    if let Some(weights) = forest_weights {
        let weight_side = n / 2;
        src.par_chunks_mut(n).enumerate().for_each(|(y, row)| {
            for (x, pixel) in row.iter_mut().enumerate() {
                if pixel[0] != 0 {
                    let weight = weights[(y / 2) * weight_side + x / 2];
                    pixel[0] = ((u16::from(pixel[0]) * u16::from(weight) + 127) / 255) as u8;
                }
            }
        });
    }
    if let (Some(base), Some(weights)) = (&mut unfilled, forest_weights) {
        let weight_side = n / 2;
        base.par_chunks_mut(n).enumerate().for_each(|(y, row)| {
            for (x, [value]) in row.iter_mut().enumerate() {
                let weight = weights[(y / 2) * weight_side + x / 2];
                *value = ((u16::from(*value) * u16::from(weight) + 127) / 255) as u8;
            }
        });
    }
    let mut levels = Vec::new();
    for step in 0..=6 {
        if step != 0 {
            src = lpf_half(&src, n);
            if let Some(base) = &mut unfilled {
                *base = lpf_half(base, n);
            }
            n /= 2;
        }
        let z = work_zoom + 4 - step as u32;
        if !zooms.contains(&z) {
            continue;
        }
        let scale = if z < work_zoom {
            0
        } else {
            1 << (z - work_zoom)
        };
        let center = if step == 0 { 0 } else { 1 << (step - 1) };
        let side = CORE >> step;
        let pad = halo >> step;
        let connected = unfilled.as_ref().map(|base| {
            let quantized: Vec<_> = src
                .par_iter()
                .zip(base)
                .enumerate()
                .map(|(i, (pixel, base))| {
                    let x = ((i % n) << step) + center;
                    let y = ((i / n) << step) + center;
                    let a = pixel[3];
                    if a == 0 || classes[y * source_side + x] == 4 {
                        return [0; 3];
                    }
                    [
                        forest_gray4(forest_unpremultiply(base[0], a)),
                        forest_gray4(forest_unpremultiply(pixel[0], a)),
                        forest_gray4(forest_unpremultiply(pixel[1], a)),
                    ]
                })
                .collect();
            let mut before: Vec<_> = quantized.iter().map(|p| p[0]).collect();
            let after = compact_forest_under_water(&quantized, n);
            connected_forest_levels(&mut before, &after, |i| quantized[i][2] == 15, n);
            before
        });
        let mut rgba = Vec::with_capacity(side * side * 4);
        for y in pad..pad + side {
            for (x, &[r, g, b, a]) in src[y * n + pad..y * n + pad + side].iter().enumerate() {
                let original_x = ((x + pad) << step) + center;
                let original_y = (y << step) + center;
                if layer == Some("water") {
                    let [r, g, b, _] = PALETTE[3];
                    rgba.extend_from_slice(&[r, g, b, a]);
                    continue;
                }
                let original = original_y * source_side + original_x;
                if layer == Some("land")
                    && !source_water_fill
                    && !categorical_water_fill
                    && !no_land_underlay
                    && composite_order.is_none()
                    && land[original] == 0
                {
                    rgba.extend_from_slice(&palette[0]);
                } else if (layer == Some("land") && (classes[original] == 4 || a == 0))
                    || (layer.is_none() && a < 128)
                {
                    rgba.extend_from_slice(&[0; 4]);
                } else {
                    let unpremultiply = |v| forest_unpremultiply(v, a);
                    let forest = connected.as_ref().map_or_else(
                        || unpremultiply(r),
                        |strengths| strengths[y * n + pad + x] * 17,
                    );
                    rgba.extend_from_slice(&[forest, unpremultiply(g), unpremultiply(b), 255]);
                }
            }
        }
        levels.push((z, scale, rgba));
    }
    levels
}

#[cfg(test)]
#[test]
fn lpf_reduces_checkerboard_to_middle_color() {
    let raster: Vec<_> = (0..8)
        .flat_map(|y| (0..8).map(move |x| [(255 * ((x + y) % 2)) as u8; 4]))
        .collect();
    let half = lpf_half(&raster, 8);
    for i in [5, 6, 9, 10] {
        assert_eq!(half[i], [128; 4]);
    }
}

fn encode_water_mask(alpha: impl Iterator<Item = u8>) -> Result<Vec<u8>> {
    encode_gray4(&pack_gray4(alpha)?)
}

fn pack_gray4(mut alpha: impl Iterator<Item = u8>) -> Result<Vec<u8>> {
    let mut packed = Vec::with_capacity(TILE * TILE / 2);
    while let Some(first) = alpha.next() {
        let second = alpha.next().context("Odd water mask width")?;
        packed.push((forest_gray4(first) << 4) | forest_gray4(second));
    }
    Ok(packed)
}

fn encode_gray4(packed: &[u8]) -> Result<Vec<u8>> {
    encode_gray_filtered(packed, png::BitDepth::Four, png::Filter::Adaptive)
}

fn encode_gray_filtered(
    packed: &[u8],
    depth: png::BitDepth,
    filter: png::Filter,
) -> Result<Vec<u8>> {
    ensure!(
        packed.len() == TILE * TILE * depth as usize / 8,
        "Invalid packed mask tile size"
    );
    let mut bytes = Vec::new();
    {
        let mut png = png::Encoder::new(&mut bytes, TILE as u32, TILE as u32);
        png.set_color(png::ColorType::Grayscale);
        png.set_depth(depth);
        png.set_compression(png::Compression::High);
        png.set_filter(filter);
        png.write_header()?.write_image_data(packed)?;
    }
    Ok(bytes)
}

const MASK_FILTERS: [png::Filter; 7] = [
    png::Filter::Adaptive,
    png::Filter::NoFilter,
    png::Filter::Sub,
    png::Filter::Up,
    png::Filter::Avg,
    png::Filter::Paeth,
    png::Filter::MinEntropy,
];

fn smallest_gray(
    packed: &[u8],
    depth: png::BitDepth,
    adaptive: Vec<u8>,
) -> Result<(Vec<u8>, usize)> {
    if packed.iter().all(|&byte| byte == packed[0]) {
        return Ok((adaptive, 0));
    }
    let mut best = (adaptive, 0);
    for (index, &filter) in MASK_FILTERS.iter().enumerate().skip(1) {
        let candidate = encode_gray_filtered(packed, depth, filter)?;
        if candidate.len() < best.0.len() {
            best = (candidate, index);
        }
    }
    Ok(best)
}

fn encode_png(tile: &Tile, water_mask: bool) -> Result<Vec<u8>> {
    if water_mask {
        return encode_water_mask(tile.rgba.chunks_exact(4).map(|pixel| pixel[3]));
    }
    let mut bytes = Vec::new();
    let opaque = tile.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255);
    {
        let mut png = png::Encoder::new(&mut bytes, TILE as u32, TILE as u32);
        png.set_color(if opaque {
            png::ColorType::Rgb
        } else {
            png::ColorType::Rgba
        });
        png.set_depth(png::BitDepth::Eight);
        png.set_compression(png::Compression::High);
        let mut writer = png.write_header()?;
        if opaque {
            let rgb: Vec<_> = tile
                .rgba
                .chunks_exact(4)
                .flat_map(|p| p[..3].iter().copied())
                .collect();
            writer.write_image_data(&rgb)?;
        } else {
            writer.write_image_data(&tile.rgba)?;
        }
    }
    Ok(bytes)
}

fn encode_land_index([other, forest, bare, alpha]: [u8; 4]) -> u8 {
    if alpha == 0 {
        return 255;
    }
    let components = [other, forest, bare].map(u16::from);
    let sum: u16 = components.iter().sum();
    if sum == 0 {
        return 0;
    }
    let scaled = components.map(|weight| weight * 21);
    let mut parts = scaled.map(|weight| weight / sum);
    let mut remainders = scaled.map(|weight| weight % sum);
    for _ in parts.iter().sum::<u16>()..21 {
        let i = (0..3).max_by_key(|&i| remainders[i]).unwrap();
        parts[i] += 1;
        remainders[i] = 0;
    }
    let [_, forest, bare] = parts;
    (forest * (45 - forest) / 2 + bare) as u8
}

#[test]
fn land_mix_index_covers_all_quantized_mixtures() {
    let mut seen = [false; 253];
    for forest in 0..=21 {
        for bare in 0..=21 - forest {
            let index = encode_land_index([21 - forest - bare, forest, bare, 255]);
            assert!(!std::mem::replace(&mut seen[index as usize], true));
        }
    }
    assert!(seen.into_iter().all(|v| v));
    assert_eq!(encode_land_index([0; 4]), 255);
}

#[test]
fn source_water_fill_keeps_a_guard_around_shore_and_missing_samples() {
    let mut classes = vec![3; 49];
    classes[3 * 7 + 3] = 4;
    let mut source = vec![[0; 4]; 49];
    fill_deep_source_water(&mut source, &classes, 7, 1);
    for y in 0..7usize {
        for x in 0..7usize {
            let eligible = (1..6).contains(&x)
                && (1..6).contains(&y)
                && (x.abs_diff(3) > 1 || y.abs_diff(3) > 1);
            assert_eq!(
                source[y * 7 + x],
                if eligible { MIX_PALETTE[0] } else { [0; 4] }
            );
        }
    }
}

fn encode_indexed_land(tile: &Tile) -> Result<Vec<u8>> {
    let indices: Vec<_> = tile
        .rgba
        .chunks_exact(4)
        .map(|pixel| encode_land_index(pixel.try_into().unwrap()))
        .collect();
    let mut bytes = Vec::new();
    {
        // A one-byte index plane. The shared color table lives in the DB format.
        let mut png = png::Encoder::new(&mut bytes, TILE as u32, TILE as u32);
        png.set_color(png::ColorType::Grayscale);
        png.set_depth(png::BitDepth::Eight);
        png.set_compression(png::Compression::High);
        png.write_header()?.write_image_data(&indices)?;
    }
    Ok(bytes)
}

fn encode(args: &Args, tile: &Tile) -> Result<Vec<u8>> {
    if args.forest_underlay_from.is_some() {
        return encode_water_mask(tile.rgba.chunks_exact(4).map(|pixel| pixel[1]));
    }
    if args.indexed_land {
        return encode_indexed_land(tile);
    }
    if args.png {
        return encode_png(tile, args.layer.as_deref() == Some("water"));
    }
    let stem = args
        .output
        .join("scratch")
        .join(format!("{}-{}-{}", tile.z, tile.x, tile.y));
    let png_path = stem.with_extension("png");
    let astc = stem.with_extension("astc");
    {
        let mut png = png::Encoder::new(fs::File::create(&png_path)?, TILE as u32, TILE as u32);
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.set_compression(png::Compression::Fast);
        png.write_header()?.write_image_data(&tile.rgba)?;
    }
    let status = hidden_command(&args.astcenc)
        .arg("-cl")
        .arg(&png_path)
        .arg(&astc)
        .args(["8x8", "-fast", "-j", "1", "-silent"])
        .output()?;
    ensure!(
        status.status.success(),
        "astcenc failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let bytes = fs::read(&astc)?;
    ensure!(
        bytes.len() == 16 + PAYLOAD
            && bytes[..16] == [0x13, 0xab, 0xa1, 0x5c, 8, 8, 1, 0, 1, 0, 0, 1, 0, 1, 0, 0],
        "Unexpected ASTC header"
    );
    fs::remove_file(png_path)?;
    fs::remove_file(astc)?;
    Ok(bytes[16..].to_vec())
}

fn save_ordered(pool: &rayon::ThreadPool, db: &mut Connection, tiles: &[Tile]) -> Result<()> {
    let encoded = pool.install(|| {
        tiles
            .par_iter()
            .map(|tile| {
                let channel = |c| -> Result<Vec<u8>> {
                    let packed = pack_gray4(tile.rgba.chunks_exact(4).map(|p| p[c]))?;
                    Ok(smallest_gray(&packed, png::BitDepth::Four, encode_gray4(&packed)?)?.0)
                };
                Ok([channel(0)?, channel(1)?, channel(2)?])
            })
            .collect::<Result<Vec<_>>>()
    })?;
    let tx = db.transaction()?;
    for (i, table) in ["tiles", "water_tiles", "bare_tiles"]
        .into_iter()
        .enumerate()
    {
        let mut insert = tx.prepare(&format!("INSERT INTO {table} VALUES (?,?,?,?)"))?;
        for (tile, layers) in tiles.iter().zip(&encoded) {
            insert.execute(params![tile.z, tile.x, tile.y, &layers[i]])?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn save(args: &Args, pool: &rayon::ThreadPool, db: &mut Connection, tiles: &[Tile]) -> Result<()> {
    if args.composite_order().is_some() {
        return save_ordered(pool, db, tiles);
    }
    let compressed = pool.install(|| {
        tiles
            .par_iter()
            .map(|tile| encode(args, tile))
            .collect::<Result<Vec<_>>>()
    })?;
    let tx = db.transaction()?;
    {
        let mut insert = tx.prepare("INSERT INTO tiles VALUES (?1,?2,?3,?4)")?;
        for (tile, data) in tiles.iter().zip(compressed) {
            insert.execute(params![tile.z, tile.x, tile.y, data])?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn gray4_pixels(payload: &[u8]) -> Result<Vec<u8>> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(payload)).read_info()?;
    let info = reader.info();
    ensure!(
        info.width == TILE as u32
            && info.height == TILE as u32
            && info.bit_depth == png::BitDepth::Four
            && info.color_type == png::ColorType::Grayscale,
        "Invalid 4-bit tile"
    );
    let mut packed = vec![0; TILE * TILE / 2];
    reader.next_frame(&mut packed)?;
    Ok(packed)
}

fn attach_forest_underlay(
    target: &mut Connection,
    path: &std::path::Path,
    report: &mut serde_json::Value,
) -> Result<()> {
    let source = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let manifest: String =
        source.query_row("SELECT value FROM metadata WHERE key='manifest'", [], |r| {
            r.get(0)
        })?;
    let mut base: serde_json::Value = serde_json::from_str(&manifest)?;
    ensure!(
        base["landEncoding"] == "forest-bare-over-gray4"
            && base["layer"] == "combined"
            && base["tilePixels"] == TILE
            && base["sourceGaps"] == report["sourceGaps"]
            && base["crs"] == report["crs"]
            && base["waterUnderlayFill"] == report["waterUnderlayFill"],
        "Expected compatible split 4-bit landcover SQLite"
    );
    let tx = target.transaction()?;
    tx.execute_batch("ALTER TABLE tiles RENAME TO generated_underlay")?;
    for table in [
        "tiles",
        "bare_tiles",
        "water_tiles",
        "forest_underlay_tiles",
    ] {
        tx.execute_batch(&format!("CREATE TABLE {table}(z INTEGER,x INTEGER,y INTEGER,payload BLOB NOT NULL,PRIMARY KEY(z,x,y)) WITHOUT ROWID"))?;
    }
    let mut levels = BTreeMap::<u32, [usize; 5]>::new();
    {
        let mut read = tx.prepare("SELECT z,x,y,payload FROM generated_underlay ORDER BY z,x,y")?;
        let mut generated = read.query([])?;
        let mut baseline = source.prepare(
            "SELECT f.payload,b.payload,w.payload FROM tiles f JOIN bare_tiles b USING(z,x,y)
            JOIN water_tiles w USING(z,x,y) WHERE f.z=? AND f.x=? AND f.y=?",
        )?;
        let mut insert_forest = tx.prepare("INSERT INTO tiles VALUES (?,?,?,?)")?;
        let mut insert_bare = tx.prepare("INSERT INTO bare_tiles VALUES (?,?,?,?)")?;
        let mut insert_water = tx.prepare("INSERT INTO water_tiles VALUES (?,?,?,?)")?;
        let mut insert_delta = tx.prepare("INSERT INTO forest_underlay_tiles VALUES (?,?,?,?)")?;
        while let Some(row) = generated.next()? {
            let (z, x, y): (u32, u32, u32) = (row.get(0)?, row.get(1)?, row.get(2)?);
            let mut rows = baseline.query(params![z, x, y])?;
            let reference = rows
                .next()?
                .with_context(|| format!("Missing baseline z{z}/{x}/{y}"))?;
            let forest = reference.get_ref(0)?.as_blob()?;
            let bare = reference.get_ref(1)?.as_blob()?;
            let water = reference.get_ref(2)?.as_blob()?;
            let mut delta = gray4_pixels(row.get_ref(3)?.as_blob()?)?;
            let base_forest = gray4_pixels(forest)?;
            let base_bare = gray4_pixels(bare)?;
            for ((byte, f), b) in delta.iter_mut().zip(base_forest).zip(base_bare) {
                // Missing source samples keep the existing sentinel and need no correction.
                let missing = f & b;
                let mask = (if b & 0xf0 == 0 || missing & 0xf0 == 0xf0 {
                    0
                } else {
                    0xf0
                }) | (if b & 0x0f == 0 || missing & 0x0f == 0x0f {
                    0
                } else {
                    0x0f
                });
                *byte = (*byte ^ f) & mask;
            }
            let level = levels.entry(z).or_default();
            level[0] += 1;
            level[1] += forest.len();
            level[2] += bare.len();
            level[3] += water.len();
            if delta.iter().any(|&v| v != 0) {
                let encoded = encode_gray4(&delta)?;
                level[4] += encoded.len();
                insert_delta.execute(params![z, x, y, encoded])?;
            }
            insert_forest.execute(params![z, x, y, forest])?;
            insert_bare.execute(params![z, x, y, bare])?;
            insert_water.execute(params![z, x, y, water])?;
        }
    }
    tx.execute_batch("DROP TABLE generated_underlay")?;
    tx.commit()?;
    for (key, water) in [("levels", false), ("waterLevels", true)] {
        if let Some(rows) = base[key].as_array_mut() {
            for row in rows {
                let z = row["z"].as_u64().context("Level zoom")? as u32;
                let [tiles, forest, bare, water_bytes, delta] = levels[&z];
                row["tiles"] = json!(tiles);
                row["payloadBytes"] = json!(if water {
                    water_bytes
                } else {
                    forest + bare + delta
                });
                if !water {
                    row["forestPayloadBytes"] = json!(forest);
                    row["barePayloadBytes"] = json!(bare);
                    row["forestUnderlayPayloadBytes"] = json!(delta);
                }
            }
        }
    }
    base["completeRegions"] = report["completeRegions"].clone();
    base["forestUnderlay"] = json!("source-lpf-gray4-xor");
    base["forestUnderlayFill"] = json!("adjacent-source-class-4");
    base["layers"]["forestUnderlay"] = json!("forest_underlay_tiles");
    *report = base;
    Ok(())
}

fn attach_water_layer(
    target: &mut Connection,
    path: &std::path::Path,
    report: &mut serde_json::Value,
) -> Result<()> {
    let source = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let manifest: String = source.query_row(
        "SELECT value FROM metadata WHERE key='manifest'",
        [],
        |row| row.get(0),
    )?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest)?;
    ensure!(
        manifest["layer"] == "combined"
            && manifest["waterEncoding"] == "gray4-coverage"
            && manifest["tilePixels"] == TILE
            && manifest["crs"] == report["crs"]
            && manifest["sourceGaps"] == report["sourceGaps"],
        "Expected compatible combined landcover SQLite with 4-bit water"
    );
    let tx = target.transaction()?;
    tx.execute_batch("CREATE TABLE water_tiles(z INTEGER,x INTEGER,y INTEGER,payload BLOB NOT NULL,PRIMARY KEY(z,x,y)) WITHOUT ROWID;")?;
    {
        let mut tiles = tx.prepare("SELECT z,x,y FROM tiles ORDER BY z,x,y")?;
        let mut rows = tiles.query([])?;
        let mut lookup =
            source.prepare("SELECT payload FROM water_tiles WHERE z=? AND x=? AND y=?")?;
        let mut insert = tx.prepare("INSERT INTO water_tiles VALUES (?,?,?,?)")?;
        while let Some(row) = rows.next()? {
            let (z, x, y): (u32, u32, u32) = (row.get(0)?, row.get(1)?, row.get(2)?);
            let payload: Vec<u8> = lookup
                .query_row(params![z, x, y], |row| row.get(0))
                .with_context(|| format!("Read water tile z{z}/{x}/{y}"))?;
            insert.execute(params![z, x, y, payload])?;
        }
    }
    let mut levels = report["levels"].clone();
    if let Some(rows) = levels.as_array_mut() {
        for row in rows {
            let bytes: u64 = tx.query_row(
                "SELECT sum(length(payload)) FROM water_tiles WHERE z=?",
                [row["z"].as_u64().context("Water level zoom")?],
                |r| r.get(0),
            )?;
            row["payloadBytes"] = json!(bytes);
        }
    }
    tx.commit()?;
    report["layer"] = json!("combined");
    report["layers"] = json!({"land":"tiles","water":"water_tiles"});
    report["waterEncoding"] = manifest["waterEncoding"].clone();
    report["waterLevels"] = levels;
    Ok(())
}

fn ordered_manifest(
    db: &Connection,
    report: &mut serde_json::Value,
    order: CompositeOrder,
) -> Result<()> {
    report["layer"] = json!("combined");
    report["layers"] = json!({"forest":"tiles","water":"water_tiles","bare":"bare_tiles"});
    report["landEncoding"] = json!("forest-bare-unified-gray4");
    report["waterEncoding"] = json!("gray4-coverage");
    report["maskPngEncoding"] = json!("smallest-filter");
    report["compositeOrder"] = json!(match order {
        CompositeOrder::BareOverWater => "forest-water-bare",
        CompositeOrder::WaterOverBare => "forest-bare-water",
    });
    report["forestUnderlayFill"] = json!("adjacent-source-class-4");
    report["coastalForestClosingRadiusSourcePixels"] = json!(COAST_FOREST_CLOSE_RADIUS);
    report["coastalForestConnectivity"] =
        json!("4-connected-to-original-forest-within-twice-radius");
    report["waterUnderlayFill"] = json!(if order == CompositeOrder::BareOverWater {
        "adjacent-source-class-4"
    } else {
        "none"
    });
    report["bareUnderlayFill"] = json!(if order == CompositeOrder::WaterOverBare {
        "adjacent-source-class-4"
    } else {
        "none"
    });
    report
        .as_object_mut()
        .context("Manifest is not an object")?
        .remove("sourceWaterGuardPixels");
    let mut water_levels = report["levels"].clone();
    for (land, water) in report["levels"]
        .as_array_mut()
        .context("Missing land levels")?
        .iter_mut()
        .zip(
            water_levels
                .as_array_mut()
                .context("Missing water levels")?,
        )
    {
        let z = land["z"].as_u64().context("Missing level zoom")?;
        let bytes = |table| {
            db.query_row(
                &format!("SELECT COALESCE(sum(length(payload)),0) FROM {table} WHERE z=?"),
                [z],
                |r| r.get::<_, u64>(0),
            )
        };
        let forest = land["payloadBytes"]
            .as_u64()
            .context("Missing forest size")?;
        let bare = bytes("bare_tiles")?;
        land["forestPayloadBytes"] = json!(forest);
        land["barePayloadBytes"] = json!(bare);
        land["payloadBytes"] = json!(forest + bare);
        water["payloadBytes"] = json!(bytes("water_tiles")?);
    }
    report["waterLevels"] = water_levels;
    Ok(())
}

pub(super) fn run() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.layer.is_none() || (args.png && args.lpf),
        "Layers require --png --lpf"
    );
    ensure!(
        !(args.source_water_fill || args.indexed_land) || args.layer.as_deref() == Some("land"),
        "--source-water-fill and --indexed-land require --layer land"
    );
    let gaps: Vec<Area> = serde_json::from_slice(&fs::read(&args.source_gaps)?)?;
    let gaps_set: BTreeSet<_> = gaps.iter().map(|a| (a.x, a.y)).collect();
    let mut areas: Vec<Area> = serde_json::from_slice(&fs::read(&args.plan)?)?;
    areas.retain(|a| {
        (args.esa_zero_land_dem.is_some() || !gaps_set.contains(&(a.x, a.y)))
            && args
                .only_area
                .as_ref()
                .is_none_or(|only| *only == format!("{}-{}", a.x, a.y))
    });
    ensure!(
        !areas.is_empty()
            && areas.iter().all(|a| (8..=9).contains(&a.work_zoom)
                && a.x < (1 << a.work_zoom)
                && a.y < (1 << a.work_zoom)),
        "Invalid region list"
    );
    areas.sort_by_key(|area| area.work_zoom);
    let has_fine = areas.iter().any(|area| area.work_zoom == 9);
    ensure!(
        !has_fine || args.include_l2,
        "高精細区画には --include-l2 が必要です"
    );
    let complete_coarse_tiles =
        args.only_area.is_none() && args.lpf && areas.iter().all(|a| a.max_zoom.is_some());
    fs::create_dir(&args.output).context("Use a new output directory")?;
    fs::create_dir(args.output.join("scratch"))?;
    let halo = if args.full_forested_inland_water {
        // 水域の全幅と、LPFおよび出力側の半径3画素の判定に必要な余白。
        (FULL_INLAND_MAX_SIDE + (2 + 3) * (CORE / (TILE / 4))).next_multiple_of(64)
    } else if args.suppress_urban_forest {
        // Include the longest eligible component, its context, and the LPF support.
        (URBAN_FOREST_MAX_PIXELS + URBAN_FOREST_RADIUS + LPF_HALO).next_multiple_of(64)
    } else if args.forest_underlay_from.is_some()
        || args.categorical_water_fill
        || args.no_land_underlay
    {
        2 * LPF_HALO
    } else if args.lpf {
        LPF_HALO
    } else {
        0
    };
    let sample_side = CORE + 2 * halo;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(args.jobs.max(1))
        .build()?;
    let composite_order = args.composite_order();
    let relief_dem = args
        .urban_forest_relief_dem
        .as_deref()
        .map(|path| urban_relief::ReliefDem::open_for_zoom(path, 11))
        .transpose()?;
    let zero_land_dem = args
        .esa_zero_land_dem
        .as_deref()
        .map(|path| urban_relief::ReliefDem::open_for_zoom(path, 12))
        .transpose()?;
    let filename = if relief_dem.is_some() {
        "landcover-national-urban-forest-relief.sqlite3"
    } else if args.suppress_urban_forest {
        "landcover-national-urban-forest-clean.sqlite3"
    } else if args.remove_isolated_source_pixels {
        "landcover-national-source-clean.sqlite3"
    } else if args.water_over_bare {
        "landcover-national-water-over-bare.sqlite3"
    } else if args.bare_over_water {
        "landcover-national-bare-over-water.sqlite3"
    } else if args.forest_underlay_from.is_some() {
        "landcover-national-forest-underlay.sqlite3"
    } else if args.png {
        "landcover-national-png.sqlite3"
    } else {
        "landcover-national-astc.sqlite3"
    };
    let temporary = args.output.join(format!("{filename}.partial"));
    let mut db = Connection::open(&temporary)?;
    let page_size = if args.layer.is_some() { 512 } else { 16384 };
    db.pragma_update(None, "page_size", page_size)?;
    db.execute_batch("CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,payload BLOB NOT NULL,PRIMARY KEY(z,x,y)) WITHOUT ROWID;
        CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;")?;
    if composite_order.is_some() {
        for table in ["water_tiles", "bare_tiles"] {
            db.execute_batch(&format!("CREATE TABLE {table}(z INTEGER,x INTEGER,y INTEGER,payload BLOB NOT NULL,PRIMARY KEY(z,x,y)) WITHOUT ROWID"))?;
        }
    }
    let mut coarse: BTreeMap<(u32, u32, u32), Vec<u8>> = BTreeMap::new();
    let mut classes = vec![0u8; sample_side * sample_side];
    let mut built = args.suppress_urban_forest.then(|| vec![0u8; classes.len()]);
    let mut cleaned = args
        .remove_isolated_source_pixels
        .then(|| vec![0u8; classes.len()]);
    let mut land = vec![0u8; classes.len()];
    let mut finished = 0;
    for group in areas.chunk_by(|a, b| a.work_zoom == b.work_zoom) {
        let work_zoom = group[0].work_zoom;
        let min_x = group.iter().map(|a| a.x).min().unwrap();
        let min_y = group.iter().map(|a| a.y).min().unwrap();
        let max_x = group.iter().map(|a| a.x).max().unwrap() + 1;
        let max_y = group.iter().map(|a| a.y).max().unwrap() + 1;
        let world_tiles = f64::from(1u32 << work_zoom);
        let edge = |v: u32| WORLD * (f64::from(v) / world_tiles - 0.5);
        let padding = WORLD / world_tiles / CORE as f64 * halo as f64;
        let extent = [
            edge(min_x) - padding,
            -edge(max_y) - padding,
            edge(max_x) + padding,
            -edge(min_y) + padding,
        ];
        let projection_dir = args.output.join(format!("source-z{work_zoom}"));
        fs::create_dir(&projection_dir)?;
        let projected = gdal::project_worldcover(
            &args.source,
            &projection_dir,
            extent,
            (max_x - min_x) as usize * CORE + 2 * halo,
            (max_y - min_y) as usize * CORE + 2 * halo,
        )?;
        for area in group {
            finished += 1;
            let x = area.x;
            let y = area.y;
            projected
                .read(
                    (x - min_x) as usize * CORE,
                    (y - min_y) as usize * CORE,
                    sample_side,
                    &mut classes,
                )
                .with_context(|| format!("Read z{work_zoom}/{x}/{y}"))?;
            // ESAの分類済み画素をそのまま使い、欠損画素だけ標高で補う。
            land.par_iter_mut().zip(&classes).for_each(|(land, &code)| {
                *land = u8::from(code != 0);
            });
            if let Some(dem) = &zero_land_dem {
                dem.mark_unclassified_land(x, y, CORE, halo, &classes, &mut land, work_zoom + 4)
                    .with_context(|| format!("Confirm NoData land z{work_zoom}/{x}/{y}"))?;
            }
            if let Some(built) = &mut built {
                built
                    .par_iter_mut()
                    .zip(&classes)
                    .for_each(|(built, &class)| *built = u8::from(class == 50));
            }
            for (code, land) in classes.iter_mut().zip(&mut land) {
                if *code == 0 {
                    let confirmed = *land == 2;
                    *land = u8::from(confirmed);
                    *code = if confirmed { 0 } else { 3 };
                } else {
                    *code = classify(*code);
                }
            }
            if let Some(cleaned) = &mut cleaned {
                pool.install(|| remove_isolated_classes(&classes, &land, sample_side, cleaned));
                std::mem::swap(&mut classes, cleaned);
            }
            let (automatic_full_water, full_water_count) = if args.full_forested_inland_water {
                forested_inland_water(&classes, &land, sample_side)
            } else {
                (Vec::new(), 0)
            };
            if args.full_forested_inland_water {
                println!(
                    "z{work_zoom}/{x}/{y}: full forest underlay for {full_water_count} inland bodies, {} source pixels including halo",
                    automatic_full_water.len()
                );
            }
            if let Some(built) = &built {
                let removed = suppress_urban_forest(&mut classes, built, sample_side);
                println!(
                    "z{work_zoom}/{x}/{y}: suppressed {removed} urban forest source pixels (including halo)"
                );
            }
            let forest_weights = match (&relief_dem, &built) {
                (Some(dem), Some(built)) if built.contains(&1) => Some(
                    dem.forest_weights(x, y, CORE, halo, built, &classes, work_zoom + 3)
                        .with_context(|| format!("Evaluate urban relief z{work_zoom}/{x}/{y}"))?,
                ),
                _ => None,
            };
            let full_inland_water = if let Some(point) = &args.full_inland_forest_at {
                let [lat, lon] = [point[0], point[1]];
                ensure!(
                    lat.abs() <= 85.05 && lon.abs() <= 180.,
                    "緯度経度が範囲外です"
                );
                let gx = (lon / 360. + 0.5) * (world_tiles * CORE as f64);
                let gy = (0.5 - lat.to_radians().tan().asinh() / std::f64::consts::TAU)
                    * (world_tiles * CORE as f64);
                let sx = gx.floor() as i64 - i64::from(x) * CORE as i64 + halo as i64;
                let sy = gy.floor() as i64 - i64::from(y) * CORE as i64 + halo as i64;
                let core = halo as i64..(halo + CORE) as i64;
                ensure!(
                    core.contains(&sx) && core.contains(&sy),
                    "指定点が対象区画の外にあります"
                );
                let (water, shore) = inland_water_component(
                    &classes,
                    &land,
                    sample_side,
                    sy as usize * sample_side + sx as usize,
                )?;
                println!(
                    "inland forest trial: {} source pixels, shore forest {}/{}",
                    water.len(),
                    shore[1],
                    shore[0] + shore[1]
                );
                water
            } else {
                automatic_full_water
            };
            let zooms = if work_zoom == 9 {
                &[13, 12][..]
            } else {
                args.zooms()
            };
            let zooms = &zooms[zooms.partition_point(|&z| z > area.max_zoom.unwrap_or(11))..];
            let mut levels = pool.install(|| {
                if args.lpf {
                    return lpf_levels(
                        &classes,
                        &land,
                        sample_side,
                        args.layer.as_deref(),
                        args.source_water_fill,
                        args.indexed_land,
                        args.forest_underlay_from.is_some(),
                        args.categorical_water_fill,
                        args.no_land_underlay,
                        composite_order,
                        forest_weights.as_deref(),
                        args.coastal_forest_median_radius,
                        &full_inland_water,
                        zooms,
                        work_zoom,
                    );
                }
                [(11, 8), (9, 2), (8, 1)]
                    .into_par_iter()
                    .map(|(z, scale)| {
                        (
                            z,
                            scale,
                            downsample(&classes, TILE * scale, args.area_average),
                        )
                    })
                    .collect::<Vec<_>>()
            });
            if !args.lpf {
                levels.push((6, 0, downsample(&classes, TILE / 4, args.area_average)));
            }
            let mut tiles = Vec::new();
            for (z, scale, rgba) in levels {
                if z < 8 {
                    let factor = 1 << (8 - z);
                    let side = TILE / factor as usize;
                    let image = coarse
                        .entry((z, x / factor, y / factor))
                        .or_insert_with(|| vec![0; TILE * TILE * 4]);
                    for row in 0..side {
                        let dst = (((y % factor) as usize * side + row) * TILE
                            + (x % factor) as usize * side)
                            * 4;
                        image[dst..dst + side * 4]
                            .copy_from_slice(&rgba[row * side * 4..(row + 1) * side * 4]);
                    }
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let mut pixels = Vec::with_capacity(TILE * TILE * 4);
                        for py in 0..TILE {
                            let start = ((dy * TILE + py) * TILE * scale + dx * TILE) * 4;
                            pixels.extend_from_slice(&rgba[start..start + TILE * 4]);
                        }
                        tiles.push(Tile {
                            z,
                            x: x * scale as u32 + dx as u32,
                            y: y * scale as u32 + dy as u32,
                            rgba: pixels,
                        });
                    }
                }
            }
            save(&args, &pool, &mut db, &tiles)
                .with_context(|| format!("Encode z{work_zoom}/{x}/{y}"))?;
            println!(
                "region {}/{} z{work_zoom}/{x}/{y}: {} raster tiles",
                finished,
                areas.len(),
                tiles.len()
            );
        }
    }
    let roots: Vec<_> = coarse
        .into_iter()
        .map(|((z, x, y), rgba)| Tile { z, x, y, rgba })
        .collect();
    save(&args, &pool, &mut db, &roots)?;
    let mut levels = Vec::new();
    let first_level = if args.include_l2 { 2 } else { 3 };
    let output_zooms = if has_fine {
        &[13, 12, 11, 10, 9, 8, 7, 6][..]
    } else {
        args.zooms()
    };
    for (level, &z) in (first_level..).zip(output_zooms) {
        let (count, bytes): (u64, u64) = db.query_row(
            "SELECT count(*),sum(length(payload)) FROM tiles WHERE z=?",
            [z],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        levels.push(json!({"level":level,"z":z,"tiles":count,"payloadBytes":bytes,"projectedMetresPerPixel":WORLD/f64::from(1u32<<z)/TILE as f64}));
    }
    let gaps: serde_json::Value = serde_json::from_slice(&fs::read(&args.source_gaps)?)?;
    let mut report = json!({"version":1,"format":if args.png {"PNG"} else {"ASTC_8x8_UNORM"},"tilePixels":TILE,
        "block":if args.png {None} else {Some([8,8])},
        "payload":if args.png {"PNG file bytes"} else {"raw ASTC blocks without file header"},"crs":"EPSG:3857","worldWidthM":WORLD,
        "orientation":"XYZ east/south","layer":args.layer,"waterEncoding":if args.layer.as_deref()==Some("water") {Some("gray4-coverage")} else {None},"inlandWaterUnderlay":args.layer.as_deref()==Some("land") && !args.no_land_underlay,"sourceSeaUnderlay":args.source_water_fill,"palette":PALETTE,"classes":["other","forest","bare","water","unknown"],
        "sampling":if args.lpf {"centered six-tap binomial LPF before each 2x reduction; 192 source texel halo"} else if args.area_average {"area-weighted RGB on shared z8 4096 raster"} else {"categorical area vote on shared z8 4096 raster"},"sourceGaps":gaps,
        "completeRegions":areas.len(),"levels":levels,"attribution":"© ESA WorldCover project 2021 / Contains modified Copernicus Sentinel data (2021) processed by ESA WorldCover consortium"});
    if complete_coarse_tiles {
        report["completeCoarseTiles"] = json!(true);
    }
    if args.indexed_land {
        report["landEncoding"] = json!(LAND_MIX_ENCODING);
        if args.categorical_water_fill || args.no_land_underlay {
            report["waterUnderlayFill"] = json!(if args.no_land_underlay {
                "none"
            } else {
                "categorical-majority"
            });
            report["sampling"] = json!(
                "centered six-tap binomial LPF before each 2x reduction; 384 source texel halo"
            );
        } else {
            report["sourceWaterGuardPixels"] = json!(SOURCE_WATER_GUARD);
        }
    }
    if let Some(order) = composite_order {
        ordered_manifest(&db, &mut report, order)?;
        if let Some(radius) = args.coastal_forest_median_radius {
            report
                .as_object_mut()
                .unwrap()
                .remove("coastalForestClosingRadiusSourcePixels");
            report["coastalForestMedianRadiusSourcePixels"] = json!(radius);
            report["coastalForestMedianVotes"] = json!("exclude-unclassified-water");
            report["inlandForestUnderlayFill"] = json!({
                "votes": "dry-land-forest-majority",
                "maxReachSourcePixels": INLAND_FOREST_MAX_REACH.min(radius as usize)
            });
            report["coastalForestLodConnectivity"] = json!("4-connected-at-each-gray4-threshold");
            report["coastalForestOpaqueWaterFill"] = json!("solid-forest-disk-majority-radius-3");
        }
    }
    if args.remove_isolated_source_pixels {
        report["sourceNoiseFilter"] = json!("isolated-8-neighbor-plurality");
    }
    if args.suppress_urban_forest {
        report["sampling"] = json!(format!(
            "centered six-tap binomial LPF before each 2x reduction; {halo} source texel halo"
        ));
        report["urbanForestFilter"] = json!({
            "maxComponentPixels": URBAN_FOREST_MAX_PIXELS,
            "builtUpRadiusPixels": URBAN_FOREST_RADIUS,
            "minBuiltUpPercent": URBAN_FOREST_MIN_BUILT_PERCENT
        });
    }
    if relief_dem.is_some() {
        report["urbanForestReliefFilter"] = json!({
            "demZ": 11, "reliefWindowPixels": 9, "minValidFraction": 0.8,
            "builtUpWindowPixels": 25, "builtUpPercent": [10, 25], "reliefMeters": [25, 8],
            "forestProtectionWindowPixels": 9, "forestProtectionPercent": [30, 60],
            "application": "forest coverage before LPF"
        });
    }
    if zero_land_dem.is_some() {
        report["sourceNoDataClassification"] = json!({
            "default": "water", "confirmedLand": "other", "demZoom": 12,
            "landEvidence": "positive-elevation",
            "includeEsaSourceGapRegions": true
        });
    }
    if args.full_forested_inland_water {
        report["fullInlandForestUnderlay"] = json!({
            "minShoreForestPercent": FULL_INLAND_MIN_FOREST_PERCENT,
            "maxBodySideSourcePixels": FULL_INLAND_MAX_SIDE,
            "connectivity": "inland-four-neighbor"
        });
    }
    if let Some(point) = &args.full_inland_forest_at {
        report["fullInlandForestTrialAt"] = json!(point);
    }
    if let Some(source) = &args.forest_underlay_from {
        attach_forest_underlay(&mut db, source, &mut report)?;
        db.execute_batch("VACUUM")?;
    }
    if let Some(water) = &args.reuse_water {
        attach_water_layer(&mut db, water, &mut report)?;
    }
    db.execute(
        "INSERT INTO metadata VALUES ('manifest',?)",
        [report.to_string()],
    )?;
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(integrity == "ok", "SQLite integrity: {integrity}");
    drop(db);
    let target = args.output.join(filename);
    fs::rename(&temporary, &target)?;
    fs::write(
        args.output.join("manifest.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("Complete: {}", target.display());
    Ok(())
}
