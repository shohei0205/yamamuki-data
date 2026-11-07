//! Shared cubic Bezier approximation of categorical coverage boundaries.
mod geos;
#[path = "landcover_xyz.rs"]
mod landcover_xyz;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use flate2::{
    Compression,
    read::ZlibDecoder,
    write::{GzEncoder, ZlibEncoder},
};
use geos::Geos;
use rstar::{AABB, PointDistance, RTree, RTreeObject};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    path::PathBuf,
    time::Instant,
};
type P = [f64; 2];
fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn mul(a: P, t: f64) -> P {
    [a[0] * t, a[1] * t]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn norm(a: P) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: P) -> P {
    let n = norm(a);
    if n > 1e-12 { mul(a, 1. / n) } else { [1., 0.] }
}
fn dist_seg(p: P, a: P, b: P) -> f64 {
    let d = sub(b, a);
    let t = if dot(d, d) > 0. {
        (dot(sub(p, a), d) / dot(d, d)).clamp(0., 1.)
    } else {
        0.
    };
    norm(sub(p, add(a, mul(d, t))))
}
#[derive(Clone, Debug)]
struct Curve {
    p: [P; 4],
    line: bool,
}
impl Curve {
    fn line(a: P, b: P) -> Self {
        Self {
            p: [
                a,
                add(a, mul(sub(b, a), 1. / 3.)),
                add(a, mul(sub(b, a), 2. / 3.)),
                b,
            ],
            line: true,
        }
    }
    fn at(&self, t: f64) -> P {
        let s = 1. - t;
        add(
            add(mul(self.p[0], s * s * s), mul(self.p[1], 3. * s * s * t)),
            add(mul(self.p[2], 3. * s * t * t), mul(self.p[3], t * t * t)),
        )
    }
}
#[derive(Clone)]
struct Edge {
    a: usize,
    b: usize,
    classes: u8,
    chain: usize,
}
#[derive(Clone)]
struct IndexedEdge {
    a: P,
    b: P,
    chain: usize,
}
impl RTreeObject for IndexedEdge {
    type Envelope = AABB<P>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(
            [self.a[0].min(self.b[0]), self.a[1].min(self.b[1])],
            [self.a[0].max(self.b[0]), self.a[1].max(self.b[1])],
        )
    }
}
impl PointDistance for IndexedEdge {
    fn distance_2(&self, p: &P) -> f64 {
        dist_seg(*p, self.a, self.b).powi(2)
    }
}
#[derive(Clone)]
struct Node(P, usize);
impl RTreeObject for Node {
    type Envelope = AABB<P>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.0)
    }
}
#[derive(Clone)]
struct Polygon {
    code: u8,
    rings: Vec<Vec<usize>>,
}
#[derive(Clone)]
struct Chain {
    nodes: Vec<usize>,
    frozen: bool,
}
struct Graph {
    points: Vec<P>,
    edges: Vec<Edge>,
    chains: Vec<Chain>,
    polys: Vec<Polygon>,
    edge_ids: HashMap<(usize, usize), usize>,
    tree: RTree<IndexedEdge>,
}
fn edge_key(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}
fn key(p: P) -> (u64, u64) {
    (
        if p[0] == 0. { 0 } else { p[0].to_bits() },
        if p[1] == 0. { 0 } else { p[1].to_bits() },
    )
}
#[cfg(test)]
fn build(input: Vec<(u8, Vec<Vec<P>>)>, compact: bool, split_water: bool) -> Result<Graph> {
    build_graph(input, compact, split_water, false)
}

fn build_graph(
    input: Vec<(u8, Vec<Vec<P>>)>, compact: bool, split_water: bool, independent_water: bool,
) -> Result<Graph> {
    let mut water_points = Vec::new();
    let mut points = Vec::new();
    let mut lookup = HashMap::new();
    let mut polys = Vec::new();
    for (code, rings) in input {
        let mut rr = Vec::new();
        for ring in rings {
            let mut ids = Vec::new();
            for p in ring {
                let id = *lookup.entry((key(p), independent_water && code >= 3)).or_insert_with(|| {
                    water_points.push(independent_water && code >= 3);
                    points.push(p);
                    points.len() - 1
                });
                if ids.last() != Some(&id) {
                    ids.push(id)
                }
            }
            if ids.first() == ids.last() {
                ids.pop();
            }
            ensure!(ids.len() >= 3, "Degenerate source ring");
            rr.push(ids);
        }
        polys.push(Polygon { code, rings: rr });
    }
    // Node collinear T-junctions before assigning shared chain references.
    let nodes = RTree::bulk_load(
        points
            .iter()
            .enumerate()
            .map(|(i, &p)| Node(p, i))
            .collect(),
    );
    for poly in &mut polys {
        for ring in &mut poly.rings {
            let old = ring.clone();
            let mut out = Vec::new();
            for i in 0..old.len() {
                let a = points[old[i]];
                let b = points[old[(i + 1) % old.len()]];
                let d = sub(b, a);
                let dd = dot(d, d);
                let env = AABB::from_corners(
                    [a[0].min(b[0]) - 1e-8, a[1].min(b[1]) - 1e-8],
                    [a[0].max(b[0]) + 1e-8, a[1].max(b[1]) + 1e-8],
                );
                let mut split = vec![(0., old[i])];
                for node in nodes.locate_in_envelope_intersecting(&env) {
                    if water_points[node.1] != water_points[old[i]] {
                        continue;
                    }
                    let t = dot(sub(node.0, a), d) / dd;
                    if t > 1e-12
                        && t < 1. - 1e-12
                        && robust::orient2d(
                            robust::Coord { x: a[0], y: a[1] },
                            robust::Coord { x: b[0], y: b[1] },
                            robust::Coord {
                                x: node.0[0],
                                y: node.0[1],
                            },
                        ) == 0.
                    {
                        split.push((t, node.1))
                    }
                }
                split.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                for (_, id) in split {
                    if out.last() != Some(&id) {
                        out.push(id)
                    }
                }
            }
            *ring = out;
        }
    }
    let mut edges = Vec::<Edge>::new();
    let mut edge_ids = HashMap::new();
    for poly in &polys {
        for ring in &poly.rings {
            for i in 0..ring.len() {
                let a = ring[i];
                let b = ring[(i + 1) % ring.len()];
                if a == b {
                    continue;
                }
                let id = *edge_ids.entry(edge_key(a, b)).or_insert_with(|| {
                    edges.push(Edge {
                        a,
                        b,
                        classes: 0,
                        chain: usize::MAX,
                    });
                    edges.len() - 1
                });
                edges[id].classes |= 1 << poly.code;
            }
        }
    }
    let mut adjacent = vec![Vec::new(); points.len()];
    for (i, e) in edges.iter().enumerate() {
        adjacent[e.a].push(i);
        adjacent[e.b].push(i);
    }
    let mut terminal = vec![false; points.len()];
    for (i, a) in adjacent.iter().enumerate() {
        terminal[i] = a.len() != 2 || (a.len() == 2 && edges[a[0]].classes != edges[a[1]].classes);
    }
    // Break each closed degree-two loop into four balanced source paths.
    let mut visited = vec![false; edges.len()];
    for start in 0..edges.len() {
        if visited[start] {
            continue;
        }
        let mut ei = start;
        let mut at = edges[ei].a;
        let mut ids = Vec::new();
        let mut closed = true;
        loop {
            if visited[ei] {
                closed = ei == start && ids.first() == Some(&at);
                break;
            }
            visited[ei] = true;
            ids.push(at);
            let e = &edges[ei];
            let next = if e.a == at { e.b } else { e.a };
            if terminal[at] || terminal[next] {
                closed = false;
                break;
            }
            let ne = *adjacent[next].iter().find(|&&j| j != ei).unwrap();
            at = next;
            ei = ne;
        }
        if closed && ids.len() >= 3 {
            let pieces = if compact { 1 } else { 4 };
            for q in 0..pieces {
                terminal[ids[q * ids.len() / pieces]] = true
            }
        }
    }
    if split_water {
        let mut seen = vec![false; edges.len()];
        let starts: Vec<_> = terminal.iter().enumerate()
            .filter_map(|(i, &is_end)| is_end.then_some(i)).collect();
        for start in starts {
            for &initial in &adjacent[start] {
                if seen[initial] || edges[initial].classes & 8 == 0
                    || (edges[initial].classes & 7).count_ones() != 1
                {
                    continue;
                }
                let (mut at, mut edge, mut distance) = (start, initial, 0.);
                loop {
                    if seen[edge] { break; }
                    seen[edge] = true;
                    let e = &edges[edge];
                    let next = if e.a == at { e.b } else { e.a };
                    distance += norm(sub(points[at], points[next]));
                    if terminal[next] { break; }
                    if distance >= 500. {
                        terminal[next] = true;
                        distance = 0.;
                    }
                    edge = *adjacent[next].iter().find(|&&id| id != edge).unwrap();
                    at = next;
                }
            }
        }
    }
    let mut chains = Vec::new();
    // Start at every terminal, so no ring ever begins in the middle of a chain.
    for a in 0..points.len() {
        if !terminal[a] {
            continue;
        }
        for &initial in &adjacent[a] {
            if edges[initial].chain != usize::MAX {
                continue;
            }
            let id = chains.len();
            let mut ids = vec![a];
            let mut at = a;
            let mut ei = initial;
            let classes = edges[ei].classes;
            loop {
                ensure!(edges[ei].chain == usize::MAX, "Repeated edge");
                edges[ei].chain = id;
                let e = &edges[ei];
                let next = if e.a == at { e.b } else { e.a };
                ids.push(next);
                if terminal[next] {
                    break;
                }
                let ne = *adjacent[next].iter().find(|&&j| j != ei).unwrap();
                at = next;
                ei = ne;
            }
            // Preserve all coastline/inland-water and domain perimeter paths exactly.
            let frozen = classes & (1 << 3 | 1 << 4) != 0 || classes.count_ones() != 2;
            chains.push(Chain { nodes: ids, frozen });
        }
    }
    ensure!(
        edges.iter().all(|e| e.chain != usize::MAX),
        "Unassigned closed chain"
    );
    let tree = RTree::bulk_load(
        edges
            .iter()
            .map(|e| IndexedEdge {
                a: points[e.a],
                b: points[e.b],
                chain: e.chain,
            })
            .collect(),
    );
    Ok(Graph {
        points,
        edges,
        chains,
        polys,
        edge_ids,
        tree,
    })
}
fn fit_once(points: &[P], t: &[f64], left: P, right: P) -> Curve {
    let a = points[0];
    let b = *points.last().unwrap();
    let mut c00 = 0.;
    let mut c01 = 0.;
    let mut c11 = 0.;
    let mut x0 = 0.;
    let mut x1 = 0.;
    for (&p, &u) in points.iter().zip(t) {
        let s = 1. - u;
        let b0 = s * s * s;
        let b1 = 3. * s * s * u;
        let b2 = 3. * s * u * u;
        let b3 = u * u * u;
        let aa = mul(left, b1);
        let bb = mul(right, b2);
        let residual = sub(p, add(mul(a, b0 + b1), mul(b, b2 + b3)));
        c00 += dot(aa, aa);
        c01 += dot(aa, bb);
        c11 += dot(bb, bb);
        x0 += dot(aa, residual);
        x1 += dot(bb, residual);
    }
    let det = c00 * c11 - c01 * c01;
    let mut alpha = if det.abs() > 1e-16 {
        (x0 * c11 - x1 * c01) / det
    } else {
        -1.
    };
    let mut beta = if det.abs() > 1e-16 {
        (c00 * x1 - c01 * x0) / det
    } else {
        -1.
    };
    let len = points
        .windows(2)
        .map(|w| norm(sub(w[1], w[0])))
        .sum::<f64>();
    if alpha < 1e-6 || beta < 1e-6 || alpha > len || beta > len {
        alpha = norm(sub(b, a)) / 3.;
        beta = alpha
    }
    Curve {
        p: [a, add(a, mul(left, alpha)), add(b, mul(right, beta)), b],
        line: false,
    }
}
fn flatten(curve: &Curve, tolerance: f64, out: &mut Vec<P>, depth: usize) {
    let [a, b, c, d] = curve.p;
    if curve.line || depth >= 16 || dist_seg(b, a, d).max(dist_seg(c, a, d)) <= tolerance {
        out.push(d);
        return;
    }
    let ab = mul(add(a, b), 0.5);
    let bc = mul(add(b, c), 0.5);
    let cd = mul(add(c, d), 0.5);
    let abc = mul(add(ab, bc), 0.5);
    let bcd = mul(add(bc, cd), 0.5);
    let mid = mul(add(abc, bcd), 0.5);
    flatten(
        &Curve {
            p: [a, ab, abc, mid],
            line: false,
        },
        tolerance,
        out,
        depth + 1,
    );
    flatten(
        &Curve {
            p: [mid, bcd, cd, d],
            line: false,
        },
        tolerance,
        out,
        depth + 1,
    );
}
fn fit_path(
    points: &[P],
    left: P,
    right: P,
    tolerance: f64,
    chain: usize,
    tree: Option<&RTree<IndexedEdge>>,
    out: &mut Vec<Curve>,
    max_error: &mut f64,
) {
    if points.len() == 2 {
        out.push(Curve::line(points[0], points[1]));
        return;
    }
    let mut t = vec![0.];
    for w in points.windows(2) {
        t.push(t.last().unwrap() + norm(sub(w[1], w[0])))
    }
    let total = *t.last().unwrap();
    for u in &mut t {
        *u /= total;
    }
    let curve = fit_once(points, &t, left, right);
    let (mut error, mut split) = (0., points.len() / 2);
    for (i, (&p, &u)) in points
        .iter()
        .zip(&t)
        .enumerate()
        .skip(1)
        .take(points.len() - 2)
    {
        let e = norm(sub(curve.at(u), p));
        if e > error {
            error = e;
            split = i
        }
    }
    let mut safe = error <= tolerance;
    // Test both fit directions and a clearance tube around the original path.
    // Deformation <= 35% of the distance to another shared boundary.
    if safe {
        let count = ((total / 8.).ceil() as usize).clamp(8, 4096);
        let mut j = 0;
        for i in 0..=count {
            let u = i as f64 / count as f64;
            while j + 1 < t.len() - 1 && t[j + 1] < u {
                j += 1
            }
            let v = (u - t[j]) / (t[j + 1] - t[j]);
            let original = add(points[j], mul(sub(points[j + 1], points[j]), v));
            let q = curve.at(u);
            let delta = norm(sub(q, original));
            if delta > tolerance {
                safe = false;
                break;
            }
            if let Some(tree) = tree {
                let nearest = tree
                    .nearest_neighbor_iter(&original)
                    .find(|e| e.chain != chain);
                if let Some(e) = nearest {
                    if delta > 0.35 * dist_seg(original, e.a, e.b) + 1e-7 {
                        safe = false;
                        break;
                    }
                }
            }
        }
    }
    if safe {
        *max_error = max_error.max(error);
        out.push(curve);
        return;
    }
    split = split.clamp(1, points.len() - 2);
    let before = split.saturating_sub(2);
    let after = (split + 2).min(points.len() - 1);
    let tangent = unit(sub(points[before], points[after]));
    fit_path(
        &points[..=split],
        left,
        tangent,
        tolerance,
        chain,
        tree,
        out,
        max_error,
    );
    fit_path(
        &points[split..],
        mul(tangent, -1.),
        right,
        tolerance,
        chain,
        tree,
        out,
        max_error,
    );
}
fn project_segment(p: P, a: P, b: P) -> P {
    let d = sub(b, a);
    let dd = dot(d, d);
    let t = if dd > 0. {
        (dot(sub(p, a), d) / dd).clamp(0., 1.)
    } else {
        0.
    };
    add(a, mul(d, t))
}
fn closest_parameter(c: &Curve, p: P, initial: f64) -> f64 {
    let mut u = initial;
    for _ in 0..8 {
        let s = 1. - u;
        let first = add(
            add(
                mul(sub(c.p[1], c.p[0]), 3. * s * s),
                mul(sub(c.p[2], c.p[1]), 6. * s * u),
            ),
            mul(sub(c.p[3], c.p[2]), 3. * u * u),
        );
        let second = add(
            mul(add(sub(c.p[2], mul(c.p[1], 2.)), c.p[0]), 6. * s),
            mul(add(sub(c.p[3], mul(c.p[2], 2.)), c.p[1]), 6. * u),
        );
        let delta = sub(c.at(u), p);
        let denominator = dot(first, first) + dot(delta, second);
        if denominator.abs() < 1e-12 {
            break;
        }
        let next = (u - dot(delta, first) / denominator).clamp(0., 1.);
        if norm(sub(c.at(next), p)) > norm(delta) + 1e-9 {
            break;
        }
        if (next - u).abs() < 1e-10 {
            u = next;
            break;
        }
        u = next;
    }
    u
}
fn fit_path_geometric(
    points: &[P],
    left: P,
    right: P,
    tolerance: f64,
    chain: usize,
    tree: Option<&RTree<IndexedEdge>>,
    out: &mut Vec<Curve>,
    max_error: &mut f64,
) {
    let mut t = vec![0.];
    for w in points.windows(2) {
        t.push(t.last().unwrap() + norm(sub(w[1], w[0])))
    }
    let total = *t.last().unwrap();
    for u in &mut t {
        *u /= total;
    }
    let mut curve = fit_once(points, &t, left, right);
    // Reparameterize along the curve: do not mistake tangential parameter
    // displacement for a displacement of the actual categorical boundary.
    for _ in 0..5 {
        let next: Vec<_> = points
            .iter()
            .zip(&t)
            .enumerate()
            .map(|(i, (&p, &u))| {
                if i == 0 {
                    0.
                } else if i + 1 == points.len() {
                    1.
                } else {
                    closest_parameter(&curve, p, u)
                }
            })
            .collect();
        if !next.windows(2).all(|w| w[0] < w[1]) {
            break;
        }
        let candidate = fit_once(points, &next, left, right);
        let before = points
            .iter()
            .zip(&t)
            .map(|(&p, &u)| norm(sub(curve.at(u), p)))
            .fold(0., f64::max);
        let after = points
            .iter()
            .zip(&next)
            .map(|(&p, &u)| norm(sub(candidate.at(u), p)))
            .fold(0., f64::max);
        if after > before + 1e-9 {
            break;
        }
        curve = candidate;
        t = next;
    }
    let (mut error, mut split) = (0., points.len() / 2);
    for (i, (&p, &u)) in points
        .iter()
        .zip(&t)
        .enumerate()
        .skip(1)
        .take(points.len() - 2)
    {
        let e = norm(sub(curve.at(u), p));
        if e > error {
            error = e;
            split = i
        }
    }
    // A closed path must be split before approximation; otherwise a loose
    // tolerance can collapse its entire ring into the shared endpoint.
    let mut safe =
        error <= tolerance && norm(sub(*points.last().unwrap(), points[0])) > total * 1e-8;
    // Test both fit directions and a clearance tube around the original path.
    // Deformation <= 35% of the distance to another shared boundary.
    if safe {
        let count = ((total / 8.).ceil() as usize).clamp(8, 4096);
        let source_tree = RTree::bulk_load(
            points
                .windows(2)
                .map(|w| IndexedEdge {
                    a: w[0],
                    b: w[1],
                    chain,
                })
                .collect(),
        );
        for i in 0..=count {
            let u = i as f64 / count as f64;
            let q = curve.at(u);
            let nearest = source_tree.nearest_neighbor(&q).unwrap();
            let original = project_segment(q, nearest.a, nearest.b);
            let delta = norm(sub(q, original));
            if delta > tolerance {
                safe = false;
                break;
            }
            if let Some(tree) = tree {
                let nearest = tree
                    .nearest_neighbor_iter(&original)
                    .find(|e| e.chain != chain);
                if let Some(e) = nearest {
                    if delta > 0.35 * dist_seg(original, e.a, e.b) + 1e-7 {
                        safe = false;
                        break;
                    }
                }
            }
        }
    }
    if safe {
        *max_error = max_error.max(error);
        out.push(curve);
        return;
    }
    if points.len() == 2 {
        out.push(Curve::line(points[0], points[1]));
        return;
    }
    split = split.clamp(1, points.len() - 2);
    let before = split.saturating_sub(2);
    let after = (split + 2).min(points.len() - 1);
    let tangent = unit(sub(points[before], points[after]));
    fit_path_geometric(
        &points[..=split],
        left,
        tangent,
        tolerance,
        chain,
        tree,
        out,
        max_error,
    );
    fit_path_geometric(
        &points[split..],
        mul(tangent, -1.),
        right,
        tolerance,
        chain,
        tree,
        out,
        max_error,
    );
}
fn join_collinear_lines(curves: Vec<Curve>) -> Vec<Curve> {
    let mut joined: Vec<Curve> = Vec::new();
    for c in curves {
        if let Some(prev) = joined.last_mut() {
            if prev.line && c.line && prev.p[0] != c.p[3]
                && dist_seg(prev.p[3], prev.p[0], c.p[3]) < 1e-7
            {
                *prev = Curve::line(prev.p[0], c.p[3]);
                continue;
            }
        }
        joined.push(c);
    }
    joined
}

