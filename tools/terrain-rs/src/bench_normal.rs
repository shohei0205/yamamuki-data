use crate::{bench_png, write_json};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs, io::Cursor, path::Path, process::Command, time::Instant};
fn direction(p: [u8; 4]) -> [f64; 3] {
    let x = p[0] as f64 / 127.5 - 1.;
    let y = p[1] as f64 / 127.5 - 1.;
    let z = (1. - x * x - y * y).max(0.).sqrt();
    let n = (x * x + y * y + z * z).sqrt();
    [x / n, y / n, z / n]
}
fn angles_with(a: &[[u8; 4]], b: &[[u8; 4]], decode: fn([u8; 4]) -> [f64; 3]) -> Result<Value> {
    ensure!(a.len() == b.len(), "法線の画素数が違います");
    let mut angles = Vec::with_capacity(a.len());
    let mut sum = 0.;
    let mut sq = 0.;
    let mut over = 0;
    for (&a, &b) in a.iter().zip(b) {
        let u = decode(a);
        let v = decode(b);
        let dot = u.iter().zip(v).map(|(x, y)| x * y).sum::<f64>();
        let e = dot.clamp(-1., 1.).acos().to_degrees();
        angles.push(e as f32);
        sum += e;
        sq += e * e;
        if e > 5. {
            over += 1;
        }
    }
    angles.sort_unstable_by(f32::total_cmp);
    let n = angles.len();
    Ok(
        json!({"meanDegrees":sum/n as f64,"rmsDegrees":(sq/n as f64).sqrt(),"p95Degrees":angles[((n-1) as f64*0.95) as usize],"p99Degrees":angles[((n-1) as f64*0.99) as usize],"maxDegrees":angles[n-1],"over5DegreesPercent":100.*over as f64/n as f64}),
    )
}
fn direction_rgb(p: [u8; 4]) -> [f64; 3] {
    let n = [0, 1, 2].map(|i| p[i] as f64 / 127.5 - 1.);
    let length = n.iter().map(|x| x * x).sum::<f64>().sqrt();
    n.map(|x| x / length)
}
fn angles(a: &[[u8; 4]], b: &[[u8; 4]]) -> Result<Value> {
    angles_with(a, b, direction)
}
pub(crate) fn compare_angles(a: &[[u8; 4]], b: &[[u8; 4]], rgb: bool) -> Result<Value> {
    angles_with(a, b, if rgb { direction_rgb } else { direction })
}
fn ktx2(astc: &[u8], block: u32, width: u32, height: u32, zstd: bool) -> Result<Vec<u8>> {
    ensure!(
        astc.len() >= 16 && astc[..4] == [0x13, 0xab, 0xa1, 0x5c],
        "ASTC ヘッダーが不正です"
    );
    let raw = &astc[16..];
    let body = if zstd {
        zstd::stream::encode_all(Cursor::new(raw), 9)?
    } else {
        raw.to_vec()
    };
    let vk = match block {
        4 => 157,
        6 => 165,
        8 => 171,
        12 => 183,
        _ => anyhow::bail!("ASTC のブロック寸法が不正です"),
    };
    // KTX2 の単一 2D レベル。DFD は Khronos の ASTC・線形 UNORM の定義。
    let mut out = Vec::new();
    out.extend([
        0xab, 0x4b, 0x54, 0x58, 0x20, 0x32, 0x30, 0xbb, 0x0d, 0x0a, 0x1a, 0x0a,
    ]);
    for v in [
        vk,
        1,
        width,
        height,
        0,
        0,
        1,
        1,
        if zstd { 2 } else { 0 },
        104,
        44,
        0,
        0,
    ] {
        out.extend(v.to_le_bytes());
    }
    out.extend(0u64.to_le_bytes());
    out.extend(0u64.to_le_bytes());
    out.extend(160u64.to_le_bytes());
    out.extend((body.len() as u64).to_le_bytes());
    out.extend((raw.len() as u64).to_le_bytes());
    for v in [
        44u32,
        0,
        (40 << 16) | 2,
        162 | (1 << 8) | (1 << 16),
        (block - 1) | ((block - 1) << 8),
        16,
        0,
        127 << 16,
        0,
        0,
        u32::MAX,
    ] {
        out.extend(v.to_le_bytes());
    }
    ensure!(out.len() == 148, "KTX2 のデータ記述の寸法が不正です");
    out.resize(160, 0);
    out.extend(body);
    let reconstructed = if zstd {
        zstd::stream::decode_all(Cursor::new(&out[160..]))?
    } else {
        out[160..].to_vec()
    };
    ensure!(
        reconstructed == raw,
        "KTX2 の格納で ASTC のデータが変わりました"
    );
    Ok(out)
}
pub fn run(root: &Path, output: &Path, encoder: &Path) -> Result<Value> {
    run_mode(root, output, encoder, false)
}
pub fn run_tangent(root: &Path, output: &Path, encoder: &Path) -> Result<Value> {
    run_mode(root, output, encoder, true)
}
fn run_mode(root: &Path, output: &Path, encoder: &Path, rgb: bool) -> Result<Value> {
    let report: Value = serde_json::from_slice(&fs::read(root.join("levels.json"))?)?;
    let dir = output.join("normals");
    fs::create_dir_all(&dir)?;
    let mut results = Vec::new();
    for variant in report["normalVariants"].as_array().unwrap() {
        let size = variant["size"].as_u64().unwrap() as usize;
        let source = bench_png::read(&root.join(format!("normals/{size}.png")))?;
        let xyz: Vec<_> = source
            .pixels
            .iter()
            .map(|p| {
                if rgb {
                    return *p;
                }
                let n = direction(*p);
                [p[0], p[1], ((n[2] + 1.) * 127.5).round() as u8, 255]
            })
            .collect();
        let input = dir.join(format!("input-{size}-xyz.png"));
        bench_png::write_encoder_input(&input, size, size, &xyz)?;
        for block in [4u32, 6, 8] {
            let base = format!("normal-{size}-{block}");
            let astc = dir.join(format!("{base}.astc"));
            let decoded = dir.join(format!("{base}-decoded.png"));
            let t = Instant::now();
            let status = Command::new(encoder)
                .args(["-cl"])
                .arg(&input)
                .arg(&astc)
                .args([
                    &format!("{block}x{block}"),
                    "-thorough",
                    "-j",
                    "4",
                    "-decode_unorm8",
                    "-silent",
                ])
                .args(if rgb {
                    vec!["-esw", "rgb1"]
                } else {
                    vec!["-normal", "-esw", "rrrg"]
                })
                .status()?;
            ensure!(status.success(), "ASTC の符号化に失敗しました");
            let encode_ms = t.elapsed().as_secs_f64() * 1000.;
            let t = Instant::now();
            let status = Command::new(encoder)
                .arg("-dl")
                .arg(&astc)
                .arg(&decoded)
                .args([
                    "-dsw",
                    if rgb { "rgb1" } else { "raz1" },
                    "-j",
                    "4",
                    "-silent",
                ])
                .status()?;
            ensure!(status.success(), "ASTC の復元に失敗しました");
            let decode_ms = t.elapsed().as_secs_f64() * 1000.;
            let restored = bench_png::read(&decoded)?;
            let angular = if rgb {
                angles_with(&source.pixels, &restored.pixels, direction_rgb)?
            } else {
                angles(&source.pixels, &restored.pixels)?
            };
            let data = fs::read(&astc)?;
            let plain = ktx2(&data, block, size as u32, size as u32, false)?;
            let compressed = ktx2(&data, block, size as u32, size as u32, true)?;
            fs::write(dir.join(format!("{base}.ktx2")), &plain)?;
            fs::write(dir.join(format!("{base}-zstd.ktx2")), &compressed)?;
            results.push(json!({"size":size,"block":block,"pngBytes":variant["bytes"],"astcBytes":data.len(),"gpuPayloadBytes":data.len()-16,"ktx2Bytes":plain.len(),"ktx2ZstdBytes":compressed.len(),"encodeProcessMs":encode_ms,"decodeProcessMs":decode_ms,"angularError":angular,"swizzle":if rgb {"rgb"} else {"ra"},"url":format!("/codecs/normals/{base}.ktx2"),"zstdUrl":format!("/codecs/normals/{base}-zstd.ktx2"),"previewUrl":format!("/codecs/normals/{base}-decoded.png")}));
            println!(
                "{}",
                json!({"part":"normals","size":size,"block":block,"state":"complete","angle":angular})
            );
            write_json(
                &output.join("normals.partial.json"),
                &json!({"results":results}),
            )?;
        }
    }
    Ok(
        json!({"results":results,"encoder":if rgb {"Arm astcenc 5.7.0 -thorough -esw rgb1 -decode_unorm8; 4 threads"} else {"Arm astcenc 5.7.0 -thorough -normal -esw rrrg -decode_unorm8; 4 threads"},"profile":if rgb {"linear UNORM; signed tangent XYZ in RGB"} else {"linear UNORM; XY stored in RA"},"timing":"外部プロセス起動・PNG 入出力を含む。ほかのコーデックのメモリー内復元時間と直接比較しない。","ktx2":"単一 2D レベル。ミップなし。格納後に ASTC 本体が一致することを確認。"}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_normals_have_zero_angular_error() {
        let p = vec![[128, 128, 255, 255], [90, 160, 240, 255]];
        assert!(angles(&p, &p).unwrap()["maxDegrees"].as_f64().unwrap() < 0.00001);
    }
    #[test]
    fn ktx2_preserves_astc_payload() {
        let mut a = vec![0; 32];
        a[..4].copy_from_slice(&[0x13, 0xab, 0xa1, 0x5c]);
        for z in [false, true] {
            let k = ktx2(&a, 4, 4, 4, z).unwrap();
            assert_eq!(u32::from_le_bytes(k[12..16].try_into().unwrap()), 157);
        }
    }
}
