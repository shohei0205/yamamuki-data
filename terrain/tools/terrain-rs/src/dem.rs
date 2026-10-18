use crate::{
    codec::{Grid, decode_grid},
    mesh::{Mesh, Surface},
};
use anyhow::{Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeSet;

pub fn read_grid(db: &Connection, z: u32, x: u32, y: u32) -> Result<Grid> {
    let mut values = vec![f32::NAN; 257 * 257];
    for dy in 0..2 {
        for dx in 0..2 {
            let blob: Option<Vec<u8>> = db
                .query_row(
                    "SELECT data FROM tiles WHERE z=?1 AND x=?2 AND y=?3",
                    (z, x + dx, y + dy),
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(blob) = blob {
                let tile = decode_grid(&blob)?;
                ensure!(
                    tile.rows == 256 && tile.columns == 256,
                    "元標高タイルは 256 四方が必要です"
                );
                for row in 0..if dy == 0 { 256 } else { 1 } {
                    for col in 0..if dx == 0 { 256 } else { 1 } {
                        values[(dy as usize * 256 + row) * 257 + dx as usize * 256 + col] =
                            tile.values[row * 256 + col];
                    }
                }
            }
        }
    }
    Ok(Grid {
        rows: 257,
        columns: 257,
        values,
    })
}
#[cfg(test)]
pub fn grid_mesh(grid: &Grid) -> Mesh {
    let mut mesh = Mesh {
        vertices: grid
            .values
            .iter()
            .enumerate()
            .map(|(i, z)| {
                [
                    (i % grid.columns) as f64,
                    (i / grid.columns) as f64,
                    *z as f64,
                ]
            })
            .collect(),
        faces: Vec::new(),
    };
    for y in 0..grid.rows - 1 {
        for x in 0..grid.columns - 1 {
            let a = y * grid.columns + x;
            let b = a + 1;
            let c = a + grid.columns;
            let d = c + 1;
            if [a, b, c, d].iter().all(|i| grid.values[*i].is_finite()) {
                mesh.faces.extend([[a, b, c], [b, d, c]]);
            }
        }
    }
    mesh.compact()
}
fn profile(values: &[f32], tolerance: f64) -> BTreeSet<usize> {
    let mut result = BTreeSet::new();
    let mut start = 0;
    while start < values.len() {
        if !values[start].is_finite() {
            start += 1;
            continue;
        }
        let mut end = start;
        while end + 1 < values.len() && values[end + 1].is_finite() {
            end += 1
        }
        let mut stack = vec![(start, end)];
        while let Some((a, b)) = stack.pop() {
            result.insert(a);
            result.insert(b);
            if b - a < 2 {
                continue;
            }
            let mut worst = 0.;
            let mut chosen = a;
            for i in a + 1..b {
                let predicted = values[a] as f64
                    + (values[b] - values[a]) as f64 * (i - a) as f64 / (b - a) as f64;
                let error = (predicted - values[i] as f64).abs();
                if error > worst {
                    worst = error;
                    chosen = i
                }
            }
            if worst > tolerance * 0.5 {
                stack.push((a, chosen));
                stack.push((chosen, b));
            }
        }
        start = end + 1;
    }
    result
}
pub fn simplify(grid: &Grid, tolerance: f64) -> Result<Mesh> {
    ensure!(
        grid.rows == grid.columns && grid.rows >= 2 && tolerance.is_finite() && tolerance >= 0.,
        "標高配列と誤差上限が不正です"
    );
    let size = grid.rows;
    let n = size - 1;
    let v = &grid.values;
    let valid = |x: usize, y: usize| v[y * size + x].is_finite();
    let mut selected = BTreeSet::new();
    for i in profile(&v[..size], tolerance) {
        selected.insert(i);
    }
    for i in profile(&v[n * size..], tolerance) {
        selected.insert(n * size + i);
    }
    for (x, values) in [
        (0, (0..size).map(|y| v[y * size]).collect::<Vec<_>>()),
        (n, (0..size).map(|y| v[y * size + n]).collect()),
    ] {
        for i in profile(&values, tolerance) {
            selected.insert(i * size + x);
        }
    }
    let mut interior = Vec::new();
    for y in 1..n {
        for x in 1..n {
            if valid(x, y) {
                let i = y * size + x;
                interior.push(i);
                if !valid(x - 1, y) || !valid(x + 1, y) || !valid(x, y - 1) || !valid(x, y + 1) {
                    selected.insert(i);
                }
            }
        }
    }
    if let Some(i) = interior.iter().max_by(|a, b| v[**a].total_cmp(&v[**b])) {
        selected.insert(*i);
    }
    if let Some(i) = interior.iter().min_by(|a, b| v[**a].total_cmp(&v[**b])) {
        selected.insert(*i);
    }
    if selected.len() < 3 {
        return Ok(Mesh::default());
    }
    for _ in 0..200 {
        let ids: Vec<_> = selected.iter().copied().collect();
        let points: Vec<_> = ids
            .iter()
            .map(|i| delaunator::Point {
                x: (i % size) as f64,
                y: (i / size) as f64,
            })
            .collect();
        let tri = delaunator::triangulate(&points);
        if tri.triangles.is_empty() {
            return Ok(Mesh::default());
        }
        let mut mesh = Mesh {
            vertices: ids
                .iter()
                .map(|i| [(i % size) as f64, (i / size) as f64, v[*i] as f64])
                .collect(),
            faces: tri
                .triangles
                .chunks_exact(3)
                .map(|f| [f[0], f[2], f[1]])
                .collect(),
        };
        let surface = Surface::new(&mesh);
        let mut worst = vec![None::<(f64, usize)>; mesh.faces.len()];
        for (i, height) in v.iter().enumerate() {
            if !height.is_finite() {
                continue;
            }
            let p = [(i % size) as f64, (i / size) as f64];
            if let Some((face, predicted)) = surface.sample(p) {
                let error = (predicted - *height as f64).abs();
                if error > tolerance + 0.001 && worst[face].is_none_or(|(old, _)| error > old) {
                    worst[face] = Some((error, i));
                }
            } else {
                bail!("有限な標高が三角形の範囲外です")
            }
        }
        let mut additions: Vec<_> = worst.into_iter().flatten().collect();
        if additions.is_empty() {
            // 欠損セルをまたぐ面は採用しない。セルと面が重なる範囲を調べる。
            mesh.faces.retain(|f| {
                let [a, b, c] = f.map(|i| mesh.vertices[i]);
                let minx = a[0].min(b[0]).min(c[0]).floor() as usize;
                let maxx = a[0].max(b[0]).max(c[0]).ceil() as usize;
                let miny = a[1].min(b[1]).min(c[1]).floor() as usize;
                let maxy = a[1].max(b[1]).max(c[1]).ceil() as usize;
                for y in miny..maxy.min(n) {
                    for x in minx..maxx.min(n) {
                        if !(valid(x, y)
                            && valid(x + 1, y)
                            && valid(x, y + 1)
                            && valid(x + 1, y + 1))
                            && overlap([a, b, c], x as f64, y as f64)
                        {
                            return false;
                        }
                    }
                }
                true
            });
            return Ok(mesh.compact());
        }
        additions.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let before = selected.len();
        for (_, i) in additions.into_iter().take(512) {
            selected.insert(i);
        }
        ensure!(
            before < selected.len(),
            "標高誤差を減らす頂点を追加できません"
        );
    }
    bail!("不規則メッシュの生成が収束しません")
}
fn overlap(points: [[f64; 3]; 3], x: f64, y: f64) -> bool {
    // 分離軸で、欠損セルとの正の面積の交差を判定する。
    let square = [[x, y], [x + 1., y], [x + 1., y + 1.], [x, y + 1.]];
    let mut axes = vec![[1., 0.], [0., 1.]];
    for i in 0..3 {
        let a = points[i];
        let b = points[(i + 1) % 3];
        axes.push([b[1] - a[1], a[0] - b[0]]);
    }
    for axis in axes {
        let mut tmin = f64::INFINITY;
        let mut tmax = f64::NEG_INFINITY;
        for p in points {
            let s = p[0] * axis[0] + p[1] * axis[1];
            tmin = tmin.min(s);
            tmax = tmax.max(s);
        }
        let mut smin = f64::INFINITY;
        let mut smax = f64::NEG_INFINITY;
        for p in square {
            let s = p[0] * axis[0] + p[1] * axis[1];
            smin = smin.min(s);
            smax = smax.max(s);
        }
        if tmax <= smin + 1e-9 || smax <= tmin + 1e-9 {
            return false;
        }
    }
    true
}
pub fn extrema(grid: &Grid, meters: f64) -> Vec<crate::mesh::Point> {
    let size = grid.columns;
    let radius = (250. / meters).round_ties_even().max(1.) as usize;
    let radius = radius.min(size);
    let n = grid.values.len();
    let mut horizontal = vec![[f32::INFINITY, f32::NEG_INFINITY]; n];
    let mut both = horizontal.clone();
    for y in 0..grid.rows {
        for x in 0..size {
            for xx in x.saturating_sub(radius)..=(x + radius).min(size - 1) {
                let z = grid.values[y * size + xx];
                if z.is_finite() {
                    horizontal[y * size + x][0] = horizontal[y * size + x][0].min(z);
                    horizontal[y * size + x][1] = horizontal[y * size + x][1].max(z);
                }
            }
        }
    }
    for y in 0..grid.rows {
        for x in 0..size {
            for yy in y.saturating_sub(radius)..=(y + radius).min(grid.rows - 1) {
                both[y * size + x][0] = both[y * size + x][0].min(horizontal[yy * size + x][0]);
                both[y * size + x][1] = both[y * size + x][1].max(horizontal[yy * size + x][1]);
            }
        }
    }
    let mut result = Vec::new();
    for high in [false, true] {
        let mut visited = vec![false; n];
        let candidate = |i: usize| {
            grid.values[i].is_finite()
                && both[i][1] - both[i][0] >= 40.
                && grid.values[i] == both[i][usize::from(high)]
        };
        for start in 0..n {
            if visited[start] || !candidate(start) {
                continue;
            }
            visited[start] = true;
            let mut component = vec![start];
            let mut cursor = 0;
            while cursor < component.len() {
                let i = component[cursor];
                cursor += 1;
                let x = i % size;
                let y = i / size;
                for j in [
                    if x > 0 { Some(i - 1) } else { None },
                    if x + 1 < size { Some(i + 1) } else { None },
                    if y > 0 { Some(i - size) } else { None },
                    if y + 1 < grid.rows {
                        Some(i + size)
                    } else {
                        None
                    },
                ]
                .into_iter()
                .flatten()
                {
                    if !visited[j] && candidate(j) {
                        visited[j] = true;
                        component.push(j);
                    }
                }
            }
            component.sort_unstable();
            let i = component[component.len() / 2];
            result.push([(i % size) as f64, (i / size) as f64, grid.values[i] as f64]);
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approximation() {
        let values = (0..33 * 33)
            .map(|i| {
                let x = (i % 33) as f64;
                let y = (i / 33) as f64;
                (200. * (-((x - 15.).powi(2) + (y - 14.).powi(2)) / 50.).exp()) as f32
            })
            .collect();
        let grid = Grid {
            rows: 33,
            columns: 33,
            values,
        };
        let mesh = simplify(&grid, 5.).unwrap();
        assert!(mesh.faces.len() < 2048);
        mesh.validate().unwrap();
        let surface = Surface::new(&mesh);
        for (i, z) in grid.values.iter().enumerate() {
            assert!(
                (surface.height([(i % 33) as f64, (i / 33) as f64]).unwrap() - *z as f64).abs()
                    <= 5.001
            );
        }
    }
    #[test]
    fn missing_cell() {
        let mut grid = Grid {
            rows: 9,
            columns: 9,
            values: vec![10.; 81],
        };
        grid.values[4 * 9 + 4] = f32::NAN;
        let mesh = simplify(&grid, 5.).unwrap();
        assert!(Surface::new(&mesh).height([4., 4.]).is_none());
    }
    #[test]
    fn raw_missing() {
        let mut grid = Grid {
            rows: 257,
            columns: 257,
            values: vec![f32::NAN; 257 * 257],
        };
        for y in 0..3 {
            for x in 0..3 {
                grid.values[y * 257 + x] = 100.;
            }
        }
        let mesh = grid_mesh(&grid);
        assert_eq!(mesh.faces.len(), 8);
        mesh.validate().unwrap();
    }
}