fn simplify_coast(points: &[P], tolerance: f64, chain: usize, tree: &RTree<IndexedEdge>, subpixel: bool) -> Vec<Curve> {
    let limits: Vec<_> = points.iter().map(|p| {
        let clearance = tree.nearest_neighbor_iter(p).find(|e| e.chain != chain)
            .map_or(f64::INFINITY, |e| dist_seg(*p, e.a, e.b));
        tolerance.min((0.35 * clearance).max(if subpixel { 31.25 } else { 1e-8 }))
    }).collect();
    let mut result = Vec::new();
    let mut pending = vec![(0, points.len()-1)];
    while let Some((a,b)) = pending.pop() {
        let mut split = (1., None);
        for i in a+1..b {
            let ratio = dist_seg(points[i], points[a], points[b]) / limits[i];
            if ratio > split.0 {
                split = (ratio, Some(i));
            }
        }
        // A whole closed coast needs at least three distinct vertices.
        if b > a+1 && points[a] == points[b] && split.1.is_none() {
            split.1 = Some((a+b)/2);
        }
        if let Some(i) = split.1 {
            pending.push((i,b));
            pending.push((a,i));
        } else {
            result.push(Curve::line(points[a], points[b]));
        }
    }
    result
}

fn fair_path(
    points: &[P],
    tolerance: f64,
    chain: usize,
    tree: Option<&RTree<IndexedEdge>>,
) -> Vec<P> {
    let mut result = points.to_vec();
    if points.len() < 3 {
        return result;
    }
    let limits: Vec<_> = points
        .iter()
        .map(|&p| {
            let distance = tree
                .and_then(|t| t.nearest_neighbor_iter(&p).find(|e| e.chain != chain))
                .map(|e| dist_seg(p, e.a, e.b))
                .unwrap_or(f64::INFINITY);
            (tolerance * 0.5).min(distance * 0.3)
        })
        .collect();
    for _ in 0..12 {
        let old = result.clone();
        for i in 1..old.len() - 1 {
            let left = norm(sub(old[i], old[i - 1]));
            let right = norm(sub(old[i + 1], old[i]));
            if left + right < 1e-10 {
                continue;
            }
            let target = mul(
                add(mul(old[i - 1], right), mul(old[i + 1], left)),
                1. / (left + right),
            );
            let candidate = mul(add(old[i], target), 0.5);
            let delta = sub(candidate, points[i]);
            let length = norm(delta);
            result[i] = if length > limits[i] {
                add(points[i], mul(delta, limits[i] / length))
            } else {
                candidate
            };
        }
    }
    result
}
fn fit_path_faired(
    points: &[P],
    _left: P,
    _right: P,
    tolerance: f64,
    chain: usize,
    tree: Option<&RTree<IndexedEdge>>,
    out: &mut Vec<Curve>,
    max_error: &mut f64,
) {
    let fair = fair_path(points, tolerance, chain, tree);
    let displacement = points
        .iter()
        .zip(&fair)
        .map(|(&a, &b)| norm(sub(a, b)))
        .fold(0., f64::max);
    let left = unit(sub(fair[2.min(fair.len() - 1)], fair[0]));
    let right = unit(sub(
        fair[fair.len().saturating_sub(3)],
        *fair.last().unwrap(),
    ));
    let mut error = 0.;
    fit_path_geometric(
        &fair,
        left,
        right,
        tolerance * 0.5,
        chain,
        tree,
        out,
        &mut error,
    );
    *max_error = max_error.max(displacement + error);
}

fn references(g: &Graph, ring: &[usize]) -> Result<Vec<i32>> {
    let ids: Vec<_> = (0..ring.len())
        .map(|i| g.edges[g.edge_ids[&edge_key(ring[i], ring[(i + 1) % ring.len()])]].chain)
        .collect();
    let start = (0..ring.len())
        .find(|&i| ids[i] != ids[(i + ring.len() - 1) % ring.len()])
        .or_else(|| (0..ring.len()).find(|&i| g.chains[ids[i]].nodes[0] == ring[i]))
        .context("Ring has no chain endpoint")?;
    let mut out = Vec::new();
    let mut prev = usize::MAX;
    for j in 0..ring.len() {
        let i = (start + j) % ring.len();
        let id = ids[i];
        if id == prev {
            continue;
        }
        let forward =
            g.chains[id].nodes[0] == ring[i] && g.chains[id].nodes[1] == ring[(i + 1) % ring.len()];
        ensure!(
            forward || *g.chains[id].nodes.last().unwrap() == ring[i],
            "Ring starts inside shared chain"
        );
        out.push(if forward {
            id as i32 + 1
        } else {
            -(id as i32 + 1)
        });
        prev = id;
    }
    Ok(out)
}
fn rings_from(refs: &[Vec<i32>], samples: &[Vec<P>]) -> Vec<Vec<P>> {
    refs.iter()
        .map(|rr| {
            let mut ring = Vec::new();
            for &r in rr {
                let s = &samples[(r.unsigned_abs() - 1) as usize];
                if r > 0 {
                    ring.extend(s.iter().copied().take(s.len() - 1))
                } else {
                    ring.extend(s.iter().rev().copied().take(s.len() - 1))
                }
            }
            ring.push(ring[0]);
            ring
        })
        .collect()
}
fn wkb_poly(rings: &[Vec<P>]) -> Vec<u8> {
    let mut b = vec![1];
    b.extend(3u32.to_le_bytes());
    b.extend((rings.len() as u32).to_le_bytes());
    for r in rings {
        b.extend((r.len() as u32).to_le_bytes());
        for p in r {
            b.extend(p[0].to_le_bytes());
            b.extend(p[1].to_le_bytes());
        }
    }
    b
}
fn wkb_multi(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![1];
    b.extend(6u32.to_le_bytes());
    b.extend((parts.len() as u32).to_le_bytes());
    for p in parts {
        b.extend(p)
    }
    b
}
fn zip(b: &[u8], gzip: bool) -> Result<Vec<u8>> {
    if gzip {
        let mut z = GzEncoder::new(Vec::new(), Compression::best());
        z.write_all(b)?;
        Ok(z.finish()?)
    } else {
        let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
        z.write_all(b)?;
        Ok(z.finish()?)
    }
}
fn encode(curves: &[Vec<Curve>], polys: &[Polygon], refs: &[Vec<Vec<i32>>]) -> Vec<u8> {
    let mut b = b"BZF1".to_vec();
    b.extend((curves.len() as u32).to_le_bytes());
    b.extend((polys.len() as u32).to_le_bytes());
    for chain in curves {
        b.extend((chain.len() as u32).to_le_bytes());
        for v in chain[0].p[0] {
            b.extend(v.to_le_bytes())
        }
        for c in chain {
            b.push(if c.line { 0 } else { 1 });
            for p in if c.line { &c.p[3..] } else { &c.p[1..] } {
                for v in p {
                    b.extend(v.to_le_bytes())
                }
            }
        }
    }
    for (p, rr) in polys.iter().zip(refs) {
        b.push(p.code);
        b.extend((rr.len() as u32).to_le_bytes());
        for r in rr {
            b.extend((r.len() as u32).to_le_bytes());
            for v in r {
                b.extend(v.to_le_bytes())
            }
        }
    }
    b
}
fn put_varint(bytes: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        bytes.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn put_point_delta(bytes: &mut Vec<u8>, point: P, previous: P) {
    // XOR on the exact IEEE coordinates. Store the window of changed bytes.
    for axis in 0..2 {
        let delta = point[axis].to_bits() ^ previous[axis].to_bits();
        if delta == 0 {
            bytes.push(0);
            continue;
        }
        let low = delta.trailing_zeros() as usize / 8;
        let high = 8 - delta.leading_zeros() as usize / 8;
        bytes.push(((high - low) * 8 + low) as u8);
        bytes.extend_from_slice(&delta.to_le_bytes()[low..high]);
    }
}

fn encode_compact(curves: &[Vec<Curve>], polys: &[Polygon], refs: &[Vec<Vec<i32>>], background: bool) -> Vec<u8> {
    let polygons: Vec<_> = polys.iter().zip(refs).filter(|(p, _)| !background || p.code != 0).collect();
    let mut used = vec![false; curves.len()];
    for (_, rings) in &polygons {
        for &r in rings.iter().flatten() {
            used[r.unsigned_abs() as usize - 1] = true;
        }
    }
    let mut remap = vec![0; curves.len()];
    let curves: Vec<_> = curves.iter().enumerate().filter_map(|(i, path)| {
        if used[i] { Some((i, path)) } else { None }
    }).enumerate().map(|(new, (old, path))| {
        remap[old] = new;
        path
    }).collect();
    let mut bytes = b"BZF2".to_vec();
    let mut endpoint_ids = HashMap::new();
    let mut endpoints = Vec::new();
    let mut id = |p: P| {
        *endpoint_ids.entry(p.map(f64::to_bits)).or_insert_with(|| {
            let id = endpoints.len();
            endpoints.push(p);
            id
        })
    };
    let endpoint_refs: Vec<_> = curves.iter()
        .map(|path| (id(path[0].p[0]), id(path.last().unwrap().p[3])))
        .collect();
    for n in [curves.len(), polygons.len(), endpoints.len()] {
        put_varint(&mut bytes, n as u64);
    }
    let mut previous = [0.; 2];
    for point in endpoints {
        put_point_delta(&mut bytes, point, previous);
        previous = point;
    }
    for (path, (start, end)) in curves.iter().zip(endpoint_refs) {
        put_varint(&mut bytes, path.len() as u64);
        put_varint(&mut bytes, start as u64);
        put_varint(&mut bytes, end as u64);
        for (i, c) in path.iter().enumerate() {
            bytes.push(u8::from(!c.line));
            if !c.line {
                put_point_delta(&mut bytes, c.p[1], c.p[0]);
                put_point_delta(&mut bytes, c.p[2], c.p[0]);
            }
            if i + 1 != path.len() {
                put_point_delta(&mut bytes, c.p[3], c.p[0]);
            }
        }
    }
    for (p, rings) in polygons {
        bytes.push(p.code);
        put_varint(&mut bytes, rings.len() as u64);
        for ring in rings {
            put_varint(&mut bytes, ring.len() as u64);
            for &r in ring {
                put_varint(&mut bytes, remap[r.unsigned_abs() as usize - 1] as u64 * 2 + u64::from(r < 0));
            }
        }
    }
    bytes
}

struct UnderlayFace {
    id: usize,
    envelope: AABB<P>,
}
impl RTreeObject for UnderlayFace {
    type Envelope = AABB<P>;
    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

fn ring_bounds(ring: &[P]) -> AABB<P> {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for p in ring {
        for axis in 0..2 {
            lo[axis] = lo[axis].min(p[axis]);
            hi[axis] = hi[axis].max(p[axis]);
        }
    }
    AABB::from_corners(lo, hi)
}

fn water_underlay(
    api: &Geos,
    input: Vec<(u8, Vec<Vec<P>>)>,
) -> Result<Vec<(u8, Vec<Vec<P>>)>> {
    let mut groups: [Vec<geos::Geometry<'_>>; 3] = std::array::from_fn(|_| Vec::new());
    let mut dry = Vec::new();
    let mut entries = Vec::new();
    for (code, rings) in &input {
        if *code < 3 {
            entries.push(UnderlayFace {
                id: dry.len(),
                envelope: ring_bounds(&rings[0]),
            });
            let poly = api.read(&wkb_poly(rings))?;
            dry.push((*code as usize, poly.boundary()?));
            groups[*code as usize].push(poly);
        }
    }
    let index = RTree::bulk_load(entries);
    let mut overlay = Vec::new();
    for (code, rings) in input {
        if code == 3 {
            let water = api.read(&wkb_poly(&rings))?;
            let bank = water.boundary()?;
            let mut lengths = [0.; 3];
            for face in index.locate_in_envelope_intersecting(&ring_bounds(&rings[0])) {
                let (class, boundary) = &dry[face.id];
                lengths[*class] += bank.intersection(boundary)?.length()?;
            }
            let owner = (0..3)
                .max_by(|&a, &b| lengths[a].total_cmp(&lengths[b]).then_with(|| b.cmp(&a)))
                .unwrap();
            groups[owner].push(water);
        }
        if code >= 3 {
            overlay.push((code, rings));
        }
    }
    for (code, group) in groups.into_iter().enumerate() {
        for poly in api.collection(&group)?.union()?.polygons()? {
            overlay.push((code as u8, poly.rings()?));
        }
    }
    Ok(overlay)
}

fn visible_at_500m(api: &Geos, part: &geos::Geometry<'_>, extent: AABB<P>) -> Result<bool> {
    visible_in_pixels(api, part, extent, 500.)
}

fn visible_in_pixels(api: &Geos, part: &geos::Geometry<'_>, extent: AABB<P>, cell: f64) -> Result<bool> {
    covers_pixel_fraction(api, part, extent, cell, 0.125)
}

fn covers_pixel_fraction(api: &Geos, part: &geos::Geometry<'_>, extent: AABB<P>, cell: f64, fraction: f64) -> Result<bool> {
    let visible_area = cell * cell * fraction;
    if part.area()? < visible_area {
        return Ok(false);
    }
    let lo = extent.lower().map(|v| (v / cell).floor() as i32);
    let hi = extent.upper().map(|v| (v / cell).ceil() as i32);
    for y in lo[1]..hi[1] {
        for x in lo[0]..hi[0] {
            let x = f64::from(x) * cell;
            let y = f64::from(y) * cell;
            let pixel = api.rectangle(x, y, x + cell, y + cell)?;
            let visible = if fraction == 1. {
                part.covers(&pixel)?
            } else {
                part.intersection(&pixel)?.area()? >= visible_area
            };
            if visible {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn merge_coastal_faces(
    api: &Geos,
    dry: Vec<(u8, Vec<Vec<P>>)>,
    sea: &geos::Geometry<'_>,
    background_only: bool,
) -> Result<(Vec<(u8, Vec<Vec<P>>)>, usize)> {
    let mut faces = Vec::new();
    let mut entries = Vec::new();
    for (code, rings) in dry {
        let extent = ring_bounds(&rings[0]);
        let geometry = api.read(&wkb_poly(&rings))?;
        let area = geometry.area()?;
        let boundary = geometry.boundary()?;
        entries.push(UnderlayFace { id: faces.len(), envelope: extent });
        faces.push((code, geometry, boundary, area));
    }
    let index = RTree::bulk_load(entries);
    let mut parents: Vec<_> = (0..faces.len()).collect();
    for entry in index.iter() {
        let id = entry.id;
        let (code, geometry, boundary, area) = &faces[id];
        if (background_only && *code != 0) || *area >= 250000. || !geometry.intersects(sea)?
            || visible_in_pixels(api, geometry, entry.envelope, if background_only {400.} else {500.})?
        {
            continue;
        }
        let mut best = (0., id);
        for neighbor in index.locate_in_envelope_intersecting(&entry.envelope) {
            if faces[neighbor.id].3 <= *area || (background_only && faces[neighbor.id].0 != 1) {
                continue;
            }
            let shared = boundary.intersection(&faces[neighbor.id].2)?.length()?;
            if shared > best.0 || (shared > 0. && shared == best.0 && neighbor.id < best.1) {
                best = (shared, neighbor.id);
            }
        }
        parents[id] = best.1;
    }
    let count = parents.iter().enumerate().filter(|&(i, p)| i != *p).count();
    let codes: Vec<_> = faces.iter().map(|f| f.0).collect();
    let mut groups: [Vec<_>; 3] = std::array::from_fn(|_| Vec::new());
    for (id, (_, geometry, _, _)) in faces.into_iter().enumerate() {
        let mut owner = id;
        while parents[owner] != owner {
            owner = parents[owner];
        }
        groups[codes[owner] as usize].push(geometry);
    }
    let mut output = Vec::new();
    for (code, group) in groups.into_iter().enumerate() {
        for part in api.collection(&group)?.union()?.polygons()? {
            output.push((code as u8, part.rings()?));
        }
    }
    Ok((output, count))
}

fn generalize_coast(
    api: &Geos,
    domain_side: f64,
    input: Vec<(u8, Vec<Vec<P>>)>,
    use_pixel_visibility: bool,
) -> Result<(Vec<(u8, Vec<Vec<P>>)>, [usize; 4])> {
    let domain = api.rectangle(0., 0., domain_side, domain_side)?;
    let border = domain.boundary()?;
    let collect_class = |class| -> Result<_> {
        let parts = input.iter().filter(|(code, _)| *code == class)
            .map(|(_, rings)| api.read(&wkb_poly(rings))).collect::<Result<Vec<_>>>()?;
        api.collection(&parts)?.union()
    };
    let sea = collect_class(4)?;
    let inland = collect_class(3)?;
    let coastal_band = sea.buffer(125., 1)?;
    let mut rivers = Vec::new();
    for water in inland.polygons()? {
        if !coastal_band.covers(&water)? {
            rivers.push(water);
        }
    }
    let protected = api.collection(&rivers)?.union()?.buffer(50., 1)?;
    let land = domain.difference(&sea)?;
    let mut islands = Vec::new();
    let island_area = if use_pixel_visibility { 250000. } else { 150000. };
    let island_extent = if use_pixel_visibility { 1000. } else { 750. };
    for part in land.polygons()? {
        let rings = part.rings()?;
        let extent = ring_bounds(&rings[0]);
        if part.area()? < island_area
            && norm(sub(extent.upper(), extent.lower())) < island_extent
            && (!use_pixel_visibility || !visible_at_500m(api, &part, extent)?)
            && !part.intersects(&border)?
            && !part.intersects(&protected)?
        {
            islands.push(part);
        }
    }
    let island_count = islands.len();
    if use_pixel_visibility {
        let initial_mask = api.collection(&islands)?.union()?;
        let mainland = land.difference(&initial_mask)?;
        let count_land = |geometry: &geos::Geometry<'_>| -> Result<usize> {
            let mut count = 0;
            for part in geometry.polygons()? {
                if part.area()? >= 100. { count += 1; }
            }
            Ok(count)
        };
        let mut cuts = Vec::new();
        const SIDE: f64 = 8000.;
        const PAD: f64 = 1800.;
        let windows = (domain_side / SIDE).ceil() as u32;
        for row in 0..windows {
            for col in 0..windows {
                let x = f64::from(col) * SIDE;
                let y = f64::from(row) * SIDE;
                let window = api.rectangle(x-PAD, y-PAD, x+SIDE+PAD, y+SIDE+PAD)?;
                if !sea.intersects(&window)? || !mainland.intersects(&window)? { continue; }
                let edge = window.boundary()?;
                let piece = mainland.intersection(&window)?;
                let land_parts = count_land(&piece)?;
                let sea_piece = window.difference(&piece)?;
                let expanded = sea_piece.buffer(65., 1)?;
                let closed = expanded.buffer(-65., 1)?;
                for cut in piece.intersection(&closed)?.polygons()? {
                    let area = cut.area()?;
                    if !(100. ..125000.).contains(&area) { continue; }
                    let [cx, cy] = cut.interior_xy()?;
                    if cx < x || cx >= x+SIDE || cy < y || cy >= y+SIDE
                        || cut.intersects(&edge)? || cut.intersects(&border)?
                        || cut.intersects(&protected)? || !cut.intersects(&coastal_band)?
                    {
                        continue;
                    }
                    let extent = ring_bounds(&cut.rings()?[0]);
                    if norm(sub(extent.upper(), extent.lower())) < 3000.
                        && !visible_in_pixels(api, &cut, extent, 400.)?
                        && count_land(&piece.difference(&cut)?)? <= land_parts
                    {
                        cuts.push(cut);
                    }
                }
            }
        }
        let mask = api.collection(&cuts)?.union()?;
        if count_land(&mainland.difference(&mask)?)? <= count_land(&mainland)? {
            islands.extend(cuts);
        }
    }
    let removed_protrusions = islands.len() - island_count;
    let island_mask = api.collection(&islands)?.union()?;
    // Close narrow inlets before fitting any shared boundary. Keep only
    // small additions that neither reach the domain edge nor touch a river.
    let retained_land = land.difference(&island_mask)?;
    let radius = if use_pixel_visibility { 200. } else { 100. };
    let closed = retained_land.buffer(radius, 1)?.buffer(-radius, 1)?;
    let mut inlets = Vec::new();
    for part in closed.difference(&retained_land)?.intersection(&sea)?.polygons()? {
        if part.area()? >= 100.
            && part.area()? < if use_pixel_visibility { 250000. } else { 80000. }
            && !part.intersects(&border)?
            && !part.intersects(&protected)?
            && (!use_pixel_visibility || !visible_at_500m(api, &part, ring_bounds(&part.rings()?[0]))?)
        {
            inlets.push(part);
        }
    }
    let fill = api.collection(&inlets)?.union()?;
    let mut dry = Vec::new();
    let mut output = Vec::new();
    for (code, rings) in input {
        if code < 3 {
            let face = api.read(&wkb_poly(&rings))?;
            let kept = if face.intersects(&island_mask)? {
                face.difference(&island_mask)?
            } else {
                face
            };
            for p in kept.polygons()? {
                dry.push((code, p.rings()?));
            }
        } else if code == 3 {
            let water = api.read(&wkb_poly(&rings))?;
            let kept = if water.intersects(&island_mask)? { water.difference(&island_mask)? } else { water };
            for p in kept.polygons()? {
                let rings = p.rings()?;
                if use_pixel_visibility && coastal_band.covers(&p)? && p.intersects(&sea)?
                    && !p.intersects(&protected)?
                    && !visible_at_500m(api, &p, ring_bounds(&rings[0]))?
                {
                    dry.push((3, rings));
                } else {
                    output.push((code, rings));
                }
            }
        }
    }
    for p in fill.polygons()? {
        dry.push((3, p.rings()?));
    }
    // Assign each filled inlet to the class sharing the longest shore.
    let dry: Vec<_> = water_underlay(api, dry)?.into_iter().filter(|(code, _)| *code < 3).collect();
    let new_sea = api.collection(&[sea.difference(&fill)?, island_mask])?.union()?;
    let (dry, merged) = if use_pixel_visibility {
        merge_coastal_faces(api, dry, &new_sea, false)?
    } else {
        (dry, 0)
    };
    output.extend(dry);
    for p in new_sea.polygons()? {
        output.push((4, p.rings()?));
    }
    Ok((output, [island_count, inlets.len(), merged, removed_protrusions]))
}

fn omit_small_bare_ground(
    api: &Geos,
    input: Vec<(u8, Vec<Vec<P>>)>,
    cell: f64,
) -> Result<(Vec<(u8, Vec<Vec<P>>)>, usize)> {
    let mut background = Vec::new();
    let mut output = Vec::new();
    let mut omitted = 0;
    for (code, rings) in input {
        if code == 0 {
            background.push(api.read(&wkb_poly(&rings))?);
            continue;
        }
        if code == 2 {
            let extent = ring_bounds(&rings[0]);
            if norm(sub(extent.upper(), extent.lower())) < cell / 2. {
                let face = api.read(&wkb_poly(&rings))?;
                if !covers_pixel_fraction(api, &face, extent, cell, 0.125)? {
                    background.push(face);
                    omitted += 1;
                    continue;
                }
            }
        }
        output.push((code, rings));
    }
    for face in api.collection(&background)?.union()?.polygons()? {
        output.push((0, face.rings()?));
    }
    Ok((output, omitted))
}

fn fill_small_forest_gaps(
    api: &Geos,
    input: Vec<(u8, Vec<Vec<P>>)>,
    final_water: Option<&geos::Geometry>,
) -> Result<(Vec<(u8, Vec<Vec<P>>)>, usize)> {
    let background = input.iter().filter(|(code, _)| *code == 0)
        .map(|(_, rings)| api.read(&wkb_poly(rings))).collect::<Result<Vec<_>>>()?;
    let background = api.collection(&background)?.union()?;
    let protected = if let Some(water) = final_water {
        let mut layers = input.iter().filter(|(code, _)| *code >= 2)
            .map(|(_, rings)| api.read(&wkb_poly(rings))).collect::<Result<Vec<_>>>()?;
        layers.push(api.read(&water.write()?)?);
        Some(api.collection(&layers)?.union()?)
    } else {
        None
    };
    let mut fills = Vec::new();
    for (_, rings) in input.iter().filter(|(code, _)| *code == 1) {
        for ring in rings.iter().skip(1) {
            let extent = ring_bounds(ring);
            if norm(sub(extent.upper(), extent.lower())) >= 250. { continue; }
            let hole = api.read(&wkb_poly(std::slice::from_ref(ring)))?;
            let unoccupied = if let Some(protected) = &protected {
                hole.area()? < 1000. && protected.intersection(&hole)?.area()? < 1e-6
            } else {
                false
            };
            if !visible_at_500m(api, &hole, extent)? && (unoccupied || background.covers(&hole)?) {
                fills.push(hole);
            }
        }
    }
    let count = fills.len();
    if count == 0 { return Ok((input, 0)); }
    let fill = api.collection(&fills)?.union()?;
    let mut forest = fills;
    let mut output = Vec::new();
    for (code, rings) in input {
        match code {
            0 => {},
            1 => forest.push(api.read(&wkb_poly(&rings))?),
            _ => output.push((code, rings)),
        }
    }
    for (code, geometry) in [(0, background.difference(&fill)?), (1, api.collection(&forest)?.union()?)] {
        for part in geometry.polygons()? {
            output.push((code, part.rings()?));
        }
    }
    Ok((output, count))
}

fn close_subpixel_water_breaks<'a>(
    api: &'a Geos,
    domain_side: f64,
    water: geos::Geometry<'a>,
) -> Result<(geos::Geometry<'a>, usize)> {
    let (water, narrow) = close_water_breaks_pass(api, domain_side, water, 90., 0.125, None)?;
    let (water, wider) = close_water_breaks_pass(api, domain_side, water, 160., 0.25, Some(110.))?;
    Ok((water, narrow + wider))
}

fn close_water_breaks_pass<'a>(
    api: &'a Geos,
    domain_side: f64,
    water: geos::Geometry<'a>,
    radius: f64,
    fraction: f64,
    maximum_width: Option<f64>,
) -> Result<(geos::Geometry<'a>, usize)> {
    let border = api.rectangle(0., 0., domain_side, domain_side)?.boundary()?;
    let parts = water.polygons()?;
    let index = RTree::bulk_load(parts.iter().enumerate().map(|(id, p)| {
        Ok(UnderlayFace { id, envelope: ring_bounds(&p.rings()?[0]) })
    }).collect::<Result<Vec<_>>>()?);
    let expanded = water.buffer(radius, 1)?;
    let closed = expanded.buffer(-radius, 1)?;
    let mut fills = Vec::new();
    for candidate in closed.difference(&water)?.polygons()? {
        let area = candidate.area()?;
        if !(100. ..125000.).contains(&area)
            || candidate.intersects(&border)?
        {
            continue;
        }
        let extent = ring_bounds(&candidate.rings()?[0]);
        if norm(sub(extent.upper(), extent.lower())) >= 800.
            || covers_pixel_fraction(api, &candidate, extent, 500., fraction)?
        {
            continue;
        }
        let mut shores = [0f64; 2];
        for hit in index.locate_in_envelope_intersecting(&extent) {
            let contact = candidate.intersection(&parts[hit.id])?.length()?;
            if contact > shores[0] {
                shores[1] = shores[0];
                shores[0] = contact;
            } else if contact > shores[1] {
                shores[1] = contact;
            }
        }
        if shores[1] > 1. && maximum_width.is_none_or(|width| {
            shores[1] >= 40. && 2. * area / (shores[0] + shores[1]) <= width
        }) {
            fills.push(candidate);
        }
    }
    let count = fills.len();
    if count == 0 { return Ok((water, count)); }
    fills.push(water);
    Ok((api.collection(&fills)?.union()?, count))
}

fn absorb_coastal_background<'a>(
    api: &'a Geos,
    input: &[(u8, Vec<Vec<P>>)],
    mut water: geos::Geometry<'a>,
    coast: &RTree<IndexedEdge>,
    pixel: f64,
    maximum_extent: f64,
) -> Result<geos::Geometry<'a>> {
    let background = input.iter().filter(|(code, _)| *code == 0)
        .map(|(_, rings)| api.read(&wkb_poly(rings))).collect::<Result<Vec<_>>>()?;
    let exposed = api.collection(&background)?.union()?.difference(&water)?;
    let mut patches = Vec::new();
    for p in exposed.polygons()? {
        let area = p.area()?;
        if area <= 0. || area >= 20000. || !p.intersects(&water)? { continue; }
        let extent = ring_bounds(&p.rings()?[0]);
        let center = p.interior_xy()?;
        if norm(sub(extent.upper(), extent.lower())) < maximum_extent
            && coast.nearest_neighbor(&center).is_some_and(|e| e.distance_2(&center) < 125f64.powi(2))
            && !visible_in_pixels(api, &p, extent, pixel)?
        {
            patches.push(p);
        }
    }
    if !patches.is_empty() {
        patches.push(water);
        water = api.collection(&patches)?.union()?;
    }
    Ok(water)
}

fn fill_small_water_holes<'a>(api: &'a Geos, water: geos::Geometry<'a>) -> Result<geos::Geometry<'a>> {
    let mut fills = Vec::new();
    for part in water.polygons()? {
        for ring in part.rings()?.into_iter().skip(1) {
            let extent = ring_bounds(&ring);
            if norm(sub(extent.upper(), extent.lower())) >= 650. {
                continue;
            }
            let hole = api.read(&wkb_poly(&[ring]))?;
            if hole.area()? < 80000. && !covers_pixel_fraction(api, &hole, extent, 500., 1. / 3.)? {
                fills.push(hole);
            }
        }
    }
    if fills.is_empty() {
        return Ok(water);
    }
    fills.push(water);
    api.collection(&fills)?.union()
}

fn omit_subpixel_inlets<'a>(
    api: &'a Geos,
    water: geos::Geometry<'a>,
    sea: &geos::Geometry<'a>,
    border: &geos::Geometry<'a>,
    radius: f64,
    area_range: std::ops::Range<f64>,
    max_extent: f64,
    pixel_fraction: f64,
    pixel_cell: f64,
    require_source_sea: bool,
) -> Result<geos::Geometry<'a>> {
    let mut inlets = Vec::new();
    for part in water.polygons()? {
        if !part.intersects(sea)? {
            continue;
        }
        let segments = if radius >= 500. { 4 } else { 1 };
        let opened = part.buffer(-radius, segments)?.buffer(radius, segments)?;
        if opened.is_empty()? {
            continue;
        }
        for inlet in part.difference(&opened)?.polygons()? {
            let area = inlet.area()?;
            if !area_range.contains(&area) || inlet.intersects(border)?
                || (require_source_sea && inlet.intersection(sea)?.area()? < area * 0.5)
            {
                continue;
            }
            let bounds = ring_bounds(&inlet.rings()?[0]);
            if norm(sub(bounds.upper(), bounds.lower())) < max_extent
                && !covers_pixel_fraction(api, &inlet, bounds, pixel_cell, pixel_fraction)?
            {
                inlets.push(inlet);
            }
        }
    }
    if inlets.is_empty() {
        return Ok(water);
    }
    water.difference(&api.collection(&inlets)?.union()?)
}

