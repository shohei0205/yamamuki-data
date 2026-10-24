//! 細かい側が所有するLOD接合帯。中央の二分割・四分割を検証する地域試験。
mod seam_shared;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io::{self, Read},
};
type P = [f64; 3];
type F = [usize; 3];
#[derive(Clone, Deserialize)]
struct Mesh {
    points: Vec<P>,
    faces: Vec<F>,
}
#[derive(Deserialize)]
struct Request {
    meshes: Vec<Mesh>,
    errors: Vec<f64>,
    #[serde(default)]
    quad: bool,
    #[serde(default)]
    axis: usize,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    detail: Option<Mesh>,
    #[serde(default)]
    shared: bool,
}
fn yes() -> bool {
    true
}
#[derive(Clone)]
struct Cell {
    tile: usize,
    face: usize,
    poly: Vec<P>,
}
fn key(x: f64) -> i64 {
    (x * 1e6).round() as i64
}
fn xy(p: P) -> [i64; 2] {
    [key(p[0]), key(p[1])]
}
fn cross(a: P, b: P, c: P) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn clip(poly: Vec<P>, axis: usize, value: f64, side: f64) -> Vec<P> {
    let mut out = Vec::new();
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let ia = side * (a[axis] - value) >= -1e-8;
        let ib = side * (b[axis] - value) >= -1e-8;
        if ia {
            out.push(a)
        }
        if ia != ib {
            let t = (value - a[axis]) / (b[axis] - a[axis]);
            let mut p = std::array::from_fn(|k| a[k] + t * (b[k] - a[k]));
            p[axis] = value;
            out.push(p)
        }
    }
    out.dedup_by(|a, b| xy(*a) == xy(*b));
    if out.len() > 1 && xy(out[0]) == xy(*out.last().unwrap()) {
        out.pop();
    }
    if out.len() < 3 {
        out.clear()
    }
    out
}
// 面積重心を使い、境界点の密度によって内点が接合線へ偏るのを防ぐ。
fn centroid(poly: &[P]) -> P {
    let mut s = [0.; 3];
    let mut area = 0.;
    for i in 1..poly.len() - 1 {
        let w = cross(poly[0], poly[i], poly[i + 1]);
        area += w;
        for k in 0..3 {
            s[k] += w * (poly[0][k] + poly[i][k] + poly[i + 1][k]) / 3.;
        }
    }
    for v in &mut s {
        *v /= area
    }
    s
}
fn height(mesh: &Mesh, face: usize, p: P) -> f64 {
    let [a, b, c] = mesh.faces[face].map(|i| mesh.points[i]);
    let den = cross(a, b, c);
    let u = cross(a, p, c) / den;
    let v = cross(a, b, p) / den;
    a[2] + u * (b[2] - a[2]) + v * (c[2] - a[2])
}
fn angle(a: P, b: P, c: P) -> f64 {
    let x = [b[0] - a[0], b[1] - a[1]];
    let y = [c[0] - a[0], c[1] - a[1]];
    ((x[0] * y[0] + x[1] * y[1]) / (x[0].hypot(x[1]) * y[0].hypot(y[1])))
        .clamp(-1., 1.)
        .acos()
        .to_degrees()
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Patch {
    owner: usize,
    tile: usize,
    points: Vec<P>,
    faces: Vec<F>,
    removed_coarse_faces: Vec<usize>,
    face_ids: Vec<usize>,
    vertex_ids: Vec<usize>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Stats {
    vertices: usize,
    triangles: usize,
    patch_triangles: usize,
    extra_vertices: usize,
    minimum_angle: f64,
    sharp_triangles: usize,
}
#[derive(Serialize)]
struct Output {
    points: Vec<P>,
    faces: Vec<F>,
    roles: Vec<usize>,
    patches: Vec<Patch>,
    stats: Stats,
}
// 同じ所有領域の内部だけで対角線を交換する。接合線と外周は動かさない。
fn improve_edges(points: &[P], faces: &mut [F], tags: &[(usize, usize)]) {
    for _ in 0..12 {
        let mut edges = BTreeMap::<[usize; 2], Vec<usize>>::new();
        for (i, f) in faces.iter().enumerate() {
            for k in 0..3 {
                let mut e = [f[k], f[(k + 1) % 3]];
                e.sort();
                edges.entry(e).or_default().push(i);
            }
        }
        let mut busy = vec![false; faces.len()];
        let mut changed = false;
        let minangle = |f: F| {
            (0..3)
                .map(|k| angle(points[f[k]], points[f[(k + 1) % 3]], points[f[(k + 2) % 3]]))
                .fold(180_f64, f64::min)
        };
        for (&[a, b], pair) in &edges {
            if pair.len() != 2 {
                continue;
            }
            let (i, j) = (pair[0], pair[1]);
            if busy[i] || busy[j] || tags[i] != tags[j] {
                continue;
            }
            let old = minangle(faces[i]).min(minangle(faces[j]));
            if old >= 10. {
                continue;
            }
            let c = *faces[i].iter().find(|&&v| v != a && v != b).unwrap();
            let d = *faces[j].iter().find(|&&v| v != a && v != b).unwrap();
            let mut other = [c, d];
            other.sort();
            if edges.contains_key(&other) {
                continue;
            }
            if cross(points[c], points[d], points[a]) * cross(points[c], points[d], points[b])
                >= -1e-8
            {
                continue;
            }
            let mut f = [c, d, a];
            let mut g = [d, c, b];
            if cross(points[f[0]], points[f[1]], points[f[2]]) < 0. {
                f.swap(1, 2)
            }
            if cross(points[g[0]], points[g[1]], points[g[2]]) < 0. {
                g.swap(1, 2)
            }
            let new = minangle(f).min(minangle(g));
            if new <= old + 1e-7 {
                continue;
            }
            faces[i] = f;
            faces[j] = g;
            busy[i] = true;
            busy[j] = true;
            changed = true;
        }
        if !changed {
            break;
        }
    }
}
fn build(r: &Request) -> Result<Output> {
    let count = if r.quad { 4 } else { 2 };
    ensure!(
        r.meshes.len() == count && r.errors.len() == count && r.axis < 2,
        "タイル構成が不正"
    );
    for m in &r.meshes {
        ensure!(
            m.points.iter().flatten().all(|v| v.is_finite()),
            "座標が有限値ではない"
        );
        ensure!(
            m.faces.iter().flatten().all(|&v| v < m.points.len()),
            "頂点番号が範囲外"
        );
    }
    // 同じ形状の領域同士なら接合帯自体を作らず、元の面をそのまま使う。
    if r.meshes
        .iter()
        .all(|m| m.points == r.meshes[0].points && m.faces == r.meshes[0].faces)
    {
        let m = &r.meshes[0];
        let angles = m
            .faces
            .iter()
            .map(|f| {
                (0..3)
                    .map(|k| {
                        angle(
                            m.points[f[k]],
                            m.points[f[(k + 1) % 3]],
                            m.points[f[(k + 2) % 3]],
                        )
                    })
                    .fold(180_f64, f64::min)
            })
            .collect::<Vec<_>>();
        return Ok(Output {
            points: m.points.clone(),
            faces: m.faces.clone(),
            roles: vec![0; m.faces.len()],
            patches: vec![],
            stats: Stats {
                vertices: m.points.len(),
                triangles: m.faces.len(),
                patch_triangles: 0,
                extra_vertices: 0,
                minimum_angle: angles.iter().copied().fold(180_f64, f64::min),
                sharp_triangles: angles.iter().filter(|&&a| a < 10.).count(),
            },
        });
    }
    let axes: Vec<usize> = if r.quad { vec![0, 1] } else { vec![r.axis] };
    let mut cells = Vec::new();
    // tile 0,1 は下段、2,3 は上段。
    for tile in 0..count {
        for (fid, f) in r.meshes[tile].faces.iter().enumerate() {
            let mut poly = f
                .iter()
                .map(|&v| r.meshes[tile].points[v])
                .collect::<Vec<_>>();
            for &a in &axes {
                let bit = if r.quad { (tile >> a) & 1 } else { tile }; // Xは最下位、Yは次のビット。
                poly = clip(poly, a, 8000., if bit == 0 { -1. } else { 1. });
                if poly.is_empty() {
                    break;
                }
            }
            if !poly.is_empty() {
                cells.push(Cell {
                    tile,
                    face: fid,
                    poly,
                })
            }
        }
    }
    // 各保存領域の接合線上の区分線形プロファイル。
    let mut profiles = vec![vec![BTreeMap::<i64, f64>::new(); count]; 2];
    for cell in &cells {
        for &a in &axes {
            for p in &cell.poly {
                if key(p[a]) == key(8000.) {
                    profiles[a][cell.tile].insert(key(p[1 - a]), p[2]);
                }
            }
        }
    }
    let profile_z = |a: usize, t: usize, k: i64| -> Option<f64> {
        let row = &profiles[a][t];
        if let Some(v) = row.get(&k) {
            return Some(*v);
        }
        let (&lo, &zl) = row.range(..k).next_back()?;
        let (&hi, &zh) = row.range(k..).next()?;
        Some(zl + (zh - zl) * (k - lo) as f64 / (hi - lo) as f64)
    };
    let choose = |a: usize, k: i64| -> Option<(usize, f64)> {
        (0..count)
            .filter_map(|t| profile_z(a, t, k).map(|z| (t, z)))
            .min_by(|x, y| r.errors[x.0].total_cmp(&r.errors[y.0]).then(x.0.cmp(&y.0)))
    };
    let mut seams = vec![BTreeMap::<i64, (usize, f64)>::new(); 2];
    for &a in &axes {
        for row in &profiles[a] {
            for &k in row.keys() {
                seams[a].insert(k, choose(a, k).unwrap());
            }
        }
    }
    // 四領域の交点は、最も詳細な領域の同じ高さを両軸に使う。
    if r.quad {
        let k = key(8000.);
        let best = axes
            .iter()
            .filter_map(|&a| choose(a, k))
            .min_by(|x, y| r.errors[x.0].total_cmp(&r.errors[y.0]).then(x.0.cmp(&y.0)));
        if let Some(v) = best {
            seams[0].insert(k, v);
            seams[1].insert(k, v);
        }
    }
    let mut points = Vec::<P>::new();
    let mut faces = Vec::<F>::new();
    let mut roles = Vec::new();
    let mut tags = Vec::new();
    let mut vertices = HashMap::<[i64; 3], usize>::new();
    let mut groups = BTreeMap::<(usize, usize), (Vec<usize>, Vec<usize>)>::new();
    fn vertex(p: P, points: &mut Vec<P>, map: &mut HashMap<[i64; 3], usize>) -> usize {
        let k = [key(p[0]), key(p[1]), (p[2] * 1e7).round() as i64];
        *map.entry(k).or_insert_with(|| {
            let i = points.len();
            points.push([k[0] as f64 / 1e6, k[1] as f64 / 1e6, k[2] as f64 / 1e7]);
            i
        })
    }
    for cell in cells {
        let mut touched = false;
        let mut owner = cell.tile;
        let mut poly = Vec::new();
        for i in 0..cell.poly.len() {
            let mut a = cell.poly[i];
            let b = cell.poly[(i + 1) % cell.poly.len()];
            if r.enabled {
                for &axis in &axes {
                    if key(a[axis]) == key(8000.) {
                        let k = key(a[1 - axis]);
                        a[1 - axis] = k as f64 / 1e6;
                        a[2] = seams[axis][&k].1;
                    }
                }
            }
            poly.push(a);
            if r.enabled {
                for &axis in &axes {
                    if key(a[axis]) == key(8000.) && key(b[axis]) == key(8000.) {
                        touched = true;
                        let lo = key(a[1 - axis]).min(key(b[1 - axis]));
                        let hi = key(a[1 - axis]).max(key(b[1 - axis]));
                        let mut inside = seams[axis]
                            .range((std::ops::Bound::Excluded(lo), std::ops::Bound::Excluded(hi)))
                            .map(|(&k, &v)| (k, v))
                            .collect::<Vec<_>>();
                        // 所有者は辺全体の中点で決定。角だけが細かい領域にも一意な所有者を与える。
                        if let Some((candidate, _)) = choose(axis, (lo + hi) / 2) {
                            if r.errors[candidate] < r.errors[owner]
                                || (r.errors[candidate] == r.errors[owner] && candidate < owner)
                            {
                                owner = candidate
                            }
                        }
                        if a[1 - axis] > b[1 - axis] {
                            inside.reverse()
                        }
                        for (k, (_, z)) in inside {
                            let mut p = [0., 0., z];
                            p[axis] = 8000.;
                            p[1 - axis] = k as f64 / 1e6;
                            poly.push(p);
                        }
                    }
                }
            }
        }
        let ids = poly
            .iter()
            .map(|&p| vertex(p, &mut points, &mut vertices))
            .collect::<Vec<_>>();
        let mut tris = Vec::new();
        if touched {
            let mut center = centroid(&cell.poly);
            center[2] = height(&r.meshes[cell.tile], cell.face, center);
            let ci = vertex(center, &mut points, &mut vertices);
            for i in 0..ids.len() {
                tris.push([ci, ids[i], ids[(i + 1) % ids.len()]])
            }
        } else {
            for i in 1..ids.len() - 1 {
                tris.push([ids[0], ids[i], ids[i + 1]])
            }
        }
        let patch = touched && r.errors[owner] < r.errors[cell.tile];
        for f in tris {
            if cross(points[f[0]], points[f[1]], points[f[2]]) <= 1e-6 {
                continue;
            }
            if patch {
                let entry = groups.entry((owner, cell.tile)).or_default();
                entry.0.push(cell.face);
                entry.1.push(faces.len());
            }
            faces.push(f);
            tags.push((cell.tile, if patch { owner } else { cell.tile }));
            roles.push(if patch {
                2
            } else if cell.tile == 0 {
                0
            } else {
                1
            });
        }
    }
    improve_edges(&points, &mut faces, &tags);
    let mut minimum_angle = 180_f64;
    let mut sharp = 0;
    for f in &faces {
        let a = (0..3)
            .map(|k| angle(points[f[k]], points[f[(k + 1) % 3]], points[f[(k + 2) % 3]]))
            .fold(180_f64, f64::min);
        minimum_angle = minimum_angle.min(a);
        if a < 10. {
            sharp += 1
        }
    }
    let mut patches = Vec::new();
    let mut extra_vertices = 0;
    for ((owner, tile), (mut removed, face_ids)) in groups {
        removed.sort_unstable();
        removed.dedup();
        let mut ids = face_ids.iter().flat_map(|&i| faces[i]).collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        let map = ids
            .iter()
            .enumerate()
            .map(|(i, &v)| (v, i))
            .collect::<HashMap<_, _>>();
        let original = r.meshes[tile]
            .points
            .iter()
            .map(|&p| xy(p))
            .collect::<std::collections::HashSet<_>>();
        extra_vertices += ids
            .iter()
            .filter(|&&v| !original.contains(&xy(points[v])))
            .count();
        patches.push(Patch {
            owner,
            tile,
            points: ids.iter().map(|&i| points[i]).collect(),
            faces: face_ids
                .iter()
                .map(|&i| faces[i].map(|v| map[&v]))
                .collect(),
            removed_coarse_faces: removed,
            face_ids,
            vertex_ids: ids,
        });
    }
    let stats = Stats {
        vertices: points.len(),
        triangles: faces.len(),
        patch_triangles: patches.iter().map(|p| p.faces.len()).sum(),
        extra_vertices,
        minimum_angle,
        sharp_triangles: sharp,
    };
    Ok(Output {
        points,
        faces,
        roles,
        patches,
        stats,
    })
}
fn main() -> Result<()> {
    let mut s = String::new();
    io::stdin().read_to_string(&mut s)?;
    let r: Request = serde_json::from_str(&s)?;
    let out = if r.shared {
        seam_shared::build_shared(&r)?
    } else {
        serde_json::to_value(build(&r)?)?
    };
    serde_json::to_writer(io::stdout(), &out)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn mesh(z: f64) -> Mesh {
        Mesh {
            points: vec![
                [0., 0., 0.],
                [16000., 0., 0.],
                [16000., 16000., 0.],
                [0., 16000., 0.],
                [8000., 8000., z],
            ],
            faces: vec![[0, 1, 4], [1, 2, 4], [2, 3, 4], [3, 0, 4]],
        }
    }
    fn check(o: &Output) {
        let mut edges = HashMap::<[usize; 2], usize>::new();
        let mut area = 0.;
        for &f in &o.faces {
            let p = f.map(|v| o.points[v]);
            let a = cross(p[0], p[1], p[2]);
            assert!(a > 0.);
            area += a;
            for i in 0..3 {
                let mut e = [f[i], f[(i + 1) % 3]];
                e.sort();
                *edges.entry(e).or_default() += 1;
            }
        }
        assert!((area - 512000000.).abs() < 1.);
        for ([a, b], n) in edges {
            assert!(n == 1 || n == 2);
            if n == 1 {
                assert!(
                    (0..2).any(|k| o.points[a][k] == o.points[b][k]
                        && (o.points[a][k] == 0. || o.points[a][k] == 16000.)),
                    "内部の未接続辺 {:?} {:?}",
                    o.points[a],
                    o.points[b]
                );
            }
        }
    }
    #[test]
    fn four_corners_share_one_height() {
        let r = Request {
            meshes: vec![mesh(100.), mesh(50.), mesh(20.), mesh(0.)],
            errors: vec![25., 50., 100., 200.],
            quad: true,
            axis: 0,
            enabled: true,
            detail: None,
            shared: false,
        };
        let o = build(&r).unwrap();
        check(&o);
        let center = o
            .points
            .iter()
            .filter(|p| p[0] == 8000. && p[1] == 8000.)
            .collect::<Vec<_>>();
        assert_eq!(center.len(), 1);
        assert_eq!(center[0][2], 100.);
        assert!(o.patches.len() >= 2);
    }
    #[test]
    fn both_axes_connect() {
        for axis in 0..2 {
            let r = Request {
                meshes: vec![mesh(100.), mesh(0.)],
                errors: vec![25., 50.],
                quad: false,
                axis,
                enabled: true,
                detail: None,
                shared: false,
            };
            check(&build(&r).unwrap());
        }
    }
    #[test]
    fn equal_levels_preserve_original_faces() {
        let m = mesh(100.);
        let r = Request {
            meshes: vec![m.clone(); 4],
            errors: vec![25.; 4],
            quad: true,
            axis: 0,
            enabled: true,
            detail: None,
            shared: false,
        };
        let o = build(&r).unwrap();
        assert_eq!(o.points, m.points);
        assert_eq!(o.faces, m.faces);
        assert!(o.patches.is_empty());
    }
    #[test]
    fn quality_flip_stays_inside_owner() {
        let p = vec![[0., 0., 0.], [10., 0., 0.], [10., 1., 0.], [0., 2., 0.]];
        let original = vec![[0, 1, 3], [1, 2, 3]];
        let mut locked = original.clone();
        improve_edges(&p, &mut locked, &[(0, 0), (1, 1)]);
        assert_eq!(locked, original);
        let mut f = original.clone();
        improve_edges(&p, &mut f, &[(0, 0), (0, 0)]);
        assert_ne!(f, original);
        let min = |fs: &[F]| {
            fs.iter()
                .flat_map(|t| (0..3).map(|k| angle(p[t[k]], p[t[(k + 1) % 3]], p[t[(k + 2) % 3]])))
                .fold(180_f64, f64::min)
        };
        assert!(min(&f) > min(&original));
    }
    #[test]
    fn invalid_indices_fail() {
        let mut m = mesh(0.);
        m.faces[0][0] = 99;
        assert!(
            build(&Request {
                meshes: vec![m.clone(), m],
                errors: vec![1., 2.],
                quad: false,
                axis: 0,
                enabled: true,
                detail: None,
                shared: false
            })
            .is_err()
        );
    }
}
