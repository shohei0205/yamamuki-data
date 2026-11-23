//! 制御点本文に、点順序に対応する8bitの局所傾き調整量を付ける。
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
pub const MAGIC: &[u8; 4] = b"NPA1";
pub fn attach(controls: &[u8], amounts: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        controls.len() >= 24
            && [b"NQC1", b"NDC1", b"NQZ1", b"NDZ1"]
                .iter()
                .any(|m| controls.starts_with(*m)),
        "属性を付ける制御点本文が不正です"
    );
    let n = u32::from_le_bytes(controls[4..8].try_into()?) as usize;
    ensure!(n > 0 && amounts.len() == n, "傾き調整量の個数が不正です");
    let compressed = zstd::bulk::compress(amounts, 3)?;
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&controls[4..24]);
    out.extend((controls.len() as u32).to_le_bytes());
    out.extend_from_slice(controls);
    out.extend(Sha256::digest(amounts));
    out.extend((compressed.len() as u32).to_le_bytes());
    out.extend(compressed);
    ensure!(out.len() <= 16777216, "属性付き制御点本文が上限超過です");
    Ok(out)
}
pub fn split(bytes: &[u8]) -> Result<(&[u8], Vec<u8>)> {
    ensure!(
        bytes.len() >= 64 && bytes.len() <= 16777216 && bytes.starts_with(MAGIC),
        "属性付き制御点本文の長さが不正です"
    );
    let n = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
    ensure!(n > 0, "傾き調整量の個数が上限超過です");
    let length = u32::from_le_bytes(bytes[24..28].try_into()?) as usize;
    let end = 28usize
        .checked_add(length)
        .ok_or_else(|| anyhow::anyhow!("制御点本文の長さが上限超過です"))?;
    ensure!(
        length >= 24 && end.checked_add(36).is_some_and(|v| v <= bytes.len()),
        "属性付き制御点本文が途中で切れています"
    );
    let controls = &bytes[28..end];
    ensure!(
        [b"NQC1", b"NDC1", b"NQZ1", b"NDZ1"]
            .iter()
            .any(|m| controls.starts_with(*m))
            && controls[4..24] == bytes[4..24],
        "制御点本文と属性のヘッダーが一致しません"
    );
    let compressed = u32::from_le_bytes(bytes[end + 32..end + 36].try_into()?) as usize;
    ensure!(
        end + 36 + compressed == bytes.len(),
        "傾き調整量の本文長が不正です"
    );
    let frame = &bytes[end + 36..];
    ensure!(
        zstd::zstd_safe::find_frame_compressed_size(frame)
            .map_err(|_| anyhow::anyhow!("傾き調整量のフレームが不正です"))?
            == frame.len(),
        "傾き調整量に余分なフレームがあります"
    );
    let amounts = zstd::bulk::decompress(frame, n)?;
    ensure!(
        amounts.len() == n && Sha256::digest(&amounts).as_slice() == &bytes[end..end + 32],
        "傾き調整量の個数またはSHA-256が一致しません"
    );
    Ok((controls, amounts))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_mode_is_explicit_and_cannot_mix_with_stored_coefficients() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("automatic.sqlite3");
        let mut conn = rusqlite::Connection::open(&path)?;
        yamamuki_data_sqlite::initialize(&conn)?;
        crate::fixture::create(&mut conn)?;
        conn.execute("UPDATE terrain_levels SET settings_json=json_set(settings_json,'$.slopeMode','unknown') WHERE z=13",[])?;
        assert!(crate::sqlite::Database::open(&path).is_err());
        conn.execute("UPDATE terrain_levels SET settings_json=json_set(settings_json,'$.slopeMode',?1) WHERE z=13",[crate::natural_local_gradient::AUTOMATIC_SLOPE_MODE])?;
        let mut db = crate::sqlite::Database::open(&path)?;
        let key = *db.catalog.keys().find(|k| k.0 == 13).unwrap();
        assert!(db.automatic_slope_levels.contains(&13));
        let points = db.points(key)?;
        let mut tile = db.tile(key)?;
        tile.controls = attach(&tile.controls, &vec![255; points.len()])?;
        conn.execute(
            "UPDATE terrain_tiles SET payload=?1 WHERE level_key=13",
            [tile.encode()?],
        )?;
        let mut db = crate::sqlite::Database::open(&path)?;
        assert!(db.points(key).is_err());
        Ok(())
    }
    #[test]
    fn attributes_survive_recompression_and_reject_corruption() -> Result<()> {
        let points = (0..512)
            .map(|i| [i * 16, (i % 17) * 16, i % 37])
            .collect::<Vec<_>>();
        let base = crate::natural_codec::independent(&points)?;
        let amounts = (0..points.len())
            .map(|i| (i % 256) as u8)
            .collect::<Vec<_>>();
        let packed = attach(&base, &amounts)?;
        assert_eq!(split(&packed)?.1, amounts);
        assert_eq!(
            crate::natural_codec::decode_global(&packed, [0, 0], &[])?,
            points
        );
        let smaller = crate::natural_codec_zstd::recompress(&packed)?;
        assert_eq!(split(&smaller)?.1, amounts);
        assert_eq!(
            crate::natural_codec::decode_global(&smaller, [0, 0], &[])?,
            points
        );
        assert!(attach(&packed, &amounts).is_err());
        assert!(attach(&base, &amounts[..511]).is_err());
        for end in 0..packed.len() {
            assert!(split(&packed[..end]).is_err());
        }
        let mut bad = packed.clone();
        bad.push(0);
        assert!(split(&bad).is_err());
        let mut bad = packed.clone();
        bad[28 + base.len()] ^= 1;
        assert!(split(&bad).is_err());
        let mut bad = packed.clone();
        bad[4] ^= 1;
        assert!(split(&bad).is_err());
        Ok(())
    }
    #[test]
    fn sqlite_restores_and_checks_attributes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.sqlite3");
        let mut connection = rusqlite::Connection::open(&path)?;
        yamamuki_data_sqlite::initialize(&connection)?;
        crate::fixture::create(&mut connection)?;
        let mut db = crate::sqlite::Database::open(&path)?;
        let key = *db.catalog.keys().min().unwrap();
        let original = db.points(key)?;
        let mut tile = db.tile(key)?;
        let amounts = vec![127; original.len()];
        tile.controls = attach(&tile.controls, &amounts)?;
        connection.execute(
            "UPDATE terrain_tiles SET payload=?1 WHERE level_key=?2",
            rusqlite::params![tile.encode()?, key.0],
        )?;
        drop(db);
        let mut db = crate::sqlite::Database::open(&path)?;
        assert_eq!(db.points(key)?, original);
        assert_eq!(db.slope_adjustments(key)?.unwrap().as_ref(), &amounts);
        db.validate_all()?;
        Ok(())
    }
}
