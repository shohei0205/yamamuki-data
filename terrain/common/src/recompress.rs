//! 既存SQLiteの全制御点・親依存を保ち、圧縮本文だけを最適化する。
use crate::{natural_codec, natural_codec_zstd, sqlite::Database};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, backup::Backup, params};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};
#[derive(Default)]
struct Stats {
    tiles: usize,
    changed: usize,
    old: usize,
    new: usize,
    decode_old: f64,
    decode_new: f64,
}
pub fn copy(destination: &mut Connection, input: &Path) -> Result<()> {
    let mut source = Database::open(input)?;
    Backup::new(&source.connection, destination)?.run_to_completion(256, Duration::ZERO, None)?;
    let mut keys = source.catalog.keys().copied().collect::<Vec<_>>();
    keys.sort_unstable();
    let mut levels = BTreeMap::<u32, Stats>::new();
    let tx = destination.transaction()?;
    for (i, key) in keys.iter().copied().enumerate() {
        let original = source.tile(key)?;
        let mut tile = original.clone();
        tile.controls = natural_codec_zstd::recompress(&tile.controls)?;
        ensure!(
            tile.controls.len() <= original.controls.len(),
            "再圧縮で容量が増えました"
        );
        let expected = source.points(key)?;
        let parents = tile
            .parents
            .iter()
            .map(|k| source.points(*k))
            .collect::<Result<Vec<_>>>()?;
        let references = parents.iter().map(|p| p.as_slice()).collect::<Vec<_>>();
        let decoded = natural_codec::decode_global_refs(&tile.controls, tile.origin, &references)?;
        ensure!(
            decoded.as_slice() == expected.as_slice(),
            "再圧縮で整数XYZ・順序が変化しました: {key:?}"
        );
        let stats = levels.entry(key.0).or_default();
        stats.tiles += 1;
        stats.old += original.controls.len();
        stats.new += tile.controls.len();
        stats.changed += usize::from(tile.controls != original.controls);
        // I/O・親復元・圧縮・SHA照合と、本文の復号時間を分けて計測する。
        for _ in 0..5 {
            let t = Instant::now();
            std::hint::black_box(natural_codec::decode_global_refs(
                &original.controls,
                tile.origin,
                &references,
            )?);
            stats.decode_old += t.elapsed().as_secs_f64() / 5.;
            let t = Instant::now();
            std::hint::black_box(natural_codec::decode_global_refs(
                &tile.controls,
                tile.origin,
                &references,
            )?);
            stats.decode_new += t.elapsed().as_secs_f64() / 5.;
        }
        tx.execute("UPDATE terrain_tiles SET payload=?1 WHERE dataset_key=?2 AND level_key=(SELECT level_key FROM terrain_levels WHERE dataset_key=?2 AND z=?3) AND x=?4 AND y=?5",params![tile.encode()?,source.dataset,key.0,key.1,key.2])?;
        if (i + 1) % 1024 == 0 {
            eprintln!("地形再圧縮 {}/{}", i + 1, keys.len());
        }
    }
    tx.commit()?;
    destination.execute_batch("VACUUM;PRAGMA optimize;")?;
    yamamuki_data_sqlite::validate(destination)?;
    let path = destination
        .path()
        .context("再圧縮先SQLiteのパスがありません")?;
    Database::open(Path::new(path))?.validate_all()?;
    println!("z,tiles,changed,old_bytes,new_bytes,old_decode_ms,new_decode_ms");
    let mut total = Stats::default();
    for (z, s) in levels {
        println!(
            "{z},{},{},{},{},{:.3},{:.3}",
            s.tiles,
            s.changed,
            s.old,
            s.new,
            s.decode_old * 1000.,
            s.decode_new * 1000.
        );
        total.tiles += s.tiles;
        total.changed += s.changed;
        total.old += s.old;
        total.new += s.new;
        total.decode_old += s.decode_old;
        total.decode_new += s.decode_new;
    }
    println!(
        "total,{},{},{},{},{:.3},{:.3}",
        total.tiles,
        total.changed,
        total.old,
        total.new,
        total.decode_old * 1000.,
        total.decode_new * 1000.
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recompression_preserves_container_and_all_xyz() -> Result<()> {
        let original = tempfile::NamedTempFile::new()?;
        let mut conn = Connection::open(original.path())?;
        yamamuki_data_sqlite::initialize(&conn)?;
        crate::fixture::create(&mut conn)?;
        drop(conn);
        let output = tempfile::NamedTempFile::new()?;
        let mut conn = Connection::open(output.path())?;
        copy(&mut conn, original.path())?;
        let mut before = Database::open(original.path())?;
        let mut after = Database::open(output.path())?;
        assert_eq!(before.catalog, after.catalog);
        assert_eq!(before.generation, after.generation);
        for key in before.catalog.keys().copied().collect::<Vec<_>>() {
            assert_eq!(before.points(key)?, after.points(key)?);
            assert_eq!(before.tile(key)?.parents, after.tile(key)?.parents);
        }
        Ok(())
    }
}
