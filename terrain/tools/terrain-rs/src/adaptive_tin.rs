//! Garland–Heckbert の逐次最大誤差挿入。格子座標で局所Delaunay更新を行う。
use anyhow::{Result, ensure};
use serde::Serialize;
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
};

type Point = [i64; 2];
type Face = [usize; 3];

fn orient(a: Point, b: Point, c: Point) -> i128 {
    (b[0] - a[0]) as i128 * (c[1] - a[1]) as i128 - (b[1] - a[1]) as i128 * (c[0] - a[0]) as i128
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brute(builder: &Builder<'_>) -> (f64, usize) {
        let mut best = (0., 0);
        for y in 0..builder.height {
            for x in 0..builder.width {
                let sample = y * builder.width + x;
                if builder.selected[sample] {
                    continue;
                }
                let p = [x as i64, y as i64];
                let mut covered = false;
                for t in builder.triangles.values() {
                    let [a, b, c] = t.face.map(|i| builder.points[i]);
                    let (u, v, w) = (orient(b, c, p), orient(c, a, p), orient(a, b, p));
                    if u < 0 || v < 0 || w < 0 {
                        continue;
                    }
                    covered = true;
                    let za = builder.height_at(a);
                    let predicted = za
                        + (builder.height_at(b) - za) * v as f64 / orient(a, b, c) as f64
                        + (builder.height_at(c) - za) * w as f64 / orient(a, b, c) as f64;
                    let error = (builder.grid[sample] - predicted).abs();
                    if error > best.0 {
                        best = (error, sample);
                    }
                    break;
                }
                assert!(covered, "未被覆の標高格子 {x},{y}");
            }
        }
        best
    }

    fn validate(builder: &Builder<'_>) {
        let mut area = 0;
        for (&id, t) in &builder.triangles {
            let [a, b, c] = t.face.map(|i| builder.points[i]);
            let size = orient(a, b, c);
            assert!(size > 0);
            area += size;
            for &p in &builder.points {
                assert!(incircle(a, b, c, p) <= 0, "Delaunay条件違反");
            }
            for (a, b) in [
                (t.face[0], t.face[1]),
                (t.face[1], t.face[2]),
                (t.face[2], t.face[0]),
            ] {
                assert_eq!(builder.edges[&(a, b)], id);
            }
        }
        assert_eq!(
            area,
            2 * (builder.width - 1) as i128 * (builder.height - 1) as i128
        );
        let mut points = HashSet::new();
        for p in &builder.points {
            assert!(points.insert(*p));
        }
    }

    #[test]
    fn flat_and_plane_start_with_four_corners() {
        for (width, height) in [(2, 2), (2, 19), (23, 2), (17, 13)] {
            let z: Vec<_> = (0..width * height)
                .map(|i| 7. + (i % width) as f64 * 2. + (i / width) as f64 * 3.)
                .collect();
            let builder = Builder::new(&z, width, height).unwrap();
            assert_eq!(builder.triangles.len(), 2);
            validate(&builder);
            let mesh = builder.generate(100, 1e-10).unwrap();
            assert_eq!(mesh.vertices.len(), 4);
            assert!(mesh.tolerance_reached);
        }
    }

    #[test]
    fn each_insert_is_global_greedy_and_only_local_errors_change() {
        let (w, h) = (17, 13);
        let z: Vec<_> = (0..w * h).map(|i| ((i * 173 + 19) % 251) as f64).collect();
        let mut b = Builder::new(&z, w, h).unwrap();
        for _ in 0..80 {
            let entry = b.worst();
            let expected = brute(&b);
            assert!((entry.error - expected.0).abs() < 1e-9);
            let old: HashSet<_> = b.triangles.keys().copied().collect();
            let before = b.evaluations;
            b.insert(entry).unwrap();
            validate(&b);
            let new = b.triangles.keys().filter(|id| !old.contains(id)).count();
            assert_eq!(
                b.evaluations - before,
                new,
                "変更していない三角形を再評価しています"
            );
        }
    }

    #[test]
    fn boundary_peaks_cocircular_points_and_full_resolution() {
        for (w, h) in [(2, 9), (9, 2), (7, 7), (8, 5)] {
            let z: Vec<_> = (0..w * h).map(|i| ((i * 37 + 11) % 101) as f64).collect();
            let mut b = Builder::new(&z, w, h).unwrap();
            while b.points.len() < w * h && b.worst().error > 0. {
                let e = b.worst();
                b.insert(e).unwrap();
                validate(&b);
            }
            assert!(brute(&b).0 < 1e-10);
        }
    }

    #[test]
    fn tolerance_and_vertex_budget_are_reported_independently() {
        let mut z = vec![0.; 81];
        z[40] = 100.;
        let a = Builder::new(&z, 9, 9).unwrap().generate(4, 1.).unwrap();
        assert_eq!(a.max_error, 100.);
        assert!(!a.tolerance_reached);
        assert_eq!(a.stop_reason, "maxVertices");
        let b = Builder::new(&z, 9, 9).unwrap().generate(81, 0.01).unwrap();
        assert!(b.max_error <= 0.01);
        assert_eq!(b.stop_reason, "tolerance");
        assert!(b.vertices.len() <= 81);
    }

    #[test]
    fn invalid_and_nonfinite_inputs_are_rejected() {
        assert!(Builder::new(&[0.; 4], 1, 4).is_err());
        assert!(Builder::new(&[0.; 3], 2, 2).is_err());
        assert!(Builder::new(&[0., 0., f64::NAN, 0.], 2, 2).is_err());
        assert!(
            Builder::new(&[0.; 4], 2, 2)
                .unwrap()
                .generate(3, 1.)
                .is_err()
        );
        assert!(
            Builder::new(&[0.; 4], 2, 2)
                .unwrap()
                .generate(4, f64::NAN)
                .is_err()
        );
    }

    #[test]
    fn minimum_vertices_refine_even_after_reaching_tolerance() {
        let z: Vec<_> = (0..289)
            .map(|i| ((i * 31 + i / 17 * 19) % 151) as f64)
            .collect();
        let coarse = Builder::new(&z, 17, 17)
            .unwrap()
            .generate(289, 1000.)
            .unwrap();
        let detailed = Builder::new(&z, 17, 17)
            .unwrap()
            .generate_with_minimum(289, 1000., 128)
            .unwrap();
        assert_eq!(coarse.vertices.len(), 4);
        assert_eq!(detailed.vertices.len(), 128);
        assert!(detailed.minimum_vertices_reached && detailed.tolerance_reached);
        assert!(detailed.max_error < coarse.max_error);
        assert!(
            Builder::new(&z, 17, 17)
                .unwrap()
                .generate_with_minimum(64, 100., 128)
                .is_err()
        );
    }

    #[test]
    fn minimum_vertices_do_not_add_points_to_an_exact_plane() {
        let z = vec![20.; 289];
        let mesh = Builder::new(&z, 17, 17)
            .unwrap()
            .generate_with_minimum(289, 100., 128)
            .unwrap();
        assert_eq!(mesh.vertices.len(), 4);
        assert!(mesh.tolerance_reached);
        assert!(!mesh.minimum_vertices_reached);
        assert_eq!(mesh.stop_reason, "exactSurface");
    }

    #[test]
    fn integer_predicates_handle_large_and_cocircular_coordinates() {
        let n = 1_048_575;
        assert!(incircle([0, 0], [n, 0], [n, n], [n / 2, n / 2]) > 0);
        assert_eq!(incircle([0, 0], [n, 0], [n, n], [0, n]), 0);
        assert_eq!(orient([0, 0], [n, n], [n / 2, n / 2]), 0);
    }

    #[test]
    fn output_is_deterministic() {
        let z: Vec<_> = (0..81).map(|i| ((i * 173) % 47) as f64).collect();
        let a = Builder::new(&z, 9, 9).unwrap().generate(30, 1.).unwrap();
        let b = Builder::new(&z, 9, 9).unwrap().generate(30, 1.).unwrap();
        assert_eq!(a.vertices, b.vertices);
        assert_eq!(a.triangles, b.triangles);
    }

    #[test]
    fn many_random_insertions_keep_a_valid_delaunay_partition() {
        for seed in 0..32 {
            let mut rng = seed + 1u64;
            let z: Vec<_> = (0..63)
                .map(|_| {
                    rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                    (rng >> 32) as f64
                })
                .collect();
            let mut b = Builder::new(&z, 9, 7).unwrap();
            while b.points.len() < 63 && b.worst().error > 0. {
                let e = b.worst();
                b.insert(e).unwrap();
                validate(&b);
            }
            assert!(brute(&b).0 < 1e-5);
        }
    }

    #[test]
    fn seamless_tiles_choose_the_same_shared_edge_vertices() {
        let w = 17;
        let h = 17;
        let a: Vec<_> = (0..w * h)
            .map(|i| ((i % w * 17 + i / w * 31) % 93) as f64)
            .collect();
        let mut b: Vec<_> = (0..w * h)
            .map(|i| ((i % w * 11 + i / w * 23) % 71) as f64)
            .collect();
        for y in 0..h {
            b[y * w] = a[y * w + w - 1];
        }
        for tolerance in [5., 20., 50.] {
            let left = Builder::new(&a, w, h)
                .unwrap()
                .generate_seamless_with_minimum(w * h, tolerance, 128)
                .unwrap();
            let right = Builder::new(&b, w, h)
                .unwrap()
                .generate_seamless_with_minimum(w * h, tolerance, 128)
                .unwrap();
            let mut edge_a: Vec<_> = left
                .vertices
                .iter()
                .filter(|p| p[0] == (w - 1) as f64)
                .map(|p| (p[1] as usize, p[2] as i64))
                .collect();
            let mut edge_b: Vec<_> = right
                .vertices
                .iter()
                .filter(|p| p[0] == 0.)
                .map(|p| (p[1] as usize, p[2] as i64))
                .collect();
            edge_a.sort();
            edge_b.sort();
            assert_eq!(edge_a, edge_b);
            assert!(left.max_error <= tolerance && right.max_error <= tolerance);
        }
    }
}