fn simplify_inlet_mouths<'a>(
    api: &'a Geos,
    before: &geos::Geometry<'a>,
    water: geos::Geometry<'a>,
    tolerance: f64,
) -> Result<geos::Geometry<'a>> {
    let original_boundary = before.boundary()?.buffer(0.001, 1)?;
    let mut parts = Vec::new();
    for part in water.polygons()? {
        let mut rings = part.rings()?;
        for ring in &mut rings {
            let points = &ring[..ring.len() - 1];
            let anchors = points.iter().enumerate().filter_map(|(i, p)| {
                match original_boundary.covers_xy(p[0], p[1]) {
                    Ok(true) => Some(Ok(i)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error)),
                }
            }).collect::<Result<Vec<_>>>()?;
            if anchors.len() < 3 || anchors.len() == points.len() {
                continue;
            }
            let mut keep = vec![false; points.len()];
            let mut spans = Vec::new();
            for (i, &a) in anchors.iter().enumerate() {
                keep[a] = true;
                let b = anchors.get(i + 1).copied().unwrap_or(anchors[0] + points.len());
                spans.push((a, b));
            }
            while let Some((a, b)) = spans.pop() {
                let left = points[a % points.len()];
                let right = points[b % points.len()];
                let mut worst = (tolerance, None);
                for i in a + 1..b {
                    let distance = dist_seg(points[i % points.len()], left, right);
                    if distance > worst.0 {
                        worst = (distance, Some(i));
                    }
                }
                if let Some(i) = worst.1 {
                    keep[i % points.len()] = true;
                    spans.push((a, i));
                    spans.push((i, b));
                }
            }
            let mut reduced: Vec<_> = points.iter().zip(keep)
                .filter_map(|(&point, keep)| keep.then_some(point)).collect();
            reduced.push(reduced[0]);
            *ring = reduced;
        }
        let candidate = api.read(&wkb_poly(&rings))?;
        parts.push(if candidate.is_valid()? { candidate } else { part });
    }
    let simplified = api.collection(&parts)?.union()?;
    if simplified.counts()? == water.counts()? {
        Ok(simplified)
    } else {
        Ok(water)
    }
}

fn simplify_filled_shore<'a>(
    before: &geos::Geometry<'a>,
    mut water: geos::Geometry<'a>,
    border: &geos::Geometry<'a>,
) -> Result<geos::Geometry<'a>> {
    let filled = before.difference(&water)?;
    if filled.is_empty()? {
        return Ok(water);
    }
    let coordinate_count = |shape: &geos::Geometry| -> Result<usize> {
        shape.polygons()?.iter().try_fold(0, |sum, polygon| {
            Ok(sum + polygon.rings()?.iter().map(Vec::len).sum::<usize>())
        })
    };
    let mut coordinates = coordinate_count(&water)?;
    let shape_counts = water.counts()?;
    let edge = water.intersection(border)?;
    let maximum_refill = filled.area()? * 0.01;
    for tolerance in [10., 20., 35.] {
        loop {
            let candidate = water.topology_simplify(tolerance)?;
            let remaining = coordinate_count(&candidate)?;
            if remaining >= coordinates
                || !candidate.is_valid()?
                || candidate.counts()? != shape_counts
                || candidate.intersection(&filled)?.area()? > maximum_refill
            {
                break;
            }
            let candidate_edge = candidate.intersection(border)?;
            if edge.difference(&candidate_edge)?.length()? > 1e-6
                || candidate_edge.difference(&edge)?.length()? > 1e-6
            {
                break;
            }
            water = candidate;
            coordinates = remaining;
        }
    }
    Ok(water)
}

fn simplify_final_coast<'a>(
    api: &'a Geos,
    water: geos::Geometry<'a>,
    sea: &geos::Geometry<'a>,
    domain_side: f64,
    tolerance: f64,
    pixel_cell: Option<f64>,
) -> Result<geos::Geometry<'a>> {
    let border_width = tolerance.min(750.);
    let inner = api.rectangle(border_width, border_width, domain_side - border_width, domain_side - border_width)?;
    let mut parts = Vec::new();
    for part in water.polygons()? {
        if !part.intersects(sea)? {
            parts.push(part);
            continue;
        }
        let simplified = part.topology_simplify(tolerance)?;
        let mut restored = vec![simplified];
        // A long shortcut can create a thin strip of land across the sea.
        for strip in part.difference(&restored[0])?.polygons()? {
            let area = strip.area()?;
            if area <= 0. {
                continue;
            }
            let bounds = ring_bounds(&strip.rings()?[0]);
            let extent = norm(sub(bounds.upper(), bounds.lower()));
            let thin = extent >= 2000. && 2. * area <= 100. * strip.boundary()?.length()?;
            let small = if let Some(cell) = pixel_cell {
                area < cell * cell && extent < 4. * cell
                    && !covers_pixel_fraction(api, &strip, bounds, cell, 0.5)?
            } else {
                false
            };
            if thin || small {
                restored.push(strip);
            }
        }
        let simplified = if restored.len() == 1 {
            restored.pop().unwrap()
        } else {
            api.collection(&restored)?.union()?
        };
        // Keep the exact shoreline at the edge of the generated extent.
        let inside = simplified.intersection(&inner)?;
        let edge = part.difference(&inner)?;
        parts.push(api.collection(&[inside, edge])?.union()?);
    }
    api.collection(&parts)?.union()
}

