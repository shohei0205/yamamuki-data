// 角度補修と全格子の誤差検証もRust内で行う。標高は元の格子から取得する。
use super::{Builder, Face, Point, orient};
use anyhow::{Result, bail, ensure};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashSet};

pub struct QualityMesh {
    pub points: Vec<Point>,
    pub faces: Vec<Face>,
    pub max_error: f64,
    pub minimum_angle: f64,
    pub rounds: usize,
}

pub fn minimum_angle(p: &[Point], f: Face) -> f64 {
    let mut minimum = 180f64;
    for i in 0..3 {
        let a = p[f[i]];
        let b = p[f[(i + 1) % 3]];
        let c = p[f[(i + 2) % 3]];
        let u = [(b[0] - a[0]) as f64, (b[1] - a[1]) as f64];
        let v = [(c[0] - a[0]) as f64, (c[1] - a[1]) as f64];
        minimum = minimum.min(
            ((u[0] * v[0] + u[1] * v[1]) / (u[0].hypot(u[1]) * v[0].hypot(v[1])))
                .clamp(-1., 1.)
                .acos()
                .to_degrees(),
        );
    }
    minimum
}

fn triangulate(p: &[Point], n: usize) -> Result<Vec<Face>> {
    let xy = p
        .iter()
        .map(|p| delaunator::Point {
            x: p[0] as f64,
            y: p[1] as f64,
        })
        .collect::<Vec<_>>();
    let mesh = delaunator::triangulate(&xy);
    let mut area = 0i128;
    let mut result = Vec::with_capacity(mesh.triangles.len() / 3);
    for t in mesh.triangles.chunks_exact(3) {
        let mut f = [t[0], t[1], t[2]];
        let mut a = orient(p[f[0]], p[f[1]], p[f[2]]);
        if a < 0 {
            f.swap(1, 2);
            a = -a;
        }
        ensure!(a > 0, "退化した面を検出しました");
        area += a;
        result.push(f);
    }
    ensure!(
        area == 2 * (n - 1).pow(2) as i128,
        "基本TINの被覆面積が一致しません"
    );
    Ok(result)
}

