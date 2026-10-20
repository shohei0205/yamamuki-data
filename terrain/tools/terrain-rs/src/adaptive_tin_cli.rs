//! PNG標高マップから局所更新型Adaptive TINを生成する独立コマンド。
mod adaptive_tin;
use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, ValueEnum};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Write},
    path::PathBuf,
    time::Instant,
};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Encoding {
    Gray,
    Gsi,
}
#[derive(Parser)]
#[command(about = "PNG標高マップから最大誤差優先のAdaptive TINを生成")]
struct Args {
    #[arg(long)]
    input: PathBuf,
    /// 共有境界を先に固定して生成する。
    #[arg(long)]
    seamless: bool,
    /// 固定シードの共有辺に、それ以外の頂点を追加しない。
    #[arg(long)]
    lock_seed_boundary: bool,
    /// 各画素の許容最大標高誤差を表すグレースケールPNG。
    #[arg(long)]
    error_limits: Option<PathBuf>,
    /// 頂点からの被覆半径を画素単位で示す数値PNG。許容誤差PNGと併用する。
    #[arg(long)]
    mesh_sizes: Option<PathBuf>,
    /// 隣接面の最長辺の比がこの値を超えた場所だけを追加頂点で補う。
    #[arg(long)]
    neighbor_ratio: Option<f64>,
    #[arg(long, default_value_t = 0.01)]
    mesh_size_scale: f64,
    /// 元の標高格子のXYで、山頂として残す頂点のJSON配列。
    #[arg(long)]
    seed_points: Option<PathBuf>,
    #[arg(long, default_value_t = 0.01)]
    error_limit_scale: f64,
    /// 比較用の挿入履歴を生成する頂点上限の列。
    #[arg(long, value_delimiter = ',')]
    progressive_vertices: Vec<usize>,
    #[arg(long, default_value_t = 25.0)]
    boundary_error: f64,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value_t = 10000)]
    max_vertices: usize,
    /// 誤差内でもこの頂点数まで追加する。平面を丸め誤差内で再現できた場合は停止。
    #[arg(long, default_value_t = 4)]
    min_vertices: usize,
    #[arg(long, default_value_t = 5.0)]
    max_error: f64,
    #[arg(long,value_enum,default_value_t=Encoding::Gray)]
    encoding: Encoding,
    /// グレースケール値から標高への倍率。GSIでは標高メートルに追加適用。
    #[arg(long, default_value_t = 1.0, allow_hyphen_values = true)]
    scale: f64,
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    offset: f64,
    /// 出力XYの1画素の間隔。Delaunay判定は正方画素を前提とする。
    #[arg(long, default_value_t = 1.0)]
    spacing: f64,
}

