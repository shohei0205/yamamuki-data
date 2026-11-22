//! 大きなRiceブロックとZstandard。点の順序・整数XYZ・親参照を変えない。
use anyhow::{Result, ensure};
use std::cell::RefCell;
const LIMIT: usize = 16777216;
pub const BLOCKS: [usize; 4] = [128, 512, 1024, 4096];
fn header(bytes: &[u8]) -> Result<(bool, usize, usize, usize)> {
    ensure!(
        bytes.len() >= 24 && bytes.len() <= LIMIT,
        "制御点本文の長さが不正です"
    );
    let delta = &bytes[..4] == b"NDC1";
    ensure!(
        delta || &bytes[..4] == b"NQC1",
        "変換元はNQC1/NDC1である必要があります"
    );
    let n = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
    let parents = u32::from_le_bytes(bytes[8..12].try_into()?) as usize;
    let fresh = u32::from_le_bytes(bytes[12..16].try_into()?) as usize;
    ensure!(
        n > 0
            && n <= 262144
            && parents <= 4
            && fresh <= n
            && (delta || (parents == 0 && fresh == n)),
        "点数・親数が不正です"
    );
    ensure!(
        bytes[16..20] == 0.0625f32.to_le_bytes() && bytes[20..24] == 0.125f32.to_le_bytes(),
        "量子化幅が不正です"
    );
    Ok((delta, n, parents, fresh))
}
fn columns(bytes: &[u8], delta: bool, n: usize, fresh: usize) -> Result<Vec<Vec<i64>>> {
    let counts = if delta {
        vec![n, n - fresh, n - fresh, n - fresh, fresh, fresh, fresh]
    } else {
        vec![n; 3]
    };
    let mut pos = 24usize;
    let mut result = Vec::new();
    for count in counts {
        let length = u32::from_le_bytes(
            bytes
                .get(pos..pos + 4)
                .ok_or_else(|| anyhow::anyhow!("整数列が途中で切れています"))?
                .try_into()?,
        ) as usize;
        pos += 4;
        let end = pos
            .checked_add(length)
            .ok_or_else(|| anyhow::anyhow!("整数列の長さが上限超過です"))?;
        let values = crate::natural_codec::decode_stream(
            bytes
                .get(pos..end)
                .ok_or_else(|| anyhow::anyhow!("整数列が途中で切れています"))?,
            n,
        )?;
        ensure!(values.len() == count, "整数列の個数が不正です");
        pos = end;
        result.push(values);
    }
    ensure!(pos == bytes.len(), "整数列に余分な末尾があります");
    Ok(result)
}
fn compress(bytes: &[u8], raw: &[u8], delta: bool, block: usize, level: i32) -> Result<Vec<u8>> {
    let mut out = if delta {
        b"NDZ1".to_vec()
    } else {
        b"NQZ1".to_vec()
    };
    out.extend_from_slice(&bytes[4..24]);
    out.extend([block.trailing_zeros() as u8, 0, 0, 0]);
    out.extend((raw.len() as u32).to_le_bytes());
    out.extend(zstd::bulk::compress(raw, level)?);
    ensure!(out.len() <= LIMIT, "制御点本文の圧縮容量が上限超過です");
    Ok(out)
}
/// 比較・検査用。指定したブロック長とZstandardレベルで本文を符号化する。
pub fn transcode(bytes: &[u8], block: usize, level: i32) -> Result<Vec<u8>> {
    let (delta, n, _, fresh) = header(bytes)?;
    ensure!(BLOCKS.contains(&block), "未対応のRiceブロック長です");
    if block == 128 {
        return compress(bytes, &bytes[24..], delta, block, level);
    }
    let mut raw = Vec::new();
    for values in columns(bytes, delta, n, fresh)? {
        let data = crate::natural_codec::stream_block(&values, block)?;
        raw.extend((data.len() as u32).to_le_bytes());
        raw.extend(data);
    }
    compress(bytes, &raw, delta, block, level)
}
/// 小さな本文は従来形式を維持する。全候補を比較し、32byte以上小さくなる場合だけ採用。
pub fn recompress(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.starts_with(b"NQZ1") || bytes.starts_with(b"NDZ1") {
        return Ok(bytes.to_vec());
    }
    let (delta, n, _, fresh) = header(bytes)?;
    if bytes.len() < 128 {
        return Ok(bytes.to_vec());
    }
    let values = columns(bytes, delta, n, fresh)?;
    let mut best = bytes.to_vec();
    for block in BLOCKS {
        let raw = if block == 128 {
            bytes[24..].to_vec()
        } else {
            let mut raw = Vec::new();
            for column in &values {
                let data = crate::natural_codec::stream_block(column, block)?;
                raw.extend((data.len() as u32).to_le_bytes());
                raw.extend(data);
            }
            raw
        };
        let candidate = compress(bytes, &raw, delta, block, 3)?;
        if candidate.len() < best.len() {
            best = candidate;
        }
    }
    if best.len() + 32 <= bytes.len() {
        Ok(best)
    } else {
        Ok(bytes.to_vec())
    }
}
thread_local! {static DECODER:RefCell<Option<zstd::bulk::Decompressor<'static>>>=const {RefCell::new(None)};}
pub(crate) fn decode(
    bytes: &[u8],
    origin: [i32; 2],
    parents: &[&[[i32; 3]]],
) -> Result<Vec<[i32; 3]>> {
    ensure!(
        bytes.len() >= 32 && bytes.len() <= LIMIT,
        "Zstandard制御点本文の長さが不正です"
    );
    let delta = &bytes[..4] == b"NDZ1";
    ensure!(delta || &bytes[..4] == b"NQZ1", "未対応の制御点本文です");
    let n = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
    let length = u32::from_le_bytes(bytes[28..32].try_into()?) as usize;
    let parent_count = u32::from_le_bytes(bytes[8..12].try_into()?) as usize;
    let fresh = u32::from_le_bytes(bytes[12..16].try_into()?) as usize;
    ensure!(
        parent_count <= 4
            && parent_count == parents.len()
            && fresh <= n
            && (delta || (parent_count == 0 && fresh == n)),
        "点数・親参照が不正です"
    );
    ensure!(
        bytes[16..20] == 0.0625f32.to_le_bytes() && bytes[20..24] == 0.125f32.to_le_bytes(),
        "量子化幅が不正です"
    );
    ensure!(
        n > 0
            && n <= 262144
            && [7, 9, 10, 12].contains(&bytes[24])
            && bytes[25..28] == [0; 3]
            && length <= n * 31 + 64,
        "Zstandard制御点ヘッダーが不正です"
    );
    ensure!(
        zstd::zstd_safe::find_frame_compressed_size(&bytes[32..])
            .map_err(|_| anyhow::anyhow!("Zstandardフレームが不正です"))?
            == bytes.len() - 32,
        "余分なZstandardフレームがあります"
    );
    let mut legacy = if delta {
        b"NDC1".to_vec()
    } else {
        b"NQC1".to_vec()
    };
    legacy.extend_from_slice(&bytes[4..24]);
    DECODER.with(|cell| -> Result<()> {
        let mut decoder = cell.borrow_mut();
        if decoder.is_none() {
            *decoder = Some(zstd::bulk::Decompressor::new()?);
        }
        let raw = decoder.as_mut().unwrap().decompress(&bytes[32..], length)?;
        ensure!(raw.len() == length, "Zstandard展開後の長さが一致しません");
        legacy.extend(raw);
        Ok(())
    })?;
    crate::natural_codec::decode_global_block_refs(&legacy, origin, parents, 1usize << bytes[24])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn legacy(delta: bool) -> (Vec<u8>, Vec<Vec<[i32; 3]>>, Vec<[i32; 3]>) {
        let points = (0..512)
            .map(|i| [10000 + i * 16, 20000 + (i % 23) * 16, 100 + i % 19])
            .collect::<Vec<_>>();
        let mut columns = if delta {
            vec![
                vec![0; 512],
                vec![0; 512],
                vec![1; 512],
                vec![0; 512],
                vec![],
                vec![],
                vec![],
            ]
        } else {
            vec![Vec::new(); 3]
        };
        let parents = if delta {
            vec![
                std::iter::once([0, 0, 0])
                    .chain(points.iter().copied())
                    .collect(),
            ]
        } else {
            vec![]
        };
        if !delta {
            let mut previous = [0; 3];
            for p in &points {
                for k in 0..3 {
                    columns[k].push(i64::from(p[k] - previous[k]));
                    previous[k] = p[k];
                }
            }
        }
        let mut bytes = if delta {
            b"NDC1".to_vec()
        } else {
            b"NQC1".to_vec()
        };
        for value in [
            512u32,
            u32::from(delta),
            if delta { 0 } else { 512 },
            0.0625f32.to_bits(),
            0.125f32.to_bits(),
        ] {
            bytes.extend(value.to_le_bytes());
        }
        for values in columns {
            let data = crate::natural_codec::stream(&values).unwrap();
            bytes.extend((data.len() as u32).to_le_bytes());
            bytes.extend(data);
        }
        (bytes, parents, points)
    }
    #[test]
    fn all_blocks_restore_exact_xyz_and_parent_refs() -> Result<()> {
        for delta in [false, true] {
            let (bytes, parents, points) = legacy(delta);
            for block in BLOCKS {
                let packed = transcode(&bytes, block, 3)?;
                assert_eq!(
                    crate::natural_codec::decode_global(&packed, [0, 0], &parents)?,
                    points
                );
            }
            let packed = recompress(&bytes)?;
            assert!(packed.len() <= bytes.len());
            assert_eq!(
                crate::natural_codec::decode_global(&packed, [0, 0], &parents)?,
                points
            );
        }
        Ok(())
    }
    #[test]
    fn truncated_trailing_and_oversized_packets_are_rejected() -> Result<()> {
        let (bytes, _, _) = legacy(false);
        let packed = transcode(&bytes, 1024, 3)?;
        for end in 0..packed.len() {
            assert!(crate::natural_codec::decode_global(&packed[..end], [0, 0], &[]).is_err());
        }
        let mut bad = packed.clone();
        bad.push(0);
        assert!(crate::natural_codec::decode_global(&bad, [0, 0], &[]).is_err());
        for (at, value) in [(24, 8u8), (25, 1u8)] {
            let mut bad = packed.clone();
            bad[at] = value;
            assert!(crate::natural_codec::decode_global(&bad, [0, 0], &[]).is_err());
        }
        let mut bad = packed.clone();
        bad[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(crate::natural_codec::decode_global(&bad, [0, 0], &[]).is_err());
        let mut bad = packed;
        let last = bad.len() - 1;
        bad[last] ^= 128;
        assert!(crate::natural_codec::decode_global(&bad, [0, 0], &[]).is_err());
        Ok(())
    }
}