// 反時計回りの三角形に対して正なら円内。同一円上では変更しない。
fn incircle(a: Point, b: Point, c: Point, p: Point) -> i128 {
    let (ax, ay) = ((a[0] - p[0]) as i128, (a[1] - p[1]) as i128);
    let (bx, by) = ((b[0] - p[0]) as i128, (b[1] - p[1]) as i128);
    let (cx, cy) = ((c[0] - p[0]) as i128, (c[1] - p[1]) as i128);
    (ax * ax + ay * ay) * (bx * cy - by * cx) - (bx * bx + by * by) * (ax * cy - ay * cx)
        + (cx * cx + cy * cy) * (ax * by - ay * bx)
}

#[derive(Clone, Debug)]
struct Triangle {
    face: Face,
    error: f64,
    sample: usize,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    error: f64,
    triangle: usize,
    sample: usize,
}
impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.error
            .total_cmp(&other.error)
            .then_with(|| other.sample.cmp(&self.sample))
            .then_with(|| other.triangle.cmp(&self.triangle))
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<Face>,
    pub max_error: f64,
    pub tolerance_reached: bool,
    pub minimum_vertices_reached: bool,
    pub stop_reason: &'static str,
    pub triangle_evaluations: usize,
    pub sample_evaluations: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct FaceRecord {
    pub id: usize,
    pub vertices: Face,
}
#[derive(Debug, Serialize)]
pub struct Insertion {
    pub removed: Vec<usize>,
    pub added: Vec<FaceRecord>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressiveStage {
    pub vertex_count: usize,
    pub step_count: usize,
    pub max_error: f64,
    pub faces: Vec<FaceRecord>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressiveTin {
    pub schema_version: u32,
    pub vertices: Vec<[f64; 3]>,
    pub base: ProgressiveStage,
    pub stages: Vec<ProgressiveStage>,
    pub steps: Vec<Insertion>,
}

pub struct Builder<'a> {
    grid: &'a [f64],
    width: usize,
    height: usize,
    points: Vec<Point>,
    selected: Vec<bool>,
    lock_boundary: bool,
    boundary_error: f64,
    triangles: HashMap<usize, Triangle>,
    edges: HashMap<(usize, usize), usize>,
    heap: BinaryHeap<Entry>,
    next_id: usize,
    evaluations: usize,
    samples: usize,
}

impl<'a> Builder<'a> {
    pub fn new(grid: &'a [f64], width: usize, height: usize) -> Result<Self> {
        ensure!(
            width >= 2 && height >= 2 && width <= 1_048_576 && height <= 1_048_576,
            "幅と高さは2以上1048576以下が必要です"
        );
        ensure!(
            width.checked_mul(height) == Some(grid.len()),
            "標高配列の長さが一致しません"
        );
        ensure!(
            grid.iter().all(|v| v.is_finite()),
            "欠損値・無限大のない標高マップが必要です"
        );
        let range = grid
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &z| {
                (lo.min(z), hi.max(z))
            });
        ensure!(
            (range.1 - range.0).is_finite(),
            "標高の差が浮動小数点の範囲を超えます"
        );
        let mut result = Self {
            grid,
            width,
            height,
            points: vec![
                [0, 0],
                [(width - 1) as i64, 0],
                [(width - 1) as i64, (height - 1) as i64],
                [0, (height - 1) as i64],
            ],
            selected: vec![false; grid.len()],
            lock_boundary: false,
            boundary_error: 0.,
            triangles: HashMap::new(),
            edges: HashMap::new(),
            heap: BinaryHeap::new(),
            next_id: 0,
            evaluations: 0,
            samples: 0,
        };
        for p in &result.points {
            result.selected[p[1] as usize * width + p[0] as usize] = true;
        }
        result.add([0, 1, 2])?;
        result.add([0, 2, 3])?;
        Ok(result)
    }

