//! 詳細法線を各 LOD の接線座標へ変換し、細かな陰影を保つ。
use crate::{
    bench_mesh::read_mesh,
    bench_normal, bench_png,
    mesh_normals::{raster, unit},
    write_json,
};
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{fs, path::Path};
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn frame(n: [f64; 3]) -> [[f64; 3]; 3] {
    let east = [1., 0., 0.];
    let p = dot(east, n);
    let mut t = [0, 1, 2].map(|i| east[i] - p * n[i]);
    if dot(t, t) < 1e-6 {
        let north = [0., 1., 0.];
        let p = dot(north, n);
        t = [0, 1, 2].map(|i| north[i] - p * n[i]);
    }
    let t = unit(t);
    [t, cross(n, t), n]
}
fn decode(p: [u8; 4]) -> [f64; 3] {
    let x = p[0] as f64 / 127.5 - 1.;
    let y = p[1] as f64 / 127.5 - 1.;
    unit([x, y, (1. - x * x - y * y).max(0.).sqrt()])
}
fn pack(n: [f64; 3]) -> [u8; 4] {
    [
        (127.5 * (n[0] + 1.)).round() as u8,
        (127.5 * (n[1] + 1.)).round() as u8,
        (127.5 * (n[2] + 1.)).round() as u8,
        255,
    ]
}
fn unpack(p: [u8; 4]) -> [f64; 3] {
    unit([0, 1, 2].map(|i| p[i] as f64 / 127.5 - 1.))
}
fn sample(pixels: &[[f64; 3]], size: usize, x: f64, y: f64) -> [f64; 3] {
    let x = x.clamp(0., (size - 1) as f64);
    let y = y.clamp(0., (size - 1) as f64);
    let (a, b) = (x.floor() as usize, y.floor() as usize);
    let (fx, fy) = (x.fract(), y.fract());
    let samples = [
        (a, b, (1. - fx) * (1. - fy)),
        ((a + 1).min(size - 1), b, fx * (1. - fy)),
        (a, (b + 1).min(size - 1), (1. - fx) * fy),
        ((a + 1).min(size - 1), (b + 1).min(size - 1), fx * fy),
    ];
    unit([0, 1, 2].map(|axis| {
        samples
            .iter()
            .map(|(x, y, w)| pixels[y * size + x][axis] * w)
            .sum()
    }))
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
    let directory = output.join("baked-normals");
    fs::create_dir_all(&directory)?;
    let mut levels = Vec::new();
    for level in 4..=14 {
        let mut mesh = read_mesh(&db, meta, level)?;
        // 描画に使う Float32 頂点から、ブラウザーと同じ基準法線を求める。
        for p in &mut mesh.vertices {
            for v in p {
                *v = *v as f32 as f64;
            }
        }
        let (base, covered) = raster(&mesh, meta, 2048);
        let folder = directory.join(format!("L{level}"));
        let source = folder.join("source");
        fs::create_dir_all(source.join("normals"))?;
        let mut variants = Vec::new();
        for size in [2048, 1024, 512, 256, 128, 64, 32, 16, 8, 4] {
            // 比較元と同じ解像度の詳細法線を使い、解像度差と焼き込み誤差を分ける。
            let detail = bench_png::read(&root.join(format!("normals/{size}.png")))?;
            let mut pixels = Vec::with_capacity(size * size);
            let mut errors = Vec::with_capacity(size * size);
            let mut negative_z = 0;
            for (i, p) in detail.pixels.iter().enumerate() {
                let n = sample(
                    &base,
                    2048,
                    (i % size) as f64 * 2047. / (size - 1) as f64,
                    (i / size) as f64 * 2047. / (size - 1) as f64,
                );
                let f = frame(n);
                let d = decode(*p);
                let local = f.map(|v| dot(v, d));
                if local[2] < 0. {
                    negative_z += 1;
                }
                let packed = pack(local);
                let restored = unpack(packed);
                let world =
                    unit([0, 1, 2].map(|axis| (0..3).map(|j| f[j][axis] * restored[j]).sum()));
                errors.push(dot(world, d).clamp(-1., 1.).acos().to_degrees());
                pixels.push(packed);
            }
            errors.sort_unstable_by(f64::total_cmp);
            let path = source.join(format!("normals/{size}.png"));
            bench_png::write(&path, size, size, &pixels)?;
            variants.push(json!({"size":size,"bytes":fs::metadata(path)?.len(),"url":format!("/baked-normals/L{level}/source/normals/{size}.png"),"pngBakeP95Degrees":errors[(errors.len()-1)*95/100],"pngBakeMaxDegrees":errors[errors.len()-1],"negativeTangentZPixels":negative_z}));
        }
        write_json(
            &source.join("levels.json"),
            &json!({"normalVariants":variants}),
        )?;
        let mut compressed = bench_normal::run_tangent(&source, &folder, encoder)?;
        for row in compressed["results"].as_array_mut().unwrap() {
            for field in ["url", "zstdUrl", "previewUrl"] {
                row[field] = json!(row[field].as_str().unwrap().replace(
                    "/codecs/normals/",
                    &format!("/baked-normals/L{level}/normals/")
                ));
            }
        }
        levels.push(json!({"level":level,"normalVariants":variants,"compressed":compressed["results"],"coveredPixels":covered}));
        write_json(
            &directory.join("report.json"),
            &json!({"levels":levels,"state":if level==14 {"complete"} else {"running"},"method":"詳細標高の法線を LOD の補間頂点法線の接線座標へ焼き込む。符号付き XYZ を RGB に保存し、負の Z も保持。描画時に同じ基準から地上座標へ復元。"}),
        )?;
        println!(
            "{}",
            json!({"part":"baked-normals","level":level,"state":"complete"})
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn steep_base_and_negative_z_roundtrip() {
        for n in [unit([1., 0., 0.]), unit([1., 2., 1.]), unit([-2., 0., 1.])] {
            let f = frame(n);
            for d in [unit([0., 0., 1.]), unit([-1., 0., 0.1]), unit([1., 2., 1.])] {
                let local = f.map(|a| dot(a, d));
                let restored = unit([0, 1, 2].map(|i| (0..3).map(|j| f[j][i] * local[j]).sum()));
                assert!(dot(restored, d) > 1. - 1e-12);
                let q = unpack(pack(local));
                let world = unit([0, 1, 2].map(|i| (0..3).map(|j| f[j][i] * q[j]).sum()));
                assert!(dot(world, d) > 0.9999);
            }
        }
    }
}
