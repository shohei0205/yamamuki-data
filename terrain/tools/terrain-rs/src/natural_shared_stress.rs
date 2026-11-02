//! 点配列を全国常駐させず、実地形の複製点集合でディスク接続の資源を計測する。
#[allow(dead_code)]
mod natural_regional_surface;
mod natural_shared_resources;
#[allow(dead_code)]
mod natural_shared_store;
#[allow(dead_code)]
mod natural_surface;
use anyhow::{Result, ensure};
use clap::Parser;
use rusqlite::params;
use serde_json::json;
use std::{
    fs,
    io::{BufReader, Read},
    path::PathBuf,
    time::Instant,
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value_t = 1)]
    copies_per_axis: usize,
    #[arg(long, default_value_t = 128000.)]
    side: f64,
}
fn q(v: f64, step: f64) -> Result<i32> {
    let n = (v / step + 0.5).floor();
    ensure!(n >= i32::MIN as f64 && n <= i32::MAX as f64, "座標が範囲外");
    Ok(n as i32)
}
fn main() -> Result<()> {
    let a = Args::parse();
    ensure!(
        (1..=4).contains(&a.copies_per_axis) && a.side > 0.,
        "複製範囲が不正"
    );
    fs::create_dir_all(&a.output)?;
    let start = Instant::now();
    let s = natural_shared_store::Store::open(&a.output.join("stress.sqlite3"), 131072)?;
    let size = fs::metadata(&a.input)?.len();
    ensure!(size % 24 == 0, "XYZ64入力が不正");
    let mut hash = sha2::Sha256::new();
    {
        use sha2::Digest;
        let mut f = fs::File::open(&a.input)?;
        let mut b = [0u8; 65536];
        loop {
            let n = f.read(&mut b)?;
            if n == 0 {
                break;
            }
            hash.update(&b[..n]);
        }
    }
    use sha2::Digest;
    let digest = hash
        .finalize()
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect::<String>();
    let identity = json!({"sha256":digest,"copies":a.copies_per_axis,"side":a.side}).to_string();
    use rusqlite::OptionalExtension;
    if let Some(previous) =
        s.db.query_row("SELECT value FROM meta WHERE key='identity'", [], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
    {
        ensure!(previous == identity, "異なる負荷試験を混ぜられません");
    } else {
        s.db.execute("INSERT INTO meta VALUES('identity',?)", [identity])?;
    }
    let world = 40075016.68557849;
    for p in [
        [0., 0., 0.],
        [world, 0., 0.],
        [world, world, 0.],
        [0., world, 0.],
    ] {
        s.insert(13, [q(p[0], 0.0625)?, q(p[1], 0.0625)?, 0], true)?;
    }
    let origin = [10_000_000., 10_000_000.];
    let mut imported = 0u64;
    for cy in 0..a.copies_per_axis {
        for cx in 0..a.copies_per_axis {
            let mut reader = BufReader::new(fs::File::open(&a.input)?);
            let mut records = 0u64;
            while records < size / 24 {
                let tx = s.db.unchecked_transaction()?;
                let batch = (size / 24 - records).min(1024);
                {
                    let mut insert = tx.prepare_cached(
                        "INSERT OR IGNORE INTO points(x,y,h,guard) VALUES(?,?,?,0)",
                    )?;
                    let mut find =
                        tx.prepare_cached("SELECT id,h FROM points WHERE x=? AND y=?")?;
                    let mut index =
                        tx.prepare_cached("INSERT OR IGNORE INTO point_index VALUES(?,?,?,?,?)")?;
                    let mut active =
                        tx.prepare_cached("INSERT OR IGNORE INTO active VALUES(13,?)")?;
                    for _ in 0..batch {
                        let mut raw = [0; 24];
                        reader.read_exact(&mut raw)?;
                        let p: [f64; 3] = std::array::from_fn(|i| {
                            f64::from_le_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap())
                        });
                        records += 1;
                        if p[0] < 0. || p[1] < 0. || p[0] >= a.side || p[1] >= a.side {
                            continue;
                        }
                        let [x, y, h] = [
                            q(origin[0] + cx as f64 * a.side + p[0], 0.0625)?,
                            q(origin[1] + cy as f64 * a.side + p[1], 0.0625)?,
                            q(p[2], 0.125)?,
                        ];
                        insert.execute(params![x, y, h])?;
                        let (id, old): (i64, i32) =
                            find.query_row(params![x, y], |r| Ok((r.get(0)?, r.get(1)?)))?;
                        ensure!(h == old, "量子化後のXYに別標高が重なりました");
                        index.execute(params![id, x, x, y, y])?;
                        active.execute([id])?;
                        imported += 1;
                    }
                }
                tx.commit()?;
                if records % 32768 == 0 {
                    natural_shared_resources::check(&a.output, 100 * 1024u64.pow(3))?;
                    fs::write(
                        a.output.join("progress.json"),
                        serde_json::to_vec_pretty(
                            &json!({"stage":"import","copy":[cx,cy],"records":records,"total_per_copy":size/24,"inserted_this_run":imported,"resources":natural_shared_resources::report()}),
                        )?,
                    )?;
                }
            }
        }
    }
    s.bump(13)?;
    let import_seconds = start.elapsed().as_secs_f64();
    let t = Instant::now();
    let mut samples = 0;
    let mut max_difference = 0f64;
    let mut max_support = 0;
    for &(cx, cy) in &[(0, 0), (a.copies_per_axis - 1, a.copies_per_axis - 1)] {
        let b = [
            origin[0] + cx as f64 * a.side + a.side * 0.4,
            origin[1] + cy as f64 * a.side + a.side * 0.4,
            origin[0] + cx as f64 * a.side + a.side * 0.45,
            origin[1] + cy as f64 * a.side + a.side * 0.45,
        ];
        let first = s.patch(13, b)?;
        let expanded = s.patch(13, [b[0] - 1000., b[1] - 1000., b[2] + 1000., b[3] + 1000.])?;
        max_support = max_support.max(first.ids.len()).max(expanded.ids.len());
        s.persist_patch(13, &format!("{cx}/{cy}"), &first)?;
        let mut aw = natural_regional_surface::Work::default();
        let mut bw = natural_regional_surface::Work::default();
        for y in 0..=8 {
            for x in 0..=8 {
                let q = [
                    b[0] + (b[2] - b[0]) * x as f64 / 8.,
                    b[1] + (b[3] - b[1]) * y as f64 / 8.,
                ];
                max_difference = max_difference
                    .max((first.evaluate(q, &mut aw)? - expanded.evaluate(q, &mut bw)?).abs());
                samples += 1;
            }
        }
    }
    natural_shared_resources::check(&a.output, 100 * 1024u64.pow(3))?;
    ensure!(max_difference < 1e-6, "作業領域で曲面が変化しました");
    let points: i64 =
        s.db.query_row("SELECT count(*) FROM active", [], |r| r.get(0))?;
    s.db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    let report = json!({"kind":"repeated-real-control-points / not new DEM generation","copies_per_axis":a.copies_per_axis,"extent_km":a.side*a.copies_per_axis as f64/1000.,"points":points,"maximum_local_points":max_support,"samples":samples,"maximum_height_difference":max_difference,"import_seconds":import_seconds,"query_seconds":t.elapsed().as_secs_f64(),"sqlite_bytes":fs::metadata(a.output.join("stress.sqlite3"))?.len(),"resources":natural_shared_resources::report(),"complete":true});
    fs::write(
        a.output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
