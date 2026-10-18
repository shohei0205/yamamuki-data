//! JPEG XL の保存容量・データ誤差を代表地域で測る。GPU 圧縮とは区別する。
use crate::{bench_normal, bench_png, error, packet, write_json};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs, io::Cursor, path::Path, process::Command, time::Instant};
fn pgm(path: &Path, w: usize, h: usize, values: &[u16]) -> Result<()> {
    ensure!(values.len() == w * h, "PGM の画素数が不正です");
    let mut bytes = format!("P5\n{w} {h}\n65535\n").into_bytes();
    for v in values {
        bytes.extend(v.to_be_bytes());
    }
    fs::write(path, bytes)?;
    Ok(())
}
pub(crate) fn read_pgm(path: &Path) -> Result<Vec<u16>> {
    let bytes = fs::read(path)?;
    let mut offset = 0;
    let mut tokens = Vec::new();
    for _ in 0..4 {
        loop {
            while offset < bytes.len() && bytes[offset].is_ascii_whitespace() {
                offset += 1;
            }
            if bytes.get(offset) == Some(&b'#') {
                while offset < bytes.len() && bytes[offset] != b'\n' {
                    offset += 1;
                }
            } else {
                break;
            }
        }
        let start = offset;
        while offset < bytes.len() && !bytes[offset].is_ascii_whitespace() {
            offset += 1;
        }
        tokens.push(std::str::from_utf8(&bytes[start..offset])?.to_owned());
    }
    ensure!(
        tokens[0] == "P5" && tokens[3] == "65535",
        "16 bit グレースケールの復元が必要です"
    );
    ensure!(
        bytes.get(offset).is_some_and(u8::is_ascii_whitespace),
        "PGM の区切りがありません"
    );
    if bytes[offset] == b'\r' && bytes.get(offset + 1) == Some(&b'\n') {
        offset += 2;
    } else {
        offset += 1;
    }
    let n = tokens[1].parse::<usize>()? * tokens[2].parse::<usize>()?;
    ensure!(bytes.len() - offset == n * 2, "PGM の長さが不正です");
    Ok(bytes[offset..]
        .chunks_exact(2)
        .map(|p| u16::from_be_bytes([p[0], p[1]]))
        .collect())
}
fn encode(bin: &Path, input: &Path, output: &Path, distance: f64, gray: bool) -> Result<f64> {
    let started = Instant::now();
    let color = if gray {
        "Gra_D65_Rel_Lin"
    } else {
        "RGB_D65_SRG_Rel_Lin"
    };
    let s = Command::new(bin.join("cjxl.exe"))
        .arg(input)
        .arg(output)
        .args([
            "-d",
            &distance.to_string(),
            "-e",
            "7",
            "--num_threads=4",
            "-x",
            &format!("color_space={color}"),
            "--quiet",
        ])
        .status()?;
    ensure!(s.success(), "JPEG XL の符号化に失敗しました");
    Ok(started.elapsed().as_secs_f64() * 1000.)
}
fn decode(bin: &Path, input: &Path, output: &Path, gray: bool) -> Result<f64> {
    let color = if gray {
        "Gra_D65_Rel_Lin"
    } else {
        "RGB_D65_SRG_Rel_Lin"
    };
    let s = Command::new(bin.join("djxl.exe"))
        .arg(input)
        .arg(output)
        .args([
            "--num_threads=4",
            &format!("--bits_per_sample={}", if gray { 16 } else { 8 }),
            &format!("--color_space={color}"),
            "--quiet",
        ])
        .status()?;
    ensure!(s.success(), "JPEG XL の復元に失敗しました");
    // 出力画像の圧縮を除き、起動・ファイル読み取りを含む 3 回の復元の平均を測る。
    let t = Instant::now();
    let s = Command::new(bin.join("djxl.exe"))
        .arg(input)
        .args([
            "--disable_output",
            "--num_reps=3",
            "--num_threads=4",
            "--quiet",
        ])
        .status()?;
    ensure!(s.success(), "JPEG XL の復元計測に失敗しました");
    Ok(t.elapsed().as_secs_f64() * 1000. / 3.)
}
fn save_report(folder: &Path, results: &[Value], complete: bool) -> Result<()> {
    write_json(
        &folder.join("report.json"),
        &json!({"state":if complete{"complete"}else{"running"},"results":results,"library":"libjxl 0.12.0; effort 7; 4 threads; linear color; raw samples","timing":"復元は出力画像の書き出しを除く 3 回の平均。プロセス起動とファイル読み取りを含み、既存のメモリー内復元計測とは直接比較しない。","scope":"槍ヶ岳周辺の代表レベルと法線解像度 1024。全国容量と Android/iOS 性能の確定値ではない。"}),
    )
}
pub fn run(root: &Path, output: &Path, bin: &Path) -> Result<()> {
    let folder = output.join("jpeg-xl");
    fs::create_dir_all(&folder)?;
    let mut results = Vec::new();
    for level in [14, 12, 10, 9] {
        let (meta, original) = bench_png::heights(root, level)?;
        let w = meta["width"].as_u64().unwrap() as usize;
        let h = meta["height"].as_u64().unwrap() as usize;
        let mask: Vec<u8> = original.iter().map(|v| v.is_finite() as u8).collect();
        let mask_zstd = zstd::stream::encode_all(Cursor::new(mask), 9)?;
        for (distance, quantum) in [(0., 1.), (0., 10.), (0.1, 0.), (0.5, 0.), (1., 0.)] {
            let min = original
                .iter()
                .copied()
                .filter(|v| v.is_finite())
                .fold(f32::INFINITY, f32::min);
            let max = original
                .iter()
                .copied()
                .filter(|v| v.is_finite())
                .fold(f32::NEG_INFINITY, f32::max);
            let scale = if quantum > 0. {
                quantum
            } else {
                (max - min).max(1.) / 65535.
            };
            let q: Vec<u16> = original
                .iter()
                .map(|v| {
                    if v.is_finite() {
                        ((v - min) / scale).round().clamp(0., 65535.) as u16
                    } else {
                        0
                    }
                })
                .collect();
            let name = format!("height-L{level}-d{distance}-q{quantum}");
            let input = folder.join(format!("{name}.pgm"));
            pgm(&input, w, h, &q)?;
            let encoded = folder.join(format!("{name}.jxl"));
            let restored = folder.join(format!("{name}-decoded.pgm"));
            let encode_ms = encode(bin, &input, &encoded, distance, true)?;
            let decode_ms = decode(bin, &encoded, &restored, true)?;
            let decoded = read_pgm(&restored)?;
            ensure!(decoded.len() == original.len(), "復元した画素数が違います");
            if distance == 0. {
                ensure!(q == decoded, "可逆 JPEG XL で画素が変わりました");
            }
            let values: Vec<f32> = decoded
                .iter()
                .zip(&original)
                .map(|(q, v)| {
                    if v.is_finite() {
                        min + *q as f32 * scale
                    } else {
                        f32::NAN
                    }
                })
                .collect();
            let (max_error, rmse) = error(&original, &values)?;
            let header = json!({"width":w,"height":h,"min":min,"scale":scale,"jxlBytes":fs::metadata(&encoded)?.len(),"maskBytes":mask_zstd.len()});
            let data = packet(b"YJH1", &header, &[&fs::read(&encoded)?, &mask_zstd])?;
            fs::write(folder.join(format!("{name}.yjh")), &data)?;
            results.push(json!({"kind":"height","level":level,"distance":distance,"quantumMeters":quantum,"bytes":data.len(),"jxlBytes":fs::metadata(&encoded)?.len(),"pngBytes":meta["heightBytes"],"maxErrorMeters":max_error,"rmseMeters":rmse,"encodeMs":encode_ms,"decodeProcessMs":decode_ms,"file":format!("{name}.yjh"),"pixelExact":distance==0.&&quantum==1.,"layout":"whole-region"}));
            save_report(&folder, &results, false)?;
            println!("{}", results.last().unwrap());
        }
    }
    results.extend(tiled_heights(root, &folder, bin)?);
    for (name, input, rgb) in [
        ("shared", root.join("normals/1024.png"), false),
        (
            "baked-L9",
            output.join("baked-normals/L9/source/normals/1024.png"),
            true,
        ),
        (
            "baked-L10",
            output.join("baked-normals/L10/source/normals/1024.png"),
            true,
        ),
    ] {
        let original = bench_png::read(&input)?;
        for distance in [0., 0.1, 0.2, 0.25, 0.3, 0.4, 0.5, 1.] {
            let name = format!("normal-{name}-d{distance}");
            let encoded = folder.join(format!("{name}.jxl"));
            let restored = folder.join(format!("{name}-decoded.png"));
            let encode_ms = encode(bin, &input, &encoded, distance, false)?;
            let decode_ms = decode(bin, &encoded, &restored, false)?;
            let decoded = bench_png::read(&restored)?;
            if distance == 0. {
                ensure!(
                    original.pixels == decoded.pixels,
                    "可逆 JPEG XL で法線の画素が変わりました"
                );
            }
            let angular = bench_normal::compare_angles(&original.pixels, &decoded.pixels, rgb)?;
            results.push(json!({"kind":"normal","name":name,"distance":distance,"size":1024,"bytes":fs::metadata(&encoded)?.len(),"pngBytes":fs::metadata(&input)?.len(),"angularError":angular,"encodeMs":encode_ms,"decodeProcessMs":decode_ms,"file":format!("{name}.jxl"),"pixelExact":distance==0.}));
            save_report(&folder, &results, false)?;
            println!("{}", results.last().unwrap());
        }
    }
    save_report(&folder, &results, true)
}
fn tiled_heights(root: &Path, folder: &Path, bin: &Path) -> Result<Vec<Value>> {
    let (meta, original) = bench_png::heights(root, 14)?;
    let w = meta["width"].as_u64().unwrap() as usize;
    let h = meta["height"].as_u64().unwrap() as usize;
    let mask: Vec<u8> = original.iter().map(|v| v.is_finite() as u8).collect();
    let mask_zstd = zstd::stream::encode_all(Cursor::new(mask), 9)?;
    let input = folder.join("tile-input.pgm");
    let encoded = folder.join("tile-encoded.jxl");
    let restored = folder.join("tile-decoded.pgm");
    let mut results = Vec::new();
    for distance in [0., 0.5] {
        let mut tiles = Vec::new();
        let mut body = Vec::new();
        let mut values = vec![f32::NAN; original.len()];
        let mut encode_ms = 0.;
        let mut decode_ms = 0.;
        for y in (0..h).step_by(256) {
            for x in (0..w).step_by(256) {
                let tw = (w - x).min(256);
                let th = (h - y).min(256);
                let mut samples = Vec::with_capacity(tw * th);
                for sy in 0..th {
                    samples.extend_from_slice(&original[(y + sy) * w + x..(y + sy) * w + x + tw]);
                }
                let min = samples
                    .iter()
                    .copied()
                    .filter(|v| v.is_finite())
                    .fold(f32::INFINITY, f32::min);
                let max = samples
                    .iter()
                    .copied()
                    .filter(|v| v.is_finite())
                    .fold(f32::NEG_INFINITY, f32::max);
                let min = if min.is_finite() { min } else { 0. };
                let scale = if distance == 0. {
                    1.
                } else {
                    (max - min).max(1.) / 65535.
                };
                let q: Vec<u16> = samples
                    .iter()
                    .map(|v| {
                        if v.is_finite() {
                            ((v - min) / scale).round().clamp(0., 65535.) as u16
                        } else {
                            0
                        }
                    })
                    .collect();
                pgm(&input, tw, th, &q)?;
                encode_ms += encode(bin, &input, &encoded, distance, true)?;
                decode_ms += decode(bin, &encoded, &restored, true)?;
                let decoded = read_pgm(&restored)?;
                ensure!(decoded.len() == q.len(), "タイルの寸法が変わりました");
                if distance == 0. {
                    ensure!(decoded == q, "可逆タイルの標高が変わりました");
                }
                for sy in 0..th {
                    for sx in 0..tw {
                        let i = (y + sy) * w + x + sx;
                        if original[i].is_finite() {
                            values[i] = min + decoded[sy * tw + sx] as f32 * scale;
                        }
                    }
                }
                let data = fs::read(&encoded)?;
                tiles.push(json!({"x":x,"y":y,"width":tw,"height":th,"min":min,"scale":scale,"bytes":data.len()}));
                body.extend(data);
            }
        }
        let (max_error, rmse) = error(&original, &values)?;
        let data = packet(
            b"YJH1",
            &json!({"width":w,"height":h,"tiles":tiles,"maskBytes":mask_zstd.len()}),
            &[&body, &mask_zstd],
        )?;
        let name = format!("height-L14-tile256-d{distance}.yjh");
        fs::write(folder.join(&name), &data)?;
        let row = json!({"kind":"height","level":14,"layout":"tile256","distance":distance,"quantumMeters":if distance==0.{1.}else{0.},"bytes":data.len(),"pngBytes":meta["heightBytes"],"maxErrorMeters":max_error,"rmseMeters":rmse,"encodeMs":encode_ms,"decodeProcessMs":decode_ms,"file":name,"pixelExact":distance==0.});
        println!("{row}");
        results.push(row);
    }
    Ok(results)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pgm_header_does_not_eat_binary_whitespace() {
        let p = std::env::temp_dir().join(format!("terrain-jxl-pgm-{}.pgm", std::process::id()));
        let q = vec![0x0a20, 0x0dff, 65535, 0];
        pgm(&p, 2, 2, &q).unwrap();
        assert_eq!(read_pgm(&p).unwrap(), q);
        fs::remove_file(p).unwrap();
    }
}
