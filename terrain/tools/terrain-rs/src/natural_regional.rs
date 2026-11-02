//! 標準XYZの地域試験。半誤差適合・誤差内削減・全画素監査をRustで行う。
mod natural_codec;
mod natural_regional_gpu;
mod natural_regional_job;
mod natural_regional_surface;
mod natural_surface;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use natural_regional_job::{Inputs, Job, LEVELS, WORLD};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "build/natural-xyz-tight-errors-v1")]
    output: PathBuf,
    #[arg(long, default_value = "build/terrain-cache")]
    cache: PathBuf,
    #[arg(
        long,
        default_value = "build/terrain-national-natural-20261029/coverage-source.sqlite3"
    )]
    coverage: PathBuf,
    #[arg(long)]
    workers: Option<usize>,
    /// 最詳細から粗レベルを作る。対象は128km範囲と接合余白。
    #[arg(long)]
    hierarchy: bool,
    /// 旧設定に加え、z13最詳細の32 m版と全段を見直した25 m版を選ぶ。
    #[arg(long, default_value = "original", value_parser=["original", "z13-32", "z13-25"])]
    level_profile: String,
    #[arg(long)]
    reuse_finest: Option<PathBuf>,
    /// 高くなる側の許容誤差比。1で従来の対称誤差。
    #[arg(long, default_value_t = 1.)]
    upward_error_ratio: f64,
    /// 元DEMの局所値に対して誤差枠を配分する階層生成の試験。
    #[arg(long)]
    hierarchy_local_budget: bool,
    /// 最詳細を最終誤差枠で直接適合する。
    #[arg(long)]
    fine_fit_full_budget: bool,
    #[arg(long)]
    prepare_only: bool,
    /// 検証時は指定した一枚だけを別出力へ生成する。
    #[arg(long, num_args = 3)]
    tile: Vec<u32>,
    /// 補間値のPython参照との照合。
    #[arg(long)]
    evaluate: Option<PathBuf>,
    /// 同じ元データで直接監査と区間監査の処理時間・候補を比較する。
    #[arg(long)]
    benchmark_audit: Option<PathBuf>,
}
fn profile_levels(profile: &str) -> &'static [(u32,f64)] {
    match profile {
        "z13-25" => &[(6,500.),(7,400.),(8,300.),(9,250.),(10,200.),(11,125.),(12,60.),(13,25.)],
        "z13-32" => &[(6,640.),(7,500.),(8,400.),(9,320.),(10,240.),(11,160.),(12,80.),(13,32.)],
        _ => LEVELS,
    }
}
#[cfg(test)]
mod profile_tests {
    use super::*;
    #[test]
    fn agreed_z13_25_profile_is_exact_and_has_no_z14() {
        let levels = profile_levels("z13-25");
        assert_eq!(levels,&[(6,500.),(7,400.),(8,300.),(9,250.),(10,200.),(11,125.),(12,60.),(13,25.)]);
        assert_eq!(levels.len(),8);
        for pair in levels.windows(2) { assert_eq!(pair[0].0+1,pair[1].0);assert!(pair[0].1>pair[1].1); }
        assert_eq!(profile_levels("z13-32").last(),Some(&(13,32.)));
        assert_eq!(profile_levels("original").last(),Some(&(14,20.)));
    }
}
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
pub fn write(path: &Path, value: &Value) -> Result<()> {
    write_bytes(path, &serde_json::to_vec_pretty(value)?)
}
pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().context("保存先に親がありません")?)?;
    let tmp = path.with_extension("rust-next");
    fs::write(&tmp, bytes)?;
    for attempt in 0..40 {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 39 && matches!(e.raw_os_error(), Some(5 | 32 | 33)) => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}
