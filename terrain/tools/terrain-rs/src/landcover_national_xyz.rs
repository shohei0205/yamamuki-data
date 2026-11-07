use super::geos::Geometry;
use super::{EdgeCoverage, Geos, P, Result, WORLD, ZOOMS, edge_coverage, pack, read_seam_layers};
use anyhow::{Context, ensure};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

type Key = (u32, u32);
type Layers<'a> = Vec<Geometry<'a>>;
type Polygon = (u8, Vec<Vec<P>>);
type Bounds = [f64; 4];
pub(crate) const GRID: f64 = 1. / 1024.;
const COLLAR: f64 = 8.;

fn axis(index: u32, z: u8) -> f64 {
    ((f64::from(index) * WORLD / f64::from(1u32 << z)) / GRID).round() * GRID
}

fn xyz_axis(index: u32, z: u8) -> f64 {
    f64::from(index) * WORLD / f64::from(1u32 << z)
}

fn xyz_bounds((x, y): Key, z: u8) -> Bounds {
    [
        xyz_axis(x, z),
        xyz_axis(y, z),
        xyz_axis(x + 1, z),
        xyz_axis(y + 1, z),
    ]
}

fn intersects_bounds(a: Bounds, b: Bounds) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

fn bounds((x, y): Key, z: u8) -> Bounds {
    [axis(x, z), axis(y, z), axis(x + 1, z), axis(y + 1, z)]
}

fn rectangle<'a>(api: &'a Geos, [x0, y0, x1, y1]: Bounds) -> Result<Geometry<'a>> {
    api.rectangle(x0, y0, x1, y1)
}

fn union<'a>(api: &'a Geos, parts: &[Geometry<'a>]) -> Result<Geometry<'a>> {
    api.collection(parts)?.union_grid(GRID)
}

fn area_intersection<'a>(
    api: &'a Geos,
    a: &Geometry<'a>,
    b: &Geometry<'a>,
) -> Result<Geometry<'a>> {
    let result = a.intersection_grid(b, GRID)?;
    if matches!(result.kind()?, 3 | 6) {
        return Ok(result);
    }
    let polygons = result.polygons()?;
    if polygons.is_empty() {
        return api.read(&crate::wkb_multi(&[]));
    }
    union(api, &polygons)
}

fn translated<'a>(geometry: &Geometry<'a>, dx: f64, dy: f64) -> Result<Geometry<'a>> {
    geometry.transform(0., 0., 1.)?.transform(-dx, dy, 1.)
}

fn read_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(
        &fs::read(path).with_context(|| format!("Read {}", path.display()))?,
    )?)
}

fn key(row: &Value) -> Result<Key> {
    let x = row["x"].as_u64().context("Missing x")?;
    let y = row["y"].as_u64().context("Missing y")?;
    ensure!(x < 256 && y < 256, "Invalid z8 key");
    Ok((x as u32, y as u32))
}

fn find_level(region: &Path, level: usize) -> Result<PathBuf> {
    let stem = format!("l{level}");
    let retry = format!("{stem}-retry-");
    let mut found = Vec::new();
    for entry in fs::read_dir(region)? {
        let path = entry?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if (name == stem || name.starts_with(&retry)) && path.join("manifest.json").is_file() {
            found.push(path);
        }
    }
    Ok(found
        .into_iter()
        .max()
        .with_context(|| format!("Missing L{level} in {}", region.display()))?
        .join("majority-clean"))
}

struct SharedPatch<'a> {
    rect: Geometry<'a>,
    layers: Layers<'a>,
}

fn add_patch<'a>(
    api: &'a Geos,
    sources: &BTreeMap<Key, Layers<'a>>,
    patch_bounds: Bounds,
    owners: &[Key],
    patches: &mut Vec<SharedPatch<'a>>,
    usage: &mut BTreeMap<Key, Vec<usize>>,
) -> Result<()> {
    if owners.len() < 2 {
        return Ok(());
    }
    let rect = rectangle(api, patch_bounds)?;
    let mut layers = Vec::new();
    for class in 0..3 {
        let mut common = area_intersection(api, &sources[&owners[0]][class], &rect)?;
        for owner in &owners[1..] {
            if common.is_empty()? {
                break;
            }
            common = area_intersection(api, &common, &sources[owner][class])?;
        }
        layers.push(common);
    }
    let id = patches.len();
    patches.push(SharedPatch { rect, layers });
    for &owner in owners {
        usage.entry(owner).or_default().push(id);
    }
    Ok(())
}

