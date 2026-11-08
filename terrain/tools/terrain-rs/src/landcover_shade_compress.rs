use anyhow::{Context, Result, ensure};
use clap::Parser;
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Cursor, path::PathBuf};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    compress_hillshade: PathBuf,
    #[arg(long)]
    libwebp: PathBuf,
    #[arg(long, required_unless_present = "lossless_masks", conflicts_with = "lossless_masks", value_parser = clap::value_parser!(u8).range(1..=100))]
    quality: Option<u8>,
    #[arg(long)]
    lossless_masks: bool,

    #[arg(long)]
    output: PathBuf,
}

type Encode = unsafe extern "C" fn(*const u8, i32, i32, i32, f32, *mut *mut u8) -> usize;
type EncodeLossless = unsafe extern "C" fn(*const u8, i32, i32, i32, *mut *mut u8) -> usize;
type Free = unsafe extern "C" fn(*mut u8);

#[derive(Clone, Copy)]
enum Compression {
    Shade(Encode, u8),

    Mask(EncodeLossless),
}

fn compress(png: Vec<u8>, codec: Compression, free: Free) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(Cursor::new(&png));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    ensure!(
        info.width == info.height
            && matches!(info.width, 256 | 384 | 512)
            && info.color_type == png::ColorType::Grayscale
            && info.bit_depth
                == match codec {
                    Compression::Shade(..) => png::BitDepth::Eight,
                    Compression::Mask(_) => png::BitDepth::Four,
                },
        "Unexpected grayscale PNG format"
    );
    let side = info.width as i32;
    let mut gray = vec![0; (side * side) as usize];
    reader.next_frame(&mut gray)?;
    let rgb: Vec<_> = gray.into_iter().flat_map(|p| [p; 3]).collect();
    let mut output = std::ptr::null_mut();
    let len = unsafe {
        match codec {
            Compression::Shade(encode, quality) => encode(
                rgb.as_ptr(),
                side,
                side,
                3 * side,
                f32::from(quality),
                &mut output,
            ),
            Compression::Mask(encode) => encode(rgb.as_ptr(), side, side, 3 * side, &mut output),
        }
    };
    ensure!(len != 0 && !output.is_null(), "WebP encoding failed");
    let encoded = if len < png.len() {
        unsafe { std::slice::from_raw_parts(output, len) }.to_vec()
    } else {
        png
    };
    unsafe { free(output) };
    Ok(encoded)
}

pub(super) fn run() -> Result<()> {
    let args = Args::parse();
    let source =
        Connection::open_with_flags(&args.compress_hillshade, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let metadata: String = source.query_row(
        "SELECT value FROM metadata WHERE key='hillshade'",
        [],
        |r| r.get(0),
    )?;
    let mut spec: Value = serde_json::from_str(&metadata)?;
    let manifest: String =
        source.query_row("SELECT value FROM metadata WHERE key='manifest'", [], |r| {
            r.get(0)
        })?;
    let mut manifest: Value = serde_json::from_str(&manifest)?;
    let webp = unsafe { libloading::Library::new(&args.libwebp) }?;
    let codec = if let Some(quality) = args.quality {
        ensure!(
            spec["encoding"] == "png-gray8" && spec["bitDepth"] == 8,
            "8-bit PNG hillshade required"
        );
        Compression::Shade(*unsafe { webp.get::<Encode>(b"WebPEncodeRGB") }?, quality)
    } else {
        ensure!(
            manifest["landEncoding"] == "forest-bare-unified-gray4"
                && manifest["waterEncoding"] == "gray4-coverage"
                && manifest["maskWebp"] != true,
            "4-bit forest and water PNG layers required"
        );
        Compression::Mask(*unsafe { webp.get::<EncodeLossless>(b"WebPEncodeLosslessRGB") }?)
    };
    let free = *unsafe { webp.get::<Free>(b"WebPFree") }?;
    fs::create_dir(&args.output).context("新しい出力フォルダーを指定してください")?;
    let temp = args.output.join("landcover-with-hillshade.sqlite3.tmp");
    let path = args.output.join("landcover-with-hillshade.sqlite3");
    source.execute(
        "VACUUM INTO ?1",
        [temp.to_str().context("Invalid output path")?],
    )?;
    let mut db = Connection::open(&temp)?;
    let tx = db.transaction()?;
    let tables: &[&str] = if args.lossless_masks {
        &["tiles", "water_tiles"]
    } else {
        &["hillshade_tiles"]
    };
    for &table in tables {
        let mut stats = BTreeMap::<u32, [usize; 2]>::new();
        let mut read =
            source.prepare(&format!("SELECT z,x,y,payload FROM {table} ORDER BY z,x,y"))?;
        let mut rows = read.query([])?;
        let mut write = tx.prepare(&format!(
            "UPDATE {table} SET payload=?4 WHERE z=?1 AND x=?2 AND y=?3"
        ))?;
        let mut count = 0;
        loop {
            let mut batch = Vec::<(u32, u32, u32, Vec<u8>)>::new();
            for _ in 0..64 {
                let Some(r) = rows.next()? else { break };
                batch.push((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?));
            }
            if batch.is_empty() {
                break;
            }
            let batch = batch
                .into_par_iter()
                .map(|(z, x, y, png)| Ok((z, x, y, compress(png, codec, free)?)))
                .collect::<Result<Vec<_>>>()?;
            for (z, x, y, encoded) in batch {
                let stats = stats.entry(z).or_default();
                stats[0] += encoded.len();
                if encoded.starts_with(b"RIFF") {
                    stats[1] += 1;
                    write.execute(params![z, x, y, encoded])?;
                }
                count += 1;
            }
            if count % 256 == 0 {
                println!("Checked {count} {table} tiles");
            }
        }
        let rows = match table {
            "tiles" => &mut manifest["levels"],
            "water_tiles" => &mut manifest["waterLevels"],
            _ => &mut spec["levels"],
        };
        for row in rows.as_array_mut().context("Missing level statistics")? {
            let z = row["z"].as_u64().context("Missing level zoom")? as u32;
            let [bytes, webp_tiles] = stats.get(&z).copied().unwrap_or_default();
            row["webpTiles"] = json!(webp_tiles);
            match table {
                "tiles" => {
                    let bare = row["barePayloadBytes"].as_u64().unwrap_or(0);
                    let missing = row["landMissingPayloadBytes"].as_u64().unwrap_or(0);
                    row["forestPayloadBytes"] = json!(bytes);
                    row["payloadBytes"] = json!(bytes as u64 + bare + missing);
                }
                "water_tiles" => row["payloadBytes"] = json!(bytes),
                _ => {
                    row.as_object_mut()
                        .context("Invalid shade level")?
                        .remove("pngBytes");
                    row["payloadBytes"] = json!(bytes);
                }
            }
        }
        println!("{table}: {stats:?}");
    }
    if args.lossless_masks {
        manifest["maskWebp"] = json!(true);
        tx.execute(
            "UPDATE metadata SET value=?1 WHERE key='manifest'",
            [manifest.to_string()],
        )?;
    } else {
        spec["encoding"] = json!("webp-or-png-gray8");
        spec["webpQuality"] = json!(args.quality);
        tx.execute(
            "UPDATE metadata SET value=?1 WHERE key='hillshade'",
            [spec.to_string()],
        )?;
    }
    tx.commit()?;
    db.execute_batch("VACUUM")?;
    let check: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(check == "ok", "SQLite integrity check failed: {check}");
    drop(db);
    fs::rename(temp, &path)?;
    println!("Complete: {}", path.display());
    Ok(())
}
