//! 三地域の200km四方を、XYZの共通地域メッシュから生成・検証する試験入口。
#[allow(dead_code)]
mod adaptive_tin;
#[allow(dead_code)]
mod codec;
#[allow(dead_code)]
mod decimation_surface;
#[allow(dead_code)]
mod mesh;
#[allow(dead_code)]
mod mesh_vertex_format;
#[allow(dead_code)]
mod parent_vertex_reference;
mod summit_policy;
#[allow(dead_code)]
mod triangle_frontier32;
#[allow(dead_code)]
mod web_mercator;
mod xyz_national;
mod xyz_region_pack;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "build/terrain-lod.sqlite3")]
    source: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value = "tsukuba,yari,fuji", value_delimiter = ',')]
    regions: Vec<String>,
    #[arg(long, default_value_t = 200000.)]
    extent: f64,
    #[arg(long)]
    prepare_only: bool,
    #[arg(long)]
    source_sha256: String,
    #[arg(long)]
    national: bool,
    #[arg(long, default_value_t = 4096)]
    national_chunk_cells: usize,
    #[arg(long, value_delimiter = ',', num_args = 4)]
    national_window: Vec<i64>,
}
#[derive(Serialize, Deserialize, Clone)]
struct Region {
    name: String,
    latitude: f64,
    longitude: f64,
    n: usize,
    px: i64,
    py: i64,
    spacing: f64,
    extent: f64,
    source_sha256: String,
}
fn same_region(a: &Region, b: &Region) -> bool {
    a.name == b.name
        && a.n == b.n
        && a.px == b.px
        && a.py == b.py
        && a.source_sha256 == b.source_sha256
        && (a.spacing - b.spacing).abs() < 1e-12
        && (a.extent - b.extent).abs() < 1e-8
        && (a.latitude - b.latitude).abs() < 1e-10
        && (a.longitude - b.longitude).abs() < 1e-10
}
fn retry_shared_io<T>(
    mut action: impl FnMut() -> std::io::Result<T>,
    delay: std::time::Duration,
) -> std::io::Result<T> {
    for attempt in 0..20 {
        match action() {
            Ok(value) => return Ok(value),
            Err(error) if attempt < 19 && matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {
                std::thread::sleep(delay)
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}
fn progress(dir: &Path, v: serde_json::Value) -> Result<()> {
    let p = dir.join("progress.json");
    let tmp = dir.join("progress.next.json");
    let bytes = serde_json::to_vec_pretty(&v)?;
    let delay = std::time::Duration::from_millis(25);
    retry_shared_io(|| fs::write(&tmp, &bytes), delay)
        .with_context(|| format!("進捗一時ファイルの書込: {}", tmp.display()))?;
    retry_shared_io(|| fs::rename(&tmp, &p), delay)
        .with_context(|| format!("進捗ファイルの置換: {} -> {}", tmp.display(), p.display()))?;
    Ok(())
}
struct Source {
    db: Connection,
    cache: HashMap<(i64, i64), Option<codec::Grid>>,
    order: VecDeque<(i64, i64)>,
}
impl Source {
    fn open(p: &Path) -> Result<Self> {
        Ok(Self {
            db: Connection::open_with_flags(p, OpenFlags::SQLITE_OPEN_READ_ONLY)?,
            cache: HashMap::new(),
            order: VecDeque::new(),
        })
    }
    fn pixel(&mut self, x: i64, y: i64) -> Result<Option<f64>> {
        let key = (x.div_euclid(256), y.div_euclid(256));
        if !self.cache.contains_key(&key) {
            let b: Option<Vec<u8>> = self
                .db
                .prepare_cached("SELECT data FROM tiles WHERE z=14 AND x=?1 AND y=?2")?
                .query_row(params![key.0, key.1], |r| r.get(0))
                .optional()?;
            let g = b.map(|b| codec::decode_grid(&b)).transpose()?;
            if let Some(g) = &g {
                ensure!(
                    g.rows == 256 && g.columns == 256,
                    "元標高のサイズが不正です"
                );
            }
            if self.cache.len() >= 512 {
                if let Some(k) = self.order.pop_front() {
                    self.cache.remove(&k);
                }
            }
            self.order.push_back(key);
            self.cache.insert(key, g);
        }
        Ok(self.cache[&key].as_ref().and_then(|g| {
            let v = g.values[(y.rem_euclid(256) * 256 + x.rem_euclid(256)) as usize];
            v.is_finite().then_some(v as f64)
        }))
    }
    // 投影格子は画像のピクセル境界に置く。周囲4ピクセル中心を平均する。
    fn at(&mut self, x: i64, y: i64) -> Result<(f64, bool)> {
        let mut sum = 0.;
        for (dx, dy) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
            let Some(z) = self.pixel(x + dx, y + dy)? else {
                return Ok((0., false));
            };
            sum += z;
        }
        Ok((sum / 4., true))
    }
}
fn region(args: &Args, name: &str) -> Result<Region> {
    let (lat, lon) = match name {
        "tsukuba" => (36.2254, 140.1067),
        "yari" => (36.3419, 137.6476),
        "fuji" => (35.3606, 138.7274),
        _ => anyhow::bail!("未知の地域です: {name}"),
    };
    let [x, y] = web_mercator::project(lon, lat)?;
    let spacing = web_mercator::width(14) / 256.;
    let m = web_mercator::half_world();
    let cells = ((args.extent / spacing / 8.).ceil() as usize) * 8;
    let px = (((x + m) / spacing / 4.).round() as i64) * 4 - cells as i64 / 2;
    let py = (((m - y) / spacing / 4.).round() as i64) * 4 - cells as i64 / 2;
    Ok(Region {
        name: name.into(),
        latitude: lat,
        longitude: lon,
        n: cells + 1,
        px,
        py,
        spacing,
        extent: cells as f64 * spacing,
        source_sha256: args.source_sha256.clone(),
    })
}
fn prepare(args: &Args, r: &Region, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    let meta = dir.join("region.json");
    if meta.exists() {
        ensure!(
            same_region(&serde_json::from_slice::<Region>(&fs::read(&meta)?)?, r),
            "保存済み地域設定が異なります"
        );
    } else {
        fs::write(meta, serde_json::to_vec_pretty(r)?)?;
    }
    let path = dir.join("dem.f64");
    let expected =
        r.n.checked_mul(r.n)
            .and_then(|v| v.checked_mul(8))
            .context("DEMサイズ超過")? as u64;
    if path.exists() {
        ensure!(
            fs::metadata(path)?.len() == expected,
            "保存DEMの長さが不一致です"
        );
        return Ok(());
    }
    let partial = dir.join("dem.partial.f64");
    let mask = dir.join("valid.partial.bits");
    let mut out = BufWriter::with_capacity(1024 * 1024, File::create(&partial)?);
    let mut validity = BufWriter::new(File::create(&mask)?);
    let mut src = Source::open(&args.source)?;
    let start = Instant::now();
    let mut valid = 0u64;
    for y in 0..r.n {
        let mut bit = 0u8;
        let mut used = 0;
        for x in 0..r.n {
            let (z, ok) = src.at(r.px + x as i64, r.py + y as i64)?;
            out.write_all(&z.to_le_bytes())?;
            if ok {
                valid += 1;
                bit |= 1 << used;
            }
            used += 1;
            if used == 8 {
                validity.write_all(&[bit])?;
                used = 0;
                bit = 0;
            }
        }
        if used > 0 {
            validity.write_all(&[bit])?;
        }
        if y % 16 == 0 || y + 1 == r.n {
            progress(
                dir,
                json!({"stage":"prepare-dem","region":r.name,"rows":y+1,"totalRows":r.n,"elapsedSeconds":start.elapsed().as_secs_f64(),"validSamples":valid,"projectionExtentMeters":r.extent,"complete":false}),
            )?;
        }
    }
    out.flush()?;
    validity.flush()?;
    drop(out);
    drop(validity);
    fs::rename(partial, path)?;
    fs::rename(mask, dir.join("valid.bits"))?;
    progress(
        dir,
        json!({"stage":"dem-prepared","region":r.name,"samples":r.n*r.n,"validSamples":valid,"complete":false}),
    )?;
    Ok(())
}
pub(crate) fn read_level(path: &Path) -> Result<(Vec<[i64; 2]>, Vec<[usize; 3]>, Vec<f64>)> {
    let mut f = std::io::BufReader::new(File::open(path)?);
    let mut h = [0u8; 12];
    f.read_exact(&mut h)?;
    ensure!(&h[..4] == b"XRG1", "地域中間形状の形式が不正です");
    let nv = u32::from_le_bytes(h[4..8].try_into().unwrap()) as usize;
    let nf = u32::from_le_bytes(h[8..12].try_into().unwrap()) as usize;
    ensure!(
        fs::metadata(path)?.len() == 12 + nv as u64 * 16 + nf as u64 * 12,
        "地域中間形状の長さが不正です"
    );
    let mut points = Vec::with_capacity(nv);
    let mut z = Vec::with_capacity(nv);
    for _ in 0..nv {
        let mut b = [0; 16];
        f.read_exact(&mut b)?;
        points.push([
            u32::from_le_bytes(b[..4].try_into().unwrap()) as i64,
            u32::from_le_bytes(b[4..8].try_into().unwrap()) as i64,
        ]);
        z.push(f64::from_le_bytes(b[8..].try_into().unwrap()));
    }
    let mut faces = Vec::with_capacity(nf);
    for _ in 0..nf {
        let mut b = [0; 12];
        f.read_exact(&mut b)?;
        let t = std::array::from_fn(|i| {
            u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()) as usize
        });
        ensure!(t.iter().all(|&v| v < nv), "地域中間形状の番号範囲");
        faces.push(t);
    }
    Ok((points, faces, z))
}
fn save_level(
    dir: &Path,
    z: u32,
    surface: &decimation_surface::Surface,
    report: serde_json::Value,
) -> Result<()> {
    let (p, f, _) = surface.compact();
    let path = dir.join(format!("z{z}.mesh"));
    let tmp = dir.join(format!("z{z}.partial.mesh"));
    let mut out = BufWriter::new(File::create(&tmp)?);
    out.write_all(b"XRG1")?;
    out.write_all(&(p.len() as u32).to_le_bytes())?;
    out.write_all(&(f.len() as u32).to_le_bytes())?;
    for xy in &p {
        out.write_all(&(xy[0] as u32).to_le_bytes())?;
        out.write_all(&(xy[1] as u32).to_le_bytes())?;
        out.write_all(&surface.grid[xy[1] as usize * surface.n + xy[0] as usize].to_le_bytes())?;
    }
    for face in &f {
        for &v in face {
            out.write_all(&(v as u32).to_le_bytes())?;
        }
    }
    out.flush()?;
    drop(out);
    fs::rename(tmp, path)?;
    fs::write(
        dir.join(format!("z{z}.report.json")),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
fn generate_levels(r: &Region, dir: &Path) -> Result<()> {
    if (6..=14).all(|z| dir.join(format!("z{z}.mesh")).exists()) {
        return Ok(());
    }
    let n = r.n;
    progress(
        dir,
        json!({"stage":"read-dem","region":r.name,"complete":false}),
    )?;
    let mut grid = vec![0f64; n * n];
    // f64の有効なビット列だけを書き込む。エンディアンはWindows/x86_64のLE。
    ensure!(
        cfg!(target_endian = "little"),
        "地域試験の入力読込はLE環境が必要です"
    );
    let bytes =
        unsafe { std::slice::from_raw_parts_mut(grid.as_mut_ptr().cast::<u8>(), grid.len() * 8) };
    let dem_path = dir.join("dem.f64");
    retry_shared_io(
        || File::open(&dem_path),
        std::time::Duration::from_millis(25),
    )
    .with_context(|| format!("元標高を開く: {}", dem_path.display()))?
    .read_exact(bytes)
    .with_context(|| format!("元標高を読む: {}", dem_path.display()))?;
    progress(
        dir,
        json!({"stage":"terrain-importance","region":r.name,"samples":n*n,"complete":false}),
    )?;
    let weights = decimation_surface::importance(&grid, n, r.spacing);
    let (scales, peak_count) = summit_policy::summit_scales(&grid, n, r.spacing);
    generate_fields(r, dir, grid, weights, scales, peak_count)
}
// 全国側は作業領域の外も参照して求めた地形指標を渡す。
fn generate_fields(
    r: &Region,
    dir: &Path,
    grid: Vec<f64>,
    weights: Vec<f64>,
    scales: Vec<f64>,
    peak_count: usize,
) -> Result<()> {
    let n = r.n;
    progress(
        dir,
        json!({"stage":"summit-constraints","region":r.name,"complete":false}),
    )?;
    let mut scales = Some(scales);
    let resume = (6..=14).find(|z| dir.join(format!("z{z}.mesh")).exists());
    let (p, f, last) = if let Some(z) = resume {
        let (p, f, _) = read_level(&dir.join(format!("z{z}.mesh")))?;
        (p, f, z)
    } else {
        progress(
            dir,
            json!({"stage":"finest-tin","region":r.name,"z":14,"complete":false}),
        )?;
        let m = adaptive_tin::quality_tin::generate_with_spatial_seeds_progress(
            &grid,
            n,
            12.375,
            10.,
            Some(48),
            |part, vertices, round| {
                progress(
                    dir,
                    json!({"stage":"finest-tin","part":part,"region":r.name,"z":14,"vertices":vertices,"round":round,"complete":false}),
                )
            },
        )?;
        (m.points, m.faces, 14)
    };
    let mut surface = decimation_surface::Surface::new(p, f, grid, weights, n);
    surface.adaptive_boundary = true;
    // 初期の最詳細だけは山頂係数なし。以降は同じ点集合から削減する。
    if last < 14 {
        surface.error_scales = scales.take();
    }
    for z in (6..=last).rev() {
        if dir.join(format!("z{z}.mesh")).exists() {
            continue;
        }
        let started = Instant::now();
        let tolerance = 12.5 * 2f64.powi(14 - z as i32);
        surface.maximum_face_span = Some((256. * 2f64.powi(14 - z as i32) * 0.49).floor() as i64);
        surface.use_exact_density(256. * 2f64.powi(14 - z as i32) / 2.);
        progress(
            dir,
            json!({"stage":"reduce-mesh","region":r.name,"z":z,"errorMeters":tolerance,"complete":false}),
        )?;
        let removed = if z == 14 {
            0
        } else {
            if surface.error_scales.is_none() {
                surface.error_scales = scales.take();
            }
            let mut write_error = None;
            let mut updated = Instant::now() - std::time::Duration::from_secs(2);
            let result = surface.reduce_with_progress(tolerance - 0.125, 4, |part, processed, total, queued| {
                if updated.elapsed().as_secs_f64() >= 1. || processed == total || part == "reduction-finished" {
                    if let Err(error) = progress(dir, json!({"stage":"reduce-mesh","part":part,
                        "region":r.name,"z":z,"errorMeters":tolerance,"processed":processed,
                        "total":total,"queued":queued,"elapsedSeconds":started.elapsed().as_secs_f64(),"complete":false})) {
                        write_error = Some(error);
                    }
                    updated = Instant::now();
                }
            });
            if let Some(error) = write_error {
                return Err(error);
            }
            result.0
        };
        progress(
            dir,
            json!({"stage":"reduce-mesh","part":"audit","region":r.name,"z":z,
            "removed":removed,"elapsedSeconds":started.elapsed().as_secs_f64(),"complete":false}),
        )?;
        let (max, weighted, angle) = surface.audit();
        let local = surface
            .faces
            .iter()
            .flatten()
            .map(|&f| surface.metric(f).2)
            .fold(0., f64::max);
        ensure!(
            max <= tolerance + 1e-6 && local <= tolerance + 1e-6 && angle >= 10. - 1e-7,
            "地域生成の誤差・角度に不合格です"
        );
        let vertices = surface.live.iter().filter(|&&v| v).count();
        let report = json!({"z":z,"errorMeters":tolerance,"maximum":max,"localConstraintMaximum":local,"weightedMaximum":weighted,"minimumAngle":angle,"vertices":vertices,"removed":removed,"detectedPeaks":peak_count,"seconds":started.elapsed().as_secs_f64(),"stage":"geometry-saved","region":r.name,"complete":false});
        save_level(dir, z, &surface, report.clone())?;
        progress(dir, report)?;
    }
    Ok(())
}
fn main() -> Result<()> {
    let args = Args::parse();
    let workers = std::thread::available_parallelism()
        .map(|v| (v.get() / 2).max(1))
        .unwrap_or(1);
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build_global()?;
    ensure!(
        args.extent.is_finite() && args.extent > 0.,
        "範囲が不正です"
    );
    ensure!(
        args.source_sha256.len() == 64 && args.source_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "元標高のSHA256が必要です"
    );
    fs::create_dir_all(&args.output)?;
    if args.national {
        return xyz_national::run(&args);
    }
    for name in &args.regions {
        let r = region(&args, name)?;
        let dir = args.output.join(name);
        prepare(&args, &r, &dir)?;
        if !args.prepare_only {
            let result = generate_levels(&r, &dir).and_then(|_| xyz_region_pack::pack(&r, &dir));
            if let Err(error) = result {
                progress(
                    &dir,
                    json!({"stage":"failed","region":r.name,"error":format!("{error:#}"),"complete":false}),
                )?;
                return Err(error);
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn region_is_aligned_to_all_quantization_grids() {
        let a = Args {
            source: PathBuf::new(),
            output: PathBuf::new(),
            regions: vec![],
            extent: 200000.,
            prepare_only: true,
            source_sha256: "0".repeat(64),
            national: false,
            national_chunk_cells: 4096,
            national_window: vec![],
        };
        for name in ["tsukuba", "yari", "fuji"] {
            let r = region(&a, name).unwrap();
            assert_eq!((r.n - 1) % 4, 0);
            assert_eq!(r.px % 4, 0);
            assert_eq!(r.py % 4, 0);
            assert!(r.extent >= 200000. && r.extent < 200040.);
        }
    }
}

#[cfg(test)]
mod shared_io_tests {
    use super::*;
    #[test]
    fn retries_only_transient_windows_sharing_errors() {
        let mut attempts = 0;
        let value = retry_shared_io(
            || {
                attempts += 1;
                if attempts < 4 {
                    Err(std::io::Error::from_raw_os_error(32))
                } else {
                    Ok(7)
                }
            },
            std::time::Duration::ZERO,
        )
        .unwrap();
        assert_eq!((value, attempts), (7, 4));
        let mut attempts = 0;
        let result: std::io::Result<()> = retry_shared_io(
            || {
                attempts += 1;
                Err(std::io::Error::from_raw_os_error(2))
            },
            std::time::Duration::ZERO,
        );
        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }
}
