use super::{Geos, P, Curve, Result, build_graph, encode_compact, ensure, references, zip, wkb_poly};
use flate2::read::ZlibDecoder;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Read, path::{Path, PathBuf}};

type Interval = (i64, i64);
type EdgeCoverage = [Vec<Interval>; 3];
const WORLD: f64 = 40_075_016.685_578_49;
const ZOOMS: [u8; 4] = [11, 9, 8, 6];
const SUMMIT_WINDOW_M: f64 = 10_000.;
const EDGE_SCALE: f64 = 1_048_576.;

fn edge_coverage(layers: &[(u8, Vec<Vec<P>>)], axis: usize, edge: f64) -> EdgeCoverage {
    let mut result: EdgeCoverage = Default::default();
    let tangent = 1 - axis;
    for (class, rings) in layers {
        for ring in rings {
            for segment in ring.windows(2) {
                if segment.iter().all(|p| p[axis] == edge) {
                    let a = (segment[0][tangent] * EDGE_SCALE).round() as i64;
                    let b = (segment[1][tangent] * EDGE_SCALE).round() as i64;
                    if a != b {
                        result[usize::from(*class - 1)].push((a.min(b), a.max(b)));
                    }
                }
            }
        }
    }
    for spans in &mut result {
        spans.sort_unstable();
        let mut merged: Vec<Interval> = Vec::new();
        for (start, end) in spans.drain(..) {
            if let Some(last) = merged.last_mut().filter(|last| start <= last.1) {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        *spans = merged;
    }
    result
}

fn pack(layers: Vec<(u8, Vec<Vec<P>>)>) -> Result<Vec<u8>> {
    let graph = build_graph(layers, true, false, true)?;
    let curves: Vec<_> = graph.chains.iter().map(|chain| {
        chain.nodes.windows(2).map(|edge| Curve::line(graph.points[edge[0]], graph.points[edge[1]])).collect()
    }).collect();
    let refs = graph.polys.iter().map(|poly| {
        poly.rings.iter().map(|ring| references(&graph, ring)).collect::<Result<Vec<_>>>()
    }).collect::<Result<Vec<_>>>()?;
    zip(&encode_compact(&curves, &graph.polys, &refs, true), true)
}

pub fn generate(api: &Geos, sources: &[PathBuf], output: &Path) -> Result<()> {
    ensure!(sources.len() == ZOOMS.len(), "Pass one --tile-source per L3–L6 in order");
    let path = output.join("landcover-xyz.sqlite3");
    ensure!(!path.exists(), "Use a new tile output");
    let mut db = Connection::open(&path)?;
    db.execute_batch("PRAGMA page_size=1024;
        CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,kind INTEGER NOT NULL,
            payload BLOB,PRIMARY KEY(z,x,y)) WITHOUT ROWID;")?;
    let transaction = db.transaction()?;
    let mut bounds = None;
    let mut levels = Vec::new();
    let mut tiles = Vec::new();
    for (index, (source, z)) in sources.iter().zip(ZOOMS).enumerate() {
        let level = index + 3;
        let manifest: Value = serde_json::from_slice(&fs::read(source.join("manifest.json"))?)?;
        let extent: [f64; 4] = serde_json::from_value(manifest["bounds"].clone())?;
        ensure!(*bounds.get_or_insert(extent) == extent, "Input bounds differ");
        let [left, bottom, right, top] = extent;
        let side = right - left;
        ensure!((top - bottom - side).abs() < 1e-6 && side > 0., "Expected square extent");
        let origin = [left + WORLD / 2., WORLD / 2. - top];
        let tile_side = WORLD / (1u32 << z) as f64;
        let first = origin.map(|v| (v / tile_side).floor() as u32);
        let last = origin.map(|v| ((v + side) / tile_side).ceil() as u32);
        let source_db = Connection::open_with_flags(source.join("landcover-vectors.sqlite3"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut geometries = Vec::new();
        for class in 1..=3u8 {
            let compressed: Vec<u8> = source_db.query_row(
                "SELECT geometry FROM features WHERE level=? AND class=?", params![level, class], |r| r.get(0))?;
            let mut raw = Vec::new();
            ZlibDecoder::new(compressed.as_slice()).read_to_end(&mut raw)?;
            geometries.push((class, api.read(&raw)?));
        }
        let original_bytes = fs::metadata(source.join(format!("level-{level}.curves.gz")))?.len();
        let mut edge_index = BTreeMap::new();
        let mut count = 0;
        let mut payload_bytes = 0;
        let mut summit_count = 0;
        let mut summit_bytes = 0;
        let mut seams = 0;
        let mut coastal_seams = 0;
        let mut water_example: Option<P> = None;
        for y in first[1]..last[1] {
            for x in first[0]..last[0] {
                let x0 = (x as f64 * tile_side - origin[0]).max(0.);
                let y0 = (y as f64 * tile_side - origin[1]).max(0.);
                let x1 = ((x + 1) as f64 * tile_side - origin[0]).min(side);
                let y1 = ((y + 1) as f64 * tile_side - origin[1]).min(side);
                let rect = api.rectangle(x0, y0, x1, y1)?;
                let mut layers = Vec::new();
                for (class, geometry) in &geometries {
                    for polygon in geometry.intersection(&rect)?.polygons()? {
                        if polygon.is_empty()? || polygon.area()? == 0. { continue; }
                        let mut rings = polygon.rings()?;
                        for ring in &mut rings {
                            for point in ring.iter_mut() {
                                if (point[0] == x0 || point[0] == x1) && point[1] != y0 && point[1] != y1 {
                                    point[1] = ((point[1] * EDGE_SCALE).round() / EDGE_SCALE).clamp(y0, y1);
                                } else if (point[1] == y0 || point[1] == y1) && point[0] != x0 && point[0] != x1 {
                                    point[0] = ((point[0] * EDGE_SCALE).round() / EDGE_SCALE).clamp(x0, x1);
                                }
                            }
                            ring.dedup();
                        }
                        ensure!(api.read(&wkb_poly(&rings))?.is_valid()?, "Invalid snapped tile {z}/{x}/{y}, class {class}");
                        layers.push((*class, rings));
                    }
                }
                for (axis, edge, neighbor) in [(0, x0, (x.wrapping_sub(1), y)), (1, y0, (x, y.wrapping_sub(1)))] {
                    if let Some(previous) = edge_index.get(&(neighbor, axis)) {
                        let current = edge_coverage(&layers, axis, edge);
                        ensure!(*previous == current, "Layer seam mismatch at {z}/{x}/{y}, axis {axis}");
                        seams += 1;
                        let range = if axis == 0 { (y0, y1) } else { (x0, x1) };
                        let full = ((range.0 * EDGE_SCALE).round() as i64, (range.1 * EDGE_SCALE).round() as i64);
                        if !current[2].is_empty() && current[2].as_slice() != [full] {
                            coastal_seams += 1;
                            for endpoint in current[2].iter().flat_map(|&(a, b)| [a, b]) {
                                if endpoint > full.0 && endpoint < full.1 {
                                    let mut point = [edge; 2];
                                    point[1 - axis] = endpoint as f64 / EDGE_SCALE;
                                    let distance = |p: P| (p[0] - 60422.).hypot(p[1] - 94382.);
                                    if water_example.is_none_or(|previous| distance(point) < distance(previous)) {
                                        water_example = Some(point);
                                    }
                                }
                            }
                        }
                    }
                }
                edge_index.insert(((x, y), 0), edge_coverage(&layers, 0, x1));
                edge_index.insert(((x, y), 1), edge_coverage(&layers, 1, y1));
                let kind = if layers.is_empty() { 0 } else if layers.len() == 1 && layers[0].0 == 3
                    && rect.difference(&api.read(&wkb_poly(&layers[0].1))?)?.area()? == 0. { 3 } else { -1 };
                let payload = if kind == -1 { Some(pack(layers)?) } else { None };
                let bytes = payload.as_ref().map_or(0, Vec::len);
                transaction.execute("INSERT INTO tiles VALUES(?,?,?,?,?)", params![z,x,y,kind,payload])?;
                tiles.push(json!({"z":z,"x":x,"y":y,"bounds":[x0,y0,x1,y1],"kind":kind,"bytes":bytes}));
                count += 1;
                payload_bytes += bytes;
                let lo = (side - SUMMIT_WINDOW_M) / 2.;
                let hi = lo + SUMMIT_WINDOW_M;
                if x0 < hi && y0 < hi && x1 > lo && y1 > lo {
                    summit_count += 1;
                    summit_bytes += bytes;
                }
            }
        }
        levels.push(json!({"level":level,"z":z,"tileCount":count,"originalBytes":original_bytes,
            "payloadBytes":payload_bytes,"summitTiles":summit_count,"summitBytes":summit_bytes,
            "checkedSeams":seams,"waterCrossingSeams":coastal_seams,"waterExample":water_example}));
    }
    let mut report = json!({"bounds":bounds,"format":"BZF2+gzip","crs":"EPSG:3857",
        "coordinateFrame":"dataset-local, northwest origin, metres east/south",
        "edgeGridMetres":1. / EDGE_SCALE,"summitWindowMetres":SUMMIT_WINDOW_M,"levels":levels,"tiles":tiles});
    transaction.execute("INSERT INTO metadata VALUES('manifest',?)", [report.to_string()])?;
    transaction.commit()?;
    report["sqliteBytes"] = json!(fs::metadata(&path)?.len());
    fs::write(output.join("manifest.json"), serde_json::to_vec_pretty(&report)?)?;
    println!("{}", report["levels"]);
    Ok(())
}
