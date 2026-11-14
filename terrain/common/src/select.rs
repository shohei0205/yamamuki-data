//! 復号結果を変えずに収録ズームを選び、差分の親参照が収録内で完結することを確認する。
use crate::sqlite::{Database, ENCODING};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use std::{collections::BTreeSet, path::Path};

pub fn copy_levels(destination: &mut Connection, input: &Path, zooms: &[u32]) -> Result<usize> {
    let selected = zooms.iter().copied().collect::<BTreeSet<_>>();
    ensure!(
        !selected.is_empty() && selected.len() == zooms.len() && selected.iter().all(|z| *z <= 30),
        "収録ズームが空・重複・範囲外です"
    );
    let source = Database::open(input)?;
    let metadata:(String,String,String,i64,String)=source.connection.query_row("SELECT id,title,data_version,format_version,metadata_json FROM datasets WHERE dataset_key=?1",[source.dataset],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    let mut query = source.connection.prepare(
        "SELECT level_key,z,settings_json FROM terrain_levels WHERE dataset_key=?1 ORDER BY z",
    )?;
    let levels = query
        .query_map([source.dataset], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(
        selected.iter().all(|z| levels.iter().any(|l| l.1 == *z)),
        "元データにないズームが指定されています"
    );
    let tx = destination.transaction()?;
    tx.execute("INSERT INTO datasets(id,kind,title,data_version,format_version,metadata_json) VALUES(?1,'terrain',?2,?3,?4,?5)",params![metadata.0,metadata.1,metadata.2,metadata.3,metadata.4])?;
    let dataset = tx.last_insert_rowid();
    let mut count = 0;
    for (level, z, settings) in levels {
        if !selected.contains(&z) {
            continue;
        }
        tx.execute(
            "INSERT INTO terrain_levels VALUES(?1,?2,?3,?4)",
            params![dataset, level, z, settings],
        )?;
        let mut query=source.connection.prepare("SELECT x,y,encoding,payload FROM terrain_tiles WHERE dataset_key=?1 AND level_key=?2 ORDER BY x,y")?;
        let mut rows = query.query(params![source.dataset, level])?;
        while let Some(row) = rows.next()? {
            let x: u32 = row.get(0)?;
            let y: u32 = row.get(1)?;
            let encoding: String = row.get(2)?;
            let payload: Vec<u8> = row.get(3)?;
            ensure!(encoding == ENCODING, "未対応の地形形式です");
            let tile = crate::sqlite::Tile::decode(&payload)?;
            for parent in tile.parents {
                ensure!(
                    selected.contains(&parent.0) && source.catalog.contains_key(&parent),
                    "分割後に親タイルが欠けます: {:?} -> {:?}",
                    (z, x, y),
                    parent
                );
            }
            tx.execute(
                "INSERT INTO terrain_tiles VALUES(?1,?2,?3,?4,?5,?6)",
                params![dataset, level, x, y, encoding, payload],
            )?;
            count += 1;
        }
    }
    ensure!(count > 0, "選択範囲に地形タイルがありません");
    tx.commit()?;
    let path = destination.path().context("出力先のパスがありません")?;
    Database::open(Path::new(path))?.validate_all()?;
    Ok(count)
}
