//! Web Mercatorと、注視点を原点とする球面の接平面座標を相互変換する。
use crate::shared_data::WORLD;
use glam::DVec3;
pub const RADIUS: f64 = WORLD / std::f64::consts::TAU;
pub fn latitude(y: f64) -> f64 {
    (std::f64::consts::PI - y / RADIUS).sinh().atan()
}
pub fn local(center: [f64; 2], world: [f64; 2], height: f64) -> DVec3 {
    let a = latitude(center[1]);
    let b = latitude(world[1]);
    let dl = (world[0] - center[0]) / RADIUS;
    let db = b - a;
    let r = RADIUS + height;
    DVec3::new(
        r * b.cos() * dl.sin(),
        height
            - r * (2. * (db / 2.).sin().powi(2) + 2. * a.cos() * b.cos() * (dl / 2.).sin().powi(2)),
        -r * (db.sin() + 2. * a.sin() * b.cos() * (dl / 2.).sin().powi(2)),
    )
}
pub fn world(center: [f64; 2], p: DVec3) -> [f64; 2] {
    let a = latitude(center[1]);
    let u = p.y + RADIUS;
    let z = u * a.cos() + p.z * a.sin();
    let lat = (u * a.sin() - p.z * a.cos()).atan2(p.x.hypot(z));
    [
        (center[0] + RADIUS * p.x.atan2(z)).rem_euclid(WORLD),
        RADIUS * (std::f64::consts::PI - lat.tan().asinh()),
    ]
}
pub fn intersect(origin: DVec3, ray: DVec3, height: f64) -> Option<DVec3> {
    let b = origin.dot(ray) + RADIUS * ray.y;
    let a = ray.length_squared();
    let c = origin.x * origin.x
        + origin.z * origin.z
        + (origin.y - height) * (2. * RADIUS + origin.y + height);
    let d = b * b - a * c;
    if d < 0. {
        return None;
    }
    let near = if b < 0. {
        c / (-b + d.sqrt())
    } else {
        (-b - d.sqrt()) / a
    };
    let t = if near >= 0. {
        near
    } else {
        (-b + d.sqrt()) / a
    };
    (t >= 0. && t.is_finite()).then_some(origin + ray * t)
}
/// 日付変更線をまたぐ連続範囲を、保存タイルの経度0〜1の範囲へ分割する。
pub fn texture_parts(area: [f64; 4]) -> Vec<[f64; 4]> {
    let width = area[2] - area[0];
    if width >= 1. {
        return vec![[0., area[1], 1., area[3]]];
    }
    let start = area[0].rem_euclid(1.);
    let end = start + width;
    if end <= 1. {
        vec![[start, area[1], end, area[3]]]
    } else {
        vec![
            [start, area[1], 1., area[3]],
            [0., area[1], end - 1., area[3]],
        ]
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapped_texture_parts_cover_both_ends_without_requesting_the_world() {
        for left in [0.99999, -0.00001, 1.99999] {
            let parts = texture_parts([left, 0.4, left + 0.00002, 0.4001]);
            assert_eq!(parts.len(), 2);
            assert!(
                parts
                    .iter()
                    .all(|p| p[0] >= 0. && p[2] <= 1. && p[2] > p[0])
            );
            assert!((parts.iter().map(|p| p[2] - p[0]).sum::<f64>() - 0.00002).abs() < 1e-12);
            for x in [0.000005, 0.999995] {
                assert!(parts.iter().any(|p| x >= p[0] && x <= p[2]));
            }
        }
        assert_eq!(texture_parts([-1., 0.1, 2., 0.9]), vec![[0., 0.1, 1., 0.9]]);
    }
    #[test]
    fn coordinates_roundtrip_from_centimeters_to_the_far_hemisphere() {
        let c = [WORLD * 0.88, WORLD * 0.39];
        for d in [
            [0., 0.],
            [0.01, -0.02],
            [1000., 2000.],
            [1e6, -2e6],
            [1e7, 3e6],
        ] {
            let q = [c[0] + d[0], c[1] + d[1]];
            let p = local(c, q, 123.);
            let w = world(c, p);
            assert!((w[0] - q[0].rem_euclid(WORLD)).abs() < 1e-6 && (w[1] - q[1]).abs() < 1e-6);
            assert!(((p + DVec3::Y * RADIUS).length() - RADIUS - 123.).abs() < 1e-6);
        }
    }
    #[test]
    fn rays_hit_the_globe_and_miss_space() {
        assert!(
            intersect(DVec3::new(0., 1e7, 0.), -DVec3::Y, 0.)
                .unwrap()
                .length()
                < 1e-6
        );
        assert!(intersect(DVec3::new(0., 1e7, 0.), DVec3::X, 0.).is_none());
    }
}
