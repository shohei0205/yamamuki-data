//! 地域試作と全国生成で共有する、山頂制約付きTINの頂点削減。
use crate::adaptive_tin::quality_tin::minimum_angle;
use rayon::prelude::*;
use std::collections::{BinaryHeap, HashMap, HashSet};
pub(crate) type Face = [usize; 3];
pub(crate) type Point = [i64; 2];
pub(crate) fn orient(a: Point, b: Point, c: Point) -> i128 {
    (b[0] - a[0]) as i128 * (c[1] - a[1]) as i128 - (b[1] - a[1]) as i128 * (c[0] - a[0]) as i128
}
pub(crate) struct Surface {
    pub(crate) p: Vec<Point>,
    pub(crate) faces: Vec<Option<Face>>,
    pub(crate) adj: Vec<HashSet<usize>>,
    pub(crate) live: Vec<bool>,
    pub(crate) version: Vec<u64>,
    pub(crate) grid: Vec<f64>,
    pub(crate) weights: Vec<f64>,
    pub(crate) error_scales: Option<Vec<f64>>,
    pub(crate) n: usize,
    pub(crate) removed: Vec<usize>,
    pub(crate) density: Vec<f64>,
    pub(crate) density_step: f64,
    pub(crate) adaptive_boundary: bool,
    pub(crate) boundary_required: HashSet<usize>,
    pub(crate) maximum_face_span: Option<i64>,
    exact_density: Option<rstar::RTree<rstar::primitives::GeomWithData<[f64; 2], usize>>>,
    exact_radius: f64,
    candidate_cache_enabled: bool,
    candidate_cache: [std::sync::Mutex<HashMap<Face, ((f64, f64, f64), bool)>>; 16],
}
#[derive(Clone)]
struct Removal {
    vertex: usize,
    version: u64,
    faces: Vec<Face>,
    raw: f64,
    constrained: f64,
    weighted: f64,
}
#[derive(Clone)]
struct Entry {
    score: f64,
    removal: Removal,
}
impl PartialEq for Entry {
    fn eq(&self, o: &Self) -> bool {
        self.score == o.score && self.removal.vertex == o.removal.vertex
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Entry {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.score
            .total_cmp(&self.score)
            .then_with(|| o.removal.vertex.cmp(&self.removal.vertex))
    }
}
pub(crate) fn importance(grid: &[f64], n: usize, spacing: f64) -> Vec<f64> {
    let radius = (500. / spacing).ceil() as i64;
    let sigma = 125. / spacing;
    let mut kernel = (-radius..=radius)
        .map(|i| (-(i * i) as f64 / (2. * sigma * sigma)).exp())
        .collect::<Vec<_>>();
    let sum = kernel.iter().sum::<f64>();
    for v in &mut kernel {
        *v /= sum;
    }
    let mut a = vec![0.; grid.len()];
    let mut b = vec![0.; grid.len()];
    a.par_chunks_mut(n).enumerate().for_each(|(y, row)| {
        for (x, value) in row.iter_mut().enumerate() {
            *value = kernel
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    grid[y * n + (x as i64 + i as i64 - radius).clamp(0, n as i64 - 1) as usize] * w
                })
                .sum();
        }
    });
    b.par_chunks_mut(n).enumerate().for_each(|(y, row)| {
        for (x, value) in row.iter_mut().enumerate() {
            *value = kernel
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    a[(y as i64 + i as i64 - radius).clamp(0, n as i64 - 1) as usize * n + x] * w
                })
                .sum();
        }
    });
    drop(a);
    let d = (250. / spacing).round() as usize;
    let mut out = vec![0.; grid.len()];
    for y in 0..n {
        for x in 0..n {
            let l = x.saturating_sub(1);
            let r = (x + 1).min(n - 1);
            let t = y.saturating_sub(1);
            let u = (y + 1).min(n - 1);
            let slope = ((b[y * n + r] - b[y * n + l]) / ((r - l) as f64 * spacing))
                .hypot((b[u * n + x] - b[t * n + x]) / ((u - t) as f64 * spacing));
            let s = ((slope - 1f64.to_radians().tan())
                / (8f64.to_radians().tan() - 1f64.to_radians().tan()))
            .clamp(0., 1.);
            let convex = ((4. * b[y * n + x]
                - b[y * n + x.saturating_sub(d)]
                - b[y * n + (x + d).min(n - 1)]
                - b[y.saturating_sub(d) * n + x]
                - b[(y + d).min(n - 1) * n + x])
                / 50.)
                .clamp(0., 1.);
            out[y * n + x] = (0.02 + 0.98 * s * s * (3. - 2. * s)) * (1. + 0.5 * convex);
        }
    }
    out
}
impl Surface {
    pub(crate) fn new(
        p: Vec<Point>,
        faces: Vec<Face>,
        grid: Vec<f64>,
        weights: Vec<f64>,
        n: usize,
    ) -> Self {
        let count = p.len();
        let mut s = Self {
            p,
            faces: faces.into_iter().map(Some).collect(),
            adj: vec![HashSet::new(); count],
            live: vec![true; count],
            version: vec![0; count],
            grid,
            weights,
            error_scales: None,
            n,
            removed: vec![],
            density: vec![0.; 81],
            density_step: (n - 1) as f64 / 8.,
            adaptive_boundary: false,
            boundary_required: HashSet::new(),
            maximum_face_span: None,
            exact_density: None,
            exact_radius: 0.,
            candidate_cache_enabled: false,
            candidate_cache: std::array::from_fn(|_| std::sync::Mutex::new(HashMap::new())),
        };
        for (id, f) in s.faces.iter().enumerate() {
            for &v in f.as_ref().unwrap() {
                s.adj[v].insert(id);
            }
        }
        for v in 0..count {
            s.change_density(v, 1.);
        }
        s
    }
    pub(crate) fn use_exact_density(&mut self, radius: f64) {
        self.exact_radius = radius;
        self.exact_density = Some(rstar::RTree::bulk_load(
            (0..self.p.len())
                .filter(|&v| self.live[v])
                .map(|v| rstar::primitives::GeomWithData::new(self.p[v].map(|x| x as f64), v))
                .collect(),
        ));
    }
    pub(crate) fn change_density(&mut self, v: usize, amount: f64) {
        if let Some(tree) = self.exact_density.as_mut() {
            let point = rstar::primitives::GeomWithData::new(self.p[v].map(|x| x as f64), v);
            if amount < 0. {
                tree.remove(&point);
            } else {
                tree.insert(point);
            }
            return;
        }
        let p = self.p[v];
        let r = (self.n - 1) as f64 / 2.;
        for y in 0..9 {
            for x in 0..9 {
                if (p[0] as f64 - x as f64 * self.density_step)
                    .hypot(p[1] as f64 - y as f64 * self.density_step)
                    <= r
                {
                    self.density[y * 9 + x] += amount;
                }
            }
        }
    }
    pub(crate) fn penalty(&self, v: usize) -> f64 {
        if let Some(tree) = &self.exact_density {
            let count = tree
                .locate_within_distance(
                    self.p[v].map(|x| x as f64),
                    self.exact_radius * self.exact_radius,
                )
                .count() as f64;
            return 1. + (count - 4096.).max(0.) / 4096.;
        }
        let p = self.p[v];
        let x = p[0] as f64 / self.density_step;
        let y = p[1] as f64 / self.density_step;
        let ix = (x.floor() as usize).min(7);
        let iy = (y.floor() as usize).min(7);
        let u = x - ix as f64;
        let v = y - iy as f64;
        let count = self.density[iy * 9 + ix] * (1. - u) * (1. - v)
            + self.density[iy * 9 + ix + 1] * u * (1. - v)
            + self.density[(iy + 1) * 9 + ix] * (1. - u) * v
            + self.density[(iy + 1) * 9 + ix + 1] * u * v;
        1. + (count - 4096.).max(0.) / 4096.
    }
    pub(crate) fn metric(&self, f: Face) -> (f64, f64, f64) {
        self.metric_rows_limited(f, true, f64::INFINITY)
    }
    fn metric_rows(&self, f: Face, parallel: bool) -> (f64, f64, f64) {
        self.metric_rows_limited(f, parallel, f64::INFINITY)
    }
    fn metric_rows_limited(&self, f: Face, parallel: bool, limit: f64) -> (f64, f64, f64) {
        let [a, b, c] = f.map(|v| self.p[v]);
        let den = orient(a, b, c) as f64;
        let za = self.grid[a[1] as usize * self.n + a[0] as usize];
        let zb = self.grid[b[1] as usize * self.n + b[0] as usize] - za;
        let zc = self.grid[c[1] as usize * self.n + c[0] as usize] - za;
        let first = a[1].min(b[1]).min(c[1]);
        let end = a[1].max(b[1]).max(c[1]) + 1;
        let scan = |rows: std::ops::Range<i64>| {
            let (mut raw, mut weighted, mut constrained) = (0f64, 0f64, 0f64);
            for y in rows {
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
                for x in ((lo - 1e-8).ceil() as i64).max(0)
                    ..=((hi + 1e-8).floor() as i64).min(self.n as i64 - 1)
                {
                    let p = [x, y];
                    let u = orient(c, a, p);
                    let v = orient(a, b, p);
                    if u < 0 || v < 0 || (u + v) as f64 > den {
                        continue;
                    }
                    let index = y as usize * self.n + x as usize;
                    let e =
                        (za + u as f64 / den * zb + v as f64 / den * zc - self.grid[index]).abs();
                    raw = raw.max(e);
                    weighted = weighted.max(e * self.weights[index]);
                    constrained = constrained
                        .max(e / self.error_scales.as_ref().map(|v| v[index]).unwrap_or(1.));
                    // 不合格が確定した候補は残りの標本を走査しない。
                    if constrained > limit {
                        return (raw, weighted, constrained);
                    }
                }
            }
            (raw, weighted, constrained)
        };
        // 大きな面だけを行の塊に分ける。各標本の演算順序と最大値は変えない。
        let width = a[0].max(b[0]).max(c[0]) - a[0].min(b[0]).min(c[0]) + 1;
        if parallel && (end - first) * width >= 262144 && end - first >= 64 {
            let chunks = ((end - first + 31) / 32) as usize;
            (0..chunks)
                .into_par_iter()
                .map(|i| {
                    let start = first + i as i64 * 32;
                    scan(start..(start + 32).min(end))
                })
                .reduce(
                    || (0., 0., 0.),
                    |a, b| (a.0.max(b.0), a.1.max(b.1), a.2.max(b.2)),
                )
        } else {
            scan(first..end)
        }
    }
    pub(crate) fn is_boundary(&self, v: usize) -> bool {
        self.p[v].iter().any(|&a| a == 0 || a == self.n as i64 - 1)
    }
    // 各辺だけの1次元標高から保護点を決める。内部の頂点分布を判断に使わない。
    pub(crate) fn boundary_plan(&self, tolerance: f64) -> HashSet<usize> {
        let mut required = HashSet::new();
        for axis in 0..2 {
            for value in [0, self.n as i64 - 1] {
                let mut edge = (0..self.p.len())
                    .filter(|&v| self.p[v][axis] == value)
                    .collect::<Vec<_>>();
                edge.sort_by_key(|&v| self.p[v][1 - axis]);
                if edge.len() < 2 {
                    continue;
                }
                let mut segments = vec![(0, edge.len() - 1)];
                while let Some((lo, hi)) = segments.pop() {
                    required.insert(edge[lo]);
                    required.insert(edge[hi]);
                    if hi <= lo + 1 {
                        continue;
                    }
                    let a = self.p[edge[lo]];
                    let b = self.p[edge[hi]];
                    let za = self.grid[a[1] as usize * self.n + a[0] as usize];
                    let zb = self.grid[b[1] as usize * self.n + b[0] as usize];
                    let estimate = |t: i64| {
                        za + (zb - za) * (t - a[1 - axis]) as f64
                            / (b[1 - axis] - a[1 - axis]) as f64
                    };
                    let mut maximum = 0f64;
                    for t in a[1 - axis]..=b[1 - axis] {
                        let mut p = a;
                        p[1 - axis] = t;
                        maximum = maximum.max(
                            (self.grid[p[1] as usize * self.n + p[0] as usize] - estimate(t)).abs(),
                        );
                    }
                    if maximum <= tolerance * 0.5 + 1e-6 {
                        continue;
                    }
                    let split = (lo + 1..hi)
                        .max_by(|&u, &v| {
                            let error = |i: usize| {
                                let p = self.p[edge[i]];
                                (self.grid[p[1] as usize * self.n + p[0] as usize]
                                    - estimate(p[1 - axis]))
                                .abs()
                            };
                            error(u).total_cmp(&error(v)).then_with(|| v.cmp(&u))
                        })
                        .unwrap();
                    segments.push((lo, split));
                    segments.push((split, hi));
                }
            }
        }
        required
    }
    fn candidate_metric(&self, f: Face, limit: f64) -> (f64, f64, f64) {
        if !self.candidate_cache_enabled {
            return self.metric_rows_limited(f, true, limit);
        }
        // 頂点順もキーに含め、浮動小数点の演算順を変えない。
        let shard = (f[0].wrapping_mul(73856093)
            ^ f[1].wrapping_mul(19349663)
            ^ f[2].wrapping_mul(83492791))
            & 15;
        {
            let cache = self.candidate_cache[shard].lock().unwrap();
            if let Some(&(value, complete)) = cache.get(&f) {
                if complete || value.2 > limit {
                    return value;
                }
            }
        }
        let value = self.metric_rows_limited(f, true, limit);
        let mut cache = self.candidate_cache[shard].lock().unwrap();
        // 全体で最大262144面。上限到達時はその区画を空にし、最近の面を保存する。
        if cache.len() >= 16384 && !cache.contains_key(&f) {
            cache.clear();
        }
        cache.insert(f, (value, value.2 <= limit));
        value
    }
    pub(crate) fn candidate(&self, v: usize) -> Option<Removal> {
        self.candidate_limited(v, f64::INFINITY)
    }
    fn candidate_limited(&self, v: usize, limit: f64) -> Option<Removal> {
        if !self.live[v] {
            return None;
        }
        let boundary = self.is_boundary(v);
        if boundary
            && (!self.adaptive_boundary
                || self.boundary_required.contains(&v)
                || self.p[v].iter().all(|&a| a == 0 || a == self.n as i64 - 1))
        {
            return None;
        }
        let mut edges = HashMap::new();
        for &id in &self.adj[v] {
            let f = self.faces[id]?;
            let k = f.iter().position(|&a| a == v)?;
            if edges.insert(f[(k + 1) % 3], f[(k + 2) % 3]).is_some() {
                return None;
            }
        }
        let incoming = edges.values().copied().collect::<HashSet<_>>();
        let start = if boundary {
            *edges.keys().filter(|v| !incoming.contains(v)).min()?
        } else {
            *edges.keys().min()?
        };
        let mut ring = vec![start];
        let mut at = start;
        while let Some(&next) = edges.get(&at) {
            if next == start {
                break;
            }
            if ring.len() > edges.len() {
                return None;
            }
            ring.push(next);
            at = next;
        }
        if ring.len() != edges.len() + usize::from(boundary) || ring.len() < 3 {
            return None;
        }
        let xy = ring
            .iter()
            .flat_map(|&id| self.p[id].map(|v| v as f64))
            .collect::<Vec<_>>();
        let ids = earcutr::earcut(&xy, &[], 2).ok()?;
        let mut faces = ids
            .chunks_exact(3)
            .map(|f| {
                let mut t = [ring[f[0]], ring[f[1]], ring[f[2]]];
                if orient(self.p[t[0]], self.p[t[1]], self.p[t[2]]) < 0 {
                    t.swap(1, 2);
                }
                t
            })
            .collect::<Vec<_>>();
        // 面を変更した場合だけ角度を再計算する。
        let mut angles = faces
            .iter()
            .map(|&f| minimum_angle(&self.p, f))
            .collect::<Vec<_>>();
        // 穴内部の対角線を、最小角が改善する方向に交換する。
        for _ in 0..ring.len() * ring.len() {
            let mut changed = false;
            'outer: for i in 0..faces.len() {
                for j in i + 1..faces.len() {
                    let mut common = [0usize; 3];
                    let mut common_count = 0;
                    for &v in &faces[i] {
                        if faces[j].contains(&v) {
                            common[common_count] = v;
                            common_count += 1;
                        }
                    }
                    if common_count != 2 {
                        continue;
                    }
                    let common = &common[..2];
                    let a = *faces[i].iter().find(|v| !common.contains(v)).unwrap();
                    let b = *faces[j].iter().find(|v| !common.contains(v)).unwrap();
                    let mut f = [a, b, common[0]];
                    let mut g = [b, a, common[1]];
                    if orient(self.p[f[0]], self.p[f[1]], self.p[f[2]]) < 0 {
                        f.swap(1, 2);
                    }
                    if orient(self.p[g[0]], self.p[g[1]], self.p[g[2]]) < 0 {
                        g.swap(1, 2);
                    }
                    let old_area = orient(
                        self.p[faces[i][0]],
                        self.p[faces[i][1]],
                        self.p[faces[i][2]],
                    ) + orient(
                        self.p[faces[j][0]],
                        self.p[faces[j][1]],
                        self.p[faces[j][2]],
                    );
                    let new_area = orient(self.p[f[0]], self.p[f[1]], self.p[f[2]])
                        + orient(self.p[g[0]], self.p[g[1]], self.p[g[2]]);
                    if new_area != old_area || new_area <= 0 {
                        continue;
                    }
                    let before = angles[i].min(angles[j]);
                    let fa = minimum_angle(&self.p, f);
                    let ga = minimum_angle(&self.p, g);
                    let after = fa.min(ga);
                    if after > before + 1e-7 {
                        faces[i] = f;
                        faces[j] = g;
                        angles[i] = fa;
                        angles[j] = ga;
                        changed = true;
                        break 'outer;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let old_area: i128 = self.adj[v]
            .iter()
            .map(|&id| {
                let f = self.faces[id].unwrap();
                orient(self.p[f[0]], self.p[f[1]], self.p[f[2]])
            })
            .sum();
        let area: i128 = faces
            .iter()
            .map(|f| orient(self.p[f[0]], self.p[f[1]], self.p[f[2]]))
            .sum();
        if old_area != area
            || faces.iter().any(|&f| {
                let too_wide = self.maximum_face_span.is_some_and(|limit| {
                    (0..2).any(|axis| {
                        let lo = f.iter().map(|&v| self.p[v][axis]).min().unwrap();
                        let hi = f.iter().map(|&v| self.p[v][axis]).max().unwrap();
                        hi - lo > limit
                    })
                });
                too_wide
                    || orient(self.p[f[0]], self.p[f[1]], self.p[f[2]]) <= 0
                    || minimum_angle(&self.p, f) < 10. - 1e-7
            })
        {
            return None;
        }
        let (mut raw, mut weighted, mut constrained) = (0f64, 0f64, 0f64);
        for &f in &faces {
            let (a, b, c) = self.candidate_metric(f, limit);
            if c > limit {
                return None;
            }
            raw = raw.max(a);
            weighted = weighted.max(b);
            constrained = constrained.max(c);
        }
        Some(Removal {
            vertex: v,
            version: self.version[v],
            faces,
            raw,
            constrained,
            weighted,
        })
    }
    pub(crate) fn apply(&mut self, r: &Removal) -> Vec<usize> {
        let v = r.vertex;
        let mut changed = HashSet::new();
        let ids = self.adj[v].iter().copied().collect::<Vec<_>>();
        for id in ids {
            let f = self.faces[id].take().unwrap();
            for a in f {
                self.adj[a].remove(&id);
                changed.insert(a);
            }
        }
        self.live[v] = false;
        self.change_density(v, -1.);
        self.removed.push(v);
        for &f in &r.faces {
            let id = self.faces.len();
            self.faces.push(Some(f));
            for a in f {
                self.adj[a].insert(id);
                changed.insert(a);
            }
        }
        let mut changed = changed
            .into_iter()
            .filter(|&a| self.live[a])
            .collect::<Vec<_>>();
        changed.sort_unstable();
        for &a in &changed {
            self.version[a] += 1;
        }
        changed
    }
    pub(crate) fn reduce(&mut self, tolerance: f64, target: usize) -> (usize, bool) {
        self.reduce_with_candidate_parallelism(tolerance, target, true)
    }
    fn reduce_with_candidate_parallelism(
        &mut self,
        tolerance: f64,
        target: usize,
        parallel: bool,
    ) -> (usize, bool) {
        self.reduce_progress_internal(tolerance, target, parallel, |_, _, _, _| {})
    }
    pub(crate) fn reduce_with_progress(
        &mut self,
        tolerance: f64,
        target: usize,
        progress: impl FnMut(&str, usize, usize, usize),
    ) -> (usize, bool) {
        self.reduce_progress_internal(tolerance, target, true, progress)
    }
    fn reduce_progress_internal(
        &mut self,
        tolerance: f64,
        target: usize,
        parallel: bool,
        mut progress: impl FnMut(&str, usize, usize, usize),
    ) -> (usize, bool) {
        // 地形の局所誤差係数がレベル間で変わるため、前レベルの値は使わない。
        self.candidate_cache_enabled = parallel;
        for cache in &mut self.candidate_cache {
            cache.get_mut().unwrap().clear();
        }
        let initial = self.live.iter().filter(|&&v| v).count();
        progress("boundary-plan", 0, initial, 0);
        if self.adaptive_boundary {
            self.boundary_required = self.boundary_plan(tolerance);
        }
        let mut heap = BinaryHeap::new();
        progress("prepare-candidates", 0, self.p.len(), 0);
        // IndexedParallelIterator の収集順は頂点番号順。ヒープへの投入順を保つ。
        // 全候補を同時に保持せず、作業用メモリーを4096頂点分に制限する。
        for start in (0..self.p.len()).step_by(4096) {
            let end = (start + 4096).min(self.p.len());
            let candidates = if parallel {
                (start..end)
                    .into_par_iter()
                    .map(|v| self.candidate_limited(v, tolerance + 1e-6))
                    .collect::<Vec<_>>()
            } else {
                (start..end).map(|v| self.candidate(v)).collect::<Vec<_>>()
            };
            for (offset, candidate) in candidates.into_iter().enumerate() {
                if let Some(r) = candidate {
                    if r.constrained <= tolerance + 1e-6 {
                        heap.push(Entry {
                            score: r.weighted / self.penalty(start + offset),
                            removal: r,
                        });
                    }
                }
            }
            progress("prepare-candidates", end, self.p.len(), heap.len());
        }
        let mut count = initial;
        let mut steps = 0;
        let mut last_progress = std::time::Instant::now();
        progress("remove-vertices", 0, initial, heap.len());
        while count > target {
            let Some(mut e) = heap.pop() else { break };
            let v = e.removal.vertex;
            if !self.live[v] || self.version[v] != e.removal.version {
                continue;
            }
            let score = e.removal.weighted / self.penalty(v);
            if score > e.score + 1e-10 {
                e.score = score;
                heap.push(e);
                continue;
            }
            if last_progress.elapsed().as_secs_f64() >= 1. {
                progress("remove-vertices", steps, initial, heap.len());
                last_progress = std::time::Instant::now();
            }
            let changed = self.apply(&e.removal);
            count -= 1;
            steps += 1;
            for a in changed {
                let candidate = if parallel {
                    self.candidate_limited(a, tolerance + 1e-6)
                } else {
                    self.candidate(a)
                };
                if let Some(r) = candidate {
                    if r.constrained <= tolerance + 1e-6 {
                        heap.push(Entry {
                            score: r.weighted / self.penalty(a),
                            removal: r,
                        });
                    }
                }
            }
        }
        progress("reduction-finished", steps, initial, heap.len());
        self.candidate_cache_enabled = false;
        for cache in &mut self.candidate_cache {
            cache.get_mut().unwrap().clear();
        }
        (steps, count <= target)
    }
    pub(crate) fn audit(&self) -> (f64, f64, f64) {
        let (mut max, mut weighted, mut angle) = (0f64, 0f64, 180f64);
        for &f in self.faces.iter().flatten() {
            let (a, b, _) = self.metric(f);
            max = max.max(a);
            weighted = weighted.max(b);
            angle = angle.min(minimum_angle(&self.p, f));
        }
        (max, weighted, angle)
    }
    pub(crate) fn compact(&self) -> (Vec<Point>, Vec<Face>, Vec<usize>) {
        let ids = (0..self.p.len())
            .filter(|&v| self.live[v])
            .collect::<Vec<_>>();
        let mut map = vec![0; self.p.len()];
        for (i, &v) in ids.iter().enumerate() {
            map[v] = i;
        }
        let faces = self
            .faces
            .iter()
            .flatten()
            .map(|f| f.map(|v| map[v]))
            .collect();
        (ids.iter().map(|&v| self.p[v]).collect(), faces, ids)
    }
}

#[cfg(test)]
mod regional_controls_tests {
    use super::*;
    #[test]
    fn exact_density_counts_points_and_applies_removals() {
        let p = (0..5000)
            .map(|i| [(i % 100) as i64, (i / 100) as i64])
            .collect::<Vec<_>>();
        let mut s = Surface::new(p, vec![], vec![0.; 10000], vec![1.; 10000], 100);
        s.use_exact_density(200.);
        assert!((s.penalty(0) - (1. + 904. / 4096.)).abs() < 1e-12);
        s.change_density(10, -1.);
        assert!((s.penalty(0) - (1. + 903. / 4096.)).abs() < 1e-12);
    }
    #[test]
    fn span_limit_preserves_geometry_needed_for_small_tile_halos() {
        let n = 33;
        let grid = vec![0.; n * n];
        let m = crate::adaptive_tin::quality_tin::generate_with_spatial_seeds(
            &grid,
            n,
            1.,
            10.,
            Some(8),
        )
        .unwrap();
        let mut s = Surface::new(m.points, m.faces, grid, vec![1.; n * n], n);
        s.maximum_face_span = Some(8);
        s.reduce(100., 4);
        assert!(s.live.iter().filter(|&&v| v).count() > 4);
        for f in s.faces.iter().flatten() {
            for axis in 0..2 {
                assert!(
                    f.iter().map(|&v| s.p[v][axis]).max().unwrap()
                        - f.iter().map(|&v| s.p[v][axis]).min().unwrap()
                        <= 8
                );
            }
        }
    }
}

#[cfg(test)]
mod parallel_metric_tests {
    use super::*;
    #[test]
    fn parallel_raster_metric_is_bit_identical() {
        let n = 1025;
        let grid = (0..n * n)
            .map(|i| ((i % n) as f64 * 0.013).sin() * 110. + ((i / n) as f64 * 0.021).cos() * 57.)
            .collect::<Vec<_>>();
        let weights = (0..n * n).map(|i| 0.02 + (i % 137) as f64 / 100.).collect();
        let mut s = Surface::new(
            vec![[0, 0], [1024, 0], [1024, 1024], [0, 1024], [517, 503]],
            vec![],
            grid,
            weights,
            n,
        );
        for scales in [false, true] {
            if scales {
                s.error_scales = Some((0..n * n).map(|i| 0.5 + (i % 53) as f64 / 106.).collect());
            }
            for f in [
                [0, 1, 2],
                [0, 2, 3],
                [0, 1, 4],
                [1, 2, 4],
                [2, 3, 4],
                [3, 0, 4],
            ] {
                assert_eq!(s.metric_rows(f, false), s.metric_rows(f, true));
            }
        }
    }
}

#[cfg(test)]
mod parallel_reduction_tests {
    use super::*;
    #[test]
    fn parallel_candidates_keep_removal_order_and_mesh() {
        let n = 65;
        let grid = (0..n * n)
            .map(|i| ((i % n) as f64 * 0.13).sin() * 20. + ((i / n) as f64 * 0.17).cos() * 15.)
            .collect::<Vec<_>>();
        let mesh = crate::adaptive_tin::quality_tin::generate_with_spatial_seeds(
            &grid,
            n,
            1.,
            10.,
            Some(8),
        )
        .unwrap();
        let make = || {
            let mut s = Surface::new(
                mesh.points.clone(),
                mesh.faces.clone(),
                grid.clone(),
                vec![1.; n * n],
                n,
            );
            s.adaptive_boundary = true;
            s.use_exact_density(20.);
            s
        };
        let mut serial = make();
        let mut parallel = make();
        for error in [2., 5., 10., 20.] {
            assert_eq!(
                serial.reduce_with_candidate_parallelism(error, 4, false),
                parallel.reduce_with_candidate_parallelism(error, 4, true)
            );
            assert_eq!(serial.removed, parallel.removed);
            assert_eq!(serial.compact(), parallel.compact());
            assert_eq!(serial.audit(), parallel.audit());
        }
    }
    #[test]
    #[ignore = "リリース構成で誤差評価の速度を手動計測する"]
    fn benchmark_large_face_metric() {
        let n = 4097;
        let grid = (0..n * n)
            .map(|i| ((i % n) as f64 * 0.013).sin() * 110. + ((i / n) as f64 * 0.021).cos() * 57.)
            .collect::<Vec<_>>();
        let s = Surface::new(
            vec![[0, 0], [4096, 0], [4096, 4096], [0, 4096]],
            vec![],
            grid,
            vec![1.; n * n],
            n,
        );
        let threads = std::thread::available_parallelism()
            .unwrap()
            .get()
            .div_ceil(2);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let mut reference = vec![];
            for parallel in [false, true] {
                let start = std::time::Instant::now();
                let mut values = vec![];
                for _ in 0..4 {
                    for f in [[0, 1, 2], [0, 2, 3]] {
                        values.push(std::hint::black_box(s.metric_rows(f, parallel)));
                    }
                }
                if parallel {
                    assert_eq!(reference, values);
                } else {
                    reference = values;
                }
                eprintln!(
                    "metric parallel={parallel} threads={threads} seconds={:.6}",
                    start.elapsed().as_secs_f64()
                );
            }
        });
    }
}

#[cfg(test)]
mod bounded_candidate_tests {
    use super::*;
    #[test]
    fn rejected_metric_can_stop_but_accepted_values_are_exact() {
        let n = 129;
        let grid = (0..n * n)
            .map(|i| ((i % n) as f64 * 0.13).sin() * 100. + ((i / n) as f64 * 0.17).cos() * 70.)
            .collect::<Vec<_>>();
        let s = Surface::new(
            vec![[0, 0], [128, 0], [128, 128], [0, 128]],
            vec![],
            grid,
            vec![1.; n * n],
            n,
        );
        let f = [0, 1, 2];
        let full = s.metric(f);
        let rejected = s.metric_rows_limited(f, false, 1.);
        assert!(rejected.2 > 1. && rejected.2 <= full.2);
        assert_eq!(s.metric_rows_limited(f, true, full.2), full);
    }
    #[test]
    #[ignore = "リリース構成で削減候補の評価時間を手動計測する"]
    fn benchmark_rejected_candidates() {
        let n = 1025;
        let grid = (0..n * n)
            .map(|i| ((i % n) as f64 * 0.013).sin() * 100. + ((i / n) as f64 * 0.017).cos() * 70.)
            .collect::<Vec<_>>();
        let points = (0..=8)
            .flat_map(|y| (0..=8).map(move |x| [x * 128, y * 128]))
            .collect::<Vec<_>>();
        let mut faces = vec![];
        for y in 0..8 {
            for x in 0..8 {
                let a = y * 9 + x;
                faces.push([a, a + 1, a + 10]);
                faces.push([a, a + 10, a + 9]);
            }
        }
        let s = Surface::new(points, faces, grid, vec![1.; n * n], n);
        let mut expected = vec![];
        for bounded in [false, true] {
            let start = std::time::Instant::now();
            let mut values = vec![];
            for v in 0..s.p.len() {
                let r = if bounded {
                    s.candidate_limited(v, 1.)
                } else {
                    s.candidate(v).filter(|r| r.constrained <= 1.)
                };
                values.push(r.map(|r| (r.vertex, r.faces, r.raw, r.weighted, r.constrained)));
            }
            if bounded {
                assert_eq!(expected, values)
            } else {
                expected = values;
            }
            eprintln!(
                "candidate bounded={bounded} seconds={:.6}",
                start.elapsed().as_secs_f64()
            );
        }
    }
}

#[cfg(test)]
mod candidate_cache_tests {
    use super::*;
    #[test]
    fn partial_failure_cache_never_becomes_an_accepted_inexact_metric() {
        let n = 65;
        let grid = (0..n * n)
            .map(|i| ((i % n) as f64 * 0.13).sin() * 100. + ((i / n) as f64 * 0.17).cos() * 70.)
            .collect::<Vec<_>>();
        let mut s = Surface::new(
            vec![[0, 0], [64, 0], [64, 64], [0, 64]],
            vec![],
            grid,
            vec![1.; n * n],
            n,
        );
        let f = [0, 1, 2];
        let full = s.metric(f);
        s.candidate_cache_enabled = true;
        assert!(s.candidate_metric(f, 1.).2 > 1.);
        assert_eq!(s.candidate_metric(f, f64::INFINITY), full);
        assert_eq!(s.candidate_metric(f, 1.), full);
    }
}

#[cfg(test)]
mod cache_benchmark_tests {
    use super::*;
    #[test]
    #[ignore = "リリース構成で重複面の評価を手動計測する"]
    fn benchmark_repeated_face_metrics() {
        let n = 1025;
        let grid = (0..n * n)
            .map(|i| ((i % n) as f64 * 0.013).sin() * 100. + ((i / n) as f64 * 0.017).cos() * 70.)
            .collect::<Vec<_>>();
        let mut s = Surface::new(
            vec![[0, 0], [1024, 0], [1024, 1024], [0, 1024]],
            vec![],
            grid,
            vec![1.; n * n],
            n,
        );
        let mut expected = vec![];
        for enabled in [false, true] {
            s.candidate_cache_enabled = enabled;
            let start = std::time::Instant::now();
            let mut values = vec![];
            for _ in 0..8 {
                for f in [[0, 1, 2], [0, 2, 3]] {
                    values.push(std::hint::black_box(s.candidate_metric(f, f64::INFINITY)));
                }
            }
            if enabled {
                assert_eq!(expected, values)
            } else {
                expected = values;
            }
            eprintln!(
                "face-cache enabled={enabled} seconds={:.6}",
                start.elapsed().as_secs_f64()
            );
        }
    }
}
