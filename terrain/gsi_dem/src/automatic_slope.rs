//! 制御点からの係数計算を、同じ地域・単独タイル符号化で比較する。
use crate::{
    local_slope::{errors, fully_inside},
    local_slope_prune::{PEAK_GUARD, Raster, read_rasters, shape_audit, visit},
};
use anyhow::{Context, Result, ensure};
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, backup::Backup, params};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::Path,
};
use yamamuki_terrain::{
    control_attributes,
    local_slope::Prepared,
    natural_codec,
    natural_local_gradient::{AUTOMATIC_SLOPE_MODE, FIXED_SLOPE_MODE},
    natural_regional_surface::{Fast, Work},
    sqlite::{Database, Key, Tile, WORLD, point_hash},
};
const STEP: f64 = WORLD / (16384. * 256.);
#[derive(Clone)]
struct Input {
    key: Key,
    rasters: Vec<Raster>,
}
#[derive(Default, Clone)]
struct Audit {
    pixels: u64,
    squared: f64,
    up: f64,
    down: f64,
    failures: u64,
    candidates: Vec<[f64; 3]>,
}
fn audit(
    model: &Fast,
    inputs: &[Input],
    origin: [f64; 2],
    down: f64,
    up: f64,
    fixed: bool,
) -> Result<Audit> {
    let prepared = if fixed {
        Prepared::new(model)
    } else {
        Prepared::automatic(model)
    };
    let parts = inputs
        .par_iter()
        .map(|input| -> Result<Audit> {
            let mut result = Audit::default();
            let mut work = Work::default();
            for raster in &input.rasters {
                let mut worst_score = 1.;
                let mut worst = None;
                for (index, &h) in raster.values.iter().enumerate() {
                    if !h.is_finite() {
                        continue;
                    }
                    let q = [
                        (f64::from(raster.x) * 256. + (index % 256) as f64 + 0.5) * STEP
                            - origin[0],
                        (f64::from(raster.y) * 256. + (index / 256) as f64 + 0.5) * STEP
                            - origin[1],
                    ];
                    let e = prepared.evaluate(q, None, 1., &mut work)? - h;
                    result.pixels += 1;
                    result.squared += e * e;
                    result.up = result.up.max(e);
                    result.down = result.down.max(-e);
                    if e > up + 1e-7 || -e > down + 1e-7 {
                        result.failures += 1;
                        let score = (e / up).max(-e / down);
                        if score > worst_score {
                            worst_score = score;
                            worst = Some([q[0], q[1], h]);
                        }
                    }
                }
                if let Some(q) = worst {
                    result.candidates.push(q);
                }
            }
            Ok(result)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut total = Audit::default();
    for a in parts {
        total.pixels += a.pixels;
        total.squared += a.squared;
        total.up = total.up.max(a.up);
        total.down = total.down.max(a.down);
        total.failures += a.failures;
        total.candidates.extend(a.candidates);
    }
    Ok(total)
}
fn support(model: &Fast, bounds: [f64; 4]) -> Vec<usize> {
    let mut selected = BTreeSet::new();
    for face in model.base.faces_for_rect(bounds) {
        selected.extend(
            model.base.topology.triangles[face * 3..face * 3 + 3]
                .iter()
                .copied(),
        );
    }
    let core = selected.iter().copied().collect::<Vec<_>>();
    for id in core {
        selected.extend(model.base.neighbors[id].iter().copied());
    }
    selected.into_iter().collect()
}
fn create_variant(
    source_path: &Path,
    output: &Path,
    identifier: &str,
    keys: &[Key],
    window: &[u32],
    kind: &str,
    automatic: Option<(&Fast, [f64; 2])>,
    inputs: &[Input],
    fixed: bool,
    smoothing: Option<&Value>,
) -> Result<Value> {
    ensure!(
        !output.exists(),
        "比較データの出力先は新規ファイルを指定してください"
    );
    let mut source = Database::open(source_path)?;
    let mut metadata: Value = serde_json::from_str(&source.connection.query_row::<String, _, _>(
        "SELECT metadata_json FROM datasets WHERE dataset_key=?1",
        [source.dataset],
        |r| r.get(0),
    )?)?;
    let temporary = tempfile::NamedTempFile::new_in(output.parent().unwrap())?.into_temp_path();
    let mut db = Connection::open(&temporary)?;
    Backup::new(&source.connection, &mut db)?.run_to_completion(
        256,
        std::time::Duration::ZERO,
        None,
    )?;
    db.execute(
        "DELETE FROM terrain_tiles WHERE dataset_key=?1",
        [source.dataset],
    )?;
    db.execute(
        "DELETE FROM terrain_levels WHERE dataset_key=?1 AND z<>?2",
        params![source.dataset, keys[0].0],
    )?;
    let side = WORLD / 2f64.powi(keys[0].0 as i32);
    let source_origin = [f64::from(keys[0].1) * side, f64::from(keys[0].2) * side];
    let mut source_points = BTreeSet::new();
    let mut source_coefficients = HashMap::new();
    if automatic.is_none() {
        for &key in keys {
            let points = source.points(key)?;
            source_points.extend(points.iter().copied());
            if kind == "stored" {
                let amounts = source
                    .slope_adjustments(key)?
                    .context("比較する保存係数がありません")?;
                for (p, &a) in points.iter().zip(amounts.iter()) {
                    if let Some(old) = source_coefficients.insert(*p, a) {
                        ensure!(old == a, "共有点の保存係数が一致しません");
                    }
                }
            }
        }
    }
    let source_union = if automatic.is_none() {
        Some(Fast::certified(
            source_points
                .iter()
                .map(|p| {
                    [
                        f64::from(p[0]) / 16. - source_origin[0],
                        f64::from(p[1]) / 16. - source_origin[1],
                        f64::from(p[2]) / 8.,
                    ]
                })
                .collect(),
        )?)
    } else {
        None
    };
    let automatic_model = automatic.map(|v| {
        if fixed {
            Prepared::new(v.0)
        } else {
            Prepared::automatic(v.0)
        }
    });
    let mut total_points = 0u64;
    let mut total_bytes = 0u64;
    let (mut squared, mut pixels, mut maximum) = (0., 0u64, 0f64);
    for &key in keys {
        let old = source.tile(key)?;
        let side = WORLD / 2f64.powi(key.0 as i32);
        let (field, origin) =
            automatic.unwrap_or_else(|| (source_union.as_ref().unwrap(), source_origin));
        let bounds = [
            f64::from(key.1) * side - origin[0] - 32.,
            f64::from(key.2) * side - origin[1] - 32.,
            f64::from(key.1 + 1) * side - origin[0] + 32.,
            f64::from(key.2 + 1) * side - origin[1] + 32.,
        ];
        let mut points = support(field, bounds)
            .into_iter()
            .map(|i| {
                let p = field.base.points[i];
                [
                    ((p[0] + origin[0]) * 16.).round() as i32,
                    ((p[1] + origin[1]) * 16.).round() as i32,
                    (p[2] * 8.).round() as i32,
                ]
            })
            .collect::<Vec<_>>();
        points.sort_unstable();
        let amounts = (kind == "stored").then(|| {
            points
                .iter()
                .map(|p| source_coefficients[p])
                .collect::<Vec<_>>()
        });
        let local = points
            .iter()
            .map(|p| [p[0] - old.origin[0], p[1] - old.origin[1], p[2]])
            .collect::<Vec<_>>();
        let controls = natural_codec::independent(&local)?;
        let controls = if let Some(a) = &amounts {
            control_attributes::attach(&controls, a)?
        } else {
            controls
        };
        let tile = Tile {
            origin: old.origin,
            count: points.len(),
            parents: vec![],
            hash: point_hash(&points),
            controls,
        };
        let payload = tile.encode()?;
        total_points += points.len() as u64;
        total_bytes += payload.len() as u64;
        db.execute("INSERT INTO terrain_tiles SELECT dataset_key,level_key,?1,?2,'natural-controls-v1',?3 FROM terrain_levels WHERE dataset_key=?4 AND z=?5",params![key.1,key.2,payload,source.dataset,key.0])?;
        let local_model = crate::local_slope::model(&points, key)?;
        let surface = if automatic.is_some() {
            if fixed {
                Prepared::new(&local_model)
            } else {
                Prepared::automatic(&local_model)
            }
        } else {
            Prepared::new(&local_model)
        };
        let mut work = Work::default();
        let mut reference_work = Work::default();
        let (down, up) = errors(&source, key.0)?;
        let input = inputs
            .iter()
            .find(|i| i.key == key)
            .context("監査する元DEMがありません")?;
        let multiplier = if kind == "baseline" { 0. } else { 1. };
        visit(&input.rasters, key, [0., 0., side, side], |q, h| {
            let value = surface.evaluate(q, amounts.as_deref(), multiplier, &mut work)?;
            let e = value - h;
            ensure!(
                e <= up + 1e-7 && -e <= down + 1e-7,
                "比較データの保存点が許容誤差を超えています: {kind} {key:?} error={e}"
            );
            if let Some(reference) = &automatic_model {
                let origin = automatic.unwrap().1;
                let global = [
                    q[0] + f64::from(key.1) * side - origin[0],
                    q[1] + f64::from(key.2) * side - origin[1],
                ];
                let expected = reference.evaluate(global, None, 1., &mut reference_work)?;
                ensure!(
                    (expected - value).abs() < 1e-5,
                    "局所支持の曲面が全体の曲面と一致しません"
                );
            }
            pixels += 1;
            squared += e * e;
            maximum = maximum.max(e.abs());
            Ok(true)
        })?;
        if automatic.is_some() {
            let reference = automatic_model.as_ref().unwrap();
            let origin = automatic.unwrap().1;
            // 境界と法線差分用の外側1mも、共通曲面へ照合する。
            for i in 0..=32 {
                let t = side * i as f64 / 32.;
                for q in [
                    [-1., t],
                    [side + 1., t],
                    [t, -1.],
                    [t, side + 1.],
                    [0., t],
                    [side, t],
                    [t, 0.],
                    [t, side],
                ] {
                    let actual = surface.evaluate(q, None, 1., &mut work)?;
                    let expected = reference.evaluate(
                        [
                            q[0] + f64::from(key.1) * side - origin[0],
                            q[1] + f64::from(key.2) * side - origin[1],
                        ],
                        None,
                        1.,
                        &mut reference_work,
                    )?;
                    ensure!(
                        (actual - expected).abs() < 1e-5,
                        "タイル境界の自動係数曲面が一致しません"
                    );
                }
            }
        }
    }
    let mut settings: Value = serde_json::from_str(&db.query_row::<String, _, _>(
        "SELECT settings_json FROM terrain_levels WHERE dataset_key=?1",
        [source.dataset],
        |r| r.get(0),
    )?)?;
    if automatic.is_some() {
        settings["slopeMode"] = json!(if fixed {
            FIXED_SLOPE_MODE
        } else {
            AUTOMATIC_SLOPE_MODE
        });
    } else {
        settings.as_object_mut().unwrap().remove("slopeMode");
    }
    settings["errorReference"] = json!(if smoothing.is_some() {
        "smoothed-dem"
    } else {
        "raw-dem"
    });
    db.execute(
        "UPDATE terrain_levels SET settings_json=?1 WHERE dataset_key=?2",
        params![settings.to_string(), source.dataset],
    )?;
    if let Some(profile) = smoothing {
        metadata["generation"]["sourceSmoothing"] = profile.clone();
        if let Some(sources) = metadata.get_mut("sources").and_then(Value::as_array_mut) {
            for source in sources {
                if let Some(changes) = source["changes"].as_str() {
                    source["changes"] =
                        json!(format!("{changes}。主要地形を保護し斜面の小起伏を平滑化"));
                }
            }
        }
    }
    metadata["generation"]["scope"] = json!("window-test");
    metadata["generation"]["sourceWindowZ14"] = json!(window);
    metadata["generation"]["errorEvaluationMultiplier"] =
        json!(if kind == "baseline" { 0 } else { 1 });
    let result = json!({"variant":kind,"tiles":keys.len(),"points":total_points,"payloadBytes":total_bytes,"validPixels":pixels,"rmse":(squared/pixels as f64).sqrt(),"maximumError":maximum,"independentEncoding":true,"sourceDataVersion":source.generation});
    metadata["generation"]["coefficientExperiment"] = result.clone();
    if automatic.is_some() {
        metadata["generation"]
            .as_object_mut()
            .unwrap()
            .remove("localSlopeAdjustment");
        metadata["generation"]
            .as_object_mut()
            .unwrap()
            .remove("localSlopePruning");
        metadata["generation"][if fixed {
            "fixedSlopeAdjustment"
        } else {
            "automaticSlopeAdjustment"
        }] = json!({"mode":if fixed{FIXED_SLOPE_MODE}else{AUTOMATIC_SLOPE_MODE},"certifiedMultiplier":1,"peakGuardMetres":if fixed{Some(PEAK_GUARD)}else{None},"storedCoefficientBytes":0,"completeVoronoiOneRing":true,"engineSha256":crate::natural_inventory::hash_file(&std::env::current_exe()?)?});
    }
    db.execute(
        "UPDATE datasets SET data_version=?1,metadata_json=?2 WHERE dataset_key=?3",
        params![identifier, metadata.to_string(), source.dataset],
    )?;
    db.execute_batch("VACUUM;PRAGMA optimize;")?;
    yamamuki_data_sqlite::validate(&db)?;
    drop(db);
    let mut check = Database::open(&temporary)?;
    check.validate_all()?;
    drop(check);
    temporary.persist_noclobber(output)?;
    let mut result = result;
    result["sqliteBytes"] = json!(std::fs::metadata(output)?.len());
    Ok(result)
}
/// 地域比較専用。対象レベルのタイルを独立本文へ揃え、親の容量差を比較へ混ぜない。
pub fn experiment(
    prepared: &Path,
    source_path: &Path,
    stored_path: &Path,
    output: &Path,
    identifier: &str,
    z: u32,
    window: &[u32],
    point_limit: usize,
) -> Result<()> {
    experiment_with_mode(
        prepared,
        source_path,
        stored_path,
        output,
        identifier,
        z,
        window,
        point_limit,
        false,
    )
}
pub fn experiment_with_mode(
    prepared: &Path,
    source_path: &Path,
    stored_path: &Path,
    output: &Path,
    identifier: &str,
    z: u32,
    window: &[u32],
    point_limit: usize,
    fixed: bool,
) -> Result<()> {
    experiment_with_processing(
        prepared,
        source_path,
        stored_path,
        output,
        identifier,
        z,
        window,
        point_limit,
        fixed,
        false,
    )
}
pub fn experiment_with_processing(
    prepared: &Path,
    source_path: &Path,
    stored_path: &Path,
    output: &Path,
    identifier: &str,
    z: u32,
    window: &[u32],
    point_limit: usize,
    fixed: bool,
    smooth_slopes: bool,
) -> Result<()> {
    ensure!(
        !smooth_slopes || fixed,
        "斜面平滑化は固定係数モードで指定してください"
    );
    ensure!(
        z <= 14
            && window.len() == 4
            && window[0] < window[2]
            && window[1] < window[3]
            && window[2] <= 16384
            && window[3] <= 16384,
        "比較するz14窓が不正です"
    );
    ensure!(
        !output.exists(),
        "比較結果のディレクトリは新規にしてください"
    );
    let job: Value = serde_json::from_slice(&std::fs::read(prepared.join("job.json"))?)?;
    let mut source = Database::open(source_path)?;
    let raw: String = source.connection.query_row(
        "SELECT metadata_json FROM datasets WHERE dataset_key=?1",
        [source.dataset],
        |r| r.get(0),
    )?;
    let metadata: Value = serde_json::from_str(&raw)?;
    ensure!(
        job["inputs_verified"] == true
            && job["generation_id"].as_str().is_some()
            && job["generation_id"] == metadata["generation"]["id"],
        "元DEMの世代が一致しません"
    );
    let root = Path::new(
        job["identity"]["cache"]
            .as_str()
            .context("元DEMの場所がありません")?,
    );
    let inventory = Connection::open_with_flags(
        prepared.join("inputs.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut keys = source
        .catalog
        .keys()
        .copied()
        .filter(|k| k.0 == z && fully_inside(*k, window))
        .collect::<Vec<_>>();
    keys.sort_unstable();
    ensure!(!keys.is_empty(), "窓に全体が入る比較タイルがありません");
    let mut seed = BTreeSet::new();
    let mut inputs = Vec::new();
    for &key in &keys {
        seed.extend(source.points(key)?.iter().copied());
        inputs.push(Input {
            key,
            rasters: read_rasters(root, &inventory, key)?,
        });
    }
    ensure!(
        inputs.iter().all(|i| !i.rasters.is_empty()),
        "元DEMのない比較タイルがあります"
    );
    ensure!(
        seed.len() < point_limit,
        "初期の制御点が点数予算を超えています"
    );
    let raw_inputs = inputs.clone();
    let smoothing = if smooth_slopes {
        Some(crate::slope_smoothing::Filtered::load(
            prepared,
            window,
            Default::default(),
        )?)
    } else {
        None
    };
    if let Some(filtered) = &smoothing {
        eprintln!("斜面平滑化: {}", serde_json::to_string(&filtered.profile)?);
        inputs = keys
            .iter()
            .map(|&key| {
                Ok(Input {
                    key,
                    rasters: filtered.rasters(key)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut adjusted = BTreeMap::new();
        for p in &seed {
            let height = filtered
                .sample(f64::from(p[0]) / 16., f64::from(p[1]) / 16.)
                .map_or(p[2], |h| (h * 8.).round() as i32);
            adjusted.insert([p[0], p[1]], height);
        }
        for p in filtered.anchors() {
            adjusted.insert([p[0], p[1]], p[2]);
        }
        seed = adjusted
            .into_iter()
            .map(|(xy, h)| [xy[0], xy[1], h])
            .collect();
        ensure!(
            seed.len() < point_limit,
            "平滑化後の保護点が点数予算を超えています"
        );
    }
    let side = WORLD / 2f64.powi(z as i32);
    let origin = [f64::from(keys[0].1) * side, f64::from(keys[0].2) * side];
    let mut model = Fast::certified(
        seed.iter()
            .map(|p| {
                [
                    f64::from(p[0]) / 16. - origin[0],
                    f64::from(p[1]) / 16. - origin[1],
                    f64::from(p[2]) / 8.,
                ]
            })
            .collect(),
    )?;
    let mut occupied = seed.iter().map(|p| [p[0], p[1]]).collect::<HashSet<_>>();
    let initial_points = model.base.points.len();
    let (down, up) = errors(&source, z)?;
    let mut initial = None;
    let all_rasters = inputs
        .iter()
        .flat_map(|i| i.rasters.iter().cloned())
        .collect::<Vec<_>>();
    let region = [
        keys.iter()
            .map(|k| f64::from(k.1) * side - origin[0])
            .fold(f64::INFINITY, f64::min),
        keys.iter()
            .map(|k| f64::from(k.2) * side - origin[1])
            .fold(f64::INFINITY, f64::min),
        keys.iter()
            .map(|k| f64::from(k.1 + 1) * side - origin[0])
            .fold(f64::NEG_INFINITY, f64::max),
        keys.iter()
            .map(|k| f64::from(k.2 + 1) * side - origin[1])
            .fold(f64::NEG_INFINITY, f64::max),
    ];
    let mut peak_passes = 0;
    let mut peak_maximum = 0.;
    let mut passes = 0;
    loop {
        let mut result = audit(&model, &inputs, origin, down, up, fixed)?;
        if initial.is_none() {
            initial = Some(result.clone());
        }
        eprintln!(
            "自動係数監査 pass={passes} points={} violations={} up={:.6} down={:.6}",
            model.base.points.len(),
            result.failures,
            result.up,
            result.down
        );
        ensure!(result.pixels > 0, "比較する有効DEM画素がありません");
        if result.failures == 0 && fixed {
            let surface = Prepared::new(&model);
            let shape = shape_audit(&all_rasters, keys[0], region, &surface, None)?;
            peak_maximum = shape.maximum;
            eprintln!(
                "固定係数の突起監査 peaks={} excess={:.6}m",
                shape.count, shape.maximum
            );
            if shape.count > 0 {
                peak_passes += 1;
                result.failures = shape.count;
                result.candidates.extend(shape.candidates);
            }
        }
        if result.failures == 0 {
            break;
        }

        ensure!(
            model.base.points.len() + result.candidates.len() <= point_limit && passes < 4096,
            "自動係数曲面の適合が点数・反復予算を超えました"
        );
        let mut added = 0;
        for q in result.candidates {
            let x = ((q[0] + origin[0]) * 16.).round() as i32;
            let y = ((q[1] + origin[1]) * 16.).round() as i32;
            if !occupied.insert([x, y]) {
                continue;
            }
            let h = (q[2] * 8.).round() / 8.;
            model.insert_local([
                f64::from(x) / 16. - origin[0],
                f64::from(y) / 16. - origin[1],
                h,
            ])?;
            added += 1;
        }
        ensure!(added > 0, "既存点の位置で自動係数の誤差が収束しません");
        passes += 1;
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let staging = tempfile::tempdir_in(parent)?;
    let destination = output;
    let output = staging.path();
    let baseline = create_variant(
        source_path,
        &output.join("baseline.sqlite3"),
        identifier,
        &keys,
        window,
        "baseline",
        None,
        &raw_inputs,
        fixed,
        None,
    )?;
    let stored = create_variant(
        stored_path,
        &output.join("stored.sqlite3"),
        identifier,
        &keys,
        window,
        "stored",
        None,
        &raw_inputs,
        fixed,
        None,
    )?;
    let automatic = create_variant(
        source_path,
        &output.join(if fixed {
            "fixed.sqlite3"
        } else {
            "automatic.sqlite3"
        }),
        identifier,
        &keys,
        window,
        if fixed { "fixed" } else { "automatic" },
        Some((&model, origin)),
        &inputs,
        fixed,
        smoothing.as_ref().map(|f| &f.profile),
    )?;
    crate::local_slope_prune::generate(
        prepared,
        &output.join(if fixed {
            "fixed.sqlite3"
        } else {
            "automatic.sqlite3"
        }),
        &output.join(if fixed {
            "fixed-pruned.sqlite3"
        } else {
            "automatic-pruned.sqlite3"
        }),
        identifier,
        &[],
        usize::MAX,
    )?;
    let pruned = Database::open(&output.join(if fixed {
        "fixed-pruned.sqlite3"
    } else {
        "automatic-pruned.sqlite3"
    }))?;
    let raw: String = pruned.connection.query_row(
        "SELECT metadata_json FROM datasets WHERE dataset_key=?1",
        [pruned.dataset],
        |r| r.get(0),
    )?;
    let pruned_metadata: Value = serde_json::from_str(&raw)?;
    let stats = &pruned_metadata["generation"]["localSlopePruning"];
    let payload: u64 = pruned.connection.query_row(
        "SELECT sum(length(payload)) FROM terrain_tiles WHERE dataset_key=?1",
        [pruned.dataset],
        |r| r.get(0),
    )?;
    let raw_final = if smooth_slopes {
        let mut reference_db = Database::open(&output.join("fixed-pruned.sqlite3"))?;
        let mut count = 0u64;
        let (mut total, mut maximum) = (0., 0f64);
        for input in &raw_inputs {
            let points = reference_db.points(input.key)?;
            let local = crate::local_slope::model(&points, input.key)?;
            let surface = Prepared::new(&local);
            let mut work = Work::default();
            let side = WORLD / 2f64.powi(z as i32);
            visit(&input.rasters, input.key, [0., 0., side, side], |q, h| {
                let e = surface.evaluate(q, None, 1., &mut work)? - h;
                count += 1;
                total += e * e;
                maximum = maximum.max(e.abs());
                Ok(true)
            })?;
        }
        Some(json!({"validPixels":count,"rmse":(total/count as f64).sqrt(),"maximumError":maximum}))
    } else {
        None
    };
    let automatic_pruned = json!({"points":stats["pointsAfter"],"payloadBytes":payload,"sqliteBytes":std::fs::metadata(output.join(if fixed{"fixed-pruned.sqlite3"}else{"automatic-pruned.sqlite3"}))?.len(),"rmse":stats["rmse"],"maximumError":stats["maximumError"],"removed":stats["removed"],"validPixels":stats["auditedPixels"]});
    drop(pruned);
    let initial = initial.unwrap();
    let mut report = json!({"mode":if fixed{FIXED_SLOPE_MODE}else{AUTOMATIC_SLOPE_MODE},"supportPolicy":"32m-plus-complete-one-ring","pointOrder":"lexicographic-xyz","windowZ14":window,"z":z,"initialUniquePoints":initial_points,"addedUniquePoints":model.base.points.len()-initial_points,"peakRefinementPasses":peak_passes,"maximumExtraProminenceBeforePruning":peak_maximum,"refinementPasses":passes,"initialAutomaticViolations":initial.failures,"initialAutomaticMaximumError":initial.up.max(initial.down),"baseline":baseline,"stored":stored,"automatic":automatic,"automaticAfterPruning":automatic_pruned});
    if let Some(filtered) = &smoothing {
        report["sourceSmoothing"] = filtered.profile.clone();
        report["rawDemComparisonAfterPruning"] = raw_final.unwrap();
    }
    if fixed {
        let object = report.as_object_mut().unwrap();
        let value = object.remove("automatic").unwrap();
        object.insert("fixed".into(), value);
        let value = object.remove("automaticAfterPruning").unwrap();
        object.insert("fixedAfterPruning".into(), value);
    }
    std::fs::write(
        output.join("comparison.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    std::fs::rename(staging.path(), destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "実DEMと生成済み固定係数SQLiteを環境変数で指定する形状検査"]
    fn real_fixed_surface_has_no_new_sharp_peaks() -> Result<()> {
        let path = std::path::PathBuf::from(std::env::var("YAMAMUKI_FIXED_SHAPE_DB")?);
        let prepared = std::path::PathBuf::from(std::env::var("YAMAMUKI_FIXED_SHAPE_PREPARED")?);
        let job: Value = serde_json::from_slice(&std::fs::read(prepared.join("job.json"))?)?;
        let root = Path::new(job["identity"]["cache"].as_str().unwrap());
        let inventory = Connection::open(prepared.join("inputs.sqlite3"))?;
        let mut db = Database::open(&path)?;
        let raw: String = db.connection.query_row(
            "SELECT metadata_json FROM datasets WHERE dataset_key=?1",
            [db.dataset],
            |r| r.get(0),
        )?;
        let filtered = crate::slope_smoothing::Filtered::from_metadata(
            &prepared,
            &serde_json::from_str(&raw)?,
        )?;
        let mut keys = db.catalog.keys().copied().collect::<Vec<_>>();
        keys.sort_unstable();
        let mut fields = HashMap::new();
        let mut peak_slopes = Vec::new();
        for key in keys {
            ensure!(
                db.fixed_slope_levels.contains(&key.0),
                "固定1のデータを指定してください"
            );
            let points = db.points(key)?;
            let model = crate::local_slope::model(&points, key)?;
            let surface = Prepared::new(&model);
            let side = WORLD / 2f64.powi(key.0 as i32);
            let mut work = Work::default();
            for (i, p) in model.base.points.iter().enumerate() {
                if p[0] < 10.
                    || p[1] < 10.
                    || p[0] > side - 10.
                    || p[1] > side - 10.
                    || model.base.neighbors[i].is_empty()
                    || !model.base.neighbors[i]
                        .iter()
                        .all(|&j| model.base.points[j][2] <= p[2])
                {
                    continue;
                }
                let h = p[2];
                for d in [[1., 0.], [0., 1.], [0.6, 0.8]] {
                    let mut slopes = Vec::new();
                    for r in [1., 0.1, 0.01] {
                        let value = surface.evaluate(
                            [p[0] + d[0] * r, p[1] + d[1] * r],
                            None,
                            1.,
                            &mut work,
                        )?;
                        slopes.push((value - h).abs() / r);
                    }
                    ensure!(
                        slopes[2] <= slopes[0] * 0.25 + 1e-4,
                        "山頂に円錐状の傾きが残ります: {slopes:?}"
                    );
                    peak_slopes.push(slopes[2]);
                }
            }
            for raster in if let Some(filtered) = &filtered {
                filtered.rasters(key)?
            } else {
                read_rasters(root, &inventory, key)?
            } {
                let mut heights = Vec::with_capacity(65536);
                for index in 0..65536 {
                    let q = [
                        (f64::from(raster.x) * 256. + (index % 256) as f64 + 0.5) * STEP
                            - f64::from(key.1) * side,
                        (f64::from(raster.y) * 256. + (index / 256) as f64 + 0.5) * STEP
                            - f64::from(key.2) * side,
                    ];
                    heights.push(if raster.values[index].is_finite() {
                        surface.evaluate(q, None, 1., &mut work)?
                    } else {
                        f64::NAN
                    });
                }
                fields.insert((raster.x as i32, raster.y as i32), (heights, raster.values));
            }
        }
        let mut excess_peaks = 0u64;
        let mut maximum_excess = 0f64;
        let mut compared = 0u64;
        for (&(x, y), (heights, dem)) in &fields {
            for index in 0..65536 {
                let h = heights[index];
                let source = dem[index];
                if !h.is_finite() || !source.is_finite() {
                    continue;
                }
                let gx = x * 256 + (index % 256) as i32;
                let gy = y * 256 + (index / 256) as i32;
                let (mut high, mut source_high, mut valid) =
                    (f64::NEG_INFINITY, f64::NEG_INFINITY, true);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let nx = gx + dx;
                        let ny = gy + dy;
                        if let Some((a, b)) = fields.get(&(nx.div_euclid(256), ny.div_euclid(256)))
                        {
                            let k = (ny.rem_euclid(256) * 256 + nx.rem_euclid(256)) as usize;
                            valid &= a[k].is_finite() && b[k].is_finite();
                            high = high.max(a[k]);
                            source_high = source_high.max(b[k]);
                        } else {
                            valid = false;
                        }
                    }
                }
                if valid {
                    compared += 1;
                    let excess = ((h - high).max(0.) - (source - source_high).max(0.)).max(0.);
                    maximum_excess = maximum_excess.max(excess);
                    excess_peaks += u64::from(excess > 0.5);
                }
            }
        }
        let mut surface_comparison = Value::Null;
        if let Ok(before_path) = std::env::var("YAMAMUKI_SURFACE_COMPARE_BEFORE") {
            let mut previous = Database::open(Path::new(&before_path))?;
            let mut old_fields = HashMap::new();
            let keys = previous.catalog.keys().copied().collect::<Vec<_>>();
            for key in keys {
                let points = previous.points(key)?;
                let model = crate::local_slope::model(&points, key)?;
                let surface = Prepared::new(&model);
                let side = WORLD / 2f64.powi(key.0 as i32);
                let mut work = Work::default();
                for raster in read_rasters(root, &inventory, key)? {
                    let mut heights = Vec::with_capacity(65536);
                    for i in 0..65536 {
                        let q = [
                            (f64::from(raster.x) * 256. + (i % 256) as f64 + 0.5) * STEP
                                - f64::from(key.1) * side,
                            (f64::from(raster.y) * 256. + (i / 256) as f64 + 0.5) * STEP
                                - f64::from(key.2) * side,
                        ];
                        heights.push(if raster.values[i].is_finite() {
                            surface.evaluate(q, None, 1., &mut work)?
                        } else {
                            f64::NAN
                        });
                    }
                    old_fields.insert((raster.x as i32, raster.y as i32), (heights, raster.values));
                }
            }
            let mut count = 0u64;
            let (mut before_rough, mut after_rough) = (0., 0.);
            let get =
                |map: &HashMap<(i32, i32), (Vec<f64>, Vec<f64>)>, x: i32, y: i32| -> Option<f64> {
                    let a = map.get(&(x.div_euclid(256), y.div_euclid(256)))?;
                    let h = a.0[(y.rem_euclid(256) * 256 + x.rem_euclid(256)) as usize];
                    h.is_finite().then_some(h)
                };
            for (&(x, y), (heights, _)) in &fields {
                for i in 0..65536 {
                    let gx = x * 256 + (i % 256) as i32;
                    let gy = y * 256 + (i / 256) as i32;
                    if filtered.as_ref().is_some_and(|f| {
                        f.protects((f64::from(gx) + 0.5) * STEP, (f64::from(gy) + 0.5) * STEP)
                    }) {
                        continue;
                    }
                    let Some(old) = get(&old_fields, gx, gy) else {
                        continue;
                    };
                    let h = heights[i];
                    if !h.is_finite() {
                        continue;
                    }
                    let (mut a, mut b, mut valid) = (4. * old, 4. * h, true);
                    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        if let (Some(old), Some(new)) = (
                            get(&old_fields, gx + dx, gy + dy),
                            get(&fields, gx + dx, gy + dy),
                        ) {
                            a -= old;
                            b -= new;
                        } else {
                            valid = false;
                        }
                    }
                    if valid {
                        count += 1;
                        before_rough += a * a;
                        after_rough += b * b;
                    }
                }
            }
            surface_comparison = json!({"slopePixels":count,"surfaceRoughnessBefore":(before_rough/count.max(1)as f64).sqrt(),"surfaceRoughnessAfter":(after_rough/count.max(1)as f64).sqrt()});
            if let Ok(output) = std::env::var("YAMAMUKI_SURFACE_COMPARE_EXPORT") {
                let directory = std::path::PathBuf::from(output);
                std::fs::create_dir_all(&directory)?;
                let x0 = fields.keys().map(|k| k.0).min().unwrap() * 256;
                let y0 = fields.keys().map(|k| k.1).min().unwrap() * 256;
                let x1 = (fields.keys().map(|k| k.0).max().unwrap() + 1) * 256;
                let y1 = (fields.keys().map(|k| k.1).max().unwrap() + 1) * 256;
                for (name, map, component) in [
                    ("before.f32", &old_fields, 0),
                    ("after.f32", &fields, 0),
                    ("raw-dem.f32", &old_fields, 1),
                    ("smoothed-dem.f32", &fields, 1),
                ] {
                    let mut bytes = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let h = map.get(&(x.div_euclid(256), y.div_euclid(256))).map_or(
                                f64::NAN,
                                |a| {
                                    let i = (y.rem_euclid(256) * 256 + x.rem_euclid(256)) as usize;
                                    if component == 0 { a.0[i] } else { a.1[i] }
                                },
                            );
                            bytes.extend((h as f32).to_le_bytes());
                        }
                    }
                    std::fs::write(directory.join(name), bytes)?;
                }
                std::fs::write(
                    directory.join("grid.json"),
                    serde_json::to_vec_pretty(
                        &json!({"width":x1-x0,"height":y1-y0,"stepProjectedMetres":STEP,"pixelX":x0,"pixelY":y0,"dtype":"float32-le"}),
                    )?,
                )?;
            }
        }
        let report = json!({"surfaceComparison":surface_comparison,"comparedPixels":compared,"extraPeakThresholdMetres":0.5,"newSharpPeaks":excess_peaks,"maximumExtraProminenceMetres":maximum_excess,"testedPeakDirections":peak_slopes.len(),"maximumSlopeAtOneCentimetre":peak_slopes.into_iter().fold(0f64,f64::max)});
        eprintln!("{}", serde_json::to_string_pretty(&report)?);
        if let Ok(output) = std::env::var("YAMAMUKI_FIXED_SHAPE_REPORT") {
            std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
        }
        ensure!(excess_peaks == 0, "元DEMにない鋭いピークが残っています");
        Ok(())
    }
    #[test]
    fn experiment_exports_no_coefficients_and_publishes_only_after_validation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let source_path = dir.path().join("source.sqlite3");
        let stored_path = dir.path().join("stored.sqlite3");
        let mut source = Connection::open(&source_path)?;
        yamamuki_data_sqlite::initialize(&source)?;
        yamamuki_terrain::fixture::create(&mut source)?;
        let dataset: i64 =
            source.query_row("SELECT dataset_key FROM datasets", [], |r| r.get(0))?;
        let (x, y, payload): (u32, u32, Vec<u8>) = source.query_row(
            "SELECT x,y,payload FROM terrain_tiles WHERE level_key=13",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut tile = Tile::decode(&payload)?;
        let n = (WORLD / 8192. * 16.).round() as i32;
        let local = (-2..=4)
            .flat_map(|y| (-2..=4).map(move |x| [x * n / 2, y * n / 2, 800]))
            .collect::<Vec<_>>();
        let points = local
            .iter()
            .map(|p| [p[0] + tile.origin[0], p[1] + tile.origin[1], p[2]])
            .collect::<Vec<_>>();
        tile.count = points.len();
        tile.hash = point_hash(&points);
        tile.controls = natural_codec::independent(&local)?;
        source.execute(
            "UPDATE terrain_tiles SET payload=?1 WHERE level_key=13",
            [tile.encode()?],
        )?;
        let metadata = json!({"fixture":true,"generation":{"id":"auto-test"}});
        source.execute(
            "UPDATE datasets SET metadata_json=?1",
            [metadata.to_string()],
        )?;
        let mut stored = Connection::open(&stored_path)?;
        Backup::new(&source, &mut stored)?.run_to_completion(
            64,
            std::time::Duration::ZERO,
            None,
        )?;
        tile.controls = control_attributes::attach(&tile.controls, &vec![255; points.len()])?;
        stored.execute(
            "UPDATE terrain_tiles SET payload=?1 WHERE level_key=13",
            [tile.encode()?],
        )?;
        drop(stored);
        let prepared = dir.path().join("prepared");
        let cache = dir.path().join("dem");
        std::fs::create_dir_all(&prepared)?;
        std::fs::create_dir_all(cache.join("14").join((x * 2).to_string()))?;
        let mut bytes = Vec::new();
        {
            let mut png = png::Encoder::new(&mut bytes, 256, 256);
            png.set_color(png::ColorType::Rgb);
            png.set_depth(png::BitDepth::Eight);
            png.write_header()?
                .write_image_data(&[0, 39, 16].repeat(65536))?;
        }
        let png_path = cache
            .join("14")
            .join((x * 2).to_string())
            .join(format!("{}.png", y * 2));
        std::fs::write(&png_path, &bytes)?;
        let input = Connection::open(prepared.join("inputs.sqlite3"))?;
        input.execute_batch(
            "CREATE TABLE inputs(x INTEGER,y INTEGER,sha256 TEXT,state TEXT,verified INTEGER)",
        )?;
        input.execute(
            "INSERT INTO inputs VALUES(?1,?2,?3,'cached',1)",
            params![x * 2, y * 2, crate::natural_inventory::hash(&bytes)],
        )?;
        std::fs::write(
            prepared.join("job.json"),
            serde_json::to_vec(
                &json!({"generation_id":"auto-test","inputs_verified":true,"identity":{"cache":cache}}),
            )?,
        )?;
        let output = dir.path().join("experiment");
        let window = [x * 2, y * 2, x * 2 + 2, y * 2 + 2];
        experiment(
            &prepared,
            &source_path,
            &stored_path,
            &output,
            "test-automatic",
            13,
            &window,
            1000,
        )?;
        for name in ["automatic.sqlite3", "automatic-pruned.sqlite3"] {
            let mut db = Database::open(&output.join(name))?;
            assert!(db.automatic_slope_levels.contains(&13));
            assert_eq!(db.catalog.len(), 1);
            let key = *db.catalog.keys().next().unwrap();
            assert!(db.slope_adjustments(key)?.is_none());
            assert!(
                !db.tile(key)?
                    .controls
                    .starts_with(control_attributes::MAGIC)
            );
            db.validate_all()?;
        }
        assert!(output.join("comparison.json").exists());
        let smoothed_output = dir.path().join("smoothed");
        let original_png_hash = crate::natural_inventory::hash_file(&png_path)?;
        experiment_with_processing(
            &prepared,
            &source_path,
            &stored_path,
            &smoothed_output,
            "test-smoothed",
            13,
            &window,
            1000,
            true,
            true,
        )?;
        let mut smoothed = Database::open(&smoothed_output.join("fixed-pruned.sqlite3"))?;
        let raw: String =
            smoothed
                .connection
                .query_row("SELECT metadata_json FROM datasets", [], |r| r.get(0))?;
        let meta: Value = serde_json::from_str(&raw)?;
        assert_eq!(
            meta["generation"]["sourceSmoothing"]["mode"],
            "ridge-preserving-gaussian-v2"
        );
        let filtered = crate::slope_smoothing::Filtered::from_metadata(&prepared, &meta)?.unwrap();
        assert_eq!(filtered.profile["maximumChangeMetres"], 0.);
        smoothed.validate_all()?;
        assert_eq!(
            crate::natural_inventory::hash_file(&png_path)?,
            original_png_hash
        );

        assert!(
            experiment(
                &prepared,
                &source_path,
                &stored_path,
                &output,
                "test",
                13,
                &window,
                1000
            )
            .is_err()
        );
        std::fs::write(png_path, b"changed")?;
        let failed = dir.path().join("failed");
        assert!(
            experiment(
                &prepared,
                &source_path,
                &stored_path,
                &failed,
                "test",
                13,
                &window,
                1000
            )
            .is_err()
        );
        assert!(!failed.exists());
        assert_eq!(
            source.query_row("SELECT dataset_key FROM datasets", [], |r| r
                .get::<_, i64>(0))?,
            dataset
        );
        Ok(())
    }
}
