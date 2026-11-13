use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use surface_tool::write_database;
use yamamuki_data_sqlite as db;

fn surface_level_map<'a>(
    connection: &db::rusqlite::Connection,
    dataset: i64,
    layers: &'a [db::SurfaceLayer],
) -> Result<BTreeMap<&'a str, Vec<u32>>> {
    layers
        .iter()
        .map(|layer| {
            let zooms = db::surface_levels(connection, dataset, layer.key)?
                .into_iter()
                .map(|level| level.z)
                .collect();
            Ok((layer.id.as_str(), zooms))
        })
        .collect()
}

fn manifest(
    path: &Path,
    release_root: &str,
    timestamp: &str,
    release_id: Option<&str>,
) -> Result<()> {
    let mut file = File::open(path)?;
    let connection = db::open(path)?;
    db::validate(&connection)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 65536];
    let mut bytes = 0u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        bytes += count as u64;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("ファイル名が不正です")?;
    let datasets = db::datasets(&connection)?;
    let [dataset] = datasets.as_slice() else {
        anyhow::bail!("配布版を特定できるよう、1データセットのSQLiteを指定してください");
    };
    let version = &dataset.data_version;
    let release_id = release_id.unwrap_or(&dataset.id);
    for value in [version.as_str(), release_id] {
        ensure!(
            value
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
            "データセットIDまたはデータ版が不正です: {value}"
        );
    }
    ensure!(
        file_name.ends_with(".sqlite3") && !file_name.contains(char::is_whitespace),
        "配布ファイル名が不正です"
    );
    let container_version: u32 =
        connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let manifest = serde_json::json!({
        "schemaFormat": "yamamuki-manifest",
        "schemaVersion": 6,
        "dataSchemaFormat": "yamamuki-sqlite",
        "dataSchemaVersion": container_version,
        "name": dataset.title,
        "version": version,
        "fileName": file_name,
        "downloadUrl": format!("{}/{release_id}-{version}/{file_name}", release_root.trim_end_matches('/')),
        "sha256": format!("{:x}", hasher.finalize()),
        "sizeBytes": bytes,
        "uncompressedSizeBytes": bytes,
        "dataTimestamp": timestamp,
        "metadata": dataset.metadata,
        "datasets": datasets,
    });
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        return usage();
    };
    match (command, &args[1..]) {
        ("merge", [output, inputs @ ..]) if !inputs.is_empty() => {
            write_database(Path::new(output), true, |destination| {
                for input in inputs.iter().map(PathBuf::from) {
                    db::merge_from(destination, &db::open(&input)?)
                        .with_context(|| format!("統合元: {}", input.display()))?;
                }
                Ok(())
            })
        }
        ("select-layers", [input, output, ids @ ..]) if !ids.is_empty() => {
            let ids = ids
                .iter()
                .map(|value| {
                    value
                        .to_str()
                        .map(str::to_owned)
                        .context("レイヤーIDが不正です")
                })
                .collect::<Result<Vec<_>>>()?;
            write_database(Path::new(output), true, |destination| {
                db::copy_surface_layers(destination, &db::open(Path::new(input))?, &ids)
            })
        }
        ("select-surface", [input, output, selection]) => {
            let selection: db::SurfaceSelection = serde_json::from_str(
                selection
                    .to_str()
                    .context("レイヤーとズームをJSONで指定してください")?,
            )?;
            write_database(Path::new(output), true, |destination| {
                db::copy_surface_selection(destination, &db::open(Path::new(input))?, &selection)
            })
        }
        ("set-data-version", [input, output, version]) => {
            let version = version.to_str().context("配布識別子が不正です")?;
            ensure!(
                version
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                    && version
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
                "配布識別子が不正です"
            );
            let source = db::open(Path::new(input))?;
            ensure!(
                db::datasets(&source)?.len() == 1,
                "データセット1件のSQLiteを指定してください"
            );
            write_database(Path::new(output), true, |destination| {
                db::merge_from(destination, &source)?;
                destination.execute("UPDATE datasets SET data_version=?1", [version])?;
                Ok(())
            })
        }
        ("update-coverage", [path]) => {
            // 公開済みファイルではなく、配布準備で作った複製に対して呼ぶ。
            db::open(Path::new(path))?;
            let mut connection = db::rusqlite::Connection::open_with_flags(
                Path::new(path),
                db::rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
            )?;
            let tx = connection.transaction()?;
            for dataset in db::datasets(&tx)? {
                if dataset.kind == "surface" {
                    db::recompute_surface_coverage(&tx, dataset.key)?;
                }
            }
            db::validate(&tx)?;
            tx.commit()?;
            Ok(())
        }
        ("surface-levels", [path]) => {
            let connection = db::open(Path::new(path))?;
            let datasets = db::datasets(&connection)?;
            let [dataset] = datasets.as_slice() else {
                anyhow::bail!("地表データセットを1件だけ指定してください");
            };
            ensure!(
                dataset.kind == "surface",
                "地表データセットを指定してください"
            );
            let layers = db::surface_layers(&connection, dataset.key)?;
            let levels = surface_level_map(&connection, dataset.key, &layers)?;
            println!("{}", serde_json::to_string(&levels)?);
            Ok(())
        }
        ("validate", [path]) => {
            let db = db::open(Path::new(path))?;
            db::validate(&db)?;
            let images = db::validate_surface_images(&db)?;
            println!("整合性、参照、地表画像 {images} 枚を確認しました");
            Ok(())
        }
        ("manifest", [path, release_root, timestamp, release_id @ ..]) if release_id.len() <= 1 => {
            manifest(
                Path::new(path),
                release_root
                    .to_str()
                    .filter(|url| url.starts_with("https://"))
                    .context("配布先のHTTPS URLを指定してください")?,
                timestamp
                    .to_str()
                    .context("生成日時はUTCで指定してください")?,
                release_id
                    .first()
                    .map(|value| value.to_str().context("公開IDが不正です"))
                    .transpose()?,
            )
        }
        _ => usage(),
    }
}

fn usage() -> Result<()> {
    anyhow::bail!(
        "使い方: surface-tool merge <出力> <入力>... | select-layers <入力> <出力> <レイヤーID>... | select-surface <入力> <出力> <レイヤー別ズームJSON> | set-data-version <入力> <出力> <配布識別子> | update-coverage <作業用SQLite> | surface-levels <入力> | validate <入力> | manifest <入力> <ReleaseのdownloadルートURL> <生成日時UTC> [公開ID]"
    );
}
