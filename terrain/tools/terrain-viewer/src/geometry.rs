use crate::data::{Point, Tile, UNIT, bounds};
use anyhow::{Result, ensure};
use glam::Vec3;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Vertex {
    pub base: [f32; 3],
    pub curved: [f32; 3],
}
#[derive(Clone)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub control: crate::gpu_loop::Control,
    pub source_faces: usize,
    pub tiles: usize,
    pub divisions: u32,
    pub adaptive: Option<Arc<crate::adaptive_curve::Plan>>,
    pub levels: Vec<u32>,
}
fn key(p: Point) -> (i64, i64) {
    (
        (p[0] * 1024.0).round() as i64,
        (p[1] * 1024.0).round() as i64,
    )
}
fn area(a: Point, b: Point, c: Point) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn clip(mut poly: Vec<Point>, b: [f64; 4]) -> Vec<Point> {
    for (axis, v, s) in [
        (0, b[0], 1.0),
        (0, b[2], -1.0),
        (1, b[1], 1.0),
        (1, b[3], -1.0),
    ] {
        let mut out = Vec::new();
        for i in 0..poly.len() {
            let a = poly[i];
            let c = poly[(i + 1) % poly.len()];
            let aa = s * (a[axis] - v) >= -1e-4;
            let cc = s * (c[axis] - v) >= -1e-4;
            if aa {
                out.push(a);
            }
            if aa != cc {
                let r = (v - a[axis]) / (c[axis] - a[axis]);
                let mut p = std::array::from_fn(|j| a[j] + r * (c[j] - a[j]));
                p[axis] = v;
                out.push(p);
            }
        }
        poly = out;
        if poly.is_empty() {
            break;
        }
    }
    poly.dedup_by_key(|p| key(*p));
    if poly.len() > 1 && key(poly[0]) == key(*poly.last().unwrap()) {
        poly.pop();
    }
    poly
}
struct Seam {
    axis: usize,
    value: f64,
    ranges: Vec<[f64; 2]>,
    nodes: BTreeMap<i64, usize>,
    segments: Vec<(Point, Point, u32)>,
}
fn add(
    p: Point,
    z: u32,
    lookup: &mut HashMap<(i64, i64), usize>,
    points: &mut Vec<Point>,
    levels: &mut Vec<u32>,
) -> usize {
    let k = key(p);
    if let Some(&id) = lookup.get(&k) {
        if z > levels[id] {
            points[id][2] = p[2];
            levels[id] = z;
        }
        id
    } else {
        let id = points.len();
        lookup.insert(k, id);
        points.push([k.0 as f64 / 1024.0, k.1 as f64 / 1024.0, p[2]]);
        levels.push(z);
        id
    }
}
fn triangulate(mut ids: Vec<usize>, p: &[Point], out: &mut Vec<[usize; 3]>) -> Result<()> {
    while ids.len() > 3 {
        let mut ear = None;
        for i in 0..ids.len() {
            let a = ids[(i + ids.len() - 1) % ids.len()];
            let b = ids[i];
            let c = ids[(i + 1) % ids.len()];
            if area(p[a], p[b], p[c]) >= -1e-4 {
                continue;
            }
            let blocked = ids.iter().any(|&d| {
                d != a
                    && d != b
                    && d != c
                    && area(p[a], p[b], p[d]) <= 1e-4
                    && area(p[b], p[c], p[d]) <= 1e-4
                    && area(p[c], p[a], p[d]) <= 1e-4
            });
            if !blocked {
                ear = Some((i, [a, b, c]));
                break;
            }
        }
        let Some((i, f)) = ear else {
            anyhow::bail!("境界の三角形分割に失敗しました")
        };
        out.push(f);
        ids.remove(i);
    }
    if ids.len() == 3 && area(p[ids[0]], p[ids[1]], p[ids[2]]) < -1e-4 {
        out.push([ids[0], ids[1], ids[2]]);
    }
    Ok(())
}
pub fn merge(tiles: &[Arc<Tile>]) -> Result<(Vec<Point>, Vec<[usize; 3]>)> {
    let mixed = tiles.iter().map(|t| t.key.0).collect::<HashSet<_>>().len() > 1;
    let mut seams: BTreeMap<(usize, i64), Seam> = BTreeMap::new();
    if mixed {
        for (i, a) in tiles.iter().enumerate() {
            for b in &tiles[i + 1..] {
                if a.key.0 == b.key.0 {
                    continue;
                }
                let aa = bounds(a.key);
                let bb = bounds(b.key);
                for axis in 0..2 {
                    let v = if aa[axis + 2] == bb[axis] {
                        aa[axis + 2]
                    } else if bb[axis + 2] == aa[axis] {
                        aa[axis]
                    } else {
                        continue;
                    };
                    let j = 1 - axis;
                    let range = [aa[j].max(bb[j]), aa[j + 2].min(bb[j + 2])];
                    if range[1] > range[0] {
                        seams
                            .entry((axis, v as i64))
                            .or_insert_with(|| Seam {
                                axis,
                                value: v,
                                ranges: vec![],
                                nodes: BTreeMap::new(),
                                segments: vec![],
                            })
                            .ranges
                            .push(range);
                    }
                }
            }
        }
    }
    let mut points = Vec::new();
    let mut levels = Vec::new();
    let mut lookup = HashMap::new();
    let mut polys = Vec::new();
    for tile in tiles {
        for f in &tile.faces {
            let p = f.map(|i| tile.points[i]).to_vec();
            let p = if mixed { clip(p, bounds(tile.key)) } else { p };
            if p.len() < 3 {
                continue;
            }
            let ids: Vec<_> = p
                .iter()
                .map(|&p| add(p, tile.key.0, &mut lookup, &mut points, &mut levels))
                .collect();
            for i in 0..p.len() {
                let a = p[i];
                let b = p[(i + 1) % p.len()];
                for axis in 0..2 {
                    if (a[axis] - b[axis]).abs() > 1e-4 || (a[axis] - a[axis].round()).abs() > 1e-4
                    {
                        continue;
                    }
                    let Some(s) = seams.get_mut(&(axis, a[axis].round() as i64)) else {
                        continue;
                    };
                    let j = 1 - axis;
                    let lo = a[j].min(b[j]);
                    let hi = a[j].max(b[j]);
                    if hi - lo < 1e-4
                        || !s
                            .ranges
                            .iter()
                            .any(|r| hi >= r[0] - 1e-4 && lo <= r[1] + 1e-4)
                    {
                        continue;
                    }
                    s.segments.push((a, b, tile.key.0));
                    s.nodes.insert((a[j] * 1024.0).round() as i64, ids[i]);
                    s.nodes
                        .insert((b[j] * 1024.0).round() as i64, ids[(i + 1) % p.len()]);
                    for r in &s.ranges {
                        for &v in r {
                            if v <= lo + 1e-4 || v >= hi - 1e-4 {
                                continue;
                            }
                            let q = (v - a[j]) / (b[j] - a[j]);
                            let mut p = std::array::from_fn(|k| a[k] + q * (b[k] - a[k]));
                            p[axis] = s.value;
                            let id = add(p, tile.key.0, &mut lookup, &mut points, &mut levels);
                            s.nodes.insert((v * 1024.0).round() as i64, id);
                        }
                    }
                }
            }
            polys.push(ids);
        }
    }
    for s in seams.values_mut() {
        s.segments.sort_by_key(|v| v.2);
        let j = 1 - s.axis;
        for &(a, b, z) in &s.segments {
            let lo = (a[j].min(b[j]) * 1024.0).round() as i64;
            let hi = (a[j].max(b[j]) * 1024.0).round() as i64;
            for (_, &id) in s.nodes.range(lo..=hi) {
                if z < levels[id] {
                    continue;
                }
                let q = (points[id][j] - a[j]) / (b[j] - a[j]);
                points[id][2] = a[2] + q * (b[2] - a[2]);
                levels[id] = z;
            }
        }
    }
    let mut faces = Vec::new();
    let mut unique = HashSet::new();
    for ids in polys {
        let mut expanded = Vec::new();
        for i in 0..ids.len() {
            let a = points[ids[i]];
            let b = points[ids[(i + 1) % ids.len()]];
            expanded.push(ids[i]);
            for axis in 0..2 {
                if (a[axis] - b[axis]).abs() > 1e-4 {
                    continue;
                }
                let Some(s) = seams.get(&(axis, a[axis].round() as i64)) else {
                    continue;
                };
                if (a[axis] - s.value).abs() > 1e-4 {
                    continue;
                }
                let j = 1 - axis;
                let lo = (a[j].min(b[j]) * 1024.0).round() as i64;
                let hi = (a[j].max(b[j]) * 1024.0).round() as i64;
                let mut mid: Vec<_> = s
                    .nodes
                    .range((std::ops::Bound::Excluded(lo), std::ops::Bound::Excluded(hi)))
                    .map(|(_, &id)| id)
                    .collect();
                if b[j] < a[j] {
                    mid.reverse();
                }
                expanded.extend(mid);
            }
        }
        let mut ff = Vec::new();
        triangulate(expanded, &points, &mut ff)?;
        for f in ff {
            let mut k = f;
            k.sort();
            if unique.insert(k) {
                faces.push(f);
            }
        }
    }
    // 未使用の境界候補を除いてLoopの接続表を小さくする。
    let used: BTreeSet<_> = faces.iter().flatten().copied().collect();
    let mut remap = vec![0; points.len()];
    let compact: Vec<_> = used
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            remap[id] = i;
            points[id]
        })
        .collect();
    for f in &mut faces {
        *f = f.map(|i| remap[i]);
    }
    Ok((compact, faces))
}
// 頂点ごとの接続面を連続領域に詰める。GUIでの頂点数分の小さな確保を避ける。
#[cfg(test)]
pub fn adjacency(vertices: usize, indices: &[u32]) -> Vec<u32> {
    let mut counts = vec![0u32; vertices];
    for &i in indices {
        counts[i as usize] += 1;
    }
    let mut out = vec![0u32; vertices * 2 + indices.len()];
    let mut next = Vec::with_capacity(vertices);
    let mut at = vertices * 2;
    for (i, &count) in counts.iter().enumerate() {
        out[2 * i] = at as u32;
        out[2 * i + 1] = count;
        next.push(at);
        at += count as usize;
    }
    for (face, ids) in indices.chunks_exact(3).enumerate() {
        for &i in ids {
            let i = i as usize;
            out[next[i]] = face as u32;
            next[i] += 1;
        }
    }
    out
}
pub fn build(tiles: &[Arc<Tile>], origin: [f64; 2], requested: u32) -> Result<Mesh> {
    let source_faces = tiles.iter().map(|t| t.faces.len()).sum();
    let (p, faces) = merge(tiles)?;
    let base: Vec<Vec3> = p
        .iter()
        .map(|p| {
            Vec3::new(
                ((p[0] * UNIT) - origin[0]) as f32,
                p[2] as f32,
                ((p[1] * UNIT) - origin[1]) as f32,
            )
        })
        .collect();
    let curved = base.clone();
    let divisions = requested;
    ensure!(
        base.len() <= u32::MAX as usize,
        "描画頂点が32ビット範囲外です"
    );
    let vertices: Vec<Vertex> = base
        .into_iter()
        .zip(curved)
        .map(|(a, b)| Vertex {
            base: a.to_array(),
            curved: b.to_array(),
        })
        .collect();
    let indices: Vec<u32> = faces.into_iter().flatten().map(|i| i as u32).collect();
    let control = crate::gpu_loop::control(vertices.len(), &indices);
    Ok(Mesh {
        vertices,
        indices,
        control,
        source_faces,
        tiles: tiles.len(),
        divisions,
        adaptive: None,
        levels: tiles
            .iter()
            .map(|t| t.key.0)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_adjacency_contains_each_connected_face_once() {
        let indices = [0, 1, 2, 2, 1, 3];
        let table = adjacency(5, &indices);
        for (vertex, expected) in [vec![0], vec![0, 1], vec![0, 1], vec![1], vec![]]
            .iter()
            .enumerate()
        {
            let at = table[2 * vertex] as usize;
            let count = table[2 * vertex + 1] as usize;
            assert_eq!(&table[at..at + count], expected.as_slice());
        }
        assert_eq!(table.len(), 5 * 2 + 6);
        assert_eq!(std::mem::size_of::<Vertex>(), 24);
    }
    #[test]
    fn partial_mixed_edge_is_shared_without_t_junctions() {
        let fine = Arc::new(Tile {
            key: (14, 1, 0),
            points: Arc::new(vec![
                [16384., 0., 0.],
                [32768., 0., 0.],
                [32768., 8192., 2.],
                [32768., 16384., 0.],
                [16384., 16384., 0.],
            ]),
            faces: vec![[0, 4, 2], [0, 2, 1], [4, 3, 2]],
        });
        let coarse = Arc::new(Tile {
            key: (13, 1, 0),
            points: Arc::new(vec![
                [32768., 0., 0.],
                [65536., 0., 0.],
                [32768., 32768., 0.],
                [65536., 32768., 0.],
            ]),
            faces: vec![[0, 2, 1], [1, 2, 3]],
        });
        let (points, faces) = merge(&[fine, coarse]).unwrap();
        let mut edges = HashMap::new();
        for f in faces {
            assert!(area(points[f[0]], points[f[1]], points[f[2]]) < 0.);
            for (a, b) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        let mut shared = 0;
        for ((a, b), count) in edges {
            if points[a][0] == 32768.
                && points[b][0] == 32768.
                && points[a][1].max(points[b][1]) <= 16384.
            {
                assert_eq!(count, 2);
                shared += 1;
            }
        }
        assert_eq!(shared, 2);
    }
    #[test]
    fn subdivision_preserves_plane() {
        let tile = Arc::new(Tile {
            key: (14, 0, 0),
            points: Arc::new(vec![
                [0., 0., 10.],
                [0., 16384., 10.],
                [16384., 0., 10.],
                [16384., 16384., 10.],
            ]),
            faces: vec![[0, 1, 2], [2, 1, 3]],
        });
        let m = build(&[tile], [0., 0.], 4).unwrap();
        assert_eq!(m.indices.len() / 3, 2);
        assert_eq!(m.vertices.len(), 4);
        assert_eq!(m.divisions, 4);
        assert!(m.vertices.iter().all(|v| (v.curved[1] - 10.).abs() < 1e-5));
    }
    #[test]
    fn same_level_welds_vertices() {
        let p = Arc::new(vec![[0., 0., 0.], [0., 10., 0.], [10., 0., 0.]]);
        let a = Arc::new(Tile {
            key: (14, 0, 0),
            points: p.clone(),
            faces: vec![[0, 1, 2]],
        });
        let b = Arc::new(Tile {
            key: (14, 1, 0),
            points: p,
            faces: vec![[0, 1, 2]],
        });
        let (p, f) = merge(&[a, b]).unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(f.len(), 1);
    }
}
