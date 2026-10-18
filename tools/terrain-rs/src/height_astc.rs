//! タイル単位に正規化した標高の ASTC 比較。欠損マスクは可逆に分離する。
use crate::{bench_png, packet, write_json};
use anyhow::{Result, ensure};
use serde_json::json;
use std::{fs, io::Cursor, path::Path, process::Command, time::Instant};
const CORE: usize = 256;
const PAD: usize = 4;
const SIDE: usize = 264; // 8 と 12 の公倍数。両端に余白を置く。
fn range(values: &[f32]) -> (f32, f32) {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for &v in values {
        if v.is_finite() {
            min = min.min(v);
            max = max.max(v);
        }
    }
    if !min.is_finite() {
        (0., 0.)
    } else {
        (min, max - min)
    }
}
fn encode(v: f32, min: f32, span: f32) -> u8 {
    if !v.is_finite() || span == 0. {
        0
    } else {
        ((v - min) / span * 255.).round().clamp(0., 255.) as u8
    }
}
fn decode(v: u8, min: f32, span: f32) -> f32 {
    min + v as f32 / 255. * span
}
fn errors(original: &[f32], restored: &[f32], width: usize, height: usize) -> serde_json::Value {
    let mut max = 0f64;
    let mut squares = 0.;
    let mut count = 0;
    let mut differences = Vec::new();
    let mut peak = f32::NEG_INFINITY;
    let mut restored_peak = f32::NEG_INFINITY;
    let mut peak_index = 0;
    let mut restored_index = 0;
    let mut boundary = 0f64;
    let mut boundary_sq = 0.;
    let mut boundary_count = 0;
    for i in 0..original.len() {
        if !original[i].is_finite() {
            continue;
        }
        let e = (restored[i] - original[i]) as f64;
        max = max.max(e.abs());
        squares += e * e;
        count += 1;
        differences.push(e.abs());
        if original[i] > peak {
            peak = original[i];
            peak_index = i;
        }
        if restored[i] > restored_peak {
            restored_peak = restored[i];
            restored_index = i;
        }
        let x = i % width;
        let y = i / width;
        for neighbor in [
            if x > 0 && x % CORE == 0 {
                Some(i - 1)
            } else {
                None
            },
            if y > 0 && y % CORE == 0 {
                Some(i - width)
            } else {
                None
            },
        ]
        .into_iter()
        .flatten()
        {
            if original[neighbor].is_finite() {
                let jump = (e - (restored[neighbor] - original[neighbor]) as f64).abs();
                boundary = boundary.max(jump);
                boundary_sq += jump * jump;
                boundary_count += 1;
            }
        }
    }
    differences.sort_unstable_by(f64::total_cmp);
    let p95 = if differences.is_empty() {
        0.
    } else {
        differences[(differences.len() - 1) * 95 / 100]
    };
    let shift = (((peak_index % width) as f64 - (restored_index % width) as f64).powi(2)
        + ((peak_index / width) as f64 - (restored_index / width) as f64).powi(2))
    .sqrt();
    json!({"maxErrorMeters":max,"rmseMeters":if count>0{(squares/count as f64).sqrt()}else{0.},"p95ErrorMeters":p95,"validPixels":count,"boundaryResidualJumpMaxMeters":boundary,"boundaryResidualJumpRmseMeters":if boundary_count>0{(boundary_sq/boundary_count as f64).sqrt()}else{0.},"peakHeightErrorMeters":if count>0{restored_peak-peak}else{0.},"peakShiftPixels":shift,"width":width,"height":height})
}
pub fn run(root: &Path, output: &Path, encoder: &Path) -> Result<()> {
    let directory = output.join("height-astc");
    fs::create_dir_all(&directory)?;
    let temporary = directory.join("work");
    fs::create_dir_all(&temporary)?;
    let mut results = Vec::new();
    for level in (4..=14).rev() {
        let (meta, original) = bench_png::heights(root, level)?;
        let width = meta["width"].as_u64().unwrap() as usize;
        let height = meta["height"].as_u64().unwrap() as usize;
        let columns = width.div_ceil(CORE);
        let rows = height.div_ceil(CORE);
        let mask: Vec<u8> = original
            .iter()
            .map(|v| if v.is_finite() { 255 } else { 0 })
            .collect();
        let mask_compressed = zstd::stream::encode_all(Cursor::new(&mask), 9)?;
        for block in [8usize, 12] {
            let started = Instant::now();
            let mut tiles = Vec::new();
            let mut raw = Vec::new();
            let mut restored = vec![f32::NAN; width * height];
            let mut normalized = vec![f32::NAN; width * height];
            let mut preview = vec![[0, 0, 0, 255]; width * height];
            for ty in 0..rows {
                for tx in 0..columns {
                    let w = (width - tx * CORE).min(CORE);
                    let h = (height - ty * CORE).min(CORE);
                    let tile: Vec<_> = (0..h)
                        .flat_map(|y| (0..w).map(move |x| (y, x)))
                        .map(|(y, x)| original[(ty * CORE + y) * width + tx * CORE + x])
                        .collect();
                    let (min, span) = range(&tile);
                    let mut input = Vec::with_capacity(SIDE * SIDE);
                    for y in 0..SIDE {
                        for x in 0..SIDE {
                            let sx = x.saturating_sub(PAD).min(w - 1);
                            let sy = y.saturating_sub(PAD).min(h - 1);
                            let q = encode(tile[sy * w + sx], min, span);
                            input.push([q, q, q, 255]);
                        }
                    }
                    let input_path = temporary.join("input.png");
                    let astc_path = temporary.join("encoded.astc");
                    let decoded_path = temporary.join("decoded.png");
                    bench_png::write_encoder_input(&input_path, SIDE, SIDE, &input)?;
                    let status = Command::new(encoder)
                        .arg("-cl")
                        .arg(&input_path)
                        .arg(&astc_path)
                        .args([
                            &format!("{block}x{block}"),
                            "-thorough",
                            "-esw",
                            "rrr1",
                            "-cw",
                            "1",
                            "0",
                            "0",
                            "0",
                            "-decode_unorm8",
                            "-j",
                            "4",
                            "-silent",
                        ])
                        .status()?;
                    ensure!(status.success(), "標高 ASTC の符号化に失敗しました");
                    let status = Command::new(encoder)
                        .arg("-dl")
                        .arg(&astc_path)
                        .arg(&decoded_path)
                        .args(["-j", "4", "-silent"])
                        .status()?;
                    ensure!(status.success(), "標高 ASTC の復元に失敗しました");
                    let astc = fs::read(&astc_path)?;
                    ensure!(
                        astc.len() == 16 + SIDE.div_ceil(block).pow(2) * 16,
                        "標高 ASTC の寸法が不正です"
                    );
                    raw.extend(&astc[16..]);
                    let decoded = bench_png::read(&decoded_path)?;
                    for y in 0..h {
                        for x in 0..w {
                            let i = (ty * CORE + y) * width + tx * CORE + x;
                            let q = decoded.pixels[(y + PAD) * SIDE + x + PAD][0];
                            preview[i] = [q, q, q, 255];
                            if mask[i] > 0 {
                                restored[i] = decode(q, min, span);
                                normalized[i] = decode(encode(original[i], min, span), min, span);
                            }
                        }
                    }
                    tiles.push(json!({"x":tx,"y":ty,"min":min,"range":span}));
                }
            }
            let compressed = zstd::stream::encode_all(Cursor::new(&raw), 9)?;
            let header = json!({"width":width,"height":height,"core":CORE,"padding":PAD,"side":SIDE,"columns":columns,"rows":rows,"layers":tiles.len(),"block":block,"tiles":tiles,"astcBytes":compressed.len(),"gpuBytes":raw.len(),"maskBytes":mask_compressed.len(),"normalization":"tile min + R * range; UNORM8 reference decode"});
            let packed = packet(b"YHA1", &header, &[&compressed, &mask_compressed])?;
            let base = format!("L{level}-astc{block}");
            fs::write(directory.join(format!("{base}.yha")), &packed)?;
            bench_png::write(
                &directory.join(format!("{base}-decoded.png")),
                width,
                height,
                &preview,
            )?;
            let metrics = errors(&original, &restored, width, height);
            let mut row = metrics.as_object().unwrap().clone();
            row.extend(json!({"level":level,"codec":format!("height-astc{block}"),"block":block,"bytes":packed.len(),"gpuPayloadBytes":raw.len(),"pngBytes":meta["heightBytes"],"normalizedOnly":errors(&original,&normalized,width,height),"url":format!("/height-astc/{base}.yha"),"previewUrl":format!("/height-astc/{base}-decoded.png"),"encodeProcessSeconds":started.elapsed().as_secs_f64(),"peakShiftMeters":metrics["peakShiftPixels"].as_f64().unwrap()*meta["meters"].as_f64().unwrap()}).as_object().unwrap().clone());
            results.push(serde_json::Value::Object(row));
            write_json(
                &directory.join("report.json"),
                &json!({"state":if level==4&&block==12{"complete"}else{"running"},"results":results,"method":"256 四方ごとに最小標高と範囲を保存。単一標高を R に正規化、ASTC 8/12 と Zstd。欠損マスクは可逆。全描画格子が同じ標高サンプルを参照し、境界頂点を二重に持たない。境界指標は圧縮誤差の段差であり、隙間の大きさではない。"}),
            )?;
            println!(
                "{}",
                json!({"level":level,"block":block,"state":"complete","maxErrorMeters":metrics["maxErrorMeters"],"bytes":packed.len()})
            );
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization_handles_negative_flat_and_missing() {
        assert_eq!(range(&[-20., f32::NAN, 30.]), (-20., 50.));
        assert_eq!(decode(encode(5., 5., 0.), 5., 0.), 5.);
        assert_eq!(encode(f32::NAN, 0., 100.), 0);
        for v in [-20., -5., 30.] {
            assert!((decode(encode(v, -20., 50.), -20., 50.) - v).abs() <= 50. / 510. + 0.0001);
        }
    }
    #[test]
    fn boundary_error_is_error_difference() {
        let a = vec![0.; 512];
        let mut b = a.clone();
        b[255] = 1.;
        b[256] = -2.;
        let e = errors(&a, &b, 512, 1);
        assert_eq!(e["boundaryResidualJumpMaxMeters"], 3.);
    }
}
