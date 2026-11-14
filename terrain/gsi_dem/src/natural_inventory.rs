//! 入力PNGを復号・監査し、再開可能な全国作業DBへ固定する。
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, io::Cursor, path::Path, time::Duration};
pub fn hash(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
pub fn progress(output: &Path, value: Value) -> Result<()> {
    let tmp = output.join("progress.next.json");
    fs::write(&tmp, serde_json::to_vec_pretty(&value)?)?;
    for attempt in 0..40 {
        match fs::rename(&tmp, output.join("progress.json")) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 39 && matches!(e.raw_os_error(), Some(5 | 32 | 33)) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}
pub fn decode(data: &[u8]) -> Result<Vec<f64>> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    ensure!(
        reader.info().width == 256 && reader.info().height == 256,
        "PNGは256×256が必要です"
    );
    let mut out = vec![0; reader.output_buffer_size().context("PNGサイズ不明")?];
    let info = reader.next_frame(&mut out)?;
    ensure!(
        info.bit_depth == png::BitDepth::Eight,
        "PNGのビット深度が不正です"
    );
    let stride = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        _ => anyhow::bail!("標高PNGがRGB/RGBAではありません"),
    };
    let mut z = Vec::with_capacity(65536);
    for p in out[..info.buffer_size()].chunks_exact(stride) {
        let code = ((p[0] as i32) << 16) | ((p[1] as i32) << 8) | p[2] as i32;
        z.push(if code == 0x800000 || stride == 4 && p[3] == 0 {
            f64::NAN
        } else {
            (if code >= 0x800000 {
                code - 0x1000000
            } else {
                code
            }) as f64
                * 0.01
        });
    }
    ensure!(z.len() == 65536, "PNG画素数が不正です");
    Ok(z)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_signed_and_nodata() {
        let mut b = Vec::new();
        {
            let mut e = png::Encoder::new(&mut b, 256, 256);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            let mut w = e.write_header().unwrap();
            let mut rgb = vec![0u8; 256 * 256 * 3];
            rgb[..9].copy_from_slice(&[0, 0, 100, 255, 255, 156, 128, 0, 0]);
            w.write_image_data(&rgb).unwrap();
        }
        let z = decode(&b).unwrap();
        assert_eq!(z[0], 1.);
        assert_eq!(z[1], -1.);
        assert!(z[2].is_nan());
    }
    #[test]
    fn bad_png_is_not_empty_ocean() {
        assert!(decode(b"bad").is_err());
    }
}

pub fn hash_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        h.update(&buffer[..n]);
    }
    Ok(h.finalize().iter().map(|v| format!("{v:02x}")).collect())
}
