//! 地表生成に使う公開データを取得する。
pub mod worldcover;

use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use std::{collections::BTreeMap, path::Path};

fn tile_keys(database: &Path, zoom: u8) -> Result<Vec<[u32; 2]>> {
    let db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut query = db.prepare("SELECT x,y FROM tiles WHERE z=? ORDER BY x,y")?;
    Ok(query
        .query_map([zoom], |row| Ok([row.get(0)?, row.get(1)?]))?
        .collect::<rusqlite::Result<_>>()?)
}

fn work_areas(keys: &[[u32; 2]]) -> BTreeMap<[u32; 2], u32> {
    let mut areas = BTreeMap::new();
    for &[x, y] in keys {
        areas.insert([x, y], 11);
        // パック後の実XYZズームはこの処理のzより1小さい。
        for z in (6..=8).rev() {
            let width = 1 << (9 - z);
            let x0 = x / width * width;
            let y0 = y / width * width;
            for y in y0..y0 + width {
                for x in x0..x0 + width {
                    let max_zoom = areas.entry([x, y]).or_insert(z);
                    *max_zoom = (*max_zoom).max(z);
                }
            }
        }
    }
    areas
}

pub fn plan_surface(database: &Path) -> Result<()> {
    let keys = tile_keys(database, 8)?;
    anyhow::ensure!(!keys.is_empty(), "標高の範囲が空です");
    let mut areas: Vec<_> = work_areas(&keys)
        .into_iter()
        .map(|([x, y], max_zoom)| serde_json::json!({ "x": x, "y": y, "maxZoom": max_zoom }))
        .collect();
    areas.extend(
        tile_keys(database, 9)?
            .into_iter()
            .map(|[x, y]| serde_json::json!({ "x": x, "y": y, "workZoom": 9, "maxZoom": 13 })),
    );
    println!("{}", serde_json::to_string(&areas)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarse_tile_extents_are_completed_without_expanding_fine_levels() {
        let seeds = [[0, 0], [2, 0], [12, 8]];
        let areas = work_areas(&seeds);
        for z in 6..=11 {
            let width = 1 << (9_u32.saturating_sub(z));
            let expected = seeds
                .iter()
                .map(|[x, y]| [x / width, y / width])
                .collect::<std::collections::BTreeSet<_>>();
            let mut counts = BTreeMap::new();
            for (&[x, y], &max_zoom) in &areas {
                if max_zoom >= z {
                    *counts.entry([x / width, y / width]).or_insert(0) += 1;
                }
            }
            assert_eq!(
                counts
                    .keys()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>(),
                expected
            );
            assert!(counts.values().all(|&count| count == width * width));
        }
    }
}
