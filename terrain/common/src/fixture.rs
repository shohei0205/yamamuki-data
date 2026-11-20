//! 外部データ不要の復号・描画検査用データ。配布には使わない。
use crate::{
    natural_codec,
    sqlite::{ENCODING, LEVELS, Tile, WORLD, point_hash},
};
use anyhow::Result;
use rusqlite::{Connection, params};
pub fn create(db: &mut Connection) -> Result<()> {
    create_levels(db, &LEVELS)
}
pub fn create_with_coarse(db: &mut Connection) -> Result<()> {
    let levels = crate::sqlite::COARSE_LEVELS
        .into_iter()
        .chain(LEVELS)
        .collect::<Vec<_>>();
    create_levels(db, &levels)
}
fn create_levels(db: &mut Connection, levels: &[(u32, f64)]) -> Result<()> {
    let tx = db.transaction()?;
    tx.execute("INSERT INTO datasets(id,kind,title,data_version,format_version,metadata_json) VALUES('terrain-fixture','terrain','地形描画テスト','20000101T000000Z',1,'{\"fixture\":true}')",[])?;
    let dataset = tx.last_insert_rowid();
    let lon = 138.7274;
    let lat = 35.3606f64;
    let center = [
        (lon + 180.) / 360. * WORLD,
        (1. - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2. * WORLD,
    ];
    for &(z, error) in levels {
        tx.execute(
            "INSERT INTO terrain_levels VALUES(?1,?2,?2,?3)",
            params![
                dataset,
                z,
                serde_json::json!({"errorDownMetres":error,"errorUpMetres":error}).to_string()
            ],
        )?;
        let side = WORLD / 2f64.powi(z as i32);
        let x = (center[0] / side).floor() as u32;
        let y = (center[1] / side).floor() as u32;
        let origin = [
            (x as f64 * side * 16.).round() as i32,
            (y as f64 * side * 16.).round() as i32,
        ];
        let n = (side * 16.).ceil() as i32;
        let local = vec![
            [0, 0, 0],
            [n, 0, 0],
            [n, n, 0],
            [0, n, 0],
            [n / 2, n / 2, 8000],
        ];
        let global = local
            .iter()
            .map(|p| [p[0] + origin[0], p[1] + origin[1], p[2]])
            .collect::<Vec<_>>();
        let tile = Tile {
            origin,
            count: local.len(),
            parents: vec![],
            hash: point_hash(&global),
            controls: natural_codec::independent(&local)?,
        };
        tx.execute(
            "INSERT INTO terrain_tiles VALUES(?1,?2,?3,?4,?5,?6)",
            params![dataset, z, x, y, ENCODING, tile.encode()?],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// 読み込み・混在LODの検査用。通常fixtureのz13だけを独立した格子へ置き換える。
pub fn create_mixed(db: &mut Connection) -> Result<()> {
    create(db)?;
    db.execute("UPDATE datasets SET format_version=2", [])?;
    let levels = db
        .prepare("SELECT level_key,settings_json FROM terrain_levels")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (z, raw) in levels {
        let mut settings: serde_json::Value = serde_json::from_str(&raw)?;
        settings["encoding"] = serde_json::json!(if z == 13 {
            crate::heightmap::ENCODING
        } else {
            ENCODING
        });
        if z == 13 {
            settings["cellsPerTile"] = serde_json::json!(8);
            settings["halo"] = serde_json::json!(1);
            settings["heightStepMetres"] = serde_json::json!(0.125);
            settings["interpolation"] = serde_json::json!("bilinear");
        }
        db.execute(
            "UPDATE terrain_levels SET settings_json=?2 WHERE level_key=?1",
            params![z, settings.to_string()],
        )?;
    }
    let heights = (0..121)
        .map(|i| Some(100. + (i % 11) as f64 * 10. + (i / 11) as f64 * 5.))
        .collect::<Vec<_>>();
    let map = crate::heightmap::Heightmap::quantize(8, 0.125, &heights)?;
    db.execute(
        "UPDATE terrain_tiles SET encoding=?1,payload=?2 WHERE level_key=13",
        params![crate::heightmap::ENCODING, map.encode()?],
    )?;
    Ok(())
}
