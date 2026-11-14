//! 画面占有面積に応じて分割し、隣接辺は同じ全球格子を使う。
use crate::{selection::Camera, shared_data::WORLD};
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Debug)]
pub struct Patch {
    pub bounds: [f32; 4],
    pub edges: [f32; 4],
    pub origins: [f32; 4],
}
pub struct Plan {
    pub patches: Vec<Patch>,
    pub groups: Vec<(u32, std::ops::Range<u32>)>,
}
pub struct Input {
    pub origin: [f64; 2],
    pub side: f64,
    pub n: u32,
    pub density: Vec<u32>,
}
struct Item {
    tile: usize,
    x: f64,
    y: f64,
    side: f64,
    n: u32,
    min: u32,
    pixels: f64,
    edges: [f32; 4],
    origins: [f32; 4],
}
pub fn plan(inputs: &[Input], camera: &Camera) -> Vec<Plan> {
    let mut items = Vec::new();
    let latitude = (std::f64::consts::PI * (1. - 2. * camera.center[1] / WORLD))
        .sinh()
        .atan();
    let scale = latitude.cos();
    for (tile, t) in inputs.iter().enumerate() {
        if camera
            .visible_depth([
                t.origin[0],
                t.origin[1],
                t.origin[0] + t.side,
                t.origin[1] + t.side,
            ])
            .is_none()
        {
            continue;
        }
        // 粗い評価格子を1024個の小パッチへ分解しない。各パッチには少なくとも8区間をまとめる。
        let k = ((t.side / 2000.).ceil().max(1.) as u32)
            .next_power_of_two()
            .clamp(1, 32)
            .min((t.n / 8).max(1));
        let side = t.side / k as f64;
        let max = (t.n / k.min(8)).max(1);
        for y in 0..k {
            for x in 0..k {
                let ox = t.origin[0] + x as f64 * side;
                let oy = t.origin[1] + y as f64 * side;
                let Some(depth) = camera.visible_depth([ox, oy, ox + side, oy + side]) else {
                    continue;
                };
                let pixels = side * scale * camera.viewport[1]
                    / (2. * camera.tan_half_fov() * depth.max(0.2));
                let mut density = 0u32;
                for cy in y * 32 / k..((y + 1) * 32).div_ceil(k) {
                    for cx in x * 32 / k..((x + 1) * 32).div_ceil(k) {
                        density =
                            density.max(*t.density.get((cy * 32 + cx) as usize).unwrap_or(&0));
                    }
                }
                let required = ((side / (t.side / 32.)) * f64::from(density).sqrt())
                    .ceil()
                    .max(1.) as u32;
                let min = required.next_power_of_two().min(max);
                let n = ((pixels / camera.mesh_pixels).ceil().clamp(1., max as f64) as u32)
                    .next_power_of_two()
                    .clamp(min, max);
                items.push(Item {
                    tile,
                    x: ox,
                    y: oy,
                    side,
                    n,
                    min,
                    pixels,
                    edges: [0.; 4],
                    origins: [0.; 4],
                });
            }
        }
    }
    // ヒープ更新はO(log パッチ数)。局所制御点密度による分割下限を守る。
    let mut heap = std::collections::BinaryHeap::new();
    let mut triangles = 0u64;
    for (i, p) in items.iter().enumerate() {
        triangles += 2 * u64::from(p.n).pow(2);
        if p.n > p.min {
            heap.push((
                std::cmp::Reverse((p.pixels / p.n as f64 * 1024.).max(0.) as u64),
                i,
                p.n,
            ));
        }
    }
    while triangles > 2_000_000 {
        let Some((_, i, n)) = heap.pop() else { break };
        let p = &mut items[i];
        if n != p.n || p.n <= p.min {
            continue;
        }
        triangles -= 2 * u64::from(p.n).pow(2);
        p.n /= 2;
        triangles += 2 * u64::from(p.n).pow(2);
        if p.n > p.min {
            heap.push((
                std::cmp::Reverse((p.pixels / p.n as f64 * 1024.).max(0.) as u64),
                i,
                p.n,
            ));
        }
    }
    // 同一直線上の辺を走査し、部分接合も拾う。分割間隔は2の累乗にそろえる。
    let mut lines = std::collections::BTreeMap::<(u8, i64), Vec<(usize, usize, f64, f64)>>::new();
    for (i, p) in items.iter().enumerate() {
        for (e, axis, line, start) in [
            (0, 0, p.x, p.y),
            (1, 0, p.x + p.side, p.y),
            (2, 1, p.y, p.x),
            (3, 1, p.y + p.side, p.x),
        ] {
            lines
                .entry((axis, (line * 1000.).round() as i64))
                .or_default()
                .push((i, e, start, start + p.side));
        }
    }
    for edges in lines.values_mut() {
        edges.sort_by(|a, b| a.2.total_cmp(&b.2));
        let mut parent = (0..edges.len()).collect::<Vec<_>>();
        fn root(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i]
            }
            i
        }
        for a in 0..edges.len() {
            for b in a + 1..edges.len() {
                if edges[b].2 >= edges[a].3 - 1e-5 {
                    break;
                }
                if edges[a].1 ^ 1 == edges[b].1 && edges[b].3 > edges[a].2 + 1e-5 {
                    let ra = root(&mut parent, a);
                    let rb = root(&mut parent, b);
                    parent[ra] = rb;
                }
            }
        }
        let mut groups = std::collections::BTreeMap::<usize, (f64, f64)>::new();
        for (j, &(i, _, start, _)) in edges.iter().enumerate() {
            let r = root(&mut parent, j);
            let step = items[i].side / items[i].n as f64;
            let g = groups.entry(r).or_insert((start, step));
            g.0 = g.0.min(start);
            g.1 = g.1.max(step);
        }
        for (j, &(i, e, _, _)) in edges.iter().enumerate() {
            let r = root(&mut parent, j);
            let (origin, step) = groups[&r];
            items[i].edges[e] = step as f32;
            // 1m未満の精度を保つため、所属タイル原点からの相対座標で保持する。
            items[i].origins[e] =
                (origin - inputs[items[i].tile].origin[if e < 2 { 1 } else { 0 }]) as f32;
        }
    }
    inputs
        .iter()
        .enumerate()
        .map(|(tile, t)| {
            let mut selected = items.iter().filter(|p| p.tile == tile).collect::<Vec<_>>();
            selected.sort_by_key(|p| p.n);
            let mut patches = Vec::new();
            let mut groups: Vec<(u32, std::ops::Range<u32>)> = Vec::new();
            for p in selected {
                let id = patches.len() as u32;
                patches.push(Patch {
                    bounds: [
                        (p.x - t.origin[0]) as f32,
                        (p.y - t.origin[1]) as f32,
                        p.side as f32,
                        p.n as f32,
                    ],
                    edges: p.edges,
                    origins: p.origins,
                });
                if let Some((n, range)) = groups.last_mut() {
                    if *n == p.n {
                        range.end = id + 1;
                        continue;
                    }
                }
                groups.push((p.n, id..id + 1));
            }
            Plan { patches, groups }
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn input(x: f64, y: f64, side: f64, n: u32) -> Input {
        Input {
            origin: [x, y],
            side,
            n,
            density: vec![0; 1024],
        }
    }
    #[test]
    fn mesh_edges_target_three_screen_pixels() {
        let center = crate::selection::lonlat(138.7274, 35.3606);
        let t = input(center[0] - 500., center[1] - 500., 1000., 512);
        let camera = Camera {
            center,
            distance: 10000.,
            pitch: 1.56,
            ..Default::default()
        };
        let depth = camera
            .visible_depth([
                t.origin[0],
                t.origin[1],
                t.origin[0] + t.side,
                t.origin[1] + t.side,
            ])
            .unwrap();
        let pixels =
            t.side * camera.scale() * camera.viewport[1] / (2. * camera.tan_half_fov() * depth);
        let old = ((pixels / 6.).ceil() as u32).next_power_of_two();
        let plans = plan(&[t], &camera);
        assert_eq!(plans[0].patches.len(), 1);
        let n = plans[0].groups[0].0;
        assert!(n >= old * 2, "分割数が増えていない: old={old} new={n}");
        assert!(pixels / f64::from(n) <= 3.);
    }

    #[test]
    fn coarse_overview_does_not_explode_into_tiny_patches() {
        let mut tile = input(0., 0., 600000., 32);
        tile.density.fill(4);
        let p = plan(
            &[tile],
            &Camera {
                center: [300000.; 2],
                distance: 2000000.,
                pitch: 1.4,
                ..Default::default()
            },
        );
        assert!(!p[0].patches.is_empty());
        assert!(p[0].patches.len() <= 16);
        let triangles = p[0]
            .groups
            .iter()
            .map(|(n, r)| 2 * u64::from(*n).pow(2) * u64::from(r.end - r.start))
            .sum::<u64>();
        assert!(triangles <= 2 * 32 * 32);
    }
    #[test]
    #[ignore = "広域表示のパッチ計画を手元で計測する"]
    fn benchmark_zoomed_out_patch_planning() {
        let center = crate::selection::lonlat(138.7274, 35.3606);
        let side = WORLD / 64.;
        let inputs = (-3..=3)
            .flat_map(|y| {
                (-3..=3).map(move |x| {
                    let mut tile = input(
                        center[0] + x as f64 * side,
                        center[1] + y as f64 * side,
                        side,
                        32,
                    );
                    tile.density.fill(4);
                    tile
                })
            })
            .collect::<Vec<_>>();
        let camera = Camera {
            center,
            distance: 1500000.,
            pitch: 0.9,
            ..Default::default()
        };
        let mut elapsed = Vec::new();
        let mut patches = 0;
        for _ in 0..8 {
            let start = std::time::Instant::now();
            let plans = plan(&inputs, &camera);
            elapsed.push(start.elapsed().as_secs_f64() * 1000.);
            patches = plans.iter().map(|p| p.patches.len()).sum::<usize>();
        }
        elapsed.sort_by(f64::total_cmp);
        eprintln!(
            "広域計画 {}タイル / {}パッチ / 中央値 {:.2} ms",
            inputs.len(),
            patches,
            elapsed[4]
        );
    }
    #[test]
    fn partial_edges_share_lattice() {
        let camera = Camera {
            center: [8000., 4000.],
            distance: 20000.,
            pitch: 1.4,
            ..Default::default()
        };
        let plans = plan(
            &[
                input(0., 0., 8000., 128),
                input(8000., 0., 4000., 64),
                input(8000., 4000., 4000., 16),
            ],
            &camera,
        );
        assert!(!plans[0].patches.is_empty());
        for p in &plans[0].patches {
            if (p.bounds[0] + p.bounds[2] - 8000.).abs() < 0.1 {
                for (j, other) in plans[1..].iter().enumerate() {
                    for q in &other.patches {
                        let y = q.bounds[1] + if j == 0 { 0. } else { 4000. };
                        if q.bounds[0] == 0.
                            && y < p.bounds[1] + p.bounds[2]
                            && y + q.bounds[2] > p.bounds[1]
                        {
                            assert_eq!(p.edges[1], q.edges[0]);
                            let delta =
                                (p.origins[1] - q.origins[0] - if j == 0 { 0. } else { 4000. })
                                    / p.edges[1];
                            assert!((delta - delta.round()).abs() < 0.001);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn density_floor_survives_budget() {
        let mut t = input(0., 0., 8000., 512);
        t.density.fill(256);
        let p = plan(
            &[t],
            &Camera {
                center: [4000., 4000.],
                distance: 20000.,
                pitch: 1.4,
                ..Default::default()
            },
        );
        assert!(!p[0].patches.is_empty());
        assert!(p[0].groups.iter().all(|(n, _)| *n >= 64));
    }
}