fn reconcile<'a>(
    api: &'a Geos,
    sources: BTreeMap<Key, Layers<'a>>,
) -> Result<(BTreeMap<Key, Layers<'a>>, f64, f64, f64)> {
    let mut patches = Vec::new();
    let mut usage = BTreeMap::new();
    let mut corners = BTreeSet::new();
    for &(x, y) in sources.keys() {
        let [left, top, right, bottom] = bounds((x, y), 8);
        for point in [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)] {
            corners.insert(point);
        }
        if sources.contains_key(&(x + 1, y)) {
            add_patch(
                api,
                &sources,
                [
                    right - COLLAR,
                    top + COLLAR,
                    right + COLLAR,
                    bottom - COLLAR,
                ],
                &[(x, y), (x + 1, y)],
                &mut patches,
                &mut usage,
            )?;
        }
        if sources.contains_key(&(x, y + 1)) {
            add_patch(
                api,
                &sources,
                [
                    left + COLLAR,
                    bottom - COLLAR,
                    right - COLLAR,
                    bottom + COLLAR,
                ],
                &[(x, y), (x, y + 1)],
                &mut patches,
                &mut usage,
            )?;
        }
    }
    for (x, y) in corners {
        let owners: Vec<_> = [
            (x, y),
            (x.wrapping_sub(1), y),
            (x, y.wrapping_sub(1)),
            (x.wrapping_sub(1), y.wrapping_sub(1)),
        ]
        .into_iter()
        .filter(|candidate| sources.contains_key(candidate))
        .collect();
        let [px, py] = [axis(x, 8), axis(y, 8)];
        add_patch(
            api,
            &sources,
            [px - COLLAR, py - COLLAR, px + COLLAR, py + COLLAR],
            &owners,
            &mut patches,
            &mut usage,
        )?;
    }
    let mut cores = BTreeMap::new();
    let (mut water_removed, mut water_added) = (0., 0.);
    let mut largest_water_gain = 0_f64;
    for (key, layers) in sources {
        let core = rectangle(api, bounds(key, 8))?;
        let ids = usage.get(&key).map_or(&[][..], Vec::as_slice);
        let mut windows = Vec::new();
        for &id in ids {
            windows.push(area_intersection(api, &patches[id].rect, &core)?);
        }
        let untouched = core.difference_grid(&union(api, &windows)?, GRID)?;
        let mut output = Vec::new();
        for (class, layer) in layers.iter().enumerate() {
            let mut parts = vec![area_intersection(api, layer, &untouched)?];
            for &id in ids {
                parts.push(area_intersection(api, &patches[id].layers[class], &core)?);
            }
            let joined = union(api, &parts)
                .with_context(|| format!("Join region {key:?} class {}", class + 1))?;
            ensure!(joined.is_valid()?, "Invalid reconciled region {key:?}");
            if class == 2 {
                let previous = area_intersection(api, layer, &core)?;
                let gain = joined.difference_grid(&previous, GRID)?;
                water_added += gain.area()?;
                for part in gain.polygons()? {
                    largest_water_gain = largest_water_gain.max(part.area()?);
                }
                water_removed += previous.difference_grid(&joined, GRID)?.area()?;
            }
            // WKB retains the shared coordinates but drops GEOS's precision
            // model, so clipping against an exact XYZ rectangle stays floating.
            output.push(api.read(&joined.write()?)?);
        }
        cores.insert(key, output);
    }
    Ok((cores, water_added, water_removed, largest_water_gain))
}

fn append_polygons(shape: &Geometry<'_>, class: u8, polygons: &mut Vec<Polygon>) -> Result<()> {
    for part in shape.polygons()? {
        if !part.is_empty()? && part.area()? > 0. {
            polygons.push((class, part.rings()?));
        }
    }
    Ok(())
}

fn seam_difference(a: &EdgeCoverage, b: &EdgeCoverage, ranges: &[(i64, i64)]) -> f64 {
    if a == b {
        return 0.;
    }
    let mut worst = 0_i64;
    for (a, b) in a.iter().zip(b) {
        let mut difference = 0;
        for &(left, right) in ranges {
            let clipped = |spans: &[(i64, i64)]| -> Vec<_> {
                spans
                    .iter()
                    .filter_map(|&(start, end)| {
                        let (start, end) = (start.max(left), end.min(right));
                        (start < end).then_some((start, end))
                    })
                    .collect()
            };
            let (a, b) = (clipped(a), clipped(b));
            let total: i64 = a.iter().chain(&b).map(|(start, end)| end - start).sum();
            let mut overlap = 0;
            let (mut i, mut j) = (0, 0);
            while i < a.len() && j < b.len() {
                overlap += (a[i].1.min(b[j].1) - a[i].0.max(b[j].0)).max(0);
                if a[i].1 < b[j].1 {
                    i += 1;
                } else {
                    j += 1;
                }
            }
            difference += total - 2 * overlap;
        }
        worst = worst.max(difference);
    }
    worst as f64 / super::EDGE_SCALE
}

