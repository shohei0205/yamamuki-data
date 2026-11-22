use anyhow::{Result, ensure};
use std::{path::Path, time::Instant};
use yamamuki_terrain::{natural_codec, natural_codec_zstd, sqlite::Database};
fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    ensure!(
        args.len() >= 2 && args.len() <= 3,
        "使い方: codec_probe <従来形式のSQLite> [抽出タイル数]"
    );
    let mut db = Database::open(Path::new(&args[1]))?;
    let mut keys = db.catalog.keys().copied().collect::<Vec<_>>();
    keys.sort_unstable();
    let limit = args
        .get(2)
        .map(|v| v.parse::<usize>())
        .transpose()?
        .unwrap_or(1000);
    ensure!(limit > 0, "抽出タイル数は1以上を指定してください");
    let stride = (keys.len() / limit).max(1);
    let mut samples = Vec::new();
    for key in keys.iter().step_by(stride) {
        let tile = db.tile(*key)?;
        let parents = tile
            .parents
            .iter()
            .map(|k| db.points(*k).map(|p| p.as_ref().clone()))
            .collect::<Result<Vec<_>>>()?;
        let expected = natural_codec::decode_global(&tile.controls, tile.origin, &parents)?;
        samples.push((tile, parents, expected));
    }
    let mut old_bytes = 0usize;
    let mut old_time = 0.;
    for (tile, parents, expected) in &samples {
        old_bytes += tile.controls.len();
        let t = Instant::now();
        for _ in 0..5 {
            let p = natural_codec::decode_global(&tile.controls, tile.origin, parents)?;
            std::hint::black_box(p);
        }
        old_time += t.elapsed().as_secs_f64() / 5.;
        assert_eq!(
            *expected,
            natural_codec::decode_global(&tile.controls, tile.origin, parents)?
        );
    }
    println!(
        "legacy tiles={} bytes={old_bytes} decode_ms={:.3}",
        samples.len(),
        old_time * 1000.
    );
    for block in [128, 512, 1024, 4096] {
        let mut size = 0;
        let mut time = 0.;
        let started = Instant::now();
        for (tile, parents, expected) in &samples {
            let b = natural_codec_zstd::transcode(&tile.controls, block, 3)?;
            size += b.len();
            assert_eq!(
                *expected,
                natural_codec::decode_global(&b, tile.origin, parents)?
            );
            let t = Instant::now();
            for _ in 0..5 {
                std::hint::black_box(natural_codec::decode_global(&b, tile.origin, parents)?);
            }
            time += t.elapsed().as_secs_f64() / 5.;
        }
        println!(
            "block={block} bytes={size} decode_ms={:.3} encode+bench_s={:.2}",
            time * 1000.,
            started.elapsed().as_secs_f64()
        );
    }
    let mut size = 0;
    let mut time = 0.;
    for (tile, parents, expected) in &samples {
        let b = natural_codec_zstd::recompress(&tile.controls)?;
        size += b.len();
        assert!(b.len() <= tile.controls.len());
        assert_eq!(
            *expected,
            natural_codec::decode_global(&b, tile.origin, parents)?
        );
        let t = Instant::now();
        for _ in 0..5 {
            std::hint::black_box(natural_codec::decode_global(&b, tile.origin, parents)?);
        }
        time += t.elapsed().as_secs_f64() / 5.;
    }
    println!("adaptive bytes={size} decode_ms={:.3}", time * 1000.);

    Ok(())
}
