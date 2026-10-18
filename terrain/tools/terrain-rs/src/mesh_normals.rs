//! 各 LOD の三角形から滑らかな頂点法線を求め、地上座標の画像へ保存する。
use crate::{
    bench_mesh::{WorldMesh, read_mesh},
    bench_normal, bench_png, write_json,
};
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{collections::HashMap, fs, path::Path};

pub(crate) fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    if length < 1e-15 {
        [0., 0., 1.]
    } else {
        v.map(|x| x / length)
    }
}
fn key(p: [f64; 3]) -> [i64; 3] {
    p.map(|x| (x * 10000.).round() as i64)
}
fn vertex_normals(mesh: &WorldMesh) -> Vec<[f64; 3]> {
    let mut sums = HashMap::<[i64; 3], [f64; 3]>::new();
    for face in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [0, 1, 2].map(|i| mesh.vertices[face[i] as usize]);
        let u = [0, 1, 2].map(|i| b[i] - a[i]);
        let v = [0, 1, 2].map(|i| c[i] - a[i]);
        let mut n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        if n[2] < 0. {
            n = n.map(|x| -x);
        }
        // タイル境界で重複した頂点もまとめ、面積で重みを付ける。
        for p in [a, b, c] {
            let sum = sums.entry(key(p)).or_default();
            for i in 0..3 {
                sum[i] += n[i];
            }
        }
    }
    mesh.vertices
        .iter()
        .map(|p| unit(*sums.get(&key(*p)).unwrap_or(&[0., 0., 1.])))
        .collect()
}
fn barycentric(p: [f64; 2], t: [[f64; 2]; 3]) -> Option<[f64; 3]> {
    let [a, b, c] = t;
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if d.abs() < 1e-14 {
        return None;
    }
    let u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    let w = [u, v, 1. - u - v];
    w.iter().all(|x| *x >= -1e-8).then_some(w)
}
pub(crate) fn raster(mesh: &WorldMesh, meta: &Value, size: usize) -> (Vec<[f64; 3]>, usize) {
    let normals = vertex_normals(mesh);
    let meters = meta["meters"].as_f64().unwrap();
    let width = meta["width"].as_f64().unwrap();
    let height = meta["height"].as_f64().unwrap();
    let positions: Vec<_> = mesh
        .vertices
        .iter()
        .map(|p| {
            [
                (p[0] / meters + (width - 1.) * 0.5) / (width - 1.) * (size - 1) as f64,
                (-p[1] / meters + (height - 1.) * 0.5) / (height - 1.) * (size - 1) as f64,
            ]
        })
        .collect();
    let mut out = vec![[0., 0., 1.]; size * size];
    let mut depth = vec![f64::NEG_INFINITY; size * size];
    for face in mesh.indices.chunks_exact(3) {
        let t = [0, 1, 2].map(|i| positions[face[i] as usize]);
        let minx = t
            .iter()
            .map(|p| p[0])
            .fold(f64::INFINITY, f64::min)
            .ceil()
            .max(0.) as usize;
        let maxx = t
            .iter()
            .map(|p| p[0])
            .fold(f64::NEG_INFINITY, f64::max)
            .floor()
            .min((size - 1) as f64);
        let miny = t
            .iter()
            .map(|p| p[1])
            .fold(f64::INFINITY, f64::min)
            .ceil()
            .max(0.) as usize;
        let maxy = t
            .iter()
            .map(|p| p[1])
            .fold(f64::NEG_INFINITY, f64::max)
            .floor()
            .min((size - 1) as f64);
        if maxx < minx as f64 || maxy < miny as f64 {
            continue;
        }
        for y in miny..=maxy as usize {
            for x in minx..=maxx as usize {
                if let Some(w) = barycentric([x as f64, y as f64], t) {
                    let z = (0..3)
                        .map(|j| w[j] * mesh.vertices[face[j] as usize][2])
                        .sum();
                    let i = y * size + x;
                    if z > depth[i] {
                        depth[i] = z;
                        out[i] = unit([0, 1, 2].map(|axis| {
                            (0..3).map(|j| w[j] * normals[face[j] as usize][axis]).sum()
                        }));
                    }
                }
            }
        }
    }
    let covered = depth.iter().filter(|z| z.is_finite()).count();
    // 描画域の縁では隣接画素の法線を使い、画像の線形補間で縁だけ平坦にならないようにする。
    let mut valid: Vec<_> = depth.iter().map(|z| z.is_finite()).collect();
    for reverse in [false, true] {
        for k in 0..size * size {
            let i = if reverse { size * size - 1 - k } else { k };
            if valid[i] {
                continue;
            }
            let x = i % size;
            let y = i / size;
            for j in [
                if x > 0 { Some(i - 1) } else { None },
                if x + 1 < size { Some(i + 1) } else { None },
                if y > 0 { Some(i - size) } else { None },
                if y + 1 < size { Some(i + size) } else { None },
            ]
            .into_iter()
            .flatten()
            {
                if valid[j] {
                    out[i] = out[j];
                    valid[i] = true;
                    break;
                }
            }
        }
    }
    (out, covered)
}
fn resize(source: &[[f64; 3]], width: usize, size: usize) -> Vec<[u8; 4]> {
    (0..size * size)
        .map(|i| {
            let x = (i % size) as f64 * (width - 1) as f64 / (size - 1) as f64;
            let y = (i / size) as f64 * (width - 1) as f64 / (size - 1) as f64;
            let (x0, y0) = (x.floor() as usize, y.floor() as usize);
            let (fx, fy) = (x.fract(), y.fract());
            let samples = [
                (x0, y0, (1. - fx) * (1. - fy)),
                ((x0 + 1).min(width - 1), y0, fx * (1. - fy)),
                (x0, (y0 + 1).min(width - 1), (1. - fx) * fy),
                ((x0 + 1).min(width - 1), (y0 + 1).min(width - 1), fx * fy),
            ];
            let n = unit([0, 1, 2].map(|axis| {
                samples
                    .iter()
                    .map(|(x, y, w)| source[y * width + x][axis] * w)
                    .sum()
            }));
            [
                (127.5 * (n[0] + 1.)).round() as u8,
                (127.5 * (n[1] + 1.)).round() as u8,
                255,
                255,
            ]
        })
        .collect()
}
pub fn run(root: &Path, output: &Path, polygon: &Path, encoder: &Path) -> Result<()> {
    let report: Value = serde_json::from_slice(&fs::read(root.join("levels.json"))?)?;
    let meta = report["levels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["level"] == 14)
        .unwrap();
    let db = Connection::open_with_flags(polygon, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let directory = output.join("mesh-normals");
    fs::create_dir_all(&directory)?;
    let mut levels = Vec::new();
    // 計測と全圧縮が完了したレベルは再利用し、中断したレベルから続ける。
    let checkpoint = directory.join("report.json");
    if checkpoint.exists() {
        let previous: Value = serde_json::from_slice(&fs::read(&checkpoint)?)?;
        if let Some(rows) = previous["levels"].as_array() {
            levels = rows.clone();
        }
    }
    for level in 4..=14 {
        if levels.iter().any(|row| row["level"] == level) {
            continue;
        }
        let mesh = read_mesh(&db, meta, level)?;
        let (pixels, covered) = raster(&mesh, meta, 2048);
        let folder = directory.join(format!("L{level}"));
        let source = folder.join("source");
        fs::create_dir_all(source.join("normals"))?;
        let mut variants = Vec::new();
        for size in [2048, 1024, 512, 256, 128, 64, 32, 16, 8, 4] {
            let path = source.join(format!("normals/{size}.png"));
            bench_png::write(&path, size, size, &resize(&pixels, 2048, size))?;
            variants.push(json!({"size":size,"bytes":fs::metadata(path)?.len(),"url":format!("/mesh-normals/L{level}/source/normals/{size}.png")}));
        }
        write_json(
            &source.join("levels.json"),
            &json!({"normalVariants":variants}),
        )?;
        let mut compressed = bench_normal::run(&source, &folder, encoder)?;
        for row in compressed["results"].as_array_mut().unwrap() {
            for field in ["url", "zstdUrl", "previewUrl"] {
                row[field] = json!(row[field].as_str().unwrap().replace(
                    "/codecs/normals/",
                    &format!("/mesh-normals/L{level}/normals/")
                ));
            }
        }
        levels.push(json!({"level":level,"vertices":mesh.vertices.len(),"triangles":mesh.indices.len()/3,"coveredPixels":covered,"rasterSize":2048,"normalVariants":variants,"compressed":compressed["results"]}));
        write_json(
            &directory.join("report.json"),
            &json!({"levels":levels,"state":if level==14 {"complete"} else {"running"},"method":"各 LOD の面積重み付き頂点法線。境界の重複頂点を共有し、三角形内で補間。東・北・上の座標系。詳細標高の凹凸を追加で焼き込まない。"}),
        )?;
        println!(
            "{}",
            json!({"part":"mesh-normals","level":level,"state":"complete"})
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_boundary_vertices_share_normals() {
        let mesh = WorldMesh {
            vertices: vec![
                [0., 0., 0.],
                [1., 0., 0.],
                [0., 1., 0.],
                [0., 0., 0.],
                [0., 1., 0.],
                [-1., 0., 1.],
            ],
            indices: vec![0, 1, 2, 3, 4, 5],
            source_bytes: 0,
            source_decode_ms: 0.,
        };
        let n = vertex_normals(&mesh);
        assert_eq!(n[0], n[3]);
        assert_eq!(n[2], n[4]);
        assert!(n[0][0] > 0.);
    }
    #[test]
    fn raster_preserves_slope_and_north_direction() {
        let mesh = WorldMesh {
            vertices: vec![[-1., -1., -3.], [1., -1., -1.], [-1., 1., 1.], [1., 1., 3.]],
            indices: vec![0, 1, 2, 1, 3, 2],
            source_bytes: 0,
            source_decode_ms: 0.,
        };
        let (r, covered) = raster(&mesh, &json!({"meters":1,"width":3,"height":3}), 3);
        assert_eq!(covered, 9);
        let expected = unit([-1., -2., 1.]);
        for n in r {
            for i in 0..3 {
                assert!((n[i] - expected[i]).abs() < 1e-12);
            }
        }
    }
    #[test]
    fn triangle_weights_and_outside() {
        let t = [[0., 0.], [2., 0.], [0., 2.]];
        assert_eq!(barycentric([0.5, 0.5], t), Some([0.5, 0.25, 0.25]));
        assert!(barycentric([2., 2.], t).is_none());
    }
    #[test]
    fn slope_has_correct_east_north_direction_with_reversed_faces() {
        let mesh = WorldMesh {
            vertices: vec![[0., 0., 0.], [1., 0., 1.], [0., 1., 2.]],
            indices: vec![0, 2, 1],
            source_bytes: 0,
            source_decode_ms: 0.,
        };
        let normals = vertex_normals(&mesh);
        let expected = unit([-1., -2., 1.]);
        for n in normals {
            for i in 0..3 {
                assert!((n[i] - expected[i]).abs() < 1e-12);
            }
        }
    }
}
