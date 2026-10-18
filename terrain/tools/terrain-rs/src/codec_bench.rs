//! 既存の地形を変更せず、圧縮・誤差・復元時間を計測する比較用ツール。
mod baked_normals;
mod bench_height;
mod bench_jxl;
mod bench_mesh;
mod bench_normal;
mod bench_png;
#[allow(dead_code)]
mod codec;
mod height_astc;
#[allow(dead_code)]
mod mesh;
mod mesh_normals;
mod national_height_jxl;
mod national_jxl;
mod national_shading;
use anyhow::{Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "build/terrain-heightmap-yari")]
    root: PathBuf,
    #[arg(long, default_value = "build/terrain-codec-comparison")]
    output: PathBuf,
    #[arg(long, default_value = "build/terrain-yari-local.sqlite3")]
    polygon: PathBuf,
    #[arg(
        long,
        default_value = "build/terrain-codec-tools/astc/bin/astcenc-avx2.exe"
    )]
    astc: PathBuf,
    #[arg(
        long,
        default_value = "build/terrain-codec-tools/lerc/pylerc-4.2.0.data/purelib/lerc/Lerc.dll"
    )]
    lerc: PathBuf,
    #[arg(
        long,
        default_value = "build/terrain-codec-tools/jxl-0.12.0/x64-windows-static/bin"
    )]
    jxl: PathBuf,
    #[arg(long, default_value = "all")]
    part: String,
    /// 全国ハイトマップの生成対象の最低レベル。
    #[arg(long, default_value_t = 7, value_parser=clap::value_parser!(u32).range(7..=14))]
    min_level: u32,
    /// 全国ハイトマップの生成対象の最高レベル。
    #[arg(long, default_value_t = 12, value_parser=clap::value_parser!(u32).range(7..=14))]
    max_level: u32,
    #[arg(long, default_value_t = 315.)]
    sun_azimuth: f64,
    #[arg(long, default_value_t = 45.)]
    sun_altitude: f64,
    #[arg(long, allow_hyphen_values = true)]
    solar_declination: Option<f64>,
    #[arg(long, allow_hyphen_values = true)]
    solar_hour_angle: Option<f64>,
    #[arg(long)]
    solar_datetime: Option<String>,
}
fn main() -> Result<()> {
    let args = Args::parse();
    fs::create_dir_all(&args.output)?;
    ensure!(
        [
            "all",
            "normals",
            "mesh",
            "heights",
            "mesh-normals",
            "baked-normals",
            "height-astc",
            "jxl",
            "national-jxl",
            "national-height-jxl",
            "national-shading",
            "national-geometry"
        ]
        .contains(&args.part.as_str()),
        "比較の種類が不正です"
    );
    let started = Instant::now();
    if args.part == "national-geometry" {
        return national_shading::geometry(&args.root, &args.output);
    }
    if args.part == "national-shading" {
        if args.solar_declination.is_some() || args.solar_hour_angle.is_some() {
            let a = args
                .solar_declination
                .ok_or_else(|| anyhow::anyhow!("赤緯が必要です"))?;
            let h = args
                .solar_hour_angle
                .ok_or_else(|| anyhow::anyhow!("時角が必要です"))?;
            return national_shading::run_solar(
                &args.root,
                &args.output,
                args.sun_azimuth,
                args.sun_altitude,
                Some([a, h]),
                args.solar_datetime.as_deref(),
            );
        }
        return national_shading::run(
            &args.root,
            &args.output,
            args.sun_azimuth,
            args.sun_altitude,
        );
    }
    if args.part == "national-height-jxl" {
        return national_height_jxl::run(
            &args.root, &args.output, &args.jxl, args.min_level, args.max_level,
        );
    }
    if args.part == "national-jxl" {
        return national_jxl::run(&args.root, &args.output, &args.jxl);
    }
    write_json(
        &args.output.join("status.json"),
        &json!({"state":"running","part":args.part,"tool":"terrain-codec-bench Rust"}),
    )?;
    if args.part == "jxl" {
        bench_jxl::run(&args.root, &args.output, &args.jxl)?;
    }
    if args.part == "height-astc" {
        height_astc::run(&args.root, &args.output, &args.astc)?;
    }
    if args.part == "baked-normals" {
        baked_normals::run(&args.root, &args.output, &args.polygon, &args.astc)?;
    }
    if args.part == "mesh-normals" {
        mesh_normals::run(&args.root, &args.output, &args.polygon, &args.astc)?;
    }
    if args.part == "all" || args.part == "normals" {
        write_json(
            &args.output.join("normals.json"),
            &bench_normal::run(&args.root, &args.output, &args.astc)?,
        )?;
    }
    if args.part == "all" || args.part == "mesh" {
        write_json(
            &args.output.join("meshes.json"),
            &bench_mesh::run(&args.root, &args.output, &args.polygon)?,
        )?;
    }
    if args.part == "all" || args.part == "heights" {
        write_json(
            &args.output.join("heights.json"),
            &bench_height::run(&args.root, &args.output, &args.lerc)?,
        )?;
    }
    write_json(
        &args.output.join("status.json"),
        &json!({"state":"complete","part":args.part,"seconds":started.elapsed().as_secs_f64(),"tool":"terrain-codec-bench Rust","originalFilesPreserved":true}),
    )?;
    println!("{}", fs::read_to_string(args.output.join("status.json"))?);
    Ok(())
}
pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
pub fn packet(magic: &[u8; 4], header: &Value, chunks: &[&[u8]]) -> Result<Vec<u8>> {
    let h = serde_json::to_vec(header)?;
    let mut out = Vec::new();
    out.extend(magic);
    out.extend((h.len() as u32).to_le_bytes());
    out.extend(h);
    for chunk in chunks {
        out.extend(*chunk);
    }
    Ok(out)
}
pub fn unpacket<'a>(data: &'a [u8], magic: &[u8; 4]) -> Result<(Value, &'a [u8])> {
    ensure!(
        data.len() >= 8 && &data[..4] == magic,
        "比較データの識別子が不正です"
    );
    let n = u32::from_le_bytes(data[4..8].try_into()?) as usize;
    ensure!(n <= data.len() - 8, "比較データの見出しが不足しています");
    Ok((serde_json::from_slice(&data[8..8 + n])?, &data[8 + n..]))
}
pub fn error(a: &[f32], b: &[f32]) -> Result<(f64, f64)> {
    ensure!(a.len() == b.len(), "復元した標高の数が違います");
    let mut max = 0f64;
    let mut sum = 0.;
    let mut count = 0;
    for (&x, &y) in a.iter().zip(b) {
        ensure!(x.is_finite() == y.is_finite(), "欠損の位置が変わりました");
        if x.is_finite() {
            let e = (x as f64 - y as f64).abs();
            max = max.max(e);
            sum += e * e;
            count += 1;
        }
    }
    Ok((
        max,
        if count > 0 {
            (sum / count as f64).sqrt()
        } else {
            0.
        },
    ))
}
pub fn median_ms<F, T>(mut decode: F) -> Result<(T, f64)>
where
    F: FnMut() -> Result<T>,
{
    let mut times = Vec::new();
    let mut last = None;
    for _ in 0..3 {
        let t = Instant::now();
        last = Some(decode()?);
        times.push(t.elapsed().as_secs_f64() * 1000.);
    }
    times.sort_by(f64::total_cmp);
    Ok((last.unwrap(), times[1]))
}