// 各面で誤差最大の点を評価し、範囲を変更せず元の全格子と比較する。
pub fn audit(
    p: &[Point],
    faces: &[Face],
    grid: &[f64],
    n: usize,
    tolerance: f64,
) -> Result<(f64, Vec<Point>)> {
    audit_with_limits(p, faces, grid, n, tolerance, None)
}
fn audit_with_limits(
    p: &[Point],
    faces: &[Face],
    grid: &[f64],
    n: usize,
    tolerance: f64,
    scales: Option<&[f64]>,
) -> Result<(f64, Vec<Point>)> {
    // 面順を維持して候補を結合し、補修頂点の追加順を変えない。
    let results = faces
        .par_chunks(64)
        .map(|part| audit_faces_serial(p, part, grid, n, tolerance, scales))
        .collect::<Vec<_>>();
    let mut maximum = 0f64;
    let mut additions = Vec::new();
    for result in results {
        let (value, extra) = result?;
        maximum = maximum.max(value);
        additions.extend(extra);
    }
    Ok((maximum, additions))
}
fn audit_faces_serial(
    p: &[Point],
    faces: &[Face],
    grid: &[f64],
    n: usize,
    tolerance: f64,
    scales: Option<&[f64]>,
) -> Result<(f64, Vec<Point>)> {
    let mut maximum = 0f64;
    let mut additions = Vec::new();
    for &f in faces {
        let [a, b, c] = f.map(|i| p[i]);
        let den = orient(a, b, c) as f64;
        ensure!(den > 0., "反転した三角形を検出しました");
        let za = grid[a[1] as usize * n + a[0] as usize];
        let db = grid[b[1] as usize * n + b[0] as usize] - za;
        let dc = grid[c[1] as usize * n + c[0] as usize] - za;
        let mut best = 0f64;
        let mut sample = a;
        for y in a[1].min(b[1]).min(c[1])..=a[1].max(b[1]).max(c[1]) {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for (u, v) in [(a, b), (b, c), (c, a)] {
                if y < u[1].min(v[1]) || y > u[1].max(v[1]) {
                    continue;
                }
                if u[1] == v[1] {
                    lo = lo.min(u[0].min(v[0]) as f64);
                    hi = hi.max(u[0].max(v[0]) as f64);
                } else {
                    let x = u[0] as f64
                        + (y - u[1]) as f64 * (v[0] - u[0]) as f64 / (v[1] - u[1]) as f64;
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
            for x in
                ((lo - 1e-8).ceil() as i64).max(0)..=((hi + 1e-8).floor() as i64).min(n as i64 - 1)
            {
                let q = [x, y];
                let wb = orient(c, a, q);
                let wc = orient(a, b, q);
                if wb < 0 || wc < 0 || wb + wc > den as i128 {
                    continue;
                }
                let predicted = za + wb as f64 / den * db + wc as f64 / den * dc;
                let raw = (predicted - grid[y as usize * n + x as usize]).abs();
                let effective = scales
                    .map(|s| {
                        (((tolerance + 0.125) * s[y as usize * n + x as usize] - 0.125).max(12.375))
                            / tolerance
                    })
                    .unwrap_or(1.);
                let error = raw / effective;
                if error > best {
                    best = error;
                    sample = q;
                }
            }
        }
        maximum = maximum.max(best);
        if best > tolerance + 1e-6 {
            additions.push(sample);
        }
    }
    Ok((maximum, additions))
}

fn quality_candidates(p: &[Point], faces: &[Face], n: usize, minimum: f64) -> Vec<Point> {
    let existing = p.iter().copied().collect::<HashSet<_>>();
    let mut segments = Vec::new();
    for (axis, value) in [(0, 0), (0, n as i64 - 1), (1, 0), (1, n as i64 - 1)] {
        let mut edge = p
            .iter()
            .copied()
            .filter(|p| p[axis] == value)
            .collect::<Vec<_>>();
        edge.sort_by_key(|p| p[1 - axis]);
        segments.extend(edge.windows(2).map(|s| (s[0], s[1])));
    }
    let mut bad = faces
        .iter()
        .copied()
        .map(|f| (minimum_angle(p, f), f))
        .filter(|v| v.0 < minimum - 1e-7)
        .collect::<Vec<_>>();
    bad.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut candidates = BTreeMap::<Point, f64>::new();
    for &(angle, f) in &bad {
        let [a, b, c] = f.map(|i| p[i]);
        let ux = (b[0] - a[0]) as f64;
        let uy = (b[1] - a[1]) as f64;
        let vx = (c[0] - a[0]) as f64;
        let vy = (c[1] - a[1]) as f64;
        let det = 2. * (ux * vy - uy * vx);
        let (uu, vv) = (ux * ux + uy * uy, vx * vx + vy * vy);
        let mut q = [
            a[0] as f64 + (uu * vy - vv * uy) / det,
            a[1] as f64 + (ux * vv - vx * uu) / det,
        ];
        let encroached = segments
            .iter()
            .enumerate()
            .filter(|(_, (a, b))| {
                (q[0] - a[0] as f64) * (q[0] - b[0] as f64)
                    + (q[1] - a[1] as f64) * (q[1] - b[1] as f64)
                    < -1e-8
            })
            .map(|v| v.0)
            .collect::<Vec<_>>();
        if !encroached.is_empty() || q.iter().any(|&v| v <= 0. || v >= (n - 1) as f64) {
            let selected = if encroached.is_empty() {
                segments
                    .iter()
                    .enumerate()
                    .min_by(|(_, (a, b)), (_, (c, d))| {
                        let distance = |(a, b): &(Point, Point)| {
                            let u = [(b[0] - a[0]) as f64, (b[1] - a[1]) as f64];
                            let t = (((q[0] - a[0] as f64) * u[0] + (q[1] - a[1] as f64) * u[1])
                                / (u[0] * u[0] + u[1] * u[1]))
                                .clamp(0., 1.);
                            (q[0] - a[0] as f64 - t * u[0]).hypot(q[1] - a[1] as f64 - t * u[1])
                        };
                        distance(&(*a, *b)).total_cmp(&distance(&(*c, *d)))
                    })
                    .map(|v| v.0)
                    .unwrap()
            } else {
                *encroached
                    .iter()
                    .max_by_key(|&&i| {
                        let (a, b) = segments[i];
                        (b[0] - a[0]).pow(2) + (b[1] - a[1]).pow(2)
                    })
                    .unwrap()
            };
            let (a, b) = segments[selected];
            q = [(a[0] + b[0]) as f64 / 2., (a[1] + b[1]) as f64 / 2.];
        }
        let xy = q.map(|v| v.round_ties_even() as i64);
        if xy.iter().all(|&v| v >= 0 && v < n as i64) && !existing.contains(&xy) {
            candidates
                .entry(xy)
                .and_modify(|v| *v = v.min(angle))
                .or_insert(angle);
        }
        if candidates.len() >= 2048 {
            break;
        }
    }
    if candidates.is_empty() {
        for (_, f) in bad {
            let edges = [(p[f[0]], p[f[1]]), (p[f[1]], p[f[2]]), (p[f[2]], p[f[0]])];
            let (a, b) = edges
                .into_iter()
                .max_by_key(|(a, b)| (a[0] - b[0]).pow(2) + (a[1] - b[1]).pow(2))
                .unwrap();
            let xy = [
                ((a[0] + b[0]) as f64 / 2.).round_ties_even() as i64,
                ((a[1] + b[1]) as f64 / 2.).round_ties_even() as i64,
            ];
            if !existing.contains(&xy) {
                candidates.insert(xy, 0.);
            }
        }
    }
    let mut out = candidates.into_iter().collect::<Vec<_>>();
    out.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    out.into_iter().map(|v| v.0).collect()
}

pub fn generate(grid: &[f64], n: usize, tolerance: f64, minimum: f64) -> Result<QualityMesh> {
    generate_with_spatial_seeds(grid, n, tolerance, minimum, None)
}
// 余白の16ビット座標に収まる面幅を確保するための形状補助点。保存境界は固定しない。
pub fn generate_with_spatial_seeds(
    grid: &[f64],
    n: usize,
    tolerance: f64,
    minimum: f64,
    seed_step: Option<usize>,
) -> Result<QualityMesh> {
    generate_with_spatial_seeds_progress(grid, n, tolerance, minimum, seed_step, |_, _, _| Ok(()))
}
pub fn generate_with_spatial_seeds_progress(
    grid: &[f64],
    n: usize,
    tolerance: f64,
    minimum: f64,
    seed_step: Option<usize>,
    mut progress: impl FnMut(&str, usize, usize) -> Result<()>,
) -> Result<QualityMesh> {
    let builder;
    if let Some(step) = seed_step {
        ensure!(step > 0, "形状補助格子の間隔が不正です");
        let axis = (0..n)
            .step_by(step)
            .chain(std::iter::once(n - 1))
            .collect::<std::collections::BTreeSet<_>>();
        let seeds = axis
            .iter()
            .flat_map(|&y| axis.iter().map(move |&x| [x, y]))
            .collect::<Vec<_>>();
        progress("seed-topology", seeds.len(), 0)?;
        builder = Builder::seeded(grid, n, n, &seeds, grid.len())?;
    } else {
        builder = Builder::new(grid, n, n)?;
    }
    progress("greedy-insertion", 0, 0)?;
    let mesh = builder.generate_with_minimum(grid.len(), tolerance, 4)?;
    ensure!(mesh.tolerance_reached, "初期TINの誤差条件を満たしません");
    let p = mesh
        .vertices
        .iter()
        .map(|p| [p[0] as i64, p[1] as i64])
        .collect::<Vec<_>>();
    let f = triangulate(&p, n)?;
    repair_incremental(grid, n, p, f, tolerance, minimum, || false, None, progress)
}

// 親・共有辺・元の頂点を保持したまま、追加だけで品質を補修する。
pub fn refine_seeded(
    grid: &[f64],
    n: usize,
    p: Vec<Point>,
    tolerance: f64,
    minimum: f64,
    stop: impl Fn() -> bool,
) -> Result<QualityMesh> {
    refine_seeded_with_limits(grid, n, p, tolerance, minimum, stop, None)
}
pub fn refine_seeded_with_limits(
    grid: &[f64],
    n: usize,
    p: Vec<Point>,
    tolerance: f64,
    minimum: f64,
    stop: impl Fn() -> bool,
    scales: Option<&[f64]>,
) -> Result<QualityMesh> {
    ensure!(
        scales.is_none_or(|s| s.len() == grid.len()),
        "局所誤差係数のサイズが不正です"
    );
    ensure!(
        grid.len() == n * n && n >= 2 && p.len() >= 4,
        "階層補修の格子・頂点が不正です"
    );
    let selected = p.iter().copied().collect::<HashSet<_>>();
    ensure!(
        selected.len() == p.len() && p.iter().all(|p| p.iter().all(|&v| v >= 0 && v < n as i64)),
        "階層補修の頂点が重複または範囲外です"
    );
    let f = triangulate(&p, n)?;
    repair_incremental(
        grid,
        n,
        p,
        f,
        tolerance,
        minimum,
        stop,
        scales,
        |_, _, _| Ok(()),
    )
}

// 初回だけ全体の位相を作り、その後はBowyer–Watsonの変更面だけを更新する。
fn repair_incremental(
    grid: &[f64],
    n: usize,
    points: Vec<Point>,
    faces: Vec<Face>,
    tolerance: f64,
    minimum: f64,
    stop: impl Fn() -> bool,
    scales: Option<&[f64]>,
    mut progress: impl FnMut(&str, usize, usize) -> Result<()>,
) -> Result<QualityMesh> {
    let mut builder = Builder::new_cow(std::borrow::Cow::Borrowed(grid), n, n, true)?;
    builder.points = points;
    builder.selected.fill(false);
    for p in &builder.points {
        builder.selected[p[1] as usize * n + p[0] as usize] = true;
    }
    builder.triangles.clear();
    builder.edges.clear();
    builder.heap.clear();
    builder.next_id = 0;
    // 面IDは不変。削除した面のキャッシュだけ失効させる。
    let mut cache = BTreeMap::<usize, (Face, f64, Option<(f64, Option<Point>)>)>::new();
    for f in faces {
        let id = builder.next_id;
        builder.add(f)?;
        cache.insert(id, (f, minimum_angle(&builder.points, f), None));
    }
    for round in 0..10000 {
        ensure!(!stop(), "階層補修を停止しました");
        progress("quality-repair", builder.points.len(), round)?;
        let angle = cache.values().map(|v| v.1).fold(180f64, f64::min);
        let additions = if angle < minimum - 1e-7 {
            let bad = cache
                .values()
                .filter(|v| v.1 < minimum - 1e-7)
                .map(|v| v.0)
                .collect::<Vec<_>>();
            quality_candidates(&builder.points, &bad, n, minimum)
        } else {
            let pending = cache
                .iter()
                .filter(|(_, v)| v.2.is_none())
                .map(|(&id, v)| (id, v.0))
                .collect::<Vec<_>>();
            let evaluated = pending
                .par_iter()
                .map(|&(id, f)| {
                    let (error, extra) =
                        audit_faces_serial(&builder.points, &[f], grid, n, tolerance, scales)?;
                    Ok((id, error, extra.first().copied()))
                })
                .collect::<Result<Vec<_>>>()?;
            for (id, error, point) in evaluated {
                cache.get_mut(&id).unwrap().2 = Some((error, point));
            }
            let maximum = cache.values().map(|v| v.2.unwrap().0).fold(0f64, f64::max);
            if maximum <= tolerance + 1e-6 {
                return Ok(QualityMesh {
                    points: builder.points,
                    faces: cache.into_values().map(|v| v.0).collect(),
                    max_error: maximum,
                    minimum_angle: angle,
                    rounds: round,
                });
            }
            cache.values().filter_map(|v| v.2.unwrap().1).collect()
        };
        let before = builder.points.len();
        for xy in additions {
            let sample = xy[1] as usize * n + xy[0] as usize;
            if builder.selected[sample] {
                continue;
            }
            ensure!(
                builder.points.len() < u32::MAX as usize,
                "階層補修が32ビットの頂点数上限に達しました"
            );
            let mut id = builder.next_id - 1;
            let mut walked = 0;
            loop {
                let f = builder.triangles[&id].face;
                if let Some((a, b)) = [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])]
                    .into_iter()
                    .find(|&(a, b)| orient(builder.points[a], builder.points[b], xy) < 0)
                {
                    id = *builder
                        .edges
                        .get(&(b, a))
                        .ok_or_else(|| anyhow::anyhow!("補修点の面探索が範囲外です"))?;
                    walked += 1;
                    ensure!(
                        walked <= builder.triangles.len(),
                        "補修点の面探索が収束しません"
                    );
                } else {
                    break;
                }
            }
            let change = builder
                .insert_recorded(
                    super::Entry {
                        triangle: id,
                        sample,
                        error: 0.,
                    },
                    true,
                )?
                .unwrap();
            for id in change.removed {
                cache.remove(&id);
            }
            for f in change.added {
                cache.insert(
                    f.id,
                    (f.vertices, minimum_angle(&builder.points, f.vertices), None),
                );
            }
        }
        ensure!(
            builder.points.len() > before,
            "誤差・角度の補修候補がありません（最小角{angle}度）"
        );
    }
    bail!("誤差と角度の補修が収束しません")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plane_keeps_four_vertices() {
        let grid = (0..33 * 33)
            .map(|i| (i % 33 + i / 33) as f64)
            .collect::<Vec<_>>();
        let m = generate(&grid, 33, 1., 10.).unwrap();
        assert_eq!(m.points.len(), 4);
        assert!(m.max_error < 1e-8);
    }
    #[test]
    fn peaks_and_random_fields_meet_both_conditions() {
        for seed in 0..8 {
            let grid = (0..33 * 33)
                .map(|i| {
                    let x = i % 33;
                    let y = i / 33;
                    ((i * (97 + seed * 2)) % 101) as f64 * 0.25
                        + if (x as i32 - 16).pow(2) + (y as i32 - 16).pow(2) < 36 {
                            30.
                        } else {
                            0.
                        }
                })
                .collect::<Vec<_>>();
            let m = generate(&grid, 33, 12.5, 10.).unwrap();
            assert!(m.minimum_angle >= 10. - 1e-7);
            assert!(audit(&m.points, &m.faces, &grid, 33, 12.5).unwrap().0 <= 12.5 + 1e-6);
        }
    }
}

