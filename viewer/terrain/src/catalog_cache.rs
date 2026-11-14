//! SQLite本体を書き換えず、閲覧用の全タイル索引だけを再利用する。
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};
use yamamuki_terrain::sqlite::{Database, Key};
fn stamp(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!(
        "{}:{}:{}",
        path.canonicalize().ok()?.display(),
        metadata.len(),
        modified
    ))
}
fn root() -> Option<PathBuf> {
    if let Some(path) =
        std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("XDG_CACHE_HOME"))
    {
        return Some(PathBuf::from(path).join("yamamuki-viewer/terrain-index-v1"));
    }
    std::env::var_os("HOME").map(|p| {
        PathBuf::from(p).join(if cfg!(target_os = "macos") {
            "Library/Caches/yamamuki-viewer/terrain-index-v1"
        } else {
            ".cache/yamamuki-viewer/terrain-index-v1"
        })
    })
}
pub fn load(db: &mut Database, path: &Path) -> anyhow::Result<()> {
    let Some(signature) = stamp(path) else {
        return db.load_catalog();
    };
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    signature.hash(&mut hash);
    let Some(cache) = root().map(|p| p.join(format!("{:016x}.json", hash.finish()))) else {
        return db.load_catalog();
    };
    if read(db, &cache, &signature) {
        return Ok(());
    }
    db.load_catalog()?;
    let _ = save(db, &cache, &signature);
    Ok(())
}
fn read(db: &mut Database, cache: &Path, signature: &str) -> bool {
    let result = (|| -> Option<()> {
        if std::fs::metadata(cache).ok()?.len() > 16 * 1024 * 1024 {
            return None;
        }
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(cache).ok()?).ok()?;
        if value["version"] != 1
            || value["signature"] != signature
            || value["generation"] != db.generation
        {
            return None;
        }
        let rows: Vec<(Key, usize)> = serde_json::from_value(value["tiles"].clone()).ok()?;
        let len = rows.len();
        let catalog = rows.into_iter().collect::<HashMap<_, _>>();
        if catalog.len() != len {
            return None;
        }
        db.use_catalog(catalog).ok()?;
        Some(())
    })();
    result.is_some()
}
fn save(db: &Database, path: &Path, signature: &str) -> anyhow::Result<()> {
    let directory = path.parent().unwrap();
    std::fs::create_dir_all(directory)?;
    let rows = db
        .catalog
        .iter()
        .map(|(key, count)| (*key, *count))
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(
        &serde_json::json!({"version":1,"signature":signature,"generation":db.generation,"tiles":rows}),
    )?;
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    std::fs::write(&temporary, bytes)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn index_cache_reuses_valid_data_and_rejects_stale_or_broken_data() -> anyhow::Result<()> {
        let folder = std::env::temp_dir().join(format!(
            "terrain-index-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir_all(&folder)?;
        let source = folder.join("source.sqlite3");
        let cache = folder.join("index.json");
        let mut conn = yamamuki_data_sqlite::rusqlite::Connection::open(&source)?;
        yamamuki_data_sqlite::initialize(&conn)?;
        yamamuki_terrain::fixture::create(&mut conn)?;
        drop(conn);
        let reference = Database::open(&source)?;
        save(&reference, &cache, "original")?;
        let mut restored = Database::open_metadata(&source)?;
        assert!(read(&mut restored, &cache, "original"));
        assert_eq!(reference.catalog, restored.catalog);
        assert!(!read(&mut restored, &cache, "modified"));
        std::fs::write(&cache, b"not json")?;
        assert!(!read(&mut restored, &cache, "original"));
        std::fs::remove_file(cache)?;
        drop(reference);
        drop(restored);
        std::fs::remove_file(source)?;
        std::fs::remove_dir(folder)?;
        Ok(())
    }
}
