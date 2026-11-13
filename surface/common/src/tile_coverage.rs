//! 収録済み画像タイルの領域を結合し、配布ファイルごとの提供範囲を作る。

use anyhow::{Context, Result};
use geo::{Coord, LineString, MultiPolygon, Polygon, Rect, algorithm::unary_union};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

fn tile_union(
    db: &Connection,
    dataset: i64,
    density: Option<i64>,
) -> Result<Option<MultiPolygon<f64>>> {
    let mut query = db.prepare(
        "SELECT DISTINCT t.z,t.y,t.x FROM surface_tiles t
         JOIN surface_levels l USING(dataset_key,layer_key,z)
         WHERE t.dataset_key=?1 AND (?2 IS NULL OR l.tile_pixels*(1 << t.z)=?2)
         ORDER BY t.z,t.y,t.x",
    )?;
    let mut rows = query.query(params![dataset, density])?;
    let mut previous: Option<(u32, u32, u32, u32)> = None;
    let mut rectangles = Vec::new();
    let mut add = |(z, y, left, right): (u32, u32, u32, u32)| {
        let side = f64::from(1u32 << z);
        rectangles.push(
            Rect::new(
                Coord {
                    x: f64::from(left) / side,
                    y: f64::from(y) / side,
                },
                Coord {
                    x: f64::from(right + 1) / side,
                    y: f64::from(y + 1) / side,
                },
            )
            .to_polygon(),
        );
    };
    while let Some(row) = rows.next()? {
        let (z, y, x): (u32, u32, u32) = (row.get(0)?, row.get(1)?, row.get(2)?);
        if let Some((level, line, start, end)) = previous {
            if level == z && line == y && x == end + 1 {
                previous = Some((level, line, start, x));
                continue;
            }
            add((level, line, start, end));
        }
        previous = Some((z, y, x, x));
    }
    if let Some(run) = previous {
        add(run);
    }
    if rectangles.is_empty() {
        return Ok(None);
    }
    Ok(Some(unary_union(&rectangles)))
}

fn ring_json(ring: &LineString<f64>, exterior: bool) -> Vec<[f64; 2]> {
    let mut corners = Vec::new();
    let points = &ring.0[..ring.0.len() - 1];
    for i in 0..points.len() {
        let (a, b, c) = (
            points[(i + points.len() - 1) % points.len()],
            points[i],
            points[(i + 1) % points.len()],
        );
        if (b.x - a.x) * (c.y - b.y) != (b.y - a.y) * (c.x - b.x) {
            corners.push(b);
        }
    }
    let mut result = Vec::new();
    for i in 0..corners.len() {
        let (a, b) = (corners[i], corners[(i + 1) % corners.len()]);
        // 世界全体の長い辺も、一辺の経度差を180度以下にする。
        let steps = ((b.x - a.x).abs() * 2.).ceil().max(1.) as usize;
        for step in 0..steps {
            let world = Coord {
                x: a.x + (b.x - a.x) * step as f64 / steps as f64,
                y: a.y + (b.y - a.y) * step as f64 / steps as f64,
            };
            result.push([
                world.x * 360. - 180.,
                (std::f64::consts::PI * (1. - 2. * world.y))
                    .sinh()
                    .atan()
                    .to_degrees(),
            ]);
        }
    }
    let origin = result[0];
    let area: f64 = (0..result.len())
        .map(|i| {
            let (a, b) = (result[i], result[(i + 1) % result.len()]);
            (a[0] - origin[0]) * (b[1] - origin[1]) - (b[0] - origin[0]) * (a[1] - origin[1])
        })
        .sum();
    if (area > 0.) != exterior {
        result.reverse();
    }
    if let Some(first) = (0..result.len()).min_by(|&a, &b| {
        result[a][0]
            .total_cmp(&result[b][0])
            .then_with(|| result[a][1].total_cmp(&result[b][1]))
    }) {
        result.rotate_left(first);
    }
    result.push(result[0]);
    result
}

fn geometry_json(geometry: MultiPolygon<f64>) -> Value {
    let mut polygons: Vec<_> = geometry
        .0
        .iter()
        .map(|p: &Polygon<f64>| {
            let mut rings = vec![ring_json(p.exterior(), true)];
            rings.extend(p.interiors().iter().map(|r| ring_json(r, false)));
            rings
        })
        .collect();
    polygons.sort_by(|a, b| {
        a[0][0][0]
            .total_cmp(&b[0][0][0])
            .then_with(|| a[0][0][1].total_cmp(&b[0][0][1]))
    });
    if polygons.len() == 1 {
        json!({"type":"Polygon","coordinates":polygons.remove(0)})
    } else {
        json!({"type":"MultiPolygon","coordinates":polygons})
    }
}

/// 元ファイルの図形をコピーせず、このファイルにあるタイルから計算し直す。
pub fn recompute(db: &Connection, dataset: i64) -> Result<()> {
    let saved: Option<String> = db
        .query_row(
            "SELECT metadata_json FROM datasets WHERE dataset_key=?1 AND kind='surface'",
            [dataset],
            |row| row.get(0),
        )
        .optional()?;
    let Some(saved) = saved else { return Ok(()) };
    let mut metadata: Value = serde_json::from_str(&saved)?;
    if metadata.get("coverage").is_none() {
        metadata["coverage"] = json!({});
    }
    let coverage = metadata["coverage"]
        .as_object_mut()
        .context("coverage は object にしてください")?;
    coverage.remove("geometry");
    coverage.remove("finest");
    if let Some(whole) = tile_union(db, dataset, None)? {
        let density: i64=db.query_row("SELECT max(l.tile_pixels*(1 << t.z)) FROM surface_tiles t JOIN surface_levels l USING(dataset_key,layer_key,z) WHERE t.dataset_key=?1",[dataset],|row| row.get(0))?;
        let finest =
            tile_union(db, dataset, Some(density))?.context("最も精細なタイルがありません")?;
        coverage.insert("geometry".into(), geometry_json(whole));
        coverage.insert("finest".into(), json!({"geometry":geometry_json(finest)}));
    }
    crate::validate_coverage_metadata(&metadata, true)?;
    db.execute(
        "UPDATE datasets SET metadata_json=?1 WHERE dataset_key=?2",
        params![metadata.to_string(), dataset],
    )?;
    Ok(())
}
