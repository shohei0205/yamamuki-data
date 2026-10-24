//! 余白付きタイルのマージ試験。既存の本体面を制約として保持する。
use anyhow::{Result, ensure};
use rstar::{AABB, PointDistance, RTree, RTreeObject};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io::{self, Read},
    time::Instant,
};
type P = [f64; 3];
type F = [usize; 3];
#[derive(Clone, Deserialize)]
struct Tile {
    bounds: [f64; 4],
    points: Vec<P>,
    faces: Vec<F>,
    support: Vec<P>,
    #[serde(default, rename = "meshKey")]
    mesh_key: String,
}
#[derive(Deserialize)]
struct Input {
    tiles: Vec<Tile>,
    width: f64,
    #[serde(default = "yes")]
    local: bool,
    #[serde(default)]
    partial: bool,
}
fn yes() -> bool {
    true
}
#[derive(Clone)]
struct C {
    p: [f64; 2],
    id: usize,
}
impl RTreeObject for C {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.p)
    }
}
impl PointDistance for C {
    fn distance_2(&self, p: &[f64; 2]) -> f64 {
        (self.p[0] - p[0]).powi(2) + (self.p[1] - p[1]).powi(2)
    }
}
fn key(p: P) -> [i64; 2] {
    [(p[0] * 1e6).round() as i64, (p[1] * 1e6).round() as i64]
}
fn cross(a: P, b: P, c: P) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn inside(p: P, b: [f64; 4]) -> bool {
    p[0] >= b[0] - 1e-7 && p[1] >= b[1] - 1e-7 && p[0] <= b[2] + 1e-7 && p[1] <= b[3] + 1e-7
}
fn segment_distance(p: P, a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}
fn compatible(a: &Tile, b: &Tile) -> bool {
    !a.mesh_key.is_empty() && a.mesh_key == b.mesh_key
}
fn seams(input: &Input) -> (Vec<([f64; 2], [f64; 2])>, usize) {
    let mut out = Vec::new();
    let mut skipped = BTreeSet::new();
    for (ti, t) in input.tiles.iter().enumerate() {
        let [x, y, u, v] = t.bounds;
        for (a, b) in [
            ([x, y], [u, y]),
            ([u, y], [u, v]),
            ([u, v], [x, v]),
            ([x, v], [x, y]),
        ] {
            if (a[0] == b[0] && (a[0] == 0. || a[0] == 16000.))
                || (a[1] == b[1] && (a[1] == 0. || a[1] == 16000.))
            {
                continue;
            }
            let axis = usize::from(a[0] != b[0]);
            let other = 1 - axis;
            let lo = a[other].min(b[other]);
            let hi = a[other].max(b[other]);
            let coordinate = a[axis];
            let mut ranges = Vec::new();
            let mut cuts = vec![lo, hi];
            for (j, q) in input.tiles.iter().enumerate() {
                if j == ti || !compatible(t, q) {
                    continue;
                }
                let opposite = if coordinate == t.bounds[axis] {
                    q.bounds[axis + 2]
                } else {
                    q.bounds[axis]
                };
                if opposite != coordinate {
                    continue;
                }
                let l = lo.max(q.bounds[other]);
                let h = hi.min(q.bounds[other + 2]);
                if h > l {
                    ranges.push((l, h));
                    cuts.extend([l, h]);
                }
            }
            cuts.sort_by(f64::total_cmp);
            cuts.dedup();
            for part in cuts.windows(2) {
                let l = part[0];
                let h = part[1];
                let mid = (l + h) / 2.;
                let mut c = a;
                let mut d = b;
                c[other] = l;
                d[other] = h;
                if ranges.iter().any(|&(l, h)| mid >= l && mid <= h) {
                    let mut pair = [key([c[0], c[1], 0.]), key([d[0], d[1], 0.])];
                    pair.sort();
                    skipped.insert(pair);
                } else {
                    out.push((c, d));
                }
            }
        }
    }
    (out, skipped.len())
}
// 同じ元メッシュの領域の和に面全体が含まれるか、面積で確認する。
fn in_compatible_union(p: &[P], f: F, input: &Input, tile: usize) -> bool {
    let area = cross(p[f[0]], p[f[1]], p[f[2]]).abs() / 2.;
    let mut sum = 0.;
    for (i, t) in input.tiles.iter().enumerate() {
        if i != tile && !compatible(&input.tiles[tile], t) {
            continue;
        }
        let mut poly = f.map(|v| [p[v][0], p[v][1]]).to_vec();
        for (axis, value, sign) in [
            (0, t.bounds[0], 1.),
            (0, t.bounds[2], -1.),
            (1, t.bounds[1], 1.),
            (1, t.bounds[3], -1.),
        ] {
            let mut next = Vec::new();
            for k in 0..poly.len() {
                let a = poly[k];
                let b = poly[(k + 1) % poly.len()];
                let ia = sign * (a[axis] - value) >= 0.;
                let ib = sign * (b[axis] - value) >= 0.;
                if ia {
                    next.push(a)
                }
                if ia != ib {
                    let t = (value - a[axis]) / (b[axis] - a[axis]);
                    let mut q = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                    q[axis] = value;
                    next.push(q)
                }
            }
            poly = next;
            if poly.is_empty() {
                break;
            }
        }
        if poly.len() > 2 {
            sum += (0..poly.len())
                .map(|i| {
                    poly[i][0] * poly[(i + 1) % poly.len()][1]
                        - poly[(i + 1) % poly.len()][0] * poly[i][1]
                })
                .sum::<f64>()
                .abs()
                / 2.;
        }
    }
    (sum - area).abs() <= 1e-6 * area.max(1.)
}
fn direct_merge(input: &Input, start: Instant, skipped: usize) -> Result<serde_json::Value> {
    let mut points = Vec::new();
    let mut lookup = HashMap::new();
    let mut faces = Vec::new();
    let mut unique = HashSet::new();
    let mut preserved = Vec::new();
    let mut roles = Vec::new();
    let mut loaded_faces = 0;
    for (ti, t) in input.tiles.iter().enumerate() {
        let mut ids = Vec::new();
        for &q in &t.points {
            let id = *lookup.entry(key(q)).or_insert_with(|| {
                let i = points.len();
                points.push(q);
                i
            });
            ensure!(
                (points[id][2] - q[2]).abs() < 1e-6,
                "同じ頂点の高さが異なる"
            );
            ids.push(id)
        }
        for (fi, &f) in t.faces.iter().enumerate() {
            ensure!(f.iter().all(|&v| v < ids.len()), "面番号が範囲外");
            let f = f.map(|v| ids[v]);
            if unique.insert(facekey(f)) {
                faces.push(f);
                preserved.push(json!({"tile":ti,"face":fi}));
                roles.push(usize::from(ti != 0));
            }
        }
        loaded_faces += t.faces.len();
    }
    let min = faces
        .iter()
        .map(|&f| minangle(&points, f))
        .fold(180_f64, f64::min);
    let sharp = faces
        .iter()
        .filter(|&&f| minangle(&points, f) < 10. - 1e-6)
        .count();
    let n = faces.len();
    let v = points.len();
    Ok(
        json!({"points":points,"faces":faces,"roles":roles,"patches":[],"preservedCore":preserved,"loadedPoolIds":(0..v).collect::<Vec<_>>(),"stats":{"vertices":v,"triangles":n,"coreFaces":n,"retainedCoreFaces":n,"patchTriangles":0,"extraVertices":0,"qualityAdditions":0,"qualitySatisfied":sharp==0,"minimumAngle":min,"sharpTriangles":sharp,"runtimeMs":start.elapsed().as_secs_f64()*1000.,"loadedVertices":v,"loadedTriangles":loaded_faces,"triangulationVertices":0,"excludedCoreVertices":v,"refinementRounds":0,"sameMeshFastPath":true,"sameLevelSkippedSegments":skipped}}),
    )
}
fn angle(a: P, b: P, c: P) -> f64 {
    let u = [b[0] - a[0], b[1] - a[1]];
    let v = [c[0] - a[0], c[1] - a[1]];
    ((u[0] * v[0] + u[1] * v[1]) / (u[0].hypot(u[1]) * v[0].hypot(v[1])))
        .clamp(-1., 1.)
        .acos()
        .to_degrees()
}
fn minangle(p: &[P], f: F) -> f64 {
    (0..3)
        .map(|k| angle(p[f[k]], p[f[(k + 1) % 3]], p[f[(k + 2) % 3]]))
        .fold(180_f64, f64::min)
}
fn circle(a: P, b: P, c: P) -> [f64; 2] {
    let d = 2. * cross(a, b, c);
    let bx = b[0] - a[0];
    let by = b[1] - a[1];
    let cx = c[0] - a[0];
    let cy = c[1] - a[1];
    let q = bx * bx + by * by;
    let r = cx * cx + cy * cy;
    [a[0] + (q * cy - r * by) / d, a[1] + (bx * r - cx * q) / d]
}
fn edge(mut e: [usize; 2]) -> [usize; 2] {
    e.sort();
    e
}
fn facekey(mut f: F) -> F {
    f.sort();
    f
}
fn triangles(p: &[P], ids: &[usize]) -> Vec<F> {
    let pts = ids
        .iter()
        .map(|&i| delaunator::Point {
            x: p[i][0],
            y: p[i][1],
        })
        .collect::<Vec<_>>();
    delaunator::triangulate(&pts)
        .triangles
        .chunks_exact(3)
        .map(|t| {
            let mut f = [ids[t[0]], ids[t[1]], ids[t[2]]];
            if cross(p[f[0]], p[f[1]], p[f[2]]) < 0. {
                f.swap(1, 2)
            }
            f
        })
        .collect()
}
fn recover(p: &[P], faces: &mut [F], constraints: &BTreeSet<[usize; 2]>) -> Result<()> {
    let mut edges = BTreeMap::<[usize; 2], Vec<usize>>::new();
    fn insert(map: &mut BTreeMap<[usize; 2], Vec<usize>>, f: F, id: usize) {
        for k in 0..3 {
            map.entry(edge([f[k], f[(k + 1) % 3]]))
                .or_default()
                .push(id)
        }
    }
    for (i, &f) in faces.iter().enumerate() {
        insert(&mut edges, f, i)
    }
    for &target in constraints {
        let [u, v] = target;
        let mut attempts = 0;
        while !edges.contains_key(&target) {
            attempts += 1;
            ensure!(attempts <= faces.len() * 2, "本体の辺を復元できない");
            let candidate = edges.iter().find_map(|(&[a, b], adj)| {
                if adj.len() != 2 || constraints.contains(&[a, b]) {
                    return None;
                }
                if cross(p[u], p[v], p[a]) * cross(p[u], p[v], p[b]) >= -1e-6
                    || cross(p[a], p[b], p[u]) * cross(p[a], p[b], p[v]) >= -1e-6
                {
                    return None;
                }
                let (i, j) = (adj[0], adj[1]);
                let c = *faces[i].iter().find(|&&w| w != a && w != b)?;
                let d = *faces[j].iter().find(|&&w| w != a && w != b)?;
                if cross(p[c], p[d], p[a]) * cross(p[c], p[d], p[b]) >= -1e-6 {
                    return None;
                }
                Some((i, j, c, d, a, b))
            });
            let (i, j, c, d, a, b) =
                candidate.ok_or_else(|| anyhow::anyhow!("制約辺の交換候補がない"))?;
            for id in [i, j] {
                for k in 0..3 {
                    let e = edge([faces[id][k], faces[id][(k + 1) % 3]]);
                    if let Some(v) = edges.get_mut(&e) {
                        v.retain(|&w| w != id);
                        if v.is_empty() {
                            edges.remove(&e);
                        }
                    }
                }
            }
            let mut f = [c, d, a];
            let mut g = [d, c, b];
            if cross(p[f[0]], p[f[1]], p[f[2]]) < 0. {
                f.swap(1, 2)
            }
            if cross(p[g[0]], p[g[1]], p[g[2]]) < 0. {
                g.swap(1, 2)
            }
            faces[i] = f;
            faces[j] = g;
            insert(&mut edges, f, i);
            insert(&mut edges, g, j);
        }
    }
    Ok(())
}
// 保持する本体面を穴として扱う。内部の頂点は分割計算へ入れない。
#[derive(Clone)]
struct CoreTriangle {
    envelope: AABB<[f64; 2]>,
    face: F,
}
impl RTreeObject for CoreTriangle {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}
fn outside_core(p: &[P], f: F, tree: &RTree<CoreTriangle>) -> bool {
    let c = [
        (p[f[0]][0] + p[f[1]][0] + p[f[2]][0]) / 3.,
        (p[f[0]][1] + p[f[1]][1] + p[f[2]][1]) / 3.,
        0.,
    ];
    !tree
        .locate_in_envelope_intersecting(&AABB::from_point([c[0], c[1]]))
        .any(|t| (0..3).all(|k| cross(p[t.face[k]], p[t.face[(k + 1) % 3]], c) >= -1e-6))
}
fn merge(input: &Input) -> Result<serde_json::Value> {
    let start = Instant::now();
    ensure!(
        input.width > 0. && !input.tiles.is_empty(),
        "余白幅またはタイルが不正"
    );
    let (segments, skipped_same) = seams(input);
    if !input.tiles[0].mesh_key.is_empty()
        && input.tiles.iter().all(|t| compatible(&input.tiles[0], t))
    {
        return direct_merge(input, start, skipped_same);
    }
    let distance = |p: P| {
        segments
            .iter()
            .map(|&(a, b)| segment_distance(p, a, b))
            .fold(f64::INFINITY, f64::min)
    };
    let mut p = Vec::<P>::new();
    let mut lookup = HashMap::new();
    let mut allids = Vec::new();
    let mut support_ids = Vec::new();
    let mut loaded_faces = 0;
    for t in &input.tiles {
        let mut ids = Vec::new();
        for &q in &t.points {
            let k = key(q);
            let id = *lookup.entry(k).or_insert_with(|| {
                let id = p.len();
                p.push(q);
                id
            });
            ensure!((p[id][2] - q[2]).abs() < 1e-6, "同じ頂点の高さが異なる");
            ids.push(id)
        }
        allids.push(ids);
        for &q in &t.support {
            let k = key(q);
            let id = *lookup.entry(k).or_insert_with(|| {
                let id = p.len();
                p.push(q);
                id
            });
            support_ids.push(id)
        }
        loaded_faces += t.faces.len();
    }
    let band = RTree::bulk_load(
        p.iter()
            .enumerate()
            .filter(|&(_, q)| distance(*q) <= input.width + 1e-6)
            .map(|(id, q)| C {
                p: [q[0], q[1]],
                id,
            })
            .collect(),
    );
    let mut selected = HashSet::new();
    let mut core = BTreeMap::<F, (usize, usize, F)>::new();
    let mut constraints = BTreeSet::new();
    for (ti, t) in input.tiles.iter().enumerate() {
        for (i, &q) in t.points.iter().enumerate() {
            if input.partial || inside(q, t.bounds) {
                selected.insert(allids[ti][i]);
            }
        }
        for (fi, &f) in t.faces.iter().enumerate() {
            ensure!(f.iter().all(|&v| v < t.points.len()), "面番号が範囲外");
            let f = f.map(|v| allids[ti][v]);
            let inside_own = f.iter().all(|&v| inside(p[v], t.bounds));
            let has_compatible = input
                .tiles
                .iter()
                .enumerate()
                .any(|(j, q)| j != ti && compatible(t, q));
            if f.iter().all(|&v| distance(p[v]) > input.width + 1e-6)
                && (inside_own || (has_compatible && in_compatible_union(&p, f, input, ti)))
            {
                let [a, b, c] = f.map(|v| p[v]);
                let center = circle(a, b, c);
                let radius2 = (center[0] - a[0]).powi(2) + (center[1] - a[1]).powi(2);
                let clear = band
                    .nearest_neighbor(&center)
                    .map(|q| q.distance_2(&center) >= radius2 * (1. - 1e-9))
                    .unwrap_or(true);
                if clear {
                    for k in 0..3 {
                        constraints.insert(edge([f[k], f[(k + 1) % 3]]));
                        selected.insert(f[k]);
                    }
                    core.insert(facekey(f), (ti, fi, f));
                }
            }
        }
    }
    for &(x, y) in &[(0., 0.), (16000., 0.), (16000., 16000.), (0., 16000.)] {
        let k = [(x * 1e6) as i64, (y * 1e6) as i64];
        if let Some(&v) = lookup.get(&k) {
            selected.insert(v);
        } else {
            ensure!(input.partial, "四隅が読み込み範囲にない");
        }
    }
    let (interior_vertices, core_tree) = if input.local {
        let mut incidence = BTreeMap::<[usize; 2], usize>::new();
        let mut core_vertices = HashSet::new();
        for (_, _, f) in core.values() {
            for k in 0..3 {
                *incidence.entry(edge([f[k], f[(k + 1) % 3]])).or_default() += 1;
                core_vertices.insert(f[k]);
            }
        }
        let boundary = incidence
            .iter()
            .filter_map(|(&e, &n)| (n == 1).then_some(e))
            .collect::<BTreeSet<_>>();
        let boundary_vertices = boundary.iter().flatten().copied().collect::<HashSet<_>>();
        let interior_vertices = core_vertices
            .difference(&boundary_vertices)
            .copied()
            .collect::<HashSet<_>>();
        let core_tree = RTree::bulk_load(
            core.values()
                .map(|(_, _, f)| CoreTriangle {
                    face: *f,
                    envelope: AABB::from_corners(
                        [
                            (0..3).map(|k| p[f[k]][0]).fold(f64::INFINITY, f64::min),
                            (0..3).map(|k| p[f[k]][1]).fold(f64::INFINITY, f64::min),
                        ],
                        [
                            (0..3).map(|k| p[f[k]][0]).fold(f64::NEG_INFINITY, f64::max),
                            (0..3).map(|k| p[f[k]][1]).fold(f64::NEG_INFINITY, f64::max),
                        ],
                    ),
                })
                .collect(),
        );
        constraints = boundary;
        (interior_vertices, core_tree)
    } else {
        (HashSet::new(), RTree::new())
    };
    let triangulation_vertices = selected
        .iter()
        .filter(|&&v| !input.local || !interior_vertices.contains(&v))
        .count();
    let base = selected.len();
    let mut rounds = 0;
    let mut exhausted = false;
    let finalfaces = loop {
        let mut ids = selected
            .iter()
            .copied()
            .filter(|v| !input.local || !interior_vertices.contains(v))
            .collect::<Vec<_>>();
        ids.sort();
        let mut fs = triangles(&p, &ids);
        recover(&p, &mut fs, &constraints)?;
        if input.local {
            fs.retain(|&f| outside_core(&p, f, &core_tree));
            fs.extend(core.values().map(|(_, _, f)| *f));
        }
        let present = fs.iter().copied().map(facekey).collect::<HashSet<_>>();
        ensure!(core.keys().all(|f| present.contains(f)), "本体面が変化した");
        let mut bad = fs
            .iter()
            .copied()
            .filter(|f| !core.contains_key(&facekey(*f)) && minangle(&p, *f) < 10. - 1e-6)
            .collect::<Vec<_>>();
        bad.sort_by(|&a, &b| minangle(&p, a).total_cmp(&minangle(&p, b)).then(a.cmp(&b)));
        if bad.is_empty() {
            break fs;
        }
        let mut added = Vec::new();
        for f in bad.iter().take(256) {
            let [a, b, c] = f.map(|v| p[v]);
            let center = circle(a, b, c);
            let r = (center[0] - a[0]).powi(2) + (center[1] - a[1]).powi(2);
            if let Some(q) = band
                .nearest_neighbor_iter(&center)
                .find(|q| !selected.contains(&q.id) && q.distance_2(&center) < r * (1. - 1e-9))
            {
                added.push(q.id)
            }
        }
        added.sort();
        added.dedup();
        if added.is_empty() || rounds >= 128 {
            exhausted = true;
            break fs;
        }
        selected.extend(added);
        rounds += 1;
    };
    let mut ids = finalfaces.iter().flatten().copied().collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    let remap = ids
        .iter()
        .enumerate()
        .map(|(i, &v)| (v, i))
        .collect::<HashMap<_, _>>();
    let faces = finalfaces
        .iter()
        .map(|f| f.map(|v| remap[&v]))
        .collect::<Vec<_>>();
    let roles = finalfaces
        .iter()
        .map(|f| {
            if core.contains_key(&facekey(*f)) {
                let center = f.map(|v| p[v]);
                let q = [
                    (center[0][0] + center[1][0] + center[2][0]) / 3.,
                    (center[0][1] + center[1][1] + center[2][1]) / 3.,
                    0.,
                ];
                usize::from(
                    input
                        .tiles
                        .iter()
                        .position(|t| inside(q, t.bounds))
                        .unwrap_or(0)
                        != 0,
                )
            } else {
                2
            }
        })
        .collect::<Vec<_>>();
    let minimum = finalfaces
        .iter()
        .map(|&f| minangle(&p, f))
        .fold(180_f64, f64::min);
    let sharp = finalfaces
        .iter()
        .filter(|&&f| minangle(&p, f) < 10. - 1e-6)
        .count();
    let preserved = core
        .values()
        .map(|(t, f, _)| json!({"tile":t,"face":f}))
        .collect::<Vec<_>>();
    let runtime = start.elapsed().as_secs_f64() * 1000.;
    Ok(
        json!({"points":ids.iter().map(|&v|p[v]).collect::<Vec<_>>(),"faces":faces,"roles":roles,"patches":[],"preservedCore":preserved,"stats":{"vertices":ids.len(),"triangles":finalfaces.len(),"patchTriangles":finalfaces.len()-core.len(),"extraVertices":selected.len()-base,"minimumAngle":minimum,"sharpTriangles":sharp,"coreFaces":core.len(),"retainedCoreFaces":core.len(),"loadedVertices":p.len(),"loadedTriangles":loaded_faces,"qualityAdditions":selected.len()-base,"qualitySatisfied":!exhausted&&sharp==0,"runtimeMs":runtime,"refinementRounds":rounds,"local":input.local,"sameMeshFastPath":false,"sameLevelSkippedSegments":skipped_same,"triangulationVertices":triangulation_vertices+selected.len()-base,"excludedCoreVertices":if input.local{interior_vertices.len()}else{0}},"loadedPoolIds":ids}),
    )
}
// 事前生成専用。追加点の影響外の面を保持し、同レベル全体で共通の形状を作る。
fn bake(tile: &Tile, pool: &[P]) -> Result<serde_json::Value> {
    let mut p = tile.points.clone();
    let mut seen = p.iter().copied().map(key).collect::<HashSet<_>>();
    let mut added = Vec::new();
    for &q in &tile.support {
        if seen.insert(key(q)) {
            added.push(q);
            p.push(q);
        }
    }
    if added.is_empty() {
        return Ok(json!({"points":p,"faces":tile.faces}));
    }
    let mut retained = Vec::new();
    let candidates = RTree::bulk_load(
        pool.iter()
            .enumerate()
            .map(|(id, q)| C {
                p: [q[0], q[1]],
                id,
            })
            .collect(),
    );
    let mut rounds = 0;
    let fs = loop {
        let tree = RTree::bulk_load(
            p[tile.points.len()..]
                .iter()
                .enumerate()
                .map(|(id, q)| C {
                    p: [q[0], q[1]],
                    id,
                })
                .collect(),
        );
        let mut constraints = BTreeSet::new();
        retained.clear();
        for &f in &tile.faces {
            let [a, b, c] = f.map(|v| p[v]);
            let center = circle(a, b, c);
            let r = (center[0] - a[0]).powi(2) + (center[1] - a[1]).powi(2);
            if tree
                .nearest_neighbor(&center)
                .map(|q| q.distance_2(&center) >= r * (1. - 1e-9))
                .unwrap_or(true)
            {
                retained.push(facekey(f));
                for k in 0..3 {
                    constraints.insert(edge([f[k], f[(k + 1) % 3]]));
                }
            }
        }

        let mut fs = triangles(&p, &(0..p.len()).collect::<Vec<_>>());
        recover(&p, &mut fs, &constraints)?;
        let mut selected = BTreeSet::new();
        for &f in &fs {
            if minangle(&p, f) >= 10. - 1e-6 {
                continue;
            }
            let [a, b, c] = f.map(|i| p[i]);
            let center = circle(a, b, c);
            let r = (center[0] - a[0]).powi(2) + (center[1] - a[1]).powi(2);
            if let Some(q) = candidates.nearest_neighbor_iter(&center).find(|q| {
                !seen.contains(&key(pool[q.id])) && q.distance_2(&center) < r * (1. - 1e-9)
            }) {
                selected.insert(q.id);
            }
        }
        if selected.is_empty() || rounds >= 128 {
            break fs;
        }
        for i in selected {
            let q = pool[i];
            if seen.insert(key(q)) {
                p.push(q);
            }
        }
        rounds += 1;
    };
    let present = fs.iter().copied().map(facekey).collect::<HashSet<_>>();
    ensure!(
        retained.iter().all(|f| present.contains(f)),
        "事前補修で影響外の面が変化した"
    );
    Ok(json!({"points":p,"faces":fs}))
}
fn main() -> Result<()> {
    let mut s = String::new();
    io::stdin().read_to_string(&mut s)?;
    let value: serde_json::Value = serde_json::from_str(&s)?;
    let result = if value.get("bake").and_then(|v| v.as_bool()).unwrap_or(false) {
        bake(
            &serde_json::from_value::<Tile>(value["tile"].clone())?,
            &serde_json::from_value::<Vec<P>>(value["pool"].clone())?,
        )?
    } else {
        merge(&serde_json::from_value::<Input>(value)?)?
    };
    serde_json::to_writer(io::stdout(), &result)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cores_are_retained_and_merge_is_deterministic() {
        let points = (0..81)
            .map(|i| [(i % 9) as f64 * 2000., (i / 9) as f64 * 2000., i as f64])
            .collect::<Vec<_>>();
        let mut faces = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                let a = y * 9 + x;
                faces.push([a, a + 1, a + 10]);
                faces.push([a, a + 10, a + 9]);
            }
        }
        let a = Tile {
            bounds: [0., 0., 8000., 16000.],
            points: points.clone(),
            faces: faces.clone(),
            support: vec![],
            mesh_key: String::new(),
        };
        let mut b = a.clone();
        b.bounds = [8000., 0., 16000., 16000.];
        let input = Input {
            tiles: vec![a, b],
            width: 500.,
            local: true,
            partial: false,
        };
        let first = merge(&input).unwrap();
        let second = merge(&input).unwrap();
        assert!(first["stats"]["excludedCoreVertices"].as_u64().unwrap() > 0);
        assert!(
            first["stats"]["triangulationVertices"].as_u64().unwrap()
                < first["stats"]["vertices"].as_u64().unwrap()
        );
        assert_eq!(first["faces"], second["faces"]);
        assert_eq!(first["points"], second["points"]);
        assert!(first["stats"]["coreFaces"].as_u64().unwrap() > 0);
        assert_eq!(first["stats"]["qualitySatisfied"], true);
        assert!(
            first["points"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| points.iter().any(|q| serde_json::json!(q) == *p))
        );
    }
    #[test]
    fn same_mesh_skips_seam_and_triangulation() {
        let a = Tile {
            bounds: [0., 0., 8000., 16000.],
            points: vec![
                [0., 0., 0.],
                [16000., 0., 0.],
                [16000., 16000., 0.],
                [0., 16000., 0.],
            ],
            faces: vec![[0, 1, 2], [0, 2, 3]],
            support: vec![],
            mesh_key: "source-50".into(),
        };
        let mut b = a.clone();
        b.bounds = [8000., 0., 16000., 16000.];
        let mut input = Input {
            tiles: vec![a, b],
            width: 1000.,
            local: true,
            partial: false,
        };
        let (segments, skipped) = seams(&input);
        assert!(segments.is_empty());
        assert_eq!(skipped, 1);
        let result = merge(&input).unwrap();
        assert_eq!(result["stats"]["sameMeshFastPath"], true);
        assert_eq!(result["stats"]["triangulationVertices"], 0);
        assert_eq!(result["faces"].as_array().unwrap().len(), 2);
        input.tiles[1].mesh_key = "another-source-50".into();
        let (segments, skipped) = seams(&input);
        assert!(!segments.is_empty());
        assert_eq!(skipped, 0);
    }
    #[test]
    fn prebaked_points_are_part_of_faces() {
        let tile = Tile {
            bounds: [0., 0., 16000., 16000.],
            points: vec![
                [0., 0., 0.],
                [16000., 0., 0.],
                [16000., 16000., 0.],
                [0., 16000., 0.],
            ],
            faces: vec![[0, 1, 2], [0, 2, 3]],
            support: vec![[8000., 8000., 50.]],
            mesh_key: String::new(),
        };
        let result = bake(&tile, &[]).unwrap();
        let points: Vec<P> = serde_json::from_value(result["points"].clone()).unwrap();
        let faces: Vec<F> = serde_json::from_value(result["faces"].clone()).unwrap();
        assert_eq!(points.len(), 5);
        assert_eq!(faces.len(), 4);
        assert!(faces.iter().all(|&f| minangle(&points, f) >= 10.));
        assert_eq!(points[4], tile.support[0]);
    }
    #[test]
    fn segment_endpoints_are_local() {
        assert_eq!(
            segment_distance([10., 20., 0.], [0., 0.], [0., 10.]),
            (200_f64).sqrt()
        );
    }
    #[test]
    fn constrained_diagonal_is_recovered() {
        let p = vec![[0., 0., 0.], [10., 0., 0.], [10., 10., 0.], [0., 10., 0.]];
        let mut f = vec![[0, 1, 2], [0, 2, 3]];
        recover(&p, &mut f, &BTreeSet::from([[1, 3]])).unwrap();
        assert!(f.iter().all(|f| f.contains(&1) && f.contains(&3)));
    }
}
