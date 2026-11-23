//! 元DEMで局所傾き係数を学習し、共有点の係数を統一して全画素で監査する。
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, backup::Backup, params};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};
use yamamuki_terrain::{
    control_attributes,
    local_slope::{Fit, Prepared},
    natural_regional_surface::{Fast, Work},
    sqlite::{Database, Key, WORLD},
};
const STEP: f64 = WORLD / (16384. * 256.);
struct Dem {
    root: PathBuf,
    db: Connection,
    cache: HashMap<(u32, u32), Arc<Vec<f64>>>,
    order: VecDeque<(u32, u32)>,
}
impl Dem {
    fn inputs(&self, key: Key) -> Result<Vec<(u32, u32, String)>> {
        ensure!(key.0 <= 14, "z14より精細な入力は未対応です");
        let width = 1u32 << (14 - key.0);
        let mut stmt=self.db.prepare("SELECT x,y,sha256 FROM inputs WHERE state='cached' AND verified=1 AND x>=?1 AND x<?2 AND y>=?3 AND y<?4 ORDER BY y,x")?;
        Ok(stmt
            .query_map(
                params![
                    key.1 * width,
                    (key.1 + 1) * width,
                    key.2 * width,
                    (key.2 + 1) * width
                ],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    fn read(&mut self, x: u32, y: u32, sha: &str) -> Result<Arc<Vec<f64>>> {
        if let Some(v) = self.cache.get(&(x, y)) {
            return Ok(v.clone());
        }
        let path = self
            .root
            .join("14")
            .join(x.to_string())
            .join(format!("{y}.png"));
        let bytes = std::fs::read(&path)
            .with_context(|| format!("元DEMを読めません: {}", path.display()))?;
        ensure!(
            crate::natural_inventory::hash(&bytes) == sha,
            "固定した元DEMのSHA-256が変化しました"
        );
        let data = Arc::new(crate::natural_inventory::decode(&bytes)?);
        if self.order.len() >= 64 {
            let k = self.order.pop_front().unwrap();
            self.cache.remove(&k);
        }
        self.order.push_back((x, y));
        self.cache.insert((x, y), data.clone());
        Ok(data)
    }
}
pub(super) fn fully_inside(key: Key, window: &[u32]) -> bool {
    if window.is_empty() {
        return true;
    }
    if key.0 > 14 {
        return false;
    }
    let width = 1u32 << (14 - key.0);
    key.1 * width >= window[0]
        && (key.1 + 1) * width <= window[2]
        && key.2 * width >= window[1]
        && (key.2 + 1) * width <= window[3]
}
pub(super) fn model(points: &[[i32; 3]], key: Key) -> Result<Fast> {
    let side = WORLD / 2f64.powi(key.0 as i32);
    Fast::certified(
        points
            .iter()
            .map(|p| {
                [
                    f64::from(p[0]) / 16. - f64::from(key.1) * side,
                    f64::from(p[1]) / 16. - f64::from(key.2) * side,
                    f64::from(p[2]) / 8.,
                ]
            })
            .collect(),
    )
}
pub(super) fn errors(source: &Database, z: u32) -> Result<(f64, f64)> {
    let raw: String = source.connection.query_row(
        "SELECT settings_json FROM terrain_levels WHERE dataset_key=?1 AND z=?2",
        params![source.dataset, z],
        |r| r.get(0),
    )?;
    let settings: Value = serde_json::from_str(&raw)?;
    Ok((
        settings["errorDownMetres"]
            .as_f64()
            .context("下方向誤差がありません")?,
        settings["errorUpMetres"]
            .as_f64()
            .context("上方向誤差がありません")?,
    ))
}
fn samples(
    dem: &mut Dem,
    key: Key,
    mut visit: impl FnMut([f64; 2], f64) -> Result<()>,
) -> Result<u64> {
    let side = WORLD / 2f64.powi(key.0 as i32);
    let mut valid = 0;
    for (x, y, sha) in dem.inputs(key)? {
        let raster = dem.read(x, y, &sha)?;
        for (index, &h) in raster.iter().enumerate() {
            if !h.is_finite() {
                continue;
            }
            let q = [
                (f64::from(x) * 256. + (index % 256) as f64 + 0.5) * STEP - f64::from(key.1) * side,
                (f64::from(y) * 256. + (index / 256) as f64 + 0.5) * STEP - f64::from(key.2) * side,
            ];
            visit(q, h)?;
            valid += 1;
        }
    }
    Ok(valid)
}
/// 完成済みXYZへ係数を付ける。出力は検証完了まで一時ファイルに置く。
pub fn generate(
    prepared: &Path,
    input: &Path,
    output: &Path,
    identifier: &str,
    window: &[u32],
) -> Result<()> {
    ensure!(!output.exists(), "出力先は新規ファイルを指定してください");
    ensure!(!identifier.is_empty(), "配布識別子が空です");
    ensure!(
        window.is_empty()
            || (window.len() == 4
                && window[0] < window[2]
                && window[1] < window[3]
                && window[2] <= 16384
                && window[3] <= 16384),
        "z14窓は左上・右下の半開区間です"
    );
    let job: Value = serde_json::from_slice(&std::fs::read(prepared.join("job.json"))?)?;
    let mut source = Database::open(input)?;
    let metadata: String = source.connection.query_row(
        "SELECT metadata_json FROM datasets WHERE dataset_key=?1",
        [source.dataset],
        |r| r.get(0),
    )?;
    let mut metadata: Value = serde_json::from_str(&metadata)?;
    let generation = job["generation_id"]
        .as_str()
        .filter(|v| !v.is_empty())
        .context("元DEMの準備世代がありません")?;
    ensure!(
        metadata["generation"]["id"].as_str() == Some(generation) && job["inputs_verified"] == true,
        "元DEMの準備世代が入力SQLiteと一致しません"
    );
    let root = job["identity"]["cache"]
        .as_str()
        .context("元DEMキャッシュの場所がありません")?;
    let mut dem = Dem {
        root: root.into(),
        db: Connection::open_with_flags(
            prepared.join("inputs.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?,
        cache: HashMap::new(),
        order: VecDeque::new(),
    };
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
    target.execute_batch("CREATE TEMP TABLE slope_fit(z INTEGER,x INTEGER,y INTEGER,h INTEGER,n REAL,d REAL,lo REAL,cap REAL,PRIMARY KEY(z,x,y,h)) WITHOUT ROWID;")?;
    let mut keys = source.catalog.keys().copied().collect::<Vec<_>>();
    keys.sort_unstable();
    let mut trained = 0u64;
    let mut trained_tiles = 0u64;
    // 同じz・XYZを共有するタイルの統計を合算し、最も厳しい上限を採用する。
    {
        let tx = target.transaction()?;
        for &key in keys.iter().filter(|k| fully_inside(**k, window)) {
            let points = source.points(key)?;
            let m = model(&points, key)?;
            let prepared = Prepared::new(&m);
            let mut fit = Fit::new(points.len());
            let mut work = Work::default();
            let (down, up) = errors(&source, key.0)?;
            let valid = samples(&mut dem, key, |q, h| {
                fit.observe(&prepared, q, h, down, up, &mut work)
            })?;
            if valid == 0 {
                continue;
            }
            trained += valid;
            trained_tiles += 1;
            let mut insert=tx.prepare_cached("INSERT INTO slope_fit VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(z,x,y,h) DO UPDATE SET n=n+excluded.n,d=d+excluded.d,lo=max(lo,excluded.lo),cap=min(cap,excluded.cap)")?;
            for (i, p) in points.iter().enumerate() {
                insert.execute(params![
                    key.0,
                    p[0],
                    p[1],
                    p[2],
                    fit.numerator[i],
                    fit.denominator[i],
                    fit.floor[i],
                    fit.cap[i]
                ])?;
            }
            eprintln!("局所傾き学習 z{} {}/{} valid={valid}", key.0, key.1, key.2);
        }
        tx.commit()?;
    }
    ensure!(
        trained > 0,
        "全体を含むタイルまたは有効DEM画素がありません。z13では2×2以上のz14窓を指定してください"
    );
    // 窓の外に影響する共有点は0へ戻す。未監査領域で新しい変形を生じさせない。
    if !window.is_empty() {
        let tx = target.transaction()?;
        for &key in keys.iter().filter(|k| !fully_inside(**k, window)) {
            let points = source.points(key)?;
            let mut update = tx.prepare_cached(
                "UPDATE slope_fit SET cap=0 WHERE z=?1 AND x=?2 AND y=?3 AND h=?4",
            )?;
            for p in points.iter() {
                update.execute(params![key.0, p[0], p[1], p[2]])?;
            }
        }
        tx.commit()?;
    }
    let mut final_max = 0f64;
    let mut initial_squared = 0.;
    let mut final_squared = 0.;
    let mut audited = 0u64;
    let mut zero = 0u64;
    let mut full = 0u64;
    let mut local = 0u64;
    {
        let tx = target.transaction()?;
        for &key in &keys {
            let points = source.points(key)?;
            let mut amounts = Vec::with_capacity(points.len());
            {
                let mut query = tx.prepare_cached(
                    "SELECT n,d,lo,cap FROM slope_fit WHERE z=?1 AND x=?2 AND y=?3 AND h=?4",
                )?;
                for p in points.iter() {
                    let fit: Option<(f64, f64, f64, f64)> = query
                        .query_row(params![key.0, p[0], p[1], p[2]], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                        })
                        .optional()?;
                    let a = match fit {
                        Some((n, d, lo, cap)) => Fit::quantize(n, d, lo, cap)?,
                        None => 0,
                    };
                    amounts.push(a);
                    zero += u64::from(a == 0);
                    full += u64::from(a == 255);
                    local += u64::from(a > 0 && a < 255);
                }
            }
            if fully_inside(key, window) {
                let m = model(&points, key)?;
                let prepared = Prepared::new(&m);
                let mut work = Work::default();
                let (down, up) = errors(&source, key.0)?;
                audited += samples(&mut dem, key, |q, h| {
                    let basis = m.basis_borrowed(q, &mut work)?;
                    let pair = prepared.pair(q, basis, Some(&amounts))?;
                    let h1 = pair.value(1.);
                    let b = (h1 - h).abs();
                    ensure!(
                        h - h1 <= down + 1e-7 && h1 - h <= up + 1e-7,
                        "局所傾きの全画素誤差が許容値を超えています"
                    );
                    initial_squared += (pair.original - h).powi(2);
                    final_squared += (h1 - h).powi(2);
                    final_max = final_max.max(b);
                    Ok(())
                })?;
            } else {
                ensure!(
                    amounts.iter().all(|v| *v == 0),
                    "未監査領域に局所変形が残っています"
                );
            }
            let mut tile = source.tile(key)?;
            ensure!(
                !tile.controls.starts_with(control_attributes::MAGIC),
                "局所傾き付き入力の再学習は未対応です"
            );
            tile.controls = control_attributes::attach(&tile.controls, &amounts)?;
            tx.execute("UPDATE terrain_tiles SET payload=?1 WHERE dataset_key=?2 AND level_key=(SELECT level_key FROM terrain_levels WHERE dataset_key=?2 AND z=?3) AND x=?4 AND y=?5",params![tile.encode()?,source.dataset,key.0,key.1,key.2])?;
        }
        ensure!(
            audited == trained,
            "局所傾きの学習・監査範囲または二乗誤差が不一致です"
        );
        metadata["generation"]["localSlopeAdjustment"] = json!({"format":"NPA1","bits":8,"fitting":"per-control-sibson-weighted-least-squares","certifiedMultiplier":1,"trainedTiles":trained_tiles,"validPixels":audited,"rmseBefore":(initial_squared/audited as f64).sqrt(),"rmseAfter":(final_squared/audited as f64).sqrt(),"maximumError":final_max,"zero":zero,"full":full,"partial":local,"xyzUnchanged":true,"sourceDataVersion":source.generation,"engineSha256":crate::natural_inventory::hash_file(&std::env::current_exe()?)?});
        if !window.is_empty() {
            metadata["generation"]["scope"] = json!("window-test");
            metadata["generation"]["localSlopeWindowZ14"] = json!(window);
        }
        tx.execute(
            "UPDATE datasets SET data_version=?1,metadata_json=?2 WHERE dataset_key=?3",
            params![identifier, metadata.to_string(), source.dataset],
        )?;
        tx.commit()?;
    }
    target.execute_batch("VACUUM;PRAGMA optimize;")?;
    yamamuki_data_sqlite::validate(&target)?;
    drop(target);
    let mut check = Database::open(&temporary)?;
    check.validate_all()?;
    for &key in &keys {
        ensure!(
            check.points(key)? == source.points(key)?,
            "係数付加で整数XYZ・順序が変化しました"
        );
    }
    drop(check);
    temporary.persist_noclobber(output)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&metadata["generation"]["localSlopeAdjustment"])?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_dem_fit_is_atomic_preserves_xyz_and_rejects_changed_inputs() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("source.sqlite3");
        let output = dir.path().join("local.sqlite3");
        let mut connection = Connection::open(&source)?;
        yamamuki_data_sqlite::initialize(&connection)?;
        yamamuki_terrain::fixture::create(&mut connection)?;
        connection.execute(
            "UPDATE datasets SET metadata_json=?1",
            [json!({"fixture":true,"generation":{"id":"fit-fixture"}}).to_string()],
        )?;
        connection.execute("UPDATE terrain_levels SET settings_json=?1",[json!({"encoding":"natural-controls-v1","errorDownMetres":2500.,"errorUpMetres":2000.}).to_string()])?;
        let (x, y): (u32, u32) = connection.query_row(
            "SELECT x,y FROM terrain_tiles WHERE level_key=13",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let root = dir.path().join("cache");
        let prepared = dir.path().join("prepared");
        std::fs::create_dir_all(&prepared)?;
        std::fs::create_dir_all(root.join("14").join((x * 2).to_string()))?;
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 256, 256);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()?
                .write_image_data(&vec![0; 256 * 256 * 3])?;
        }
        let input = root
            .join("14")
            .join((x * 2).to_string())
            .join(format!("{}.png", y * 2));
        std::fs::write(&input, &bytes)?;
        let db = Connection::open(prepared.join("inputs.sqlite3"))?;
        db.execute_batch(
            "CREATE TABLE inputs(x INTEGER,y INTEGER,sha256 TEXT,state TEXT,verified INTEGER)",
        )?;
        db.execute(
            "INSERT INTO inputs VALUES(?1,?2,?3,'cached',1)",
            params![x * 2, y * 2, crate::natural_inventory::hash(&bytes)],
        )?;
        std::fs::write(
            prepared.join("job.json"),
            serde_json::to_vec(
                &json!({"generation_id":"fit-fixture","inputs_verified":true,"identity":{"cache":root}}),
            )?,
        )?;
        generate(&prepared, &source, &output, "20261123T000000Z-test", &[])?;
        let mut before = Database::open(&source)?;
        let mut after = Database::open(&output)?;
        assert_eq!(after.generation, "20261123T000000Z-test");
        for key in before.catalog.clone().keys() {
            assert_eq!(before.points(*key)?, after.points(*key)?);
            assert!(after.slope_adjustments(*key)?.is_some());
        }
        let raw: String =
            after
                .connection
                .query_row("SELECT metadata_json FROM datasets", [], |r| r.get(0))?;
        let metadata: Value = serde_json::from_str(&raw)?;
        assert_eq!(
            metadata["generation"]["localSlopeAdjustment"]["validPixels"],
            8 * 65536
        );
        assert_eq!(
            metadata["generation"]["localSlopeAdjustment"]["certifiedMultiplier"],
            1
        );
        let reduced = dir.path().join("reduced.sqlite3");
        crate::local_slope_prune::generate(
            &prepared,
            &output,
            &reduced,
            "20261123T000000Z-pruned",
            &[],
            8,
        )?;
        let mut reduced = Database::open(&reduced)?;
        for key in before.catalog.clone().keys() {
            assert_eq!(before.points(*key)?, reduced.points(*key)?);
        }

        assert!(generate(&prepared, &source, &output, "other", &[]).is_err());
        std::fs::write(input, b"changed")?;
        let bad = dir.path().join("bad.sqlite3");
        assert!(generate(&prepared, &source, &bad, "other", &[]).is_err());
        assert!(!bad.exists());
        Ok(())
    }
}