    fn height_at(&self, p: Point) -> f64 {
        self.grid[p[1] as usize * self.width + p[0] as usize]
    }

    fn evaluate(&mut self, face: Face) -> Triangle {
        self.evaluations += 1;
        let [a, b, c] = face.map(|i| self.points[i]);
        let area = orient(a, b, c) as f64;
        let za = self.height_at(a);
        let db = self.height_at(b) - za;
        let dc = self.height_at(c) - za;
        let low = a[1].min(b[1]).min(c[1]);
        let high = a[1].max(b[1]).max(c[1]).min(self.height as i64 - 1);
        let mut best = Triangle {
            face,
            error: 0.,
            sample: 0,
        };
        // 各行と三角形の交差区間だけ走査。境界の包含判定は整数で行う。
        for y in low..=high {
            let mut left = f64::INFINITY;
            let mut right = f64::NEG_INFINITY;
            for (p, q) in [(a, b), (b, c), (c, a)] {
                if y < p[1].min(q[1]) || y > p[1].max(q[1]) {
                    continue;
                }
                if p[1] == q[1] {
                    left = left.min(p[0].min(q[0]) as f64);
                    right = right.max(p[0].max(q[0]) as f64);
                } else {
                    let x = p[0] as f64
                        + (y - p[1]) as f64 * (q[0] - p[0]) as f64 / (q[1] - p[1]) as f64;
                    left = left.min(x);
                    right = right.max(x);
                }
            }
            if !left.is_finite() {
                continue;
            }
            // 交点の丸めで格子点を落とさないよう1点だけ外へ広げる。
            let lo = (left.floor() as i64 - 1).max(0);
            let hi = (right.ceil() as i64 + 1).min(self.width as i64 - 1);
            for x in lo..=hi {
                let p = [x, y];
                let e0 = orient(a, b, p);
                let e1 = orient(b, c, p);
                let e2 = orient(c, a, p);
                if e0 < 0 || e1 < 0 || e2 < 0 {
                    continue;
                }
                let sample = y as usize * self.width + x as usize;
                if self.selected[sample]
                    || (self.lock_boundary
                        && (x == 0
                            || y == 0
                            || x == self.width as i64 - 1
                            || y == self.height as i64 - 1))
                {
                    continue;
                }
                self.samples += 1;
                let error = ((self.grid[sample] - za)
                    - (e2 as f64 / area * db + e0 as f64 / area * dc))
                    .abs();
                if error > best.error || (error == best.error && sample < best.sample) {
                    best.error = error;
                    best.sample = sample;
                }
            }
        }
        best
    }

