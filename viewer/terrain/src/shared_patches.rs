//! 画面占有面積に応じて分割し、隣接辺は同じ全球格子を使う。
use crate::{selection::Camera, shared_data::WORLD};
#[repr(C)]
#[derive(
    Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Debug, serde::Serialize, serde::Deserialize,
)]
pub struct Patch {
    pub bounds: [f32; 4],
    pub edges: [f32; 4],
    pub origins: [f32; 4],
    pub lines: [f32; 4],
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Plan {
    pub patches: Vec<Patch>,
    pub groups: Vec<(u32, std::ops::Range<u32>)>,
}
pub struct Input {
    pub origin: [f64; 2],
    pub side: f64,
    pub n: u32,
    pub covered_by: Vec<[f64; 4]>,
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
        let Some(depth) = camera.visible_depth([
            t.origin[0],
            t.origin[1],
            t.origin[0] + t.side,
            t.origin[1] + t.side,
        ]) else {
            continue;
        };
        // 評価格子が細かくても、画面上で小さなタイルを1024パッチには分けない。
        let pixels = t.side * scale * camera.viewport[1] / (2. * camera.tan_half_fov() * depth);
        let screen_k = ((pixels / 128.).ceil().clamp(1., 32.) as u32).next_power_of_two();
        let k = ((t.side / 2000.).ceil().max(1.) as u32)
            .next_power_of_two()
            .clamp(1, 32)
            .min((t.n / 8).max(1))
            .min(screen_k);
        let side = t.side / k as f64;
        let mut pending = Vec::new();
        for y in 0..k {
            for x in 0..k {
                pending.push((
                    t.origin[0] + x as f64 * side,
                    t.origin[1] + y as f64 * side,
                    side,
                ));
            }
        }
        while let Some((ox, oy, side)) = pending.pop() {
            let Some(depth) = camera.visible_depth([ox, oy, ox + side, oy + side]) else {
                continue;
            };
            let overlaps = t
                .covered_by
                .iter()
                .filter(|b| {
                    ox < b[2] - 0.001
                        && ox + side > b[0] + 0.001
                        && oy < b[3] - 0.001
                        && oy + side > b[1] + 0.001
                })
                .collect::<Vec<_>>();
            if overlaps.iter().any(|b| {
                ox >= b[0] - 0.001
                    && oy >= b[1] - 0.001
                    && ox + side <= b[2] + 0.001
                    && oy + side <= b[3] + 0.001
            }) {
                continue;
            }
            if !overlaps.is_empty() && side > 0.002 {
                let half = side / 2.;
                for y in 0..2 {
                    for x in 0..2 {
                        pending.push((ox + x as f64 * half, oy + y as f64 * half, half));
                    }
                }
                continue;
            }
            let pixels =
                side * scale * camera.viewport[1] / (2. * camera.tan_half_fov() * depth.max(0.2));
            let max = (((t.n / k.min(8)).max(1) as f64 * side / (t.side / k as f64))
                .ceil()
                .max(1.) as u32)
                .next_power_of_two()
                .min(512);
            // 保存点の密度は画面上の描画下限にしない。縮小時は投影間隔で減らす。
            let min = 1;
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
    // 接合端点のための局所分割は一段にとどめる。反復すると細かい辺が全域へ伝播する。
    {
        let components = edge_components(&items, inputs);
        let mut split = vec![false; items.len()];
        for group in &components {
            let shortest = group
                .iter()
                .map(|&(i, _, _)| items[i].side)
                .fold(f64::INFINITY, f64::min);
            for &(i, _, _) in group {
                if items[i].side / f64::from(items[i].n) > shortest * 1.001 {
                    split[i] = true;
                }
            }
        }

        let mut next = Vec::new();
        for (i, p) in items.into_iter().enumerate() {
            if !split[i] {
                next.push(p);
                continue;
            }
            let half = p.side / 2.;
            for y in 0..2 {
                for x in 0..2 {
                    next.push(Item {
                        tile: p.tile,
                        x: p.x + x as f64 * half,
                        y: p.y + y as f64 * half,
                        side: half,
                        n: (p.n / 2).max(1),
                        min: 1,
                        pixels: p.pixels / 2.,
                        edges: [0.; 4],
                        origins: [0.; 4],
                    });
                }
            }
        }
        items = next;
    }
    let components = edge_components(&items, inputs);
    // 部分接合の端点が格子頂点になる最小数だけを保持する。
    for group in &components {
        let shortest = group
            .iter()
            .map(|&(i, _, _)| items[i].side)
            .fold(f64::INFINITY, f64::min);
        for &(i, _, _) in group {
            let required = ((items[i].side / shortest).round() as u32)
                .next_power_of_two()
                .min(512);
            items[i].min = items[i].min.max(required);
            items[i].n = items[i].n.max(required);
        }
    }
    // ヒープ更新はO(log パッチ数)。接合端点に必要な下限だけを守る。
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
    // 予算適用後の粗い側の格子へ辺をそろえる。評価密度へ内部全体を引き上げない。
    for group in components {
        let origin = group
            .iter()
            .map(|&(_, _, start)| start)
            .fold(f64::INFINITY, f64::min);
        let shortest = group
            .iter()
            .map(|&(i, _, _)| items[i].side)
            .fold(f64::INFINITY, f64::min);
        let draw_step = group
            .iter()
            .map(|&(i, _, _)| items[i].side / f64::from(items[i].n))
            .fold(0., f64::max);
        let field_step = group
            .iter()
            .map(|&(i, _, _)| inputs[items[i].tile].side / f64::from(inputs[items[i].tile].n))
            .fold(0., f64::max);
        let cross_tile = group
            .iter()
            .any(|&(i, _, _)| items[i].tile != items[group[0].0].tile);
        let step = if cross_tile {
            field_step.max(draw_step)
        } else {
            draw_step
        }
        .min(shortest);
        for (i, e, _) in group {
            items[i].edges[e] = step as f32;
            items[i].origins[e] = (origin - camera.center[if e < 2 { 1 } else { 0 }]) as f32;
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
                    lines: [
                        (p.x - camera.center[0]) as f32,
                        (p.x + p.side - camera.center[0]) as f32,
                        (p.y - camera.center[1]) as f32,
                        (p.y + p.side - camera.center[1]) as f32,
                    ],
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
// 重なっている代替面を除外し、同じ直線上の部分接合もまとめる。
fn edge_components(items: &[Item], inputs: &[Input]) -> Vec<Vec<(usize, usize, f64)>> {
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
    let mut components = Vec::new();
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
                    let ta = items[edges[a].0].tile;
                    let tb = items[edges[b].0].tile;
                    let tile_a = &inputs[ta];
                    let tile_b = &inputs[tb];
                    // 重ねて描く背景の内部辺は、詳細タイルの接合相手ではない。
                    // これを結ぶと背景の巨大な格子が詳細面の辺へ伝播してしまう。
                    if ta != tb
                        && tile_a.covered_by.is_empty()
                        && tile_b.covered_by.is_empty()
                        && (0..2).all(|axis| {
                            (tile_a.origin[axis] + tile_a.side)
                                .min(tile_b.origin[axis] + tile_b.side)
                                - tile_a.origin[axis].max(tile_b.origin[axis])
                                > 0.01
                        })
                    {
                        continue;
                    }
                    let ra = root(&mut parent, a);
                    let rb = root(&mut parent, b);
                    parent[ra] = rb;
                }
            }
        }
        let mut groups = std::collections::BTreeMap::<usize, Vec<(usize, usize, f64)>>::new();
        for (j, &(i, e, start, _)) in edges.iter().enumerate() {
            groups
                .entry(root(&mut parent, j))
                .or_default()
                .push((i, e, start));
        }
        components.extend(groups.into_values());
    }
    components
}
#[cfg(test)]
mod tests {
    use super::*;
    fn input(x: f64, y: f64, side: f64, n: u32) -> Input {
        Input {
            origin: [x, y],
            side,
            n,
            covered_by: Vec::new(),
        }
    }
    #[test]
    fn distant_dense_fields_do_not_create_thousands_of_small_patches() {
        let center = crate::selection::lonlat(138.7274, 35.3606);
        let side = WORLD / 64.;
        let tile = input(center[0] - side / 2., center[1] - side / 2., side, 512);
        let mut c = Camera {
            center,
            distance: 10000000.,
            pitch: 1.3,
            ..Default::default()
        };
        let far = plan(&[tile], &c);
        let count = far[0].patches.len();
        assert!(
            count > 0 && count <= 4,
            "遠方の評価区画を過剰に小分けしている: {count}"
        );
        c.distance = 1000000.;
        let tile = input(center[0] - side / 2., center[1] - side / 2., side, 512);
        let near = plan(&[tile], &c);
        assert!(
            near[0].patches.len() > count,
            "近づいたときに局所分割へ戻らない"
        );
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
        let tile = input(0., 0., 600000., 32);
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
                    let tile = input(
                        center[0] + x as f64 * side,
                        center[1] + y as f64 * side,
                        side,
                        32,
                    );
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
    fn final_mesh_budget_includes_cross_tile_stitching() {
        let center = crate::selection::lonlat(138.7274, 35.3606);
        let inputs = (-4..4)
            .flat_map(|y| {
                (-4..4).map(move |x| {
                    input(
                        center[0] + x as f64 * 4000.,
                        center[1] + y as f64 * 4000.,
                        4000.,
                        512,
                    )
                })
            })
            .collect::<Vec<_>>();
        let plans = plan(
            &inputs,
            &Camera {
                center,
                distance: 30000.,
                pitch: 1.4,
                mesh_pixels: 0.2,
                ..Default::default()
            },
        );
        let triangles = plans
            .iter()
            .flat_map(|p| &p.groups)
            .map(|(n, r)| 2 * u64::from(*n).pow(2) * u64::from(r.end - r.start))
            .sum::<u64>();
        assert!(triangles <= 2_000_000, "接合後に予算を超えた: {triangles}");
    }

    #[test]
    #[ignore = "保存スナップショットの分割計画をGPUなしで計測する"]
    fn benchmark_external_patch_plan() {
        let path = std::env::var("YAMAMUKI_SNAPSHOT_TEST_INPUT").unwrap();
        #[derive(serde::Deserialize)]
        struct SavedMesh {
            key: crate::shared_data::Key,
            field: crate::shared_field::Field,
            n: u32,
        }
        #[derive(serde::Deserialize)]
        struct Saved {
            meshes: Vec<SavedMesh>,
            counts: [usize; 4],
            camera: Camera,
        }
        let snapshot: Saved = serde_json::from_reader(std::fs::File::open(path).unwrap()).unwrap();
        let bounds = snapshot
            .meshes
            .iter()
            .map(|m| m.field.world_bounds(m.key))
            .collect::<Vec<_>>();
        let inputs = snapshot
            .meshes
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let b = bounds[i];
                let covered_by = if i < snapshot.counts[0] {
                    Vec::new()
                } else {
                    bounds
                        .iter()
                        .enumerate()
                        .filter(|&(j, c)| {
                            j != i
                                && (j < snapshot.counts[0]
                                    || snapshot.meshes[j].key.0 > m.key.0
                                    || (snapshot.meshes[j].key.0 == m.key.0
                                        && c[2] - c[0] < b[2] - b[0]))
                                && c[0] < b[2]
                                && c[2] > b[0]
                                && c[1] < b[3]
                                && c[3] > b[1]
                        })
                        .map(|(_, c)| *c)
                        .collect()
                };
                Input {
                    origin: [b[0], b[1]],
                    side: b[2] - b[0],
                    n: m.n,
                    covered_by,
                }
            })
            .collect::<Vec<_>>();
        let start = std::time::Instant::now();
        let plans = plan(&inputs, &snapshot.camera);
        let triangles = plans
            .iter()
            .flat_map(|p| &p.groups)
            .map(|(n, r)| 2 * u64::from(*n).pow(2) * u64::from(r.end - r.start))
            .sum::<u64>();
        eprintln!(
            "分割計画 {:?}: {}パッチ / {triangles}三角形",
            start.elapsed(),
            plans.iter().map(|p| p.patches.len()).sum::<usize>()
        );
        assert!(triangles <= 2_000_000);
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
                            let delta = (p.origins[1] - q.origins[0]) / p.edges[1];
                            assert!((delta - delta.round()).abs() < 0.001);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn tile_boundary_grid_tracks_screen_refinement_without_exceeding_budget() {
        let inputs = [
            input(0., 0., 8000., 64),
            input(8000., 0., 4000., 128),
            input(8000., 4000., 4000., 128),
        ];
        let mut previous = 0.;
        for (pixels, distance) in [(0.75, 40000.), (3., 45000.), (12., 60000.)] {
            let camera = Camera {
                center: [8000., 4000.],
                distance,
                pitch: 1.4,
                mesh_pixels: pixels,
                ..Default::default()
            };
            let plans = plan(&inputs, &camera);
            let steps = plans[0]
                .patches
                .iter()
                .filter(|p| p.lines[1].abs() < 0.01)
                .map(|p| p.edges[1].to_bits())
                .collect::<std::collections::BTreeSet<_>>();
            assert!(!steps.is_empty());
            let step = f32::from_bits(*steps.first().unwrap());
            assert!(step >= previous, "縮小しても接合格子が粗くならない");
            previous = step;
        }
    }
    #[test]
    fn background_is_partitioned_at_partial_foreground_boundaries() {
        let camera = Camera {
            center: [4000., 4000.],
            distance: 2_000_000.,
            pitch: 1.4,
            ..Default::default()
        };
        let mut background = input(0., 0., 8000., 32);
        background.covered_by = vec![[2000., 2000., 3000., 3000.], [3000., 2000., 4000., 3000.]];
        let plans = plan(
            &[
                input(2000., 2000., 1000., 128),
                input(3000., 2000., 1000., 32),
                background,
            ],
            &camera,
        );
        let area = plans[2]
            .patches
            .iter()
            .map(|p| f64::from(p.bounds[2]).powi(2))
            .sum::<f64>();
        assert!(
            (area - 62_000_000.).abs() < 0.1,
            "背景の面積に穴や重なりがある: {area}"
        );
        for patch in &plans[2].patches {
            let x = patch.bounds[0];
            let y = patch.bounds[1];
            let side = patch.bounds[2];
            assert!(
                !(x < 4000. && x + side > 2000. && y < 3000. && y + side > 2000.),
                "背景の三角形が詳細タイルをまたいでいる"
            );
        }
    }
    #[test]
    fn fine_edge_endpoints_are_vertices_of_the_coarse_edge() {
        let camera = Camera {
            center: [8000., 2000.],
            distance: 2_000_000.,
            pitch: 1.4,
            ..Default::default()
        };
        let plans = plan(
            &[
                input(0., 0., 8000., 32),
                input(8000., 0., 1000., 128),
                input(8000., 1000., 1000., 128),
            ],
            &camera,
        );
        let mut pairs = 0;
        for coarse in &plans[0].patches {
            if (coarse.lines[1] - 0.).abs() > 0.01 {
                continue;
            }
            for fine in plans[1..].iter().flat_map(|p| &p.patches) {
                let lo = coarse.lines[2].max(fine.lines[2]);
                let hi = coarse.lines[3].min(fine.lines[3]);
                if hi <= lo {
                    continue;
                }
                pairs += 1;
                assert_eq!(coarse.lines[1], fine.lines[0], "境界のXYが一致しない");
                let step = coarse.edges[1];
                assert_eq!(step, fine.edges[0]);
                for endpoint in [fine.lines[2], fine.lines[3]] {
                    let t = (endpoint - coarse.origins[1]) / step;
                    assert!(
                        (t - t.round()).abs() < 1e-5,
                        "短い辺の端点が粗い線分の途中にある: {t}"
                    );
                    let vertex =
                        (endpoint - coarse.lines[2]) / (coarse.bounds[2] / coarse.bounds[3]);
                    assert!(
                        (vertex - vertex.round()).abs() < 1e-5,
                        "粗い側に接合頂点がない: {vertex}"
                    );
                }
            }
        }
        assert!(pairs >= 2);
    }
    #[test]
    fn overlapping_background_does_not_coarsen_foreground_edges() {
        let camera = Camera {
            center: [4000., 4000.],
            distance: 2_000_000.,
            pitch: 1.4,
            ..Default::default()
        };
        let alone = plan(&[input(2000., 2000., 1000., 128)], &camera);
        let combined = plan(
            &[input(2000., 2000., 1000., 128), input(0., 0., 8000., 32)],
            &camera,
        );
        assert!(!alone[0].patches.is_empty());
        assert_eq!(alone[0].patches.len(), combined[0].patches.len());
        for (a, b) in alone[0].patches.iter().zip(&combined[0].patches) {
            assert_eq!(a.edges, b.edges, "背景の格子が詳細面に混入した");
            assert_eq!(a.origins, b.origins);
        }
    }
    #[test]
    fn dense_control_points_do_not_prevent_screen_space_reduction() {
        let t = input(0., 0., 8000., 512);
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
        let triangles = |plans: &[Plan]| {
            plans
                .iter()
                .flat_map(|p| &p.groups)
                .map(|(n, r)| 2 * u64::from(*n).pow(2) * u64::from(r.end - r.start))
                .sum::<u64>()
        };
        let far = input(0., 0., 8000., 512);
        let distant = plan(
            &[far],
            &Camera {
                center: [4000.; 2],
                distance: 2_000_000.,
                pitch: 1.4,
                ..Default::default()
            },
        );
        assert!(triangles(&distant) * 16 < triangles(&p));
        assert!(triangles(&p) <= 2_000_000);
    }
}
