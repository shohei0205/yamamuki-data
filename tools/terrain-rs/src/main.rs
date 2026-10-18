mod coast;
mod codec;
mod dem;
mod geos;
mod mesh;
mod national;
mod qem;
mod stitch;
use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, time::Instant};
#[derive(Parser)]
#[command(about = "地形の読み書きと誤差制限付きポリゴン削減")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// 指定した最高詳細の処理区画を、全国生成と同じ方法で検証する。
    Cluster {
        #[arg(long, default_value_t = 14, value_parser=clap::value_parser!(u32).range(4..=14))]
        level: u32,
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        reference: PathBuf,
        #[arg(long)]
        library: PathBuf,
        #[arg(long)]
        coasts: PathBuf,
        #[arg(long)]
        x: u32,
        #[arg(long)]
        y: u32,
        #[arg(long)]
        output: PathBuf,
    },
    /// 全国の最高詳細と全 LOD を生成する。保存済み区画から再開できる。
    Generate {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        library: PathBuf,
        #[arg(long)]
        coast: PathBuf,
        #[arg(long, default_value_t = 4)]
        workers: usize,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// 全レベルの標高誤差と共有辺を検証する。
    Verify {
        #[arg(long)]
        input: PathBuf,
    },
    /// 海岸線を簡略化して、島と穴の数を検証する。
    Coast {
        #[arg(long)]
        library: PathBuf,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 250.)]
        tolerance: f64,
    },
    /// 最高詳細の標高から不規則な三角形を生成する。
    Source {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        x: u32,
        #[arg(long)]
        y: u32,
        #[arg(long)]
        output: PathBuf,
    },
    /// SQLite 内の一つの地形を検証し、必要なら削減する。
    Tile {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        level: u32,
        #[arg(long)]
        x: u32,
        #[arg(long)]
        y: u32,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 25.)]
        error: f64,
        #[arg(long, default_value_t = 0.5)]
        ratio: f64,
        #[arg(long, default_value_t = 1.)]
        meters: f64,
    },
    /// 圧縮標高タイルを検証する。
    Dem {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        level: u32,
        #[arg(long)]
        x: u32,
        #[arg(long)]
        y: u32,
    },
}
fn tile(input: &std::path::Path, z: u32, x: u32, y: u32) -> Result<Vec<u8>> {
    let db =
        rusqlite::Connection::open_with_flags(input, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    Ok(db.query_row(
        "SELECT data FROM tiles WHERE z=?1 AND x=?2 AND y=?3",
        (z, x, y),
        |r| r.get(0),
    )?)
}
fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Cluster {
            level,
            source,
            reference,
            library,
            coasts,
            x,
            y,
            output,
        } => {
            let started = Instant::now();
            let r = national::convert(&source, &reference, &library, &coasts, level, x, y)?;
            if let Some(body) = r.body {
                std::fs::write(output, &body)?;
            }
            println!(
                "{}",
                serde_json::json!({"x":x,"y":y,"vertices":r.vertices,"triangles":r.triangles,"sampledErrorMeters":r.height,"seconds":started.elapsed().as_secs_f64()})
            );
        }
        Command::Generate {
            source,
            output,
            library,
            coast,
            workers,
            limit,
        } => {
            ensure!(workers > 0, "並列数は 1 以上にしてください");
            if let Err(error) =
                national::generate(&source, &output, &library, &coast, workers, limit)
            {
                let status = serde_json::json!({"state":"failed","generator":"rust","error":format!("{error:#}")});
                let _ = std::fs::write(
                    output.with_extension("progress.json"),
                    serde_json::to_vec_pretty(&status)?,
                );
                return Err(error);
            }
        }
        Command::Verify { input } => println!("{}", national::verify(&input)?),
        Command::Coast {
            library,
            input,
            output,
            tolerance,
        } => {
            let started = Instant::now();
            let api = geos::Geos::open(&library)?;
            let land = api.read(&std::fs::read(input)?)?;
            let simple = land.simplify(tolerance)?;
            let bytes = simple.write()?;
            std::fs::write(output, &bytes)?;
            println!(
                "{}",
                serde_json::json!({"polygonsAndHoles":simple.counts()?,"bytes":bytes.len(),"elapsedSeconds":started.elapsed().as_secs_f64()})
            );
        }
        Command::Source {
            input,
            x,
            y,
            output,
        } => {
            let started = Instant::now();
            let db = rusqlite::Connection::open_with_flags(
                input,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let grid = dem::read_grid(&db, 14, x, y)?;
            let mesh = dem::simplify(&grid, 5.)?;
            let blob = codec::encode_mesh(&mesh)?;
            std::fs::write(output, &blob)?;
            println!(
                "{}",
                serde_json::json!({"vertices":mesh.vertices.len(),"triangles":mesh.faces.len(),"bytes":blob.len(),"elapsedSeconds":started.elapsed().as_secs_f64()})
            );
        }
        Command::Dem { input, level, x, y } => {
            let grid = codec::decode_grid(&tile(&input, level, x, y)?)?;
            println!(
                "{}",
                serde_json::json!({"rows":grid.rows,"columns":grid.columns,"validSamples":grid.values.iter().filter(|v|v.is_finite()).count()})
            );
        }
        Command::Tile {
            input,
            level,
            x,
            y,
            output,
            error,
            ratio,
            meters,
        } => {
            ensure!(
                ratio > 0. && ratio <= 1. && meters.is_finite() && meters > 0.,
                "削減率または座標の縮尺が不正です"
            );
            let started = Instant::now();
            let body = tile(&input, level, x, y)?;
            let original = codec::decode_mesh(&body)?;
            let nv = original.vertices.len();
            let nf = original.faces.len();
            let mut physical = original.clone();
            for p in &mut physical.vertices {
                p[0] *= meters;
                p[1] *= meters;
            }
            let probes = physical.probes();
            let mut reducer = qem::Qem::new(physical, &[], Some(probes.clone()))?;
            let mut result = reducer.reduce(((nf as f64 * ratio) as usize).max(1), error)?;
            let surface = mesh::Surface::new(&result);
            let mut max_error = 0f64;
            for p in probes {
                let height = surface
                    .height([p[0], p[1]])
                    .ok_or_else(|| anyhow::anyhow!("削減後の面に標本が含まれていません"))?;
                max_error = max_error.max((height - p[2]).abs());
            }
            ensure!(
                max_error <= error + 1e-5,
                "削減後の誤差が上限を超えています"
            );
            for p in &mut result.vertices {
                p[0] /= meters;
                p[1] /= meters;
            }
            let encoded = codec::encode_mesh(&result)?;
            codec::decode_mesh(&encoded)?;
            if let Some(path) = output {
                std::fs::write(path, &encoded)?;
            }
            println!(
                "{}",
                serde_json::json!({"inputVertices":nv,"inputTriangles":nf,"vertices":result.vertices.len(),"triangles":result.faces.len(),"sampledMaxErrorMeters":max_error,"inputBytes":body.len(),"bytes":encoded.len(),"elapsedSeconds":started.elapsed().as_secs_f64()})
            );
        }
    }
    Ok(())
}
