use crate::{bench_png, codec, error, median_ms, packet, unpacket, write_json};
use anyhow::{Result, ensure};
use flate2::{Compression, write::GzEncoder};
use libloading::Library;
use serde_json::{Value, json};
use std::{
    ffi::c_void,
    fs,
    io::{Cursor, Write},
    path::Path,
    time::Instant,
};
type Size = unsafe extern "C" fn(
    *const c_void,
    u32,
    i32,
    i32,
    i32,
    i32,
    i32,
    *const u8,
    f64,
    *mut u32,
) -> u32;
type Encode = unsafe extern "C" fn(
    *const c_void,
    u32,
    i32,
    i32,
    i32,
    i32,
    i32,
    *const u8,
    f64,
    *mut u8,
    u32,
    *mut u32,
) -> u32;
type Decode =
    unsafe extern "C" fn(*const u8, u32, i32, *mut u8, i32, i32, i32, i32, u32, *mut c_void) -> u32;
struct Lerc {
    _lib: Library,
    size: Size,
    encode: Encode,
    decode: Decode,
}
impl Lerc {
    fn open(path: &Path) -> Result<Self> {
        unsafe {
            let lib = Library::new(path)?;
            let size = *lib.get(b"lerc_computeCompressedSize\0")?;
            let encode = *lib.get(b"lerc_encode\0")?;
            let decode = *lib.get(b"lerc_decode\0")?;
            Ok(Self {
                _lib: lib,
                size,
                encode,
                decode,
            })
        }
    }
    fn encode(&self, values: &[f32], w: usize, h: usize, tolerance: f64) -> Result<Vec<u8>> {
        let mask: Vec<u8> = values.iter().map(|x| u8::from(x.is_finite())).collect();
        let data: Vec<f32> = values
            .iter()
            .map(|x| if x.is_finite() { *x } else { 0. })
            .collect();
        let mut n = 0;
        unsafe {
            ensure!(
                (self.size)(
                    data.as_ptr().cast(),
                    6,
                    1,
                    w as i32,
                    h as i32,
                    1,
                    1,
                    mask.as_ptr(),
                    tolerance,
                    &mut n
                ) == 0,
                "LERC の容量計算に失敗しました"
            );
        }
        let mut out = vec![0; n as usize];
        unsafe {
            ensure!(
                (self.encode)(
                    data.as_ptr().cast(),
                    6,
                    1,
                    w as i32,
                    h as i32,
                    1,
                    1,
                    mask.as_ptr(),
                    tolerance,
                    out.as_mut_ptr(),
                    n,
                    &mut n
                ) == 0,
                "LERC の符号化に失敗しました"
            );
        }
        out.truncate(n as usize);
        Ok(out)
    }
    fn decode(&self, data: &[u8], w: usize, h: usize) -> Result<Vec<f32>> {
        let mut out = vec![0f32; w * h];
        let mut mask = vec![0; w * h];
        unsafe {
            ensure!(
                (self.decode)(
                    data.as_ptr(),
                    data.len() as u32,
                    1,
                    mask.as_mut_ptr(),
                    1,
                    w as i32,
                    h as i32,
                    1,
                    6,
                    out.as_mut_ptr().cast()
                ) == 0,
                "LERC の復元に失敗しました"
            );
        }
        for (i, v) in out.iter_mut().enumerate() {
            if mask[i] == 0 {
                *v = f32::NAN;
            }
        }
        Ok(out)
    }
}
fn ymd(values: &[f32], w: usize, h: usize, step: f32, xz: bool) -> Result<Vec<u8>> {
    let base = values
        .iter()
        .filter(|v| v.is_finite())
        .map(|v| (v / step).round() as i32)
        .min()
        .unwrap_or(0);
    let q: Vec<i32> = values
        .iter()
        .map(|v| {
            if v.is_finite() {
                (v / step).round() as i32 - base
            } else {
                0
            }
        })
        .collect();
    let mut delta = Vec::with_capacity(q.len());
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            delta.push(
                q[i] - if x > 0 { q[i - 1] } else { 0 } - if y > 0 { q[i - w] } else { 0 }
                    + if x > 0 && y > 0 { q[i - w - 1] } else { 0 },
            );
        }
    }
    let wide = delta.iter().any(|&v| !(-32768..=32767).contains(&v));
    let missing = values.iter().any(|v| !v.is_finite());
    let mut raw = Vec::new();
    for d in delta {
        if wide {
            raw.extend(d.to_le_bytes())
        } else {
            raw.extend((d as i16).to_le_bytes())
        }
    }
    if missing {
        let mut mask = vec![0u8; values.len().div_ceil(8)];
        for (i, v) in values.iter().enumerate() {
            if !v.is_finite() {
                mask[i / 8] |= 1 << (i % 8);
            }
        }
        raw.extend(mask);
    }
    let body = if xz {
        let mut out = Vec::new();
        {
            let mut writer =
                lzma_rust2::XzWriter::new(&mut out, lzma_rust2::XzOptions::with_preset(6))?;
            writer.write_all(&raw)?;
            writer.finish()?;
        }
        out
    } else {
        let mut writer = GzEncoder::new(Vec::new(), Compression::best());
        writer.write_all(&raw)?;
        writer.finish()?
    };
    let mut out = Vec::new();
    out.extend(b"YMD1");
    out.extend((h as u16).to_le_bytes());
    out.extend((w as u16).to_le_bytes());
    out.extend(step.to_le_bytes());
    out.extend(base.to_le_bytes());
    out.push(u8::from(wide) | (u8::from(missing) << 1));
    out.push(u8::from(xz));
    out.extend(body);
    Ok(out)
}
fn pack_tiles(values: &[f32], w: usize, h: usize, name: &str, lerc: &Lerc) -> Result<Vec<u8>> {
    let mut chunks = Vec::new();
    let mut descriptors = Vec::new();
    for y in (0..h).step_by(256) {
        for x in (0..w).step_by(256) {
            let tw = 256.min(w - x);
            let th = 256.min(h - y);
            let mut tile = Vec::with_capacity(tw * th);
            for yy in y..y + th {
                tile.extend_from_slice(&values[yy * w + x..yy * w + x + tw]);
            }
            let data = match name {
                "ymd-xz-1" => ymd(&tile, tw, th, 1., true)?,
                "ymd-xz-5" => ymd(&tile, tw, th, 5., true)?,
                "ymd-gzip-1" => ymd(&tile, tw, th, 1., false)?,
                _ => lerc.encode(
                    &tile,
                    tw,
                    th,
                    name.strip_prefix("lerc-")
                        .unwrap()
                        .replace('p', ".")
                        .parse()?,
                )?,
            };
            descriptors.push(json!({"x":x,"y":y,"width":tw,"height":th,"bytes":data.len()}));
            chunks.push(data);
        }
    }
    packet(
        b"YHB1",
        &json!({"width":w,"height":h,"codec":if name.starts_with("ymd"){"ymd"}else{"lerc"},"tiles":descriptors}),
        &chunks.iter().map(|b| b.as_slice()).collect::<Vec<_>>(),
    )
}
fn unpack_tiles(bytes: &[u8], lerc: &Lerc) -> Result<Vec<f32>> {
    let (header, body) = unpacket(bytes, b"YHB1")?;
    let w = header["width"].as_u64().unwrap() as usize;
    let h = header["height"].as_u64().unwrap() as usize;
    let mut out = vec![f32::NAN; w * h];
    let mut p = 0;
    for tile in header["tiles"].as_array().unwrap() {
        let x = tile["x"].as_u64().unwrap() as usize;
        let y = tile["y"].as_u64().unwrap() as usize;
        let tw = tile["width"].as_u64().unwrap() as usize;
        let th = tile["height"].as_u64().unwrap() as usize;
        let n = tile["bytes"].as_u64().unwrap() as usize;
        ensure!(p + n <= body.len(), "標高タイルが不足しています");
        let data = if header["codec"] == "ymd" {
            codec::decode_grid(&body[p..p + n])?.values
        } else {
            lerc.decode(&body[p..p + n], tw, th)?
        };
        p += n;
        for yy in 0..th {
            out[(y + yy) * w + x..(y + yy) * w + x + tw]
                .copy_from_slice(&data[yy * tw..(yy + 1) * tw]);
        }
    }
    ensure!(p == body.len(), "標高タイルの余分なデータがあります");
    Ok(out)
}
pub fn prediction(
    parent: &[f32],
    pw: usize,
    ph: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
) -> i32 {
    let sx = x as f64 * (pw - 1) as f64 / (w - 1) as f64;
    let sy = y as f64 * (ph - 1) as f64 / (h - 1) as f64;
    let x0 = sx.floor() as usize;
    let y0 = sy.floor() as usize;
    let fx = sx - x0 as f64;
    let fy = sy - y0 as f64;
    let mut sum = 0.;
    let mut weight = 0.;
    for (dx, dy, k) in [
        (0, 0, (1. - fx) * (1. - fy)),
        (1, 0, fx * (1. - fy)),
        (0, 1, (1. - fx) * fy),
        (1, 1, fx * fy),
    ] {
        let v = parent[(y0 + dy).min(ph - 1) * pw + (x0 + dx).min(pw - 1)];
        if v.is_finite() {
            sum += v as f64 * k;
            weight += k;
        }
    }
    if weight > 0. {
        (sum / weight).round() as i32
    } else {
        0
    }
}
fn residual(
    values: &[f32],
    w: usize,
    h: usize,
    parent: Option<(&[f32], usize, usize)>,
    level: u32,
) -> Result<Vec<u8>> {
    let q: Vec<i32> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if v.is_finite() {
                v.round() as i32
                    - parent.map_or(0, |(p, pw, ph)| prediction(p, pw, ph, i % w, i / w, w, h))
            } else {
                0
            }
        })
        .collect();
    let mut raw = Vec::with_capacity(values.len() * 5);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let d = q[i] - if x > 0 { q[i - 1] } else { 0 } - if y > 0 { q[i - w] } else { 0 }
                + if x > 0 && y > 0 { q[i - w - 1] } else { 0 };
            raw.extend(d.to_le_bytes());
        }
    }
    let missing = values.iter().any(|v| !v.is_finite());
    if missing {
        raw.extend(values.iter().map(|v| u8::from(v.is_finite())));
    }
    let body = zstd::stream::encode_all(Cursor::new(raw), 9)?;
    packet(
        b"YHR1",
        &json!({"width":w,"height":h,"level":level,"parentLevel":if parent.is_some(){Some(level-1)}else{None},"missing":missing,"encoding":"delta2d-i32-zstd","stepMeters":1}),
        &[&body],
    )
}
fn decode_residual(bytes: &[u8], parent: Option<(&[f32], usize, usize)>) -> Result<Vec<f32>> {
    let (header, body) = unpacket(bytes, b"YHR1")?;
    let w = header["width"].as_u64().unwrap() as usize;
    let h = header["height"].as_u64().unwrap() as usize;
    let n = w * h;
    let raw = zstd::stream::decode_all(Cursor::new(body))?;
    ensure!(
        raw.len() == n * 4 + if header["missing"] == true { n } else { 0 },
        "階層差分の長さが不正です"
    );
    let mut q = vec![0i32; n];
    let mut out = vec![f32::NAN; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let d = i32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into()?);
            q[i] = d + if x > 0 { q[i - 1] } else { 0 } + if y > 0 { q[i - w] } else { 0 }
                - if x > 0 && y > 0 { q[i - w - 1] } else { 0 };
            if header["missing"] != true || raw[n * 4 + i] != 0 {
                out[i] = (q[i] + parent.map_or(0, |(p, pw, ph)| prediction(p, pw, ph, x, y, w, h)))
                    as f32;
            }
        }
    }
    Ok(out)
}
pub fn run(root: &Path, output: &Path, dll: &Path) -> Result<Value> {
    let lerc = Lerc::open(dll)?;
    let dir = output.join("heights");
    fs::create_dir_all(&dir)?;
    let mut results = Vec::new();
    let mut parent: Option<(Vec<f32>, usize, usize)> = None;
    let mut dependency_bytes = 0usize;
    let mut decode_chain_ms = 0.;
    for level in 4..=14 {
        let (meta, values) = bench_png::heights(root, level)?;
        let w = meta["width"].as_u64().unwrap() as usize;
        let h = meta["height"].as_u64().unwrap() as usize;
        for name in [
            "ymd-xz-1",
            "ymd-xz-5",
            "ymd-gzip-1",
            "lerc-0",
            "lerc-1",
            "lerc-2p5",
            "lerc-5",
        ] {
            let t = Instant::now();
            let data = pack_tiles(&values, w, h, name, &lerc)?;
            let encode_ms = t.elapsed().as_secs_f64() * 1000.;
            let (decoded, decode_ms) = median_ms(|| unpack_tiles(&data, &lerc))?;
            let (max, rmse) = error(&values, &decoded)?;
            let allowed = match name {
                "ymd-xz-5" => 2.5,
                "lerc-1" => 1.,
                "lerc-2p5" => 2.5,
                "lerc-5" => 5.,
                _ => 0.,
            };
            ensure!(
                max <= allowed + 0.001,
                "{name} L{level} の誤差 {max} が上限を超えました"
            );
            let file = format!("L{level}-{name}.yhb");
            fs::write(dir.join(&file), &data)?;
            results.push(json!({"level":level,"codec":name,"bytes":data.len(),"pngBytes":meta["heightBytes"],"maxErrorMeters":max,"rmseMeters":rmse,"allowedAdditionalErrorMeters":allowed,"encodeMs":encode_ms,"decodeMsMedian":decode_ms,"url":format!("/codecs/heights/{file}")}));
        }
        let reference = parent.as_ref().map(|(p, pw, ph)| (p.as_slice(), *pw, *ph));
        for name in ["delta-zstd", "residual-zstd"] {
            let p = if name == "residual-zstd" {
                reference
            } else {
                None
            };
            let t = Instant::now();
            let data = residual(&values, w, h, p, level)?;
            let encode_ms = t.elapsed().as_secs_f64() * 1000.;
            let (decoded, decode_ms) = median_ms(|| decode_residual(&data, p))?;
            let (max, rmse) = error(&values, &decoded)?;
            ensure!(max <= 0.001, "階層差分の復元で標高が変わりました");
            let file = format!("L{level}-{name}.yhr");
            fs::write(dir.join(&file), &data)?;
            if name == "residual-zstd" {
                dependency_bytes += data.len();
                decode_chain_ms += decode_ms;
            }
            results.push(json!({"level":level,"codec":name,"bytes":data.len(),"requiredBytes":if name=="residual-zstd"{dependency_bytes}else{data.len()},"decodeChainMs":if name=="residual-zstd"{decode_chain_ms}else{decode_ms},"pngBytes":meta["heightBytes"],"maxErrorMeters":max,"rmseMeters":rmse,"encodeMs":encode_ms,"decodeMsMedian":decode_ms,"parentLevel":if p.is_some(){Some(level-1)}else{None},"url":format!("/codecs/heights/{file}")}));
        }
        parent = Some((values, w, h));
        println!(
            "{}",
            json!({"part":"heights","level":level,"state":"complete"})
        );
        write_json(
            &output.join("heights.partial.json"),
            &json!({"results":results}),
        )?;
    }
    Ok(
        json!({"results":results,"source":"各 PNG レベルの標高。元の DEM の量子化・縮小誤差とは別に、追加誤差を測定。","decodeTiming":"ディスク読み込みを除く 3 回の中央値。階層差分は親からの合計も表示。","lercVersion":"4.2.0"}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn residual_roundtrip_keeps_missing_and_height() {
        let p = vec![1., 2., 3., 4.];
        let v = vec![1., 2., 3., 4., f32::NAN, 6., 7., 8., 9.];
        let b = residual(&v, 3, 3, Some((&p, 2, 2)), 5).unwrap();
        let d = decode_residual(&b, Some((&p, 2, 2))).unwrap();
        assert_eq!(error(&v, &d).unwrap().0, 0.);
    }
    #[test]
    fn ymd_roundtrip() {
        for xz in [true, false] {
            let v = vec![-15., 0., 50., f32::NAN, 200., 1000.];
            let b = ymd(&v, 3, 2, 1., xz).unwrap();
            assert_eq!(
                error(&v, &codec::decode_grid(&b).unwrap().values)
                    .unwrap()
                    .0,
                0.
            );
        }
    }
}
