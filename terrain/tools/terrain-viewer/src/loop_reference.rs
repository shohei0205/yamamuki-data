// GPU照合用の旧CPU計算。通常のビューワーからは呼ばない。
use glam::Vec3;
use std::collections::{BTreeMap, BTreeSet};
pub fn subdivide(
    vertices: &[crate::geometry::Vertex],
    indices: &[u32],
    requested: u32,
) -> (Vec<crate::geometry::Vertex>, Vec<u32>) {
    let mut base: Vec<_> = vertices.iter().map(|v| Vec3::from_array(v.base)).collect();
    let mut faces: Vec<[usize; 3]> = indices
        .chunks_exact(3)
        .map(|f| [f[0] as usize, f[1] as usize, f[2] as usize])
        .collect();
    let mut curved = base.clone();
    let mut divisions = 1;
    while divisions < requested && faces.len() * 4 <= 2_000_000 {
        let mut edges: BTreeMap<(usize, usize), (usize, Vec<usize>)> = BTreeMap::new();
        let mut neighbors = vec![BTreeSet::new(); base.len()];
        for &[a, b, c] in &faces {
            for (a, b, c) in [(a, b, c), (b, c, a), (c, a, b)] {
                neighbors[a].insert(b);
                neighbors[b].insert(a);
                let k = (a.min(b), a.max(b));
                edges.entry(k).or_insert((0, vec![])).1.push(c);
            }
        }
        let mut boundary = vec![Vec::new(); base.len()];
        for (&(a, b), (_, opp)) in &edges {
            if opp.len() == 1 {
                boundary[a].push(b);
                boundary[b].push(a);
            }
        }
        let mut next = curved.clone();
        for i in 0..curved.len() {
            if boundary[i].len() == 2 {
                next[i] =
                    curved[i] * 0.75 + (curved[boundary[i][0]] + curved[boundary[i][1]]) * 0.125;
            } else if boundary[i].is_empty() && !neighbors[i].is_empty() {
                let n = neighbors[i].len() as f32;
                let beta = if n == 3.0 {
                    3.0 / 16.0
                } else {
                    3.0 / (8.0 * n)
                };
                let sum: Vec3 = neighbors[i].iter().map(|&j| curved[j]).sum();
                next[i] = curved[i] * (1.0 - n * beta) + sum * beta;
            }
        }
        for (&(a, b), (id, opp)) in &mut edges {
            *id = next.len();
            base.push((base[a] + base[b]) * 0.5);
            next.push(if opp.len() == 2 {
                (curved[a] + curved[b]) * 0.375 + (curved[opp[0]] + curved[opp[1]]) * 0.125
            } else {
                (curved[a] + curved[b]) * 0.5
            });
        }
        let mut ff = Vec::with_capacity(faces.len() * 4);
        for [a, b, c] in faces {
            let ab = edges[&(a.min(b), a.max(b))].0;
            let bc = edges[&(b.min(c), b.max(c))].0;
            let ca = edges[&(c.min(a), c.max(a))].0;
            ff.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
        }
        faces = ff;
        curved = next;
        divisions *= 2;
    }

    (
        base.into_iter()
            .zip(curved)
            .map(|(a, b)| crate::geometry::Vertex {
                base: a.to_array(),
                curved: b.to_array(),
            })
            .collect(),
        faces.into_iter().flatten().map(|i| i as u32).collect(),
    )
}
