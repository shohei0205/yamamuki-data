use crate::sqlite::{Database, ENCODING, LEVELS, Tile};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::json;
use std::path::Path;

pub fn import_shared(destination: &mut Connection, path: &Path, identifier: &str) -> Result<usize> {
    ensure!(
        !identifier.is_empty()
            && identifier
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
        "配布識別子が不正です"
    );
    let source = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let complete: String =
        source.query_row("SELECT value FROM metadata WHERE key='complete'", [], |r| {
            r.get(0)
        })?;
    ensure!(
        complete == "true",
        "生成・検証が完了した共有曲面SQLiteを指定してください"
    );
    let generation: String = source.query_row(
        "SELECT value FROM metadata WHERE key='generation_id'",
        [],
        |r| r.get(0),
    )?;
    let generation = serde_json::from_str::<String>(&generation).unwrap_or(generation);
    let national = source
        .query_row(
            "SELECT value FROM metadata WHERE key='national_complete'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .is_some_and(|v| v == "true");
    let metadata = json!({"authors":[{"name":"The yamamuki Authors","role":"配布データの編集・加工","url":"https://github.com/shohei0205/yamamuki-data/blob/main/AUTHORS.md"}],"sources":[{"name":"国土地理院の標高タイル（DEM10B）","source":"https://maps.gsi.go.jp/development/demtile.html","license":"公共データ利用規約（第1.0版） https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html","attribution":"地理院タイル（標高タイル）を加工して作成","changes":"標高を方向別誤差で適合・削減し、自由配置制御点と独立・差分Rice圧縮の地形タイルに加工"}],"generation":{"id":generation,"scope":if national{"national"}else{"window-test"},"interpolation":"sibson","lpf":false,"errorUpRatio":0.25,"xyStepMetres":0.0625,"heightStepMetres":0.125}});
    let tx = destination.transaction()?;
    tx.execute("INSERT INTO datasets(id,kind,title,data_version,format_version,metadata_json) VALUES('gsi-terrain-japan','terrain','日本全国の地形（国土地理院標高）',?1,1,?2)",params![identifier,metadata.to_string()])?;
    let dataset = tx.last_insert_rowid();
    for (z, error) in LEVELS {
        let settings = json!({"errorDownMetres":error,"errorUpMetres":error/4.,"anchor":([6,9,12].contains(&z)),"interpolation":"sibson","xyStepMetres":0.0625,"heightStepMetres":0.125,"maxParentDepth":2,"maxDependencies":16});
        tx.execute(
            "INSERT INTO terrain_levels VALUES(?1,?2,?2,?3)",
            params![dataset, z, settings.to_string()],
        )?;
    }
    let mut count = 0;
    let mut stmt = source.prepare(
        "SELECT z,x,y,origin_x,origin_y,points,xyz_sha256,payload FROM tiles ORDER BY z,x,y",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let key = (
            row.get::<_, u32>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, u32>(2)?,
        );
        ensure!(
            (6..=13).contains(&key.0) && key.1 < (1 << key.0) && key.2 < (1 << key.0),
            "地形タイル座標が不正です"
        );
        let mut query=source.prepare("SELECT slot,parent_z,parent_x,parent_y FROM parents WHERE z=?1 AND x=?2 AND y=?3 ORDER BY slot")?;
        let values = query
            .query_map(params![key.0, key.1, key.2], |r| {
                Ok((
                    r.get::<_, usize>(0)?,
                    (
                        r.get::<_, u32>(1)?,
                        r.get::<_, u32>(2)?,
                        r.get::<_, u32>(3)?,
                    ),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            values.iter().enumerate().all(|(i, v)| i == v.0),
            "親スロットが連続していません"
        );
        let hash: String = row.get(6)?;
        ensure!(hash.len() == 64 && hash.is_ascii(), "XYZハッシュが不正です");
        let hash = hash
            .as_bytes()
            .chunks(2)
            .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let tile = Tile {
            origin: [row.get(3)?, row.get(4)?],
            count: row.get(5)?,
            parents: values.into_iter().map(|v| v.1).collect(),
            hash: hash.try_into().unwrap(),
            controls: row.get(7)?,
        };
        tx.execute(
            "INSERT INTO terrain_tiles VALUES(?1,?2,?3,?4,?5,?6)",
            params![dataset, key.0, key.1, key.2, ENCODING, tile.encode()?],
        )?;
        count += 1;
    }
    ensure!(count > 0, "地形タイルがありません");
    tx.commit()?;
    let path = destination
        .path()
        .context("出力先SQLiteのパスがありません")?;
    let mut restored = Database::open(Path::new(path))?;
    restored.validate_all()?;
    Ok(count)
}
