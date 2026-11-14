use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use terrain_gsi_dem::{natural_inventory, natural_shared_pipeline};

#[derive(Parser)]
#[command(about = "国土地理院DEMから共通点集合の地形データを生成する")]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Prepare {
        #[arg(long)]
        dem14: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Run {
        #[arg(long)]
        prepared: PathBuf,
        #[arg(long, num_args = 4)]
        window: Vec<u32>,
        #[arg(long, default_value_t = 131072)]
        local_point_limit: usize,
    },
    Verify {
        #[arg(long)]
        prepared: PathBuf,
        #[arg(long, default_value_t = 131072)]
        local_point_limit: usize,
    },
    Qualify {
        #[arg(long)]
        prepared: PathBuf,
        #[arg(long, num_args = 3)]
        pilots: Vec<PathBuf>,
    },
}
fn prepare(dem14: &Path, output: &Path) -> Result<()> {
    let dem14 = dem14.canonicalize()?;
    ensure!(
        dem14.file_name().and_then(|v| v.to_str()) == Some("14"),
        "z14 PNGの14ディレクトリを指定してください"
    );
    ensure!(
        !output.exists(),
        "出力先は新規ディレクトリを指定してください"
    );
    let cache = dem14.parent().context("DEMキャッシュの親がありません")?;
    let mut files = Vec::new();
    for entry in fs::read_dir(&dem14)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Ok(x) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        ensure!(x < 16384, "DEMのXが範囲外です");
        for tile in fs::read_dir(entry.path())? {
            let tile = tile?;
            let path = tile.path();
            if path.extension().and_then(|v| v.to_str()) != Some("png") {
                continue;
            }
            let y = path
                .file_stem()
                .and_then(|v| v.to_str())
                .context("DEMファイル名が不正")?
                .parse::<u32>()?;
            ensure!(y < 16384, "DEMのYが範囲外です");
            files.push((x, y, path));
        }
    }
    files.sort_by_key(|(x, y, _)| (*y, *x));
    ensure!(!files.is_empty(), "z14 PNGがありません");
    fs::create_dir_all(output)?;
    let settings: Value = serde_json::from_str(include_str!("../settings.json"))?;
    let settings_bytes = serde_json::to_vec(&settings)?;
    let settings_hash = natural_inventory::hash(&settings_bytes);
    let db = Connection::open(output.join("inputs.sqlite3"))?;
    db.execute_batch("CREATE TABLE inputs(x INTEGER,y INTEGER,state TEXT,sha256 TEXT,bytes INTEGER,verified INTEGER,PRIMARY KEY(x,y)) WITHOUT ROWID;CREATE TABLE planned_tiles(z INTEGER,x INTEGER,y INTEGER,cached_inputs INTEGER,missing_inputs INTEGER,PRIMARY KEY(z,x,y)) WITHOUT ROWID;BEGIN IMMEDIATE")?;
    let mut tiles = BTreeMap::<(u32, u32, u32), usize>::new();
    let mut digest = Sha256::new();
    let mut total = 0u64;
    for (index, (x, y, path)) in files.iter().enumerate() {
        let bytes =
            fs::read(path).with_context(|| format!("DEMを読み込めません: {}", path.display()))?;
        natural_inventory::decode(&bytes)
            .with_context(|| format!("DEMを復号できません: {}", path.display()))?;
        let sha = natural_inventory::hash(&bytes);
        digest.update(x.to_le_bytes());
        digest.update(y.to_le_bytes());
        digest.update(sha.as_bytes());
        db.execute(
            "INSERT INTO inputs VALUES(?1,?2,'cached',?3,?4,1)",
            params![x, y, sha, bytes.len()],
        )?;
        total += bytes.len() as u64;
        for z in 6..=13 {
            *tiles.entry((z, x >> (14 - z), y >> (14 - z))).or_default() += 1;
        }
        if index % 256 == 0 {
            natural_inventory::progress(
                output,
                json!({"stage":"verify-inputs","done":index+1,"total":files.len()}),
            )?;
        }
    }
    for ((z, x, y), count) in tiles {
        db.execute(
            "INSERT INTO planned_tiles VALUES(?1,?2,?3,?4,0)",
            params![z, x, y, count],
        )?;
    }
    db.execute_batch("COMMIT")?;
    let source_hash = format!("{:x}", digest.finalize());
    let generation = natural_inventory::hash(format!("{settings_hash}{source_hash}").as_bytes());
    let job = json!({"format":"national-shared-prepared-v1","generation_id":format!("NS1-{}",&generation[..24]),"identity":{"cache":cache.to_string_lossy(),"source":dem14.to_string_lossy(),"settings_sha256":settings_hash},"input_manifest_sha256":source_hash,"prepared":true,"inputs_verified":true,"ready_to_generate":false,"complete":false,"distributable":false,"settings":settings,"source_bytes":total});
    fs::write(output.join("job.json"), serde_json::to_vec_pretty(&job)?)?;
    fs::write(output.join("settings.json"), settings_bytes)?;
    natural_inventory::progress(
        output,
        json!({"stage":"prepared","done":files.len(),"total":files.len(),"generation_id":job["generation_id"]}),
    )?;
    println!(
        "{} PNGを検査しました。{}",
        files.len(),
        job["generation_id"]
    );
    Ok(())
}
fn qualify(prepared: &Path, pilots: &[PathBuf]) -> Result<()> {
    ensure!(
        pilots.len() == 3,
        "三地点の検証済み作業ディレクトリを指定してください"
    );
    let mut job: Value = serde_json::from_slice(&fs::read(prepared.join("job.json"))?)?;
    let mut reports = Vec::new();
    let engine = natural_inventory::hash_file(&std::env::current_exe()?)?;
    let mut windows = std::collections::HashSet::new();
    for path in pilots {
        let pilot: Value = serde_json::from_slice(&fs::read(path.join("job.json"))?)?;
        let completed = Connection::open_with_flags(
            path.join("terrain-shared.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let complete: String =
            completed.query_row("SELECT value FROM metadata WHERE key='complete'", [], |r| {
                r.get(0)
            })?;
        ensure!(complete == "true", "試験生成が未完了です");
        let raw: String = completed.query_row(
            "SELECT value FROM metadata WHERE key='verification'",
            [],
            |r| r.get(0),
        )?;
        let report: Value = serde_json::from_str(&raw)?;
        ensure!(
            pilot["identity"]["settings_sha256"] == job["identity"]["settings_sha256"]
                && report["integer_roundtrip"]["integer_roundtrip"] == true
                && report["saved_dem"]["all_valid_pixels"] == true,
            "試験データの設定または最終監査が一致しません: {}",
            path.display()
        );
        ensure!(
            report["engine_sha256"].as_str() == Some(&engine)
                && report["support_certified"] == true
                && report["generation_id"] == pilot["generation_id"],
            "試験生成器・支持領域の監査が一致しません"
        );
        let window = report["window"]
            .as_array()
            .context("試験範囲がありません")?;
        ensure!(
            window.len() == 4 && windows.insert(serde_json::to_string(window)?),
            "異なる三地点の試験範囲を指定してください"
        );
        reports.push(report);
    }
    let proof = json!({"ready_to_generate":true,"generation_id":job["generation_id"],"engine_sha256":natural_inventory::hash_file(&std::env::current_exe()?)?,"pilots":reports});
    fs::write(
        prepared.join("ready.json"),
        serde_json::to_vec_pretty(&proof)?,
    )?;
    job["ready_to_generate"] = json!(true);
    fs::write(prepared.join("job.json"), serde_json::to_vec_pretty(&job)?)?;
    Ok(())
}
fn main() -> Result<()> {
    match Args::parse().command {
        Command::Prepare { dem14, output } => prepare(&dem14, &output),
        Command::Run {
            prepared,
            window,
            local_point_limit,
        } => natural_shared_pipeline::execute(&prepared, local_point_limit, "run", window),
        Command::Verify {
            prepared,
            local_point_limit,
        } => natural_shared_pipeline::execute(&prepared, local_point_limit, "verify", vec![]),
        Command::Qualify { prepared, pilots } => qualify(&prepared, &pilots),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepare_decodes_and_fixes_source_identity() {
        let dir = tempfile::tempdir().unwrap();
        let png_dir = dir.path().join("dem/14/14505");
        fs::create_dir_all(&png_dir).unwrap();
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 256, 256);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&vec![0; 256 * 256 * 3])
                .unwrap();
        }
        fs::write(png_dir.join("6465.png"), &bytes).unwrap();
        let output = dir.path().join("prepared");
        prepare(&dir.path().join("dem/14"), &output).unwrap();
        let job: Value =
            serde_json::from_slice(&fs::read(output.join("job.json")).unwrap()).unwrap();
        assert_eq!(job["inputs_verified"], true);
        assert_eq!(job["ready_to_generate"], false);
        let conn = Connection::open(output.join("inputs.sqlite3")).unwrap();
        let sha: String = conn
            .query_row("SELECT sha256 FROM inputs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sha, natural_inventory::hash(&bytes));
        assert_eq!(
            conn.query_row("SELECT count(*) FROM planned_tiles", [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            8
        );
        assert!(prepare(&dir.path().join("dem/14"), &output).is_err());
    }
    #[test]
    fn bad_png_never_becomes_verified_input() {
        let dir = tempfile::tempdir().unwrap();
        let png_dir = dir.path().join("dem/14/14505");
        fs::create_dir_all(&png_dir).unwrap();
        fs::write(png_dir.join("6465.png"), b"invalid").unwrap();
        assert!(prepare(&dir.path().join("dem/14"), &dir.path().join("prepared")).is_err());
    }
}

#[cfg(test)]
mod qualification_tests {
    use super::*;
    #[test]
    fn qualification_requires_three_distinct_reports_from_this_engine() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        let job = json!({"format":"national-shared-prepared-v1","inputs_verified":true,"generation_id":"test-generation","identity":{"settings_sha256":"a".repeat(64)}});
        fs::write(target.join("job.json"), serde_json::to_vec(&job).unwrap()).unwrap();
        let mut pilots = Vec::new();
        let hash = natural_inventory::hash_file(&std::env::current_exe().unwrap()).unwrap();
        for i in 0..3 {
            let path = dir.path().join(format!("pilot-{i}"));
            fs::create_dir(&path).unwrap();
            fs::write(path.join("job.json"), serde_json::to_vec(&job).unwrap()).unwrap();
            let conn = Connection::open(path.join("terrain-shared.sqlite3")).unwrap();
            conn.execute_batch("CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)")
                .unwrap();
            conn.execute("INSERT INTO metadata VALUES('complete','true')", [])
                .unwrap();
            let report = json!({"generation_id":"test-generation","engine_sha256":hash,"integer_roundtrip":{"integer_roundtrip":true},"saved_dem":{"all_valid_pixels":true},"support_certified":true,"window":[i,0,i+1,1]});
            conn.execute(
                "INSERT INTO metadata VALUES('verification',?1)",
                [report.to_string()],
            )
            .unwrap();
            pilots.push(path);
        }
        qualify(&target, &pilots).unwrap();
        assert!(target.join("ready.json").exists());
        assert!(
            qualify(
                &target,
                &[pilots[0].clone(), pilots[0].clone(), pilots[2].clone()]
            )
            .is_err()
        );
        let conn = Connection::open(pilots[0].join("terrain-shared.sqlite3")).unwrap();
        let raw: String = conn
            .query_row(
                "SELECT value FROM metadata WHERE key='verification'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut report: Value = serde_json::from_str(&raw).unwrap();
        report["engine_sha256"] = json!("changed");
        conn.execute(
            "UPDATE metadata SET value=?1 WHERE key='verification'",
            [report.to_string()],
        )
        .unwrap();
        assert!(qualify(&target, &pilots).is_err());
    }
}
