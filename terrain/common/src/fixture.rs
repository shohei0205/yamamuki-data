//! 外部データ不要の復号・描画検査用データ。配布には使わない。
use crate::{
    natural_codec,
    sqlite::{ENCODING, LEVELS, Tile, WORLD, point_hash},
};
use anyhow::Result;
use rusqlite::{Connection, params};
pub fn create(db: &mut Connection) -> Result<()> {
    let tx = db.transaction()?;
    tx.execute("INSERT INTO datasets(id,kind,title,data_version,format_version,metadata_json) VALUES('terrain-fixture','terrain','地形描画テスト','20000101T000000Z',1,'{\"fixture\":true}')",[])?;
    let dataset = tx.last_insert_rowid();
    let lon = 138.7274;
    let lat = 35.3606f64;
    let center = [
        (lon + 180.) / 360. * WORLD,
        (1. - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2. * WORLD,
    ];
    for (z, error) in LEVELS {
        tx.execute(
            "INSERT INTO terrain_levels VALUES(?1,?2,?2,?3)",
            params![
                dataset,
                z,
                serde_json::json!({"errorDownMetres":error,"errorUpMetres":error/4.}).to_string()
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
