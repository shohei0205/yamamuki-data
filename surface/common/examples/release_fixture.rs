//! 地表Releaseの模擬テストに使う、3レイヤー・全8レベルの小さなSQLiteを作る。
use anyhow::{Result, ensure};
use std::{io::Cursor, path::PathBuf};
use yamamuki_data_sqlite::{
    self as db,
    rusqlite::{Connection, params},
};

fn main() -> Result<()> {
    let output = PathBuf::from(std::env::args_os().nth(1).expect("出力先SQLite"));
    ensure!(!output.exists(), "新しい出力先を指定してください");
    let mut conn = Connection::open(output)?;
    db::initialize(&conn)?;
    let tx = conn.transaction()?;
    tx.execute_batch(r#"
        INSERT INTO datasets VALUES
          (1,'esa-worldcover-japan','surface','地表公開テスト','20261112T000000Z-12345-1',1,
           '{"sources":[{"name":"テスト","source":"架空の画像","license":"CC0-1.0","attribution":"公開テスト","changes":"テスト用に作成"}]}');
        INSERT INTO surface_layers VALUES
          (1,1,'forest','mask',0,'["テスト"]','{"color":[111,148,88,255]}'),
          (1,2,'hillshade','hillshade',10,'["テスト"]','{"neutral":136,"strength":0.65}'),
          (1,3,'water','mask',20,'["テスト"]','{"color":[114,172,204,255]}');
    "#)?;
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_luma8(1, 1).write_to(&mut png, image::ImageFormat::Png)?;
    for layer in 1..=3 {
        for z in 5..=12 {
            tx.execute(
                "INSERT INTO surface_levels VALUES(1,?1,?2,?3,1)",
                params![layer, z, format!("z{z}")],
            )?;
            tx.execute(
                "INSERT INTO surface_tiles VALUES(1,?1,?2,0,0,1,?3)",
                params![layer, z, png.get_ref()],
            )?;
        }
    }
    tx.commit()?;
    db::validate(&conn)?;
    db::validate_surface_images(&conn)?;
    Ok(())
}
