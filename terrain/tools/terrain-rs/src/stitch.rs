use crate::{
    codec::{decode_mesh, encode_mesh},
    mesh::Mesh,
};
use anyhow::Result;
use rusqlite::Connection;
use std::collections::BTreeMap;
type Key = (u64, u64);
fn key(x: f64, y: f64) -> Key {
    // 近接する別の海岸頂点を同一視しない。負のゼロだけをそろえる。
    (
        if x == 0. { 0 } else { x.to_bits() },
        if y == 0. { 0 } else { y.to_bits() },
    )
}
pub fn profiles(mesh: &Mesh) -> [Vec<[f64; 2]>; 4] {
    std::array::from_fn(|side| {
        let axis = if side < 2 { 0 } else { 1 };
        let value = if side % 2 == 0 { 0. } else { 256. };
        let mut points: Vec<_> = mesh
            .vertices
            .iter()
            .filter(|p| (p[axis] - value).abs() < 1e-7)
            .map(|p| [p[1 - axis], p[2]])
            .collect();
        points.sort_by(|a, b| a[0].total_cmp(&b[0]));
        points
    })
}
pub fn interpolate(points: &[[f64; 2]], t: f64) -> f64 {
    let i = points.partition_point(|p| p[0] < t);
    if i == 0 {
        return points[0][1];
    }
    if i == points.len() {
        return points[i - 1][1];
    }
    let [a, b] = [points[i - 1], points[i]];
    a[1] + (b[1] - a[1]) * (t - a[0]) / (b[0] - a[0])
}
pub fn stitch(db: &mut Connection, level: u32) -> Result<serde_json::Value> {
    let mut collected = BTreeMap::new();
    {
        let mut statement = db.prepare("SELECT x,y,data FROM tiles WHERE z=?1")?;
        let rows = statement.query_map([level], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        for row in rows {
            let (x, y, b) = row?;
            collected.insert((x, y), profiles(&decode_mesh(&b)?));
        }
    }
    let mut plans: BTreeMap<(u32, u32), BTreeMap<usize, Vec<f64>>> = BTreeMap::new();
    let mut samples: BTreeMap<Key, Vec<f64>> = BTreeMap::new();
    let mut pairs = 0;
    for (&(x, y), edges) in &collected {
        for (neighbor, a, b, axis) in [((x + 1, y), 1, 0, 0), ((x, y + 1), 3, 2, 1)] {
            let Some(right) = collected.get(&neighbor) else {
                continue;
            };
            let (left, right) = (&edges[a], &right[b]);
            if left.is_empty() || right.is_empty() {
                continue;
            }
            let mut values: Vec<_> = left.iter().chain(right).map(|p| p[0]).collect();
            values.sort_by(f64::total_cmp);
            values.dedup();
            plans.entry((x, y)).or_default().insert(a, values.clone());
            plans.entry(neighbor).or_default().insert(b, values.clone());
            for t in values {
                let k = if axis == 0 {
                    key((x + 1) as f64 * 256., y as f64 * 256. + t)
                } else {
                    key(x as f64 * 256. + t, (y + 1) as f64 * 256.)
                };
                samples
                    .entry(k)
                    .or_default()
                    .extend([interpolate(left, t), interpolate(right, t)]);
            }
            pairs += 1;
        }
    }
    let heights: BTreeMap<_, _> = samples
        .into_iter()
        .map(|(k, mut values)| {
            values.sort_by(f64::total_cmp);
            let n = values.len();
            let median = if n % 2 == 0 {
                (values[n / 2 - 1] + values[n / 2]) / 2.
            } else {
                values[n / 2]
            };
            let h = if values.contains(&0.) {
                0.
            } else {
                (median / 5.).round_ties_even() * 5.
            };
            (k, h)
        })
        .collect();
    let mut changed = 0;
    let mut maximum = 0f64;
    let transaction = db.transaction()?;
    for ((x, y), plan) in plans {
        let body: Vec<u8> = transaction.query_row(
            "SELECT data FROM tiles WHERE z=?1 AND x=?2 AND y=?3",
            (level, x, y),
            |r| r.get(0),
        )?;
        let mut mesh = decode_mesh(&body)?;
        let before = mesh.clone();
        let mut lookup: BTreeMap<_, _> = mesh
            .vertices
            .iter()
            .enumerate()
            .map(|(i, p)| (key(p[0], p[1]), i))
            .collect();
        for p in &mut mesh.vertices {
            if let Some(h) = heights.get(&key(x as f64 * 256. + p[0], y as f64 * 256. + p[1])) {
                maximum = maximum.max((p[2] - h).abs());
                p[2] = *h;
            }
        }
        for (side, values) in plan {
            let axis = if side < 2 { 0 } else { 1 };
            let value = if side % 2 == 0 { 0. } else { 256. };
            let mut faces = Vec::new();
            for face in &mesh.faces {
                let mut edited = false;
                for k in 0..3 {
                    let (a, b, c) = (face[k], face[(k + 1) % 3], face[(k + 2) % 3]);
                    if mesh.vertices[a][axis] != value || mesh.vertices[b][axis] != value {
                        continue;
                    }
                    let lo = mesh.vertices[a][1 - axis].min(mesh.vertices[b][1 - axis]);
                    let hi = mesh.vertices[a][1 - axis].max(mesh.vertices[b][1 - axis]);
                    let selected: Vec<_> = values
                        .iter()
                        .copied()
                        .filter(|t| *t > lo && *t < hi)
                        .collect();
                    if selected.is_empty() {
                        continue;
                    }
                    let mut chain = Vec::new();
                    for t in selected {
                        let mut p = [0.; 3];
                        p[axis] = value;
                        p[1 - axis] = t;
                        p[2] = heights[&key(x as f64 * 256. + p[0], y as f64 * 256. + p[1])];
                        let index = *lookup.entry(key(p[0], p[1])).or_insert_with(|| {
                            let index = mesh.vertices.len();
                            mesh.vertices.push(p);
                            index
                        });
                        chain.push(index);
                    }
                    if mesh.vertices[a][1 - axis] > mesh.vertices[b][1 - axis] {
                        chain.reverse()
                    }
                    chain.insert(0, a);
                    chain.push(b);
                    for pair in chain.windows(2) {
                        faces.push([pair[0], pair[1], c]);
                    }
                    edited = true;
                    break;
                }
                if !edited {
                    faces.push(*face)
                }
            }
            mesh.faces = faces;
        }
        if mesh.vertices != before.vertices || mesh.faces != before.faces {
            transaction.execute(
                "UPDATE tiles SET data=?1 WHERE z=?2 AND x=?3 AND y=?4",
                (encode_mesh(&mesh)?, level, x, y),
            )?;
            transaction.execute(
                "UPDATE metrics SET vertices=?1,triangles=?2 WHERE z=?3 AND x=?4 AND y=?5",
                (mesh.vertices.len(), mesh.faces.len(), level, x, y),
            )?;
            changed += 1;
        }
    }
    let report = serde_json::json!({"level":level,"sharedBorders":pairs,"repairedMeshes":changed,"maximumHeightAdjustmentMeters":maximum,"heightQuantizationMeters":5});
    transaction.execute(
        "INSERT OR REPLACE INTO metadata VALUES (?1,?2)",
        (format!("boundaryStitch{level}"), report.to_string()),
    )?;
    transaction.commit()?;
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_edge_insertion() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,data BLOB);CREATE TABLE metrics(z INTEGER,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER);CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT);").unwrap();
        let left = Mesh {
            vertices: vec![
                [0., 0., 100.],
                [256., 0., 100.],
                [0., 256., 100.],
                [256., 256., 100.],
            ],
            faces: vec![[0, 1, 2], [1, 3, 2]],
        };
        let right = Mesh {
            vertices: vec![
                [0., 0., 90.],
                [256., 0., 100.],
                [0., 128., 100.],
                [0., 256., 100.],
                [256., 256., 100.],
            ],
            faces: vec![[0, 1, 2], [2, 1, 4], [2, 4, 3]],
        };
        for (x, mesh) in [(0, left), (1, right)] {
            db.execute(
                "INSERT INTO tiles VALUES (14,?1,0,?2)",
                (x, encode_mesh(&mesh).unwrap()),
            )
            .unwrap();
            db.execute(
                "INSERT INTO metrics VALUES (14,?1,0,?2,?3)",
                (x, mesh.vertices.len(), mesh.faces.len()),
            )
            .unwrap();
        }
        let report = stitch(&mut db, 14).unwrap();
        assert_eq!(report["sharedBorders"], 1);
        let get = |x| {
            let b: Vec<u8> = db
                .query_row("SELECT data FROM tiles WHERE x=?1", [x], |r| r.get(0))
                .unwrap();
            decode_mesh(&b).unwrap()
        };
        let a = get(0);
        let b = get(1);
        assert_eq!(profiles(&a)[1], profiles(&b)[0]);
        assert_eq!(a.faces.len(), 3);
        assert!(
            a.faces
                .iter()
                .all(|f| crate::mesh::normal(&a.vertices, *f)[2] > 0.)
        );
        assert_eq!(profiles(&a)[1][0][1], 95.);
    }
}