    fn add(&mut self, face: Face) -> Result<()> {
        ensure!(
            orient(
                self.points[face[0]],
                self.points[face[1]],
                self.points[face[2]]
            ) > 0,
            "面積ゼロまたは裏返った三角形です"
        );
        let triangle = self.evaluate(face);
        let id = self.next_id;
        self.next_id += 1;
        for (a, b) in [(face[0], face[1]), (face[1], face[2]), (face[2], face[0])] {
            ensure!(
                self.edges.insert((a, b), id).is_none(),
                "有向辺が重複しています"
            );
        }
        self.heap.push(Entry {
            error: triangle.error,
            triangle: id,
            sample: triangle.sample,
        });
        self.triangles.insert(id, triangle);
        Ok(())
    }

    fn worst(&mut self) -> Entry {
        while !self
            .triangles
            .contains_key(&self.heap.peek().expect("三角形のヒープ").triangle)
        {
            self.heap.pop();
        }
        *self.heap.peek().unwrap()
    }

    fn insert(&mut self, entry: Entry) -> Result<()> {
        self.insert_recorded(entry, false).map(|_| ())
    }

    fn insert_recorded(&mut self, entry: Entry, record: bool) -> Result<Option<Insertion>> {
        let p = [
            (entry.sample % self.width) as i64,
            (entry.sample / self.width) as i64,
        ];
        ensure!(
            !self.selected[entry.sample],
            "挿入済みの頂点が再選択されました"
        );
        // Bowyer–Watson: 追加点を外接円に含む接続領域だけを置き換える。
        let mut cavity = HashSet::new();
        let mut visited = HashSet::new();
        let mut stack = vec![entry.triangle];
        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            let f = self.triangles[&id].face;
            if id != entry.triangle
                && incircle(self.points[f[0]], self.points[f[1]], self.points[f[2]], p) <= 0
            {
                continue;
            }
            cavity.insert(id);
            for (a, b) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
                if let Some(&other) = self.edges.get(&(b, a)) {
                    stack.push(other);
                }
            }
        }
        let mut boundary = Vec::new();
        for &id in &cavity {
            let f = self.triangles[&id].face;
            for (a, b) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
                if !self
                    .edges
                    .get(&(b, a))
                    .is_some_and(|other| cavity.contains(other))
                {
                    boundary.push((a, b));
                }
            }
        }
        boundary.sort_unstable(); // ハッシュ表の順序に依存しない生成結果。
        let mut removed = if record {
            cavity.iter().copied().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        removed.sort_unstable();
        let first_added = self.next_id;
        for id in cavity {
            let f = self.triangles.remove(&id).unwrap().face;
            for edge in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
                self.edges.remove(&edge);
            }
        }
        let new = self.points.len();
        self.points.push(p);
        self.selected[entry.sample] = true;
        for (a, b) in boundary {
            let area = orient(self.points[a], self.points[b], p);
            ensure!(area >= 0, "Delaunay更新領域が不正です");
            // 外周の辺上に挿入する場合は縮退面を作らない。
            if area > 0 {
                self.add([a, b, new])?;
            }
        }
        // 失効済み項目を遅延破棄。長時間の生成でもヒープをO(面数)に保つ。
        if self.heap.len() > self.triangles.len() * 4 + 64 {
            self.heap = self
                .triangles
                .iter()
                .map(|(&id, t)| Entry {
                    error: t.error,
                    triangle: id,
                    sample: t.sample,
                })
                .collect();
        }
        Ok(if record {
            Some(Insertion {
                removed,
                added: (first_added..self.next_id)
                    .map(|id| FaceRecord {
                        id,
                        vertices: self.triangles[&id].face,
                    })
                    .collect(),
            })
        } else {
            None
        })
    }

    /// 隣接区画と同じ境界標高列から固定頂点を選び、その後に最大誤差挿入する。
    pub fn generate_seamless(self, max_vertices: usize, tolerance: f64) -> Result<Mesh> {
        self.generate_seamless_with_minimum(max_vertices, tolerance, 4)
    }
    pub fn generate_seamless_with_minimum(
        mut self,
        max_vertices: usize,
        tolerance: f64,
        min_vertices: usize,
    ) -> Result<Mesh> {
        self.prepare_boundary(max_vertices, tolerance, min_vertices > 4)?;
        self.generate_with_minimum(max_vertices, tolerance, min_vertices)
    }

    fn prepare_boundary(&mut self, max_vertices: usize, tolerance: f64, lock: bool) -> Result<()> {
        ensure!(
            tolerance.is_finite() && tolerance >= 0.,
            "許容誤差が不正です"
        );
        let n = self.width;
        let h = self.height;
        let sides = [
            (0..n).collect::<Vec<_>>(),
            (0..h).map(|y| y * n + n - 1).collect(),
            (0..n).map(|x| (h - 1) * n + x).collect(),
            (0..h).map(|y| y * n).collect(),
        ];
        let mut seeds = std::collections::BTreeSet::new();
        let mut boundary_error = 0f64;
        for side in sides {
            let mut stack = vec![(0, side.len() - 1)];
            while let Some((a, b)) = stack.pop() {
                if b - a < 2 {
                    continue;
                }
                let mut worst = 0.;
                let mut at = a;
                for i in a + 1..b {
                    let predicted =
                        (self.grid[side[b]] - self.grid[side[a]]) * (i - a) as f64 / (b - a) as f64;
                    let error = ((self.grid[side[i]] - self.grid[side[a]]) - predicted).abs();
                    if error > worst {
                        worst = error;
                        at = i;
                    }
                }
                // 挿入後の面内補間の丸めで境界点が追加選択されないよう余裕を取る。
                if worst > tolerance * (1. - 1e-10) {
                    seeds.insert(side[at]);
                    stack.push((a, at));
                    stack.push((at, b));
                } else {
                    boundary_error = boundary_error.max(worst);
                }
            }
        }
        ensure!(
            max_vertices >= seeds.len() + 4,
            "境界の固定頂点が最大頂点数を超えます"
        );
        for sample in seeds {
            let p = [(sample % n) as i64, (sample / n) as i64];
            let mut id = *self.triangles.keys().min().unwrap();
            let mut count = 0;
            loop {
                let f = self.triangles[&id].face;
                let outside = [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])]
                    .into_iter()
                    .find(|&(a, b)| orient(self.points[a], self.points[b], p) < 0);
                if let Some((a, b)) = outside {
                    id = *self
                        .edges
                        .get(&(b, a))
                        .ok_or_else(|| anyhow::anyhow!("境界点が区画外です"))?;
                    count += 1;
                    ensure!(count <= self.triangles.len(), "境界点の探索が収束しません");
                } else {
                    break;
                }
            }
            self.insert(Entry {
                triangle: id,
                sample,
                error: 0.,
            })?;
        }
        if lock {
            // 最低頂点数のための追加でも、共有辺には独自の頂点を増やさない。
            self.lock_boundary = true;
            self.boundary_error = boundary_error;
            self.heap.clear();
            let faces: Vec<_> = self.triangles.iter().map(|(&id, t)| (id, t.face)).collect();
            for (id, face) in faces {
                let triangle = self.evaluate(face);
                self.heap.push(Entry {
                    error: triangle.error,
                    triangle: id,
                    sample: triangle.sample,
                });
                self.triangles.insert(id, triangle);
            }
        }
        Ok(())
    }

    fn progressive_stage(&mut self, step_count: usize) -> ProgressiveStage {
        let max_error = self.worst().error.max(self.boundary_error);
        let mut faces: Vec<_> = self
            .triangles
            .iter()
            .map(|(&id, t)| FaceRecord {
                id,
                vertices: t.face,
            })
            .collect();
        faces.sort_unstable_by_key(|face| face.id);
        ProgressiveStage {
            vertex_count: self.points.len(),
            step_count,
            max_error,
            faces,
        }
    }

    /// 共通境界を先に固定し、内部の挿入順と面更新を記録する比較用出力。
    pub fn generate_progressive(
        mut self,
        budgets: &[usize],
        boundary_error: f64,
    ) -> Result<ProgressiveTin> {
        ensure!(
            !budgets.is_empty() && budgets[0] >= 4 && budgets.windows(2).all(|w| w[0] < w[1]),
            "頂点上限は4以上の昇順で指定してください"
        );
        self.prepare_boundary(budgets[0], boundary_error, true)?;
        ensure!(
            self.points.len() <= budgets[0],
            "境界頂点が最初の段階の上限を超えます"
        );
        let base = self.progressive_stage(0);
        let precision = self.grid.iter().fold(1f64, |a, &b| a.max(b.abs())) * f64::EPSILON * 64.;
        let mut steps = Vec::new();
        let mut stages = Vec::new();
        for &budget in budgets {
            while self.points.len() < budget.min(self.grid.len()) {
                let top = self.worst();
                if top.error <= precision {
                    break;
                }
                steps.push(self.insert_recorded(top, true)?.expect("挿入履歴"));
            }
            stages.push(self.progressive_stage(steps.len()));
        }
        let vertices = self
            .points
            .iter()
            .map(|&p| [p[0] as f64, p[1] as f64, self.height_at(p)])
            .collect();
        Ok(ProgressiveTin {
            schema_version: 1,
            vertices,
            base,
            stages,
            steps,
        })
    }

    pub fn generate(self, max_vertices: usize, tolerance: f64) -> Result<Mesh> {
        self.generate_with_minimum(max_vertices, tolerance, 4)
    }
    pub fn generate_with_minimum(
        mut self,
        max_vertices: usize,
        tolerance: f64,
        min_vertices: usize,
    ) -> Result<Mesh> {
        ensure!(max_vertices >= 4, "最大頂点数は4以上が必要です");
        ensure!(
            min_vertices >= 4 && min_vertices <= max_vertices && min_vertices <= self.grid.len(),
            "最低頂点数は4以上で最大頂点数と画素数以下が必要です"
        );
        ensure!(
            tolerance.is_finite() && tolerance >= 0.,
            "許容誤差は0以上の有限値が必要です"
        );
        let limit = max_vertices.min(self.grid.len());
        let scale = self.grid.iter().fold(1f64, |a, &b| a.max(b.abs()));
        let precision = scale * f64::EPSILON * 64.;
        loop {
            let top = self.worst();
            if (top.error <= tolerance
                && (self.points.len() >= min_vertices || top.error <= precision))
                || top.error == 0.
                || self.points.len() >= limit
            {
                break;
            }
            self.insert(top)?;
        }
        let max_error = self.worst().error.max(self.boundary_error);
        let reached = max_error <= tolerance;
        let vertices = self
            .points
            .iter()
            .map(|&p| [p[0] as f64, p[1] as f64, self.height_at(p)])
            .collect();
        let mut faces: Vec<_> = self.triangles.into_iter().collect();
        faces.sort_unstable_by_key(|(id, _)| *id);
        Ok(Mesh {
            vertices,
            triangles: faces.into_iter().map(|(_, t)| t.face).collect(),
            max_error,
            tolerance_reached: reached,
            minimum_vertices_reached: self.points.len() >= min_vertices,
            stop_reason: if reached && self.points.len() < min_vertices {
                "exactSurface"
            } else if reached {
                "tolerance"
            } else {
                "maxVertices"
            },
            triangle_evaluations: self.evaluations,
            sample_evaluations: self.samples,
        })
    }
}
#[cfg(test)]
mod progressive_tests {
    use super::*;
    #[test]
    fn recorded_updates_reconstruct_every_checkpoint() {
        let grid: Vec<_> = (0..33 * 33)
            .map(|i| ((i % 33) as f64 * 0.37).sin() * ((i / 33) as f64 * 0.21).cos() * 300.)
            .collect();
        let result = Builder::new(&grid, 33, 33)
            .unwrap()
            .generate_progressive(&[128, 256, 512], 10.)
            .unwrap();
        let mut faces: HashMap<_, _> = result
            .base
            .faces
            .iter()
            .map(|f| (f.id, f.vertices))
            .collect();
        let mut step = 0;
        for stage in &result.stages {
            while step < stage.step_count {
                let update = &result.steps[step];
                for id in &update.removed {
                    assert!(faces.remove(id).is_some());
                }
                for f in &update.added {
                    assert!(faces.insert(f.id, f.vertices).is_none());
                    assert!(
                        f.vertices
                            .iter()
                            .all(|&v| v < result.base.vertex_count + step + 1)
                    );
                }
                let area: i128 = faces
                    .values()
                    .map(|f| {
                        let p =
                            f.map(|i| [result.vertices[i][0] as i64, result.vertices[i][1] as i64]);
                        let a = orient(p[0], p[1], p[2]);
                        assert!(a > 0);
                        a
                    })
                    .sum();
                assert_eq!(area, 2 * 32 * 32);
                step += 1;
            }
            assert_eq!(
                faces,
                stage.faces.iter().map(|f| (f.id, f.vertices)).collect()
            );
            assert_eq!(stage.vertex_count, result.base.vertex_count + step);
        }
        let other = Builder::new(&grid, 33, 33)
            .unwrap()
            .generate_progressive(&[128, 256, 512], 10.)
            .unwrap();
        assert_eq!(
            serde_json::to_string(&result).unwrap(),
            serde_json::to_string(&other).unwrap()
        );
    }
    #[test]
    fn progressive_flat_surface_stops_without_wasting_vertices() {
        let grid = vec![15.; 17 * 17];
        let result = Builder::new(&grid, 17, 17)
            .unwrap()
            .generate_progressive(&[64, 128, 256], 1.)
            .unwrap();
        assert_eq!(result.vertices.len(), 4);
        assert!(result.steps.is_empty());
        assert!(
            result
                .stages
                .iter()
                .all(|s| s.max_error == 0. && s.vertex_count == 4)
        );
        assert!(
            Builder::new(&grid, 17, 17)
                .unwrap()
                .generate_progressive(&[64, 32], 1.)
                .is_err()
        );
    }
}
