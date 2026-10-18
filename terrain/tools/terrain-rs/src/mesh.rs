use anyhow::{Result, bail};
use rstar::{AABB, RTree, RTreeObject};
use serde::{Deserialize, Serialize};

pub type Point = [f64; 3];
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Mesh {
    pub vertices: Vec<Point>,
    pub faces: Vec<[usize; 3]>,
}
pub fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn normal(v: &[Point], f: [usize; 3]) -> Point {
    cross(sub(v[f[1]], v[f[0]]), sub(v[f[2]], v[f[0]]))
}
pub fn barycentric(v: &[Point], f: [usize; 3], p: [f64; 2]) -> [f64; 3] {
    let [a, b, c] = f.map(|i| v[i]);
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let w = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let x = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    [w, x, 1. - w - x]
}
impl Mesh {
    pub fn weld_identical(&self) -> Self {
        let mut lookup = std::collections::BTreeMap::new();
        let mut vertices = Vec::new();
        let mut mapping = Vec::new();
        for point in &self.vertices {
            let point = point.map(|v| if v == 0. { 0. } else { v });
            let index = *lookup.entry(point.map(f64::to_bits)).or_insert_with(|| {
                let index = vertices.len();
                vertices.push(point);
                index
            });
            mapping.push(index);
        }
        let mut unique = std::collections::BTreeSet::new();
        let faces = self
            .faces
            .iter()
            .filter_map(|f| {
                let face = f.map(|i| mapping[i]);
                let mut key = face;
                key.sort_unstable();
                unique.insert(key).then_some(face)
            })
            .collect();
        Self { vertices, faces }.compact()
    }
    pub fn validate_manifold(&self) -> Result<()> {
        self.validate()?;
        let mut points = std::collections::HashSet::new();
        for p in &self.vertices {
            if !points.insert(p.map(|v| if v == 0. { 0 } else { v.to_bits() })) {
                bail!("同じ保存座標の頂点が重複しています")
            }
        }
        let mut edges = std::collections::HashMap::new();
        for f in &self.faces {
            for (a, b) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
                let count = edges.entry((a.min(b), a.max(b))).or_insert(0usize);
                *count += 1;
                if *count > 2 {
                    bail!("一つの辺に三つ以上の面があります")
                }
            }
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        if self.vertices.iter().flatten().any(|x| !x.is_finite()) {
            bail!("頂点が有限ではありません")
        }
        for f in &self.faces {
            if f.iter().any(|i| *i >= self.vertices.len()) {
                bail!("頂点番号が不正です")
            };
            if normal(&self.vertices, *f)[2] == 0. {
                bail!("退化した三角形です")
            }
        }
        Ok(())
    }
    pub fn compact(&self) -> Self {
        let mut used = vec![false; self.vertices.len()];
        for f in &self.faces {
            for i in f {
                used[*i] = true
            }
        }
        let mut map = vec![usize::MAX; used.len()];
        let mut vertices = Vec::new();
        for (i, p) in self.vertices.iter().enumerate() {
            if used[i] {
                map[i] = vertices.len();
                vertices.push(*p)
            }
        }
        Self {
            vertices,
            faces: self.faces.iter().map(|f| f.map(|i| map[i])).collect(),
        }
    }
    pub fn probes(&self) -> Vec<Point> {
        let mut p = self.vertices.clone();
        p.extend(self.faces.iter().map(|f| {
            let [a, b, c] = f.map(|i| self.vertices[i]);
            [
                (a[0] + b[0] + c[0]) / 3.,
                (a[1] + b[1] + c[1]) / 3.,
                (a[2] + b[2] + c[2]) / 3.,
            ]
        }));
        p
    }
}
#[derive(Clone)]
pub struct TriangleIndex {
    pub index: usize,
    pub envelope: AABB<[f64; 2]>,
}
impl RTreeObject for TriangleIndex {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}
pub struct Surface<'a> {
    pub mesh: &'a Mesh,
    tree: RTree<TriangleIndex>,
}
impl<'a> Surface<'a> {
    pub fn new(mesh: &'a Mesh) -> Self {
        let triangles = mesh
            .faces
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let mut lo = [f64::INFINITY; 2];
                let mut hi = [f64::NEG_INFINITY; 2];
                for v in f {
                    for axis in 0..2 {
                        lo[axis] = lo[axis].min(mesh.vertices[*v][axis]);
                        hi[axis] = hi[axis].max(mesh.vertices[*v][axis]);
                    }
                }
                TriangleIndex {
                    index: i,
                    envelope: AABB::from_corners(lo, hi),
                }
            })
            .collect();
        Self {
            mesh,
            tree: RTree::bulk_load(triangles),
        }
    }
    pub fn height(&self, p: [f64; 2]) -> Option<f64> {
        self.sample(p).map(|(_, height)| height)
    }
    pub fn sample(&self, p: [f64; 2]) -> Option<(usize, f64)> {
        let eps = 1e-7;
        let bbox = AABB::from_corners([p[0] - eps, p[1] - eps], [p[0] + eps, p[1] + eps]);
        for triangle in self.tree.locate_in_envelope_intersecting(&bbox) {
            let f = self.mesh.faces[triangle.index];
            let w = barycentric(&self.mesh.vertices, f, p);
            if w.iter().all(|v| v.is_finite() && *v >= -1e-8) {
                return Some((
                    triangle.index,
                    (0..3).map(|i| w[i] * self.mesh.vertices[f[i]][2]).sum(),
                ));
            }
        }
        None
    }
}
