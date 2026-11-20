//! 標高をタイル単位の8bit階調へ量子化し、欠損マスク付きWebPで非可逆圧縮する。
use crate::heightmap::{HEADER, Heightmap, MISSING};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
pub fn encode(
    cells: u32,
    minimum_step: f64,
    heights: &[Option<f64>],
    quality: u32,
) -> Result<(Heightmap, Vec<u8>)> {
    ensure!(quality <= 100, "WebP品質は0〜100が必要です");
    ensure!(
        minimum_step.is_finite() && (0.001..=100.).contains(&minimum_step),
        "標高量子化幅が不正です"
    );
    ensure!(
        heights.iter().flatten().all(|v| v.is_finite()),
        "非有限の標高は保存できません"
    );
    let low = heights
        .iter()
        .flatten()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let high = heights
        .iter()
        .flatten()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    // 基準値の切捨て分も含めて0〜255へ収める。255も有効値で、欠損はalphaに保存する。
    let step = if low.is_finite() {
        minimum_step.max((high - low) / 254.)
    } else {
        minimum_step
    };
    let map = Heightmap::quantize(cells, step, heights)?;
    ensure!(
        map.values.iter().all(|v| *v == MISSING || *v <= 255),
        "標高差が8bitの範囲を超えます"
    );
    let packed = encode_pixels(&map, quality)?;
    let mut blob = b"HMW1".to_vec();
    for v in [map.cells, map.halo, map.values.len() as u32] {
        blob.extend(v.to_le_bytes());
    }
    blob.extend(map.base.to_le_bytes());
    blob.extend(map.step.to_le_bytes());
    blob.extend(((map.values.len() * 4) as u32).to_le_bytes());
    blob.extend((packed.len() as u32).to_le_bytes());
    blob.extend(Sha256::digest(&packed));
    blob.extend(quality.to_le_bytes());
    blob.extend(packed);
    // 誤差評価には圧縮前でなく、実際に配布・復号する標高を使う。
    Ok((decode(&blob)?, blob))
}
fn encode_pixels(map: &Heightmap, quality: u32) -> Result<Vec<u8>> {
    let rgba = map
        .values
        .iter()
        .flat_map(|v| {
            if *v == MISSING {
                [0, 0, 0, 0]
            } else {
                [*v as u8, *v as u8, *v as u8, 255]
            }
        })
        .collect::<Vec<_>>();
    let side = map.width() as i32;
    let mut output = std::ptr::null_mut();
    let len = unsafe {
        libwebp_sys::WebPEncodeRGBA(
            rgba.as_ptr(),
            side,
            side,
            side * 4,
            quality as f32,
            &mut output,
        )
    };
    ensure!(len > 0 && !output.is_null(), "WebP圧縮に失敗しました");
    let packed = unsafe {
        let b = std::slice::from_raw_parts(output, len).to_vec();
        libwebp_sys::WebPFree(output.cast());
        b
    };
    Ok(packed)
}
pub fn decode(blob: &[u8]) -> Result<Heightmap> {
    if blob.starts_with(b"HMW2") {
        return decode_minmax(blob);
    }
    ensure!(
        blob.len() >= HEADER && blob.len() <= 2 * 1024 * 1024 && &blob[..4] == b"HMW1",
        "WebPハイトマップの形式・長さが不正です"
    );
    let u = |i| u32::from_le_bytes(blob[i..i + 4].try_into().unwrap());
    let cells = u(4);
    let halo = u(8);
    ensure!(
        (8..=256).contains(&cells) && cells.is_power_of_two() && halo == 1,
        "格子寸法が不正です"
    );
    let side = cells + 3;
    let count = side.pow(2) as usize;
    let body = &blob[HEADER..];
    ensure!(
        u(12) as usize == count
            && u(28) as usize == count * 4
            && u(32) as usize == body.len()
            && u(68) <= 100,
        "WebPハイトマップのヘッダーが不正です"
    );
    ensure!(
        Sha256::digest(body).as_slice() == &blob[36..68],
        "WebP本文のSHA-256が不一致です"
    );
    ensure!(
        body.len() >= 12
            && &body[..4] == b"RIFF"
            && &body[8..12] == b"WEBP"
            && u32::from_le_bytes(body[4..8].try_into()?) as usize + 8 == body.len(),
        "WebP本文に余分なデータがあります"
    );
    let rgba = decode_pixels(body, side, count)?;
    let map = Heightmap {
        cells,
        halo,
        base: i32::from_le_bytes(blob[16..20].try_into()?),
        offset_metres: 0.,
        step: f64::from_le_bytes(blob[20..28].try_into()?),
        values: rgba
            .chunks_exact(4)
            .map(|p| if p[3] == 0 { MISSING } else { u16::from(p[0]) })
            .collect(),
    };
    ensure!(map.step >= 0.001, "旧WebP形式の量子化幅が不正です");
    map.validate()?;
    Ok(map)
}
pub const MINMAX_HEADER: usize = 80;
fn quantize_minmax(cells: u32, heights: &[Option<f64>]) -> Result<(Heightmap, [f64; 2])> {
    ensure!(
        heights.iter().flatten().all(|v| v.is_finite()),
        "非有限の標高は保存できません"
    );
    let mut low = heights
        .iter()
        .flatten()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let mut high = heights
        .iter()
        .flatten()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    if !low.is_finite() {
        low = 0.;
        high = 0.;
    }
    let span = high - low;
    ensure!(span.is_finite() && span >= 0., "標高範囲が不正です");
    let values = heights
        .iter()
        .map(|h| match h {
            None => MISSING,
            Some(h) if span > 0. => (((h - low) / span * 255.).round().clamp(0., 255.)) as u16,
            Some(_) => 0,
        })
        .collect();
    let map = Heightmap {
        cells,
        halo: 1,
        base: 0,
        offset_metres: low,
        step: span / 255.,
        values,
    };
    map.validate()?;
    Ok((map, [low, high]))
}
pub fn encode_minmax(
    cells: u32,
    heights: &[Option<f64>],
    quality: u32,
) -> Result<(Heightmap, Vec<u8>)> {
    ensure!(quality <= 100, "WebP品質は0〜100が必要です");
    let (map, [low, high]) = quantize_minmax(cells, heights)?;
    let packed = encode_pixels(&map, quality)?;
    let mut blob = b"HMW2".to_vec();
    for v in [cells, 1, map.values.len() as u32] {
        blob.extend(v.to_le_bytes());
    }
    blob.extend(low.to_le_bytes());
    blob.extend(high.to_le_bytes());
    blob.extend(((map.values.len() * 4) as u32).to_le_bytes());
    blob.extend((packed.len() as u32).to_le_bytes());
    blob.extend(Sha256::digest(&packed));
    blob.extend(quality.to_le_bytes());
    blob.extend(0u32.to_le_bytes());
    blob.extend(packed);
    Ok((decode_minmax(&blob)?, blob))
}
fn decode_pixels(body: &[u8], side: u32, count: usize) -> Result<Vec<u8>> {
    let (mut w, mut h) = (0, 0);
    ensure!(
        unsafe { libwebp_sys::WebPGetInfo(body.as_ptr(), body.len(), &mut w, &mut h) } != 0
            && w == side as i32
            && h == side as i32,
        "WebP画像寸法が不正です"
    );
    let mut rgba = vec![0u8; count * 4];
    ensure!(
        !unsafe {
            libwebp_sys::WebPDecodeRGBAInto(
                body.as_ptr(),
                body.len(),
                rgba.as_mut_ptr(),
                rgba.len(),
                w * 4,
            )
        }
        .is_null(),
        "WebP復号に失敗しました"
    );
    ensure!(
        rgba.chunks_exact(4).all(|p| p[3] == 0 || p[3] == 255),
        "欠損マスクが二値ではありません"
    );
    Ok(rgba)
}
fn decode_minmax(blob: &[u8]) -> Result<Heightmap> {
    ensure!(
        blob.len() >= MINMAX_HEADER && blob.len() <= 2 * 1024 * 1024 && &blob[..4] == b"HMW2",
        "最低・最高標高WebPハイトマップの形式・長さが不正です"
    );
    let u = |i| u32::from_le_bytes(blob[i..i + 4].try_into().unwrap());
    let cells = u(4);
    ensure!(
        (8..=256).contains(&cells) && cells.is_power_of_two() && u(8) == 1,
        "格子寸法が不正です"
    );
    let side = cells + 3;
    let count = side.pow(2) as usize;
    let body = &blob[MINMAX_HEADER..];
    ensure!(
        u(12) as usize == count
            && u(32) as usize == count * 4
            && u(36) as usize == body.len()
            && u(72) <= 100
            && u(76) == 0,
        "最低・最高標高WebPヘッダーが不正です"
    );
    let low = f64::from_le_bytes(blob[16..24].try_into()?);
    let high = f64::from_le_bytes(blob[24..32].try_into()?);
    ensure!(
        low.is_finite() && high.is_finite() && low <= high && (high - low).is_finite(),
        "最低・最高標高が不正です"
    );
    ensure!(
        Sha256::digest(body).as_slice() == &blob[40..72],
        "WebP本文のSHA-256が不一致です"
    );
    ensure!(
        body.len() >= 12
            && &body[..4] == b"RIFF"
            && &body[8..12] == b"WEBP"
            && u32::from_le_bytes(body[4..8].try_into()?) as usize + 8 == body.len(),
        "WebP本文に余分なデータがあります"
    );
    let rgba = decode_pixels(body, side, count)?;
    let map = Heightmap {
        cells,
        halo: 1,
        base: 0,
        offset_metres: low,
        step: (high - low) / 255.,
        values: rgba
            .chunks_exact(4)
            .map(|p| if p[3] == 0 { MISSING } else { u16::from(p[0]) })
            .collect(),
    };
    map.validate()?;
    Ok(map)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minmax_uses_all_levels_for_flat_ground_and_preserves_bounds() -> Result<()> {
        let h = (0..259 * 259)
            .map(|i| Some(-123.456 + (i % 259) as f64 / 258. * 0.2))
            .collect::<Vec<_>>();
        let (quantized, range) = quantize_minmax(256, &h)?;
        assert_eq!(range, [-123.456, -123.256]);
        assert_eq!(
            quantized
                .values
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            256
        );
        let (full, blob) = encode_minmax(256, &h, 90)?;
        assert_eq!(f64::from_le_bytes(blob[16..24].try_into()?), range[0]);
        assert_eq!(f64::from_le_bytes(blob[24..32].try_into()?), range[1]);
        let old = encode(256, 0.125, &h, 90)?.0;
        let mae = |map: &Heightmap| {
            h.iter()
                .enumerate()
                .map(|(i, h)| (map.height(i % 259, i / 259).unwrap() - h.unwrap()).abs())
                .sum::<f64>()
                / h.len() as f64
        };
        eprintln!(
            "標高差0.2mの試験（品質90）: 旧平均絶対誤差={:.9}m / 最低最高方式={:.9}m",
            mae(&old),
            mae(&full)
        );
        assert!(
            mae(&full) < mae(&old) * 0.1,
            "狭い標高範囲で精度が改善しない: old={} new={}",
            mae(&old),
            mae(&full)
        );
        assert!(full.step < 0.001);
        assert!(
            full.encode().is_err(),
            "任意の最低標高をu16の旧ヘッダーへ落としている"
        );
        let mut invalid = blob.clone();
        invalid[24..32].copy_from_slice((-500f64).to_le_bytes().as_slice());
        assert!(decode(&invalid).is_err());
        let mut invalid = blob.clone();
        *invalid.last_mut().unwrap() ^= 1;
        assert!(decode(&invalid).is_err());
        Ok(())
    }
    #[test]
    fn minmax_constant_missing_and_negative_heights() -> Result<()> {
        for value in [-12.345, 0., 123.456] {
            let (map, _) = encode_minmax(8, &vec![Some(value); 121], 90)?;
            assert_eq!(map.step, 0.);
            assert_eq!(map.height(5, 5), Some(value));
        }
        let mut heights = vec![Some(-15.); 121];
        heights[60] = None;
        let (map, _) = encode_minmax(8, &heights, 90)?;
        assert_eq!(map.height(5, 5), None);
        assert_eq!(map.height(2, 2), Some(-15.));
        let (map, _) = encode_minmax(8, &vec![None; 121], 90)?;
        assert!(map.values.iter().all(|v| *v == MISSING));
        Ok(())
    }
    #[test]
    fn lossy_roundtrip_preserves_missing_and_range() -> Result<()> {
        let mut h = (0..259 * 259)
            .map(|i| Some(-200. + (i % 259) as f64 * 4. + (i / 259) as f64 * 3.))
            .collect::<Vec<_>>();
        for i in (0..h.len()).step_by(997) {
            h[i] = None;
        }
        let (map, blob) = encode(256, 0.125, &h, 90)?;
        assert!(blob.len() < 259 * 259);
        for (i, value) in h.iter().enumerate() {
            assert_eq!(value.is_none(), map.values[i] == MISSING);
        }
        assert!(map.step > 0.125);
        assert!(map.values.iter().all(|v| *v == MISSING || *v <= 255));
        let mut corrupt = blob.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(decode(&corrupt).is_err());
        let mut trailing = blob.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        assert!(
            decode(&encode(8, 0.125, &vec![None; 121], 90)?.1)?
                .values
                .iter()
                .all(|v| *v == MISSING)
        );
        Ok(())
    }
}
