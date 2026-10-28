#[allow(dead_code)]
#[path = "../src/data.rs"]
mod data;
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use std::{collections::BTreeMap, path::Path};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 3,
        "入力全国SQLiteと新規出力SQLiteを指定してください"
    );
    ensure!(
        !Path::new(&args[2]).exists(),
        "既存の試験入力は上書きしません"
    );
    let mut source = data::Database::open(Path::new(&args[1]))?;
    let mut out = Connection::open(&args[2])?;
    out.execute_batch("CREATE TABLE regions(name TEXT PRIMARY KEY,cx REAL,cy REAL,half REAL);CREATE TABLE points(region TEXT,z INTEGER,x INTEGER,y INTEGER,h REAL,PRIMARY KEY(region,z,x,y)) WITHOUT ROWID;")?;
    for (name, lon, lat) in [
        ("fuji", 138.7274, 35.3606),
        ("yari", 137.6476, 36.3419),
        ("tsukuba", 140.1067, 36.2254),
    ] {
        let x = data::WORLD * (lon + 180.) / 360.;
        let y = data::WORLD
            * (1. - f64::asinh(f64::tan(f64::to_radians(lat))) / std::f64::consts::PI)
            / 2.;
        let (cx, cy) = (
            (x / data::UNIT / 64.).round() * 64.,
            (y / data::UNIT / 64.).round() * 64.,
        );
        let half = (32000. / data::UNIT / 64.).ceil() * 64.;
        out.execute(
            "INSERT INTO regions VALUES(?1,?2,?3,?4)",
            params![name, cx, cy, half],
        )?;
        let mut inherited = BTreeMap::new();
        for z in 6..=14 {
            let mut keys: Vec<_> = source
                .catalog
                .keys()
                .filter(|k| {
                    let b = data::bounds(**k);
                    k.0 == z
                        && b[0] <= cx + half
                        && b[2] >= cx - half
                        && b[1] <= cy + half
                        && b[3] >= cy - half
                })
                .copied()
                .collect();
            keys.sort();
            for key in keys {
                for p in source.points(key)?.iter() {
                    if (p[0] - cx).abs() <= half && (p[1] - cy).abs() <= half {
                        inherited.insert((p[0].round() as i64, p[1].round() as i64), p[2]);
                    }
                }
            }
            let tx = out.transaction()?;
            {
                let mut stmt = tx.prepare("INSERT INTO points VALUES(?1,?2,?3,?4,?5)")?;
                for (&(x, y), &h) in &inherited {
                    stmt.execute(params![name, z, x, y, h])?;
                }
            }
            tx.commit()?;
            source.trim();
            println!("{name} z{z}: {} XY", inherited.len());
        }
    }
    Ok(())
}
