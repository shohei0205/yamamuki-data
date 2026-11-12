//! 作業用標高SQLiteからESA WorldCoverの取得範囲を算出する。
use super::{tile_keys, work_areas};
use anyhow::{Result, ensure};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(clap::Parser)]
pub struct Args {
    pub terrain_database: PathBuf,
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(i32).range(0..=32))]
    pub halo_tiles: i32,
}

fn worldcover_keys(tiles: &[[u32; 2]], halo: i32) -> BTreeSet<String> {
    let width = f64::from(1 << 11);
    let latitude = |y| {
        (std::f64::consts::PI * (1. - 2. * f64::from(y) / width))
            .sinh()
            .atan()
            .to_degrees()
    };
    let longitude = |x| f64::from(x) * 360. / width - 180.;
    let block = |degrees: f64| (degrees / 3.).floor() as i32 * 3;
    let mut keys = BTreeSet::new();
    for &[x, y] in tiles {
        let (x, y) = (x as i32 * 8, y as i32 * 8);
        for lat in (block(latitude(y + 8 + halo))..=block(latitude(y - halo))).step_by(3) {
            for lon in (block(longitude(x - halo))..=block(longitude(x + 8 + halo))).step_by(3) {
                let ns = if lat >= 0 { 'N' } else { 'S' };
                let ew = if lon >= 0 { 'E' } else { 'W' };
                keys.insert(format!("{ns}{:02}{ew}{:03}", lat.abs(), lon.abs()));
            }
        }
    }
    keys
}

pub fn run(args: Args) -> Result<()> {
    let areas: Vec<_> = work_areas(&tile_keys(&args.terrain_database, 8)?)
        .into_keys()
        .collect();
    let keys = worldcover_keys(&areas, args.halo_tiles);
    ensure!(
        !keys.is_empty(),
        "WorldCoverタイル候補を算出できませんでした"
    );
    for key in keys {
        println!("{key}");
    }
    Ok(())
}
