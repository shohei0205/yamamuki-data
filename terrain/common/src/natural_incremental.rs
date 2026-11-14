//! Delaunay頂点の局所削除。変更した面と隣接だけを保存し、試行を戻せる。
use super::Fast;
use crate::natural_surface::{CircleEntry, Surface};
use anyhow::{Result, bail, ensure};
use delaunator::{EMPTY, Triangulation};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

pub struct RemovalUndo {
    pub start_face: usize,
    length: usize,
    faces: Vec<(usize, [usize; 3], [f64; 3])>,
    edges: Vec<(usize, usize)>,
    incident: Vec<(usize, Vec<usize>)>,
    neighbors: Vec<(usize, Vec<usize>)>,
    cells: Vec<(usize, OnceLock<Vec<[f64; 2]>>)>,
    index: Option<rstar::RTree<CircleEntry>>,
}
fn orient(p: &[[f64; 3]], a: usize, b: usize, c: usize) -> f64 {
    let q = |i: usize| robust::Coord {
        x: p[i][0],
        y: p[i][1],
    };
    robust::orient2d(q(a), q(b), q(c))
}
fn circle(p: &[[f64; 3]], t: [usize; 3]) -> Result<[f64; 3]> {
    let [p, a, b] = [p[t[0]], p[t[1]], p[t[2]]];
    let a = [a[0] - p[0], a[1] - p[1]];
    let b = [b[0] - p[0], b[1] - p[1]];
    let det = 2. * (a[0] * b[1] - a[1] * b[0]);
    ensure!(
        det != 0. && det.is_finite(),
        "局所削除で面積ゼロの面が発生しました"
    );
    let u = a[0] * a[0] + a[1] * a[1];
    let v = b[0] * b[0] + b[1] * b[1];
    let dx = (u * b[1] - v * a[1]) / det;
    let dy = (a[0] * v - b[0] * u) / det;
    Ok([p[0] + dx, p[1] + dy, dx * dx + dy * dy])
}
fn cavity(base: &Surface, id: usize) -> Result<Vec<[usize; 3]>> {
    let q = base.points[id];
    let mut ring = base.neighbors[id].clone();
    ring.sort_by(|&a, &b| {
        (base.points[a][1] - q[1])
            .atan2(base.points[a][0] - q[0])
            .total_cmp(&(base.points[b][1] - q[1]).atan2(base.points[b][0] - q[0]))
    });
    let mut triangles = Vec::new();
    while ring.len() > 3 {
        let mut found = None;
        for k in 0..ring.len() {
            let [a, b, c] = [
                ring[(k + ring.len() - 1) % ring.len()],
                ring[k],
                ring[(k + 1) % ring.len()],
            ];
            if orient(&base.points, a, b, c) <= 0. {
                continue;
            }
            if ring.iter().copied().any(|v| {
                v != a
                    && v != b
                    && v != c
                    && orient(&base.points, a, b, v) >= 0.
                    && orient(&base.points, b, c, v) >= 0.
                    && orient(&base.points, c, a, v) >= 0.
            }) {
                continue;
            }
            found = Some((k, [a, b, c]));
            break;
        }
        let Some((k, t)) = found else {
            bail!("削除空洞を安定に三角形分割できません")
        };
        triangles.push(t);
        ring.remove(k);
    }
    ensure!(
        ring.len() == 3 && orient(&base.points, ring[0], ring[1], ring[2]) > 0.,
        "削除空洞が縮退しました"
    );
    triangles.push([ring[0], ring[1], ring[2]]);
    // 既存の面と同じ向きに統一する。
    let t = &base.topology.triangles[3 * base.incident[id][0]..3 * base.incident[id][0] + 3];
    if orient(&base.points, t[0], t[1], t[2]) < 0. {
        for t in &mut triangles {
            t.swap(1, 2);
        }
    }
    let mut topology = Triangulation {
        triangles: triangles.into_iter().flatten().collect(),
        halfedges: Vec::new(),
        hull: Vec::new(),
    };
    topology.halfedges.resize(topology.triangles.len(), EMPTY);
    let mut edges = BTreeMap::new();
    for e in 0..topology.triangles.len() {
        let a = topology.triangles[e];
        let b = topology.triangles[e / 3 * 3 + (e + 1) % 3];
        if let Some(&h) = edges.get(&(b, a)) {
            topology.halfedges[e] = h;
            topology.halfedges[h] = e;
        } else {
            edges.insert((a, b), e);
        }
    }
    crate::natural_surface::certified_legalize(&mut topology, &base.points)?;
    Ok(topology
        .triangles
        .chunks_exact(3)
        .map(|t| [t[0], t[1], t[2]])
        .collect())
}
fn remove_face(base: &mut Surface, f: usize) {
    let last = base.circles.len() - 1;
    for e in f * 3..f * 3 + 3 {
        let h = base.topology.halfedges[e];
        if h != EMPTY {
            base.topology.halfedges[h] = EMPTY;
        }
        let v = base.topology.triangles[e];
        base.incident[v].retain(|&t| t != f);
    }
    if f != last {
        for k in 0..3 {
            let v = base.topology.triangles[last * 3 + k];
            let h = base.topology.halfedges[last * 3 + k];
            base.topology.triangles[f * 3 + k] = v;
            base.topology.halfedges[f * 3 + k] = h;
            if h != EMPTY {
                base.topology.halfedges[h] = f * 3 + k;
            }
            for t in &mut base.incident[v] {
                if *t == last {
                    *t = f;
                }
            }
        }
        base.circles[f] = base.circles[last];
    }
    base.topology.triangles.truncate(last * 3);
    base.topology.halfedges.truncate(last * 3);
    base.circles.pop();
}
impl Fast {
    /// 挿入点を含む空円群だけを取り除き、境界へ扇形の面を張る。
    pub fn insert_local(&mut self, q: [f64; 3]) -> Result<()> {
        self.insert_local_from(q, 0).map(|_| ())
    }
    pub fn insert_local_from(&mut self, q: [f64; 3], hint: usize) -> Result<usize> {
        ensure!(q.iter().all(|v| v.is_finite()), "挿入点が非有限です");
        let (face, _) = self.base.locate(
            [q[0], q[1]],
            hint.min(self.base.circles.len().saturating_sub(1)),
        )?;
        let mut stack = vec![face];
        let mut seen = BTreeSet::new();
        let mut cavity = BTreeSet::new();
        let at = robust::Coord { x: q[0], y: q[1] };
        while let Some(f) = stack.pop() {
            if !seen.insert(f) {
                continue;
            }
            let t = &self.base.topology.triangles[f * 3..f * 3 + 3];
            let p = |i: usize| robust::Coord {
                x: self.base.points[i][0],
                y: self.base.points[i][1],
            };
            let o = robust::orient2d(p(t[0]), p(t[1]), p(t[2]));
            let d = robust::incircle(p(t[0]), p(t[1]), p(t[2]), at);
            if d != 0. && d.is_sign_positive() != o.is_sign_positive() {
                continue;
            }
            cavity.insert(f);
            for e in f * 3..f * 3 + 3 {
                let h = self.base.topology.halfedges[e];
                if h != EMPTY {
                    stack.push(h / 3);
                }
            }
        }
        ensure!(!cavity.is_empty(), "挿入点の空洞が見つかりません");
        let mut boundary = Vec::new();
        let mut affected = BTreeSet::new();
        for &f in &cavity {
            for e in f * 3..f * 3 + 3 {
                let a = self.base.topology.triangles[e];
                let b = self.base.topology.triangles[f * 3 + (e + 1) % 3];
                affected.insert(a);
                let h = self.base.topology.halfedges[e];
                if h == EMPTY || !cavity.contains(&(h / 3)) {
                    boundary.push((a, b));
                }
            }
        }
        ensure!(boundary.len() >= 3, "挿入空洞が縮退しました");
        let id = self.base.points.len();
        self.base.points.push(q);
        let new = boundary
            .iter()
            .map(|&(a, b)| [a, b, id])
            .collect::<Vec<_>>();
        let circles = new
            .iter()
            .map(|&t| circle(&self.base.points, t))
            .collect::<Result<Vec<_>>>()?;
        for &f in cavity.iter().rev() {
            remove_face(&mut self.base, f);
        }
        self.base.incident.push(Vec::new());
        self.base.neighbors.push(Vec::new());
        self.cells.push(OnceLock::new());
        let first = self.base.circles.len();
        for (k, t) in new.iter().enumerate() {
            let f = first + k;
            self.base.topology.triangles.extend(t);
            self.base.topology.halfedges.extend([EMPTY; 3]);
            self.base.circles.push(circles[k]);
            for &v in t {
                self.base.incident[v].push(f);
            }
        }
        for f in first..self.base.circles.len() {
            for k in 0..3 {
                let e = f * 3 + k;
                let a = self.base.topology.triangles[e];
                let b = self.base.topology.triangles[f * 3 + (k + 1) % 3];
                let h = self.base.incident[b]
                    .iter()
                    .copied()
                    .flat_map(|t| t * 3..t * 3 + 3)
                    .find(|&h| {
                        self.base.topology.triangles[h] == b
                            && self.base.topology.triangles[h / 3 * 3 + (h + 1) % 3] == a
                    });
                if let Some(h) = h {
                    self.base.topology.halfedges[e] = h;
                    self.base.topology.halfedges[h] = e;
                }
            }
        }
        affected.insert(id);
        for v in affected {
            let mut neighbors = self.base.incident[v]
                .iter()
                .flat_map(|&f| {
                    self.base.topology.triangles[f * 3..f * 3 + 3]
                        .iter()
                        .copied()
                })
                .filter(|&u| u != v)
                .collect::<Vec<_>>();
            neighbors.sort_unstable();
            neighbors.dedup();
            self.base.neighbors[v] = neighbors;
            self.cells[v] = OnceLock::new();
        }
        self.base.circle_index = OnceLock::new();
        Ok(first)
    }
    pub fn remove_local(&mut self, id: usize) -> Result<RemovalUndo> {
        ensure!(
            id < self.base.points.len()
                && !self.base.incident[id].is_empty()
                && !self.base.topology.hull.contains(&id),
            "局所削除には内点が必要です"
        );
        let new = cavity(&self.base, id)?;
        let circles = new
            .iter()
            .map(|&t| circle(&self.base.points, t))
            .collect::<Result<Vec<_>>>()?;
        let mut stars = self.base.incident[id].clone();
        stars.sort_unstable();
        stars.dedup();
        let length = self.base.circles.len();
        let mut changed = stars.clone();
        changed.extend(length - stars.len()..length);
        changed.sort_unstable();
        changed.dedup();
        let mut vertices = Vec::with_capacity(changed.len() * 3);
        let mut edge_ids = Vec::with_capacity(changed.len() * 6);
        let faces = changed
            .iter()
            .map(|&f| {
                let t = &self.base.topology.triangles[f * 3..f * 3 + 3];
                vertices.extend(t.iter().copied());
                for e in f * 3..f * 3 + 3 {
                    edge_ids.push(e);
                    let h = self.base.topology.halfedges[e];
                    if h != EMPTY {
                        edge_ids.push(h);
                    }
                }
                (f, [t[0], t[1], t[2]], self.base.circles[f])
            })
            .collect();
        vertices.sort_unstable();
        vertices.dedup();
        edge_ids.sort_unstable();
        edge_ids.dedup();
        let mut affected = self.base.neighbors[id].clone();
        affected.push(id);
        let undo = RemovalUndo {
            start_face: length - stars.len(),
            length,
            faces,
            edges: edge_ids
                .into_iter()
                .map(|e| (e, self.base.topology.halfedges[e]))
                .collect(),
            incident: vertices
                .into_iter()
                .map(|v| (v, self.base.incident[v].clone()))
                .collect(),
            neighbors: affected
                .iter()
                .map(|&v| (v, self.base.neighbors[v].clone()))
                .collect(),
            cells: affected
                .iter()
                .map(|&v| (v, std::mem::take(&mut self.cells[v])))
                .collect(),
            index: self.base.circle_index.take(),
        };
        for &f in stars.iter().rev() {
            remove_face(&mut self.base, f);
        }
        let first = self.base.circles.len();
        for (k, t) in new.iter().enumerate() {
            let f = first + k;
            self.base.topology.triangles.extend(t);
            self.base.topology.halfedges.extend([EMPTY; 3]);
            self.base.circles.push(circles[k]);
            for &v in t {
                self.base.incident[v].push(f);
            }
        }
        // 既存の空洞境界、および新しい内部辺を接続する。
        for f in first..self.base.circles.len() {
            for k in 0..3 {
                let e = f * 3 + k;
                let a = self.base.topology.triangles[e];
                let b = self.base.topology.triangles[f * 3 + (k + 1) % 3];
                let h = self.base.incident[b]
                    .iter()
                    .copied()
                    .flat_map(|t| t * 3..t * 3 + 3)
                    .find(|&h| {
                        self.base.topology.triangles[h] == b
                            && self.base.topology.triangles[h / 3 * 3 + (h + 1) % 3] == a
                    });
                if let Some(h) = h {
                    self.base.topology.halfedges[e] = h;
                    self.base.topology.halfedges[h] = e;
                }
            }
        }
        for v in affected {
            let mut neighbors = self.base.incident[v]
                .iter()
                .flat_map(|&f| {
                    self.base.topology.triangles[f * 3..f * 3 + 3]
                        .iter()
                        .copied()
                })
                .filter(|&u| u != v)
                .collect::<Vec<_>>();
            neighbors.sort_unstable();
            neighbors.dedup();
            self.base.neighbors[v] = neighbors;
        }
        Ok(undo)
    }
    pub fn undo_remove_local(&mut self, undo: RemovalUndo) {
        self.base.topology.triangles.resize(undo.length * 3, 0);
        self.base.topology.halfedges.resize(undo.length * 3, EMPTY);
        self.base.circles.resize(undo.length, [0.; 3]);
        for (f, t, c) in undo.faces {
            self.base.topology.triangles[f * 3..f * 3 + 3].copy_from_slice(&t);
            self.base.circles[f] = c;
        }
        for (e, h) in undo.edges {
            self.base.topology.halfedges[e] = h;
        }
        for (v, incident) in undo.incident {
            self.base.incident[v] = incident;
        }
        for (v, neighbors) in undo.neighbors {
            self.base.neighbors[v] = neighbors;
        }
        for (v, cell) in undo.cells {
            self.cells[v] = cell;
        }
        self.base.circle_index = OnceLock::new();
        if let Some(index) = undo.index {
            let _ = self.base.circle_index.set(index);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::natural_regional_surface::Work;
    #[test]
    fn local_insertion_and_removal_match_full_rebuild() -> Result<()> {
        let mut points = vec![
            [-20., -20., 0.],
            [20., -20., 0.],
            [20., 20., 0.],
            [-20., 20., 0.],
        ];
        let mut model = Fast::certified(points.clone())?;
        let mut active = vec![true; 4];
        let mut insertion_hint = 0;
        for k in 0..120 {
            let x = ((k * 71) % 127) as f64 / 8. - 8.;
            let y = ((k * 109) % 131) as f64 / 8. - 8.;
            let q = [x, y, (x * 0.4).sin() + (y * 0.7).cos()];
            points.push(q);
            active.push(true);
            insertion_hint = model.insert_local_from(q, insertion_hint)?;
            if k % 3 == 2 {
                let id = points.len() - 2;
                let _ = model.remove_local(id)?;
                active[id] = false;
            }
            let expected = Fast::certified(
                points
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &p)| active[i].then_some(p))
                    .collect(),
            )?;
            let mut a = Work::default();
            let mut b = Work::default();
            for y in 0..13 {
                for x in 0..13 {
                    let p = [-8. + x as f64 * 1.25, -8. + y as f64 * 1.25];
                    assert!(
                        (model.evaluate(p, &mut a)? - expected.evaluate(p, &mut b)?).abs() < 1e-8,
                        "insert={k} p={p:?}"
                    );
                }
            }
            for e in 0..model.base.topology.halfedges.len() {
                let h = model.base.topology.halfedges[e];
                if h != EMPTY {
                    assert_eq!(model.base.topology.halfedges[h], e);
                }
            }
        }
        Ok(())
    }
    #[test]
    fn cocircular_removal_keeps_plane_and_exact_undo() -> Result<()> {
        let mut points = vec![
            [-50., -50., -250.],
            [50., -50., -50.],
            [50., 50., 250.],
            [-50., 50., 50.],
        ];
        for y in 0..15 {
            for x in 0..15 {
                let x = x as f64 - 7.;
                let y = y as f64 - 7.;
                points.push([x, y, 2. * x + 3. * y]);
            }
        }
        let mut model = Fast::certified(points.clone())?;
        let mut active = vec![true; points.len()];
        for id in 4..points.len() {
            let undo = model.remove_local(id)?;
            active[id] = false;
            let expected = Fast::certified(
                points
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &p)| active[i].then_some(p))
                    .collect(),
            )?;
            let mut a = Work::default();
            let mut b = Work::default();
            for y in 0..9 {
                for x in 0..9 {
                    let p = [-6. + x as f64 * 1.5, -6. + y as f64 * 1.5];
                    assert!(
                        (model.evaluate(p, &mut a)? - expected.evaluate(p, &mut b)?).abs() < 1e-7,
                        "id={id}"
                    );
                }
            }
            if id % 7 == 0 {
                model.undo_remove_local(undo);
                active[id] = true;
            }
        }
        Ok(())
    }
    #[test]
    fn local_removal_matches_rebuild_and_undo() -> Result<()> {
        let mut points = vec![
            [-100., -100., 0.],
            [100., -100., 0.],
            [100., 100., 0.],
            [-100., 100., 0.],
        ];
        for k in 0..160 {
            let x = ((k * 73) % 167) as f64 / 10. - 8.;
            let y = ((k * 113) % 173) as f64 / 10. - 8.;
            points.push([x, y, (x * 0.7).sin() + (y * 0.3).cos()]);
        }
        let mut model = Fast::certified(points.clone())?;
        let mut active = vec![true; points.len()];
        for id in 4..65 {
            // 全面再構築との曲面一致、棄却時の配列の完全復元、連続採用を確認。
            let original = model.base.topology.triangles.clone();
            let edges = model.base.topology.halfedges.clone();
            let incident = model.base.incident.clone();
            let neighbors = model.base.neighbors.clone();
            let undo = model.remove_local(id)?;
            active[id] = false;
            let expected = Fast::certified(
                points
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &p)| active[i].then_some(p))
                    .collect(),
            )?;
            let mut a = Work::default();
            let mut b = Work::default();
            for y in 0..19 {
                for x in 0..19 {
                    let p = [-8. + x as f64 * 0.85, -8. + y as f64 * 0.85];
                    assert!(
                        (model.evaluate(p, &mut a)? - expected.evaluate(p, &mut b)?).abs() < 1e-8,
                        "id={id} p={p:?}"
                    );
                }
            }
            for e in 0..model.base.topology.halfedges.len() {
                let h = model.base.topology.halfedges[e];
                if h != EMPTY {
                    assert_eq!(model.base.topology.halfedges[h], e);
                }
            }
            if id % 3 == 0 {
                model.undo_remove_local(undo);
                active[id] = true;
                assert_eq!(model.base.topology.triangles, original);
                assert_eq!(model.base.topology.halfedges, edges);
                assert_eq!(model.base.incident, incident);
                assert_eq!(model.base.neighbors, neighbors);
            }
        }
        Ok(())
    }
}
