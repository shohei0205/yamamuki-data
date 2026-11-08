//! 市街地率と地形の起伏から森林の残存率を求める。
use anyhow::{Result, ensure};
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::path::Path;

const Z: u32 = 11;
const TILE: i64 = 256;
const BUILT_RADIUS: usize = 12;
const RELIEF_RADIUS: usize = 4;

pub(super) struct ReliefDem(Connection);

impl ReliefDem {
    pub(super) fn open(path: &Path) -> Result<Self> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        ensure!(
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM tiles WHERE z=?1 AND length(data)>0)",
                [Z],
                |r| r.get::<_, bool>(0)
            )?,
            "z11 の標高タイルが必要です"
        );
        Ok(Self(db))
    }

    pub(super) fn forest_weights(
        &self,
        x: u32,
        y: u32,
        core: usize,
        halo: usize,
        built: &[u8],
        classes: &[u8],
    ) -> Result<Vec<u8>> {
        let side = (core + 2 * halo) / 2;
        let first_x = (x as i64 * core as i64 - halo as i64) / 2;
        let first_y = (y as i64 * core as i64 - halo as i64) / 2;
        let side = side as i64;
        let mut dem = vec![f32::NAN; (side * side) as usize];
        let mut read = self
            .0
            .prepare_cached("SELECT data FROM tiles WHERE z=?1 AND x=?2 AND y=?3")?;
        for ty in
            (first_y.div_euclid(TILE)).max(0)..=((first_y + side - 1) / TILE).min((1 << Z) - 1)
        {
            for tx in
                (first_x.div_euclid(TILE)).max(0)..=((first_x + side - 1) / TILE).min((1 << Z) - 1)
            {
                let data: Option<Vec<u8>> = read.query_row((Z, tx, ty), |r| r.get(0)).optional()?;
                let Some(data) = data else { continue };
                let grid = crate::codec::decode_grid(&data)?;
                ensure!(
                    grid.rows == TILE as usize && grid.columns == TILE as usize,
                    "標高タイルの寸法が不正です"
                );
                let left = first_x.max(tx * TILE);
                let right = (first_x + side).min((tx + 1) * TILE);
                for row in first_y.max(ty * TILE)..(first_y + side).min((ty + 1) * TILE) {
                    let dst = ((row - first_y) * side + left - first_x) as usize;
                    let src = ((row - ty * TILE) * TILE + left - tx * TILE) as usize;
                    let width = (right - left) as usize;
                    dem[dst..dst + width].copy_from_slice(&grid.values[src..src + width]);
                }
            }
        }
        Ok(forest_weights(&dem, built, classes, side as usize))
    }
}

fn remaining(built_fraction: f32, relief: f32, forest_fraction: f32) -> u8 {
    let urban = ((built_fraction - 0.10) / 0.15).clamp(0.0, 1.0);
    let flat = ((25.0 - relief) / 17.0).clamp(0.0, 1.0);
    // 平地でも、森林がまとまっている部分は残す。
    let protection = ((forest_fraction - 0.30) / 0.30).clamp(0.0, 1.0);
    ((1.0 - urban * flat * (1.0 - protection)) * 255.0).round() as u8
}

fn forest_weights(dem: &[f32], built: &[u8], classes: &[u8], side: usize) -> Vec<u8> {
    let stride = side + 1;
    let source_side = side * 2;
    let mut integral = vec![[0u32; 2]; stride * stride];
    for y in 0..side {
        let row = y * 2 * source_side;
        let mut sum = [0u32; 2];
        for x in 0..side {
            let i = row + x * 2;
            for j in [i, i + 1, i + source_side, i + source_side + 1] {
                sum[0] += u32::from(built[j]);
                sum[1] += u32::from(classes[j] == 1);
            }
            let above = integral[y * stride + x + 1];
            integral[(y + 1) * stride + x + 1] = [above[0] + sum[0], above[1] + sum[1]];
        }
    }
    let fraction = |x: usize, y: usize, radius: usize, channel: usize| {
        let top = (y - radius) * stride;
        let bottom = (y + radius + 1) * stride;
        let left = x - radius;
        let right = x + radius + 1;
        let count = integral[bottom + right][channel] + integral[top + left][channel]
            - integral[bottom + left][channel]
            - integral[top + right][channel];
        count as f32 / (4 * (2 * radius + 1).pow(2)) as f32
    };
    let mut weights = vec![255; side * side];
    weights
        .par_chunks_mut(side)
        .enumerate()
        .for_each(|(y, row)| {
            if y < BUILT_RADIUS || y + BUILT_RADIUS >= side {
                return;
            }
            for (x, weight) in row
                .iter_mut()
                .enumerate()
                .skip(BUILT_RADIUS)
                .take(side.saturating_sub(2 * BUILT_RADIUS))
            {
                let built_fraction = fraction(x, y, BUILT_RADIUS, 0);
                let forest_fraction = fraction(x, y, RELIEF_RADIUS, 1);
                if built_fraction <= 0.10 || forest_fraction >= 0.60 {
                    continue;
                }
                let mut min = f32::INFINITY;
                let mut max = f32::NEG_INFINITY;
                let mut valid = 0;
                for row in y - RELIEF_RADIUS..=y + RELIEF_RADIUS {
                    for &z in &dem[row * side + x - RELIEF_RADIUS..=row * side + x + RELIEF_RADIUS]
                    {
                        if z.is_finite() {
                            min = min.min(z);
                            max = max.max(z);
                            valid += 1;
                        }
                    }
                }
                if dem[y * side + x].is_finite() && valid * 5 >= (2 * RELIEF_RADIUS + 1).pow(2) * 4
                {
                    *weight = remaining(built_fraction, max - min, forest_fraction);
                }
            }
        });
    weights
}

#[test]
fn retains_hills_and_rural_forest_and_softens_flat_urban_forest() {
    assert_eq!(remaining(0.08, 0.0, 0.0), 255);
    assert_eq!(remaining(0.40, 25.0, 0.0), 255);
    assert_eq!(remaining(0.25, 8.0, 0.0), 0);
    assert_eq!(remaining(0.175, 16.5, 0.0), 191);
    assert!((127..=128).contains(&remaining(0.40, 8.0, 0.45)));
    assert_eq!(remaining(0.40, 8.0, 0.60), 255);
    let n = 25;
    let center = 12 * n + 12;
    let mut dem = vec![10.0; n * n];
    let built = vec![1; 4 * n * n];
    let open = vec![0; 4 * n * n];
    let forest = vec![1; 4 * n * n];
    assert_eq!(forest_weights(&dem, &built, &open, n)[center], 0);
    assert_eq!(forest_weights(&dem, &built, &forest, n)[center], 255);
    dem[center] = 40.0;
    assert_eq!(forest_weights(&dem, &built, &open, n)[center], 255);
    dem[center] = f32::NAN;
    assert_eq!(forest_weights(&dem, &built, &open, n)[center], 255);
}