fn read_png(
    path: &std::path::Path,
    encoding: Encoding,
    scale: f64,
    offset: f64,
) -> Result<(usize, usize, Vec<f64>)> {
    ensure!(
        scale.is_finite() && offset.is_finite(),
        "倍率とオフセットは有限値が必要です"
    );
    let decoder = png::Decoder::new(BufReader::new(File::open(path)?));
    let mut reader = decoder.read_info()?;
    ensure!(
        reader.info().animation_control.is_none(),
        "アニメーションPNGは扱いません"
    );
    let mut data = vec![0; reader.output_buffer_size().context("PNGが大きすぎます")?];
    let info = reader.next_frame(&mut data)?;
    let data = &data[..info.buffer_size()];
    use png::{BitDepth, ColorType};
    let values: Vec<f64> = match (encoding, info.color_type, info.bit_depth) {
        (Encoding::Gray, ColorType::Grayscale, BitDepth::Eight) => {
            data.iter().map(|&v| v as f64).collect()
        }
        (Encoding::Gray, ColorType::Grayscale, BitDepth::Sixteen) => data
            .chunks_exact(2)
            .map(|v| u16::from_be_bytes([v[0], v[1]]) as f64)
            .collect(),
        (Encoding::Gsi, ColorType::Rgb, BitDepth::Eight) => data
            .chunks_exact(3)
            .map(|v| {
                let n = ((v[0] as i32) << 16) | ((v[1] as i32) << 8) | v[2] as i32;
                if n == 0x800000 {
                    f64::NAN
                } else {
                    (if n >= 0x800000 { n - 0x1000000 } else { n }) as f64 * 0.01
                }
            })
            .collect(),
        _ => bail!("grayは8/16ビットグレースケールPNG、gsiは8ビットRGB標高PNGが必要です"),
    };
    Ok((
        info.width as usize,
        info.height as usize,
        values.into_iter().map(|z| z * scale + offset).collect(),
    ))
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.spacing.is_finite() && args.spacing > 0.,
        "画素間隔は正の有限値が必要です"
    );
    ensure!(
        args.input != args.output,
        "入力と出力は別のパスにしてください"
    );
    if args.output.exists() {
        ensure!(
            std::fs::canonicalize(&args.input)? != std::fs::canonicalize(&args.output)?,
            "入力と出力は別のファイルにしてください"
        );
    }
    let start = Instant::now();
    let (width, height, values) = read_png(&args.input, args.encoding, args.scale, args.offset)?;
    ensure!(
        args.error_limits.is_none() || (!args.seamless && args.progressive_vertices.is_empty()),
        "場所ごとの誤差は独立TINの通常生成で指定してください"
    );
    ensure!(
        args.seed_points.is_none() || args.progressive_vertices.is_empty(),
        "山頂の固定と挿入履歴の併用は未対応です"
    );
    ensure!(
        args.mesh_sizes.is_none()
            || (args.error_limits.is_some()
                && !args.seamless
                && args.progressive_vertices.is_empty()),
        "メッシュ寸法は独立TINの許容誤差PNGと併用してください"
    );
    ensure!(
        args.neighbor_ratio.is_none()
            || (args.error_limits.is_some()
                && args.mesh_sizes.is_none()
                && !args.seamless
                && args.progressive_vertices.is_empty()),
        "隣接面調整は許容誤差PNGのみと併用してください"
    );
    if !args.progressive_vertices.is_empty() {
        let mut mesh = adaptive_tin::Builder::new(&values, width, height)?
            .generate_progressive(&args.progressive_vertices, args.boundary_error)?;
        for point in &mut mesh.vertices {
            point[0] *= args.spacing;
            point[1] *= args.spacing;
            ensure!(
                point.iter().all(|v| v.is_finite()),
                "出力座標が有限値の範囲を超えます"
            );
        }
        let mut writer = BufWriter::new(File::create(&args.output)?);
        serde_json::to_writer(&mut writer, &mesh)?;
        writer.flush()?;
        println!(
            "{}",
            serde_json::json!({"vertices":mesh.vertices.len(),"steps":mesh.steps.len(),"seconds":start.elapsed().as_secs_f64()})
        );
        return Ok(());
    }
    ensure!(
        args.seed_points.is_none() || (!args.seamless && args.progressive_vertices.is_empty()),
        "山頂の固定は通常の独立TINで指定してください"
    );
    let builder = adaptive_tin::Builder::new(&values, width, height)?;
    let builder = if let Some(path) = &args.seed_points {
        let seeds: Vec<[usize; 2]> = serde_json::from_reader(BufReader::new(File::open(path)?))?;
        builder.with_seed_points(&seeds, args.max_vertices)?
    } else {
        builder
    };
    ensure!(
        !args.lock_seed_boundary
            || (args.seed_points.is_some()
                && !args.seamless
                && args.progressive_vertices.is_empty()
                && args.neighbor_ratio.is_none()
                && args.mesh_sizes.is_none()),
        "共有辺の固定は通常生成の固定シードと併用してください"
    );
    let builder = if args.lock_seed_boundary {
        builder.with_locked_boundary()?
    } else {
        builder
    };
    let builder = if let Some(path) = &args.mesh_sizes {
        ensure!(
            args.mesh_size_scale.is_finite() && args.mesh_size_scale > 0.,
            "メッシュ寸法の倍率は正の有限値が必要です"
        );
        let (w, h, sizes) = read_png(path, Encoding::Gray, args.mesh_size_scale, 0.)?;
        ensure!(
            w == width && h == height,
            "メッシュ寸法PNGの大きさが一致しません"
        );
        builder.with_mesh_sizes(&sizes)?
    } else {
        builder
    };
    let mut mesh = if let Some(path) = &args.error_limits {
        ensure!(
            args.error_limit_scale.is_finite() && args.error_limit_scale > 0.,
            "許容誤差の倍率は正の有限値が必要です"
        );
        let (w, h, limits) = read_png(path, Encoding::Gray, args.error_limit_scale, 0.)?;
        ensure!(
            w == width && h == height,
            "許容誤差PNGの大きさが標高PNGと一致しません"
        );
        let builder = builder.with_error_limits(&limits)?;
        if let Some(ratio) = args.neighbor_ratio {
            builder.generate_neighbor_balanced(args.max_vertices, 1., ratio)?
        } else {
            builder.generate_with_minimum(args.max_vertices, 1., args.min_vertices)?
        }
    } else if args.seamless {
        builder.generate_seamless_with_minimum(
            args.max_vertices,
            args.max_error,
            args.min_vertices,
        )?
    } else {
        builder.generate_with_minimum(args.max_vertices, args.max_error, args.min_vertices)?
    };
    for p in &mut mesh.vertices {
        p[0] *= args.spacing;
        p[1] *= args.spacing;
        ensure!(
            p.iter().all(|v| v.is_finite()),
            "出力座標が有限値の範囲を超えます"
        );
    }
    let report = serde_json::json!({"width":width,"height":height,"vertices":mesh.vertices.len(),"triangles":mesh.triangles.len(),"maxError":mesh.max_error,"maxErrorRatio":mesh.max_error_ratio,"maxMeshSizeRatio":mesh.max_mesh_size_ratio,"balance":mesh.balance,"toleranceReached":mesh.tolerance_reached,"seconds":start.elapsed().as_secs_f64(),"triangleEvaluations":mesh.triangle_evaluations,"sampleEvaluations":mesh.sample_evaluations});
    let mut writer = BufWriter::new(File::create(&args.output)?);
    serde_json::to_writer(&mut writer, &mesh)?;
    writer.flush()?;
    println!("{}", report);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SERIAL: AtomicUsize = AtomicUsize::new(0);

    fn png_fixture(color: png::ColorType, depth: png::BitDepth, data: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "yamamuki-adaptive-tin-{}-{}.png",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let mut encoder = png::Encoder::new(File::create(&path).unwrap(), 2, 2);
        encoder.set_color(color);
        encoder.set_depth(depth);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(data)
            .unwrap();
        path
    }

    #[test]
    fn png_gray8_and_gray16_preserve_numeric_values() {
        for (depth, data, expected) in [
            (
                png::BitDepth::Eight,
                vec![0, 1, 127, 255],
                vec![-100., -98., 154., 410.],
            ),
            (
                png::BitDepth::Sixteen,
                vec![0, 0, 0, 1, 1, 0, 255, 255],
                vec![-100., -98., 412., 130970.],
            ),
        ] {
            let path = png_fixture(png::ColorType::Grayscale, depth, &data);
            let (w, h, values) = read_png(&path, Encoding::Gray, 2., -100.).unwrap();
            std::fs::remove_file(path).unwrap();
            assert_eq!((w, h), (2, 2));
            assert_eq!(values, expected);
        }
    }

    #[test]
    fn png_gsi_signed_height_and_missing_sample() {
        let path = png_fixture(
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[0, 0, 100, 255, 255, 156, 0, 0, 0, 128, 0, 0],
        );
        let (w, h, values) = read_png(&path, Encoding::Gsi, 1., 0.).unwrap();
        assert_eq!(&values[..3], &[1., -1., 0.]);
        assert!(values[3].is_nan());
        assert!(adaptive_tin::Builder::new(&values, w, h).is_err());
        assert!(read_png(&path, Encoding::Gray, 1., 0.).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
