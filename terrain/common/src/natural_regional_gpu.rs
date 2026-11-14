//! 全域C1の疎な最小二乗とGPU制御点パケット。描画メッシュは生成しない。
use crate::natural_regional_surface::{Fast, Work};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
// SciPy Sobol(seed=9124, scramble=true, bits=30)と共通の2次元方向数。
const DIRECTIONS: [[u32; 30]; 2] = [
    [
        962404388, 434502604, 160727101, 112281279, 64236218, 31557553, 11142043, 5645760, 3273377,
        1220122, 770949, 390779, 188446, 72069, 35701, 28291, 14089, 5319, 2643, 1974, 633, 360,
        160, 73, 41, 29, 14, 5, 3, 1,
    ],
    [
        956932948, 575498710, 807642011, 769430397, 995829847, 565607168, 851970394, 785957586,
        960139947, 577263690, 807363426, 770947645, 996835224, 563099175, 854998890, 787763169,
        956931469, 575490219, 807638162, 769441575, 995824872, 565598543, 851983515, 785968450,
        960146523, 577272071, 807359562, 770938459, 996819725, 563090522,
    ],
];
const SHIFT: [u32; 2] = [419786823, 305652386];
#[derive(Clone)]
struct Row {
    terms: Vec<(usize, f64)>,
    rhs: f64,
}
fn sobol(i: usize) -> [f64; 2] {
    let gray = i ^ (i >> 1);
    let mut out = [0.; 2];
    for d in 0..2 {
        let mut x = SHIFT[d];
        for j in 0..30 {
            if gray & (1 << j) != 0 {
                x ^= DIRECTIONS[d][j];
            }
        }
        out[d] = x as f64 / 1073741824.;
    }
    out
}
fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
pub fn gradient(m: &Fast, bounds: [f64; 4]) -> Result<Vec<[f64; 2]>> {
    let p = &m.base.points;
    let n = p.len();
    let samples = (n * 4).next_power_of_two().clamp(16384, 262144);
    let mut w = Work::default();
    let mut rows = Vec::with_capacity(samples);
    for i in 0..samples {
        let u = sobol(i);
        let q = [
            bounds[0] + u[0] * (bounds[2] - bounds[0]),
            bounds[1] + u[1] * (bounds[3] - bounds[1]),
        ];
        let basis = m.basis(q, &mut w)?;
        let mut den = 0.;
        let mut alpha = 0.;
        let mut beta = 0.;
        let mut z0 = 0.;
        let mut xi = 0.;
        for &(j, a) in &basis {
            let d = ((q[0] - p[j][0]).powi(2) + (q[1] - p[j][1]).powi(2)).sqrt();
            den += a / d.max(1e-30);
            alpha += a * d;
            beta += a * d * d;
            z0 += a * p[j][2];
            xi += a / d.max(1e-30) * p[j][2];
        }
        alpha /= den;
        xi /= den;
        let factor = if alpha + beta > 1e-25 {
            beta / (alpha + beta)
        } else {
            0.
        };
        let rhs = z0 - (z0 * (1. - factor) + factor * xi);
        let mut terms = Vec::with_capacity(2 * basis.len());
        for &(j, a) in &basis {
            let dx = q[0] - p[j][0];
            let dy = q[1] - p[j][1];
            let r = (dx * dx + dy * dy).sqrt().max(1e-30);
            let v = a / r / den * factor;
            terms.push((j * 2, v * dx));
            terms.push((j * 2 + 1, v * dy));
        }
        rows.push(Row { terms, rhs });
    }
    let mut edges = BTreeSet::new();
    for t in m.base.topology.triangles.chunks_exact(3) {
        for e in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            edges.insert((e.0.min(e.1), e.0.max(e.1)));
        }
    }
    let mut distances = edges
        .iter()
        .map(|&(i, j)| ((p[i][0] - p[j][0]).powi(2) + (p[i][1] - p[j][1]).powi(2)).sqrt())
        .collect::<Vec<_>>();
    distances.sort_by(f64::total_cmp);
    let typical = if distances.len() % 2 == 0 {
        (distances[distances.len() / 2 - 1] + distances[distances.len() / 2]) * 0.5
    } else {
        distances[distances.len() / 2]
    };
    let edges = edges
        .into_iter()
        .map(|(i, j)| {
            let d = ((p[i][0] - p[j][0]).powi(2) + (p[i][1] - p[j][1]).powi(2))
                .sqrt()
                .min(typical * 3.);
            (i, j, 0.01 * d * d)
        })
        .collect::<Vec<_>>();
    let mut fixed = vec![false; n * 2];
    for i in 0..n {
        if m.base.topology.hull.contains(&i) {
            continue;
        }
        let values = m.base.neighbors[i]
            .iter()
            .map(|&j| p[j][2] - p[i][2])
            .collect::<Vec<_>>();
        if values.iter().any(|v| v.abs() > 1e-8)
            && (values.iter().all(|&v| v <= 0.) || values.iter().all(|&v| v >= 0.))
        {
            fixed[2 * i] = true;
            fixed[2 * i + 1] = true;
        }
    }
    let mut rhs = vec![0.; n * 2];
    let mut diagonal = vec![1e-10; n * 2];
    for row in &rows {
        for &(j, v) in &row.terms {
            rhs[j] += v * row.rhs;
            diagonal[j] += v * v;
        }
    }
    for &(i, j, v) in &edges {
        for d in 0..2 {
            diagonal[2 * i + d] += v;
            diagonal[2 * j + d] += v;
        }
    }
    for j in 0..rhs.len() {
        if fixed[j] {
            rhs[j] = 0.;
        }
    }
    let multiply = |x: &[f64]| {
        let mut y = x.iter().map(|v| v * 1e-10).collect::<Vec<_>>();
        for row in &rows {
            let a = row
                .terms
                .iter()
                .map(|&(j, v)| if fixed[j] { 0. } else { x[j] * v })
                .sum::<f64>();
            for &(j, v) in &row.terms {
                y[j] += v * a;
            }
        }
        for &(i, j, v) in &edges {
            for d in 0..2 {
                let a = 2 * i + d;
                let b = 2 * j + d;
                let xa = if fixed[a] { 0. } else { x[a] };
                let xb = if fixed[b] { 0. } else { x[b] };
                let e = (xa - xb) * v;
                y[a] += e;
                y[b] -= e;
            }
        }
        for j in 0..y.len() {
            if fixed[j] {
                y[j] = 0.;
            }
        }
        y
    };
    let mut x = vec![0.; n * 2];
    let mut r = rhs.clone();
    let mut z = r
        .iter()
        .zip(&diagonal)
        .map(|(r, d)| r / d)
        .collect::<Vec<_>>();
    let mut direction = z.clone();
    let mut rz = dot(&r, &z);
    let threshold = dot(&rhs, &rhs).sqrt().max(1e-20) * 1e-6;
    for _ in 0..300 {
        if dot(&r, &r).sqrt() <= threshold {
            break;
        }
        let ad = multiply(&direction);
        let denom = dot(&direction, &ad);
        if denom <= 1e-30 {
            break;
        }
        let alpha = rz / denom;
        for j in 0..x.len() {
            x[j] += alpha * direction[j];
            r[j] -= alpha * ad[j];
            z[j] = r[j] / diagonal[j];
        }
        let next = dot(&r, &z);
        let beta = next / rz;
        for j in 0..direction.len() {
            direction[j] = z[j] + beta * direction[j];
        }
        rz = next;
    }
    ensure!(x.iter().all(|v| v.is_finite()), "全域C1の勾配が非有限です");
    Ok(x.chunks_exact(2).map(|p| [p[0], p[1]]).collect())
}
struct Tree<'a> {
    m: &'a Fast,
    nodes: Vec<[u32; 8]>,
    refs: Vec<usize>,
    leaf_counts: Vec<usize>,
    lists: HashMap<Vec<usize>, u32>,
}
impl Tree<'_> {
    fn candidates(&self, ids: &[usize]) -> Vec<usize> {
        let mut s = BTreeSet::new();
        for &t in ids {
            for &i in &self.m.base.topology.triangles[t * 3..t * 3 + 3] {
                s.insert(i);
            }
        }
        s.into_iter().collect()
    }
    fn intersect(&self, ids: &[usize], r: [f64; 4]) -> Vec<usize> {
        ids.iter()
            .copied()
            .filter(|&t| {
                let c = self.m.base.circles[t];
                let x = c[0].clamp(r[0] - 0.01, r[2] + 0.01);
                let y = c[1].clamp(r[1] - 0.01, r[3] + 0.01);
                (c[0] - x).powi(2) + (c[1] - y).powi(2) <= c[2] * (1. + 1e-12) + 1e-7
            })
            .collect()
    }
    fn visit(&mut self, ids: &[usize], r: [f64; 4], depth: usize) -> Result<u32> {
        let ids = self.intersect(ids, r);
        let candidates = self.candidates(&ids);
        let index = self.nodes.len();
        self.nodes.push([u32::MAX; 8]);
        ensure!(
            self.nodes.len() <= 4000000,
            "GPU探索木が安全上限を超えました"
        );
        if candidates.len() > 48 && depth < 16 {
            let (mx, my) = ((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5);
            let rs = [
                [r[0], r[1], mx, my],
                [mx, r[1], r[2], my],
                [r[0], my, mx, r[3]],
                [mx, my, r[2], r[3]],
            ];
            let child = rs
                .iter()
                .map(|&r| self.intersect(&ids, r))
                .collect::<Vec<_>>();
            // 4分割しても候補が平均25%以上減らなければ、リストを複製しない。
            // GPUの256点上限を超える場合だけは必ず分割する。
            let child_counts = child
                .iter()
                .map(|v| self.candidates(v).len())
                .sum::<usize>();
            let worthwhile = candidates.len() > 256 || child_counts < candidates.len() * 3;
            if worthwhile {
                for i in 0..4 {
                    self.nodes[index][i] = self.visit(&child[i], rs[i], depth + 1)?;
                }
                return Ok(index as u32);
            }
        }
        ensure!(candidates.len() <= 256, "GPU候補数が256を超えました");
        let offset = if let Some(&offset) = self.lists.get(&candidates) {
            offset
        } else {
            let offset = self.refs.len() as u32;
            self.refs.extend_from_slice(&candidates);
            self.lists.insert(candidates.clone(), offset);
            offset
        };
        self.nodes[index][4] = offset;
        self.nodes[index][5] = candidates.len() as u32;
        self.nodes[index][6] = 0;
        self.nodes[index][7] = 0;
        self.leaf_counts.push(candidates.len());

        Ok(index as u32)
    }
}
pub fn packet(
    m: &Fast,
    g: &[[f64; 2]],
    bounds: [f64; 4],
    mut info: Value,
) -> Result<(Vec<u8>, Value)> {
    let cells = m.closed_cells()?;
    let index_bounds = [
        bounds[0] - 2.,
        bounds[1] - 2.,
        bounds[2] + 2.,
        bounds[3] + 2.,
    ];
    let mut t = Tree {
        m,
        nodes: Vec::new(),
        refs: Vec::new(),
        leaf_counts: Vec::new(),
        lists: HashMap::new(),
    };
    t.visit(
        &(0..m.base.circles.len()).collect::<Vec<_>>(),
        index_bounds,
        0,
    )?;
    let used = t.refs.iter().copied().collect::<BTreeSet<_>>();
    let mut remap = vec![u32::MAX; m.base.points.len()];
    let mut points = Vec::new();
    let mut cellbytes = Vec::new();
    let mut offset = 0u32;
    for (i, &old) in used.iter().enumerate() {
        remap[old] = i as u32;
        let p = m.base.points[old];
        for v in [p[0] / 1000., p[1] / 1000., p[2]] {
            points.extend((v as f32).to_le_bytes());
        }
        points.extend(offset.to_le_bytes());
        for v in g[old] {
            points.extend((v as f32).to_le_bytes());
        }
        points.extend((cells[old].len() as u32).to_le_bytes());
        points.extend(0u32.to_le_bytes());
        for c in &cells[old] {
            for &v in c {
                cellbytes.extend(((v / 1000.) as f32).to_le_bytes());
            }
        }
        offset += cells[old].len() as u32;
    }
    let mut nodes = Vec::new();
    for node in &t.nodes {
        for v in node {
            nodes.extend(v.to_le_bytes());
        }
    }
    let mut refs = Vec::new();
    for old in &t.refs {
        refs.extend(remap[*old].to_le_bytes());
    }
    for (k, v) in [
        ("indexStrategy", json!("cost-limited-shared-lists-v2")),
        ("uniqueCandidateLists", json!(t.lists.len())),
        ("bounds", json!(bounds)),
        ("indexBounds", json!(index_bounds)),
        ("coordinateScale", json!(1000)),
        ("controls", json!(used.len())),
        ("sourceControls", json!(m.base.points.len())),
        ("pointBytes", json!(points.len())),
        ("cellBytes", json!(cellbytes.len())),
        ("nodeBytes", json!(nodes.len())),
        ("referenceBytes", json!(refs.len())),
        ("nodeCount", json!(t.nodes.len())),
        ("leafCount", json!(t.leaf_counts.len())),
        ("maxCandidates", json!(t.leaf_counts.iter().max())),
        ("maxCellVertices", json!(cells.iter().map(Vec::len).max())),
        ("unsafeLeaves", json!(0)),
        ("format", json!("Sibson-GPU-controls-v1")),
        ("noHeightmap", json!(true)),
        ("noTriangleMesh", json!(true)),
    ] {
        info[k] = v;
    }
    let mut header = serde_json::to_vec(&info)?;
    while header.len() % 4 != 0 {
        header.push(b' ');
    }
    let mut bytes = (header.len() as u32).to_le_bytes().to_vec();
    bytes.extend(header);
    bytes.extend(points);
    bytes.extend(cellbytes);
    bytes.extend(nodes);
    bytes.extend(refs);
    Ok((bytes, info))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sobol_values_match_reference() {
        assert!((sobol(0)[0] - SHIFT[0] as f64 / 1073741824.).abs() < 1e-15);
        assert_eq!(
            sobol(1)[0],
            (SHIFT[0] ^ DIRECTIONS[0][0]) as f64 / 1073741824.
        );
    }
    #[test]
    fn sparse_tree_is_bounded_and_keeps_every_natural_neighbor() {
        let mut points = vec![
            [-1e6, -1e6, 0.],
            [1e6, -1e6, 0.],
            [-1e6, 1e6, 0.],
            [1e6, 1e6, 0.],
        ];
        for i in 0..180 {
            let a = i as f64 * 2.399963229728653;
            let r = 10. + (i as f64).sqrt() * 7.;
            points.push([r * a.cos(), r * a.sin(), i as f64]);
        }
        let m = Fast::new(points).unwrap();
        let mut tree = Tree {
            m: &m,
            nodes: Vec::new(),
            refs: Vec::new(),
            leaf_counts: Vec::new(),
            lists: HashMap::new(),
        };
        let rect = [-100000., -100000., 100000., 100000.];
        let ids = (0..m.base.circles.len()).collect::<Vec<_>>();
        tree.visit(&ids, rect, 0).unwrap();
        assert!(tree.nodes.len() < 5000);
        let mut w = Work::default();
        for y in 0..17 {
            for x in 0..17 {
                let q = [-99000. + x as f64 * 12375., -99000. + y as f64 * 12375.];
                let mut r = rect;
                let mut index = 0usize;
                loop {
                    let node = tree.nodes[index];
                    if node[0] == u32::MAX {
                        let candidates = &tree.refs[node[4] as usize..(node[4] + node[5]) as usize];
                        for (id, weight) in m.basis(q, &mut w).unwrap() {
                            if weight > 1e-10 {
                                assert!(candidates.contains(&id));
                            }
                        }
                        break;
                    }
                    let mx = (r[0] + r[2]) * 0.5;
                    let my = (r[1] + r[3]) * 0.5;
                    let right = q[0] >= mx;
                    let bottom = q[1] >= my;
                    index = node[right as usize + 2 * bottom as usize] as usize;
                    r = [
                        if right { mx } else { r[0] },
                        if bottom { my } else { r[1] },
                        if right { r[2] } else { mx },
                        if bottom { r[3] } else { my },
                    ];
                }
            }
        }
        let before = tree.refs.len();
        tree.visit(&ids, rect, 0).unwrap();
        assert_eq!(tree.refs.len(), before);
    }
    #[test]
    fn packet_has_aligned_records() {
        let m = Fast::new(vec![
            [-100., -100., 0.],
            [100., -100., 0.],
            [-100., 100., 0.],
            [100., 100., 0.],
            [0., 0., 10.],
        ])
        .unwrap();
        let (bytes, info) =
            packet(&m, &vec![[0., 0.]; 5], [-20., -20., 20., 20.], json!({})).unwrap();
        let header = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(header % 4, 0);
        assert_eq!(
            bytes.len(),
            4 + header
                + info["pointBytes"].as_u64().unwrap() as usize
                + info["cellBytes"].as_u64().unwrap() as usize
                + info["nodeBytes"].as_u64().unwrap() as usize
                + info["referenceBytes"].as_u64().unwrap() as usize
        );
    }
}