fn omit_long_thin_coastal_land<'a>(
    api: &'a Geos,
    water: geos::Geometry<'a>,
    sea: &geos::Geometry<'a>,
    domain: &geos::Geometry<'a>,
    pixel_cell: Option<f64>,
) -> Result<geos::Geometry<'a>> {
    let border = domain.boundary()?;
    let coast = water.polygons()?.into_iter()
        .filter_map(|part| match part.intersects(sea) {
            Ok(true) => Some(Ok(part)),
            Ok(false) => None,
            Err(error) => Some(Err(error)),
        }).collect::<Result<Vec<_>>>()?;
    let coast = api.collection(&coast)?.union()?.buffer(0.001, 1)?;
    let land = domain.difference(&water)?;
    let opened = land.buffer(-150., 1)?.buffer(150., 1)?;
    let mut removed = Vec::new();
    if let Some(cell) = pixel_cell {
        for island in land.polygons()? {
            let area = island.area()?;
            if !(0. ..cell * cell).contains(&area) || island.is_empty()? || island.intersects(&border)? {
                continue;
            }
            let boundary = island.boundary()?;
            if boundary.intersection(&coast)?.length()? >= 0.99 * boundary.length()?
                && !covers_pixel_fraction(api, &island, ring_bounds(&island.rings()?[0]), cell, 0.5)?
            {
                removed.push(island);
            }
        }
    }
    for strip in land.difference(&opened)?.polygons()? {
        let area = strip.area()?;
        if area <= 0. || strip.intersects(&border)? {
            continue;
        }
        let bounds = ring_bounds(&strip.rings()?[0]);
        if norm(sub(bounds.upper(), bounds.lower())) < 2000. {
            continue;
        }
        let boundary = strip.boundary()?;
        let perimeter = boundary.length()?;
        if 2. * area <= 110. * perimeter
            && boundary.intersection(&coast)?.length()? >= perimeter * 0.8
        {
            removed.push(strip);
        }
    }
    if removed.is_empty() {
        return Ok(water);
    }
    removed.push(water);
    api.collection(&removed)?.union()
}

fn close_narrow_inlet_mouths<'a>(
    api: &'a Geos,
    water: geos::Geometry<'a>,
    sea: &geos::Geometry<'a>,
    border: &geos::Geometry<'a>,
    cell: f64,
) -> Result<geos::Geometry<'a>> {
    let pixel_area = cell * cell;
    let mut candidates = Vec::new();
    for part in water.polygons()? {
        if !part.intersects(sea)? {
            continue;
        }
        for mut ring in part.rings()? {
            ring.pop();
            let n = ring.len();
            for start in 0..n {
                let mut arc = vec![ring[start]];
                let mut length = 0.;
                for step in 1..n.min(128) {
                    let point = ring[(start + step) % n];
                    length += norm(sub(point, *arc.last().unwrap()));
                    if length > 2.5 * cell {
                        break;
                    }
                    arc.push(point);
                    let mouth = norm(sub(point, ring[start]));
                    if step < 3 || mouth > cell * 0.2 || length < 3. * mouth {
                        continue;
                    }
                    let mut closed = arc.clone();
                    closed.push(ring[start]);
                    let cut = api.read(&wkb_poly(&[closed]))?;
                    if !cut.is_valid()? {
                        continue;
                    }
                    let area = cut.area()?;
                    if !(100. ..pixel_area / 4.).contains(&area)
                        || cut.intersects(border)?
                        || cut.difference(&part)?.area()? > 1e-6
                    {
                        continue;
                    }
                    candidates.push((area, ring_bounds(&arc), cut));
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut filled: Vec<geos::Geometry<'a>> = Vec::new();
    let mut pixel_water = HashMap::new();
    for (_, bounds, cut) in candidates {
        // 重複した候補を除き、先に採用した候補による水域の減少も考慮する。
        let mut overlaps = false;
        for previous in &filled {
            if cut.intersection(previous)?.area()? > 1e-6 {
                overlaps = true;
                break;
            }
        }
        if overlaps {
            continue;
        }
        let lo = bounds.lower().map(|v| (v / cell).floor() as i32);
        let hi = bounds.upper().map(|v| (v / cell).ceil() as i32);
        let mut updates = Vec::new();
        let mut preserves_majority = true;
        for y in lo[1]..hi[1] {
            for x in lo[0]..hi[0] {
                let left = f64::from(x) * cell;
                let bottom = f64::from(y) * cell;
                let pixel = api.rectangle(left, bottom, left + cell, bottom + cell)?;
                let loss = cut.intersection(&pixel)?.area()?;
                if loss <= 1e-6 {
                    continue;
                }
                let before = match pixel_water.get(&(x, y)) {
                    Some(&area) => area,
                    None => water.intersection(&pixel)?.area()?,
                };
                let after = before - loss;
                if before >= pixel_area / 2. && after < pixel_area / 2. {
                    preserves_majority = false;
                }
                updates.push(((x, y), after));
            }
        }
        if preserves_majority {
            pixel_water.extend(updates);
            filled.push(cut);
        }
    }
    if filled.is_empty() {
        return Ok(water);
    }
    let result = water.difference(&api.collection(&filled)?.union()?)?;
    if result.is_valid()? && result.counts()? == water.counts()? {
        println!("L5 narrow inlet mouths closed: {}", filled.len());
        Ok(result)
    } else {
        Ok(water)
    }
}
fn fill_subpixel_water_hole(
    api: &Geos,
    ring: &[P],
    water: &geos::Geometry<'_>,
    cell: f64,
    pixel_water: &mut HashMap<(i32, i32), f64>,
) -> Result<bool> {
    let bounds = ring_bounds(ring);
    let hole = api.read(&wkb_poly(&[ring.to_vec()]))?;
    let pixel_area = cell * cell;
    if hole.area()? >= pixel_area {
        return Ok(false);
    }
    // 穴の中に別の水域がある場合は、その水域との重なりを避ける。
    if hole.intersection(water)?.area()? > 1e-6 {
        return Ok(false);
    }
    let lo = bounds.lower().map(|v| (v / cell).floor() as i32);
    let hi = bounds.upper().map(|v| (v / cell).ceil() as i32);
    let mut updates = Vec::new();
    for y in lo[1]..hi[1] {
        for x in lo[0]..hi[0] {
            let left = f64::from(x) * cell;
            let bottom = f64::from(y) * cell;
            let pixel = api.rectangle(left, bottom, left + cell, bottom + cell)?;
            let before = match pixel_water.get(&(x, y)) {
                Some(&area) => area,
                None => water.intersection(&pixel)?.area()?,
            };
            let after = before + hole.intersection(&pixel)?.area()?;
            if before < pixel_area / 2. && after >= pixel_area / 2. {
                return Ok(false);
            }
            updates.push(((x, y), after));
        }
    }
    pixel_water.extend(updates);
    Ok(true)
}
fn omit_subpixel_coastal_details<'a>(
    api: &'a Geos,
    water: geos::Geometry<'a>,
    domain: &geos::Geometry<'a>,
    cell: f64,
) -> Result<geos::Geometry<'a>> {
    let pixel_area = cell * cell;
    let border = domain.boundary()?;
    let land = domain.difference(&water)?;
    let mut candidates = Vec::new();
    for (shape, sign) in [(&water, -1.), (&land, 1.)] {
        for part in shape.polygons()? {
            let area = part.area()?;
            if area <= 0. {
                continue;
            }
            if area < pixel_area {
                if !part.intersects(&border)? {
                    candidates.push((area, sign, part));
                }
                continue;
            }
            let opened = part.buffer(-cell / 2., 1)?.buffer(cell / 2., 1)?;
            for cut in part.difference(&opened)?.polygons()? {
                let area = cut.area()?;
                if (pixel_area / 100. ..pixel_area).contains(&area) && !cut.intersects(&border)? {
                    // 切り口の生成で点数が増える候補は採用しない。
                    let bounds = ring_bounds(&cut.rings()?[0]);
                    let [x0, y0] = bounds.lower();
                    let [x1, y1] = bounds.upper();
                    let pad = cell * 0.001;
                    let window = api.rectangle(x0 - pad, y0 - pad, x1 + pad, y1 + pad)?;
                    let local = part.intersection(&window)?;
                    let remainder = local.difference(&cut)?;
                    let count = |g: &geos::Geometry<'_>| -> Result<usize> {
                        let mut n = 0;
                        for p in g.polygons()? {
                            n += p.rings()?.iter().map(Vec::len).sum::<usize>();
                        }
                        Ok(n)
                    };
                    if count(&remainder)? < count(&local)? {
                        candidates.push((area, sign, cut));
                    }
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut original = HashMap::new();
    let mut current = HashMap::new();
    let mut to_land = Vec::new();
    let mut to_water = Vec::new();
    for (_, sign, cut) in candidates {
        let bounds = ring_bounds(&cut.rings()?[0]);
        let lo = bounds.lower().map(|v| (v / cell).floor() as i32);
        let hi = bounds.upper().map(|v| (v / cell).ceil() as i32);
        let mut updates = Vec::new();
        let mut preserves_majority = true;
        for y in lo[1]..hi[1] {
            for x in lo[0]..hi[0] {
                let left = f64::from(x) * cell;
                let bottom = f64::from(y) * cell;
                let pixel = api.rectangle(left, bottom, left + cell, bottom + cell)?;
                let change = cut.intersection(&pixel)?.area()?;
                if change <= 1e-6 {
                    continue;
                }
                let before = match original.entry((x, y)) {
                    std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        *entry.insert(water.intersection(&pixel)?.area()?)
                    }
                };
                let after = current.get(&(x, y)).copied().unwrap_or(before) + sign * change;
                preserves_majority &= (before >= pixel_area / 2.) == (after >= pixel_area / 2.);
                updates.push(((x, y), after));
            }
        }
        if preserves_majority && !updates.is_empty() {
            current.extend(updates);
            if sign < 0. {
                to_land.push(cut);
            } else {
                to_water.push(cut);
            }
        }
    }
    to_water.push(water.difference(&api.collection(&to_land)?.union()?)?);
    let result = api.collection(&to_water)?.union()?;
    ensure!(result.is_valid()?, "L5の細部を統合した水域が不正です");
    println!("Subpixel details: {} to land, {} to water", to_land.len(), to_water.len() - 1);
    Ok(result)
}
struct PixelAreaIndex<'a> {
    parts: Vec<geos::Geometry<'a>>,
    bounds: RTree<UnderlayFace>,
}

impl<'a> PixelAreaIndex<'a> {
    fn new(forest: &geos::Geometry<'a>) -> Result<Self> {
        let parts = forest.polygons()?;
        let mut bounds = Vec::with_capacity(parts.len());
        for (id, part) in parts.iter().enumerate() {
            if !part.is_empty()? {
                bounds.push(UnderlayFace { id, envelope: ring_bounds(&part.rings()?[0]) });
            }
        }
        Ok(Self { parts, bounds: RTree::bulk_load(bounds) })
    }

    fn area(&self, pixel: &geos::Geometry<'_>, left: f64, bottom: f64, cell: f64) -> Result<f64> {
        let bounds = AABB::from_corners([left, bottom], [left + cell, bottom + cell]);
        let mut ids: Vec<_> = self.bounds.locate_in_envelope_intersecting(&bounds)
            .map(|part| part.id).collect();
        ids.sort_unstable();
        let mut area = 0.;
        for id in ids {
            area += self.parts[id].intersection(pixel)?.area()?;
        }
        Ok(area)
    }
}

fn accept_forest_pixel_loss(
    api: &Geos,
    loss: &geos::Geometry<'_>,
    visible_forest: &PixelAreaIndex<'_>,
    cell: f64,
    pixel_areas: &mut HashMap<(i32, i32), f64>,
) -> Result<bool> {
    let mut changes = Vec::new();
    for part in loss.polygons()? {
        // GEOS can return POLYGON EMPTY. Its ring has no finite extent.
        if part.is_empty()? {
            continue;
        }
        let bounds = ring_bounds(&part.rings()?[0]);
        let lo = bounds.lower().map(|v| (v / cell).floor() as i32);
        let hi = bounds.upper().map(|v| (v / cell).ceil() as i32);
        for y in lo[1]..hi[1] {
            for x in lo[0]..hi[0] {
                let left = f64::from(x) * cell;
                let bottom = f64::from(y) * cell;
                let pixel = api.rectangle(left, bottom, left + cell, bottom + cell)?;
                let area = part.intersection(&pixel)?.area()?;
                if area <= 1e-6 {
                    continue;
                }
                let before = match pixel_areas.get(&(x, y)) {
                    Some(&area) => area,
                    None => visible_forest.area(&pixel, left, bottom, cell)?,
                };
                changes.push(((x, y), before, area));
            }
        }
    }
    let mut totals = HashMap::new();
    for (key, before, area) in changes {
        let after = totals.entry(key).or_insert(before);
        *after -= area;
        if before >= cell * cell / 2. && *after < cell * cell / 2. {
            return Ok(false);
        }
    }
    pixel_areas.extend(totals);
    Ok(true)
}
fn shorten_submerged_forest_ring(
    api: &Geos,
    refs: &[i32],
    curves: &[Vec<Curve>],
    forest: &geos::Geometry<'_>,
    water: &geos::Geometry<'_>,
    visible_tolerance: Option<(f64, &geos::PreparedGeometry<'_, '_>)>,
) -> Result<Option<Vec<Curve>>> {
    let allow_visible_cut = visible_tolerance.is_some();
    let mut ring = Vec::new();
    for &id in refs {
        let path = &curves[id.unsigned_abs() as usize - 1];
        if id > 0 {
            ring.extend(path.iter().cloned());
        } else {
            ring.extend(path.iter().rev().cloned().map(|mut curve| {
                curve.p.reverse();
                curve
            }));
        }
    }
    let submerged = if allow_visible_cut {
        Vec::new()
    } else {
        ring.iter().map(|curve| {
            let [x, y] = curve.p[0];
            water.covers_xy(x, y)
        }).collect::<Result<Vec<_>>>()?
    };
    let mut output = Vec::new();
    let mut start = 0;
    let max_segments = if allow_visible_cut { 12 } else { 64 };
    while start < ring.len() {
        let mut end = start;
        while end < ring.len() && end - start < max_segments && ring[end].line
            && (allow_visible_cut || (submerged[end] && submerged[(end + 1) % ring.len()]))
        {
            end += 1;
        }
        let mut used = start + 1;
        for stop in (start + 2..=end).rev() {
            let a = ring[start].p[0];
            let b = ring[stop - 1].p[3];
            if a == b {
                continue;
            }
            let mut boundary: Vec<_> = ring[start..stop].iter().map(|curve| curve.p[0]).collect();
            if visible_tolerance.is_some_and(|(t, _)| boundary.iter().any(|&p| dist_seg(p, a, b) > t)) {
                continue;
            }
            boundary.extend([b, a]);
            let patch = api.read(&wkb_poly(&[boundary]))?;
            if patch.is_valid()? && (allow_visible_cut || patch.difference(water)?.area()? < 1e-6)
                && if let Some((_, prepared)) = visible_tolerance { prepared.covers(&patch)? } else { patch.difference(forest)?.area()? < 1e-6 }
            {
                output.push(Curve::line(a, b));
                used = stop;
                break;
            }
        }
        if used == start + 1 {
            output.push(ring[start].clone());
        }
        start = used;
    }
    Ok((output.len() >= 3 && output.len() < ring.len()).then_some(output))
}
fn subpixel_forest_polygons(
    api: &Geos,
    forest_parts: &[Vec<u8>],
    water: &geos::Geometry<'_>,
    border: &geos::Geometry<'_>,
    cell: f64,
) -> Result<Vec<usize>> {
    let pixel_area = cell * cell;
    let visible_forest = PixelAreaIndex::new(&api.read(&wkb_multi(forest_parts))?.difference(water)?)?;
    let mut pixel_areas = HashMap::new();
    let mut omitted = Vec::new();
    for (index, wkb) in forest_parts.iter().enumerate() {
        let part = api.read(wkb)?;
        if part.intersects(border)? {
            continue;
        }
        let visible = part.difference(water)?;
        if visible.area()? >= pixel_area {
            continue;
        }
        let bounds = ring_bounds(&part.rings()?[0]);
        let lo = bounds.lower().map(|v| (v / cell).floor() as i32);
        let hi = bounds.upper().map(|v| (v / cell).ceil() as i32);
        let mut changes = Vec::new();
        let mut preserves_majority = true;
        for y in lo[1]..hi[1] {
            for x in lo[0]..hi[0] {
                let left = f64::from(x) * cell;
                let bottom = f64::from(y) * cell;
                let pixel = api.rectangle(left, bottom, left + cell, bottom + cell)?;
                let loss = visible.intersection(&pixel)?.area()?;
                if loss <= 1e-6 {
                    continue;
                }
                let before = match pixel_areas.get(&(x, y)) {
                    Some(&area) => area,
                    None => visible_forest.area(&pixel, left, bottom, cell)?,
                };
                let after = before - loss;
                preserves_majority &= before < pixel_area / 2. || after >= pixel_area / 2.;
                changes.push(((x, y), after));
            }
        }
        if preserves_majority {
            pixel_areas.extend(changes);
            omitted.push(index);
        }
    }
    Ok(omitted)
}
fn independent_shore<'a>(
    api: &'a Geos,
    domain_side: f64,
    mut dry: Vec<(u8, Vec<Vec<P>>)>,
    water: geos::Geometry<'a>,
    sea: &geos::Geometry<'a>,
    tolerance: f64,
    coast: Option<&RTree<IndexedEdge>>,
    inland_water_cell: f64,
    inland_water_min_coverage: f64,
    keep_thin_inland_water: bool,
    final_coast_simplify: Option<f64>,
    coastal_pixel_cell: Option<f64>,
    forest_line_tolerance: Option<f64>,
) -> Result<(Vec<(u8, Vec<Vec<P>>)>, geos::Geometry<'a>, Option<usize>)> {
    let extent = api.rectangle(0., 0., domain_side, domain_side)?;
    let sea_boundary = sea.boundary()?;
    let mut parts = Vec::new();
    for part in water.polygons()? {
        if part.intersects(sea)? && part.boundary()?.intersects(&sea_boundary)? {
            parts.extend(part.topology_simplify(tolerance)?.polygons()?);
        } else {
            parts.push(part);
        }
    }
    let mut new_water = api.collection(&parts)?.union()?.intersection(&extent)?;
    let mut closed_water_breaks = None;
    if let Some(coast) = coast {
        let (closed, count) = close_subpixel_water_breaks(api, domain_side, new_water)?;
        let filled = fill_small_water_holes(api, closed)?;
        new_water = absorb_coastal_background(api, &dry, filled, coast, 500., 350.)?
            .topology_simplify(tolerance.max(100.))?
            .intersection(&extent)?;
        let border = extent.boundary()?;
        let before_inlets = api.read(&new_water.write()?)?;
        for radius in [200., 220., 250.] {
            new_water = omit_subpixel_inlets(api, new_water, sea, &border, radius, 100. ..160000., 800., 0.25, 500., true)?;
        }
        new_water = omit_subpixel_inlets(api, new_water, sea, &border, 500., 160000. ..245000., 1100., 0.60, 500., true)?;
        new_water = simplify_filled_shore(&before_inlets, new_water, &border)?;
        {
            // The final shore can leave thin, isolated water just above the source cutoff.
            let mut visible_water = Vec::new();
            for part in new_water.polygons()? {
                if keep_thin_inland_water || part.intersects(sea)? || part.intersects(&border)?
                    || covers_pixel_fraction(api, &part, ring_bounds(&part.rings()?[0]), inland_water_cell, inland_water_min_coverage)?
                {
                    visible_water.push(part);
                }
            }
            new_water = api.collection(&visible_water)?.union()?;
        }
        closed_water_breaks = Some(count);
    }
    if let Some(distance) = final_coast_simplify {
        new_water = simplify_final_coast(api, new_water, sea, domain_side, distance, coastal_pixel_cell)?;
        new_water = omit_long_thin_coastal_land(api, new_water, sea, &extent, coastal_pixel_cell)?;
        if let Some(count) = &mut closed_water_breaks {
            let (joined, extra) = close_water_breaks_pass(api, domain_side, new_water, 200., 0.33, Some(175.))?;
            new_water = joined;
            *count += extra;
        }
        if let Some(cell) = coastal_pixel_cell {
            let border = extent.boundary()?;
            let before_inlets = api.read(&new_water.write()?)?;
            new_water = omit_subpixel_inlets(
                api, new_water, sea, &border, cell / 2.,
                100. ..cell * cell / 2., 2. * cell, 0.5, cell, true,
            )?;
            // 元の海域マスクにない小さな入り江も除く。
            new_water = omit_subpixel_inlets(
                api, new_water, sea, &border, cell,
                100. ..cell * cell * 0.02, cell * 0.6, 0.02, cell, false,
            )?;
            new_water = simplify_inlet_mouths(api, &before_inlets, new_water, cell / 10.)?;
            let candidate = new_water.topology_simplify(cell * 0.06)?;
            if candidate.is_valid()? && candidate.counts()? == new_water.counts()? {
                let before_edge = new_water.intersection(&border)?;
                let after_edge = candidate.intersection(&border)?;
                if before_edge.difference(&after_edge)?.length()? <= 1e-6
                    && after_edge.difference(&before_edge)?.length()? <= 1e-6
                {
                    new_water = candidate;
                }
            }
        }
    }
    if let (Some(_), Some(cell)) = (final_coast_simplify, coastal_pixel_cell) {
        new_water = close_narrow_inlet_mouths(api, new_water, sea, &extent.boundary()?, cell)?;
        new_water = omit_subpixel_coastal_details(api, new_water, &extent, cell)?;
    }
    let original = dry.iter().map(|(_, rings)| api.read(&wkb_poly(rings)))
        .collect::<Result<Vec<_>>>()?;
    let assemble = |shapes: Vec<geos::Geometry<'a>>, line_tolerance: Option<f64>| -> Result<_> {
        ensure!(shapes.len() == dry.len(), "Dry simplification changed face count");
        let mut groups: [Vec<_>; 3] = std::array::from_fn(|_| Vec::new());
        for ((code, _), shape) in dry.iter().zip(shapes) {
            groups[*code as usize].push(shape);
        }
        let mut output = Vec::new();
        let mut covered = Vec::new();
        for (code, shapes) in groups.into_iter().enumerate().rev() {
            let mut layer = api.collection(&shapes)?.union()
                .with_context(|| format!("Unite dry class {code}"))?.intersection(&extent)?;
            if let Some(distance) = line_tolerance.filter(|_| code == 1) {
                layer = layer.topology_simplify(distance)?.intersection(&extent)?;
            }
            if !covered.is_empty() {
                layer = layer.difference(&api.collection(&covered)?.union()?)
                    .with_context(|| format!("Remove overlap from dry class {code}"))?;
            }
            for p in layer.polygons()? {
                if !p.is_empty()? {
                    output.push((code as u8, p.rings()?));
                    covered.push(p);
                }
            }
        }
        covered.push(api.read(&new_water.write()?)?);
        let missing = extent.difference(&api.collection(&covered)?.union()?)
            .context("Find missing dry coverage")?;
        if !missing.is_empty()? {
            for part in missing.polygons()? {
                if !part.is_empty()? {
                    output.push((3, part.rings()?));
                }
            }
            output = water_underlay(api, output)?.into_iter().filter(|(code, _)| *code < 3).collect();
        }
        Ok(output)
    };
    let reduced = api.collection(&original)?.coverage_simplify_with_boundary(tolerance, false)
        .and_then(|shapes| shapes.polygons()).context("Simplify dry coverage");
    dry = match reduced.and_then(|shapes| assemble(shapes, forest_line_tolerance)) {
        Ok(output) => output,
        Err(error) if format!("{error:#}").contains("TopologyException") => {
            println!("Retaining original dry boundaries after simplification error: {error:#}");
            assemble(original, None).context("Restore unsimplified dry coverage")?
        }
        Err(error) => return Err(error),
    };
    Ok((dry, new_water, closed_water_breaks))
}

