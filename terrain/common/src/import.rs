use crate::sqlite::{Database, ENCODING, Tile};
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
    // 生成結果の品質表を使い、旧データへ新しい設定を誤って付けない。
    let raw: String = source
        .query_row("SELECT value FROM metadata WHERE key='levels'", [], |r| {
            r.get(0)
        })
        .context("生成結果の品質表がありません")?;
    let quality: serde_json::Value = serde_json::from_str(&raw)?;
    let quality = quality.as_array().context("品質表は配列が必要です")?;
    ensure!(
        !quality.is_empty() && quality.len() <= 10,
        "品質表はz4〜z13の収録レベルが必要です"
    );
    let mut levels = std::collections::BTreeMap::new();
    for level in quality {
        let z = level["z"].as_u64().context("品質表のzが不正です")?;
        let down = level["error_down_m"]
            .as_f64()
            .context("下向き許容誤差がありません")?;
        let up = level["error_up_m"]
            .as_f64()
            .context("上向き許容誤差がありません")?;
        ensure!(
            (4..=13).contains(&z)
                && down.is_finite()
                && down > 0.
                && up.is_finite()
                && up > 0.
                && levels.insert(z as u32, (down, up)).is_none(),
            "品質表のズームが重複または誤差が不正です"
        );
    }
    let (_, first) = levels.first_key_value().context("品質表が空です")?;
    let ratio = first.1 / first.0;
    let mut metadata = json!({"authors":[{"name":"The yamamuki Authors","role":"配布データの編集・加工","url":"https://github.com/shohei0205/yamamuki-data/blob/main/AUTHORS.md"}],"sources":[{"name":"国土地理院の標高タイル（DEM10B）","source":"https://maps.gsi.go.jp/development/demtile.html","license":"公共データ利用規約（第1.0版） https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html","attribution":"地理院タイル（標高タイル）を加工して作成","changes":"標高を記録された上下の許容誤差へ適合・削減し、自由配置制御点と独立・差分Rice圧縮の地形タイルに加工"}],"generation":{"id":generation,"scope":if national{"national"}else{"window-test"},"interpolation":"sibson","lpf":false,"xyStepMetres":0.0625,"heightStepMetres":0.125}});
    if levels
        .values()
        .all(|(down, up)| (*up / *down - ratio).abs() < 1e-12)
    {
        metadata["generation"]["errorUpRatio"] = json!(ratio);
    }
    if let Some(raw) = source
        .query_row(
            "SELECT value FROM metadata WHERE key='source_window_z14'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        let window: Vec<u32> = serde_json::from_str(&raw)?;
        ensure!(
            window.is_empty()
                || window.len() == 4
                    && window[0] < window[2]
                    && window[1] < window[3]
                    && window[2] <= 16384
                    && window[3] <= 16384,
            "生成対象のz14範囲が不正です"
        );
        metadata["generation"]["sourceWindowZ14"] = json!(window);
    }
    let title = if national {
        "日本全国の地形（国土地理院標高）"
    } else {
        "地域の地形（国土地理院標高）"
    };
    let tx = destination.transaction()?;
    tx.execute("INSERT INTO datasets(id,kind,title,data_version,format_version,metadata_json) VALUES('gsi-terrain-japan','terrain',?3,?1,1,?2)",params![identifier,metadata.to_string(),title])?;
    let dataset = tx.last_insert_rowid();
    for (&z, &(down, up)) in &levels {
        let settings = json!({"encoding":ENCODING,"errorDownMetres":down,"errorUpMetres":up,"anchor":([4,6,9,12].contains(&z)),"interpolation":"sibson","xyStepMetres":0.0625,"heightStepMetres":0.125,"maxParentDepth":2,"maxDependencies":16});
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
            levels.contains_key(&key.0) && key.1 < (1 << key.0) && key.2 < (1 << key.0),
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
