use anyhow::{Context, Result};
use std::path::Path;
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [command,input,output,identifier] if command=="import-shared" => {
            surface_tool::write_database(Path::new(output),true,|db| {
                let count=yamamuki_terrain::import::import_shared(db,Path::new(input),identifier)?;
                println!("地形{count}タイルを復元・照合して収録しました");Ok(())
            })
        }
        [command,input,output,levels] if command=="select-levels" => {
            let levels:Vec<u32>=serde_json::from_str(levels)?;
            surface_tool::write_database(Path::new(output),true,|db|{
                let count=yamamuki_terrain::select::copy_levels(db,Path::new(input),&levels)?;
                println!("地形{count}タイルを選択し、全XYZ・親依存を確認しました");Ok(())
            })
        }
        [command,output] if command=="fixture" => surface_tool::write_database(Path::new(output),true,yamamuki_terrain::fixture::create),
        [command,input] if command=="validate" => {
            let mut db=yamamuki_terrain::sqlite::Database::open(Path::new(input))?;
            yamamuki_data_sqlite::validate(&db.connection)?;
            println!("地形{}タイルの全XYZと親依存を確認しました",db.validate_all()?);Ok(())
        }
        _=>anyhow::bail!("使い方: terrain-tool import-shared <入力共有曲面SQLite> <新規出力SQLite> <日時識別子> | validate <SQLite> | fixture <新規出力SQLite> | select-levels <入力> <新規出力> <ズーム配列JSON>"),
    }.context("地形データの処理に失敗しました")
}