fn generalize_inland_water(
    api: &Geos,
    domain_side: f64,
    cell: f64,
    input: Vec<(u8, Vec<Vec<P>>)>,
) -> Result<(Vec<(u8, Vec<Vec<P>>)>, [usize; 2])> {
    let domain_edge = api.rectangle(0., 0., domain_side, domain_side)?.boundary()?;
    let mut dry = Vec::new();
    let mut sea = Vec::new();
    let mut water = Vec::new();
    for (code, rings) in input {
        match code {
            3 => water.push((api.read(&wkb_poly(&rings))?, rings)),
            4 => sea.push((code, rings)),
            _ => dry.push((code, rings)),
        }
    }
    let sea_parts = sea.iter().map(|(_, rings)| api.read(&wkb_poly(rings))).collect::<Result<Vec<_>>>()?;
    let sea_mask = api.collection(&sea_parts)?.union()?;
    let mut retained = Vec::new();
    let mut fills = Vec::new();
    let mut omitted = 0;
    for (part, rings) in water {
        let extent = ring_bounds(&rings[0]);
        if !part.intersects(&domain_edge)?
            && !part.intersects(&sea_mask)?
            && !visible_in_pixels(api, &part, extent, cell)?
        {
            dry.push((3, rings));
            omitted += 1;
            continue;
        }
        for ring in rings.iter().skip(1) {
            let hole = api.read(&wkb_poly(std::slice::from_ref(ring)))?;
            let extent = ring_bounds(ring);
            if norm(sub(extent.upper(), extent.lower())) < 650.
                && !visible_at_500m(api, &hole, extent)?
                && !hole.intersects(&sea_mask)?
            {
                fills.push(hole);
            }
        }
        retained.push(part);
    }
    let mut dry: Vec<_> = water_underlay(api, dry)?.into_iter()
        .filter(|(code, _)| *code < 3).collect();
    if !fills.is_empty() {
        let fill = api.collection(&fills)?.union()?;
        let mut remaining = Vec::new();
        for (code, rings) in dry {
            let part = api.read(&wkb_poly(&rings))?;
            if !part.intersects(&fill)? {
                remaining.push((code, rings));
            } else {
                for p in part.difference(&fill)?.polygons()? {
                    remaining.push((code, p.rings()?));
                }
            }
        }
        dry = remaining;
        retained.push(fill);
    }
    for part in api.collection(&retained)?.union()?.polygons()? {
        dry.push((3, part.rings()?));
    }
    let shapes = dry.iter().map(|(_, rings)| api.read(&wkb_poly(rings))).collect::<Result<Vec<_>>>()?;
    let simplified = api.collection(&shapes)?.coverage_simplify(100.)?.polygons()?;
    ensure!(simplified.len() == dry.len(), "Inland simplification changed face count");
    let mut output = dry.into_iter().zip(simplified).map(|((code, _), part)| {
        Ok((code, part.rings()?))
    }).collect::<Result<Vec<_>>>()?;
    output.extend(sea);
    Ok((output, [omitted, fills.len()]))
}

