//! 地域試験と同じ平滑化を全国へ分割適用する。原DEMと配布SQLiteは変更しない。
use crate::{
    natural_inventory,
    slope_smoothing::{Config, Filtered},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

fn encode(height: f64, protected: bool, bytes: &mut Vec<u8>) {
    let cm = if height.is_finite() {
        (height * 100.).round() as i32
    } else {
        i32::MIN
    };
    bytes.extend(cm.to_le_bytes());
    bytes.push(u8::from(protected));
}
/// 8×8のz14窓と周囲1タイルを読み、一つのトランザクションで確定する。
/// 同じ原本・設定・実行バイナリのみ再開し、完了済み本文もハッシュ検査する。
pub fn prepare(prepared: &Path, output: &Path) -> Result<()> {
    let job: Value = serde_json::from_slice(&std::fs::read(prepared.join("job.json"))?)?;
    ensure!(
        job["inputs_verified"] == true,
        "原本の検証が完了していません"
    );
    let inventory = Connection::open_with_flags(
        prepared.join("inputs.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut groups = BTreeMap::<(u32, u32), Vec<(u32, u32)>>::new();
    let mut stmt = inventory
        .prepare("SELECT x,y FROM inputs WHERE verified=1 AND state='cached' ORDER BY y,x")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))? {
        let (x, y) = row?;
        groups.entry((x / 8, y / 8)).or_default().push((x, y));
    }
    ensure!(!groups.is_empty(), "検証済みDEMがありません");
    std::fs::create_dir_all(output)?;
    let mut db = Connection::open(output.join("smoothed-dem.work.sqlite3"))?;
    db.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=FULL;CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS tiles(x INTEGER,y INTEGER,sha256 TEXT NOT NULL,payload BLOB NOT NULL,PRIMARY KEY(x,y)) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS chunks(x INTEGER,y INTEGER,profile_json TEXT NOT NULL,PRIMARY KEY(x,y)) WITHOUT ROWID;")?;
    let identity = json!({"format":"smoothed-dem-cache-v1","mode":"ridge-preserving-gaussian-v2","config":Config::default(),"sourceGenerationId":job["generation_id"],"rawInputManifestSha256":job["input_manifest_sha256"],"engineSha256":natural_inventory::hash_file(&std::env::current_exe()?)?,"chunkTiles":8,"haloTiles":1,"pixelEncoding":"zstd-5bytes-int32-centimetres-uint8-protected","missingCentimetres":i32::MIN});
    let old = db.query_row("SELECT value FROM meta WHERE key='identity'", [], |r| {
        r.get::<_, String>(0)
    });
    match old {
        Ok(raw) => ensure!(
            serde_json::from_str::<Value>(&raw)? == identity,
            "原本・平滑化設定・生成器が異なるため再開できません"
        ),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            db.execute(
                "INSERT INTO meta VALUES('identity',?1)",
                [identity.to_string()],
            )?;
        }
        Err(e) => return Err(e.into()),
    }
    let total = groups.len();
    let expected = groups.values().map(Vec::len).sum::<usize>();
    for (index, (&(cx, cy), coords)) in groups.iter().enumerate() {
        let complete: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM chunks WHERE x=?1 AND y=?2)",
            params![cx, cy],
            |r| r.get(0),
        )?;
        if complete {
            for &(x, y) in coords {
                let (hash, bytes) = db.query_row(
                    "SELECT sha256,payload FROM tiles WHERE x=?1 AND y=?2",
                    params![x, y],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)),
                )?;
                ensure!(
                    natural_inventory::hash(&bytes) == hash,
                    "再開対象の平滑化本文が破損しています"
                );
            }
            continue;
        }
        let window = [
            cx * 8,
            cy * 8,
            (cx * 8 + 8).min(16384),
            (cy * 8 + 8).min(16384),
        ];
        let filtered = Filtered::load(prepared, &window, Config::default())
            .with_context(|| format!("平滑化窓 {window:?}"))?;
        let tx = db.transaction()?;
        for &(x, y) in coords {
            let mut raw = Vec::with_capacity(65536 * 5);
            for yy in 0..256 {
                for xx in 0..256 {
                    let (h, p) =
                        filtered.cached_pixel((x * 256 + xx) as i32, (y * 256 + yy) as i32);
                    encode(h, p, &mut raw);
                }
            }
            let bytes = zstd::bulk::compress(&raw, 3)?;
            tx.execute(
                "INSERT INTO tiles VALUES(?1,?2,?3,?4)",
                params![x, y, natural_inventory::hash(&bytes), bytes],
            )?;
        }
        tx.execute(
            "INSERT INTO chunks VALUES(?1,?2,?3)",
            params![cx, cy, filtered.profile.to_string()],
        )?;
        tx.commit()?;
        natural_inventory::progress(
            output,
            json!({"stage":"smooth-national-dem","done":index+1,"total":total,"tilesInChunk":coords.len(),"expectedTiles":expected,"complete":false}),
        )?;
        eprintln!("全国斜面平滑化: {}/{total} 窓", index + 1);
    }
    let count: usize = db.query_row("SELECT count(*) FROM tiles", [], |r| r.get(0))?;
    ensure!(count == expected, "平滑化済みDEMの件数が一致しません");
    db.execute("INSERT OR REPLACE INTO meta VALUES('complete','true')", [])?;
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    natural_inventory::progress(
        output,
        json!({"stage":"smooth-national-dem","done":total,"total":total,"tiles":count,"complete":true,"distributable":false}),
    )?;
    Ok(())
}