pub fn generate(api: &Geos, regions: &Path, plan: &Path, output: &Path) -> Result<()> {
    let plan = read_json(plan)?;
    let gaps = read_json(&regions.join("source-gaps.json"))?;
    let missing: BTreeSet<_> = gaps
        .as_array()
        .context("Expected source gaps array")?
        .iter()
        .map(key)
        .collect::<Result<_>>()?;
    let mut known = BTreeSet::new();
    for row in plan.as_array().context("Expected work-area array")? {
        let (x, y) = key(row)?;
        if missing.contains(&(x, y)) {
            continue;
        }
        let region = regions.join(format!("{x}-{y}"));
        ensure!(
            region.join("finished.json").is_file(),
            "Unfinished region {x}-{y}"
        );
        known.insert((x, y));
    }
    ensure!(!known.is_empty(), "No complete regions");
    let temp = output.join("landcover-national.sqlite3.partial");
    let target = output.join("landcover-national.sqlite3");
    ensure!(
        !temp.exists() && !target.exists(),
        "Use a new national output"
    );
    let mut db = Connection::open(&temp)?;
    db.execute_batch("PRAGMA page_size=1024;
        CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,kind INTEGER NOT NULL,coverage_mask INTEGER NOT NULL,
            payload BLOB,PRIMARY KEY(z,x,y)) WITHOUT ROWID;")?;
    let tx = db.transaction()?;
    let mut levels = Vec::new();
    for (index, z) in ZOOMS.into_iter().enumerate() {
        let level = index + 3;
        println!("L{level}: reading {} regions", known.len());
        let mut sources = BTreeMap::new();
        for &(x, y) in &known {
            let region = regions.join(format!("{x}-{y}"));
            let (extent, layers) = read_seam_layers(api, &find_level(&region, level)?, level)?;
            let dx = ((extent[0] + WORLD / 2.) / GRID).round() * GRID;
            let dy = ((WORLD / 2. - extent[3]) / GRID).round() * GRID;
            sources.insert(
                (x, y),
                layers
                    .iter()
                    .enumerate()
                    .map(|(class, shape)| {
                        let shape = shape.precision(GRID).with_context(|| {
                            format!("Snap source region {x}/{y} class {}", class + 1)
                        })?;
                        translated(&shape, dx, dy)
                    })
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        let (cores, water_added, water_removed, largest_water_gain) =
            reconcile(api, sources).with_context(|| format!("Reconcile L{level}"))?;
        println!("L{level}: assembling XYZ z{z}");
        ensure!(
            water_added <= 500. && largest_water_gain <= 100.,
            "L{level} water gain {water_added} m², largest connected gain {largest_water_gain} m²"
        );
        let mut tiles: BTreeMap<Key, Vec<Key>> = BTreeMap::new();
        for &(x, y) in &known {
            if z >= 8 {
                let scale = 1u32 << (z - 8);
                for ty in y * scale..(y + 1) * scale {
                    for tx in x * scale..(x + 1) * scale {
                        tiles.insert((tx, ty), vec![(x, y)]);
                    }
                }
            } else {
                tiles.entry((x >> 2, y >> 2)).or_default().push((x, y));
            }
        }
        let mut seams = 0;
        let mut max_seam_delta = 0_f64;
        let mut payload_bytes = 0;
        let mut edge_index: BTreeMap<(Key, usize), EdgeCoverage> = BTreeMap::new();
        let mut insert = tx.prepare("INSERT INTO tiles VALUES(?,?,?,?,?,?)")?;
        for (&(x, y), owners) in &tiles {
            let tile_bounds = xyz_bounds((x, y), z);
            let [x0, y0, x1, y1] = tile_bounds;
            let rect = rectangle(api, tile_bounds)?;
            let mut neighbors = BTreeSet::new();
            for &(ox, oy) in owners {
                for cy in oy.saturating_sub(1)..=(oy + 1).min(255) {
                    for cx in ox.saturating_sub(1)..=(ox + 1).min(255) {
                        let key = (cx, cy);
                        if cores.contains_key(&key)
                            && intersects_bounds(bounds(key, 8), tile_bounds)
                        {
                            neighbors.insert(key);
                        }
                    }
                }
            }
            let mut polygons = Vec::new();
            for class in 0..3 {
                let mut pieces = Vec::new();
                for owner in &neighbors {
                    let clipped = cores[owner][class].intersection(&rect)?;
                    for part in clipped.polygons()? {
                        if !part.is_empty()? {
                            pieces.push(part);
                        }
                    }
                }
                let shape = if pieces.len() == 1 {
                    pieces.pop().unwrap()
                } else {
                    api.collection(&pieces)?.union()?
                };
                let start = polygons.len();
                append_polygons(&shape, (class + 1) as u8, &mut polygons)?;
                for (_, rings) in &polygons[start..] {
                    for point in rings.iter().flatten() {
                        ensure!(
                            point[0] >= x0 - 1e-7 && point[0] <= x1 + 1e-7
                                && point[1] >= y0 - 1e-7 && point[1] <= y1 + 1e-7,
                            "Clipped coordinate outside z{z}/{x}/{y}: {point:?}"
                        );
                    }
                }
            }
            for (axis_id, edge, neighbor) in [
                (0, x0, (x.wrapping_sub(1), y)),
                (1, y0, (x, y.wrapping_sub(1))),
            ] {
                let Some(previous) = edge_index.get(&(neighbor, axis_id)) else {
                    continue;
                };
                let current = edge_coverage(&polygons, axis_id, edge);
                let mut shared = Vec::new();
                if z < 8 {
                    for i in 0..4 {
                        let (a, b, start) = if axis_id == 0 {
                            ((x * 4 - 1, y * 4 + i), (x * 4, y * 4 + i), y * 4 + i)
                        } else {
                            ((x * 4 + i, y * 4 - 1), (x * 4 + i, y * 4), x * 4 + i)
                        };
                        if known.contains(&a) && known.contains(&b) {
                            shared.push((
                                (xyz_axis(start, 8) * super::EDGE_SCALE).round() as i64,
                                (xyz_axis(start + 1, 8) * super::EDGE_SCALE).round() as i64,
                            ));
                        }
                    }
                    if shared.is_empty() {
                        continue;
                    }
                }
                if shared.is_empty() {
                    let (lo, hi) = if axis_id == 0 { (y0, y1) } else { (x0, x1) };
                    shared.push((
                        (lo * super::EDGE_SCALE).round() as i64,
                        (hi * super::EDGE_SCALE).round() as i64,
                    ));
                }
                let delta = seam_difference(previous, &current, &shared);
                ensure!(
                    delta <= 0.01,
                    "Tile seam mismatch L{level} z{z}/{x}/{y} axis {axis_id}: {delta} m"
                );
                max_seam_delta = max_seam_delta.max(delta);
                seams += 1;
            }
            edge_index.insert(((x, y), 0), edge_coverage(&polygons, 0, x1));
            edge_index.insert(((x, y), 1), edge_coverage(&polygons, 1, y1));
            let coverage = if z >= 8 {
                1u16
            } else {
                owners.iter().fold(0u16, |bits, &(ox, oy)| {
                    bits | (1u16 << ((oy % 4) * 4 + ox % 4))
                })
            };
            let kind = if polygons.is_empty() {
                0
            } else if polygons.len() == 1
                && polygons[0].0 == 3
                && (z >= 8 || coverage == u16::MAX)
                && rect
                    .difference(&api.read(&super::wkb_poly(&polygons[0].1))?)?
                    .area()?
                    == 0.
            {
                3
            } else {
                -1
            };
            let payload = if kind == -1 {
                for (_, rings) in &mut polygons {
                    for ring in rings {
                        for point in ring {
                            point[0] -= x0;
                            point[1] -= y0;
                        }
                    }
                }
                Some(pack(polygons).with_context(|| format!("Encode z{z}/{x}/{y}"))?)
            } else {
                None
            };
            payload_bytes += payload.as_ref().map_or(0, Vec::len);
            insert.execute(params![z, x, y, kind, coverage, payload])?;
        }
        levels.push(json!({"level":level,"z":z,"tiles":tiles.len(),"checkedSeams":seams,"maxSeamDifferenceM":max_seam_delta,
            "payloadBytes":payload_bytes,"waterAddedProjectedM2":water_added,"waterRemovedProjectedM2":water_removed,"largestConnectedWaterGainProjectedM2":largest_water_gain}));
        println!("{}", levels.last().unwrap());
    }
    let report = json!({
        "version":1,"format":"BZF2+gzip","crs":"EPSG:3857","worldWidthM":WORLD,"gridM":GRID,
        "coordinateFrame":"tile-local metres east/south; origin is the exact XYZ northwest corner",
        "coverageMask":"z6: 4x4 known z8 cores, bit x+4*y; z8 and higher: 1 means fully known; absent tile is unknown",
        "kinds":{"-1":"payload","0":"background within coverage","3":"water within coverage"},
        "collarHalfWidthM":COLLAR,"sourceGaps":gaps,"completeRegions":known.len(),"levels":levels,
        "attribution":"© ESA WorldCover project 2021 / Contains modified Copernicus Sentinel data (2021) processed by ESA WorldCover consortium"
    });
    tx.execute(
        "INSERT INTO metadata VALUES('manifest',?)",
        [report.to_string()],
    )?;
    tx.commit()?;
    let status: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    ensure!(status == "ok", "SQLite integrity check failed: {status}");
    drop(db);
    fs::rename(&temp, &target)?;
    fs::write(
        output.join("manifest.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("Complete: {}", target.display());
    Ok(())
}
