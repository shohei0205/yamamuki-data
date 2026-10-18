use crate::mesh::{Mesh, Point, barycentric, dot, normal};
use anyhow::{Result, bail};
use rstar::{AABB, RTree, RTreeObject};
use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeSet, BinaryHeap, HashMap},
};
#[derive(Clone)]
struct Probe(Point);
impl RTreeObject for Probe {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point([self.0[0], self.0[1]])
    }
}
#[derive(Clone, Debug)]
struct Entry {
    cost: f64,
    keep: usize,
    remove: usize,
    kr: u64,
    rr: u64,
}
impl PartialEq for Entry {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Entry {}
impl Ord for Entry {
    fn cmp(&self, o: &Self) -> Ordering {
        self.cost
            .total_cmp(&o.cost)
            .then(self.keep.cmp(&o.keep))
            .then(self.remove.cmp(&o.remove))
            .then(self.kr.cmp(&o.kr))
            .then(self.rr.cmp(&o.rr))
    }
}
impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
// 境界は固定し、入力の端点だけへ辺を縮める。面の折り返しと誤差超過を拒否する。
pub struct Qem {
    mesh: Mesh,
    quadrics: Vec<[f64; 16]>,
    adj: Vec<BTreeSet<usize>>,
    locked: Vec<bool>,
    alive: Vec<bool>,
    face_alive: Vec<bool>,
    revision: Vec<u64>,
    heap: BinaryHeap<Reverse<Entry>>,
    queued: HashMap<(usize, usize), (u64, u64)>,
    references: RTree<Probe>,
    remaining: usize,
    limit: f64,
}
impl Qem {
    pub fn new(mut mesh: Mesh, protected: &[usize], probes: Option<Vec<Point>>) -> Result<Self> {
        mesh.validate()?;
        for f in &mut mesh.faces {
            if normal(&mesh.vertices, *f)[2] < 0. {
                f.swap(1, 2)
            }
        }
        let nv = mesh.vertices.len();
        let nf = mesh.faces.len();
        let mut quadrics = vec![[0.; 16]; nv];
        let mut adj = vec![BTreeSet::new(); nv];
        let mut edges = HashMap::new();
        for (fi, f) in mesh.faces.iter().enumerate() {
            let n = normal(&mesh.vertices, *f);
            let length = dot(n, n).sqrt();
            let mut plane = [n[0] / length, n[1] / length, n[2] / length, 0.];
            plane[3] = -(0..3)
                .map(|i| plane[i] * mesh.vertices[f[0]][i])
                .sum::<f64>();
            for &v in f {
                adj[v].insert(fi);
                for i in 0..4 {
                    for j in 0..4 {
                        quadrics[v][i * 4 + j] += plane[i] * plane[j] * length / 2.
                    }
                }
            }
            for (u, v) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
                *edges.entry((u.min(v), u.max(v))).or_insert(0usize) += 1;
            }
        }
        let mut locked = vec![false; nv];
        for (&(u, v), &count) in &edges {
            if count > 2 {
                bail!("一つの辺に三つ以上の面があります")
            };
            if count == 1 {
                locked[u] = true;
                locked[v] = true
            }
        }
        for &i in protected {
            if i >= nv {
                bail!("固定する頂点番号が不正です")
            };
            locked[i] = true
        }
        let references = RTree::bulk_load(
            probes
                .unwrap_or_else(|| mesh.probes())
                .into_iter()
                .map(Probe)
                .collect(),
        );
        let mut q = Self {
            mesh,
            quadrics,
            adj,
            locked,
            alive: vec![true; nv],
            face_alive: vec![true; nf],
            revision: vec![0; nv],
            heap: BinaryHeap::new(),
            queued: HashMap::new(),
            references,
            remaining: nf,
            limit: f64::INFINITY,
        };
        let mut list: Vec<_> = edges.keys().copied().collect();
        list.sort_unstable();
        for (u, v) in list {
            q.enqueue(u, v)
        }
        Ok(q)
    }
    fn neighbors(&self, v: usize) -> BTreeSet<usize> {
        self.adj[v]
            .iter()
            .flat_map(|i| self.mesh.faces[*i])
            .filter(|w| *w != v)
            .collect()
    }
    fn enqueue(&mut self, u: usize, v: usize) {
        if u == v || !self.alive[u] || !self.alive[v] || self.locked[u] && self.locked[v] {
            return;
        }
        let edge = (u.min(v), u.max(v));
        let stamp = (self.revision[edge.0], self.revision[edge.1]);
        if self.queued.get(&edge) == Some(&stamp) {
            return;
        }
        self.queued.insert(edge, stamp);
        let mut choices = Vec::with_capacity(2);
        if !self.locked[v] {
            choices.push((u, v))
        }
        if !self.locked[u] {
            choices.push((v, u))
        }
        let mut best: Option<Entry> = None;
        for (keep, remove) in choices {
            let p = [
                self.mesh.vertices[keep][0],
                self.mesh.vertices[keep][1],
                self.mesh.vertices[keep][2],
                1.,
            ];
            let mut cost = 0.;
            for i in 0..4 {
                for j in 0..4 {
                    cost +=
                        p[i] * (self.quadrics[u][i * 4 + j] + self.quadrics[v][i * 4 + j]) * p[j]
                }
            }
            let e = Entry {
                cost: cost.max(0.),
                keep,
                remove,
                kr: self.revision[keep],
                rr: self.revision[remove],
            };
            if best.as_ref().is_none_or(|b| e < *b) {
                best = Some(e)
            }
        }
        if let Some(e) = best {
            self.heap.push(Reverse(e))
        }
    }
    fn valid(&self, keep: usize, remove: usize) -> bool {
        let shared: BTreeSet<_> = self.adj[keep]
            .intersection(&self.adj[remove])
            .copied()
            .collect();
        if shared.len() != 2 {
            return false;
        }
        let opposite: BTreeSet<_> = shared
            .iter()
            .flat_map(|i| self.mesh.faces[*i])
            .filter(|v| *v != keep && *v != remove)
            .collect();
        let a = self.neighbors(keep);
        let b = self.neighbors(remove);
        if a.intersection(&b).copied().collect::<BTreeSet<_>>() != opposite {
            return false;
        }
        let mut changed = Vec::new();
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for fi in self.adj[remove].difference(&shared) {
            let before = self.mesh.faces[*fi];
            let after = before.map(|v| if v == remove { keep } else { v });
            let old = normal(&self.mesh.vertices, before);
            let n = normal(&self.mesh.vertices, after);
            if n[2] <= 1e-9 || dot(old, n) <= 0.2 * (dot(old, old) * dot(n, n)).sqrt() {
                return false;
            }
            for v in after {
                for axis in 0..2 {
                    lo[axis] = lo[axis].min(self.mesh.vertices[v][axis]);
                    hi[axis] = hi[axis].max(self.mesh.vertices[v][axis]);
                }
            }
            changed.push(after);
        }
        if changed.is_empty() {
            return true;
        }
        // Python の KD-tree と同じ円内の標本を、空間索引で絞って検査する。
        let center = [(lo[0] + hi[0]) / 2., (lo[1] + hi[1]) / 2.];
        let radius = ((hi[0] - lo[0]).hypot(hi[1] - lo[1])) / 2. + 1e-7;
        let envelope = AABB::from_corners(
            [center[0] - radius, center[1] - radius],
            [center[0] + radius, center[1] + radius],
        );
        for probe in self.references.locate_in_envelope_intersecting(&envelope) {
            let p = probe.0;
            if (p[0] - center[0]).hypot(p[1] - center[1]) > radius {
                continue;
            }
            for f in &changed {
                let w = barycentric(&self.mesh.vertices, *f, [p[0], p[1]]);
                if w.iter().all(|v| *v >= -1e-8) {
                    let height = (0..3)
                        .map(|i| w[i] * self.mesh.vertices[f[i]][2])
                        .sum::<f64>();
                    if (height - p[2]).abs() > self.limit + 1e-6 {
                        return false;
                    }
                }
            }
        }
        true
    }
    pub fn reduce(&mut self, target: usize, limit: f64) -> Result<Mesh> {
        if target == 0 || !limit.is_finite() || limit < 0. {
            bail!("削減条件が不正です")
        }
        if self.limit.is_finite() && limit < self.limit {
            bail!("削減後に誤差制限を厳しくすることはできません")
        }
        if limit != self.limit {
            self.limit = limit;
            self.heap.clear();
            self.queued.clear();
            let mut edges = BTreeSet::new();
            for (i, f) in self.mesh.faces.iter().enumerate() {
                if self.face_alive[i] {
                    for (u, v) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
                        edges.insert((u.min(v), u.max(v)));
                    }
                }
            }
            for (u, v) in edges {
                self.enqueue(u, v)
            }
        }
        while self.remaining > target {
            let Some(Reverse(e)) = self.heap.pop() else {
                break;
            };
            let (keep, remove) = (e.keep, e.remove);
            if !self.alive[keep]
                || !self.alive[remove]
                || e.kr != self.revision[keep]
                || e.rr != self.revision[remove]
                || !self.valid(keep, remove)
            {
                continue;
            }
            let mut touched = self.neighbors(keep);
            touched.extend(self.neighbors(remove));
            touched.insert(keep);
            for fi in self.adj[remove].clone() {
                let before = self.mesh.faces[fi];
                if before.contains(&keep) {
                    self.face_alive[fi] = false;
                    self.remaining -= 1;
                    for v in before {
                        self.adj[v].remove(&fi);
                    }
                } else {
                    self.mesh.faces[fi] = before.map(|v| if v == remove { keep } else { v });
                    self.adj[keep].insert(fi);
                }
            }
            self.adj[remove].clear();
            self.alive[remove] = false;
            for i in 0..16 {
                self.quadrics[keep][i] += self.quadrics[remove][i]
            }
            for &v in &touched {
                self.revision[v] += 1
            }
            let mut edges = BTreeSet::new();
            for v in touched {
                if self.alive[v] {
                    for w in self.neighbors(v) {
                        edges.insert((v.min(w), v.max(w)));
                    }
                }
            }
            for (u, v) in edges {
                self.enqueue(u, v)
            }
        }
        Ok(self.snapshot())
    }
    pub fn snapshot(&self) -> Mesh {
        Mesh {
            vertices: self.mesh.vertices.clone(),
            faces: self
                .mesh
                .faces
                .iter()
                .enumerate()
                .filter_map(|(i, f)| self.face_alive[i].then_some(*f))
                .collect(),
        }
        .compact()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Surface;
    fn fixture() -> Mesh {
        let mut v = Vec::new();
        let mut f = Vec::new();
        for y in 0..17 {
            for x in 0..17 {
                let (x, y) = (x as f64, y as f64);
                v.push([
                    x,
                    y,
                    120. * (-((x - 7.).powi(2) + (y - 9.).powi(2)) / 15.).exp() + x * 2.,
                ]);
            }
        }
        for y in 0..16 {
            for x in 0..16 {
                let a = y * 17 + x;
                f.extend([[a, a + 1, a + 17], [a + 1, a + 18, a + 17]])
            }
        }
        Mesh {
            vertices: v,
            faces: f,
        }
    }
    #[test]
    fn guards_and_error() {
        let m = fixture();
        let probes = m.probes();
        let peak = m
            .vertices
            .iter()
            .map(|p| p[2])
            .fold(f64::NEG_INFINITY, f64::max);
        let pi = m.vertices.iter().position(|p| p[2] == peak).unwrap();
        let mut q = Qem::new(m.clone(), &[pi], None).unwrap();
        for limit in [2., 5., 12.] {
            let result = q.reduce(80, limit).unwrap();
            let surface = Surface::new(&result);
            for p in &probes {
                assert!((surface.height([p[0], p[1]]).unwrap() - p[2]).abs() <= limit + 1e-5)
            }
            assert!(result.vertices.iter().any(|p| p[2] == peak));
            for f in &result.faces {
                assert!(normal(&result.vertices, *f)[2] > 0.)
            }
        }
        assert!(q.reduce(80, 2.).is_err())
    }
    #[test]
    fn holes() {
        let mut m = fixture();
        m.faces.retain(|f| {
            let x = f.iter().map(|i| m.vertices[*i][0]).sum::<f64>() / 3.;
            let y = f.iter().map(|i| m.vertices[*i][1]).sum::<f64>() / 3.;
            !(x > 5. && x < 10. && y > 5. && y < 10.)
        });
        let mut q = Qem::new(m, &[], None).unwrap();
        let result = q.reduce(100, 10.).unwrap();
        assert!(Surface::new(&result).height([7.5, 7.5]).is_none())
    }
    #[test]
    fn planar() {
        let mut mesh = fixture();
        for p in &mut mesh.vertices {
            p[2] = p[0] * 2. + p[1] * 3. + 100.;
        }
        let mut q = Qem::new(mesh, &[], None).unwrap();
        let reduced = q.reduce(100, 0.).unwrap();
        assert!(reduced.faces.len() <= 100);
        for p in reduced.vertices {
            assert!((p[2] - p[0] * 2. - p[1] * 3. - 100.).abs() < 1e-9);
        }
    }
    #[test]
    fn invalid_mesh() {
        assert!(
            Qem::new(
                Mesh {
                    vertices: vec![[0., 0., 0.], [1., 0., 0.], [2., 0., 1.]],
                    faces: vec![[0, 1, 2]]
                },
                &[],
                None
            )
            .is_err()
        );
        let mut m = fixture();
        m.faces[0][0] = m.vertices.len();
        assert!(Qem::new(m, &[], None).is_err());
    }
}
