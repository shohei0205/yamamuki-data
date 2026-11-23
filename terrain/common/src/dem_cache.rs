//! 生成途中の平滑化済みDEMを読み取る。配布用SQLiteとは区別し、原本を変更しない。
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
#[derive(Clone)]
pub struct Header {
    pub identity: Value,
    pub stamp: String,
    pub tiles: BTreeMap<(u32, u32), usize>,
    pub bytes: u64,
}
/// 種別判定だけでは全国索引を走査しない。実読込時に完了状態と本文を検証する。
pub fn is_source(path: &Path) -> Result<bool> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    if !connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='meta')",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(false);
    }
    let raw = connection
        .query_row("SELECT value FROM meta WHERE key='identity'", [], |r| {
            r.get::<_, String>(0)
        })
        .optional()?;
    Ok(raw
        .map(|r| serde_json::from_str::<Value>(&r))
        .transpose()?
        .is_some_and(|v| {
            v["format"]
                .as_str()
                .is_some_and(|f| f.starts_with("smoothed-dem-cache-"))
        }))
}
pub struct Source {
    pub connection: Connection,
    pub header: Header,
}
impl Source {
    pub fn try_open(path: &Path) -> Result<Option<Self>> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        if !connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='meta')",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(None);
        }
        let Some(raw) = connection
            .query_row("SELECT value FROM meta WHERE key='identity'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
        else {
            return Ok(None);
        };
        let identity: Value = serde_json::from_str(&raw)?;
        let Some(format) = identity["format"].as_str() else {
            return Ok(None);
        };
        if !format.starts_with("smoothed-dem-cache-") {
            return Ok(None);
        }
        ensure!(
            format == "smoothed-dem-cache-v1"
                && identity["mode"] == "ridge-preserving-gaussian-v2"
                && identity["pixelEncoding"] == "zstd-5bytes-int32-centimetres-uint8-protected"
                && identity["missingCentimetres"] == i32::MIN,
            "未対応の平滑化DEM作業形式です"
        );
        ensure!(
            connection.query_row("SELECT value FROM meta WHERE key='complete'", [], |r| {
                r.get::<_, String>(0)
            })? == "true",
            "平滑化DEMの準備が未完了です"
        );
        let mut hash = Sha256::new();
        hash.update(raw.as_bytes());
        let mut tiles = BTreeMap::new();
        for row in connection
            .prepare("SELECT x,y,sha256,length(payload) FROM tiles ORDER BY y,x")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, usize>(3)?,
                ))
            })?
        {
            let (x, y, h, n) = row?;
            ensure!(
                x < 16384
                    && y < 16384
                    && h.len() == 64
                    && h.bytes().all(|b| b.is_ascii_hexdigit())
                    && n > 0,
                "平滑化DEMの索引が不正です"
            );
            hash.update(x.to_le_bytes());
            hash.update(y.to_le_bytes());
            hash.update(h.as_bytes());
            tiles.insert((x, y), n);
        }
        ensure!(!tiles.is_empty(), "平滑化DEMがありません");
        let header = Header {
            identity,
            stamp: format!("{:x}", hash.finalize()),
            tiles,
            bytes: std::fs::metadata(path)?.len(),
        };
        Ok(Some(Self { connection, header }))
    }
    pub fn tile(&self, x: u32, y: u32) -> Result<Option<Vec<f32>>> {
        if !self.header.tiles.contains_key(&(x, y)) {
            return Ok(None);
        }
        let (hash, bytes) = self.connection.query_row(
            "SELECT sha256,payload FROM tiles WHERE x=?1 AND y=?2",
            [x, y],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)),
        )?;
        ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == hash,
            "平滑化DEMのSHA-256が一致しません {x}/{y}"
        );
        let raw = zstd::bulk::decompress(&bytes, 65536 * 5).context("平滑化DEMを展開できません")?;
        ensure!(raw.len() == 65536 * 5, "平滑化DEMの画素数が不正です");
        let mut values = Vec::with_capacity(65536);
        for v in raw.chunks_exact(5) {
            let cm = i32::from_le_bytes(v[..4].try_into().unwrap());
            ensure!(v[4] <= 1, "保護マスクが不正です");
            values.push(if cm == i32::MIN {
                f32::NAN
            } else {
                cm as f32 / 100.
            });
        }
        Ok(Some(values))
    }
}
