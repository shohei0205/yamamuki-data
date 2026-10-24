//! 地理院タイルと同じXYZ番号。投影座標は東・北、番号のYは南向き。
use anyhow::{Result, ensure};
pub const R: f64 = 6378137.;
pub fn half_world() -> f64 {
    std::f64::consts::PI * R
}
pub fn width(z: u32) -> f64 {
    2. * half_world() / 2f64.powi(z as i32)
}
pub fn max_latitude() -> f64 {
    std::f64::consts::PI.sinh().atan().to_degrees()
}
pub fn project(lon: f64, lat: f64) -> Result<[f64; 2]> {
    ensure!(
        lon.is_finite()
            && lat.is_finite()
            && (-180. ..=180.).contains(&lon)
            && lat.abs() <= max_latitude(),
        "緯度経度がWebメルカトル範囲外です"
    );
    Ok([R * lon.to_radians(), R * lat.to_radians().tan().asinh()])
}
pub fn inverse(x: f64, y: f64) -> Result<[f64; 2]> {
    let m = half_world();
    ensure!(
        x.is_finite() && y.is_finite() && x.abs() <= m && y.abs() <= m,
        "投影座標が範囲外です"
    );
    Ok([(x / R).to_degrees(), (y / R).sinh().atan().to_degrees()])
}
pub fn tile(lon: f64, lat: f64, z: u32) -> Result<[i64; 2]> {
    ensure!(z <= 30, "ズームが範囲外です");
    let [x, y] = project(lon, lat)?;
    let n = 1_i64 << z;
    let s = width(z);
    let m = half_world();
    // 入力範囲は先に検査済み。世界端の丸め誤差と東端・南端を端タイルへ収める。
    Ok([
        (((x + m) / s).floor() as i64).clamp(0, n - 1),
        (((m - y) / s).floor() as i64).clamp(0, n - 1),
    ])
}
pub fn bounds(z: u32, x: i64, y: i64) -> Result<[f64; 4]> {
    ensure!(z <= 30, "ズームが範囲外です");
    let n = 1_i64 << z;
    ensure!(
        (0..n).contains(&x) && (0..n).contains(&y),
        "XYZ番号が範囲外です"
    );
    let s = width(z);
    let m = half_world();
    Ok([
        -m + x as f64 * s,
        m - (y + 1) as f64 * s,
        -m + (x + 1) as f64 * s,
        m - y as f64 * s,
    ])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_roundtrip_and_limits() {
        for p in [[138., 36.], [130., 31.], [146., 45.], [0., 0.]] {
            let [x, y] = project(p[0], p[1]).unwrap();
            let r = inverse(x, y).unwrap();
            assert!((r[0] - p[0]).abs() < 1e-10 && (r[1] - p[1]).abs() < 1e-10);
        }
        assert!(project(0., 90.).is_err());
        assert!(project(181., 0.).is_err());
        assert!(project(f64::NAN, 0.).is_err());
    }
    #[test]
    fn xyz_matches_standard_and_parent_bounds() {
        assert_eq!(tile(0., 0., 14).unwrap(), [8192, 8192]);
        assert_eq!(tile(-180., max_latitude(), 14).unwrap(), [0, 0]);
        assert_eq!(tile(180., -max_latitude(), 14).unwrap(), [16383, 16383]);
        let p = tile(138.7274, 35.3606, 14).unwrap();
        let n = 16384f64;
        assert_eq!(p[0], ((138.7274 + 180.) / 360. * n).floor() as i64);
        assert_eq!(
            p[1],
            ((1. - 35.3606_f64.to_radians().tan().asinh() / std::f64::consts::PI) / 2. * n).floor()
                as i64
        );
        for z in 7..=14 {
            let a = tile(138.7274, 35.3606, z).unwrap();
            let b = tile(138.7274, 35.3606, z - 1).unwrap();
            assert_eq!([a[0] / 2, a[1] / 2], b);
            let child = bounds(z, a[0], a[1]).unwrap();
            let parent = bounds(z - 1, b[0], b[1]).unwrap();
            assert!(
                child[0] >= parent[0] - 1e-8
                    && child[1] >= parent[1] - 1e-8
                    && child[2] <= parent[2] + 1e-8
                    && child[3] <= parent[3] + 1e-8
            );
        }
    }
}
