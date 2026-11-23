//! 局所係数を保存した曲面を倍率1で追加削減する。共有点・親タイルは維持する。
use crate::local_slope::{errors, fully_inside, model};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, backup::Backup, params};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use yamamuki_terrain::{
    control_attributes,
    local_slope::Prepared,
    natural_codec,
    natural_regional_surface::{Fast, Work},
    sqlite::{Database, Key, Tile, WORLD, point_hash},
};
const STEP: f64 = WORLD / (16384. * 256.);
#[derive(Clone)]
pub(super) struct Raster {
    pub x: u32,
    pub y: u32,
    pub values: Vec<f64>,
}
pub(super) fn read_rasters(root: &Path, inventory: &Connection, key: Key) -> Result<Vec<Raster>> {
    let width = 1u32 << (14 - key.0);
    let mut q=inventory.prepare("SELECT x,y,sha256 FROM inputs WHERE verified=1 AND state='cached' AND x>=?1 AND x<?2 AND y>=?3 AND y<?4 ORDER BY y,x")?;
    let rows = q.query_map(
        params![
            key.1 * width,
            (key.1 + 1) * width,
            key.2 * width,
            (key.2 + 1) * width
        ],
        |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, String>(2)?,
            ))
        },
    )?;
    rows.map(|r| {
        let (x, y, hash) = r?;
        let b = std::fs::read(root.join("14").join(x.to_string()).join(format!("{y}.png")))?;
        ensure!(
            crate::natural_inventory::hash(&b) == hash,
            "元DEMのSHA-256が変化しました"
        );
        Ok(Raster {
            x,
            y,
            values: crate::natural_inventory::decode(&b)?,
        })
    })
    .collect()
}
pub(super) fn visit(
    rasters: &[Raster],
    key: Key,
    bounds: [f64; 4],
    mut f: impl FnMut([f64; 2], f64) -> Result<bool>,
) -> Result<bool> {
    let side = WORLD / 2f64.powi(key.0 as i32);
    for r in rasters {
        let origin = [
            f64::from(r.x) * 256. * STEP - f64::from(key.1) * side,
            f64::from(r.y) * 256. * STEP - f64::from(key.2) * side,
        ];
        let x0 = ((bounds[0] - origin[0]) / STEP - 0.5)
            .ceil()
            .clamp(0., 256.) as i32;
        let y0 = ((bounds[1] - origin[1]) / STEP - 0.5)
            .ceil()
            .clamp(0., 256.) as i32;
        let x1 = ((bounds[2] - origin[0]) / STEP - 0.5)
            .floor()
            .clamp(-1., 255.) as i32;
        let y1 = ((bounds[3] - origin[1]) / STEP - 0.5)
            .floor()
            .clamp(-1., 255.) as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let h = r.values[y as usize * 256 + x as usize];
                if h.is_finite()
                    && !f(
                        [
                            origin[0] + (f64::from(x) + 0.5) * STEP,
                            origin[1] + (f64::from(y) + 0.5) * STEP,
                        ],
                        h,
                    )?
                {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}
pub(super) const PEAK_GUARD: f64 = 0.5;
#[derive(Default)]
pub(super) struct ShapeAudit {
    pub count: u64,
    pub maximum: f64,
    pub pixels: u64,
    pub candidates: Vec<[f64; 3]>,
}
/// DEMの8近傍より突出量を増やす補間ピークを調べる。タイル境界も画素座標で接続する。
pub(super) fn shape_audit(
    rasters: &[Raster],
    key: Key,
    region: [f64; 4],
    surface: &Prepared<'_>,
    amounts: Option<&[u8]>,
) -> Result<ShapeAudit> {
    let side = WORLD / 2f64.powi(key.0 as i32);
    let origin = [f64::from(key.1) * side, f64::from(key.2) * side];
    let x0 = ((region[0] + origin[0]) / STEP - 0.5).ceil() as i32 - 1;
    let y0 = ((region[1] + origin[1]) / STEP - 0.5).ceil() as i32 - 1;
    let x1 = ((region[2] + origin[0]) / STEP - 0.5).floor() as i32 + 1;
    let y1 = ((region[3] + origin[1]) / STEP - 0.5).floor() as i32 + 1;
    let width = usize::try_from(x1 - x0 + 1)?;
    let height = usize::try_from(y1 - y0 + 1)?;
    ensure!(
        width.checked_mul(height).is_some_and(|n| n <= 16777216),
        "形状監査の領域が大きすぎます"
    );
    let mut fields = vec![[f64::NAN; 2]; width * height];
    let mut work = Work::default();
    visit(
        rasters,
        key,
        [
            region[0] - STEP * 1.01,
            region[1] - STEP * 1.01,
            region[2] + STEP * 1.01,
            region[3] + STEP * 1.01,
        ],
        |q, h| {
            let x = ((q[0] + origin[0]) / STEP - 0.5).round() as i32;
            let y = ((q[1] + origin[1]) / STEP - 0.5).round() as i32;
            if x >= x0 && x <= x1 && y >= y0 && y <= y1 {
                fields[(y - y0) as usize * width + (x - x0) as usize] =
                    [surface.evaluate(q, amounts, 1., &mut work)?, h];
            }
            Ok(true)
        },
    )?;
    let mut result = ShapeAudit::default();
    let mut fixes = std::collections::BTreeMap::new();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let index = y * width + x;
            let [h, dem] = fields[index];
            if !h.is_finite() {
                continue;
            }
            let (mut high, mut source_high, mut valid) =
                (f64::NEG_INFINITY, f64::NEG_INFINITY, true);
            let mut best = (dem - h, index);
            for dy in -1isize..=1 {
                for dx in -1isize..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let k = (index as isize + dy * width as isize + dx) as usize;
                    let [a, b] = fields[k];
                    valid &= a.is_finite() && b.is_finite();
                    high = high.max(a);
                    source_high = source_high.max(b);
                    if b - a > best.0 {
                        best = (b - a, k);
                    }
                }
            }
            if !valid {
                continue;
            }
            result.pixels += 1;
            let excess = ((h - high).max(0.) - (dem - source_high).max(0.)).max(0.);
            result.maximum = result.maximum.max(excess);
            if excess > PEAK_GUARD {
                result.count += 1;
                // 頂点の上振れと、周囲の下振れの両方を戻せるようDEM点を補う。
                for k in [index, best.1] {
                    let gx = x0 + (k % width) as i32;
                    let gy = y0 + (k / width) as i32;
                    fixes.insert(
                        (gx, gy),
                        [
                            (f64::from(gx) + 0.5) * STEP - origin[0],
                            (f64::from(gy) + 0.5) * STEP - origin[1],
                            fields[k][1],
                        ],
                    );
                }
            }
        }
    }
    result.candidates = fixes.into_values().collect();
    Ok(result)
}
fn influence(model: &Fast, id: usize) -> [f64; 4] {
    let mut b = model.deletion_box(id, STEP * 2.);
    for &i in &model.base.neighbors[id] {
        let c = model.deletion_box(i, STEP * 2.);
        b = [
            b[0].min(c[0]),
            b[1].min(c[1]),
            b[2].max(c[2]),
            b[3].max(c[3]),
        ];
    }
    b
}
struct Reduction {
    active: Vec<bool>,
    removed: u64,
    tried: u64,
    baseline_exceeded: u64,
}
fn reduce(
    model: &mut Fast,
    amounts: &[u8],
    protected: &[bool],
    bounds: [f64; 4],
    limit: usize,
    mut audit: impl FnMut(&Fast, [f64; 4]) -> Result<(bool, bool)>,
) -> Result<Reduction> {
    ensure!(
        amounts.len() == model.base.points.len() && protected.len() == amounts.len(),
        "係数または保護点の個数が不正です"
    );
    let mut result = Reduction {
        active: vec![true; amounts.len()],
        removed: 0,
        tried: 0,
        baseline_exceeded: 0,
    };
    for id in 0..amounts.len() {
        if result.tried >= limit as u64 {
            break;
        }
        if protected[id]
            || model.base.topology.hull.contains(&id)
            || model.base.neighbors[id].iter().any(|&i| protected[i])
        {
            continue;
        }
        let region = influence(model, id);
        if region[0] < bounds[0]
            || region[1] < bounds[1]
            || region[2] > bounds[2]
            || region[3] > bounds[3]
        {
            continue;
        }
        let neighbors = model.base.neighbors[id].clone();
        result.tried += 1;
        let undo = model.remove_local(id)?;
        let mut region = region;
        for &i in &neighbors {
            let b = model.deletion_box(i, STEP * 2.);
            region = [
                region[0].min(b[0]),
                region[1].min(b[1]),
                region[2].max(b[2]),
                region[3].max(b[3]),
            ];
        }
        let changed_faces = neighbors
            .iter()
            .flat_map(|&i| model.base.incident[i].iter().copied())
            .collect::<std::collections::BTreeSet<_>>();
        let angles_ok = changed_faces.into_iter().all(|face| {
            let t = &model.base.topology.triangles[face * 3..face * 3 + 3];
            if !t.iter().all(|i| neighbors.contains(i)) {
                return true;
            }
            (0..3).all(|j| {
                let a = model.base.points[t[j]];
                let b = model.base.points[t[(j + 1) % 3]];
                let c = model.base.points[t[(j + 2) % 3]];
                let u = [b[0] - a[0], b[1] - a[1]];
                let v = [c[0] - a[0], c[1] - a[1]];
                let cos = (u[0] * v[0] + u[1] * v[1])
                    / ((u[0] * u[0] + u[1] * u[1]) * (v[0] * v[0] + v[1] * v[1])).sqrt();
                cos.clamp(-1., 1.).acos().to_degrees() >= 10. - 1e-8
            })
        });
        if region[0] < bounds[0]
            || region[1] < bounds[1]
            || region[2] > bounds[2]
            || region[3] > bounds[3]
            || !angles_ok
        {
            model.undo_remove_local(undo);
            continue;
        }
        match audit(model, region) {
            Ok((true, baseline)) => {
                result.active[id] = false;
                result.removed += 1;
                result.baseline_exceeded += u64::from(baseline);
            }
            Ok((false, _)) => model.undo_remove_local(undo),
            Err(e) => {
                model.undo_remove_local(undo);
                return Err(e);
            }
        }
    }
    Ok(result)
}
fn repack(
    tile: &mut Tile,
    points: &[[i32; 3]],
    amounts: &[u8],
    parents: &[Vec<[i32; 3]>],
    automatic: bool,
) -> Result<()> {
    let local = points
        .iter()
        .map(|p| [p[0] - tile.origin[0], p[1] - tile.origin[1], p[2]])
        .collect::<Vec<_>>();
    let independent = natural_codec::independent(&local)?;
    let mut lookup = HashMap::new();
    for (slot, parent) in parents.iter().enumerate() {
        for (index, p) in parent.iter().enumerate() {
            lookup.entry([p[0], p[1]]).or_insert((slot, index));
        }
    }
    let refs = points
        .iter()
        .map(|p| lookup.get(&[p[0], p[1]]).copied())
        .collect::<Vec<_>>();
    let controls = if parents.is_empty() {
        independent
    } else {
        let delta = natural_codec::delta(&local, &refs, parents)?;
        if delta.len() < independent.len() {
            delta
        } else {
            tile.parents.clear();
            independent
        }
    };
    tile.controls = if automatic {
        controls
    } else {
        control_attributes::attach(&controls, amounts)?
    };
    tile.count = points.len();
    tile.hash = point_hash(points);
    Ok(())
}
/// 追加削減中も元SQLiteを変更せず、全画素監査と親復号の完了後に新規出力を確定する。
pub fn generate(
    prepared: &Path,
    input: &Path,
    output: &Path,
    identifier: &str,
    window: &[u32],
    candidate_limit: usize,
) -> Result<()> {
    ensure!(!output.exists(), "出力先は新規ファイルを指定してください");
    ensure!(
        window.is_empty()
            || (window.len() == 4
                && window[0] < window[2]
                && window[1] < window[3]
                && window[2] <= 16384
                && window[3] <= 16384),
        "z14窓が不正です"
    );
    ensure!(
        !identifier.is_empty() && candidate_limit > 0,
        "配布識別子または候補上限が不正です"
    );
    let job: Value = serde_json::from_slice(&std::fs::read(prepared.join("job.json"))?)?;
    let mut source = Database::open(input)?;
    let raw: String = source.connection.query_row(
        "SELECT metadata_json FROM datasets WHERE dataset_key=?1",
        [source.dataset],
        |r| r.get(0),
    )?;
    let mut metadata: Value = serde_json::from_str(&raw)?;
    ensure!(
        job["generation_id"].as_str().is_some()
            && job["generation_id"] == metadata["generation"]["id"]
            && job["inputs_verified"] == true,
        "元DEMの世代が一致しません"
    );
    ensure!(
        metadata["generation"]["fixedSlopeAdjustment"]["certifiedMultiplier"] == 1
            || metadata["generation"]["localSlopeAdjustment"]["certifiedMultiplier"] == 1
            || metadata["generation"]["automaticSlopeAdjustment"]["certifiedMultiplier"] == 1,
        "倍率1で監査した局所係数付きデータが必要です"
    );
    if !source.fixed_slope_levels.is_empty() {
        ensure!(
            metadata["generation"]["fixedSlopeAdjustment"]["peakGuardMetres"] == PEAK_GUARD,
            "固定係数データは突起監査済みのものを指定してください"
        );
    }
    let filtered = crate::slope_smoothing::Filtered::from_metadata(prepared, &metadata)?;
    let root = Path::new(
        job["identity"]["cache"]
            .as_str()
            .context("元DEMの場所がありません")?,
    );
    let inventory = Connection::open_with_flags(
        prepared.join("inputs.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let directory = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::NamedTempFile::new_in(directory)?.into_temp_path();
    let mut target = Connection::open(&temporary)?;
    Backup::new(&source.connection, &mut target)?.run_to_completion(
        256,
        std::time::Duration::ZERO,
        None,
    )?;
    let mut keys = source.catalog.keys().copied().collect::<Vec<_>>();
    keys.sort_unstable();
    let mut required = HashSet::new();
    for &key in &keys {
        required.extend(source.tile(key)?.parents);
    }
    let (
        mut original_points,
        mut removed,
        mut tried,
        mut baseline_exceeded,
        mut audited,
        mut maximum,
        mut squared,
    ) = (0u64, 0u64, 0u64, 0u64, 0u64, 0f64, 0f64);
    let mut changed = HashMap::new();
    for z in keys
        .iter()
        .map(|k| k.0)
        .collect::<std::collections::BTreeSet<_>>()
    {
        let level = keys
            .iter()
            .copied()
            .filter(|k| k.0 == z)
            .collect::<Vec<_>>();
        let mut uses = HashMap::<[i32; 2], u32>::new();
        for &key in &level {
            for p in source.points(key)?.iter() {
                *uses.entry([p[0], p[1]]).or_default() += 1;
            }
        }
        for key in level {
            let points = source.points(key)?;
            original_points += points.len() as u64;
            if required.contains(&key) || !fully_inside(key, window) {
                continue;
            }
            let automatic = source.automatic_slope_levels.contains(&z);
            let fixed = source.fixed_slope_levels.contains(&z);
            let amounts = if automatic || fixed {
                std::sync::Arc::new(vec![255; points.len()])
            } else {
                source
                    .slope_adjustments(key)?
                    .context("局所係数がありません")?
            };
            let side = WORLD / 2f64.powi(z as i32);
            let bounds = [0., 0., side, side];
            let (down, up) = errors(&source, z)?;
            let rasters = if let Some(filtered) = &filtered {
                filtered.rasters(key)?
            } else {
                read_rasters(root, &inventory, key)?
            };
            if rasters.is_empty() {
                continue;
            }
            let mut model = model(&points, key)?;
            let protected = points
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let q = model.base.points[i];
                    let neighbors = &model.base.neighbors[i];
                    let nonflat = neighbors.iter().any(|&j| model.base.points[j][2] != q[2]);
                    let extremum = nonflat
                        && (neighbors.iter().all(|&j| model.base.points[j][2] <= q[2])
                            || neighbors.iter().all(|&j| model.base.points[j][2] >= q[2]));
                    (fixed
                        && if let Some(filtered) = &filtered {
                            filtered.protects(f64::from(p[0]) / 16., f64::from(p[1]) / 16.)
                        } else {
                            extremum
                        })
                        || uses[&[p[0], p[1]]] > 1
                        || q[0] <= 0.
                        || q[1] <= 0.
                        || q[0] >= side
                        || q[1] >= side
                })
                .collect::<Vec<_>>();
            let reduction = reduce(
                &mut model,
                &amounts,
                &protected,
                bounds,
                candidate_limit,
                |model, region| {
                    let surface = if automatic {
                        Prepared::automatic(model)
                    } else {
                        Prepared::new(model)
                    };
                    let mut work = Work::default();
                    let mut baseline = false;
                    let valid = visit(&rasters, key, region, |q, h| {
                        let pair = surface.pair(
                            q,
                            model.basis_borrowed(q, &mut work)?,
                            if automatic { None } else { Some(&amounts) },
                        )?;
                        let error = pair.value(1.) - h;
                        baseline |=
                            pair.original - h > up + 1e-7 || h - pair.original > down + 1e-7;
                        Ok(error <= up + 1e-7 && -error <= down + 1e-7)
                    })?;
                    let valid = if valid && fixed {
                        shape_audit(&rasters, key, region, &surface, None)?.count == 0
                    } else {
                        valid
                    };
                    Ok((valid, baseline))
                },
            )?;
            let surface = if automatic {
                Prepared::automatic(&model)
            } else {
                Prepared::new(&model)
            };
            let mut work = Work::default();
            visit(&rasters, key, bounds, |q, h| {
                let e = surface.evaluate(
                    q,
                    if automatic { None } else { Some(&amounts) },
                    1.,
                    &mut work,
                )? - h;
                ensure!(
                    e <= up + 1e-7 && -e <= down + 1e-7,
                    "削減後の全画素誤差が許容値を超えています"
                );
                audited += 1;
                maximum = maximum.max(e.abs());
                squared += e * e;
                Ok(true)
            })?;
            tried += reduction.tried;
            removed += reduction.removed;
            baseline_exceeded += reduction.baseline_exceeded;
            if reduction.removed > 0 {
                let kept = points
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| reduction.active[*i])
                    .map(|(_, p)| *p)
                    .collect::<Vec<_>>();
                let kept_amounts = amounts
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| reduction.active[*i])
                    .map(|(_, a)| *a)
                    .collect::<Vec<_>>();
                let mut tile = source.tile(key)?;
                let parents = tile
                    .parents
                    .iter()
                    .map(|k| source.points(*k).map(|p| p.as_ref().clone()))
                    .collect::<Result<Vec<_>>>()?;
                repack(
                    &mut tile,
                    &kept,
                    &kept_amounts,
                    &parents,
                    automatic || fixed,
                )?;
                // コンパクト化した点順序でも同じ曲面となることを元DEMで独立確認する。
                let compact = crate::local_slope::model(&kept, key)?;
                let prepared = if automatic {
                    Prepared::automatic(&compact)
                } else {
                    Prepared::new(&compact)
                };
                let mut work = Work::default();
                visit(&rasters, key, bounds, |q, h| {
                    let e = prepared.evaluate(
                        q,
                        if automatic { None } else { Some(&kept_amounts) },
                        1.,
                        &mut work,
                    )? - h;
                    ensure!(
                        e <= up + 1e-7 && -e <= down + 1e-7,
                        "保存点の再構築後に誤差が超過しました"
                    );
                    Ok(true)
                })?;
                target.execute("UPDATE terrain_tiles SET payload=?1 WHERE dataset_key=?2 AND level_key=(SELECT level_key FROM terrain_levels WHERE dataset_key=?2 AND z=?3) AND x=?4 AND y=?5",params![tile.encode()?,source.dataset,z,key.1,key.2])?;
                changed.insert(key, kept);
            }
            eprintln!(
                "局所傾き削減 z{z} {}/{} tried={} removed={}",
                key.1, key.2, reduction.tried, reduction.removed
            );
        }
    }
    let report = json!({"certifiedMultiplier":1,"pointsBefore":original_points,"pointsAfter":original_points-removed,"tried":tried,"removed":removed,"acceptedWithOriginalSurfaceError":baseline_exceeded,"auditedPixels":audited,"maximumError":maximum,"rmse":if audited>0{Some((squared/audited as f64).sqrt())}else{None},"candidateLimitPerTile":candidate_limit,"windowZ14":window,"sourceDataVersion":source.generation,"sharedPointsUnchanged":true,"parentTilesUnchanged":true,"engineSha256":crate::natural_inventory::hash_file(&std::env::current_exe()?)?});
    metadata["generation"]["localSlopePruning"] = report.clone();
    if removed > 0 && metadata["generation"].get("localSlopeAdjustment").is_some() {
        metadata["generation"]["localSlopeAdjustment"]["xyzUnchanged"] = json!(false);
    }
    target.execute(
        "UPDATE datasets SET data_version=?1,metadata_json=?2 WHERE dataset_key=?3",
        params![identifier, metadata.to_string(), source.dataset],
    )?;
    target.execute_batch("VACUUM;PRAGMA optimize;")?;
    yamamuki_data_sqlite::validate(&target)?;
    drop(target);
    let mut check = Database::open(&temporary)?;
    check.validate_all()?;
    for &key in &keys {
        let restored = check.points(key)?;
        if let Some(expected) = changed.get(&key) {
            ensure!(
                restored.as_ref() == expected,
                "削減後の復元XYZが一致しません"
            );
        } else {
            ensure!(
                restored == source.points(key)?,
                "変更対象外のタイルが変化しました"
            );
        }
    }
    drop(check);
    temporary.persist_noclobber(output)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shape_guard_detects_added_prominence_and_accepts_matching_dem() -> Result<()> {
        let peak = 128.5 * STEP;
        let model = Fast::new(
            (-2..=2)
                .flat_map(|y| {
                    (-2..=2).map(move |x| {
                        [
                            peak + x as f64 * 100.,
                            peak + y as f64 * 100.,
                            1000. - 100. * (x * x + y * y) as f64,
                        ]
                    })
                })
                .collect(),
        )?;
        let surface = Prepared::new(&model);
        let region = [
            peak - 2. * STEP,
            peak - 2. * STEP,
            peak + 2. * STEP,
            peak + 2. * STEP,
        ];
        let mut raster = Raster {
            x: 0,
            y: 0,
            values: vec![1000.; 65536],
        };
        let shape = shape_audit(&[raster.clone()], (13, 0, 0), region, &surface, None)?;
        assert!(shape.count > 0);
        assert!(!shape.candidates.is_empty());
        let mut work = Work::default();
        for y in 120..138 {
            for x in 120..138 {
                raster.values[y * 256 + x] = surface.evaluate(
                    [(x as f64 + 0.5) * STEP, (y as f64 + 0.5) * STEP],
                    None,
                    1.,
                    &mut work,
                )?;
            }
        }
        let shape = shape_audit(&[raster], (13, 0, 0), region, &surface, None)?;
        assert_eq!(shape.count, 0);
        assert!(shape.maximum < 1e-8);
        Ok(())
    }
    #[test]
    fn pruning_accepts_multiplier_one_without_requiring_zero() -> Result<()> {
        let mut points = vec![[400., 400., 160.]];
        for y in 0..9 {
            for x in 0..9 {
                if x == 4 && y == 4 {
                    continue;
                }
                points.push([
                    x as f64 * 100.,
                    y as f64 * 100.,
                    (x * x + 2 * y * y + x * y) as f64 * 10.,
                ]);
            }
        }
        // 中央の標高も同じ二次曲面にする。
        points[0][2] = 640.;
        let before = Fast::new(points.clone())?;
        let after = before.without_point(0)?;
        let mut work = Work::default();
        let q = [400., 400.];
        let surface = Prepared::new(&after);
        let pair = surface.pair(q, after.basis_borrowed(q, &mut work)?, None)?;
        let target = pair.corrected;
        let error = (pair.original - pair.corrected).abs() * 0.25;
        assert!(error > 1e-4);
        let amounts = vec![255; points.len()];
        let protected = vec![false; points.len()];
        let mut model = before;
        let result = reduce(
            &mut model,
            &amounts,
            &protected,
            [0., 0., 800., 800.],
            1,
            |m, _| {
                let surface = Prepared::new(m);
                let pair = surface.pair(q, m.basis_borrowed(q, &mut work)?, Some(&amounts))?;
                Ok((
                    (pair.value(1.) - target).abs() <= error,
                    (pair.original - target).abs() > error,
                ))
            },
        )?;
        assert_eq!(result.removed, 1);
        assert_eq!(result.baseline_exceeded, 1);
        assert!(!result.active[0]);
        Ok(())
    }
    #[test]
    fn protected_shared_neighbors_prevent_changes() -> Result<()> {
        let points = (0..7)
            .flat_map(|y| (0..7).map(move |x| [x as f64 * 10., y as f64 * 10., (x + y) as f64]))
            .collect::<Vec<_>>();
        let mut model = Fast::new(points)?;
        let n = model.base.points.len();
        let amounts = vec![255; n];
        let protected = vec![true; n];
        let result = reduce(
            &mut model,
            &amounts,
            &protected,
            [0., 0., 60., 60.],
            usize::MAX,
            |_, _| panic!("保護点を削減してはいけません"),
        )?;
        assert_eq!(result.removed, 0);
        assert_eq!(result.tried, 0);
        Ok(())
    }
}
