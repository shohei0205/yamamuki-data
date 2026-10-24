//! 親頂点参照。上位8ビットは依存表slot、下位24ビットは親のローカル番号。
use anyhow::{Result, ensure};
pub const MAX_DEPENDENCIES: usize = 256;
pub const MAX_PARENT_VERTICES: u32 = 1 << 24;
pub fn encode(slot: usize, vertex: u32) -> Result<[u8; 4]> {
    ensure!(
        slot < MAX_DEPENDENCIES,
        "親タイル参照番号が8ビット範囲外です"
    );
    ensure!(
        vertex < MAX_PARENT_VERTICES,
        "親頂点番号が24ビット範囲外です"
    );
    Ok((((slot as u32) << 24) | vertex).to_le_bytes())
}
pub fn decode(bytes: [u8; 4], parent_counts: &[u32]) -> Result<(usize, u32)> {
    ensure!(
        parent_counts.len() <= MAX_DEPENDENCIES,
        "依存タイル数が256を超えています"
    );
    let value = u32::from_le_bytes(bytes);
    let slot = (value >> 24) as usize;
    let vertex = value & 0x00ff_ffff;
    let count = *parent_counts
        .get(slot)
        .ok_or_else(|| anyhow::anyhow!("参照先の親タイルがありません"))?;
    ensure!(
        count <= MAX_PARENT_VERTICES,
        "参照先の親頂点数が24ビット範囲を超えています"
    );
    ensure!(vertex < count, "親頂点番号が親タイルの範囲外です");
    Ok((slot, vertex))
}
// mode=0はu32生列、mode=1はタイル別の昇順差分。復元順は変えない。
fn write_varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 128 {
        out.push((v as u8 & 127) | 128);
        v >>= 7;
    }
    out.push(v as u8);
}
fn read_varint(src: &[u8], at: &mut usize) -> Result<u32> {
    let mut v = 0u32;
    for i in 0..5 {
        let b = *src
            .get(*at)
            .ok_or_else(|| anyhow::anyhow!("親参照整数の途中切断"))?;
        *at += 1;
        ensure!(i < 4 || b <= 15, "親参照整数がu32範囲外です");
        v |= ((b & 127) as u32) << (7 * i);
        if b < 128 {
            ensure!(i == 0 || b != 0, "親参照整数が最短表現ではありません");
            return Ok(v);
        }
    }
    anyhow::bail!("親参照整数が長すぎます")
}
pub fn encode_stream(values: &[u32]) -> Result<(u8, Vec<u8>)> {
    ensure!(values.len() <= u32::MAX as usize, "親参照数が多すぎます");
    ensure!(
        values.windows(2).all(|v| v[0] < v[1]),
        "親参照は重複なし昇順が必要です"
    );
    let mut packed = vec![0, 0];
    let mut groups = 0u16;
    let mut at = 0;
    while at < values.len() {
        let slot = values[at] >> 24;
        let start = at;
        while at < values.len() && values[at] >> 24 == slot {
            at += 1;
        }
        groups += 1;
        packed.push(slot as u8);
        write_varint(&mut packed, (at - start) as u32);
        let mut previous = values[start] & 0xffffff;
        write_varint(&mut packed, previous);
        for &v in &values[start + 1..at] {
            let index = v & 0xffffff;
            write_varint(&mut packed, index - previous - 1);
            previous = index;
        }
    }
    packed[..2].copy_from_slice(&groups.to_le_bytes());
    let raw_len = values
        .len()
        .checked_mul(4)
        .ok_or_else(|| anyhow::anyhow!("親参照サイズ超過"))?;
    if packed.len() < raw_len {
        Ok((1, packed))
    } else {
        Ok((0, values.iter().flat_map(|v| v.to_le_bytes()).collect()))
    }
}
pub fn decode_stream(mode: u8, src: &[u8], count: usize, parents: &[u32]) -> Result<Vec<u32>> {
    ensure!(
        count <= u32::MAX as usize && parents.len() <= 256,
        "親参照数・依存数が範囲外です"
    );
    let mut out = Vec::new();
    match mode {
        0 => ensure!(
            count.checked_mul(4) == Some(src.len()),
            "生の親参照長が不正です"
        ),
        1 => ensure!(
            src.len() >= 2 && count <= src.len() - 2,
            "圧縮親参照長が不正です"
        ),
        _ => anyhow::bail!("親参照圧縮方式が不明です"),
    }
    out.try_reserve_exact(count)
        .map_err(|_| anyhow::anyhow!("親参照の復元領域を確保できません"))?;
    if mode == 0 {
        for b in src.chunks_exact(4) {
            out.push(u32::from_le_bytes(b.try_into().unwrap()));
        }
    } else {
        let groups = u16::from_le_bytes(src[..2].try_into().unwrap()) as usize;
        ensure!(
            groups > 0 && groups <= parents.len(),
            "親参照グループ数が不正です"
        );
        let mut at = 2;
        let mut previous_slot = None;
        for _ in 0..groups {
            let slot = *src
                .get(at)
                .ok_or_else(|| anyhow::anyhow!("親参照slotの途中切断"))?;
            at += 1;
            ensure!(
                previous_slot.is_none_or(|p| slot > p),
                "親参照slotが昇順ではありません"
            );
            previous_slot = Some(slot);
            let n = read_varint(src, &mut at)? as usize;
            ensure!(
                n > 0 && n <= count - out.len(),
                "親参照グループの個数が不正です"
            );
            let mut index = read_varint(src, &mut at)?;
            for j in 0..n {
                if j > 0 {
                    index = index
                        .checked_add(read_varint(src, &mut at)?)
                        .and_then(|v| v.checked_add(1))
                        .ok_or_else(|| anyhow::anyhow!("親参照差分の加算超過"))?;
                }
                let bytes = encode(slot as usize, index)?;
                decode(bytes, parents)?;
                out.push(u32::from_le_bytes(bytes));
            }
        }
        ensure!(
            at == src.len() && out.len() == count,
            "親参照末尾または個数が不一致です"
        );
    }
    ensure!(
        out.windows(2).all(|v| v[0] < v[1]),
        "親参照は重複なし昇順が必要です"
    );
    for &v in &out {
        decode(v.to_le_bytes(), parents)?;
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_layout_and_limits() {
        assert_eq!(encode(1, 5).unwrap(), [5, 0, 0, 1]);
        assert_eq!(encode(255, 0xffffff).unwrap(), [255; 4]);
        let counts = vec![MAX_PARENT_VERTICES; 256];
        for slot in [0, 1, 255] {
            for v in [0, 65536, 0xffffff] {
                assert_eq!(
                    decode(encode(slot, v).unwrap(), &counts).unwrap(),
                    (slot, v)
                );
            }
        }
        assert!(encode(256, 0).is_err());
        assert!(encode(0, 1 << 24).is_err());
    }
    #[test]
    fn invalid_dependencies_and_parent_indices_are_rejected() {
        assert!(decode([0; 4], &[]).is_err());
        assert!(decode(encode(1, 0).unwrap(), &[10]).is_err());
        assert!(decode(encode(0, 10).unwrap(), &[10]).is_err());
        assert!(decode([0; 4], &[0]).is_err());
        assert!(decode([0; 4], &[MAX_PARENT_VERTICES + 1]).is_err());
        assert!(decode([0; 4], &vec![1; 257]).is_err());
    }
    #[test]
    fn compression_roundtrip_and_raw_fallback() {
        for values in [
            vec![],
            vec![0x01000005],
            (0..1000).map(|i| 0x01000000 + i).collect(),
            vec![0, 0xffffff, 0x01000000, 0xffffffff],
        ] {
            let (mode, data) = encode_stream(&values).unwrap();
            assert!(data.len() <= values.len() * 4);
            assert_eq!(
                decode_stream(mode, &data, values.len(), &vec![1 << 24; 256]).unwrap(),
                values
            );
        }
        let values = (0..1000).map(|i| 0x01000000 + i).collect::<Vec<_>>();
        let (mode, b) = encode_stream(&values).unwrap();
        assert_eq!(mode, 1);
        assert_eq!(b.len(), 1005);
        assert_eq!(encode_stream(&[5]).unwrap().0, 0);
        assert!(encode_stream(&[5, 5]).is_err());
        assert!(encode_stream(&[5, 4]).is_err());
    }
    #[test]
    fn compressed_corruption_is_rejected() {
        let values = (0..1000).collect::<Vec<u32>>();
        let (mode, b) = encode_stream(&values).unwrap();
        assert!(decode_stream(mode, &b[..b.len() - 1], 1000, &[1000]).is_err());
        let mut tail = b.clone();
        tail.push(0);
        assert!(decode_stream(mode, &tail, 1000, &[1000]).is_err());
        assert!(decode_stream(mode, &b, 999, &[1000]).is_err());
        assert!(decode_stream(mode, &b, 1000, &[999]).is_err());
        assert!(decode_stream(2, &[], 0, &[]).is_err());
        assert!(read_varint(&[255; 5], &mut 0).is_err());
        assert!(read_varint(&[128, 0], &mut 0).is_err());
    }
}
