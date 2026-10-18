//! 全国の標高・法線を JPEG XL にする。中断後は確定済みタイルから再開する。
use crate::{bench_png, codec, write_json};
use anyhow::{Result, ensure};
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::json;
use std::{
    collections::HashMap,
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
fn unpack_normal(p: &[u8]) -> [f32; 3] {
    [
        p[0] as f32 / 127.5 - 1.,
        p[1] as f32 / 127.5 - 1.,
        p[2] as f32 / 127.5 - 1.,
    ]
}
fn pack_normal(v: [f32; 3]) -> [u8; 3] {
    let len = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if len < 1e-9 {
        return [0; 3];
    }
    v.map(|x| ((x / len + 1.) * 127.5).round().clamp(0., 255.) as u8)
}
fn highest_normal(db: &Connection, x: u32, y: u32, center: &[f32]) -> Result<Vec<u8>> {
    let mut neighbors = HashMap::new();
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            if dx != 0 || dy != 0 {
                if let (Some(tx), Some(ty)) = (x.checked_add_signed(dx), y.checked_add_signed(dy)) {
                    if let Some(v) = tile(db, 14, tx, ty)? {
                        neighbors.insert((dx, dy), v);
                    }
                }
            }
        }
    }
    let sample = |px: i32, py: i32, fallback: f32| {
        let dx = px.div_euclid(256);
        let dy = py.div_euclid(256);
        let i = py.rem_euclid(256) as usize * N + px.rem_euclid(256) as usize;
        let v = if dx == 0 && dy == 0 {
            center[i]
        } else {
            neighbors.get(&(dx, dy)).map_or(f32::NAN, |t| t[i])
        };
        if v.is_finite() { v } else { fallback }
    };
    let mut out = vec![0; N * N * 3];
    for py in 0..N {
        let lat = (std::f64::consts::PI
            * (1. - 2. * (y as f64 + (py as f64 + 0.5) / 256.) / 16384.))
            .sinh()
            .atan();
        let meters = (2. * std::f64::consts::PI * 6378137. * lat.cos() / (16384. * 256.)) as f32;
        for px in 0..N {
            let i = py * N + px;
            let h = center[i];
            if !h.is_finite() {
                continue;
            }
            let a = px as i32;
            let b = py as i32;
            let dx = (sample(a + 1, b, h) - sample(a - 1, b, h)) / (2. * meters);
            let north = (sample(a, b - 1, h) - sample(a, b + 1, h)) / (2. * meters);
            out[i * 3..i * 3 + 3].copy_from_slice(&pack_normal([-dx, -north, 1.]));
        }
    }
    Ok(out)
}
fn parent_normal(db: &Connection, z: u32, x: u32, y: u32) -> Result<Vec<u8>> {
    let mut out = vec![0; N * N * 3];
    for dy in 0..2 {
        for dx in 0..2 {
            let b: Option<Vec<u8>> = db
                .query_row(
                    "SELECT data FROM normal_work WHERE z=?1 AND x=?2 AND y=?3",
                    (z + 1, x * 2 + dx, y * 2 + dy),
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(b) = b {
                let raw = zstd::stream::decode_all(Cursor::new(b))?;
                ensure!(raw.len() == N * N * 3, "法線作業データの長さが不正です");
                for py in 0..128 {
                    for px in 0..128 {
                        let mut sum = [0.; 3];
                        for sy in 0..2 {
                            for sx in 0..2 {
                                let i = ((py * 2 + sy) * N + px * 2 + sx) * 3;
                                let p = &raw[i..i + 3];
                                if p != [0, 0, 0] {
                                    let v = unpack_normal(p);
                                    for k in 0..3 {
                                        sum[k] += v[k]
                                    }
                                }
                            }
                        }
                        let i = ((dy as usize * 128 + py) * N + dx as usize * 128 + px) * 3;
                        out[i..i + 3].copy_from_slice(&pack_normal(sum));
                    }
                }
            }
        }
    }
    Ok(out)
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
    min: i32,
    height: Vec<u8>,
    mask: Vec<u8>,
    normal: Vec<u8>,
    work: Vec<u8>,
    p95: f64,
    max: f64,
}
fn generate(
    source: &Path,
    dest: &Path,
    bin: &Path,
    z: u32,
    x: u32,
    y: u32,
    folder: &Path,
) -> Result<Packed> {
    let src = open(source)?;
    let values = tile(&src, z, x, y)?.ok_or_else(|| anyhow::anyhow!("元標高がありません"))?;
    let normals = if z == 14 {
        highest_normal(&src, x, y, &values)?
    } else {
        parent_normal(&open(dest)?, z, x, y)?
    };
    let prefix = folder.join(format!("{z}-{x}-{y}"));
    let pgm = prefix.with_extension("pgm");
    let hj = prefix.with_extension("height.jxl");
    let np = prefix.with_extension("png");
    let nj = prefix.with_extension("normal.jxl");
    let hd = prefix.with_extension("decoded.pgm");
    let nd = prefix.with_extension("decoded.png");
    let min = values
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f32::INFINITY, f32::min);
    let min = if min.is_finite() {
        min.floor() as i32
    } else {
        0
    };
    let mut input = format!("P5\n256 256\n65535\n").into_bytes();
    let mut mask = vec![0u8; (N * N) / 8];
    for (i, &v) in values.iter().enumerate() {
        let q = if v.is_finite() {
            ensure!(
                v.fract() == 0. && v - min as f32 <= 65535.,
                "整数標高の範囲外です"
            );
            mask[i / 8] |= 1 << (i % 8);
            (v - min as f32) as u16
        } else {
            0
        };
        input.extend(q.to_be_bytes());
    }
    fs::write(&pgm, &input)?;
    encode(bin, &pgm, &hj, "0", true)?;
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
    ensure!(
        crate::bench_jxl::read_pgm(&hd)?
            == input[format!("P5\n256 256\n65535\n").len()..]
                .chunks_exact(2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        "標高の可逆照合に失敗しました"
    );
    let pixels: Vec<[u8; 4]> = normals
        .chunks_exact(3)
        .map(|p| [p[0], p[1], p[2], 255])
        .collect();
    bench_png::write_encoder_input(&np, N, N, &pixels)?;
    encode(bin, &np, &nj, "0.3", false)?;
    invoke(
        &bin.join("djxl.exe"),
        &[
            nj.to_str().unwrap(),
            nd.to_str().unwrap(),
            "--bits_per_sample=8",
            "--color_space=RGB_D65_SRG_Rel_Lin",
            "--num_threads=1",
            "--quiet",
        ],
    )?;
    let decoded = bench_png::read(&nd)?;
    ensure!(
        decoded.width == N && decoded.height == N,
        "復号法線の寸法が不正です"
    );
    let mut original_valid = Vec::new();
    let mut decoded_valid = Vec::new();
    for (i, p) in pixels.iter().enumerate() {
        if normals[i * 3..i * 3 + 3] != [0, 0, 0] {
            original_valid.push(*p);
            decoded_valid.push(decoded.pixels[i]);
        }
    }
    let angular = if original_valid.is_empty() {
        json!({"p95Degrees":0.,"maxDegrees":0.})
    } else {
        crate::bench_normal::compare_angles(&original_valid, &decoded_valid, true)?
    };
    let packed = Packed {
        x,
        y,
        min,
        height: fs::read(&hj)?,
        mask: zstd::stream::encode_all(Cursor::new(mask), 9)?,
        normal: fs::read(&nj)?,
        work: zstd::stream::encode_all(Cursor::new(normals), 3)?,
        p95: angular["p95Degrees"].as_f64().unwrap(),
        max: angular["maxDegrees"].as_f64().unwrap(),
    };
    for p in [pgm, hj, np, nj, hd, nd] {
        fs::remove_file(p)?;
    }
    Ok(packed)
}
pub fn run(source: &Path, output: &Path, bin: &Path) -> Result<()> {
    fs::create_dir_all(output)?;
    let dest = output.join("terrain-jxl.sqlite3");
    ensure!(
        source.canonicalize()? != dest.canonicalize().unwrap_or(dest.clone()),
        "入力と出力が同じです"
    );
    let lock = output.join("generation.lock");
    let _lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|e| {
            anyhow::anyhow!("生成ロックを取得できません（重複起動または前回の異常終了）: {e}")
        })?;
    let result = run_locked(source, output, bin, &dest);
    if let Err(ref e) = result {
        let mut progress = fs::read(output.join("progress.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .unwrap_or(json!({}));
        progress["state"] = json!("failed");
        progress["error"] = json!(format!("{e:#}"));
        let _ = write_json(&output.join("progress.json"), &progress);
    }
    drop(_lock);
    fs::remove_file(lock)?;
    result
}
fn run_locked(source: &Path, output: &Path, bin: &Path, dest: &Path) -> Result<()> {
    let temp = output.join("work");
    fs::create_dir_all(&temp)?;
    let src = open(source)?;
    let mut db = Connection::open(dest)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS tiles(z INTEGER,x INTEGER,y INTEGER,height_min INTEGER,height_jxl BLOB,valid_mask_zstd BLOB,normal_jxl BLOB,normal_p95 REAL,normal_max REAL,PRIMARY KEY(z,x,y)) WITHOUT ROWID; CREATE TABLE IF NOT EXISTS normal_work(z INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(z,x,y)) WITHOUT ROWID;")?;
    let config=json!({"version":1,"tileSize":256,"levels":[4,14],"height":"16 bit gray JPEG XL lossless; integer meters + height_min","mask":"Zstd; 1=valid; little bit order","normal":"linear RGB signed XYZ; positive up; d=0.3; effort=7","normalPyramid":"L14 neighboring DEM central differences; parent 2x2 normalized average from uncompressed 8 bit XYZ intermediate; invalid excluded","source":source.canonicalize()?.to_string_lossy(),"sourceBytes":fs::metadata(source)?.len(),"attribution":"国土地理院"}).to_string();
    let old: Option<String> = db
        .query_row("SELECT value FROM metadata WHERE key='config'", [], |r| {
            r.get(0)
        })
        .optional()?;
    if let Some(old) = old {
        ensure!(old == config, "既存出力の生成設定が異なります")
    } else {
        db.execute("INSERT INTO metadata VALUES('config',?1)", [&config])?;
    }
    let total: u64 = src.query_row(
        "SELECT COUNT(*) FROM tiles WHERE z BETWEEN 4 AND 14",
        [],
        |r| r.get(0),
    )?;
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build()?;
    let start = Instant::now();
    for z in (4..=14).rev() {
        let coords: Vec<(u32, u32)> = src
            .prepare("SELECT x,y FROM tiles WHERE z=?1 ORDER BY y,x")?
            .query_map([z], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let done: std::collections::HashSet<(u32, u32)> = db
            .prepare("SELECT x,y FROM tiles WHERE z=?1")?
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
                    .map(|&(x, y)| generate(source, dest, bin, z, x, y, &temp))
                    .collect()
            });
            let tx = db.transaction()?;
            for row in rows {
                let p = row?;
                tx.execute(
                    "INSERT INTO tiles VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![z, p.x, p.y, p.min, p.height, p.mask, p.normal, p.p95, p.max],
                )?;
                tx.execute(
                    "INSERT INTO normal_work VALUES(?1,?2,?3,?4)",
                    params![z, p.x, p.y, p.work],
                )?;
            }
            tx.commit()?;
            progress(&db, output, total, z, start.elapsed().as_secs_f64(), false)?;
            let available = fs::metadata(dest)?.len();
            println!("L{z}: batch {} / DB {available} bytes", batch.len());
        }
        let n: u64 = db.query_row("SELECT COUNT(*) FROM tiles WHERE z=?1", [z], |r| r.get(0))?;
        ensure!(n == coords.len() as u64, "レベルの件数が一致しません");
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    }
    let check: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    ensure!(check == "ok", "完成 DB の整合性検査に失敗しました: {check}");
    // 全タイル確定後だけ中間法線を除去し、配布用 DB に空きページを残さない。
    db.execute_batch("DELETE FROM normal_work; PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
    progress(&db, output, total, 4, start.elapsed().as_secs_f64(), true)?;
    Ok(())
}
fn progress(
    db: &Connection,
    output: &Path,
    total: u64,
    z: u32,
    seconds: f64,
    complete: bool,
) -> Result<()> {
    let levels:Vec<serde_json::Value>=db.prepare("SELECT z,count(*),sum(length(height_jxl)),sum(length(normal_jxl)),sum(length(valid_mask_zstd)),max(normal_p95),max(normal_max) FROM tiles GROUP BY z ORDER BY z DESC")?.query_map([],|r|Ok(json!({"level":r.get::<_,u32>(0)?,"tiles":r.get::<_,u64>(1)?,"heightBytes":r.get::<_,u64>(2)?,"normalBytes":r.get::<_,u64>(3)?,"maskBytes":r.get::<_,u64>(4)?,"worstTileP95Degrees":r.get::<_,f64>(5)?,"maxDegrees":r.get::<_,f64>(6)?})))?.collect::<rusqlite::Result<_>>()?;
    let count: u64 = levels.iter().map(|l| l["tiles"].as_u64().unwrap()).sum();
    let temp = output.join("progress.tmp.json");
    write_json(
        &temp,
        &json!({"state":if complete{"complete"}else{"running"},"level":z,"completed":count,"total":total,"sessionSeconds":seconds,"levels":levels,"databaseBytes":fs::metadata(output.join("terrain-jxl.sqlite3"))?.len()}),
    )?;
    fs::rename(temp, output.join("progress.json"))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parent_uses_uncompressed_children() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE normal_work(z INTEGER,x INTEGER,y INTEGER,data BLOB)")
            .unwrap();
        let mut raw = vec![0; N * N * 3];
        for p in raw.chunks_exact_mut(3) {
            p.copy_from_slice(&[128, 128, 255]);
        }
        let b = zstd::stream::encode_all(Cursor::new(raw), 3).unwrap();
        db.execute("INSERT INTO normal_work VALUES(14,2,4,?1)", [b])
            .unwrap();
        let p = parent_normal(&db, 13, 1, 2).unwrap();
        assert_eq!(&p[..3], &[128, 128, 255]);
        assert_eq!(&p[127 * 3..128 * 3], &[128, 128, 255]);
        assert_eq!(&p[128 * 3..129 * 3], &[0, 0, 0]);
        assert_eq!(&p[128 * N * 3..128 * N * 3 + 3], &[0, 0, 0]);
    }
    #[test]
    fn normal_directions() {
        assert_eq!(pack_normal([0., 0., 1.]), [128, 128, 255]);
        assert_eq!(pack_normal([0.; 3]), [0; 3]);
        let p = pack_normal([-1., 0., 1.]);
        assert!(p[0] < 128 && p[2] > 128);
    }
}
