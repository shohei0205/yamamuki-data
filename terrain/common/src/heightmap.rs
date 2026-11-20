//! 独立した標高格子。欠損を海面と区別し、周囲の余白も同じ格子へ保存する。
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::io::Read;
pub const ENCODING: &str = "heightmap-u16-zstd-v1";
pub const WEBP_ENCODING: &str = "heightmap-u8-webp-v1";
pub const WEBP_MINMAX_ENCODING: &str = "heightmap-u8-webp-minmax-v1";
pub fn is_encoding(value: &str) -> bool {
    value == ENCODING || value == WEBP_ENCODING || value == WEBP_MINMAX_ENCODING
}
pub fn matches_encoding(value: &str, bytes: &[u8]) -> bool {
    let magic: &[u8] = match value {
        ENCODING => b"HMP1",
        WEBP_ENCODING => b"HMW1",
        WEBP_MINMAX_ENCODING => b"HMW2",
        _ => return false,
    };
    bytes.starts_with(magic)
}
pub const HEADER: usize = 72;
pub const MISSING: u16 = u16::MAX;
#[derive(Clone, Debug)]
pub struct Heightmap {
    pub cells: u32,
    pub halo: u32,
    pub base: i32,
    pub offset_metres: f64,
    pub step: f64,
    pub values: Vec<u16>,
}
impl Heightmap {
    pub fn width(&self) -> usize {
        (self.cells + 1 + 2 * self.halo) as usize
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (8..=256).contains(&self.cells) && self.cells.is_power_of_two() && self.halo == 1,
            "格子は8〜256セルの2の累乗、余白は1点が必要です"
        );
        ensure!(
            self.offset_metres.is_finite()
                && self.step.is_finite()
                && self.step >= 0.
                && self.step <= 100.,
            "標高量子化幅が不正です"
        );
        ensure!(
            self.values.len() == self.width().pow(2),
            "標高配列の長さまたは有効値が不正です"
        );
        Ok(())
    }
    pub fn quantize(cells: u32, step: f64, heights: &[Option<f64>]) -> Result<Self> {
        ensure!(
            step.is_finite() && step >= 0.001 && step <= 100.,
            "標高量子化幅が不正です"
        );
        ensure!(
            heights.iter().flatten().all(|v| v.is_finite()),
            "非有限の標高は保存できません"
        );
        let lo = heights
            .iter()
            .flatten()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let base = if lo.is_finite() {
            (lo / step).floor()
        } else {
            0.
        };
        ensure!(
            base.is_finite() && base >= i32::MIN as f64 && base <= i32::MAX as f64,
            "基準標高が範囲外です"
        );
        let values = heights
            .iter()
            .map(|h| match h {
                None => Ok(MISSING),
                Some(h) => {
                    let v = (h / step - base).round();
                    ensure!(v >= 0. && v < MISSING as f64, "標高差がu16の範囲を超えます");
                    Ok(v as u16)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let map = Self {
            cells,
            halo: 1,
            base: base as i32,
            offset_metres: 0.,
            step,
            values,
        };
        map.validate()?;
        Ok(map)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        ensure!(
            self.offset_metres == 0. && self.step >= 0.001,
            "u16形式へ任意の標高オフセットは保存できません"
        );
        let raw = self
            .values
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        let packed = zstd::bulk::compress(&raw, 9)?;
        let mut b = b"HMP1".to_vec();
        for v in [self.cells, self.halo, self.values.len() as u32] {
            b.extend(v.to_le_bytes());
        }
        b.extend(self.base.to_le_bytes());
        b.extend(self.step.to_le_bytes());
        b.extend((raw.len() as u32).to_le_bytes());
        b.extend((packed.len() as u32).to_le_bytes());
        b.extend(Sha256::digest(&raw));
        b.extend(0u32.to_le_bytes());
        b.extend(packed);
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.starts_with(b"HMW1") || b.starts_with(b"HMW2") {
            return crate::heightmap_webp::decode(b);
        }
        ensure!(
            b.len() >= HEADER && b.len() <= 2 * 1024 * 1024 && &b[..4] == b"HMP1",
            "ハイトマップの形式・長さが不正です"
        );
        let u = |i| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let cells = u(4);
        let halo = u(8);
        ensure!(
            (8..=256).contains(&cells) && cells.is_power_of_two() && halo == 1,
            "格子寸法が不正です"
        );
        let count = (cells + 1 + 2 * halo).pow(2) as usize;
        ensure!(
            u(12) as usize == count
                && u(28) as usize == count * 2
                && u(32) as usize == b.len() - HEADER
                && u(68) == 0,
            "ハイトマップのヘッダーが不正です"
        );
        ensure!(
            zstd::zstd_safe::find_frame_compressed_size(&b[HEADER..])
                .map_err(|_| anyhow::anyhow!("Zstandardフレームが不正です"))?
                == b.len() - HEADER,
            "圧縮本文に余分なデータがあります"
        );
        let mut decoder = zstd::stream::read::Decoder::new(&b[HEADER..])?;
        decoder.window_log_max(20)?;
        let mut raw = Vec::new();
        decoder.take((count * 2 + 1) as u64).read_to_end(&mut raw)?;
        ensure!(
            raw.len() == count * 2 && Sha256::digest(&raw).as_slice() == &b[36..68],
            "標高配列の長さまたはSHA-256が不一致です"
        );
        let map = Self {
            cells,
            halo,
            base: i32::from_le_bytes(b[16..20].try_into()?),
            offset_metres: 0.,
            step: f64::from_le_bytes(b[20..28].try_into()?),
            values: raw
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
                .collect(),
        };
        ensure!(map.step >= 0.001, "u16標高量子化幅が不正です");
        map.validate()?;
        Ok(map)
    }
    pub fn height(&self, x: usize, y: usize) -> Option<f64> {
        let v = *self
            .values
            .get(y.checked_mul(self.width())?.checked_add(x)?)?;
        if x >= self.width() || y >= self.width() || v == MISSING {
            None
        } else {
            Some(self.offset_metres + (self.base as f64 + v as f64) * self.step)
        }
    }
    /// u,vはタイル内の0〜1。余白の範囲まで評価できる。
    pub fn sample(&self, u: f64, v: f64) -> Option<f64> {
        if !u.is_finite() || !v.is_finite() {
            return None;
        }
        let n = self.width();
        let x = (u * self.cells as f64 + self.halo as f64).clamp(0., (n - 1) as f64);
        let y = (v * self.cells as f64 + self.halo as f64).clamp(0., (n - 1) as f64);
        let ix = (x.floor() as usize).min(n - 2);
        let iy = (y.floor() as usize).min(n - 2);
        let fx = x - ix as f64;
        let fy = y - iy as f64;
        let mut sum = 0.;
        for (dx, dy, w) in [
            (0, 0, (1. - fx) * (1. - fy)),
            (1, 0, fx * (1. - fy)),
            (0, 1, (1. - fx) * fy),
            (1, 1, fx * fy),
        ] {
            if w > 1e-12 {
                sum += self.height(ix + dx, iy + dy)? * w;
            }
        }
        Some(sum)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_missing_remains_unknown() -> Result<()> {
        let m = Heightmap::quantize(8, 0.125, &vec![None; 121])?;
        let r = Heightmap::decode(&m.encode()?)?;
        assert_eq!(r.sample(0.5, 0.5), None);
        Ok(())
    }
    #[test]
    fn roundtrip_halo_signed_and_missing() -> Result<()> {
        let mut h = vec![Some(-12.375); 11 * 11];
        h[5 * 11 + 5] = None;
        let m = Heightmap::quantize(8, 0.125, &h)?;
        let b = m.encode()?;
        let r = Heightmap::decode(&b)?;
        assert_eq!(r.height(0, 0), Some(-12.375));
        assert_eq!(r.sample(0.5, 0.5), None);
        let mut bad = b;
        bad[36] ^= 1;
        assert!(Heightmap::decode(&bad).is_err());
        Ok(())
    }
    #[test]
    fn rejects_range_and_trailing() -> Result<()> {
        assert!(Heightmap::quantize(8, 0.125, &vec![Some(f64::NAN); 121]).is_err());
        let m = Heightmap::quantize(8, 0.125, &vec![Some(0.); 121])?;
        let mut b = m.encode()?;
        b.push(0);
        assert!(Heightmap::decode(&b).is_err());
        Ok(())
    }
}
