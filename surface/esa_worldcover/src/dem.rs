use anyhow::{Context, Result, ensure};
use flate2::{Compression, write::GzEncoder};
use image::ImageReader;
use rayon::prelude::*;
use rusqlite::{Connection, params};
use std::{fs, io::Write, path::Path};

const SIDE: usize = 256;
const STEP: f32 = 5.;

pub fn build(db: &mut Connection, source: &Path) -> Result<()> {
    let mut files = Vec::new();
    for column in fs::read_dir(source)? {
        let column = column?;
        if !column.file_type()?.is_dir() {
            continue;
        }
        let x: u32 = column
            .file_name()
            .to_str()
            .context("DEMのX座標が不正です")?
            .parse()?;
        for row in fs::read_dir(column.path())? {
            let path = row?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
                continue;
            }
            let y: u32 = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .context("DEMのY座標が不正です")?
                .parse()?;
            ensure!(x < (1 << 14) && y < (1 << 14), "DEMのXYZ座標が範囲外です");
            files.push((x, y, path));
        }
    }
    ensure!(
        !files.is_empty(),
        "z14のDEM10B PNGがありません: {}",
        source.display()
    );
    files.sort_unstable();
    let tx = db.transaction()?;
    tx.execute_batch("CREATE TABLE tiles(z INTEGER NOT NULL,x INTEGER NOT NULL,y INTEGER NOT NULL,data BLOB NOT NULL,PRIMARY KEY(z,x,y)) WITHOUT ROWID;")?;
    {
        let mut insert = tx.prepare("INSERT INTO tiles VALUES(?1,?2,?3,?4)")?;
        for batch in files.chunks(64) {
            let images: Vec<_> = batch
                .par_iter()
                .map(|(_, _, path)| read_png(path).and_then(|values| encode(&values)))
                .collect();
            for ((x, y, _), data) in batch.iter().zip(images) {
                insert.execute(params![14, x, y, data?])?;
            }
        }
        println!("DEM z14: {}枚", files.len());
        for zoom in (7..14).rev() {
            let parents: Vec<(u32, u32)> = tx
                .prepare("SELECT DISTINCT x/2,y/2 FROM tiles WHERE z=?1 ORDER BY x/2,y/2")?
                .query_map([zoom + 1], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let mut children = tx.prepare(
                "SELECT x,y,data FROM tiles WHERE z=?1 AND x BETWEEN ?2 AND ?2+1 AND y BETWEEN ?3 AND ?3+1"
            )?;
            for batch in parents.chunks(64) {
                let mut inputs = Vec::with_capacity(batch.len());
                for &(x, y) in batch {
                    let rows = children
                        .query_map(params![zoom + 1, x * 2, y * 2], |row| {
                            Ok((
                                row.get::<_, usize>(0)? % 2,
                                row.get::<_, usize>(1)? % 2,
                                row.get::<_, Vec<u8>>(2)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    inputs.push(rows);
                }
                let results: Vec<_> = inputs.into_par_iter().map(parent).collect();
                for (&(x, y), data) in batch.iter().zip(results) {
                    insert.execute(params![zoom, x, y, data?])?;
                }
            }
            println!("DEM z{zoom}: {}枚", parents.len());
        }
    }
    tx.commit()?;
    Ok(())
}

fn read_png(path: &Path) -> Result<Vec<f32>> {
    let image = ImageReader::with_format(
        std::io::BufReader::new(fs::File::open(path)?),
        image::ImageFormat::Png,
    )
    .decode()
    .with_context(|| format!("DEMを復号できません: {}", path.display()))?;
    ensure!(
        image.width() == SIDE as u32 && image.height() == SIDE as u32,
        "DEMタイルは256×256で指定してください"
    );
    Ok(image
        .into_rgba8()
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            let code = i32::from_be_bytes([0, r, g, b]);
            if code == 0x800000 || a == 0 {
                f32::NAN
            } else {
                ((code << 8) >> 8) as f32 * 0.01
            }
        })
        .collect())
}

fn parent(children: Vec<(usize, usize, Vec<u8>)>) -> Result<Vec<u8>> {
    let mut values = vec![f32::NAN; SIDE * SIDE];
    for (dx, dy, bytes) in children {
        let child = crate::dem_codec::decode_grid(&bytes)?;
        ensure!(
            child.rows == SIDE && child.columns == SIDE,
            "標高タイルの寸法が不正です"
        );
        for y in 0..SIDE / 2 {
            for x in 0..SIDE / 2 {
                let i = (y * SIDE + x) * 2;
                let (mut total, mut count) = (0., 0);
                for offset in [0, 1, SIDE, SIDE + 1] {
                    let value = child.values[i + offset];
                    if value.is_finite() {
                        total += value;
                        count += 1;
                    }
                }
                if count > 0 {
                    values[(dy * SIDE / 2 + y) * SIDE + dx * SIDE / 2 + x] = total / count as f32;
                }
            }
        }
    }
    encode(&values)
}

fn encode(values: &[f32]) -> Result<Vec<u8>> {
    let quantized: Vec<_> = values
        .iter()
        .map(|&v| v.is_finite().then(|| (v / STEP).round_ties_even() as i32))
        .collect();
    let base = quantized.iter().filter_map(|&v| v).min().unwrap_or(0);
    let relative = |i: usize| quantized[i].map_or(0, |v| v - base);
    let mut delta = Vec::with_capacity(values.len());
    let mut missing = vec![0u8; values.len().div_ceil(8)];
    for (i, value) in quantized.iter().enumerate() {
        if value.is_none() {
            missing[i / 8] |= 1 << (i % 8);
        }
        let (x, y) = (i % SIDE, i / SIDE);
        let mut d = relative(i);
        if x > 0 {
            d -= relative(i - 1);
        }
        if y > 0 {
            d -= relative(i - SIDE);
        }
        if x > 0 && y > 0 {
            d += relative(i - SIDE - 1);
        }
        delta.push(d);
    }
    let wide = delta.iter().any(|&d| i16::try_from(d).is_err());
    let masked = missing.iter().any(|&mask| mask != 0);
    let mut raw = Vec::with_capacity(values.len() * if wide { 4 } else { 2 } + missing.len());
    for d in delta {
        if wide {
            raw.extend_from_slice(&d.to_le_bytes());
        } else {
            raw.extend_from_slice(&(d as i16).to_le_bytes());
        }
    }
    if masked {
        raw.extend(missing);
    }
    let mut output = b"YMD1".to_vec();
    output.extend((SIDE as u16).to_le_bytes());
    output.extend((SIDE as u16).to_le_bytes());
    output.extend(STEP.to_le_bytes());
    output.extend(base.to_le_bytes());
    output.extend([u8::from(wide) | (u8::from(masked) << 1), 0]);
    let mut writer = GzEncoder::new(output, Compression::fast());
    writer.write_all(&raw)?;
    Ok(writer.finish()?)
}

#[test]
fn dem_rounding_and_parent_keep_missing_samples() {
    let mut samples = vec![f32::NAN; SIDE * SIDE];
    samples[0] = 105.;
    samples[1] = 110.;
    samples[SIDE] = 100.;
    let body = encode(&samples).unwrap();
    let grid = crate::dem_codec::decode_grid(&body).unwrap();
    assert_eq!(grid.values[0], 105.);
    assert!(grid.values[SIDE + 1].is_nan());
    let body = parent(vec![(1, 0, body)]).unwrap();
    let grid = crate::dem_codec::decode_grid(&body).unwrap();
    assert_eq!(grid.values[SIDE / 2], 105.);
    assert!(grid.values[0].is_nan());
}
