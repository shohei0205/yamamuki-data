use crate::{geometry::Mesh, selection::Camera};
use glam::{DMat4, DVec3, DVec4};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone)]
pub struct Plan {
    pub levels: Arc<Vec<u8>>,
    // 元の面番号と、その面内の整数重心座標3点。実頂点番号への変換はGPUで行う。
    pub records: Vec<[u32; 4]>,
    pub visible_faces: usize,
    pub counts: [usize; 4],
    pub limited: bool,
}
type P = [i32; 2];
fn cross(a: P, b: P, c: P) -> i32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn pack(p: P) -> u32 {
    p[0] as u32 | ((p[1] as u32) << 4)
}

// 各面の内部は指定密度で分割し、外周だけ隣接面と同じ分割点を使う。
fn template(d: u32, edges: [u32; 3], max: u32) -> Vec<[u32; 3]> {
    let mut out = Vec::new();
    let step = (max / d) as i32;
    let edge_step = edges.map(|e| (max / e) as i32);
    let max = max as i32;
    let mut triangle = |t: [P; 3]| {
        let mut poly = Vec::new();
        for k in 0..3 {
            let a = t[k];
            let b = t[(k + 1) % 3];
            let edge = if a[1] == 0 && b[1] == 0 {
                Some(0)
            } else if a[0] + a[1] == max && b[0] + b[1] == max {
                Some(1)
            } else if a[0] == 0 && b[0] == 0 {
                Some(2)
            } else {
                None
            };
            let n = edge.map_or(1, |e| step / edge_step[e]);
            for j in 0..n {
                poly.push([a[0] + (b[0] - a[0]) * j / n, a[1] + (b[1] - a[1]) * j / n]);
            }
        }
        while poly.len() > 3 {
            let ear = (0..poly.len())
                .find(|&i| {
                    let a = poly[(i + poly.len() - 1) % poly.len()];
                    let b = poly[i];
                    let c = poly[(i + 1) % poly.len()];
                    cross(a, b, c) > 0
                        && !poly.iter().any(|&p| {
                            p != a
                                && p != b
                                && p != c
                                && cross(a, b, p) >= 0
                                && cross(b, c, p) >= 0
                                && cross(c, a, p) >= 0
                        })
                })
                .expect("曲面の接続テンプレートを分割できません");
            out.push([
                pack(poly[(ear + poly.len() - 1) % poly.len()]),
                pack(poly[ear]),
                pack(poly[(ear + 1) % poly.len()]),
            ]);
            poly.remove(ear);
        }
        out.push([pack(poly[0]), pack(poly[1]), pack(poly[2])]);
    };
    for j in 0..d as i32 {
        for i in 0..d as i32 - j {
            let (x, y) = (i * step, j * step);
            triangle([[x, y], [x + step, y], [x, y + step]]);
            if i + j + 1 < d as i32 {
                triangle([[x + step, y], [x + step, y + step], [x, y + step]]);
            }
        }
    }
    out
}

fn sizes(mesh: &Mesh, origin: [f64; 2], camera: &Camera) -> Vec<f64> {
    let target = DVec3::new(0., camera.height, 0.);
    let matrix = DMat4::perspective_rh(
        std::f64::consts::PI / 4.,
        camera.viewport[0] / camera.viewport[1].max(1.),
        (camera.distance / 10000.).max(1.),
        (camera.distance * 5.).max(20000000.),
    ) * DMat4::look_at_rh(target + camera.back() * camera.distance, target, DVec3::Y);
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
        .map(|f| {
            let p = [
                points[f[0] as usize],
                points[f[1] as usize],
                points[f[2] as usize],
            ];
            if !crate::curve_budget::intersects(p.to_vec()) {
                return 0.;
            }
            if p.iter().any(|v| v.w <= 0.) {
                return camera.viewport[0].max(camera.viewport[1]) * 2.;
            }
            let pixels = p.map(|v| {
                glam::DVec2::new(
                    v.x / v.w * camera.viewport[0] * 0.5,
                    v.y / v.w * camera.viewport[1] * 0.5,
                )
            });
            (pixels[0] - pixels[1])
                .length()
                .max((pixels[1] - pixels[2]).length())
                .max((pixels[2] - pixels[0]).length())
        })
        .collect()
}

fn desired(pixels: f64, step: f64, max: u32) -> u8 {
    let mut d = 1;
    while d < max && pixels / d as f64 > step {
        d *= 2;
    }
    d as u8
}

pub(crate) fn make(mesh: &Mesh, sizes: &[f64], step: f64, max: u32) -> Plan {
    let levels: Vec<_> = sizes.iter().map(|&p| desired(p, step, max)).collect();
    let f = levels.len();
    let mut edge_levels = vec![1u32; mesh.control.edges as usize];
    for (i, &level) in levels.iter().enumerate() {
        for k in 0..3 {
            let e = mesh.control.words[3 * f + 3 * i + k] as usize;
            edge_levels[e] = edge_levels[e].max(level as u32);
        }
    }
    let mut templates = HashMap::new();
    let mut records = Vec::new();
    let mut visible_faces = 0;
    let mut counts = [0; 4];
    for (i, &level) in levels.iter().enumerate() {
        counts[level.trailing_zeros() as usize] += 1;
        let edges =
            std::array::from_fn(|k| edge_levels[mesh.control.words[3 * f + 3 * i + k] as usize]);
        let t = templates
            .entry((level, edges))
            .or_insert_with(|| template(level as u32, edges, max));
        if sizes[i] > 0. {
            visible_faces += t.len();
        }
        records.extend(t.iter().map(|t| [i as u32, t[0], t[1], t[2]]));
    }
    Plan {
        levels: Arc::new(levels),
        records,
        visible_faces,
        counts,
        limited: false,
    }
}