#[cfg(test)]
mod parallel_audit_tests {
    use super::*;
    #[test]
    fn error_candidates_keep_exact_values_and_order() {
        let n = 65;
        let grid = (0..n * n)
            .map(|i| ((i % n) as f64 * 0.31).sin() * 100. + ((i / n) as f64 * 0.17).cos() * 70.)
            .collect::<Vec<_>>();
        let points = (0..=8)
            .flat_map(|y| (0..=8).map(move |x| [x * 8, y * 8]))
            .collect::<Vec<_>>();
        let faces = triangulate(&points, n).unwrap();
        let scales = (0..n * n)
            .map(|i| 0.5 + (i % 17) as f64 / 34.)
            .collect::<Vec<_>>();
        for limits in [None, Some(scales.as_slice())] {
            assert_eq!(
                audit_faces_serial(&points, &faces, &grid, n, 25., limits).unwrap(),
                audit_with_limits(&points, &faces, &grid, n, 25., limits).unwrap()
            );
        }
    }
}

#[cfg(test)]
mod incremental_repair_tests {
    use super::*;
    #[test]
    fn repaired_mesh_covers_region_and_retains_seeds_and_is_deterministic() {
        let n = 65;
        let grid = (0..n * n)
            .map(|i| {
                let x = (i % n) as f64;
                let y = (i / n) as f64;
                200. * (-((x - 29.).powi(2) + (y - 35.).powi(2)) / 90.).exp()
                    + ((i * 97) % 101) as f64 * 0.05
            })
            .collect::<Vec<_>>();
        let seeds = vec![
            [0, 0],
            [64, 0],
            [64, 64],
            [0, 64],
            [31, 32],
            [32, 32],
            [33, 32],
        ];
        let limits = vec![0.6; grid.len()];
        let a = refine_seeded_with_limits(
            &grid,
            n,
            seeds.clone(),
            12.375,
            10.,
            || false,
            Some(&limits),
        )
        .unwrap();
        let b = refine_seeded_with_limits(
            &grid,
            n,
            seeds.clone(),
            12.375,
            10.,
            || false,
            Some(&limits),
        )
        .unwrap();
        assert_eq!(a.points, b.points);
        assert_eq!(a.faces, b.faces);
        assert!(seeds.iter().all(|v| a.points.contains(v)));
        assert!(a.minimum_angle >= 10. - 1e-7);
        assert!(
            audit_faces_serial(&a.points, &a.faces, &grid, n, 12.375, Some(&limits))
                .unwrap()
                .0
                <= 12.375 + 1e-6
        );
        let area = a
            .faces
            .iter()
            .map(|f| orient(a.points[f[0]], a.points[f[1]], a.points[f[2]]))
            .sum::<i128>();
        assert_eq!(area, 2 * 64 * 64);
        let mut edges = std::collections::HashMap::new();
        for f in &a.faces {
            assert!(orient(a.points[f[0]], a.points[f[1]], a.points[f[2]]) > 0);
            for j in 0..3 {
                assert!(
                    edges
                        .insert((f[j], f[(j + 1) % 3]), f[(j + 2) % 3])
                        .is_none()
                );
            }
        }
        for (&(u, v), &w) in &edges {
            if let Some(&q) = edges.get(&(v, u)) {
                assert!(
                    super::super::incircle(a.points[u], a.points[v], a.points[w], a.points[q]) <= 0
                );
            }
        }
    }
}