fn connect(root: &Path) -> Result<Connection> {
    let c = Connection::open(root.join("queue.sqlite3"))?;
    c.busy_timeout(Duration::from_secs(60))?;
    c.execute_batch("PRAGMA journal_mode=WAL;")?;
    Ok(c)
}
fn init(root: &Path, only: &[u32]) -> Result<()> {
    fs::create_dir_all(root)?;
    let c = connect(root)?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS jobs(z INT,x INT,y INT,state TEXT,priority REAL,info TEXT,PRIMARY KEY(z,x,y)) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS region_tiles(region TEXT,z INT,x INT,y INT,PRIMARY KEY(region,z,x,y)) WITHOUT ROWID;")?;
    let mut regions = Vec::new();
    let tx = c.unchecked_transaction()?;
    for (region, lat, lon) in [
        ("fuji", 35.3606f64, 138.7274f64),
        ("yari", 36.3419, 137.6476),
        ("tsukuba", 36.2254, 140.1067),
    ] {
        let center = [
            (lon + 180.) / 360. * WORLD,
            (1. - lat.to_radians().tan().asinh() / std::f64::consts::PI) * 0.5 * WORLD,
        ];
        let bounds = [
            center[0] - 64000.,
            center[1] - 64000.,
            center[0] + 64000.,
            center[1] + 64000.,
        ];
        regions.push(json!({"region":region,"center":center,"bounds":bounds}));
        if !only.is_empty() {
            continue;
        }
        for &(z, _) in LEVELS {
            let side = WORLD / (1u64 << z) as f64;
            for y in (bounds[1] / side).floor() as u32..(bounds[3] / side).ceil() as u32 {
                for x in (bounds[0] / side).floor() as u32..(bounds[2] / side).ceil() as u32 {
                    let distance = ((x as f64 * side + side / 2. - center[0]).powi(2)
                        + (y as f64 * side + side / 2. - center[1]).powi(2))
                    .sqrt()
                        / side;
                    let priority = if distance < 1.5 {
                        distance
                    } else {
                        1000. + distance
                    };
                    c.execute(
                        "INSERT OR IGNORE INTO jobs VALUES(?1,?2,?3,'pending',?4,'{}')",
                        params![z, x, y, priority],
                    )?;
                    c.execute(
                        "UPDATE jobs SET priority=min(priority,?1) WHERE z=?2 AND x=?3 AND y=?4",
                        params![priority, z, x, y],
                    )?;
                    c.execute(
                        "INSERT OR IGNORE INTO region_tiles VALUES(?1,?2,?3,?4)",
                        params![region, z, x, y],
                    )?;
                }
            }
        }
    }
    if !only.is_empty() {
        ensure!(
            LEVELS.iter().any(|v| v.0 == only[0]),
            "z6～z14の指定が必要です"
        );
        ensure!(
            only[1] < (1 << only[0]) && only[2] < (1 << only[0]),
            "XYZが範囲外です"
        );
        c.execute(
            "INSERT OR IGNORE INTO jobs VALUES(?1,?2,?3,'pending',0,'{}')",
            params![only[0], only[1], only[2]],
        )?;
    }
    tx.commit()?;
    let new = json!({"regions":regions,"levels":LEVELS.iter().map(|&(z,e)|json!({"z":z,"error_m":e,"side_m":WORLD/(1u64<<z) as f64,"fine_error_m":e/2.})).collect::<Vec<_>>(),"tileScheme":"standard-XYZ-EPSG3857","previewSpan":128000,"lpf":false,"pointBudget":null,"generator":"terrain-natural-regional-rust-v1","productionReady":false,"codec":"NQC1 Rice independent for comparison"});
    if root.join("settings.json").exists() {
        let old: Value = serde_json::from_slice(&fs::read(root.join("settings.json"))?)?;
        ensure!(
            old["lpf"] == new["lpf"]
                && old["levels"]
                    .as_array()
                    .is_some_and(|v| v.len() == LEVELS.len()
                        && v.iter()
                            .zip(LEVELS)
                            .all(|(v, &(z, e))| v["z"].as_u64() == Some(z as u64)
                                && v["error_m"].as_f64() == Some(e)
                                && v["fine_error_m"].as_f64() == Some(e / 2.))),
            "保存済み設定が異なります"
        );
    } else {
        write(&root.join("settings.json"), &new)?;
    }
    write(
        &root.join("rust-generator.json"),
        &json!({"engine":"terrain-natural-regional-rust-v1","started":now(),"levels":LEVELS,"pythonGenerator":false}),
    )?;
    Ok(())
}
fn snapshot(root: &Path) -> Result<Value> {
    let c = connect(root)?;
    let settings: Value = serde_json::from_slice(&fs::read(root.join("settings.json"))?)?;
    let mut counts = serde_json::Map::new();
    let mut levels = Vec::new();
    let mut active = Vec::new();
    let mut failures = Vec::new();
    let mut total = 0;
    let mut s = c.prepare("SELECT state,count(*) FROM jobs GROUP BY state")?;
    for row in s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))? {
        let (state, n) = row?;
        counts.insert(state, json!(n));
        total += n;
    }
    let mut s = c.prepare("SELECT z,state,count(*) FROM jobs GROUP BY z,state")?;
    for row in s.query_map([], |r| {
        Ok((
            r.get::<_, u32>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, u64>(2)?,
        ))
    })? {
        let (z, state, n) = row?;
        levels.push(
            json!({"z":z,"state":state,"count":n,"error":settings["levels"].as_array().unwrap().iter().find(|l|l["z"].as_u64()==Some(z as u64)).unwrap()["error_m"]}),
        );
    }
    let mut s =
        c.prepare("SELECT z,x,y,state,info FROM jobs WHERE state IN ('running','failed')")?;
    for row in s.query_map([], |r| {
        Ok((
            r.get::<_, u32>(0)?,
            r.get::<_, u32>(1)?,
            r.get::<_, u32>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })? {
        let (z, x, y, state, info) = row?;
        if state == "failed" {
            failures.push(json!({"z":z,"x":x,"y":y,"info":serde_json::from_str::<Value>(&info)?}));
        } else {
            let path = Job::folder(root, z, x, y).join("progress.json");
            let p = fs::read(path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .unwrap_or(json!({}));
            active.push(json!({"z":z,"x":x,"y":y,"progress":p}));
        }
    }
    let complete = counts.get("ready").and_then(Value::as_u64) == Some(total);
    Ok(
        json!({"states":counts,"levels":levels,"active":active,"failures":failures,"total":total,"complete":complete,"updated":now(),"engine":"Rust"}),
    )
}
fn worker(root: &Path, inputs: &Arc<Inputs>, group: usize) -> Result<()> {
    loop {
        let c = connect(root)?;
        c.execute_batch("BEGIN IMMEDIATE;")?;
        let hierarchy = root.join("hierarchy.json").exists();
        let filter = if group < 2 { "z<12" } else { "z>=12" };
        let order = if group == 0 { "ASC" } else { "DESC" };
        let sql = if hierarchy {
            "SELECT j.z,j.x,j.y FROM jobs j WHERE j.state='pending' AND NOT EXISTS(SELECT 1 FROM tile_dependencies d JOIN jobs c ON c.z=d.cz AND c.x=d.cx AND c.y=d.cy WHERE d.z=j.z AND d.x=j.x AND d.y=j.y AND c.state!='ready') ORDER BY j.z DESC,j.priority,j.y,j.x LIMIT 1".to_string()
        } else {
            format!(
                "SELECT z,x,y FROM jobs WHERE state='pending' AND {filter} ORDER BY priority,z {order},y,x LIMIT 1"
            )
        };
        let row = c
            .prepare(&sql)?
            .query_map([], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, u32>(2)?,
                ))
            })?
            .next()
            .transpose()?;
        let row = if row.is_some() || hierarchy {
            row
        } else {
            c.prepare(
                "SELECT z,x,y FROM jobs WHERE state='pending' ORDER BY priority,z DESC,y,x LIMIT 1",
            )?
            .query_map([], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, u32>(2)?,
                ))
            })?
            .next()
            .transpose()?
        };
        let Some((z, x, y)) = row else {
            if hierarchy {
                let waiting: u64 =
                    c.query_row("SELECT count(*) FROM jobs WHERE state='pending'", [], |r| {
                        r.get(0)
                    })?;
                let active: u64 =
                    c.query_row("SELECT count(*) FROM jobs WHERE state='running'", [], |r| {
                        r.get(0)
                    })?;
                if waiting > 0 && active > 0 {
                    c.execute_batch("COMMIT")?;
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
                if waiting > 0 {
                    c.execute("UPDATE jobs SET state='failed',info=?1 WHERE state='pending'",[json!({"message":"子タイルが完成していないため階層生成を続行できません"}).to_string()])?;
                    c.execute_batch("COMMIT")?;
                    anyhow::bail!("子タイルの失敗または依存関係を確認してください");
                }
            }
            c.execute_batch("COMMIT")?;
            return Ok(());
        };
        c.execute(
            "UPDATE jobs SET state='running' WHERE z=?1 AND x=?2 AND y=?3",
            params![z, x, y],
        )?;
        c.execute_batch("COMMIT")?;
        let result = Job::new(root, z, x, y, inputs.clone()).and_then(|mut j| j.run());
        let (state, info) = match result {
            Ok(v) => {
                println!("ready z{z}/{x}/{y} {}点", v["finalPoints"]);
                ("ready", v)
            }
            Err(e) => {
                eprintln!("FAILED z{z}/{x}/{y}: {e:#}");
                ("failed", json!({"message":format!("{e:#}")}))
            }
        };
        c.execute(
            "UPDATE jobs SET state=?1,info=?2 WHERE z=?3 AND x=?4 AND y=?5",
            params![state, info.to_string(), z, x, y],
        )?;
    }
}
fn main() -> Result<()> {
    let a = Args::parse();
    if let Some(path) = a.benchmark_audit {
        let input: Value = serde_json::from_slice(&fs::read(path)?)?;
        let inputs = Arc::new(Inputs::load(&a.coverage, &a.cache)?);
        return write(&a.output, &natural_regional_job::benchmark(&input, inputs)?);
    }
    if let Some(path) = a.evaluate {
        let data: Value = serde_json::from_slice(&fs::read(path)?)?;
        let p: Vec<[f64; 3]> = serde_json::from_value(data["points"].clone())?;
        let q: Vec<[f64; 2]> =
            serde_json::from_value(data.get("queries").cloned().unwrap_or(json!([])))?;
        let f = natural_regional_surface::Fast::new(p)?;
        if data["operation"].as_str() == Some("gradient") {
            let bounds: [f64; 4] = serde_json::from_value(data["bounds"].clone())?;
            return write(
                &a.output,
                &json!(natural_regional_gpu::gradient(&f, bounds)?),
            );
        }
        if data["operation"].as_str() == Some("packet") {
            let bounds: [f64; 4] = serde_json::from_value(data["bounds"].clone())?;
            let gradient: Vec<[f64; 2]> = serde_json::from_value(data["gradient"].clone())?;
            let (packet, _) = natural_regional_gpu::packet(&f, &gradient, bounds, data.get("metadata").cloned().unwrap_or(json!({})))?;
            return write_bytes(&a.output, &packet);
        }
        let mut w = natural_regional_surface::Work::default();
        let values = q
            .into_iter()
            .map(|q| f.evaluate(q, &mut w))
            .collect::<Result<Vec<_>>>()?;
        return write(&a.output, &json!(values));
    }
    fs::create_dir_all(&a.output)?;
    let lock = Connection::open(a.output.join("run-lock.sqlite3"))?;
    lock.execute_batch("BEGIN EXCLUSIVE")?;
    if a.hierarchy {
        let levels = profile_levels(&a.level_profile);
        natural_regional_job::prepare_hierarchy_with_options(&a.output, &a.tile, a.reuse_finest.as_deref(), levels, a.upward_error_ratio, a.hierarchy_local_budget, a.fine_fit_full_budget)?;
    } else {
        init(&a.output, &a.tile)?;
    }
    if a.prepare_only {
        write(&a.output.join("progress.json"), &snapshot(&a.output)?)?;
        return Ok(());
    }
    let inputs = Arc::new(Inputs::load(&a.coverage, &a.cache)?);
    let workers = a.workers.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get() / 2)
            .unwrap_or(1)
            .max(1)
    });
    ensure!(workers > 0 && workers <= 128, "並列数が不正です");
    connect(&a.output)?.execute("UPDATE jobs SET state='pending' WHERE state='running'", [])?;
    let (tx, rx) = mpsc::channel();
    let mut threads = Vec::new();
    for group in 0..workers {
        let root = a.output.clone();
        let input = inputs.clone();
        let tx = tx.clone();
        threads.push(std::thread::spawn(move || {
            let r = worker(&root, &input, group).map_err(|e| format!("{e:#}"));
            let _ = tx.send(r);
        }));
    }
    drop(tx);
    let mut remaining = workers;
    let mut error = None;
    while remaining > 0 {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(r) => {
                remaining -= 1;
                if let Err(e) = r {
                    error = Some(e);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
        write(&a.output.join("progress.json"), &snapshot(&a.output)?)?;
    }
    for t in threads {
        ensure!(t.join().is_ok(), "生成ワーカーが異常終了しました");
    }
    if let Some(e) = error {
        anyhow::bail!(e);
    }
    let result = snapshot(&a.output)?;
    write(&a.output.join("progress.json"), &result)?;
    ensure!(
        result["states"]["failed"].as_u64().unwrap_or(0) == 0,
        "失敗したタイルがあります"
    );
    Ok(())
}