fn pixel_lod(
    api: &Geos,
    domain_side: f64,
    input: &[(u8, Vec<Vec<P>>)],
    side: usize,
) -> Result<Vec<(u8, Vec<Vec<P>>)>> {
    const SUB: usize = 8;
    let n = side * SUB;
    let cell = domain_side / side as f64;
    let sample = cell / SUB as f64;
    let mut pixels = vec![u8::MAX; n * n];
    let sample_index = |v: f64| ((v / sample - 0.5).ceil() as usize).min(n);
    for (code, rings) in input {
        let extent = ring_bounds(&rings[0]);
        let first = sample_index(extent.lower()[1]);
        let last = sample_index(extent.upper()[1]);
        let mut rows = vec![Vec::<f64>::new(); last - first];
        for ring in rings {
            for edge in ring.windows(2) {
                let (a, b) = (edge[0], edge[1]);
                let (lo, hi) = if a[1] < b[1] { (a, b) } else { (b, a) };
                for row in sample_index(lo[1])..sample_index(hi[1]) {
                    let y = (row as f64 + 0.5) * sample;
                    let x = lo[0] + (hi[0] - lo[0]) * (y - lo[1]) / (hi[1] - lo[1]);
                    rows[row - first].push(x);
                }
            }
        }
        for (row, xs) in rows.iter_mut().enumerate() {
            xs.sort_unstable_by(f64::total_cmp);
            ensure!(xs.len() % 2 == 0, "Invalid scanline parity");
            let offset = (row + first) * n;
            for pair in xs.chunks_exact(2) {
                pixels[offset+sample_index(pair[0])..offset+sample_index(pair[1])].fill(*code);
            }
        }
    }
    ensure!(pixels.iter().all(|&p| p < 5), "Reference polygons do not cover the preview");
    let mut rectangles: [Vec<geos::Geometry<'_>>; 5] = std::array::from_fn(|_| Vec::new());
    for row in 0..side {
        let mut classes = Vec::with_capacity(side);
        for col in 0..side {
            let mut votes = [0usize; 5];
            for y in row*SUB..(row+1)*SUB {
                for &class in &pixels[y*n+col*SUB..y*n+(col+1)*SUB] {
                    votes[class as usize] += 1;
                }
            }
            let water = votes[3] + votes[4];
            let dry = (0..3).max_by_key(|&c| (votes[c], std::cmp::Reverse(c))).unwrap();
            // Water votes count together. Retain the sea flag until the
            // underlay step so water visibility still reveals the correct base.
            classes.push(if water > votes[dry] { if votes[4] >= votes[3] {4} else {3} } else {dry});
        }
        let mut start = 0;
        while start < side {
            let code = classes[start];
            let end = (start+1..side).find(|&i| classes[i] != code).unwrap_or(side);
            rectangles[code].push(api.rectangle(
                start as f64 * cell, row as f64 * cell,
                end as f64 * cell, (row+1) as f64 * cell,
            )?);
            start = end;
        }
    }
    let mut output = Vec::new();
    for (code, group) in rectangles.into_iter().enumerate() {
        for poly in api.collection(&group)?.union()?.polygons()? {
            output.push((code as u8, poly.rings()?));
        }
    }
    Ok(output)
}

fn display_class_winners(
    api: &Geos,
    layers: &[geos::Geometry<'_>],
    domain_side: f64,
    side: usize,
) -> Result<Vec<usize>> {
    let cell = domain_side / side as f64;
    let mut result = vec![0; side * side];
    for y0 in (0..side).step_by(16) {
        for x0 in (0..side).step_by(16) {
            let x1 = (x0 + 16).min(side);
            let y1 = (y0 + 16).min(side);
            let block = api.rectangle(x0 as f64 * cell, y0 as f64 * cell,
                                      x1 as f64 * cell, y1 as f64 * cell)?;
            let local = layers.iter().map(|layer| layer.intersection(&block))
                .collect::<Result<Vec<_>>>()?;
            for y in y0..y1 {
                for x in x0..x1 {
                    let pixel = api.rectangle(x as f64 * cell, y as f64 * cell,
                                              (x + 1) as f64 * cell, (y + 1) as f64 * cell)?;
                    let mut areas = [0.; 4];
                    for (i, layer) in local.iter().enumerate() {
                        areas[i + 1] = layer.intersection(&pixel)?.area()?;
                    }
                    areas[0] = (cell * cell - areas[1..].iter().sum::<f64>()).max(0.);
                    result[y * side + x] = (0..4).max_by(|&a, &b|
                        areas[a].total_cmp(&areas[b]).then_with(|| b.cmp(&a))).unwrap();
                }
            }
        }
    }
    Ok(result)
}

fn fill_display_forest_holes<'a>(
    api: &'a Geos,
    layers: &[geos::Geometry<'a>],
    cell: f64,
) -> Result<geos::Geometry<'a>> {
    let pixel_area = cell * cell;
    let covered = api.collection(&[api.read(&layers[1].write()?)?,
                                   api.read(&layers[2].write()?)?])?.union()?;
    let visible_forest = layers[0].difference(&covered)?;
    let visible_gravel = layers[1].difference(&layers[2])?;
    let forest_pixels = PixelAreaIndex::new(&visible_forest)?;
    let gravel_pixels = PixelAreaIndex::new(&visible_gravel)?;
    let water_pixels = PixelAreaIndex::new(&layers[2])?;
    let mut forest_by_pixel = HashMap::new();
    let winner = |areas: [f64; 4]| (0..4).max_by(|&a, &b|
        areas[a].total_cmp(&areas[b]).then_with(|| b.cmp(&a))).unwrap();
    let mut parts = Vec::new();
    let mut filled = 0;
    for part in layers[0].polygons()? {
        let mut retained = Vec::new();
        for (index, ring) in part.rings()?.into_iter().enumerate() {
            if index == 0 {
                retained.push(ring);
                continue;
            }
            let hole = api.read(&wkb_poly(&[ring.clone()]))?;
            if hole.area()? >= pixel_area
                || hole.intersection(&layers[0])?.area()? > 1e-6
                || hole.intersection(&layers[1])?.area()? > 1e-6
                || hole.intersection(&layers[2])?.area()? > 1e-6
            {
                retained.push(ring);
                continue;
            }
            let bounds = ring_bounds(&ring);
            let lo = bounds.lower().map(|v| (v / cell).floor() as i32);
            let hi = bounds.upper().map(|v| (v / cell).ceil() as i32);
            let mut changes = Vec::new();
            let mut same = true;
            for y in lo[1]..hi[1] {
                for x in lo[0]..hi[0] {
                    let a = x as f64 * cell;
                    let b = y as f64 * cell;
                    let pixel = api.rectangle(a, b, a + cell, b + cell)?;
                    let gain = hole.intersection(&pixel)?.area()?;
                    if gain <= 1e-6 {
                        continue;
                    }
                    let [forest, gravel, water] = match forest_by_pixel.get(&(x,y)) {
                        Some(&areas) => areas,
                        None => [
                            forest_pixels.area(&pixel, a, b, cell)?,
                            gravel_pixels.area(&pixel, a, b, cell)?,
                            water_pixels.area(&pixel, a, b, cell)?,
                        ],
                    };
                    let background = (pixel_area - forest - gravel - water).max(0.);
                    same &= winner([background, forest, gravel, water])
                        == winner([(background - gain).max(0.), forest + gain, gravel, water]);
                    changes.push(((x,y), [forest + gain, gravel, water]));
                }
            }
            if same {
                forest_by_pixel.extend(changes);
                filled += 1;
            } else {
                retained.push(ring);
            }
        }
        parts.push(api.read(&wkb_poly(&retained))?);
    }
    println!("Subpixel forest holes filled: {filled}");
    api.collection(&parts)?.union()
}
fn simplify_display_forest<'a>(
    api: &'a Geos,
    layers: &[geos::Geometry<'a>],
    cell: f64,
    close_gaps: bool,
) -> Result<geos::Geometry<'a>> {
    let forest = &layers[0];
    let other = api.collection(&[api.read(&layers[1].write()?)?,
                                api.read(&layers[2].write()?)?])?.union()?;
    let visible_forest = forest.difference(&other)?;
    let visible_gravel = layers[1].difference(&layers[2])?;
    let forest_pixels = PixelAreaIndex::new(&visible_forest)?;
    let gravel_pixels = PixelAreaIndex::new(&visible_gravel)?;
    let water_pixels = PixelAreaIndex::new(&layers[2])?;
    let candidate = if close_gaps {
        let closed = forest.buffer(cell / 4., 1)?.buffer(-cell / 4., 1)?;
        let added = closed.difference(forest)?.difference(&other)?;
        api.collection(&[api.read(&forest.write()?)?, added])?.union()?
    } else {
        forest.topology_simplify(cell * 0.375)?.difference(&other)?
    };
    let pixel_area = cell * cell;
    let winner = |areas: [f64; 4]| (0..4).max_by(|&a, &b|
        areas[a].total_cmp(&areas[b]).then_with(|| b.cmp(&a))).unwrap();
    let mut pixel_areas = HashMap::new();
    let mut additions = Vec::new();
    let mut removals = Vec::new();
    for (direction, difference) in [(1., candidate.difference(forest)?),
                                    (-1., forest.difference(&candidate)?)] {
        for patch in difference.polygons()? {
            if patch.is_empty()? { continue; }
            if close_gaps && !(pixel_area / 50. ..pixel_area).contains(&patch.area()?) { continue; }
            let bounds = ring_bounds(&patch.rings()?[0]);
            let lo = bounds.lower().map(|v| (v / cell).floor() as i32);
            let hi = bounds.upper().map(|v| (v / cell).ceil() as i32);
            let mut updates = Vec::new();
            let mut same = true;
            for y in lo[1]..hi[1] {
                for x in lo[0]..hi[0] {
                    let left = x as f64 * cell;
                    let bottom = y as f64 * cell;
                    let pixel = api.rectangle(left, bottom, left + cell, bottom + cell)?;
                    let delta = direction * patch.intersection(&pixel)?.area()?;
                    if delta.abs() <= 1e-6 {
                        continue;
                    }
                    let [f, g, w] = match pixel_areas.get(&(x, y)) {
                        Some(&areas) => areas,
                        None => [forest_pixels.area(&pixel, left, bottom, cell)?,
                                 gravel_pixels.area(&pixel, left, bottom, cell)?,
                                 water_pixels.area(&pixel, left, bottom, cell)?],
                    };
                    let bg = (pixel_area - f - g - w).max(0.);
                    same &= winner([bg, f, g, w]) == winner([(bg - delta).max(0.), f + delta, g, w]);
                    updates.push(((x, y), [f + delta, g, w]));
                }
            }
            if same {
                pixel_areas.extend(updates);
                if direction > 0. { additions.push(patch); } else { removals.push(patch); }
            }
        }
    }
    additions.push(forest.difference(&api.collection(&removals)?.union()?)?);
    api.collection(&additions)?.union()
}
fn display_class_lod(
    api: &Geos,
    domain_side: f64,
    input: &[(u8, Vec<Vec<P>>)],
    side: usize,
) -> Result<Vec<(u8, Vec<Vec<P>>)>> {

    let tolerance = domain_side / side as f64 / 32.;
    let domain = api.rectangle(0., 0., domain_side, domain_side)?;
    let layer = |code| -> Result<geos::Geometry<'_>> {
        let parts = input.iter().filter(|(class, _)| *class == code)
            .map(|(_, rings)| api.read(&wkb_poly(rings))).collect::<Result<Vec<_>>>()?;
        api.collection(&parts)?.union()
    };
    let layers = |tolerance: f64| -> Result<Vec<geos::Geometry<'_>>> {
        let water = layer(3)?.topology_simplify(tolerance)?.intersection(&domain)?;
        let gravel = layer(2)?.topology_simplify(tolerance)?.intersection(&domain)?
            .difference(&water)?;
        let covered = api.collection(&[api.read(&water.write()?)?,
                                       api.read(&gravel.write()?)?])?.union()?;
        let forest = layer(1)?.topology_simplify(tolerance)?.intersection(&domain)?
            .difference(&covered)?;
        Ok(vec![forest, gravel, water])
    };
    let before = display_class_winners(api, &layers(0.)?, domain_side, side)?;
    let mut simplified = layers(tolerance)?;
    if side == 128 {
        let cell = domain_side / side as f64;
        simplified[0] = fill_display_forest_holes(api, &simplified, cell)?;
        simplified[0] = simplify_display_forest(api, &simplified, cell, false)?;
    }
    let after = display_class_winners(api, &simplified, domain_side, side)?;
    let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
    println!("Continuous outline: {tolerance} m simplification, {changed} display class changes");
    let background = domain.difference(&api.collection(&simplified)?.union()?)?;
    let mut result = Vec::new();
    for (code, layer) in std::iter::once(background).chain(simplified).enumerate() {
        for poly in layer.polygons()? {
            result.push((code as u8, poly.rings()?));
        }
    }
    Ok(result)
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    source_root: Option<PathBuf>,
    #[arg(long, conflicts_with = "source_root")]
    tile_source: Vec<PathBuf>,
    #[arg(long, conflicts_with_all = ["tile_source", "source_root"], requires_all = ["seam_keys", "only_level"])]
    seam_source: Vec<PathBuf>,
    #[arg(long, value_delimiter = ',')]
    seam_keys: Vec<u32>,
    #[arg(long, requires = "seam_source")]
    seam_report_only: bool,
    #[arg(long, requires = "national_plan", conflicts_with_all = ["tile_source", "seam_source", "source_root"])]
    national_regions: Option<PathBuf>,
    #[arg(long, requires = "national_regions")]
    national_plan: Option<PathBuf>,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    geos: PathBuf,
    #[arg(long, value_delimiter = ',', default_value = "majority,regions")]
    filters: Vec<String>,
    #[arg(long)]
    geometric_fit: bool,
    #[arg(long)]
    fair_boundary: bool,
    #[arg(long)]
    source_levels: bool,
    #[arg(long)]
    only_level: Option<usize>,
    #[arg(long, requires_all = ["unified_water", "implicit_background", "source_levels"])]
    fine_coastal_details: bool,
    #[arg(long)]
    water_overlay: bool,
    #[arg(long, requires = "water_overlay")]
    unified_water: bool,
    #[arg(long)]
    compact: bool,
    #[arg(long, requires = "compact")]
    implicit_background: bool,
    #[arg(long, requires = "unified_water", conflicts_with = "pixel_side")]
    water_pixel_visibility: bool,
    #[arg(long, requires = "water_pixel_visibility")]
    keep_thin_inland_water: bool,
    #[arg(long, requires = "unified_water")]
    coast_tolerance: Option<f64>,
    #[arg(long, requires = "coast_tolerance")]
    coast_source_band: Option<f64>,
    #[arg(long, requires_all = ["unified_water", "implicit_background", "coast_pixel_visibility"])]
    independent_shore: Option<f64>,
    #[arg(long, requires = "independent_shore")]
    final_coast_simplify: Option<f64>,
    #[arg(long, requires = "coast_tolerance")]
    generalize_coast: bool,
    #[arg(long, requires = "generalize_coast", conflicts_with = "pixel_side")]
    coast_pixel_visibility: bool,
    #[arg(long, requires = "unified_water", conflicts_with = "coast_tolerance", value_parser = clap::value_parser!(u16).range(1..=512))]
    pixel_side: Option<u16>,
    #[arg(long, requires_all = ["source_levels", "implicit_background"], conflicts_with_all = ["pixel_side", "water_overlay", "generalize_coast"])]
    display_classes: bool,
    #[arg(long, requires_all = ["source_levels", "implicit_background", "only_level"], conflicts_with_all = ["display_classes", "pixel_side", "water_overlay", "generalize_coast"])]
    linear_source: bool,
    #[arg(long, conflicts_with_all = ["fair_boundary", "geometric_fit", "linear_source", "display_classes"])]
    direct_lines: bool,
    #[arg(long, requires_all = ["direct_lines", "unified_water", "implicit_background", "source_levels", "only_level"], conflicts_with_all = ["fine_coastal_details", "independent_shore", "generalize_coast", "water_pixel_visibility", "pixel_side"])]
    common_lod: bool,
    #[arg(long, requires = "linear_source")]
    common_finish: bool,
}
const COMMON_EVALUATION_DISTANCES_M: [f64; 4] = [125., 500., 1000., 8000.];

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(!args.common_lod || matches!(args.only_level, Some(3..=6)), "Common landcover requires L3–L6");
    ensure!(!args.display_classes || matches!(args.only_level, Some(5 | 6)), "Display classes require L5 or L6");
    ensure!(args.final_coast_simplify.is_none_or(|v| v.is_finite() && v > 0.), "Invalid final coast tolerance");
    ensure!(args.independent_shore.is_none_or(|v| v.is_finite() && v > 0.), "Invalid shore tolerance");
    ensure!(args.coast_source_band.is_none_or(|v| v.is_finite() && v > 0.), "Invalid coast source band");
    ensure!(args.coast_tolerance.is_none_or(|v| v.is_finite() && v > 0.), "Invalid coast tolerance");
    ensure!(
        !args.output.join("manifest.json").exists(),
        "Use a new output"
    );
    fs::create_dir_all(&args.output)?;
    let api = Geos::open(&args.geos)?;
    if let (Some(regions), Some(plan)) = (&args.national_regions, &args.national_plan) {
        return landcover_xyz::national::generate(&api, regions, plan, &args.output);
    }
    if !args.seam_source.is_empty() {
        return landcover_xyz::reconcile_pair(&api, &args.seam_source, &args.seam_keys, args.only_level.unwrap(), &args.output, args.seam_report_only);
    }
    if !args.tile_source.is_empty() {
        return landcover_xyz::generate(&api, &args.tile_source, &args.output);
    }
    let source_root = args.source_root.context("Specify --source-root or --tile-source")?;
    let start = Instant::now();
    let mut summaries = Vec::new();
    ensure!(
        args.filters
            .iter()
            .all(|f| ["majority", "regions", "majority-clean"].contains(&f.as_str())),
        "Unknown filter"
    );
    for filter in args.filters {
        let filter = filter.as_str();
        let source = source_root.join(filter);
        let out = args.output.join(filter);
        fs::create_dir_all(&out)?;
        let mut manifest: Value = serde_json::from_slice(&fs::read(source.join("manifest.json"))?)?;
        let bounds: [f64; 4] = serde_json::from_value(manifest["bounds"].clone())?;
        let domain_side = bounds[2] - bounds[0];
        ensure!(domain_side > 0. && ((bounds[3] - bounds[1]) - domain_side).abs() < 1e-6, "Expected square extent");
        let source_db = Connection::open_with_flags(
            source.join("landcover-vectors.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let db = Connection::open(out.join("landcover-vectors.sqlite3"))?;
        db.execute_batch("PRAGMA page_size=512;CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID;CREATE TABLE features(level INTEGER,class INTEGER,geometry BLOB,PRIMARY KEY(level,class));")?;
        let mut rows = Vec::new();
        let tolerances: [f64; 7] = [50., 62.5, 100., 200., 500., 2000., 4000.];
        for (l, &tolerance) in tolerances.iter().enumerate() {
            let skip_level = if let Some(level) = args.only_level {
                l != level
            } else if args.fine_coastal_details {
                l != 3
            } else {
                l >= 5
            };
            if skip_level {
                continue;
            }
            let source_level = if args.display_classes || args.linear_source { l } else if args.source_levels { l.min(5) } else { 0 };
            let mut input = Vec::new();
            let mut source_vertices = 0;
            for code in 0..5 {
                let packed: Vec<u8> = source_db.query_row(
                    "SELECT geometry FROM features WHERE level=? AND class=?",
                    params![source_level as i32, code],
                    |r| r.get(0),
                )?;
                let mut raw = Vec::new();
                ZlibDecoder::new(&packed[..]).read_to_end(&mut raw)?;
                for poly in api.read(&raw)?.polygons()? {
                    let rings = poly.rings()?;
                    source_vertices += rings.iter().map(Vec::len).sum::<usize>();
                    input.push((code as u8, rings));
                }
            }
            if args.common_finish {
                let cell = COMMON_EVALUATION_DISTANCES_M[l.saturating_sub(3)];
                let mut layers = Vec::new();
                for code in 1..=3 {
                    let parts = input.iter().filter(|(class, _)| *class == code)
                        .map(|(_, rings)| api.read(&wkb_poly(rings)))
                        .collect::<Result<Vec<_>>>()?;
                    layers.push(api.collection(&parts)?.union()?);
                }
                let water_tolerance: f64 = [0., 0., 0., 750.][l.saturating_sub(3)];
                if water_tolerance > 0. {
                    let extent = api.rectangle(0., 0., domain_side, domain_side)?;
                    let border = extent.boundary()?;
                    let mut water_parts = Vec::new();
                    for part in layers[2].polygons()? {
                        let touches_border = part.intersects(&border)?;
                        if !touches_border && part.area()? < water_tolerance.powi(2) {
                            continue;
                        }
                        let original_edge = part.intersection(&border)?;
                        let mut selected = None;
                        for distance in [water_tolerance, water_tolerance / 2., water_tolerance / 4.] {
                            let candidate = part.topology_simplify(distance)?;
                            let edge = candidate.intersection(&border)?;
                            if original_edge.difference(&edge)?.length()? > 1e-6
                                || edge.difference(&original_edge)?.length()? > 1e-6
                            {
                                continue;
                            }
                            let mut excessive = false;
                            for gain in candidate.difference(&part)?.polygons()? {
                                if gain.area()? >= water_tolerance.powi(2) {
                                    excessive = true;
                                    break;
                                }
                            }
                            if !excessive {
                                selected = Some(candidate);
                                break;
                            }
                        }
                        water_parts.push(selected.unwrap_or(part));
                    }
                    layers[2] = api.collection(&water_parts)?.union()?;
                    input.retain(|(code, _)| *code != 3);
                    for part in layers[2].polygons()? {
                        input.push((3, part.rings()?));
                    }
                }
                layers[0] = fill_display_forest_holes(&api, &layers, cell)?;
                let extent = api.rectangle(0., 0., domain_side, domain_side)?;
                let forest = simplify_display_forest(&api, &layers, cell, true)?.intersection(&extent)?;
                input.retain(|(code, _)| *code != 1);
                for part in forest.polygons()? {
                    input.push((1, part.rings()?));
                }
            }
            if args.display_classes {
                input = display_class_lod(&api, domain_side, &input, if l == 5 { 256 } else { 128 })?;
            }
            if let Some(side) = args.pixel_side {
                input = pixel_lod(&api, domain_side, &input, side as usize)?;
            }
            println!("{filter} L{l}: loaded {} polygons", input.len());
            let mut coast_removed = None;
            if args.generalize_coast {
                let (updated, counts) = generalize_coast(&api, domain_side, input, args.coast_pixel_visibility)?;
                input = updated;
                coast_removed = Some(counts);
                println!("L{l} coastal generalization: {} islands, {} inlets", counts[0], counts[1]);
            }
            let evaluation_cell = COMMON_EVALUATION_DISTANCES_M[l.saturating_sub(3)];
            let water_evaluation_cell = evaluation_cell.min(2000.);
            let inland_water_coverage = [0.5, 1., 1., 1.][l.saturating_sub(3)];
            let common_line_tolerance = tolerance.min(evaluation_cell / 8.);
            let coastal_tolerance = match l {
                3 => 31.25,
                _ => 180. * (water_evaluation_cell / 500.),
            };
            let (inland_water_cell, inland_water_min_coverage) = if args.common_lod {
                (water_evaluation_cell, inland_water_coverage)
            } else if l >= 5 {
                (domain_side / f64::from(256 >> (l - 5)), 1.0)
            } else {
                (500., if l == 4 && domain_side == 256000. { 1.0 } else { 0.16 })
            };
            let mut water_removed = None;
            if args.water_pixel_visibility {
                let (updated, counts) = generalize_inland_water(&api, domain_side, inland_water_cell, input)?;
                input = updated;
                water_removed = Some(counts);
            }
            let mut omitted_bare_ground = None;
            if args.implicit_background && (args.water_pixel_visibility || args.common_lod || (args.fine_coastal_details && l == 3)) {
                let cell = if args.common_lod { evaluation_cell } else if args.fine_coastal_details && l == 3 { domain_side / 2048. } else { 500. };
                let (updated, omitted) = omit_small_bare_ground(&api, input, cell)?;
                input = updated;
                omitted_bare_ground = Some(omitted);
            }
            if args.coast_pixel_visibility && args.implicit_background {
                let shapes = input.iter().map(|(_, rings)| api.read(&wkb_poly(rings)))
                    .collect::<Result<Vec<_>>>()?;
                let simplified = api.collection(&shapes)?.coverage_simplify(250.)?.polygons()?;
                ensure!(simplified.len() == input.len(), "Coastal simplification changed face count");
                for ((_, rings), part) in input.iter_mut().zip(simplified) {
                    *rings = part.rings()?;
                }
            }
            let mut filled_forest_gaps = None;
            if args.water_pixel_visibility && args.implicit_background {
                let (updated, count) = fill_small_forest_gaps(&api, input, None)?;
                input = updated;
                filled_forest_gaps = Some(count);
            }
            if args.coast_pixel_visibility && args.implicit_background && args.generalize_coast {
                let (updated, second) = generalize_coast(&api, domain_side, input, true)?;
                let (dry, water): (Vec<_>, Vec<_>) = updated.into_iter().partition(|(code, _)| *code < 3);
                let water_shapes = water.iter().map(|(_, rings)| api.read(&wkb_poly(rings)))
                    .collect::<Result<Vec<_>>>()?;
                let water_mask = api.collection(&water_shapes)?.union()?;
                let (mut merged, count) = merge_coastal_faces(&api, dry, &water_mask, true)?;
                merged.extend(water);
                input = merged;
                if let Some(first) = &mut coast_removed {
                    first[2] += count;
                    for (total, extra) in first.iter_mut().zip(second) {
                        *total += extra;
                    }
                }
            }
            let coast = args.coast_tolerance.map(|_| RTree::bulk_load(
                input.iter().filter(|(code, _)| *code == 4)
                    .flat_map(|(_, rings)| rings.iter().flat_map(|ring| {
                        ring.windows(2).map(|w| IndexedEdge { a: w[0], b: w[1], chain: 0 })
                    })).collect(),
            ));
            let mut overlay_area = 0.;
            let mut closed_water_breaks = None;
            if args.water_overlay {
                input = water_underlay(&api, input)?;
                if args.unified_water {
                    let mut water = Vec::new();
                    let mut sea = Vec::new();
                    for (code, rings) in &input {
                        if *code >= 3 {
                            let geom = api.read(&wkb_poly(rings))?;
                            if *code == 3 {
                                overlay_area += geom.area()?;
                            } else if args.common_lod || args.independent_shore.is_some() || (args.fine_coastal_details && l == 3) {
                                sea.push(api.read(&geom.write()?)?);
                            }
                            water.push(geom);
                        }
                    }
                    input.retain(|(code, _)| *code < 3);
                    let mut water = api.collection(&water)?.union()?;
                    if args.coast_pixel_visibility && args.implicit_background {
                        if let Some(coast) = &coast {
                            water = absorb_coastal_background(&api, &input, water, coast, 400., 250.)?;
                        }
                    }
                    if let Some(tolerance) = args.independent_shore {
                        let sea = api.collection(&sea)?.union()?;
                        let cleanup_coast = if args.water_pixel_visibility { coast.as_ref() } else { None };
                        (input, water, closed_water_breaks) =
                            independent_shore(&api, domain_side, input, water, &sea, tolerance, cleanup_coast, inland_water_cell, inland_water_min_coverage, args.keep_thin_inland_water, args.final_coast_simplify, (l >= 5).then_some(inland_water_cell), None)?;
                        if cleanup_coast.is_some() {
                            let (updated, count) = fill_small_forest_gaps(&api, input, Some(&water))?;
                            input = updated;
                            filled_forest_gaps = Some(filled_forest_gaps.unwrap_or(0) + count);
                        }
                    }
                    if args.common_lod {
                        let extent = api.rectangle(0., 0., domain_side, domain_side)?;
                        water = omit_subpixel_coastal_details(&api, water, &extent, water_evaluation_cell)?;
                        let sea = api.collection(&sea)?.union()?;
                        let border = extent.boundary()?;
                        let mut visible = Vec::new();
                        for part in water.polygons()? {
                            if part.intersects(&sea)? || part.intersects(&border)?
                                || covers_pixel_fraction(
                                    &api, &part, ring_bounds(&part.rings()?[0]),
                                    water_evaluation_cell, inland_water_coverage,
                                )?
                            {
                                visible.push(part);
                            }
                        }
                        water = api.collection(&visible)?.union()?;
                        (input, water, _) = independent_shore(
                            &api, domain_side, input, water, &sea, water_evaluation_cell / 4., None,
                            water_evaluation_cell, inland_water_coverage, true, Some(coastal_tolerance),
                            Some(water_evaluation_cell), Some(evaluation_cell / 16.),
                        )?;
                    }
                    if args.fine_coastal_details && l < 4 {
                        let cell = domain_side / f64::from(16384 >> l);
                        let extent = api.rectangle(0., 0., domain_side, domain_side)?;
                        water = omit_subpixel_coastal_details(&api, water, &extent, cell)?;
                        if l == 3 {
                            let sea = api.collection(&sea)?.union()?;
                            (input, water, _) = independent_shore(
                                &api, domain_side, input, water, &sea, cell / 4., None,
                                cell, 0.5, true, None, None, Some(cell / 16.),
                            )?;
                        }
                        let mut dry = input.iter().map(|(_, rings)| api.read(&wkb_poly(rings)))
                            .collect::<Result<Vec<_>>>()?;
                        dry.push(api.read(&water.write()?)?);
                        let missing = extent.difference(&api.collection(&dry)?.union()?)?;
                        if !missing.is_empty()? {
                            let mut background = input.iter().filter(|(code, _)| *code == 0)
                                .map(|(_, rings)| api.read(&wkb_poly(rings)))
                                .collect::<Result<Vec<_>>>()?;
                            background.push(missing);
                            input.retain(|(code, _)| *code != 0);
                            for part in api.collection(&background)?.union()?.polygons()? {
                                input.push((0, part.rings()?));
                            }
                        }
                    }
                    for poly in water.polygons()? {
                        input.push((3, poly.rings()?));
                    }
                }
                source_vertices = input.iter().flat_map(|(_, rings)| rings).map(Vec::len).sum();
            }
            let mut g = build_graph(input, args.compact, args.implicit_background && !args.display_classes && !args.linear_source, (args.fine_coastal_details && l == 3) || args.common_lod)?;
            let mut edge_sides = vec![[0u8; 2]; g.edges.len()];
            if coast.is_some() {
                for poly in &g.polys {
                    for (i, ring) in poly.rings.iter().enumerate() {
                        let origin = g.points[ring[0]];
                        let mut winding = 0.;
                        for j in 0..ring.len() {
                            let a = sub(g.points[ring[j]], origin);
                            let b = sub(g.points[ring[(j+1)%ring.len()]], origin);
                            winding += a[0]*b[1] - a[1]*b[0];
                        }
                        let left = (winding > 0.) == (i == 0);
                        for j in 0..ring.len() {
                            let a = ring[j];
                            let b = ring[(j+1)%ring.len()];
                            let id = g.edge_ids[&edge_key(a,b)];
                            let side = usize::from(left != (a == g.edges[id].a));
                            edge_sides[id][side] |= 1 << poly.code;
                        }
                    }
                }
            }
            let mut coastal = vec![false; g.chains.len()];
            let mut coast_runs = vec![Vec::new(); g.chains.len()];
            let coast_distance2 = args.coast_source_band.unwrap_or(0.0001).powi(2);
            if let Some(coast) = coast.as_ref().filter(|_| args.independent_shore.is_none()) {
                for (id, chain) in g.chains.iter_mut().enumerate() {
                    let classes = g.edges[g.edge_ids[&edge_key(chain.nodes[0], chain.nodes[1])]].classes;
                    if classes & 8 == 0 || (classes & 7).count_ones() != 1 {
                        continue;
                    }
                    let eligible: Vec<_> = chain.nodes.windows(2).map(|w| {
                        let sides = edge_sides[g.edge_ids[&edge_key(w[0],w[1])]];
                        if !((sides[0] == 8 && sides[1] & 7 != 0) || (sides[1] == 8 && sides[0] & 7 != 0)) {
                            return false;
                        }
                        let a = g.points[w[0]];
                        let b = g.points[w[1]];
                        [a, mul(add(a, b), 0.5), b].into_iter().all(|p| {
                            coast.nearest_neighbor(&p).is_some_and(|edge| edge.distance_2(&p) < coast_distance2)
                        })
                    }).collect();
                    coastal[id] = if args.coast_source_band.is_some() {
                        eligible.iter().any(|&v| v)
                    } else {
                        eligible.iter().all(|&v| v)
                    };
                    if coastal[id] {
                        let mut first = 0;
                        while first < eligible.len() {
                            let end = (first+1..eligible.len())
                                .find(|&i| eligible[i] != eligible[first]).unwrap_or(eligible.len());
                            coast_runs[id].push((first, end, eligible[first]));
                            first = end;
                        }
                        chain.frozen = false;
                    }
                }
            }
            let mut refs = g
                .polys
                .iter()
                .map(|p| {
                    p.rings
                        .iter()
                        .map(|r| references(&g, r))
                        .collect::<Result<Vec<_>>>()
                })
                .collect::<Result<Vec<_>>>()?;
            println!(
                "{filter}: {} shared chains, {} source coordinates",
                g.chains.len(),
                source_vertices
            );
            if args.source_levels {
                let originals: Vec<_> = g
                    .chains
                    .iter()
                    .map(|c| c.nodes.iter().map(|&i| g.points[i]).collect::<Vec<_>>())
                    .collect();
                for (i, rr) in refs.iter().enumerate() {
                    let reconstructed = wkb_poly(&rings_from(rr, &originals));
                    if !api.read(&reconstructed)?.is_valid()? {
                        let base: Vec<_> = g.polys[i]
                            .rings
                            .iter()
                            .map(|r| {
                                let mut p: Vec<_> = r.iter().map(|&j| g.points[j]).collect();
                                p.push(p[0]);
                                p
                            })
                            .collect();
                        fs::write(out.join("invalid-source-reconstructed.wkb"), &reconstructed)?;
                        fs::write(out.join("invalid-source-noded.wkb"), wkb_poly(&base))?;
                        anyhow::bail!(
                            "Invalid source reconstruction L{l} polygon{i} class{}",
                            g.polys[i].code
                        );
                    }
                }
            }
            let now = Instant::now();
            let mut curves = Vec::new();
            let mut max_error = 0.;
            let fitter: fn(
                &[P],
                P,
                P,
                f64,
                usize,
                Option<&RTree<IndexedEdge>>,
                &mut Vec<Curve>,
                &mut f64,
            ) = if args.fair_boundary {
                fit_path_faired
            } else if args.geometric_fit {
                fit_path_geometric
            } else {
                fit_path
            };
            let water_path: Vec<_> = g.chains.iter().map(|chain| {
                let edge = g.edge_ids[&edge_key(chain.nodes[0], chain.nodes[1])];
                g.edges[edge].classes & 8 != 0
            }).collect();
            let mut collinear_changed = vec![false; g.chains.len()];
            for (id, c) in g.chains.iter().enumerate() {
                let pts: Vec<_> = c.nodes.iter().map(|&i| g.points[i]).collect();
                let mut fit = Vec::new();
                if coastal[id] {
                    for &(first, end, eligible) in &coast_runs[id] {
                        let part = &pts[first..=end];
                        if eligible {
                            fit.extend(simplify_coast(part, args.coast_tolerance.unwrap(), id, &g.tree, args.coast_pixel_visibility));
                        } else {
                            fit.extend(part.windows(2).map(|w| Curve::line(w[0], w[1])));
                        }
                    }
                } else if c.frozen || args.pixel_side.is_some() || args.display_classes || args.linear_source {
                    for p in pts.windows(2) {
                        fit.push(Curve::line(p[0], p[1]))
                    }
                } else if args.direct_lines {
                    let line_tolerance = if args.common_lod {
                        common_line_tolerance
                    } else {
                        let cell = domain_side / f64::from(2048 >> l.saturating_sub(3));
                        tolerance.min(cell / 8.)
                    };
                    fit = simplify_coast(&pts, line_tolerance, id, &g.tree, false);
                } else {
                    let left = unit(sub(pts[2.min(pts.len() - 1)], pts[0]));
                    let right = unit(sub(pts[pts.len().saturating_sub(3)], *pts.last().unwrap()));
                    fitter(
                        &pts,
                        left,
                        right,
                        tolerance,
                        id,
                        Some(&g.tree),
                        &mut fit,
                        &mut max_error,
                    );
                }
                if args.compact {
                    let stored_coordinates: usize = fit.iter().map(|c| if c.line { 1 } else { 3 }).sum();
                    if stored_coordinates > pts.len() - 1 {
                        fit = pts.windows(2).map(|p| Curve::line(p[0], p[1])).collect();
                    }
                }
                if args.implicit_background && (water_path[id] || args.linear_source) {
                    let before = fit.len();
                    let joined = join_collinear_lines(fit);
                    collinear_changed[id] = joined.len() < before;
                    fit = joined;
                }
                curves.push(fit);
            }
            let mut fallback = HashSet::new();
            let mut straight_retry = HashSet::new();
            let mut final_coords = Vec::new();
            let mut final_wkb = Vec::new();
            for pass in 0..=g.chains.len() {
                let samples: Vec<_> = curves
                    .iter()
                    .map(|c| {
                        let mut p = vec![c[0].p[0]];
                        for v in c {
                            flatten(v, 0.5, &mut p, 0)
                        }
                        p
                    })
                    .collect();
                final_coords.clear();
                final_wkb.clear();
                let mut bad = HashSet::new();
                let render_tree = RTree::bulk_load(
                    samples
                        .iter()
                        .enumerate()
                        .flat_map(|(id, p)| {
                            p.windows(2).map(move |w| IndexedEdge {
                                a: w[0],
                                b: w[1],
                                chain: id,
                            })
                        })
                        .collect(),
                );
                // Detect all proper crossings in the rendered boundary graph in
                // one pass. A single large polygon can contain many bad locations;
                // asking GEOS for only its first error caused serial full rechecks.
                // A fairing-adjusted path can contain only straight segments while
                // still being different from the source. Tags alone are not a
                // mutability test.
                let mutable: Vec<_> = g
                    .chains
                    .iter()
                    .enumerate()
                    .map(|(id, c)| args.pixel_side.is_none() && (!c.frozen || collinear_changed[id]) && !fallback.contains(&id))
                    .collect();
                let orient = |a: P, b: P, c: P| {
                    robust::orient2d(
                        robust::Coord { x: a[0], y: a[1] },
                        robust::Coord { x: b[0], y: b[1] },
                        robust::Coord { x: c[0], y: c[1] },
                    )
                };
                let opposite = |a: f64, b: f64| (a < 0. && b > 0.) || (a > 0. && b < 0.);
                for edge in render_tree.iter() {
                    if !mutable[edge.chain] {
                        continue;
                    }
                    for other in render_tree.locate_in_envelope_intersecting(&edge.envelope()) {
                        if edge.a == other.a
                            || edge.a == other.b
                            || edge.b == other.a
                            || edge.b == other.b
                        {
                            continue;
                        }
                        if opposite(
                            orient(edge.a, edge.b, other.a),
                            orient(edge.a, edge.b, other.b),
                        ) && opposite(
                            orient(other.a, other.b, edge.a),
                            orient(other.a, other.b, edge.b),
                        ) {
                            bad.insert(edge.chain);
                            if mutable[other.chain] {
                                bad.insert(other.chain);
                            }
                        }
                    }
                }
                let mut bad_locations = Vec::new();
                for rr in &refs {
                    let rings = rings_from(rr, &samples);
                    let bytes = wkb_poly(&rings);
                    if bad.is_empty() {
                        if let Some((p, reason)) = api.read(&bytes)?.invalid_detail()? {
                            let allowed: HashSet<_> = if reason.contains("Hole lies outside shell")
                            {
                                rr[0]
                                    .iter()
                                    .map(|v| (v.unsigned_abs() - 1) as usize)
                                    .collect()
                            } else {
                                rr.iter()
                                    .flatten()
                                    .map(|v| (v.unsigned_abs() - 1) as usize)
                                    .collect()
                            };
                            if pass < 3 {
                                println!(
                                    "L{l} {reason} at {p:?}, {} possible paths",
                                    allowed.len()
                                );
                            }
                            bad_locations.push((p, Some(allowed)));
                        }
                    }

                    final_coords.push(rings);
                    final_wkb.push(bytes);
                }
                // Validate the complete class, including interactions between distinct islands.
                if bad_locations.is_empty() && bad.is_empty() {
                    for code in 0..5 {
                        let parts: Vec<_> = g
                            .polys
                            .iter()
                            .enumerate()
                            .filter_map(|(i, p)| {
                                if p.code == code {
                                    Some(final_wkb[i].clone())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if let Some((p, reason)) = api.read(&wkb_multi(&parts))?.invalid_detail()? {
                            if pass < 3 {
                                println!("L{l} class{code} {reason} at {p:?}");
                            }
                            bad_locations.push((p, None))
                        }
                    }
                }
                for (location, allowed) in &bad_locations {
                    // Restore only paths at the actual invalid point, not a whole continent's
                    // polygon and its thousands of holes.
                    let mut nearest = f64::INFINITY;
                    for edge in render_tree.nearest_neighbor_iter(location) {
                        let d = edge.distance_2(location);
                        if nearest.is_finite() && d > nearest + 1e-7 {
                            break;
                        }
                        if allowed.as_ref().is_none_or(|a| a.contains(&edge.chain))
                            && mutable[edge.chain]
                        {
                            nearest = nearest.min(d);
                            bad.insert(edge.chain);
                        }
                    }
                }
                if bad_locations.is_empty() && bad.is_empty() {
                    break;
                }
                ensure!(
                    !bad.is_empty(),
                    "Invalid geometry without any fitted path nearby"
                );
                println!(
                    "{filter} L{l} validation pass {pass}: {} fallback chains",
                    bad.len()
                );
                ensure!(
                    pass < g.chains.len(),
                    "Topology validation did not converge"
                );
                for id in bad {
                    let pts = &g.chains[id].nodes;
                    let source: Vec<_> = pts.windows(2)
                        .map(|w| Curve::line(g.points[w[0]], g.points[w[1]]))
                        .collect();
                    if args.implicit_background && water_path[id] && straight_retry.insert(id) {
                        let reduced = join_collinear_lines(source.clone());
                        if reduced.len() < source.len() {
                            curves[id] = reduced;
                            continue;
                        }
                    }
                    ensure!(fallback.insert(id), "Repeated fallback without progress");
                    curves[id] = source;
                }
            }
            if args.common_lod || (args.fine_coastal_details && l == 3) {
                let water_parts: Vec<_> = g.polys.iter().zip(&final_wkb)
                    .filter(|(poly, _)| poly.code == 3)
                    .map(|(_, wkb)| wkb.clone()).collect();
                let water = api.read(&wkb_multi(&water_parts))?;
                let cell = if args.common_lod { water_evaluation_cell } else { domain_side / 2048. };
                let mut pixel_water = HashMap::new();
                let mut filled = 0;
                for (i, poly) in g.polys.iter_mut().enumerate() {
                    if poly.code != 3 || final_coords[i].len() == 1 {
                        continue;
                    }
                    let mut retain = vec![true];
                    for ring in final_coords[i].iter().skip(1) {
                        let fill = fill_subpixel_water_hole(&api, ring, &water, cell, &mut pixel_water)?;
                        filled += usize::from(fill);
                        retain.push(!fill);
                    }
                    if retain.iter().any(|keep| !keep) {
                        let mut flags = retain.iter();
                        poly.rings.retain(|_| *flags.next().unwrap());
                        let mut flags = retain.iter();
                        refs[i].retain(|_| *flags.next().unwrap());
                        let mut flags = retain.iter();
                        final_coords[i].retain(|_| *flags.next().unwrap());
                        final_wkb[i] = wkb_poly(&final_coords[i]);
                    }
                }
                println!("Subpixel water holes filled: {filled}");
                let mut keep = Vec::with_capacity(g.polys.len());
                let mut omitted = 0;
                for (poly, wkb) in g.polys.iter().zip(&final_wkb) {
                    // 完成した表示形状で、水域の下に全体が隠れる砂礫地を省く。
                    let hidden = poly.code == 2 && water.covers(&api.read(wkb)?)?;
                    omitted += usize::from(hidden);
                    keep.push(!hidden);
                }
                let mut flags = keep.iter();
                g.polys.retain(|_| *flags.next().unwrap());
                let mut flags = keep.iter();
                refs.retain(|_| *flags.next().unwrap());
                let mut flags = keep.iter();
                final_coords.retain(|_| *flags.next().unwrap());
                let mut flags = keep.iter();
                final_wkb.retain(|_| *flags.next().unwrap());
                println!("Covered bare ground polygons omitted: {omitted}");
            }
            if (l == 6 || args.common_lod) && args.implicit_background && args.unified_water {
                let water_parts: Vec<_> = g.polys.iter().zip(&final_wkb)
                    .filter(|(poly, _)| poly.code == 3)
                    .map(|(_, wkb)| wkb.clone()).collect();
                let water = api.read(&wkb_multi(&water_parts))?;
                let forest_indices: Vec<_> = g.polys.iter().enumerate()
                    .filter_map(|(i, poly)| (poly.code == 1).then_some(i)).collect();
                let forest_parts: Vec<_> = forest_indices.iter().map(|&i| final_wkb[i].clone()).collect();
                let border = api.rectangle(0., 0., domain_side, domain_side)?.boundary()?;
                let forest_cell = if args.common_lod { evaluation_cell } else { inland_water_cell };
                let omitted = subpixel_forest_polygons(&api, &forest_parts, &water, &border, forest_cell)?;
                for &index in &omitted {
                    g.polys[forest_indices[index]].code = 0;
                }
                println!("Subpixel forest polygons omitted: {}", omitted.len());
            }
            if (l >= 5 || args.common_lod) && args.implicit_background && args.unified_water {
                let water_parts: Vec<_> = g.polys.iter().zip(&final_wkb)
                    .filter(|(poly, _)| poly.code == 3)
                    .map(|(_, wkb)| wkb.clone()).collect();
                let water = api.read(&wkb_multi(&water_parts))?;
                let mut shortened = 0;
                for (i, poly) in g.polys.iter().enumerate() {
                    if poly.code != 1 {
                        continue;
                    }
                    let forest = api.read(&final_wkb[i])?;
                    let mut coords = final_coords[i].clone();
                    let mut replacements = Vec::new();
                    for (r, path) in refs[i].iter().enumerate() {
                        if let Some(curve) = shorten_submerged_forest_ring(&api, path, &curves, &forest, &water, None)? {
                        let mut points = vec![curve[0].p[0]];
                            for segment in &curve {
                                flatten(segment, 0.5, &mut points, 0);
                            }
                            coords[r] = points;
                            replacements.push((r, curve));
                        }
                    }
                    if replacements.is_empty() {
                        continue;
                    }
                    let wkb = wkb_poly(&coords);
                    let changed = api.read(&wkb)?;
                    if !changed.is_valid()? || changed.difference(&forest)?.area()? > 1e-6
                        || forest.difference(&changed)?.difference(&water)?.area()? > 1e-6
                    {
                        continue;
                    }
                    for (r, path) in replacements {
                        curves.push(path);
                        refs[i][r] = vec![i32::try_from(curves.len())?];
                        shortened += 1;
                    }
                    final_coords[i] = coords;
                    final_wkb[i] = wkb;
                }
                println!("Submerged forest rings shortened: {shortened}");
            }
            if (l == 6 || args.common_lod) && args.implicit_background && args.unified_water {
                let parts = |code| g.polys.iter().zip(&final_wkb)
                    .filter(|(poly, _)| poly.code == code)
                    .map(|(_, wkb)| wkb.clone()).collect::<Vec<_>>();
                let water = api.read(&wkb_multi(&parts(3)))?;
                let visible_forest = PixelAreaIndex::new(&api.read(&wkb_multi(&parts(1)))?.difference(&water)?)?;
                let forest_cell = if args.common_lod { evaluation_cell } else { inland_water_cell };
                let mut pixel_areas = HashMap::new();
                let mut shortened = 0;
                for (i, poly) in g.polys.iter().enumerate() {
                    if poly.code != 1 {
                        continue;
                    }
                    let forest = api.read(&final_wkb[i])?;
                    let prepared_forest = forest.prepare()?;
                    for r in 0..refs[i].len() {
                        let path = shorten_submerged_forest_ring(
                            &api, &refs[i][r], &curves, &forest, &water, Some((forest_cell / 16., &prepared_forest)),
                        )?;
                        let Some(path) = path else {
                            continue;
                        };
                        let mut points = vec![path[0].p[0]];
                        for segment in &path {
                            flatten(segment, 0.5, &mut points, 0);
                        }
                        let mut coords = final_coords[i].clone();
                        coords[r] = points;
                        let wkb = wkb_poly(&coords);
                        let changed = api.read(&wkb)?;
                        let before = api.read(&final_wkb[i])?;
                        if !changed.is_valid()? || changed.difference(&before)?.area()? > 1e-6 {
                            continue;
                        }
                        let lost = before.difference(&changed)?;
                        let loss = lost.difference(&water)?;
                        let accept = accept_forest_pixel_loss(
                            &api, &loss, &visible_forest,
                            forest_cell, &mut pixel_areas,
                        )?;
                        if !accept {
                            continue;
                        }
                        curves.push(path);
                        refs[i][r] = vec![i32::try_from(curves.len())?];
                        final_coords[i] = coords;
                        final_wkb[i] = wkb;
                        shortened += 1;
                    }
                }
                println!("Forest rings shortened at display scale: {shortened}");
            }
            let source_row = manifest["levels"].as_array().and_then(|levels| {
                levels.iter().find(|row| row["level"].as_u64() == Some(l as u64))
            });
            let source_overlay = args.linear_source
                && source_row.and_then(|row| row["waterOverlay"].as_bool()).unwrap_or(false);
            let mut features = Vec::new();
            let mut vertices = 0;
            let mut total_area = 0.;
            let mut coverage = Vec::new();
            let mut packed_bytes = 0;
            let mut areas = serde_json::Map::new();
            for code in 0..5 {
                let indices: Vec<_> = g
                    .polys
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| if p.code == code { Some(i) } else { None })
                    .collect();
                let parts: Vec<_> = indices.iter().map(|&i| final_wkb[i].clone()).collect();
                let mut bytes = wkb_multi(&parts);
                if (l == 6 || args.common_lod) && code == 0 && args.implicit_background && args.unified_water {
                    let extent = api.rectangle(0., 0., domain_side, domain_side)?;
                    let shapes = final_wkb.iter().map(|wkb| api.read(wkb))
                        .collect::<Result<Vec<_>>>()?;
                    let missing = extent.difference(&api.collection(&shapes)?.union()?)?;
                    let background = api.collection(&[api.read(&bytes)?, missing])?.union()?;
                    let visible_dry = g.polys.iter().zip(&final_wkb)
                        .filter(|(poly, _)| matches!(poly.code, 1 | 2))
                        .map(|(_, wkb)| api.read(wkb)).collect::<Result<Vec<_>>>()?;
                    let visible_dry = api.collection(&visible_dry)?.union()?;
                    bytes = background.difference(&visible_dry)?.write()?;
                }
                let geom = api.read(&bytes)?;
                // Inter-polygon validity catches overlaps between islands/holes.
                ensure!(
                    geom.is_valid()?,
                    "Invalid grouped class {code} {filter} L{l}"
                );
                let area = geom.area()?;
                total_area += area;
                if args.implicit_background && (args.coast_pixel_visibility || args.fine_coastal_details || args.linear_source || args.common_lod) {
                    coverage.push(geom);
                }
                areas.insert(code.to_string(), json!(area));
                let omitted = args.implicit_background && code == 0;
                let packed = if omitted { zip(&wkb_multi(&[]), false)? } else { zip(&bytes, false)? };
                packed_bytes += packed.len();
                db.execute(
                    "INSERT INTO features VALUES(?,?,?)",
                    params![l as i32, code, packed],
                )?;
                let coords: Vec<_> = indices
                    .into_iter().filter(|_| !omitted)
                    .map(|i| {
                        vertices += final_coords[i].iter().map(Vec::len).sum::<usize>();
                        final_coords[i].clone()
                    })
                    .collect();
                features.push(json!({"type":"Feature","properties":{"class":code},"geometry":{"type":"MultiPolygon","coordinates":coords}}));
            }
            if !coverage.is_empty() {
                let extent = api.rectangle(0., 0., domain_side, domain_side)?;
                let mut covered = api.collection(&coverage)?.union().with_context(|| format!("Union final coverage L{l}"))?;
                let mut missing = extent.difference(&covered).with_context(|| format!("Validate final coverage L{l}"))?.area()?;
                let mut outside = covered.difference(&extent)?.area()?;
                if args.common_lod && args.unified_water && missing >= 0.01 && outside < 0.01 {
                    // Recheck unstable floating-point overlays on the same grid
                    // used when packing the national tiles.
                    let grid = landcover_xyz::national::GRID;
                    covered = api.collection(&coverage)?.union_grid(grid)?;
                    let remainder = extent.difference_grid(&covered, grid)?;
                    let remainder_area = remainder.area()?;
                    if remainder_area >= 0.01 {
                        // The BZF2 tile already represents uncovered areas as
                        // implicit background; no visible geometry is changed.
                        let filled = api.collection(&[covered, remainder])?.union_grid(grid)?;
                        missing = extent.difference_grid(&filled, grid)?.area()?;
                        outside = filled.difference_grid(&extent, grid)?.area()?;
                        // Keep the independent area accounting in source
                        // coordinates. This remainder is from grid overlay.
                        println!("Verified {remainder_area} m² of implicit background on the national tile grid in L{l}");
                    } else {
                        missing = remainder_area;
                        outside = covered.difference_grid(&extent, grid)?.area()?;
                        println!("Verified coverage on the national tile grid in L{l}");
                    }
                }
                if args.linear_source {
                    total_area += missing;
                    areas.insert("0".into(), json!(missing));
                }
                let gap = (if args.linear_source { 0. } else { missing }) + outside;
                ensure!(gap < 0.01, "Coverage footprint changed in L{l} by {gap} (missing {missing}, outside {outside})");
                let dry = api.collection(&coverage[..3])?.union()?;
                let dry_area = coverage[..3].iter().map(|g| g.area()).collect::<Result<Vec<_>>>()?.iter().sum::<f64>();
                let dry_overlap = (dry_area - dry.area()?).abs();
                ensure!(dry_overlap < 0.01, "Dry classes overlap by {dry_overlap} m² in L{l}");
                if args.unified_water || source_overlay {
                    overlay_area = dry.intersection(&coverage[3])?.area()?;
                }
            }
            if args.water_overlay {
                if !args.unified_water {
                    overlay_area = areas["3"].as_f64().context("Missing water area")?;
                }
                total_area -= overlay_area;
            }
            if source_overlay { total_area -= overlay_area; }
            let gap = (total_area - domain_side.powi(2)).abs();
            let area_epsilon = if args.common_finish { (domain_side.powi(2) * 3e-13).max(0.01) } else { 0.01 };
            ensure!(gap < area_epsilon, "Area changed by {gap}");
            let controls = if args.compact {
                encode_compact(&curves, &g.polys, &refs, args.implicit_background)
            } else {
                encode(&curves, &g.polys, &refs)
            };
            ensure!(!args.direct_lines || curves.iter().flatten().all(|curve| curve.line),
                    "Direct linear output contains a curved segment");
            let compressed = zip(&controls, true)?;
            let control_name = format!("level-{l}.curves.gz");
            fs::write(out.join(&control_name), &compressed)?;
            let used: HashSet<_> = g.polys.iter().zip(&refs)
                .filter(|(p, _)| !args.implicit_background || p.code != 0)
                .flat_map(|(_, rr)| rr.iter().flatten().map(|r| r.unsigned_abs() as usize - 1))
                .collect();
            let cubic = used.iter().flat_map(|&i| &curves[i]).filter(|c| !c.line).count();
            let lines = used.iter().map(|&i| curves[i].len()).sum::<usize>() - cubic;
            let info = json!({"method":"shared-cubic-bezier-fit","sourceLevel":source_level,"progressiveLod":args.source_levels,"geometricFit":args.geometric_fit||args.fair_boundary,"fairBoundary":args.fair_boundary,"fitToleranceM":tolerance,"sampledMaximumFitErrorM":max_error,"sourceCoordinates":source_vertices,"sharedChains":used.len(),"bezierSegments":cubic,"lineSegments":lines,"topologyFallbackChains":fallback.len(),"renderFlatnessM":0.5,"controlBytes":compressed.len(),"controlSha256":Sha256::digest(&compressed).iter().map(|b|format!("{b:02x}")).collect::<String>(),"controlRawBytes":controls.len(),"controlFile":control_name,"seconds":now.elapsed().as_secs_f64()});
            let bytes = serde_json::to_vec(
                &json!({"type":"FeatureCollection","features":features,"curveInfo":info}),
            )?;
            let packed = zip(&bytes, true)?;
            let file = format!("level-{l}.geojson.gz");
            fs::write(out.join(&file), &packed)?;
            let mut row = manifest["levels"].as_array().and_then(|levels| {
                levels.iter().find(|row| row["level"].as_u64() == Some(if args.display_classes || args.linear_source { l } else { l.min(5) } as u64))
            }).context("Missing target level metadata")?.clone();
            row["file"] = json!(file);
            if l == 6 && args.common_finish {
                fs::copy(source.join("reference-6.png"), out.join("reference-6.png"))?;
            }
            if l == 6 && !args.linear_source {
                row["level"] = json!(l);
                row["tolerance"] = json!(tolerance);
                let reference = row["referenceFile"].as_str().context("Missing L5 reference")?;
                fs::copy(source.join(reference), out.join("reference-6.png"))?;
                if args.display_classes {
                    row["pixelPreviewSide"] = json!(128);
                    row["pixelPreviewCellM"] = json!(domain_side / 128.);
                }
                row["referenceFile"] = json!("reference-6.png");
            }
            row["vertices"] = json!(vertices);
            row["polygons"] = json!(g.polys.iter().filter(|p| !args.implicit_background || p.code != 0).count());
            row["implicitBackground"] = json!(args.implicit_background);
            if let Some(count) = closed_water_breaks {
                row["closedWaterBreaks"] = json!(count);
            }
            if let Some(distance) = args.final_coast_simplify {
                row["finalCoastSimplifyM"] = json!(distance);
            }
            if let Some(tolerance) = args.independent_shore {
                row["independentShoreM"] = json!(tolerance);
            }
            if let Some(count) = filled_forest_gaps {
                row["filledForestGaps"] = json!(count);
            }
            if let Some(omitted) = omitted_bare_ground {
                row["omittedBareGroundFaces"] = json!(omitted);
            }
            if let Some([omitted, holes]) = water_removed {
                row["inlandWaterVisibilityPixelM"] = json!(inland_water_cell);
                row["inlandWaterVisibilityMinCoverage"] = json!(inland_water_min_coverage);
                row["waterOmittedFaces"] = json!(omitted);
                row["waterFilledHoles"] = json!(holes);
            }
            row["transferBytes"] = json!(packed.len());
            row["wkbZlibBytes"] = json!(packed_bytes);
            row["areaM2"] = json!(areas);
            row["curveInfo"] = info.clone();
            if let Some(side) = args.pixel_side {
                row["pixelPreviewSide"] = json!(side);
                row["pixelPreviewCellM"] = json!(domain_side / f64::from(side));
            }
            if args.coast_pixel_visibility {
                row["coastVisibilityPixelM"] = json!(500);
                row["coastVisibilityMinCoverage"] = json!(0.125);
            }
            if let Some([islands, inlets, merged, protrusions]) = coast_removed {
                row["coastOmittedProtrusions"] = json!(protrusions);
                row["coastMergedDryFaces"] = json!(merged);
                row["coastOmittedIslands"] = json!(islands);
                row["coastFilledInlets"] = json!(inlets);
            }
            if let Some(tolerance) = args.coast_tolerance {
                row["coastSimplifyM"] = json!(tolerance);
                row["coastSourceSegments"] = json!(g.chains.iter().zip(&coastal)
                    .filter(|(_, coast)| **coast).map(|(c, _)| c.nodes.len()-1).sum::<usize>());
                row["coastStoredSegments"] = json!(curves.iter().zip(&coastal)
                    .filter(|(_, coast)| **coast).map(|(c, _)| c.len()).sum::<usize>());
            }
            row["boundaryCodec"] = json!(if args.compact { "BZF2" } else { "BZF1" });
            if args.direct_lines { row["directLinear"] = json!(true); }
            if args.common_lod { row["commonLod"] = json!({"evaluationDistanceM":evaluation_cell,"waterEvaluationDistanceM":water_evaluation_cell,"inlandWaterCoverage":inland_water_coverage,"coastalToleranceM":coastal_tolerance}); }
            row["waterOverlay"] = json!(args.water_overlay || source_overlay);
            if args.water_overlay || source_overlay {
                row["waterOverlayAreaM2"] = json!(overlay_area);
            }
            row["unifiedWater"] = json!(args.unified_water || (args.linear_source && row["unifiedWater"].as_bool().unwrap_or(false)));
            if !args.source_levels {
                row["minimumArea"] = json!(0);
            }
            rows.push(row);
            println!(
                "{filter} L{l}: {cubic} cubics / {lines} lines; controls {} bytes, fallback {}, {:.2}s",
                compressed.len(),
                fallback.len(),
                now.elapsed().as_secs_f64()
            );
        }
        manifest["levels"] = json!(rows);
        manifest["generator"] = json!("terrain-landcover-bezier-fit / Rust");
        manifest["curveFit"] = json!(true);
        db.execute(
            "INSERT INTO metadata VALUES('manifest',?)",
            params![serde_json::to_string(&manifest)?],
        )?;
        ensure!(
            db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))? == "ok",
            "SQLite integrity"
        );
        drop(db);
        manifest["sqliteBytes"] = json!(fs::metadata(out.join("landcover-vectors.sqlite3"))?.len());
        fs::copy(source.join("reference.png"), out.join("reference.png"))?;
        if source.join("relief.png").is_file() {
            fs::copy(source.join("relief.png"), out.join("relief.png"))?;
        }
        fs::write(
            out.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        summaries.push(json!({"filter":filter,"levels":rows}));
    }
    fs::write(
        args.output.join("manifest.json"),
        serde_json::to_vec_pretty(
            &json!({"seconds":start.elapsed().as_secs_f64(),"variants":summaries}),
        )?,
    )?;
    println!("Complete {:.2}s", start.elapsed().as_secs_f64());
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires LANDCOVER_TEST_GEOS to point to a GEOS C library"]
    fn empty_forest_loss_does_not_visit_pixels() -> Result<()> {
        let library = std::env::var_os("LANDCOVER_TEST_GEOS").context("Set LANDCOVER_TEST_GEOS")?;
        let api = Geos::open(std::path::Path::new(&library))?;
        let empty = api.read(&wkb_poly(&[]))?;
        assert!(empty.is_empty()?);
        let visible = PixelAreaIndex::new(&empty)?;
        let mut pixels = HashMap::new();
        assert!(accept_forest_pixel_loss(&api, &empty, &visible, 125., &mut pixels)?);
        assert!(pixels.is_empty());
        Ok(())
    }

    use super::*;
    #[test]
    fn fairing_reduces_grid_oscillation_with_bounded_movement() {
        let mut points = vec![[0., 0.]];
        for i in 1..=10 {
            points.push([i as f64, (i - 1) as f64]);
            points.push([i as f64, i as f64]);
        }
        let smooth = fair_path(&points, 2., 0, None);
        assert_eq!(smooth[0], points[0]);
        assert_eq!(smooth.last(), points.last());
        assert!(
            points
                .iter()
                .zip(&smooth)
                .all(|(&a, &b)| norm(sub(a, b)) <= 1. + 1e-9)
        );
        let roughness = |p: &[P]| {
            p.windows(3)
                .map(|w| norm(sub(add(w[0], w[2]), mul(w[1], 2.))))
                .sum::<f64>()
        };
        assert!(roughness(&smooth) < roughness(&points) * 0.5);
    }
    #[test]
    fn loose_tolerance_does_not_collapse_closed_ring() {
        let p = [[0., 0.], [10., 0.], [10., 10.], [0., 10.], [0., 0.]];
        let mut out = Vec::new();
        let mut e = 0.;
        fit_path_geometric(
            &p,
            unit([1., 1.]),
            unit([1., 1.]),
            500.,
            0,
            None,
            &mut out,
            &mut e,
        );
        assert!(out.len() > 1);
        let mut samples = vec![p[0]];
        for c in &out {
            flatten(c, 0.1, &mut samples, 0);
        }
        let area = samples
            .windows(2)
            .map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1])
            .sum::<f64>()
            .abs()
            * 0.5;
        assert!(area > 1.);
    }
    #[test]
    fn tangential_displacement_is_not_boundary_error() {
        let c = Curve {
            p: [[0., 0.], [1., 0.], [9., 0.], [10., 0.]],
            line: false,
        };
        let p = [2.5, 0.];
        let u = closest_parameter(&c, p, 0.25);
        assert!(norm(sub(c.at(u), p)) < 1e-8);
        assert_eq!(dist_seg(c.at(0.25), [0., 0.], [10., 0.]), 0.);
    }
    #[test]
    fn two_point_span_can_keep_neighbor_tangent() {
        let pts = [[0., 0.], [10., 0.]];
        let mut out = Vec::new();
        let mut error = 0.;
        fit_path_geometric(
            &pts,
            unit([1., 0.5]),
            unit([-1., 0.5]),
            2.,
            0,
            None,
            &mut out,
            &mut error,
        );
        assert_eq!(out.len(), 1);
        assert!(!out[0].line);
        assert_eq!(out[0].p[0], pts[0]);
        assert_eq!(out[0].p[3], pts[1]);
    }
    #[test]
    fn fit_long_curve_with_few_segments() {
        let p: Vec<_> = (0..81)
            .map(|i| {
                let x = i as f64;
                [x, 20. * (x / 80. * std::f64::consts::PI).sin()]
            })
            .collect();
        let mut out = Vec::new();
        let mut e = 0.;
        fit_path(
            &p,
            unit(sub(p[2], p[0])),
            unit(sub(p[78], p[80])),
            1.,
            0,
            None,
            &mut out,
            &mut e,
        );
        assert!(out.len() < 8);
        assert_eq!(out[0].p[0], p[0]);
        assert_eq!(out.last().unwrap().p[3], p[80]);
        assert!(e <= 1.);
    }
    #[test]
    fn flatten_preserves_endpoints() {
        let c = Curve {
            p: [[0., 0.], [0., 10.], [10., 10.], [10., 0.]],
            line: false,
        };
        let mut p = vec![c.p[0]];
        flatten(&c, 0.1, &mut p, 0);
        assert_eq!(p[0], [0., 0.]);
        assert_eq!(*p.last().unwrap(), [10., 0.]);
        assert!(p.len() > 4);
    }
    #[test]
    fn shared_chain_reversed_exactly() {
        let a = vec![[0., 0.], [4., 0.], [4., 2.], [4., 4.], [0., 4.], [0., 0.]];
        let b = vec![[4., 0.], [8., 0.], [8., 4.], [4., 4.], [4., 0.]];
        let g = build(vec![(0, vec![a]), (1, vec![b])], false, false).unwrap();
        let common = g.chains.iter().position(|c| !c.frozen).unwrap();
        let refs: Vec<_> = g
            .polys
            .iter()
            .map(|p| references(&g, &p.rings[0]).unwrap())
            .collect();
        assert!(
            refs[0].contains(&(common as i32 + 1)) && refs[1].contains(&-(common as i32 + 1))
                || refs[1].contains(&(common as i32 + 1))
                    && refs[0].contains(&-(common as i32 + 1))
        );
    }
    #[test]
    fn lollipop_and_mid_chain_ring_origin() {
        let a = vec![[4., 2.], [4., 4.], [0., 4.], [0., 0.], [4., 0.], [4., 2.]];
        let b = vec![[4., 0.], [8., 0.], [8., 4.], [4., 4.], [4., 0.]];
        let g = build(vec![(0, vec![a]), (1, vec![b])], false, false).unwrap();
        let shared: Vec<_> = g.chains.iter().filter(|c| !c.frozen).collect();
        assert_eq!(shared.len(), 1);
        for p in &g.polys {
            for r in &p.rings {
                references(&g, r).unwrap();
            }
        }
    }
    #[test]
    fn independent_water_does_not_share_dry_boundary_nodes() {
        let dry = vec![[0., 0.], [2., 0.], [4., 0.], [4., 4.], [0., 4.], [0., 0.]];
        let water = vec![[0., 0.], [4., 0.], [4., 4.], [0., 4.], [0., 0.]];
        let graph = build_graph(vec![(1, vec![dry]), (3, vec![water])], true, false, true).unwrap();
        let dry_ring = &graph.polys[0].rings[0];
        let water_ring = &graph.polys[1].rings[0];
        assert_eq!(water_ring.len(), 4);
        assert!(dry_ring.iter().all(|point| !water_ring.contains(point)));
    }
    #[test]
    fn closed_island_has_four_shared_chains() {
        let a = vec![[0., 0.], [4., 0.], [4., 4.], [0., 4.], [0., 0.]];
        let g = build(vec![
            (0, vec![a.clone()]),
            (1, vec![a.into_iter().rev().collect()]),
        ], false, false)
        .unwrap();
        assert_eq!(g.chains.len(), 4);
    }
}
