//! 詳細TINの既存頂点だけを使う接合。新しい座標は作らない。
use super::*;
use rstar::{AABB, PointDistance, RTree, RTreeObject};
use std::collections::HashSet;
#[derive(Clone)]
struct Candidate {
    point: [f64; 2],
    id: usize,
}
impl RTreeObject for Candidate {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.point)
    }
}
impl PointDistance for Candidate {
    fn distance_2(&self, p: &[f64; 2]) -> f64 {
        (self.point[0] - p[0]).powi(2) + (self.point[1] - p[1]).powi(2)
    }
}
fn tile(r: &Request, p: P) -> usize {
    if r.quad {
        usize::from(p[0] > 8000.) + 2 * usize::from(p[1] > 8000.)
    } else {
        usize::from(p[r.axis] > 8000.)
    }
}
fn face_key(f: F) -> F {
    let mut a = f;
    a.sort();
    a
}
fn triangulate(pool: &[P], ids: &[usize]) -> Vec<F> {
    let pts = ids
        .iter()
        .map(|&i| delaunator::Point {
            x: pool[i][0],
            y: pool[i][1],
        })
        .collect::<Vec<_>>();
    delaunator::triangulate(&pts)
        .triangles
        .chunks_exact(3)
        .map(|f| {
            let mut f = [f[0], f[1], f[2]];
            if cross(pool[ids[f[0]]], pool[ids[f[1]]], pool[ids[f[2]]]) < 0. {
                f.swap(1, 2)
            }
            f
        })
        .collect()
}
fn quality(pool: &[P], ids: &[usize], f: F) -> f64 {
    (0..3)
        .map(|k| {
            angle(
                pool[ids[f[k]]],
                pool[ids[f[(k + 1) % 3]]],
                pool[ids[f[(k + 2) % 3]]],
            )
        })
        .fold(180_f64, f64::min)
}
fn circum(a: P, b: P, c: P) -> [f64; 2] {
    let d = 2. * cross(a, b, c);
    let bx = b[0] - a[0];
    let by = b[1] - a[1];
    let cx = c[0] - a[0];
    let cy = c[1] - a[1];
    let b2 = bx * bx + by * by;
    let c2 = cx * cx + cy * cy;
    [
        a[0] + (b2 * cy - c2 * by) / d,
        a[1] + (bx * c2 - cx * b2) / d,
    ]
}
pub(super) fn build_shared(r: &Request) -> Result<serde_json::Value> {
    let reference = r
        .detail
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("詳細メッシュが必要"))?;
    let pool = &reference.points;
    let n = if r.quad { 4 } else { 2 };
    ensure!(
        r.meshes.len() == n && r.errors.len() == n && r.axis < 2 && !pool.is_empty(),
        "タイル構成が不正"
    );
    ensure!(
        pool.iter().flatten().all(|v| v.is_finite()),
        "有限座標が必要"
    );
    let map = pool
        .iter()
        .enumerate()
        .map(|(i, &p)| (xy(p), i))
        .collect::<HashMap<_, _>>();
    ensure!(map.len() == pool.len(), "詳細側に同じXYの頂点がある");
    let mut selected = HashSet::new();
    let mut originals = Vec::new();
    let mut original_ids = Vec::new();
    for (t, m) in r.meshes.iter().enumerate() {
        let ids = m
            .points
            .iter()
            .map(|&p| {
                let id = map
                    .get(&xy(p))
                    .copied()
                    .ok_or_else(|| anyhow::anyhow!("粗い側の頂点が詳細側に存在しない"))?;
                ensure!((pool[id][2] - p[2]).abs() < 1e-6, "共有頂点の高さが異なる");
                Ok(id)
            })
            .collect::<Result<Vec<_>>>()?;
        for (i, &p) in m.points.iter().enumerate() {
            if tile(r, p) == t {
                selected.insert(ids[i]);
            }
        }
        originals.push(
            m.faces
                .iter()
                .map(|f| face_key(f.map(|v| ids[v])))
                .collect::<HashSet<_>>(),
        );
        original_ids.push(ids);
    }
    // 四隅だけを保護する。詳細側の外周点をすべて固定し、粗い外周だけが密になるのを避ける。
    for (i, &p) in pool.iter().enumerate() {
        if [p[0], p[1]].iter().all(|&v| v == 0. || v == 16000.) {
            selected.insert(i);
        }
    }
    let base = selected.len();
    let tree = RTree::bulk_load(
        pool.iter()
            .enumerate()
            .map(|(id, p)| Candidate {
                point: [p[0], p[1]],
                id,
            })
            .collect(),
    );
    let mut rounds = 0;
    let (ids, faces) = loop {
        let mut ids = selected.iter().copied().collect::<Vec<_>>();
        ids.sort_unstable();
        let faces = triangulate(pool, &ids);
        let mut bad = faces
            .iter()
            .copied()
            .filter_map(|f| {
                let a = quality(pool, &ids, f);
                (a < 10. - 1e-6).then_some((a, f))
            })
            .collect::<Vec<_>>();
        bad.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
        if bad.is_empty() {
            break (ids, faces);
        }
        let mut added = Vec::new();
        for (_, f) in bad.iter().take(256) {
            let [a, b, c] = f.map(|v| pool[ids[v]]);
            let center = circum(a, b, c);
            let radius2 = (a[0] - center[0]).powi(2) + (a[1] - center[1]).powi(2);
            if let Some(p) = tree.nearest_neighbor_iter(&center).find(|p| {
                !selected.contains(&p.id) && p.distance_2(&center) < radius2 * (1. - 1e-10)
            }) {
                added.push(p.id)
            }
        }
        added.sort_unstable();
        added.dedup();
        if added.is_empty() || rounds >= 128 {
            // 候補不足のときは保証のある詳細TINへ戻す。鋭角を残して成功とはしない。
            let ids = (0..pool.len()).collect::<Vec<_>>();
            ensure!(
                reference
                    .faces
                    .iter()
                    .all(|&f| quality(pool, &ids, f) >= 10. - 1e-6),
                "詳細参照も最小角10度を満たしていない"
            );
            selected.extend(0..pool.len());
            break (ids, reference.faces.clone());
        }
        selected.extend(added);
        rounds += 1;
    };
    let points = ids.iter().map(|&i| pool[i]).collect::<Vec<_>>();
    let mut roles = Vec::new();
    let mut groups = BTreeMap::<(usize, usize), Vec<usize>>::new();
    let best = (0..n)
        .min_by(|&a, &b| r.errors[a].total_cmp(&r.errors[b]).then(a.cmp(&b)))
        .unwrap();
    for (i, &f) in faces.iter().enumerate() {
        let p = f.map(|v| points[v]);
        let center = std::array::from_fn(|k| (p[0][k] + p[1][k] + p[2][k]) / 3.);
        let t = tile(r, center);
        let source_face = face_key(f.map(|v| ids[v]));
        let crossing = p.iter().any(|&v| tile(r, v) != t);
        let changed = crossing || !originals[t].contains(&source_face);
        let patch = changed && r.errors[t] > r.errors[best];
        if patch {
            groups.entry((best, t)).or_default().push(i)
        }
        roles.push(if patch {
            2
        } else if t == best {
            0
        } else {
            1
        });
    }
    let allfaces = faces
        .iter()
        .map(|f| face_key(f.map(|v| ids[v])))
        .collect::<HashSet<_>>();
    let mut patches = Vec::new();
    let mut extra_vertices = 0;
    for ((owner, t), face_ids) in groups {
        let mut vertices = face_ids.iter().flat_map(|&i| faces[i]).collect::<Vec<_>>();
        vertices.sort_unstable();
        vertices.dedup();
        let local = vertices
            .iter()
            .enumerate()
            .map(|(i, &v)| (v, i))
            .collect::<HashMap<_, _>>();
        let original = original_ids[t].iter().copied().collect::<HashSet<_>>();
        extra_vertices += vertices
            .iter()
            .filter(|&&v| !original.contains(&ids[v]))
            .count();
        let removed = r.meshes[t]
            .faces
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                let p = f.map(|v| r.meshes[t].points[v]);
                let center = std::array::from_fn(|k| (p[0][k] + p[1][k] + p[2][k]) / 3.);
                (tile(r, center) == t
                    && !allfaces.contains(&face_key(f.map(|v| original_ids[t][v]))))
                .then_some(i)
            })
            .collect();
        patches.push(Patch {
            owner,
            tile: t,
            points: vertices.iter().map(|&v| points[v]).collect(),
            faces: face_ids
                .iter()
                .map(|&i| faces[i].map(|v| local[&v]))
                .collect(),
            removed_coarse_faces: removed,
            face_ids,
            vertex_ids: vertices,
        });
    }
    let minimum_angle = faces
        .iter()
        .map(|&f| quality(pool, &ids, f))
        .fold(180_f64, f64::min);
    let stats = Stats {
        vertices: points.len(),
        triangles: faces.len(),
        patch_triangles: patches.iter().map(|p| p.faces.len()).sum(),
        extra_vertices,
        minimum_angle,
        sharp_triangles: faces
            .iter()
            .filter(|&&f| quality(pool, &ids, f) < 10. - 1e-6)
            .count(),
    };
    let mut output = serde_json::to_value(Output {
        points,
        faces,
        roles,
        patches,
        stats,
    })?;
    output["sourceIds"] = serde_json::json!(ids);
    output["stats"]["qualityAdditions"] = serde_json::json!(selected.len() - base);
    output["stats"]["referenceVertices"] = serde_json::json!(pool.len());
    output["stats"]["fullReferenceFallback"] = serde_json::json!(selected.len() == pool.len());
    output["stats"]["refinementRounds"] = serde_json::json!(rounds);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reference() -> Mesh {
        let points = (0..25)
            .map(|i| {
                [
                    (i % 5) as f64 * 4000.,
                    (i / 5) as f64 * 4000.,
                    (i % 5 + i / 5) as f64,
                ]
            })
            .collect();
        let mut faces = Vec::new();
        for y in 0..4 {
            for x in 0..4 {
                let a = y * 5 + x;
                faces.push([a, a + 1, a + 6]);
                faces.push([a, a + 6, a + 5]);
            }
        }
        Mesh { points, faces }
    }
    #[test]
    fn every_output_vertex_is_from_reference() {
        let fine = reference();
        let coarse = Mesh {
            points: [0, 4, 24, 20].map(|i| fine.points[i]).to_vec(),
            faces: vec![[0, 1, 2], [0, 2, 3]],
        };
        let r = Request {
            meshes: vec![fine.clone(), coarse],
            errors: vec![25., 50.],
            quad: false,
            axis: 0,
            enabled: true,
            detail: Some(fine.clone()),
            shared: true,
        };
        let out = build_shared(&r).unwrap();
        let ids = out["sourceIds"].as_array().unwrap();
        for (p, id) in out["points"].as_array().unwrap().iter().zip(ids) {
            assert_eq!(
                *p,
                serde_json::json!(fine.points[id.as_u64().unwrap() as usize])
            );
        }
        assert_eq!(out["stats"]["sharpTriangles"], 0);
    }
    #[test]
    fn non_reference_vertex_is_rejected() {
        let fine = reference();
        let mut other = fine.clone();
        other.points[0][0] = 1.;
        let r = Request {
            meshes: vec![fine.clone(), other],
            errors: vec![25., 50.],
            quad: false,
            axis: 0,
            enabled: true,
            detail: Some(fine),
            shared: true,
        };
        assert!(build_shared(&r).is_err());
    }
}