fn memory_fits(mesh: &Mesh, plan: &Plan, max: u32) -> bool {
    let (v, e, f) = (
        mesh.vertices.len() as u64,
        mesh.control.edges as u64,
        (mesh.indices.len() / 3) as u64,
    );
    let limit = 128 * 1024 * 1024;
    let mut total = mesh.control.words.len() as u64 * 4 + plan.records.len() as u64 * 16;
    if mesh.control.words.len() as u64 * 4 > limit || plan.records.len() as u64 * 16 > limit {
        return false;
    }
    let mut d = 1u64;
    loop {
        let n = v + e * (d - 1) + f * (d - 1) * d.saturating_sub(2) / 2;
        let faces = if d == max as u64 {
            plan.records.len() as u64
        } else {
            f * d * d
        };
        let b = [n * 32, faces * 12, (n + 3 * faces) * 4];
        if b.iter().any(|&b| b > limit) {
            return false;
        }
        total += b.iter().sum::<u64>();
        if d == max as u64 {
            total += n * 32;
            break;
        }
        d *= 2;
    }
    total <= 384 * 1024 * 1024
}

pub fn for_view(mesh: &Arc<Mesh>, origin: [f64; 2], camera: &Camera, requested: u32) -> Arc<Mesh> {
    let sizes = sizes(mesh, origin, camera);
    let mut max = requested;
    let mut limited = false;
    loop {
        let mut step = 24.;
        let mut plan = make(mesh, &sizes, step, max);
        while plan.visible_faces > 2_000_000 && step < 1000000. {
            step *= 1.25;
            plan = make(mesh, &sizes, step, max);
        }
        let actual = plan.levels.iter().copied().max().unwrap_or(1) as u32;
        // 最大密度が不要なら生成格子自体も縮める。
        if actual < max {
            max = actual;
            continue;
        }
        if max > 1 && !memory_fits(mesh, &plan, max) {
            max /= 2;
            limited = true;
            continue;
        }
        plan.limited = limited;
        let mut out = (**mesh).clone();
        out.divisions = max;
        out.adaptive = Some(Arc::new(plan));
        return Arc::new(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates_cover_face_and_keep_every_shared_edge_step() {
        let unpack = |v: u32| [(v & 15) as i32, (v >> 4) as i32];
        for d in [1, 2, 4, 8] {
            for a in [d, 8] {
                for b in [d, 8] {
                    for c in [d, 8] {
                        let t = template(d, [a, b, c], 8);
                        let mut area = 0;
                        let mut edges = HashMap::new();
                        for face in t {
                            let p = face.map(unpack);
                            let n = cross(p[0], p[1], p[2]);
                            assert!(n > 0);
                            area += n;
                            for k in 0..3 {
                                let mut pair = [p[k], p[(k + 1) % 3]];
                                pair.sort();
                                *edges.entry(pair).or_insert(0) += 1;
                            }
                        }
                        assert_eq!(area, 64);
                        let boundary: Vec<_> = edges
                            .iter()
                            .filter(|(_, n)| **n == 1)
                            .map(|(p, _)| *p)
                            .collect();
                        assert_eq!(boundary.len(), (a + b + c) as usize);
                        for [p, q] in boundary {
                            let expected = if p[1] == 0 && q[1] == 0 {
                                8 / a
                            } else if p[0] == 0 && q[0] == 0 {
                                8 / c
                            } else {
                                8 / b
                            };
                            assert_eq!(
                                (p[0] - q[0]).abs().max((p[1] - q[1]).abs()),
                                expected as i32
                            );
                        }
                        assert!(edges.values().all(|&n| n <= 2));
                    }
                }
            }
        }
    }
    #[test]
    fn small_distant_faces_do_not_lower_large_near_faces() {
        assert_eq!(desired(400., 24., 8), 8);
        assert_eq!(desired(10., 24., 8), 1);
        assert_eq!(desired(70., 24., 8), 4);
        assert_eq!(desired(0., 24., 8), 1);
    }
    #[test]
    fn tilted_camera_selects_different_density_by_projected_size() {
        let vertices = [
            [-200., 0., 0.],
            [200., 0., 0.],
            [0., 0., 400.],
            [-200., 0., -50000.],
            [200., 0., -50000.],
            [0., 0., -49600.],
        ]
        .into_iter()
        .map(|p| crate::geometry::Vertex { base: p, curved: p })
        .collect();
        let indices = vec![0, 1, 2, 3, 4, 5];
        let mesh = Arc::new(Mesh {
            vertices,
            control: crate::gpu_loop::control(6, &indices),
            indices,
            source_faces: 2,
            tiles: 1,
            divisions: 8,
            adaptive: None,
            levels: vec![14],
        });
        let mut camera = Camera::default();
        camera.center = [0., 0.];
        camera.height = 0.;
        camera.distance = 2000.;
        camera.pitch = 0.3;
        let projected = sizes(&mesh, [0., 0.], &camera);
        assert!(projected[0] > projected[1] * 4. && projected[1] > 0.);
        let result = for_view(&mesh, [0., 0.], &camera, 8);
        let levels = &result.adaptive.as_ref().unwrap().levels;
        assert_eq!(&***levels, &[8, 1]);
    }
}
