//! 左上原点、X東・Y南の6バイト頂点。入力と復元結果の投影Yは北向き。
use anyhow::{Result, ensure};
pub fn encode(p: [f64; 3], origin: [f64; 2], width: f64) -> Result<[u8; 6]> {
    ensure!(
        p.iter().chain(origin.iter()).all(|v| v.is_finite()) && width.is_finite() && width > 0.,
        "座標が不正です"
    );
    let mut out = [0; 6];
    for k in 0..2 {
        let direction = if k == 0 { 1. } else { -1. };
        let v = ((p[k] - origin[k]) * direction * 16384. / width).round();
        ensure!(
            (-32768. ..=32767.).contains(&v),
            "余白頂点がxyの16ビット範囲を超えました"
        );
        out[k * 2..k * 2 + 2].copy_from_slice(&(v as i16).to_le_bytes());
    }
    ensure!(
        (-12000. ..=10383.75).contains(&p[2]),
        "標高の16ビット範囲を超えました"
    );
    let z = (24000. + p[2] / if p[2] < 0. { 0.5 } else { 0.25 }).round() as u16;
    out[4..6].copy_from_slice(&z.to_le_bytes());
    Ok(out)
}
pub fn decode(p: [u8; 6], origin: [f64; 2], width: f64) -> [f64; 3] {
    let x = i16::from_le_bytes(p[0..2].try_into().unwrap()) as f64;
    let y = i16::from_le_bytes(p[2..4].try_into().unwrap()) as f64;
    let z = u16::from_le_bytes(p[4..6].try_into().unwrap()) as i32 - 24000;
    [
        origin[0] + x * width / 16384.,
        origin[1] - y * width / 16384.,
        z as f64 * if z < 0 { 0.5 } else { 0.25 },
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_coordinates_follow_xyz_from_top_left() {
        let origin = [1000., 5000.];
        let bytes = encode([2000., 4000., 0.], origin, 2000.).unwrap();
        assert_eq!(i16::from_le_bytes(bytes[0..2].try_into().unwrap()), 8192);
        assert_eq!(i16::from_le_bytes(bytes[2..4].try_into().unwrap()), 8192);
        assert_eq!(decode(bytes, origin, 2000.), [2000., 4000., 0.]);
        assert!(encode([1000., 1000., 0.], origin, 2000.).is_err());
    }
    #[test]
    fn signed_xy_and_both_height_ranges() {
        for z in [-12000., -10000., -0.1, 0., 0.1, 8849., 10383.75] {
            let p = [-2000., 2000., z];
            let raw = encode(p, [0., 0.], 2000.).unwrap();
            let restored = decode(raw, [0., 0.], 2000.);
            assert_eq!(&restored[..2], &p[..2]);
            assert!((restored[2] - z).abs() <= if z < 0. { 0.25 } else { 0.125 });
        }
        assert_eq!(
            u16::from_le_bytes(
                encode([0., 0., 0.], [0., 0.], 2000.).unwrap()[4..6]
                    .try_into()
                    .unwrap()
            ),
            24000
        );
    }
    #[test]
    fn invalid_values_are_not_clamped() {
        for p in [
            [0., 0., -12001.],
            [0., 0., 10384.],
            [f64::NAN, 0., 0.],
            [4000., 0., 0.],
        ] {
            assert!(encode(p, [0., 0.], 2000.).is_err());
        }
    }
    #[test]
    fn shared_coarse_coordinates_are_exact_in_finer_tiles() {
        for q in 5..=12 {
            let width = 2000. * 2f64.powi(q - 4);
            let p = [width / 3., width / 4., 3000.12];
            let coarse = decode(encode(p, [0., 0.], width).unwrap(), [0., 0.], width);
            let fine = decode(
                encode(coarse, [0., 0.], width / 2.).unwrap(),
                [0., 0.],
                width / 2.,
            );
            assert_eq!(coarse, fine);
        }
    }
}
