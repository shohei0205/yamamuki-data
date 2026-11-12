use anyhow::{Result, ensure};
use std::io::Read;
fn u16_at(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes(b[p..p + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}
fn f32_at(b: &[u8], p: usize) -> f32 {
    f32::from_bits(u32_at(b, p))
}
fn unpack(b: &[u8], gzip: bool, limit: usize) -> Result<Vec<u8>> {
    let reader: Box<dyn Read + '_> = if gzip {
        Box::new(flate2::read::GzDecoder::new(b))
    } else {
        Box::new(lzma_rust2::XzReader::new(b, false))
    };
    let mut raw = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut raw)?;
    ensure!(raw.len() <= limit, "展開後のデータが上限を超えています");
    Ok(raw)
}
pub struct Grid {
    pub rows: usize,
    pub columns: usize,
    pub values: Vec<f32>,
}
pub fn decode_grid(b: &[u8]) -> Result<Grid> {
    ensure!(
        b.len() >= 18 && &b[..4] == b"YMD1",
        "標高のヘッダーが不正です"
    );
    let (rows, columns) = (u16_at(b, 4) as usize, u16_at(b, 6) as usize);
    let step = f32_at(b, 8);
    let base = u32_at(b, 12) as i32;
    let (flags, codec) = (b[16], b[17]);
    let n = rows * columns;
    ensure!(
        n > 0 && n <= 4_000_000 && flags <= 3 && codec <= 1 && step > 0. && step <= 5.,
        "標高の保存条件が不正です"
    );
    let width = if flags & 1 != 0 { 4 } else { 2 };
    let length = n * width;
    let mask = if flags & 2 != 0 { n.div_ceil(8) } else { 0 };
    let raw = unpack(&b[18..], codec == 0, length + mask)?;
    ensure!(raw.len() == length + mask, "標高のデータ長が不正です");
    let mut sums = vec![0i64; n];
    let mut values = vec![0.; n];
    for y in 0..rows {
        for x in 0..columns {
            let i = y * columns + x;
            let delta = if width == 4 {
                u32_at(&raw, i * 4) as i32 as i64
            } else {
                u16_at(&raw, i * 2) as i16 as i64
            };
            sums[i] = delta
                + if y > 0 { sums[i - columns] } else { 0 }
                + if x > 0 { sums[i - 1] } else { 0 }
                - if x > 0 && y > 0 {
                    sums[i - columns - 1]
                } else {
                    0
                };
            values[i] = ((sums[i] + base as i64) as f64 * step as f64) as f32;
            if mask > 0 && raw[length + i / 8] & (1 << (i % 8)) != 0 {
                values[i] = f32::NAN;
            }
        }
    }
    Ok(Grid {
        rows,
        columns,
        values,
    })
}
