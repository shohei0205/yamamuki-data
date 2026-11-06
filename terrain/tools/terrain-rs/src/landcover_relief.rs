//! Generate a reference hillshade from cached GSI DEM PNG tiles.
mod bench_png;

use anyhow::{Result, ensure};
use clap::Parser;
use serde_json::Value;
use std::{collections::HashMap, fs, path::PathBuf};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    dem_cache: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value_t = 1536, value_parser = clap::value_parser!(u32).range(2..=4096))]
    size: u32,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let manifest: Value = serde_json::from_slice(&fs::read(&args.manifest)?)?;
    let [left, bottom, right, top]: [f64; 4] = serde_json::from_value(manifest["bounds"].clone())?;
    ensure!(right > left && top > bottom, "Invalid DEM bounds");
    let size = args.size as usize;
    let world = 2. * std::f64::consts::PI * 6378137.;
    let pixels_per_m = (16384 * 256) as f64 / world;
    let dx = (right - left) / (size - 1) as f64;
    let dy = (top - bottom) / (size - 1) as f64;
    let mut tiles = HashMap::new();
    let mut heights = vec![0.; size * size];
    let mut tile_row = None;
    let mut loaded_tiles = 0;
    for y in 0..size {
        let py = ((world / 2. - top + y as f64 * dy) * pixels_per_m) as usize;
        if tile_row != Some(py / 256) {
            tiles.clear();
            tile_row = Some(py / 256);
        }
        for x in 0..size {
            let px = ((left + x as f64 * dx + world / 2.) * pixels_per_m) as usize;
            let key = (px / 256, py / 256);
            let tile = match tiles.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let path = args.dem_cache.join("14").join(key.0.to_string()).join(format!("{}.png", key.1));
                    let tile = if path.is_file() {
                        let image = bench_png::read(&path)?;
                        ensure!(image.width == 256 && image.height == 256, "Invalid DEM tile size");
                        loaded_tiles += 1;
                        Some(image)
                    } else {
                        None
                    };
                    entry.insert(tile)
                }
            };
            if let Some(tile) = tile {
                let [r, g, b, a] = tile.pixels[(py % 256) * 256 + px % 256];
                let raw = i32::from_be_bytes([0, r, g, b]);
                if a != 0 && raw != 0x800000 {
                    heights[y * size + x] = ((raw << 8) >> 8) as f64 * 0.01;
                }
            }
        }
    }
    let mut shade = Vec::with_capacity(heights.len());
    for y in 0..size {
        let y0 = y.saturating_sub(1);
        let y1 = (y + 1).min(size - 1);
        for x in 0..size {
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(size - 1);
            let gx = (heights[y * size + x1] - heights[y * size + x0]) / (dx * (x1 - x0) as f64);
            let gy = (heights[y1 * size + x] - heights[y0 * size + x]) / (dy * (y1 - y0) as f64);
            let light = (-0.55 * gx + 0.65 * gy + 1.) / ((1. + gx * gx + gy * gy) * (1. + 0.55f64.powi(2) + 0.65f64.powi(2))).sqrt();
            shade.push(((0.39 + 0.70 * light) * 255.).clamp(70., 255.) as u8);
        }
    }
    let mut png = png::Encoder::new(fs::File::create(args.output)?, args.size, args.size);
    png.set_color(png::ColorType::Grayscale);
    png.write_header()?.write_image_data(&shade)?;
    println!("Reference relief: {size} × {size}, {loaded_tiles} DEM tiles loaded");
    Ok(())
}