/// 完成した平滑化キャッシュを読み、本文を毎回ハッシュ検査する。
pub fn read_tile(path: &Path, x: i32, y: i32) -> Result<(Vec<f64>, Vec<u8>)> {
    use std::cell::RefCell;
    thread_local! { static READER: RefCell<Option<(std::path::PathBuf,Connection)>> = const { RefCell::new(None) }; }
    READER.with(|cell| {
        let mut reader = cell.borrow_mut();
        if reader.as_ref().is_none_or(|(p, _)| p != path) {
            let c = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            ensure!(
                c.query_row("SELECT value FROM meta WHERE key='complete'", [], |r| {
                    r.get::<_, String>(0)
                })? == "true",
                "全国平滑化キャッシュが未完成です"
            );
            *reader = Some((path.to_owned(), c));
        }
        let db = &reader.as_ref().unwrap().1;
        let (hash, bytes) = db.query_row(
            "SELECT sha256,payload FROM tiles WHERE x=?1 AND y=?2",
            params![x, y],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)),
        )?;
        ensure!(
            natural_inventory::hash(&bytes) == hash,
            "平滑化DEMの本文が破損しています {x}/{y}"
        );
        let raw = zstd::bulk::decompress(&bytes, 65536 * 5)?;
        ensure!(raw.len() == 65536 * 5, "平滑化DEMの画素数が不正です");
        let mut values = Vec::with_capacity(65536);
        let mut protection = Vec::with_capacity(65536);
        for v in raw.chunks_exact(5) {
            let cm = i32::from_le_bytes(v[..4].try_into().unwrap());
            ensure!(v[4] <= 1, "地形保護マスクが不正です");
            values.push(if cm == i32::MIN {
                f64::NAN
            } else {
                f64::from(cm) / 100.
            });
            protection.push(v[4]);
        }
        Ok((values, protection))
    })
}
/// 別世代の作業領域を作り、従来の全国版と混在させない。
pub fn prepare_fixed(
    prepared: &Path,
    smoothed: &Path,
    seed_database: &Path,
    output: &Path,
) -> Result<()> {
    ensure!(
        !output.exists(),
        "固定曲面の準備先は新規ディレクトリを指定してください"
    );
    let original: Value = serde_json::from_slice(&std::fs::read(prepared.join("job.json"))?)?;
    let seed_database = seed_database.canonicalize()?;
    let seed = yamamuki_terrain::sqlite::Database::open(&seed_database)?;
    let seed_meta: Value = serde_json::from_str(&seed.connection.query_row(
        "SELECT metadata_json FROM datasets WHERE dataset_key=?1",
        [seed.dataset],
        |r| r.get::<_, String>(0),
    )?)?;
    ensure!(
        seed_meta["generation"]["id"] == original["generation_id"],
        "初期点用のSQLiteが原DEMと異なる世代です"
    );
    let cache = smoothed.canonicalize()?;
    let c = Connection::open_with_flags(&cache, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    ensure!(
        c.query_row("SELECT value FROM meta WHERE key='complete'", [], |r| {
            r.get::<_, String>(0)
        })? == "true",
        "平滑化が未完了です"
    );
    let raw: String = c.query_row("SELECT value FROM meta WHERE key='identity'", [], |r| {
        r.get(0)
    })?;
    let identity: Value = serde_json::from_str(&raw)?;
    ensure!(
        identity["sourceGenerationId"] == original["generation_id"]
            && identity["rawInputManifestSha256"] == original["input_manifest_sha256"]
            && identity["config"] == serde_json::to_value(Config::default())?,
        "平滑化キャッシュの原本・設定が一致しません"
    );
    let input = Connection::open_with_flags(
        prepared.join("inputs.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let count: u64 = input.query_row(
        "SELECT count(*) FROM inputs WHERE verified=1 AND state='cached'",
        [],
        |r| r.get(0),
    )?;
    ensure!(
        c.query_row("SELECT count(*) FROM tiles", [], |r| r.get::<_, u64>(0))? == count,
        "全国の平滑化件数が一致しません"
    );
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    for row in c
        .prepare("SELECT x,y,sha256 FROM tiles ORDER BY y,x")?
        .query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
    {
        let (x, y, h) = row?;
        digest.update(x.to_le_bytes());
        digest.update(y.to_le_bytes());
        digest.update(h.as_bytes());
    }
    let target_hash = format!("{:x}", digest.finalize());
    let mut job = original.clone();
    job["settings"]["profile"] = json!("local-hermite-smoothed-z6-z13-v1");
    job["settings"]["codec"]["max_points_per_tile"] = Value::Null;
    job["settings"]["generation"]["slope_mode"] = json!("local-hermite-v1");
    job["settings"]["generation"]["error_evaluation_multiplier"] = json!(1);
    job["settings"]["generation"]["source_smoothing"] = identity["config"].clone();
    job["source_smoothing"] = json!({"mode":"ridge-preserving-gaussian-v2","config":identity["config"],"chunkTiles":8,"haloTiles":1,"sourceGenerationId":original["generation_id"],"rawInputManifestSha256":original["input_manifest_sha256"],"targetTileInventorySha256":target_hash,"targetQuantizationMetres":0.01,"tiles":count});
    job["identity"]["seed_database"] = json!(seed_database.to_string_lossy());
    job["identity"]["seed_database_sha256"] = json!(natural_inventory::hash_file(&seed_database)?);
    job["identity"]["raw_cache"] = original["identity"]["cache"].clone();
    job["identity"]["cache"] = json!(cache.to_string_lossy());
    job["identity"]["settings_sha256"] = json!(natural_inventory::hash(&serde_json::to_vec(
        &job["settings"]
    )?));
    job["generation_id"] = json!(format!(
        "NF1-{}",
        &natural_inventory::hash(&serde_json::to_vec(
            &json!({"settings":job["settings"],"sourceSmoothing":job["source_smoothing"]})
        )?)[..24]
    ));
    job["ready_to_generate"] = json!(false);
    job["complete"] = json!(false);
    job["distributable"] = json!(false);
    std::fs::create_dir_all(output)?;
    let mut dest = Connection::open(output.join("inputs.sqlite3"))?;
    rusqlite::backup::Backup::new(&input, &mut dest)?.run_to_completion(
        256,
        std::time::Duration::ZERO,
        None,
    )?;
    std::fs::write(output.join("job.json"), serde_json::to_vec_pretty(&job)?)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_preserves_centimetres_missing_and_protection() {
        let mut bytes = Vec::new();
        encode(3774.04, true, &mut bytes);
        encode(-2.13, false, &mut bytes);
        encode(f64::NAN, false, &mut bytes);
        let compressed = zstd::bulk::compress(&bytes, 3).unwrap();
        let restored = zstd::bulk::decompress(&compressed, 15).unwrap();
        assert_eq!(bytes, restored);
        assert_eq!(
            i32::from_le_bytes(restored[0..4].try_into().unwrap()),
            377404
        );
        assert_eq!(restored[4], 1);
        assert_eq!(i32::from_le_bytes(restored[5..9].try_into().unwrap()), -213);
        assert_eq!(
            i32::from_le_bytes(restored[10..14].try_into().unwrap()),
            i32::MIN
        );
    }
}
