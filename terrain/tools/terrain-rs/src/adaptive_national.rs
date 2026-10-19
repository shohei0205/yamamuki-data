//! 保存済みL14標高から、境界をそろえたAdaptive TINの全国メッシュを作る。
mod adaptive_tin;
#[allow(dead_code)]
mod codec;
#[allow(dead_code)]
mod mesh;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use flate2::{Compression, write::ZlibEncoder};
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::json;
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
const N: usize = 1025;
#[cfg(test)]
const ERRORS: [u32; 6] = [20, 50, 100, 250, 500, 1000];
#[derive(Parser)]
#[command(about = "全国L14標高からAdaptive TINを誤差段階別に生成・再開")]
struct Args {
    #[arg(long)]
    source: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, value_delimiter = ',', default_value = "20,50,100,250,500,1000")]
    tolerances: Vec<u32>,
    #[arg(long, default_value_t = 4)]
    workers: usize,
    #[arg(long, default_value_t = 4)]
    min_vertices: usize,
    #[arg(long, conflicts_with = "min_vertices")]
    staged_min_vertices: bool,
    #[arg(long)]
    reuse: Option<PathBuf>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long)]
    verify: bool,
}
fn read(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}
fn zip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::new(9));
    z.write_all(bytes)?;
    Ok(z.finish()?)
}
fn input_grid(db: &Connection, x: u32, y: u32) -> Result<(Vec<f64>, Vec<u8>, usize)> {
    let mut values = vec![0.; N * N];
    let mut mask = vec![0u8; (N * N).div_ceil(8)];
    let mut valid = 0;
    let mut query = db.prepare_cached("SELECT data FROM tiles WHERE z=14 AND x=?1 AND y=?2")?;
    for dy in 0..5 {
        for dx in 0..5 {
            let body: Option<Vec<u8>> = query
                .query_row((x * 4 + dx, y * 4 + dy), |r| r.get(0))
                .optional()?;
            let Some(body) = body else {
                continue;
            };
            let grid = codec::decode_grid(&body)?;
            ensure!(
                grid.rows == 256 && grid.columns == 256,
                "元標高は256四方が必要です"
            );
            for row in 0..if dy == 4 { 1 } else { 256 } {
                for col in 0..if dx == 4 { 1 } else { 256 } {
                    let index = (dy as usize * 256 + row) * N + dx as usize * 256 + col;
                    let value = grid.values[row * 256 + col];
                    if value.is_finite() {
                        values[index] = value as f64;
                        mask[index / 8] |= 1 << (index % 8);
                        valid += 1;
                    }
                }
            }
        }
    }
    Ok((values, mask, valid))
}
// 同じ境界標高列を使うため、欠損は作業用に0で埋め、元の有効域を別保存する。
fn average(input: &[f64], radius: usize) -> Vec<f64> {
    if radius == 0 {
        return input.to_vec();
    }
    let mut a = input.to_vec();
    let mut b = vec![0.; a.len()];
    for _ in 0..3 {
        for row in 0..N {
            let mut prefix = vec![0.; N + 1];
            for x in 0..N {
                prefix[x + 1] = prefix[x] + a[row * N + x];
            }
            for x in 0..N {
                let lo = x.saturating_sub(radius);
                let hi = (x + radius + 1).min(N);
                b[row * N + x] = (prefix[hi] - prefix[lo]) / (hi - lo) as f64;
            }
        }
        for x in 0..N {
            let mut prefix = vec![0.; N + 1];
            for y in 0..N {
                prefix[y + 1] = prefix[y] + b[y * N + x];
            }
            for y in 0..N {
                let lo = y.saturating_sub(radius);
                let hi = (y + radius + 1).min(N);
                a[y * N + x] = (prefix[hi] - prefix[lo]) / (hi - lo) as f64;
            }
        }
    }
    a
}
fn normals(grid: &[f64], vertices: &[[f64; 3]], meters: f64) -> Result<Vec<u8>> {
    let radius = ((N - 1) as f64 / (4. * (vertices.len() as f64).sqrt())).floor() as usize;
    let smooth = average(grid, radius);
    let mut bytes = Vec::with_capacity(vertices.len() * 2);
    for p in vertices {
        let x = p[0] as usize;
        let y = p[1] as usize;
        let l = x.saturating_sub(1);
        let r = (x + 1).min(N - 1);
        let t = y.saturating_sub(1);
        let b = (y + 1).min(N - 1);
        let nx = -(smooth[y * N + r] - smooth[y * N + l]) / ((r - l) as f64 * meters);
        let ny = (smooth[b * N + x] - smooth[t * N + x]) / ((b - t) as f64 * meters);
        let sum = nx.abs() + ny.abs() + 1.;
        bytes.push(((nx / sum * 0.5 + 0.5) * 255.).round().clamp(0., 255.) as u8);
        bytes.push(((ny / sum * 0.5 + 0.5) * 255.).round().clamp(0., 255.) as u8);
    }
    zip(&bytes)
}
fn boundary_bytes(vertices: &[[f64; 3]], side: usize) -> Vec<u8> {
    let mut values: Vec<_> = vertices
        .iter()
        .filter(|p| match side {
            0 => p[0] == 0.,
            1 => p[0] == (N - 1) as f64,
            2 => p[1] == 0.,
            _ => p[1] == (N - 1) as f64,
        })
        .map(|p| ((if side < 2 { p[1] } else { p[0] }) as u16, p[2] as f32))
        .collect();
    values.sort_unstable_by_key(|v| v.0);
    let mut bytes = Vec::new();
    for (at, z) in values {
        bytes.extend(at.to_le_bytes());
        bytes.extend(z.to_le_bytes());
    }
    bytes
}
struct Packed {
    tolerance: u32,
    boundaries: [Vec<u8>; 4],
    data: Vec<u8>,
    normal: Vec<u8>,
    vertices: usize,
    triangles: usize,
    error: f64,
}
struct Block {
    x: u32,
    y: u32,
    mask: Vec<u8>,
    valid: usize,
    levels: Vec<Packed>,
}
fn minimum_for(tolerance: u32, minimum: usize, staged: bool) -> usize {
    if staged {
        match tolerance {
            10 | 20 | 50 => 4,
            100 => 512,
            250 => 128,
            500 => 64,
            _ => 32,
        }
    } else {
        minimum
    }
}
fn effective_minimum(tolerance: u32, minimum: usize, staged: bool, finer: Option<usize>) -> usize {
    let requested = minimum_for(tolerance, minimum, staged);
    if staged {
        requested.min(finer.unwrap_or(usize::MAX)).max(4)
    } else {
        requested
    }
}
fn reuse_level(
    db: &Connection,
    x: u32,
    y: u32,
    tolerance: u32,
    minimum: usize,
) -> Result<Option<Packed>> {
    let record=db.query_row("SELECT data,normal_data,vertices,triangles,max_error FROM meshes WHERE x=?1 AND y=?2 AND tolerance=?3 AND vertices>=?4 AND max_error<=tolerance",params![x,y,tolerance,minimum],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,usize>(2)?,r.get::<_,usize>(3)?,r.get::<_,f64>(4)?))).optional()?;
    let Some((data, normal, vertices, triangles, error)) = record else {
        return Ok(None);
    };
    let mut boundaries: [Vec<u8>; 4] = std::array::from_fn(|_| Vec::new());
    for (side, edge) in boundaries.iter_mut().enumerate() {
        *edge = db.query_row(
            "SELECT data FROM boundaries WHERE x=?1 AND y=?2 AND tolerance=?3 AND side=?4",
            params![x, y, tolerance, side],
            |r| r.get(0),
        )?;
    }
    Ok(Some(Packed {
        tolerance,
        boundaries,
        data,
        normal,
        vertices,
        triangles,
        error,
    }))
}
fn block(
    source: &Path,
    x: u32,
    y: u32,
    min_vertices: usize,
    staged: bool,
    reuse: Option<&Path>,
    tolerances: &[u32],
) -> Result<Block> {
    let (grid, mask, valid) = input_grid(&read(source)?, x, y)?;
    let latitude = (std::f64::consts::PI * (1. - 2. * (y as f64 + 0.5) / 4096.))
        .sinh()
        .atan();
    let meters = 40075016.68557849 / ((1u64 << 14) as f64 * 256.) * latitude.cos();
    let mut levels: Vec<Packed> = Vec::new();
    let reused = reuse.map(read).transpose()?;
    for &tolerance in tolerances {
        let floor = effective_minimum(
            tolerance,
            min_vertices,
            staged,
            levels.last().map(|l| l.vertices),
        );
        if let Some(db) = &reused {
            if let Some(record) = reuse_level(db, x, y, tolerance, floor)? {
                levels.push(record);
                continue;
            }
        }
        let mesh = adaptive_tin::Builder::new(&grid, N, N)?.generate_seamless_with_minimum(
            N * N,
            tolerance as f64,
            floor,
        )?;
        ensure!(mesh.tolerance_reached, "許容誤差に到達しません");
        let mut raw = Vec::new();
        raw.extend((mesh.vertices.len() as u32).to_le_bytes());
        raw.extend((mesh.triangles.len() as u32 * 3).to_le_bytes());
        for p in &mesh.vertices {
            for v in p {
                raw.extend((*v as f32).to_le_bytes());
            }
        }
        for f in &mesh.triangles {
            for i in f {
                raw.extend((*i as u32).to_le_bytes());
            }
        }
        let normal = normals(&grid, &mesh.vertices, meters)?;
        levels.push(Packed {
            boundaries: std::array::from_fn(|side| boundary_bytes(&mesh.vertices, side)),
            tolerance,
            data: zip(&raw)?,
            normal,
            vertices: mesh.vertices.len(),
            triangles: mesh.triangles.len(),
            error: mesh.max_error,
        });
    }
    Ok(Block {
        x,
        y,
        mask: zip(&mask)?,
        valid,
        levels,
    })
}
fn statistics(db: &Connection) -> Result<serde_json::Value> {
    let mut q=db.prepare("SELECT tolerance,count(*),sum(length(data)),sum(length(normal_data)),sum(vertices),sum(triangles),max(max_error) FROM meshes GROUP BY tolerance ORDER BY tolerance")?;
    let levels=q.query_map([],|r|Ok(json!({"tolerance":r.get::<_,u32>(0)?,"tiles":r.get::<_,u64>(1)?,"meshBytes":r.get::<_,u64>(2)?,"normalBytes":r.get::<_,u64>(3)?,"vertices":r.get::<_,u64>(4)?,"triangles":r.get::<_,u64>(5)?,"maxError":r.get::<_,f64>(6)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
    Ok(json!(levels))
}
fn progress(
    path: &Path,
    db: &Connection,
    total: usize,
    start: &Instant,
    status: &str,
) -> Result<()> {
    let done: usize = db.query_row("SELECT count(*) FROM blocks", [], |r| r.get(0))?;
    let value = json!({"status":status,"doneBlocks":done,"totalBlocks":total,"secondsThisRun":start.elapsed().as_secs_f64(),"levels":statistics(db)?});
    let target = path.with_extension("progress.json");
    let temp = path.with_extension("progress.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(&value)?)?;
    std::fs::rename(temp, target)?;
    println!("{}", value);
    Ok(())
}
fn verify(db: &Connection) -> Result<()> {
    let config: String =
        db.query_row("SELECT value FROM metadata WHERE key='config'", [], |r| {
            r.get(0)
        })?;
    let config: serde_json::Value = serde_json::from_str(&config)?;
    let expected: Vec<u32> = serde_json::from_value(config["tolerances"].clone())?;
    let actual = db
        .prepare("SELECT DISTINCT tolerance FROM meshes ORDER BY tolerance")?
        .query_map([], |r| r.get::<_, u32>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(
        actual == expected,
        "指定した誤差段階と保存データが一致しません"
    );
    let bad: u64 = db.query_row(
        "SELECT count(*) FROM meshes WHERE max_error>tolerance OR vertices<4",
        [],
        |r| r.get(0),
    )?;
    let incomplete:u64=db.query_row("SELECT count(*) FROM blocks b WHERE (SELECT count(*) FROM meshes m WHERE m.x=b.x AND m.y=b.y)!=?1",[expected.len()],|r|r.get(0))?;
    ensure!(
        bad == 0 && incomplete == 0,
        "誤差または誤差段階の保存が不正です"
    );
    let horizontal:u64=db.query_row("SELECT count(*) FROM boundaries a JOIN boundaries b ON b.x=a.x+1 AND b.y=a.y AND b.tolerance=a.tolerance AND b.side=0 WHERE a.side=1 AND a.data!=b.data",[],|r|r.get(0))?;
    let vertical:u64=db.query_row("SELECT count(*) FROM boundaries a JOIN boundaries b ON b.x=a.x AND b.y=a.y+1 AND b.tolerance=a.tolerance AND b.side=2 WHERE a.side=3 AND a.data!=b.data",[],|r|r.get(0))?;
    let seams = horizontal + vertical;
    ensure!(seams == 0, "隣接区画の境界が一致しません: {seams}");
    let mut query =
        db.prepare("SELECT x,y,tolerance,data,normal_data,vertices,triangles FROM meshes")?;
    for row in query.query_map([], |r| {
        Ok((
            r.get::<_, u32>(0)?,
            r.get::<_, u32>(1)?,
            r.get::<_, u32>(2)?,
            r.get::<_, Vec<u8>>(3)?,
            r.get::<_, Vec<u8>>(4)?,
            r.get::<_, usize>(5)?,
            r.get::<_, usize>(6)?,
        ))
    })? {
        let (x, y, tolerance, body, normal, nv, nt) = row?;
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(&body[..])
            .take((8 + nv * 12 + nt * 12 + 1) as u64)
            .read_to_end(&mut raw)?;
        ensure!(
            raw.len() == 8 + nv * 12 + nt * 12,
            "区画 {x}/{y}/{tolerance}: メッシュの長さが不正です"
        );
        let u32_at = |i| u32::from_le_bytes(raw[i..i + 4].try_into().unwrap()) as usize;
        ensure!(
            u32_at(0) == nv && u32_at(4) == nt * 3,
            "頂点数と面数が一致しません"
        );
        for i in 0..nv {
            let p: [f32; 3] = std::array::from_fn(|k| {
                f32::from_le_bytes(
                    raw[8 + i * 12 + k * 4..12 + i * 12 + k * 4]
                        .try_into()
                        .unwrap(),
                )
            });
            ensure!(
                p.iter().all(|v| v.is_finite())
                    && p[0] >= 0.
                    && p[0] < N as f32
                    && p[1] >= 0.
                    && p[1] < N as f32,
                "頂点座標が不正です"
            );
        }
        for i in 0..nt * 3 {
            ensure!(u32_at(8 + nv * 12 + i * 4) < nv, "頂点索引が範囲外です");
        }
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(&normal[..])
            .take((nv * 2 + 1) as u64)
            .read_to_end(&mut decoded)?;
        ensure!(decoded.len() == nv * 2, "法線の長さが不正です");
    }
    let check: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    ensure!(check == "ok", "SQLite検証: {check}");
    println!("{}", json!({"levels":statistics(db)?,"sqliteCheck":check}));
    Ok(())
}
fn main() -> Result<()> {
    let mut args = Args::parse();
    args.tolerances.sort_unstable();
    args.tolerances.dedup();
    ensure!(
        !args.tolerances.is_empty()
            && args
                .tolerances
                .iter()
                .all(|t| [10, 20, 50, 100, 250, 500, 1000].contains(t)),
        "誤差段階は10・20・50・100・250・500・1000から指定してください"
    );
    ensure!((1..=16).contains(&args.workers), "並列数は1〜16が必要です");
    if args.verify {
        return verify(&read(&args.output)?);
    }
    ensure!(
        args.source.canonicalize()?
            != args
                .output
                .canonicalize()
                .unwrap_or_else(|_| args.output.clone()),
        "元データへの上書きはできません"
    );
    let start = Instant::now();
    let src = read(&args.source)?;
    let mut q = src.prepare("SELECT DISTINCT x/4,y/4 FROM tiles WHERE z=14 ORDER BY x/4,y/4")?;
    let all = q
        .query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut db = Connection::open(&args.output)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS boundaries(x INTEGER,y INTEGER,tolerance INTEGER,side INTEGER,data BLOB,PRIMARY KEY(x,y,tolerance,side)); CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT); CREATE TABLE IF NOT EXISTS blocks(x INTEGER,y INTEGER,valid_samples INTEGER,valid_mask BLOB,PRIMARY KEY(x,y)); CREATE TABLE IF NOT EXISTS meshes(x INTEGER,y INTEGER,tolerance INTEGER,data BLOB,normal_data BLOB,vertices INTEGER,triangles INTEGER,max_error REAL,PRIMARY KEY(x,y,tolerance));")?;
    let source = args.source.canonicalize()?.to_string_lossy().to_string();
    let config=json!({"version":1,"source":source,"sourceModified":std::fs::metadata(&args.source)?.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_nanos().to_string(),"sourceBytes":std::fs::metadata(&args.source)?.len(),"zoom":12,"gridSize":N,"tolerances":args.tolerances,"algorithm":"greedy-insertion-delaunay-local-boundary-profile","heightSourceZoom":14,"minVertices":args.min_vertices,"missing":"valid_mask; temporary zero fill; renderer must honor mask","meshEncoding":"zlib(counts u32 LE, pixel XYZ f32 LE, indices u32 LE)","normalEncoding":"zlib octahedral XY uint8, 2 bytes per vertex; Y axis upward","normalFilter":"three separable box passes, density-dependent radius","bounds":"global WebMercator pixel origin=(x*1024,y*1024) at zoom14"}).to_string();
    let mut config_value: serde_json::Value = serde_json::from_str(&config)?;
    if args.min_vertices == 4 {
        config_value.as_object_mut().unwrap().remove("minVertices");
    }
    if args.staged_min_vertices {
        config_value["stagedMinimumCap"] = json!("previousFinerVertexCount");
        config_value["stagedMinVertices"] =
            json!({"20":4,"50":4,"100":512,"250":128,"500":64,"1000":32});
        if args.tolerances.contains(&10) {
            config_value["stagedMinVertices"]["10"] = json!(4);
        }
    }
    if let Some(reuse) = &args.reuse {
        ensure!(
            reuse.canonicalize()?
                != args
                    .output
                    .canonicalize()
                    .unwrap_or_else(|_| args.output.clone()),
            "再利用元への上書きはできません"
        );
        let base = read(reuse)?;
        let text: String =
            base.query_row("SELECT value FROM metadata WHERE key='config'", [], |r| {
                r.get(0)
            })?;
        let previous: serde_json::Value = serde_json::from_str(&text)?;
        for key in [
            "source",
            "sourceModified",
            "sourceBytes",
            "gridSize",
            "zoom",
            "heightSourceZoom",
            "algorithm",
        ] {
            ensure!(
                previous[key] == config_value[key],
                "再利用元の生成条件が異なります: {key}"
            );
        }
        config_value["reuseBase"] = json!(reuse.canonicalize()?.to_string_lossy());
    }
    let config = config_value.to_string();
    let previous: Option<String> = db
        .query_row("SELECT value FROM metadata WHERE key='config'", [], |r| {
            r.get(0)
        })
        .optional()?;
    ensure!(
        previous.as_ref().is_none_or(|p| p == &config),
        "生成条件が異なります。別の出力先を指定してください"
    );
    db.execute(
        "INSERT OR IGNORE INTO metadata VALUES('config',?1)",
        [&config],
    )?;
    let mut q = db.prepare("SELECT x,y FROM blocks")?;
    let done = q
        .query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    drop(q);
    let pending: Vec<_> = all
        .iter()
        .copied()
        .filter(|p| !done.contains(p))
        .take(args.limit.unwrap_or(usize::MAX))
        .collect();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(args.workers)
        .build()?;
    progress(&args.output, &db, all.len(), &start, "running")?;
    for batch in pending.chunks(args.workers) {
        let results: Vec<Result<Block>> = pool.install(|| {
            batch
                .par_iter()
                .map(|&(x, y)| {
                    block(
                        &args.source,
                        x,
                        y,
                        args.min_vertices,
                        args.staged_min_vertices,
                        args.reuse.as_deref(),
                        &args.tolerances,
                    )
                    .with_context(|| format!("区画 {x}/{y}"))
                })
                .collect()
        });
        for result in results {
            let block = result?;
            let tx = db.transaction()?;
            for l in block.levels {
                for (side, body) in l.boundaries.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO boundaries VALUES(?1,?2,?3,?4,?5)",
                        params![block.x, block.y, l.tolerance, side, body],
                    )?;
                }
                tx.execute(
                    "INSERT INTO meshes VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![
                        block.x,
                        block.y,
                        l.tolerance,
                        l.data,
                        l.normal,
                        l.vertices,
                        l.triangles,
                        l.error
                    ],
                )?;
            }
            tx.execute(
                "INSERT INTO blocks VALUES(?1,?2,?3,?4)",
                params![block.x, block.y, block.valid, block.mask],
            )?;
            tx.commit()?;
        }
        progress(&args.output, &db, all.len(), &start, "running")?;
    }
    let done: usize = db.query_row("SELECT count(*) FROM blocks", [], |r| r.get(0))?;
    if done == all.len() {
        verify(&db)?;
    }
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    progress(
        &args.output,
        &db,
        all.len(),
        &start,
        if done == all.len() {
            "complete"
        } else {
            "limited"
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn staged_minimums_and_command_line_are_consistent() {
        let expected = [4, 4, 512, 128, 64, 32];
        assert_eq!(minimum_for(10, 4, true), 4);
        assert_eq!(
            Args::try_parse_from([
                "test",
                "--source",
                "source.sqlite3",
                "--output",
                "out.sqlite3",
                "--tolerances",
                "10"
            ])
            .unwrap()
            .tolerances,
            vec![10]
        );
        assert_eq!(effective_minimum(100, 4, true, Some(40)), 40);
        assert_eq!(effective_minimum(100, 4, true, Some(1000)), 512);
        assert_eq!(effective_minimum(100, 512, false, Some(40)), 512);
        for (tolerance, floor) in ERRORS.into_iter().zip(expected) {
            assert_eq!(minimum_for(tolerance, 4, true), floor);
            assert_eq!(minimum_for(tolerance, 256, false), 256);
        }
        assert!(
            Args::try_parse_from([
                "test",
                "--source",
                "source.sqlite3",
                "--output",
                "out.sqlite3",
                "--staged-min-vertices"
            ])
            .is_ok()
        );
        assert!(
            Args::try_parse_from([
                "test",
                "--source",
                "source.sqlite3",
                "--output",
                "out.sqlite3",
                "--staged-min-vertices",
                "--min-vertices",
                "128"
            ])
            .is_err()
        );
    }
    #[test]
    fn existing_mesh_is_reused_only_when_it_meets_the_floor() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE meshes(x,y,tolerance,data BLOB,normal_data BLOB,vertices,triangles,max_error);CREATE TABLE boundaries(x,y,tolerance,side,data BLOB);").unwrap();
        db.execute(
            "INSERT INTO meshes VALUES(1,2,100,?1,?2,512,1000,90)",
            params![vec![1u8, 2], vec![3u8, 4]],
        )
        .unwrap();
        for side in 0..4 {
            db.execute(
                "INSERT INTO boundaries VALUES(1,2,100,?1,?2)",
                params![side, vec![side as u8]],
            )
            .unwrap();
        }
        let packed = reuse_level(&db, 1, 2, 100, 512).unwrap().unwrap();
        assert_eq!(packed.data, vec![1, 2]);
        assert_eq!(packed.normal, vec![3, 4]);
        assert_eq!(packed.boundaries[3], vec![3]);
        assert!(reuse_level(&db, 1, 2, 100, 1024).unwrap().is_none());
        assert!(reuse_level(&db, 1, 3, 100, 512).unwrap().is_none());
    }
    fn flat_tile(height: i32) -> Vec<u8> {
        let mut body = b"YMD1".to_vec();
        body.extend(256u16.to_le_bytes());
        body.extend(256u16.to_le_bytes());
        body.extend(5f32.to_le_bytes());
        body.extend((height / 5).to_le_bytes());
        body.extend([0, 0]);
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(&vec![0u8; 256 * 256 * 2]).unwrap();
        body.extend(gzip.finish().unwrap());
        body
    }
    #[test]
    fn source_overlap_uses_the_same_samples_and_tracks_missing_data() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(z,x,y));",
        )
        .unwrap();
        db.execute("INSERT INTO tiles VALUES(14,4,0,?1)", [flat_tile(15)])
            .unwrap();
        let (left, lmask, valid) = input_grid(&db, 0, 0).unwrap();
        let (right, rmask, _) = input_grid(&db, 1, 0).unwrap();
        assert_eq!(valid, 256);
        assert_eq!(left[1024], 15.);
        assert_eq!(right[0], 15.);
        assert_eq!(lmask[1024 / 8] & (1 << (1024 % 8)), 1 << (1024 % 8));
        assert_eq!(rmask[0] & 1, 1);
        assert_eq!(left[0], 0.);
        assert_eq!(lmask[0] & 1, 0);
        for y in 0..N {
            assert_eq!(left[y * N + N - 1], right[y * N]);
        }
    }
    #[test]
    fn flat_normals_and_boundary_packets_are_well_formed() {
        let grid = vec![10.; N * N];
        let vertices = vec![
            [0., 0., 10.],
            [1024., 0., 10.],
            [1024., 1024., 10.],
            [0., 1024., 10.],
        ];
        let packed = normals(&grid, &vertices, 10.).unwrap();
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(&packed[..])
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, vec![128u8; 8]);
        assert_eq!(boundary_bytes(&vertices, 0), boundary_bytes(&vertices, 1));
        assert_eq!(boundary_bytes(&vertices, 2), boundary_bytes(&vertices, 3));
    }
}
