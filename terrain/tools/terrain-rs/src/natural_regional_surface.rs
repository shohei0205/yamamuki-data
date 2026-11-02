//! 地域試験用の事前計算セル。自然近傍の式と全画素監査を保ち、照会間の領域を再利用する。
use crate::natural_surface::Surface;
use anyhow::{Result, ensure};
use std::sync::OnceLock;

pub fn clip(poly: &[[f64; 2]], a: f64, b: f64, c: f64) -> Vec<[f64; 2]> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    if poly.is_empty() {
        return out;
    }
    let mut s = poly[poly.len() - 1];
    let mut ds = a * s[0] + b * s[1] - c;
    for &e in poly {
        let de = a * e[0] + b * e[1] - c;
        if (ds <= 0.) != (de <= 0.) {
            let t = ds / (ds - de);
            out.push([s[0] + t * (e[0] - s[0]), s[1] + t * (e[1] - s[1])]);
        }
        if de <= 0. {
            out.push(e);
        }
        s = e;
        ds = de;
    }
    out
}
fn area(p: &[[f64; 2]]) -> f64 {
    if p.len() < 3 {
        return 0.;
    }
    let a = p[0];
    (1..p.len() - 1)
        .map(|i| (p[i][0] - a[0]) * (p[i + 1][1] - a[1]) - (p[i][1] - a[1]) * (p[i + 1][0] - a[0]))
        .sum::<f64>()
        .abs()
        * 0.5
}
fn clip_into(poly: &[[f64; 2]], a: f64, b: f64, c: f64, out: &mut Vec<[f64; 2]>) {
    out.clear();
    if poly.is_empty() {
        return;
    }
    let mut s = poly[poly.len() - 1];
    let mut ds = a * s[0] + b * s[1] - c;
    for &e in poly {
        let de = a * e[0] + b * e[1] - c;
        if (ds <= 0.) != (de <= 0.) {
            let t = ds / (ds - de);
            out.push([s[0] + t * (e[0] - s[0]), s[1] + t * (e[1] - s[1])]);
        }
        if de <= 0. {
            out.push(e);
        }
        s = e;
        ds = de;
    }
}
// 有限セルは切断結果の面積だけが必要。頂点列を毎画素ごとに確保しない。
fn clipped_area(poly: &[[f64; 2]], a: f64, b: f64, c: f64) -> f64 {
    if poly.is_empty() {
        return 0.;
    }
    let mut first = None;
    let mut last = [0.; 2];
    let mut sum = 0.;
    let mut emit = |p: [f64; 2]| {
        if let Some(o) = first {
            let o: [f64; 2] = o;
            sum += (last[0] - o[0]) * (p[1] - o[1]) - (last[1] - o[1]) * (p[0] - o[0]);
        } else {
            first = Some(p);
        }
        last = p;
    };
    let mut s = poly[poly.len() - 1];
    let mut ds = a * s[0] + b * s[1] - c;
    for &e in poly {
        let de = a * e[0] + b * e[1] - c;
        if (ds <= 0.) != (de <= 0.) {
            let t = ds / (ds - de);
            emit([s[0] + t * (e[0] - s[0]), s[1] + t * (e[1] - s[1])]);
        }
        if de <= 0. {
            emit(e);
        }
        s = e;
        ds = de;
    }
    sum.abs() * 0.5
}
#[derive(Default)]
pub struct Work {
    start: usize,
    stamp: u32,
    seen: Vec<u32>,
    marked: Vec<u32>,
    stack: Vec<usize>,
    ids: Vec<usize>,
    query_cell: Vec<[f64; 2]>,
    poly: Vec<[f64; 2]>,
    temporary: Vec<[f64; 2]>,
    weights: Vec<(usize, f64)>,
}
pub struct Fast {
    pub base: Surface,
    cells: Vec<OnceLock<Vec<[f64; 2]>>>,
}
impl Fast {
    pub fn new(points: Vec<[f64; 3]>) -> Result<Self> {
        let base = Surface::new(points)?;
        let cells = (0..base.points.len()).map(|_| OnceLock::new()).collect();
        Ok(Self { base, cells })
    }
    // 元の面の検証済み点から一つだけ削除するため、XY重複の再検査は不要。
    pub fn without_point(&self, index: usize) -> Result<Self> {
        ensure!(index < self.base.points.len(), "削除する頂点番号が不正です");
        let points = self.base.points.iter().enumerate().filter_map(|(i,&p)| (i!=index).then_some(p)).collect();
        let base = Surface::verified_subset(points)?;
        let cells = (0..base.points.len()).map(|_|OnceLock::new()).collect();
        Ok(Self {base,cells})
    }
    // 削減試行の大半は棄却される。照会した点のセルだけ従来と同じ式で作る。
    fn cell(&self, i: usize) -> &Vec<[f64; 2]> {
        self.cells[i].get_or_init(|| {
            let base = &self.base;
            if base.topology.hull.contains(&i) { return Vec::new(); }
            let p = base.points[i];
            let mut ring = base.incident[i].iter()
                .map(|&t| [base.circles[t][0]-p[0],base.circles[t][1]-p[1]])
                .collect::<Vec<_>>();
            ring.sort_by(|a,b| a[1].atan2(a[0]).total_cmp(&b[1].atan2(b[0])));
            let valid = ring.len() >= 3 && base.neighbors[i].iter().all(|&j| {
                let q = base.points[j]; let d = [q[0]-p[0],q[1]-p[1]];
                let sq = d[0]*d[0]+d[1]*d[1];
                ring.iter().all(|v| v[0]*d[0]+v[1]*d[1]-sq*0.5 <= 1e-9*sq.max(1.))
            });
            if valid { ring } else { Vec::new() }
        })
    }
    pub fn basis(&self, q: [f64; 2], w: &mut Work) -> Result<Vec<(usize, f64)>> {
        self.fill_basis(q, w)?;
        Ok(w.weights.clone())
    }
    fn fill_basis(&self, q: [f64; 2], w: &mut Work) -> Result<()> {
        w.weights.clear();
        let (face, bary) = self.base.locate(q, w.start)?;
        w.start = face;
        for i in 0..3 {
            let e = 3 * face + (i + 1) % 3;
            if bary[i].abs() < 1e-11 && self.base.topology.halfedges[e] == delaunator::EMPTY {
                w.weights.extend((0..3).filter_map(|j| {
                    (bary[j] > 0.).then_some((self.base.topology.triangles[3 * face + j], bary[j]))
                }));
                return Ok(());
            }
        }
        w.seen.resize(self.base.circles.len(), 0);
        w.marked.resize(self.base.points.len(), 0);
        if w.stamp == u32::MAX {
            w.seen.fill(0);
            w.marked.fill(0);
            w.stamp = 0;
        }
        w.stamp += 1;
        w.stack.clear();
        w.ids.clear();
        w.stack.push(face);
        while let Some(t) = w.stack.pop() {
            if w.seen[t] == w.stamp {
                continue;
            }
            w.seen[t] = w.stamp;
            let c = self.base.circles[t];
            if t != face && (c[0] - q[0]).powi(2) + (c[1] - q[1]).powi(2) > c[2] * (1. + 1e-12) {
                continue;
            }
            for e in 3 * t..3 * t + 3 {
                let v = self.base.topology.triangles[e];
                if w.marked[v] != w.stamp {
                    w.marked[v] = w.stamp;
                    w.ids.push(v);
                }
                let h = self.base.topology.halfedges[e];
                if h != delaunator::EMPTY {
                    w.stack.push(h / 3);
                }
            }
        }
        for &v in &w.ids {
            let p = self.base.points[v];
            if (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) < 1e-18 {
                w.weights.push((v, 1.));
                return Ok(());
            }
        }
        let fallback = w.ids.iter().any(|&i| self.cell(i).is_empty());
        w.query_cell.clear();
        if fallback {
            let mut bound = 128000.;
            let mut closed = false;
            for _ in 0..40 {
                w.query_cell.clear();
                w.query_cell.extend([
                    [-bound, -bound],
                    [bound, -bound],
                    [bound, bound],
                    [-bound, bound],
                ]);
                for &v in &w.ids {
                    let p = self.base.points[v];
                    let dx = p[0] - q[0];
                    let dy = p[1] - q[1];
                    clip_into(
                        &w.query_cell,
                        dx,
                        dy,
                        (dx * dx + dy * dy) * 0.5,
                        &mut w.temporary,
                    );
                    std::mem::swap(&mut w.query_cell, &mut w.temporary);
                }
                if w.query_cell
                    .iter()
                    .all(|p| p[0].abs() < bound * 0.999999 && p[1].abs() < bound * 0.999999)
                {
                    closed = true;
                    break;
                }
                bound *= 2.;
            }
            ensure!(closed, "照会セルが閉じません");
        }
        let mut total = 0.;
        for &v in &w.ids {
            let p = self.base.points[v];
            let vx = p[0] - q[0];
            let vy = p[1] - q[1];
            let a = if fallback {
                w.poly.clear();
                w.poly.extend_from_slice(&w.query_cell);
                for &u in &self.base.neighbors[v] {
                    let r = self.base.points[u];
                    let ux = r[0] - q[0];
                    let uy = r[1] - q[1];
                    clip_into(
                        &w.poly,
                        ux - vx,
                        uy - vy,
                        (ux * ux + uy * uy - vx * vx - vy * vy) * 0.5,
                        &mut w.temporary,
                    );
                    std::mem::swap(&mut w.poly, &mut w.temporary);
                    if w.poly.is_empty() {
                        break;
                    }
                }
                area(&w.poly)
            } else {
                clipped_area(self.cell(v), vx, vy, -(vx * vx + vy * vy) * 0.5)
            };
            if a > 0. {
                w.weights.push((v, a));
                total += a;
            }
        }
        ensure!(
            total > 1e-15 && total.is_finite(),
            "自然近傍の面積が不正です"
        );
        for v in &mut w.weights {
            v.1 /= total;
        }
        Ok(())
    }
    pub fn evaluate(&self, q: [f64; 2], w: &mut Work) -> Result<f64> {
        self.fill_basis(q, w)?;
        let value = w
            .weights
            .iter()
            .map(|&(i, a)| a * self.base.points[i][2])
            .sum::<f64>();
        ensure!(value.is_finite(), "補間値が非有限です");
        Ok(value)
    }
    /// Sibsonの正の面積座標と一次精度から、区画内の標高差の上界を求める。
    /// サンプルを省いて推測するのではなく、支持点と元画素の区間を比較する。
    pub fn affine_error_bound(
        &self,
        rect: [f64; 4],
        samples: impl Iterator<Item = ([f64; 2], f64)>,
    ) -> Result<f64> {
        let center = [(rect[0] + rect[2]) * 0.5, (rect[1] + rect[3]) * 0.5];
        let (face, _) = self.base.locate(center, 0)?;
        let ids = &self.base.topology.triangles[face * 3..face * 3 + 3];
        let [a, b, c] = [
            self.base.points[ids[0]],
            self.base.points[ids[1]],
            self.base.points[ids[2]],
        ];
        let ab = [b[0] - a[0], b[1] - a[1]];
        let ac = [c[0] - a[0], c[1] - a[1]];
        let det = ab[0] * ac[1] - ab[1] * ac[0];
        let gx = ((b[2] - a[2]) * ac[1] - (c[2] - a[2]) * ab[1]) / det;
        let gy = (ab[0] * (c[2] - a[2]) - ac[0] * (b[2] - a[2])) / det;
        let plane = |q: [f64; 2]| a[2] + gx * (q[0] - a[0]) + gy * (q[1] - a[1]);
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for face in self.base.faces_for_rect(rect) {
            for &i in &self.base.topology.triangles[3 * face..3 * face + 3] {
                let p = self.base.points[i];
                let v = p[2] - plane([p[0], p[1]]);
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        let mut raw_lo = f64::INFINITY;
        let mut raw_hi = f64::NEG_INFINITY;
        for (q, z) in samples {
            let v = z - plane(q);
            raw_lo = raw_lo.min(v);
            raw_hi = raw_hi.max(v);
        }
        ensure!(
            [lo, hi, raw_lo, raw_hi].iter().all(|v| v.is_finite()),
            "区間誤差を計算できません"
        );
        let margin = 0.02f64
            .max(128. * f64::EPSILON * (1. + lo.abs() + hi.abs() + raw_lo.abs() + raw_hi.abs()));
        Ok((raw_lo - hi).abs().max((raw_hi - lo).abs()) + margin)
    }
    pub fn deletion_box(&self, i: usize, margin: f64) -> [f64; 4] {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for &t in &self.base.incident[i] {
            let c = self.base.circles[t];
            let r = c[2].sqrt() + margin;
            b[0] = b[0].min(c[0] - r);
            b[1] = b[1].min(c[1] - r);
            b[2] = b[2].max(c[0] + r);
            b[3] = b[3].max(c[1] + r);
        }
        b
    }
    pub fn insertion_box(&self, q: [f64; 2]) -> [f64; 4] {
        let mut b = [q[0], q[1], q[0], q[1]];
        for t in self.base.faces_for_point(q).unwrap_or_else(|_|self.base.faces_for_rect([q[0], q[1], q[0], q[1]])) {
            let c = self.base.circles[t];
            let r = c[2].sqrt();
            b[0] = b[0].min(c[0] - r);
            b[1] = b[1].min(c[1] - r);
            b[2] = b[2].max(c[0] + r);
            b[3] = b[3].max(c[1] + r);
        }
        b
    }
    pub fn closed_cells(&self) -> Result<Vec<Vec<[f64; 2]>>> {
        let lo = self
            .base
            .points
            .iter()
            .fold([f64::INFINITY; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
        let hi = self
            .base
            .points
            .iter()
            .fold([f64::NEG_INFINITY; 2], |a, p| {
                [a[0].max(p[0]), a[1].max(p[1])]
            });
        let radius = ((hi[0] - lo[0]).max(hi[1] - lo[1])) * 8.;
        self.cells
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let c = self.cell(i);
                if !c.is_empty() {
                    return Ok(c.clone());
                }
                let p = self.base.points[i];
                let mut poly = vec![
                    [-radius, -radius],
                    [radius, -radius],
                    [radius, radius],
                    [-radius, radius],
                ];
                for &j in &self.base.neighbors[i] {
                    let r = self.base.points[j];
                    let dx = r[0] - p[0];
                    let dy = r[1] - p[1];
                    poly = clip(&poly, dx, dy, (dx * dx + dy * dy) * 0.5);
                }
                ensure!(!poly.is_empty(), "GPUの外周セルが空です");
                Ok(poly)
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_baseline_and_affine() {
        let p = vec![
            [-100., -100., -499.],
            [100., -100., -99.],
            [-100., 100., 101.],
            [100., 100., 501.],
            [0., 0., 8.],
            [5., 8., 2.],
            [-9., 7., 20.],
        ];
        let f = Fast::new(p).unwrap();
        let mut a = 0;
        let mut w = Work::default();
        for y in -12..12 {
            for x in -12..12 {
                let q = [x as f64 + 0.25, y as f64 + 0.4];
                assert!(
                    (f.evaluate(q, &mut w).unwrap() - f.base.evaluate(q, &mut a).unwrap()).abs()
                        < 1e-7
                );
            }
        }
    }
    #[test]
    fn affine_interval_contains_direct_error() {
        let p = vec![
            [-100., -100., 0.],
            [100., -100., 20.],
            [-100., 100., -10.],
            [100., 100., 60.],
            [0., 0., 70.],
            [14., -16., 9.],
            [-20., 9., 45.],
        ];
        let m = Fast::new(p).unwrap();
        let mut w = Work::default();
        for x in -4..4 {
            for y in -4..4 {
                let rect = [
                    x as f64 * 12.,
                    y as f64 * 12.,
                    x as f64 * 12. + 9.,
                    y as f64 * 12. + 9.,
                ];
                let q = (0..10)
                    .flat_map(|dy| {
                        (0..10).map(move |dx| {
                            (
                                [rect[0] + dx as f64, rect[1] + dy as f64],
                                (dx * dy) as f64 * 0.1,
                            )
                        })
                    })
                    .collect::<Vec<_>>();
                let bound = m.affine_error_bound(rect, q.iter().copied()).unwrap();
                let actual = q
                    .iter()
                    .map(|&(q, z)| (m.evaluate(q, &mut w).unwrap() - z).abs())
                    .fold(0., f64::max);
                assert!(actual <= bound + 1e-8, "{actual} > {bound}");
            }
        }
    }
    #[test]
    fn streaming_clipped_area_matches_polygon() {
        let p = vec![[-3., -3.], [2., -3.], [4., 1.], [0., 5.], [-4., 2.]];
        for a in -4..4 {
            for b in -4..4 {
                for c in -5..5 {
                    let (a, b, c) = (a as f64, b as f64, c as f64);
                    let expected = area(&clip(&p, a, b, c));
                    let actual = clipped_area(&p, a, b, c);
                    assert!((actual - expected).abs() < 1e-12);
                }
            }
        }
    }
    #[test]
    fn insertion_cavity_matches_full_circle_index() {
        let mut points=vec![[-1000.,-1000.,0.],[1000.,-1000.,0.],[-1000.,1000.,0.],[1000.,1000.,0.]];
        for y in 0..20 {for x in 0..20 {points.push([x as f64*8.-80.,y as f64*8.-80.,(x+y) as f64]);}}
        let f=Fast::new(points.clone()).unwrap();let mut queries=Vec::new();
        for y in -25..25 {for x in -25..25 {queries.push([x as f64*3.17+0.123,y as f64*2.71+0.789]);}}
        for p in points.iter().skip(4) {for d in [0.,-1e-8,1e-8] {queries.push([p[0]+d,p[1]+d]);}}
        for q in queries {
            let mut actual=f.base.faces_for_point(q).unwrap();actual.sort_unstable();
            let mut expected=f.base.faces_for_rect([q[0],q[1],q[0],q[1]]);expected.sort_unstable();
            assert_eq!(actual,expected,"query={q:?}");
            let mut bounds=[q[0],q[1],q[0],q[1]];
            for t in expected {let c=f.base.circles[t];let r=c[2].sqrt();bounds=[bounds[0].min(c[0]-r),bounds[1].min(c[1]-r),bounds[2].max(c[0]+r),bounds[3].max(c[1]+r)];}
            assert_eq!(f.insertion_box(q),bounds);
        }
    }
    #[test]
    fn verified_deletion_matches_full_rebuild() {
        let points=vec![[-100.,-100.,0.],[100.,-100.,0.],[-100.,100.,0.],[100.,100.,0.],[0.,0.,10.],[20.,10.,3.],[-20.,-10.,-2.]];
        let f=Fast::new(points.clone()).unwrap();
        for index in 4..points.len() {
            let trial=f.without_point(index).unwrap();
            let expected=Fast::new(points.iter().enumerate().filter_map(|(i,&p)|(i!=index).then_some(p)).collect()).unwrap();
            assert_eq!(trial.base.topology.triangles,expected.base.topology.triangles);
            assert_eq!(trial.base.neighbors,expected.base.neighbors);
            assert_eq!(trial.closed_cells().unwrap(),expected.closed_cells().unwrap());
        }
    }
    #[test]
    fn lazy_cells_and_index_preserve_eager_results_exactly() {
        let mut points = vec![[-100.,-100.,0.],[100.,-100.,0.],[-100.,100.,0.],[100.,100.,0.]];
        for y in 0..9 { for x in 0..9 {
            points.push([-80.+x as f64*20.,-80.+y as f64*20.,(x*x+y*3) as f64]);
        }}
        let lazy = Fast::new(points.clone()).unwrap();
        let eager = Fast::new(points).unwrap();
        assert!(lazy.cells.iter().all(|c| c.get().is_none()));
        assert!(lazy.base.circle_index.get().is_none());
        for i in 0..eager.cells.len() { eager.cell(i); }
        let rect = [-11.,-15.,12.,19.];
        let mut a = lazy.base.faces_for_rect(rect); let mut b = eager.base.faces_for_rect(rect);
        a.sort_unstable(); b.sort_unstable(); assert_eq!(a,b);
        assert!(lazy.base.circle_index.get().is_some());
        let mut lw = Work::default(); let mut ew = Work::default();
        for y in -7..7 { for x in -7..7 {
            let q = [x as f64*10.+0.123,y as f64*10.+0.789];
            assert_eq!(lazy.evaluate(q,&mut lw).unwrap().to_bits(),eager.evaluate(q,&mut ew).unwrap().to_bits());
        }}
        assert_eq!(lazy.closed_cells().unwrap(),eager.closed_cells().unwrap());
    }
    #[test]
    fn guards_are_closed_for_gpu() {
        let f = Fast::new(vec![
            [-100., -100., 0.],
            [100., -100., 0.],
            [-100., 100., 0.],
            [100., 100., 0.],
            [0., 0., 10.],
        ])
        .unwrap();
        assert!(f.closed_cells().unwrap().iter().all(|c| c.len() >= 3));
    }
}
