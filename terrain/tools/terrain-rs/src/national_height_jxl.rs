//! 全国の標高を JPEG XL にする。中断後は確定済みタイルから再開する。
use crate::{codec, write_json};
use anyhow::{Result, ensure};
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::json;
use std::{
    fs,
    io::Cursor,
    path::Path,
    process::{Command, Stdio},
    time::Instant,
};
const N: usize = 256;
fn open(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}
fn tile(db: &Connection, z: u32, x: u32, y: u32) -> Result<Option<Vec<f32>>> {
    let b: Option<Vec<u8>> = db
        .query_row(
            "SELECT data FROM tiles WHERE z=?1 AND x=?2 AND y=?3",
            (z, x, y),
            |r| r.get(0),
        )
        .optional()?;
    b.map(|b| {
        let g = codec::decode_grid(&b)?;
        ensure!(g.rows == N && g.columns == N, "256 四方の元標高が必要です");
        Ok(g.values)
    })
    .transpose()
}
fn invoke(bin: &Path, args: &[&str]) -> Result<()> {
    let result = Command::new(bin)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    ensure!(
        result.status.success(),
        "JPEG XL 処理失敗: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}
fn encode(bin: &Path, input: &Path, output: &Path, d: &str, gray: bool) -> Result<()> {
    invoke(
        &bin.join("cjxl.exe"),
        &[
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "-d",
            d,
            "-e",
            "7",
            "--num_threads=1",
            "-x",
            if gray {
                "color_space=Gra_D65_Rel_Lin"
            } else {
                "color_space=RGB_D65_SRG_Rel_Lin"
            },
            "--quiet",
        ],
    )
}

struct Packed {
    x: u32,
    y: u32,
    min: f64,
    scale: f64,
    height: Vec<u8>,
    mask: Vec<u8>,
    max: f64,
    squared: f64,
    count: u64,
}
fn quantize(values: &[f32]) -> (f64, f64, Vec<u16>, Vec<u8>) {
    let min = values
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f32::INFINITY, f32::min);
    let min = if min.is_finite() { min as f64 } else { 0. };
    let max = values
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(min as f32, f32::max) as f64;
    let scale = (max - min).max(1.) / 65535.;
    let mut mask = vec![0u8; N * N / 8];
    let q = values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            if v.is_finite() {
                mask[i / 8] |= 1 << (i % 8);
                ((v as f64 - min) / scale).round().clamp(0., 65535.) as u16
            } else {
                0
            }
        })
        .collect();
    (min, scale, q, mask)
}
fn generate(source: &Path, bin: &Path, z: u32, x: u32, y: u32, folder: &Path) -> Result<Packed> {
    let values =
        tile(&open(source)?, z, x, y)?.ok_or_else(|| anyhow::anyhow!("元標高がありません"))?;
    let (min, scale, q, mask) = quantize(&values);
    let prefix = folder.join(format!("{z}-{x}-{y}"));
    let pgm = prefix.with_extension("pgm");
    let hj = prefix.with_extension("jxl");
    let hd = prefix.with_extension("decoded.pgm");
    let mut input = b"P5\n256 256\n65535\n".to_vec();
    for v in q {
        input.extend(v.to_be_bytes());
    }
    fs::write(&pgm, input)?;
    encode(bin, &pgm, &hj, "0.2", true)?;
    invoke(
        &bin.join("djxl.exe"),
        &[
            hj.to_str().unwrap(),
            hd.to_str().unwrap(),
            "--bits_per_sample=16",
            "--color_space=Gra_D65_Rel_Lin",
            "--num_threads=1",
            "--quiet",
        ],
    )?;
    let decoded = crate::bench_jxl::read_pgm(&hd)?;
    ensure!(decoded.len() == N * N, "復号標高の寸法が不正です");
    let mut max = 0f64;
    let mut squared = 0.;
    let mut count = 0;
    for (&v, &q) in values.iter().zip(&decoded) {
        if v.is_finite() {
            let e = (min + q as f64 * scale - v as f64).abs();
            max = max.max(e);
            squared += e * e;
            count += 1;
        }
    }
    let packed = Packed {
        x,
        y,
        min,
        scale,
        height: fs::read(&hj)?,
        mask: zstd::stream::encode_all(Cursor::new(mask), 9)?,
        max,
        squared,
        count,
    };
    for p in [pgm, hj, hd] {
        fs::remove_file(p)?;
    }
    Ok(packed)
}
fn validate_levels(min_level: u32, max_level: u32) -> Result<()> {
    ensure!(
        (7..=14).contains(&min_level) && (7..=14).contains(&max_level) && min_level <= max_level,
        "生成レベルは 7〜14 の昇順で指定してください"
    );
    Ok(())
}
pub fn run(source: &Path, output: &Path, bin: &Path, min_level: u32, max_level: u32) -> Result<()> {
    validate_levels(min_level, max_level)?;
    fs::create_dir_all(output)?;
    let lock = output.join("generation.lock");
    let handle = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)?;
    let result = run_locked(source, output, bin, min_level, max_level);
    if let Err(ref e) = result {
        let _ = write_json(
            &output.join("error.json"),
            &json!({"state":"failed","error":format!("{e:#}")}),
        );
    }
    drop(handle);
    fs::remove_file(lock)?;
    result
}
fn run_locked(source: &Path, output: &Path, bin: &Path, min_level: u32, max_level: u32) -> Result<()> {
    let dest = output.join("terrain-height-jxl.sqlite3");
    ensure!(
        source.canonicalize()? != dest.canonicalize().unwrap_or(dest.clone()),
        "入力と出力が同じです"
    );
    let temp = output.join("work");
    fs::create_dir_all(&temp)?;
    let src = open(source)?;
    let mut db = Connection::open(&dest)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS tiles(z INTEGER,x INTEGER,y INTEGER,height_min REAL,height_scale REAL,height_jxl BLOB,valid_mask_zstd BLOB,max_error REAL,squared_error REAL,valid_count INTEGER,PRIMARY KEY(z,x,y)) WITHOUT ROWID;")?;
    let config=json!({"version":1,"tileSize":256,"minLevel":min_level,"maxLevel":max_level,"heightDistance":0.2,"heightEncoding":"16 bit linear gray; meters = height_min + decoded * height_scale; scale = max(tile range,1)/65535","encoderMode":"auto","effort":7,"normals":false,"renderCoastline":false,"mask":"Zstd; little bit order; 1=valid; missing heights render as zero without outline","source":source.canonicalize()?.to_string_lossy(),"sourceBytes":fs::metadata(source)?.len(),"attribution":"国土地理院"}).to_string();
    let old: Option<String> = db
        .query_row("SELECT value FROM metadata WHERE key='config'", [], |r| {
            r.get(0)
        })
        .optional()?;
    if let Some(old) = old {
        ensure!(old == config, "既存出力の設定が異なります");
    } else {
        db.execute("INSERT INTO metadata VALUES('config',?1)", [&config])?;
    }
    let total: u64 = src.query_row(
        "SELECT count(*) FROM tiles WHERE z BETWEEN ?1 AND ?2",
        (min_level, max_level),
        |r| r.get(0),
    )?;
    let start = Instant::now();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build()?;
    progress(&db, output, total, 0., false)?;
    for z in min_level..=max_level {
        let coords: Vec<(u32, u32)> = src
            .prepare("SELECT x,y FROM tiles WHERE z=? ORDER BY y,x")?
            .query_map([z], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let done: std::collections::HashSet<(u32, u32)> = db
            .prepare("SELECT x,y FROM tiles WHERE z=?")?
            .query_map([z], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let missing: Vec<_> = coords
            .iter()
            .filter(|p| !done.contains(p))
            .copied()
            .collect();
        for batch in missing.chunks(32) {
            let rows: Vec<Result<Packed>> = pool.install(|| {
                batch
                    .par_iter()
                    .map(|&(x, y)| generate(source, bin, z, x, y, &temp))
                    .collect()
            });
            let tx = db.transaction()?;
            for row in rows {
                let p = row?;
                tx.execute(
                    "INSERT INTO tiles VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        z, p.x, p.y, p.min, p.scale, p.height, p.mask, p.max, p.squared, p.count
                    ],
                )?;
            }
            tx.commit()?;
            progress(&db, output, total, start.elapsed().as_secs_f64(), false)?;
            println!("L{z}: {} タイルを確定", batch.len());
        }
        let count: u64 = db.query_row("SELECT count(*) FROM tiles WHERE z=?", [z], |r| r.get(0))?;
        ensure!(count == coords.len() as u64, "レベルの件数が不一致です");
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    }
    let check: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    ensure!(check == "ok", "DB 整合性の確認に失敗しました");
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
    progress(&db, output, total, start.elapsed().as_secs_f64(), true)
}
fn progress(
    db: &Connection,
    output: &Path,
    total: u64,
    seconds: f64,
    complete: bool,
) -> Result<()> {
    let levels:Vec<serde_json::Value>=db.prepare("SELECT z,count(*),sum(length(height_jxl)),sum(length(valid_mask_zstd)),max(max_error),sum(squared_error),sum(valid_count) FROM tiles GROUP BY z ORDER BY z DESC")?.query_map([],|r|{let sq:f64=r.get(5)?;let n:u64=r.get(6)?;Ok(json!({"level":r.get::<_,u32>(0)?,"tiles":r.get::<_,u64>(1)?,"heightBytes":r.get::<_,u64>(2)?,"maskBytes":r.get::<_,u64>(3)?,"maxErrorMeters":r.get::<_,f64>(4)?,"rmseMeters":if n>0{(sq/n as f64).sqrt()}else{0.}}))})?.collect::<rusqlite::Result<_>>()?;
    let count: u64 = levels.iter().map(|v| v["tiles"].as_u64().unwrap()).sum();
    let tmp = output.join("progress.tmp.json");
    write_json(
        &tmp,
        &json!({"state":if complete{"complete"}else{"running"},"completed":count,"total":total,"sessionSeconds":seconds,"heightDistance":0.2,"normals":false,"renderCoastline":false,"levels":levels,"databaseBytes":fs::metadata(output.join("terrain-height-jxl.sqlite3"))?.len()}),
    )?;
    fs::rename(tmp, output.join("progress.json"))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn level_range() {
        assert!(validate_levels(7, 12).is_ok());
        assert!(validate_levels(13, 14).is_ok());
        assert!(validate_levels(14, 14).is_ok());
        assert!(validate_levels(14, 13).is_err());
        assert!(validate_levels(6, 14).is_err());
        assert!(validate_levels(7, 15).is_err());
    }
    #[test]
    fn quantization() {
        let mut v = vec![f32::NAN; N * N];
        v[0] = -10.;
        v[1] = 100.;
        v[2] = 45.;
        let (min, scale, q, m) = quantize(&v);
        assert_eq!(min, -10.);
        assert_eq!(q[0], 0);
        assert_eq!(q[1], 65535);
        assert_eq!(m[0], 7);
        assert!((min + q[2] as f64 * scale - 45.).abs() <= scale / 2.);
    }
    #[test]
    fn empty() {
        let (min, scale, q, m) = quantize(&vec![f32::NAN; N * N]);
        assert_eq!(min, 0.);
        assert!(scale > 0.);
        assert!(q.iter().all(|&v| v == 0));
        assert!(m.iter().all(|&v| v == 0));
    }
}
