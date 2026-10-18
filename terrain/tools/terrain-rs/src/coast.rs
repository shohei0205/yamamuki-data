use crate::{
    geos::{Geometry, Geos},
    mesh::{Mesh, Surface, normal},
};
use anyhow::{Result, ensure};
use rstar::{AABB, PointDistance, RTree, RTreeObject};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone)]
struct Segment([f64; 2], [f64; 2]);
impl RTreeObject for Segment {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(
            [self.0[0].min(self.1[0]), self.0[1].min(self.1[1])],
            [self.0[0].max(self.1[0]), self.0[1].max(self.1[1])],
        )
    }
}
impl PointDistance for Segment {
    fn distance_2(&self, p: &[f64; 2]) -> f64 {
        segment_distance(*p, self.0, self.1).powi(2)
    }
}

fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let length = dx * dx + dy * dy;
    let t = if length > 0. {
        ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length
    } else {
        0.
    }
    .clamp(0., 1.);
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}
pub fn rebuild(api: &Geos, source: &Mesh, land: &Geometry<'_>) -> Result<Mesh> {
    let mut shoreline = Vec::new();
    for ring in land.rings()? {
        for edge in ring.windows(2) {
            let [a, b] = [edge[0], edge[1]];
            if (0..2).any(|axis| {
                (a[axis] - b[axis]).abs() < 1e-8
                    && (a[axis].abs() < 1e-8 || (a[axis] - 256.).abs() < 1e-8)
            }) {
                continue;
            }
            shoreline.push((a, b));
        }
    }
    let shoreline = RTree::bulk_load(shoreline.into_iter().map(|(a, b)| Segment(a, b)).collect());
    let near = |p: [f64; 2], tolerance: f64| {
        shoreline
            .nearest_neighbor(&p)
            .is_some_and(|s| s.distance_2(&p) <= tolerance * tolerance)
    };
    let mut original = Vec::with_capacity(source.faces.len());
    let mut clipped = Vec::new();
    for face in &source.faces {
        let polygon = api.polygon(&face.map(|i| [source.vertices[i][0], source.vertices[i][1]]))?;
        if land.covers(&polygon)? {
            clipped.push(api.read(&polygon.write()?)?);
        } else if land.intersects(&polygon)? {
            clipped.extend(polygon.intersection(land)?.polygons()?);
        }
        original.push(polygon);
    }
    let existing = api.collection(&original)?.union()?;
    let extra = land.difference(&existing)?;
    for polygon in extra.polygons()? {
        // 海岸に接している追加領域だけを補い、内陸の欠損は残す。
        let rings = polygon.rings()?;
        let touches = rings.iter().flatten().any(|p| near(*p, 1e-8));
        if touches {
            clipped.push(polygon);
        }
    }
    let mut triangles = Vec::new();
    for polygon in clipped {
        if polygon.is_empty()? {
            continue;
        }
        let triangulated = match polygon.triangulate() {
            Ok(result) => result,
            Err(failure) => {
                // 保存時の座標丸め以下の精度で、微小な辺だけをそろえる。
                let mut recovered = None;
                for precision in [1e-8, 1e-7, 1e-6, 1e-5] {
                    let rounded = polygon.precision(precision)?;
                    if !rounded.is_valid()?
                        || rounded.counts()? != polygon.counts()?
                        || (rounded.area()? - polygon.area()?).abs() > 1e-3
                    {
                        continue;
                    }
                    if let Ok(result) = rounded.triangulate() {
                        eprintln!(
                            "海岸の微小な辺を {precision} 格子座標にそろえて三角形化を再試行しました"
                        );
                        recovered = Some(result);
                        break;
                    }
                }
                if let Some(result) = recovered {
                    result
                } else {
                    let result = subdivide_triangulate(api, &polygon, 0)?;
                    let union = result.union()?;
                    let difference =
                        polygon.difference(&union)?.area()? + union.difference(&polygon)?.area()?;
                    if difference > 1e-8 * polygon.area()?.max(1.) {
                        if let Ok(path) = std::env::var("TERRAIN_DIAGNOSTIC_WKB") {
                            std::fs::write(path, polygon.write()?)?;
                        }
                        eprintln!(
                            "三角形化の差分面積 {difference}、入力面積 {}、有効 {}",
                            polygon.area()?,
                            polygon.is_valid()?
                        );
                    }
                    ensure!(
                        difference <= 1e-8 * polygon.area()?.max(1.),
                        "別方式の三角形化で輪郭が変わりました: {failure}"
                    );
                    ensure!(
                        (result.area()? - union.area()?).abs() <= 1e-8 * polygon.area()?.max(1.),
                        "三角形が重なっています"
                    );
                    eprintln!(
                        "GEOS の三角形化に失敗した輪郭を分割し、面積と輪郭の一致を確認しました"
                    );
                    result
                }
            }
        };
        for ring in triangulated.rings()? {
            ensure!(ring.len() == 4, "海岸の三角形化で三角形以外ができました");
            triangles.push([ring[0], ring[1], ring[2]]);
        }
    }
    let surface = Surface::new(source);
    let mut points = BTreeMap::new();
    let quantize = |p: [f64; 2]| {
        [
            (p[0] * 100_000.).round_ties_even() as i64,
            (p[1] * 100_000.).round_ties_even() as i64,
        ]
    };
    for t in &triangles {
        for p in t {
            points.insert(quantize(*p), 0usize);
        }
    }
    let mut vertices = Vec::with_capacity(points.len());
    let mut represented = BTreeMap::new();
    for (key, index) in &mut points {
        *index = vertices.len();
        let xy = [
            (key[0] as f64 / 100_000.) as f32 as f64,
            (key[1] as f64 / 100_000.) as f32 as f64,
        ];
        let represented_key = xy.map(f64::to_bits);
        if let Some(&existing) = represented.get(&represented_key) {
            *index = existing;
            continue;
        }
        represented.insert(represented_key, *index);
        let mut height = surface.height(xy).unwrap_or(0.);
        if near(xy, 3e-5) {
            height = 0.;
        }
        vertices.push([
            xy[0].clamp(0., 256.),
            xy[1].clamp(0., 256.),
            (height / 5.).round_ties_even() * 5.,
        ]);
    }
    let faces = triangles
        .iter()
        .filter_map(|t| {
            let face = t.map(|p| points[&quantize(p)]);
            let before = (t[1][0] - t[0][0]) * (t[2][1] - t[0][1])
                - (t[1][1] - t[0][1]) * (t[2][0] - t[0][0]);
            let after = normal(&vertices, face)[2];
            (before * after > 0.).then_some(face)
        })
        .collect();
    let mut mesh = Mesh { vertices, faces };
    mesh.faces
        .retain(|f| normal(&mesh.vertices, *f)[2].abs() > 1e-10);
    for f in &mut mesh.faces {
        if normal(&mesh.vertices, *f)[2] < 0. {
            f.swap(1, 2);
        }
    }
    let mut unique = BTreeSet::new();
    mesh.faces.retain(|f| {
        let mut key = *f;
        key.sort_unstable();
        unique.insert(key)
    });
    if let Ok(path) = std::env::var("TERRAIN_DIAGNOSTIC_MESH") {
        std::fs::write(
            format!("{path}-coast-before-repair.json"),
            serde_json::to_vec(&mesh)?,
        )?;
    }
    if remove_rounding_slivers(&mut mesh).is_err() {
        remove_contained_coast_faces(api, &mut mesh)?;
        if mesh.validate_manifold().is_err() {
            repair_flat_overlap(api, &mut mesh)?;
            remove_rounding_slivers(&mut mesh)?;
        }
    }
    mesh.validate()?;
    Ok(mesh.compact())
}
fn flat_conflict_faces(mesh: &Mesh) -> Result<BTreeSet<usize>> {
    let mut edges: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (i, f) in mesh.faces.iter().enumerate() {
        for (a, b) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
            edges.entry((a.min(b), a.max(b))).or_default().push(i);
        }
    }
    let mut faces: BTreeSet<_> = edges
        .values()
        .filter(|v| v.len() > 2)
        .flatten()
        .copied()
        .collect();
    ensure!(
        faces
            .iter()
            .all(|i| mesh.faces[*i].iter().all(|v| mesh.vertices[*v][2] == 0.)),
        "海岸の平面以外で面が重なっています"
    );
    // 部分的な張り直しで隣の重なりを残さないよう、同じ海抜 0 m の面をまとめる。
    let mut incident = vec![Vec::new(); mesh.vertices.len()];
    for (i, f) in mesh.faces.iter().enumerate() {
        if f.iter().all(|v| mesh.vertices[*v][2] == 0.) {
            for v in f {
                incident[*v].push(i);
            }
        }
    }
    let mut queue: Vec<_> = faces.iter().copied().collect();
    let mut cursor = 0;
    while cursor < queue.len() {
        let i = queue[cursor];
        cursor += 1;
        for vertex in mesh.faces[i] {
            for &neighbor in &incident[vertex] {
                if faces.insert(neighbor) {
                    queue.push(neighbor);
                }
            }
        }
    }
    Ok(faces)
}
pub fn normalize_stored(api: &Geos, mesh: &Mesh) -> Result<Mesh> {
    let mut result = mesh.weld_identical();
    result.validate()?;
    if result.validate_manifold().is_err() {
        remove_contained_coast_faces(api, &mut result)?;
    }
    if result.validate_manifold().is_err() {
        flat_conflict_faces(&result)?;
        if remove_rounding_slivers(&mut result).is_err() {
            repair_flat_overlap(api, &mut result)?;
            remove_rounding_slivers(&mut result)?;
        }
    }
    result.validate_manifold()?;
    Ok(result.compact())
}
fn covered_height_error(mesh: &Mesh, inside: [usize; 3], outside: [usize; 3]) -> f64 {
    inside
        .iter()
        .map(|i| {
            let p = mesh.vertices[*i];
            let weights = crate::mesh::barycentric(&mesh.vertices, outside, [p[0], p[1]]);
            let predicted = (0..3)
                .map(|j| weights[j] * mesh.vertices[outside[j]][2])
                .sum::<f64>();
            (predicted - p[2]).abs()
        })
        .fold(0., f64::max)
}
fn remove_contained_coast_faces(api: &Geos, mesh: &mut Mesh) -> Result<usize> {
    let mut edges: BTreeMap<_, Vec<usize>> = BTreeMap::new();
    for (i, f) in mesh.faces.iter().enumerate() {
        for (a, b) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
            edges.entry((a.min(b), a.max(b))).or_default().push(i);
        }
    }
    let mut removed = BTreeSet::new();
    let mut cache = BTreeMap::new();
    for indices in edges.values().filter(|v| v.len() > 2) {
        for &i in indices {
            if !cache.contains_key(&i) {
                cache.insert(
                    i,
                    api.polygon(
                        &mesh.faces[i].map(|v| [mesh.vertices[v][0], mesh.vertices[v][1]]),
                    )?,
                );
            }
        }
    }
    for indices in edges.values().filter(|v| v.len() > 2) {
        for &inside in indices {
            if removed.contains(&inside) {
                continue;
            }
            for &outside in indices {
                if inside == outside || removed.contains(&outside) {
                    continue;
                }
                let a = mesh.faces[inside];
                let b = mesh.faces[outside];
                if !a.iter().chain(b.iter()).any(|i| mesh.vertices[*i][2] == 0.)
                    || covered_height_error(mesh, a, b) > 0.001
                {
                    continue;
                }
                if cache[&outside].covers(&cache[&inside])? {
                    removed.insert(inside);
                    break;
                }
            }
        }
    }
    if removed.is_empty() {
        return Ok(0);
    }
    let probes: Vec<_> = removed
        .iter()
        .flat_map(|i| {
            let f = mesh.faces[*i];
            let p = f.map(|v| mesh.vertices[v]);
            [
                p[0],
                p[1],
                p[2],
                [
                    (p[0][0] + p[1][0] + p[2][0]) / 3.,
                    (p[0][1] + p[1][1] + p[2][1]) / 3.,
                    (p[0][2] + p[1][2] + p[2][2]) / 3.,
                ],
            ]
        })
        .collect();
    let mut index = 0;
    mesh.faces.retain(|_| {
        let keep = !removed.contains(&index);
        index += 1;
        keep
    });
    let surface = Surface::new(mesh);
    let mut maximum = 0f64;
    for p in probes {
        let h = surface
            .height([p[0], p[1]])
            .ok_or_else(|| anyhow::anyhow!("包含された海岸の面を除くと範囲に穴ができます"))?;
        maximum = maximum.max((h - p[2]).abs());
    }
    ensure!(
        maximum <= 0.001 + 1e-8,
        "海岸の重複面が同じ斜面として一致しません"
    );
    eprintln!(
        "他の面に完全に含まれる海岸の重複を {} 面除きました。輪郭変更なし、海岸部の最大標高差 {maximum} m",
        removed.len()
    );
    Ok(removed.len())
}
fn repair_flat_overlap(api: &Geos, mesh: &mut Mesh) -> Result<()> {
    let affected = flat_conflict_faces(mesh)?;
    ensure!(!affected.is_empty(), "修復する海岸の面を特定できません");
    let polygons: Vec<_> = affected
        .iter()
        .map(|i| api.polygon(&mesh.faces[*i].map(|v| [mesh.vertices[v][0], mesh.vertices[v][1]])))
        .collect::<Result<_>>()?;
    let footprint = api.collection(&polygons)?.union()?;
    ensure!(footprint.is_valid()?, "修復前の海岸の輪郭が不正です");
    let mut triangles = Vec::new();
    for part in footprint.polygons()? {
        for ring in subdivide_triangulate(api, &part, 0)?.rings()? {
            ensure!(ring.len() == 4, "海岸の修復で三角形以外ができました");
            triangles.push([ring[0], ring[1], ring[2]]);
        }
    }
    let mut lookup: BTreeMap<_, _> = mesh
        .vertices
        .iter()
        .enumerate()
        .map(|(i, p)| (p.map(f64::to_bits), i))
        .collect();
    let mut replacements = Vec::new();
    let mut displacement = 0f64;
    for triangle in triangles {
        let mut face = [0; 3];
        for (j, p) in triangle.iter().enumerate() {
            let point = [p[0] as f32 as f64, p[1] as f32 as f64, 0.];
            displacement = displacement.max((point[0] - p[0]).hypot(point[1] - p[1]));
            face[j] = *lookup.entry(point.map(f64::to_bits)).or_insert_with(|| {
                let index = mesh.vertices.len();
                mesh.vertices.push(point);
                index
            });
        }
        let area = normal(&mesh.vertices, face)[2];
        if area == 0. {
            continue;
        }
        if area < 0. {
            face.swap(1, 2);
        }
        replacements.push(face);
    }
    let pieces: Vec<_> = replacements
        .iter()
        .map(|f| api.polygon(&f.map(|i| [mesh.vertices[i][0], mesh.vertices[i][1]])))
        .collect::<Result<_>>()?;
    let collection = api.collection(&pieces)?;
    let after = collection.union()?;
    let perimeter: f64 = footprint
        .rings()?
        .iter()
        .flat_map(|r| r.windows(2))
        .map(|p| (p[0][0] - p[1][0]).hypot(p[0][1] - p[1][1]))
        .sum();
    let tolerance = perimeter * displacement * 2. + 1e-10;
    let difference =
        footprint.difference(&after)?.area()? + after.difference(&footprint)?.area()?;
    ensure!(
        difference <= tolerance
            && after.is_valid()?
            && topology_at_precision(api, &footprint, displacement * 2. + 1e-10)?
                == topology_at_precision(api, &after, displacement * 2. + 1e-10)?,
        "海岸の重なり修復で輪郭が変わりました: 差分={difference}, 上限={tolerance}, 丸め={displacement}, 前={:?}, 後={:?}",
        footprint.counts()?,
        after.counts()?
    );
    ensure!(
        (collection.area()? - after.area()?).abs() <= tolerance,
        "修復した海岸の面が重なっています"
    );
    let before = mesh.faces.len();
    let mut i = 0;
    mesh.faces.retain(|_| {
        let keep = !affected.contains(&i);
        i += 1;
        keep
    });
    mesh.faces.extend(replacements);
    eprintln!(
        "海抜 0 m の海岸の重なりを輪郭を保って張り直しました: {before} -> {} 面、輪郭差分面積 {difference}",
        mesh.faces.len()
    );
    Ok(())
}
fn topology_at_precision(
    api: &Geos,
    geometry: &Geometry<'_>,
    precision: f64,
) -> Result<(usize, usize)> {
    fn width(g: &Geometry<'_>) -> Result<f64> {
        let rings = g.minimum_rectangle()?.rings()?;
        Ok(rings
            .iter()
            .flat_map(|r| r.windows(2))
            .map(|p| (p[0][0] - p[1][0]).hypot(p[0][1] - p[1][1]))
            .fold(f64::INFINITY, f64::min))
    }
    let mut result = (0, 0);
    for polygon in geometry.polygons()? {
        if width(&polygon)? <= precision {
            continue;
        }
        result.0 += 1;
        for ring in polygon.rings()?.into_iter().skip(1) {
            let hole = api.polygon(&ring[..ring.len() - 1])?;
            if width(&hole)? > precision {
                result.1 += 1;
            }
        }
    }
    Ok(result)
}
fn remove_rounding_slivers(mesh: &mut Mesh) -> Result<usize> {
    let mut edges = BTreeMap::new();
    for f in &mesh.faces {
        for (a, b) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
            *edges.entry((a.min(b), a.max(b))).or_insert(0usize) += 1;
        }
    }
    let mut removed = 0;
    mesh.faces.retain(|f| {
        let overused = [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])]
            .iter()
            .any(|&(a, b)| edges[&(a.min(b), a.max(b))] > 2);
        if !overused {
            return true;
        }
        let p = f.map(|i| mesh.vertices[i]);
        let longest = (0..3)
            .map(|i| (p[i][0] - p[(i + 1) % 3][0]).hypot(p[i][1] - p[(i + 1) % 3][1]))
            .fold(0., f64::max);
        // 正規化座標の丸めと f32 の丸めに収まる幅の重なりだけを除く。
        let max_coord = p
            .iter()
            .flat_map(|p| p[..2].iter())
            .map(|v| v.abs())
            .fold(0., f64::max) as f32;
        let ulp = (f32::from_bits(max_coord.to_bits() + 1) - max_coord) as f64;
        let keep = normal(&mesh.vertices, *f)[2].abs() > longest * (1e-5 + 2. * ulp);
        if !keep {
            removed += 1;
        }
        keep
    });
    edges.clear();
    for f in &mesh.faces {
        for (a, b) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
            *edges.entry((a.min(b), a.max(b))).or_insert(0usize) += 1;
        }
    }
    ensure!(
        edges.values().all(|count| *count <= 2),
        "保存精度の調整では修復できない海岸の面の重なりがあります"
    );
    if removed > 0 {
        eprintln!("海岸の座標丸めで重なった微小な三角形を {removed} 面除きました");
    }
    Ok(removed)
}
fn earcut(rings: &[Vec<[f64; 2]>]) -> Result<Vec<[[f64; 2]; 3]>> {
    let mut coords = Vec::new();
    let mut holes = Vec::new();
    for (index, ring) in rings.iter().enumerate() {
        ensure!(ring.len() >= 4, "輪郭の点が不足しています");
        if index > 0 {
            holes.push(coords.len() / 2);
        }
        for p in &ring[..ring.len() - 1] {
            coords.extend(*p);
        }
    }
    let indices = earcutr::earcut(&coords, &holes, 2)
        .map_err(|error| anyhow::anyhow!("三角形化に失敗しました: {error:?}"))?;
    Ok(indices
        .chunks_exact(3)
        .map(|f| std::array::from_fn(|i| [coords[f[i] * 2], coords[f[i] * 2 + 1]]))
        .collect())
}
fn subdivide_triangulate<'a>(
    api: &'a Geos,
    polygon: &Geometry<'a>,
    depth: u32,
) -> Result<Geometry<'a>> {
    if let Ok(result) = polygon.triangulate() {
        return Ok(result);
    }
    let rings = polygon.rings()?;
    if depth >= 16 {
        let faces = earcut(&rings)?;
        let pieces: Vec<_> = faces
            .iter()
            .map(|f| api.polygon(f))
            .collect::<Result<_>>()?;
        return api.collection(&pieces);
    }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for p in rings.iter().flatten() {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let axis = usize::from(hi[1] - lo[1] > hi[0] - lo[0]);
    let middle = (lo[axis] + hi[axis]) / 2.;
    let mut pieces = Vec::new();
    for side in 0..2 {
        let mut a = lo;
        let mut b = hi;
        if side == 0 {
            b[axis] = middle
        } else {
            a[axis] = middle
        }
        let rectangle = api.rectangle(a[0], a[1], b[0], b[1])?;
        for part in polygon.intersection(&rectangle)?.polygons()? {
            if !part.is_empty()? {
                pieces.push(subdivide_triangulate(api, &part, depth + 1)?);
            }
        }
    }
    api.collection(&pieces)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contained_slope_error() {
        let mut m = Mesh {
            vertices: vec![[0., 0., 0.], [1., 0., 15.], [0., 1., 0.], [0.1, 0.8, 0.]],
            faces: vec![[0, 1, 2], [0, 1, 3]],
        };
        assert!((covered_height_error(&m, [0, 1, 3], [0, 1, 2]) - 1.5).abs() < 1e-10);
        m.vertices[3][2] = 20.;
        assert!(covered_height_error(&m, [0, 1, 3], [0, 1, 2]) > 5.);
    }
    #[test]
    fn identical_geometry_welded() {
        let mesh = Mesh {
            vertices: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., -0., -0.]],
            faces: vec![[0, 1, 2], [3, 1, 2]],
        };
        let fixed = mesh.weld_identical();
        assert_eq!(fixed.vertices.len(), 3);
        assert_eq!(fixed.faces.len(), 1);
        fixed.validate_manifold().unwrap();
    }
    #[test]
    fn flat_patch_is_identified() {
        let m = Mesh {
            vertices: vec![
                [0., 0., 0.],
                [1., 0., 0.],
                [0., 1., 0.],
                [0., -1., 0.],
                [0.5, 0.5, 0.],
            ],
            faces: vec![[0, 1, 2], [0, 3, 1], [0, 1, 4]],
        };
        assert_eq!(flat_conflict_faces(&m).unwrap(), BTreeSet::from([0, 1, 2]));
        let mut steep = m;
        steep.vertices[4][2] = 5.;
        assert!(flat_conflict_faces(&steep).is_err());
    }
    #[test]
    fn rounding_sliver_only() {
        let mut mesh = Mesh {
            vertices: vec![
                [0., 0., 0.],
                [1., 0., 0.],
                [0., 1., 0.],
                [0., -1., 0.],
                [0.5, 0.000001, 0.],
            ],
            faces: vec![[0, 1, 2], [0, 3, 1], [0, 1, 4]],
        };
        assert_eq!(remove_rounding_slivers(&mut mesh).unwrap(), 1);
        assert_eq!(mesh.faces, vec![[0, 1, 2], [0, 3, 1]]);
    }
    #[test]
    fn broad_overlap_rejected() {
        let mut mesh = Mesh {
            vertices: vec![
                [0., 0., 0.],
                [1., 0., 0.],
                [0., 1., 0.],
                [0., -1., 0.],
                [0.5, 0.5, 0.],
            ],
            faces: vec![[0, 1, 2], [0, 3, 1], [0, 1, 4]],
        };
        assert!(remove_rounding_slivers(&mut mesh).is_err());
    }
    #[test]
    fn triangulate_hole() {
        let rings = vec![
            vec![[0., 0.], [10., 0.], [10., 10.], [0., 10.], [0., 0.]],
            vec![[3., 3.], [3., 7.], [7., 7.], [7., 3.], [3., 3.]],
        ];
        let triangles = earcut(&rings).unwrap();
        let area: f64 = triangles
            .iter()
            .map(|[a, b, c]| {
                ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.
            })
            .sum();
        assert!((area - 84.).abs() < 1e-10);
    }
}
pub fn clip(
    api: &Geos,
    source: &Mesh,
    land: &Geometry<'_>,
    level: u32,
    x: u32,
    y: u32,
) -> Result<Mesh> {
    let z = (level - 2).max(2);
    let world = 2. * std::f64::consts::PI * 6_378_137.;
    let resolution = world / (256. * 2f64.powi(z as i32));
    let left = x as f64 * 256. * resolution - world / 2.;
    let top = world / 2. - y as f64 * 256. * resolution;
    let rectangle = api.rectangle(left, top - 256. * resolution, left + 256. * resolution, top)?;
    if land.covers(&rectangle)? {
        return Ok(source.clone());
    }
    let local = land.intersection(&rectangle)?;
    if local.is_empty()? {
        return Ok(Mesh::default());
    }
    rebuild(api, source, &local.transform(left, top, resolution)?)
}
