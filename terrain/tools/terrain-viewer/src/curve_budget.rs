#[cfg(test)]
use crate::{geometry::Mesh, selection::Camera};
use glam::DVec4;
#[cfg(test)]
use glam::{DMat4, DVec3};
#[cfg(test)]
use std::sync::Arc;

#[cfg(test)]
const VISIBLE_FACE_LIMIT: usize = 2_000_000;
#[cfg(test)]
const BUFFER_LIMIT: u64 = 128 * 1024 * 1024;
#[cfg(test)]
const GENERATION_LIMIT: u64 = 384 * 1024 * 1024;

// 画面を横切る面も数える。頂点が画面外にあるだけでは除外しない。
pub(crate) fn intersects(mut polygon: Vec<DVec4>) -> bool {
    let mask = |p: DVec4| {
        u8::from(p.x < -p.w)
            | (u8::from(p.x > p.w) << 1)
            | (u8::from(p.y < -p.w) << 2)
            | (u8::from(p.y > p.w) << 3)
            | (u8::from(p.z < 0.) << 4)
            | (u8::from(p.z > p.w) << 5)
    };
    let masks: Vec<_> = polygon.iter().copied().map(mask).collect();
    if masks.iter().all(|&m| m == 0) {
        return true;
    }
    if masks.iter().fold(63, |a, &b| a & b) != 0 {
        return false;
    }
    for plane in 0..6 {
        let distance = |p: DVec4| match plane {
            0 => p.w + p.x,
            1 => p.w - p.x,
            2 => p.w + p.y,
            3 => p.w - p.y,
            4 => p.z,
            _ => p.w - p.z,
        };
        let mut next = Vec::with_capacity(9);
        for i in 0..polygon.len() {
            let a = polygon[i];
            let b = polygon[(i + 1) % polygon.len()];
            let da = distance(a);
            let db = distance(b);
            if da >= 0. {
                next.push(a);
            }
            if (da >= 0.) != (db >= 0.) {
                next.push(a + (b - a) * (da / (da - db)));
            }
        }
        polygon = next;
        if polygon.is_empty() {
            return false;
        }
    }
    true
}

#[cfg(test)]
pub fn visible_faces(mesh: &Mesh, origin: [f64; 2], camera: &Camera) -> usize {
    let target = DVec3::new(0., camera.height, 0.);
    let view = DMat4::look_at_rh(target + camera.back() * camera.distance, target, DVec3::Y);
    // 曲面変形と操作中の変化に備え、画面端に5%の余裕を取る。
    let projection = DMat4::perspective_rh(
        2. * ((std::f64::consts::PI / 8.).tan() * 1.05).atan(),
        camera.viewport[0] / camera.viewport[1].max(1.),
        (camera.distance / 10000.).max(1.),
        (camera.distance * 5.).max(20000000.),
    );
    let matrix = projection * view;
    let points: Vec<_> = mesh
        .vertices
        .iter()
        .map(|v| {
            matrix
                * DVec4::new(
                    v.base[0] as f64 + origin[0] - camera.center[0],
                    v.base[1] as f64,
                    v.base[2] as f64 + origin[1] - camera.center[1],
                    1.,
                )
        })
        .collect();
    mesh.indices
        .chunks_exact(3)
        .filter(|f| intersects(f.iter().map(|&i| points[i as usize]).collect()))
        .count()
}

// 全領域の生成メモリは可視面数と別に制限する。コマンド送信まで生存する各段も含む。
#[cfg(test)]
pub fn memory_fits(v: u64, e: u64, f: u64, words: u64, division: u32) -> bool {
    let mut total = words * 4;
    if total > BUFFER_LIMIT {
        return false;
    }
    let mut d = 1u64;
    loop {
        let n = v + e * (d - 1) + f * (d - 1) * d.saturating_sub(2) / 2;
        let faces = f * d * d;
        let buffers = [n * 32, faces * 12, (n + 3 * faces) * 4];
        if buffers.iter().any(|&b| b > BUFFER_LIMIT) {
            return false;
        }
        total += buffers.iter().sum::<u64>();
        if d == division as u64 {
            total += n * 32;
            return total <= GENERATION_LIMIT;
        }
        d *= 2;
    }
}

#[cfg(test)]
pub fn divisions(mesh: &Mesh, visible: usize, requested: u32) -> u32 {
    let mut d = 1;
    while d < requested {
        let next = d * 2;
        if visible.saturating_mul((next * next) as usize) > VISIBLE_FACE_LIMIT
            || !memory_fits(
                mesh.vertices.len() as u64,
                mesh.control.edges as u64,
                (mesh.indices.len() / 3) as u64,
                mesh.control.words.len() as u64,
                next,
            )
        {
            break;
        }
        d = next;
    }
    d
}

#[cfg(test)]
pub fn for_view(mesh: &Arc<Mesh>, origin: [f64; 2], camera: &Camera, requested: u32) -> Arc<Mesh> {
    for_visible(mesh, visible_faces(mesh, origin, camera), requested)
}

#[cfg(test)]
pub fn for_visible(mesh: &Arc<Mesh>, visible: usize, requested: u32) -> Arc<Mesh> {
    let d = divisions(mesh, visible, requested);
    if d == mesh.divisions {
        return mesh.clone();
    }
    let mut adjusted = (**mesh).clone();
    adjusted.divisions = d;
    Arc::new(adjusted)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipping_counts_crossing_triangles_but_excludes_outside_and_behind() {
        let p = |x, y, z| DVec4::new(x, y, z, 1.);
        assert!(intersects(vec![
            p(-2., -2., 0.5),
            p(2., -2., 0.5),
            p(0., 2., 0.5)
        ]));
        assert!(!intersects(vec![
            p(2., 0., 0.5),
            p(3., 0., 0.5),
            p(2., 1., 0.5)
        ]));
        assert!(!intersects(vec![
            p(0., 0., -1.),
            p(1., 0., -1.),
            p(0., 1., -1.)
        ]));
        assert!(intersects(vec![
            p(0., 0., -1.),
            p(0.5, 0., 0.5),
            p(0., 0.5, 0.5)
        ]));
    }
    #[test]
    fn offscreen_faces_do_not_consume_visible_budget_but_memory_still_limits() {
        let indices = vec![0, 1, 2].repeat(40000);
        let mesh = Arc::new(Mesh {
            vertices: [[-10., 0., -10.], [10., 0., -10.], [0., 0., 10.]]
                .into_iter()
                .map(|p| crate::geometry::Vertex { base: p, curved: p })
                .collect(),
            control: crate::gpu_loop::control(3, &indices),
            indices,
            source_faces: 40000,
            tiles: 1,
            divisions: 8,
            adaptive: None,
            levels: vec![14],
        });
        let mut camera = Camera::default();
        camera.center = [0., 0.];
        camera.height = 0.;
        camera.distance = 1000.;
        camera.pitch = 1.56;
        assert_eq!(visible_faces(&mesh, [0., 0.], &camera), 40000);
        assert_eq!(for_view(&mesh, [0., 0.], &camera, 8).divisions, 4);
        camera.center = [1000000., 0.];
        assert_eq!(visible_faces(&mesh, [0., 0.], &camera), 0);
        assert_eq!(for_view(&mesh, [0., 0.], &camera, 8).divisions, 8);
        assert_eq!(mesh.divisions, 8);
        assert!(memory_fits(25000, 75000, 50000, 400000, 8));
        assert!(!memory_fits(250000, 750000, 500000, 4000000, 8));
    }
}
