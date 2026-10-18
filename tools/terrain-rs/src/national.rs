use crate::{
    coast, codec, dem,
    geos::Geos,
    mesh::{Mesh, Point, Surface},
    qem::Qem,
    stitch,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
pub const ERRORS: [f64; 11] = [
    600., 450., 300., 225., 150., 112.5, 75., 50., 25., 17.5, 10.,
];
pub const COASTS: [f64; 11] = [250., 200., 150., 100., 65., 45., 30., 20., 15., 12.5, 10.];
pub fn zoom(level: u32) -> u32 {
    (level - 2).max(2)
}
fn readonly(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}
fn point_key(p: Point) -> [u64; 3] {
    p.map(f64::to_bits)
}
fn protected_read(body: &[u8]) -> Result<Vec<Point>> {
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(body)
        .take(12_000_001)
        .read_to_end(&mut raw)?;
    ensure!(
        raw.len() % 12 == 0 && raw.len() <= 12_000_000,
        "固定頂点の長さが不正です"
    );
    Ok(raw
        .chunks_exact(12)
        .map(|c| {
            std::array::from_fn(|i| {
                f32::from_le_bytes(c[i * 4..i * 4 + 4].try_into().unwrap()) as f64
            })
        })
        .collect())
}
fn protected_write(points: &[Point]) -> Result<Vec<u8>> {
    let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    for p in points {
        for v in p {
            writer.write_all(&(*v as f32).to_le_bytes())?;
        }
    }
    Ok(writer.finish()?)
}
struct Part {
    mesh: Mesh,
    protected: Vec<Point>,
    height: f64,
    coast: f64,
    dx: u32,
    dy: u32,
    factor: u32,
}
#[cfg(test)]
fn merge(parts: Vec<Part>) -> Result<(Mesh, Vec<Point>, f64, f64)> {
    merge_with_scale(parts, 1.)
}
fn merge_with_scale(parts: Vec<Part>, meters: f64) -> Result<(Mesh, Vec<Point>, f64, f64)> {
    let mut mesh = Mesh::default();
    let mut protected = Vec::new();
    let mut lookup: BTreeMap<[u64; 2], usize> = BTreeMap::new();
    let mut exact_heights = BTreeMap::new();
    let mut exact_indices = BTreeMap::new();
    let mut coordinate_shift = 0f64;
    let mut ranges: Vec<[f64; 2]> = Vec::new();
    let mut height = 0f64;
    let mut coast = 0f64;
    for mut part in parts {
        let mut mapping = Vec::new();
        let transform = |mut p: Point| {
            p[0] = ((p[0] + part.dx as f64 * 256.) / part.factor as f64) as f32 as f64;
            p[1] = ((p[1] + part.dy as f64 * 256.) / part.factor as f64) as f32 as f64;
            p
        };
        for p in &mut part.mesh.vertices {
            let exact = [
                ((p[0] + part.dx as f64 * 256.) / part.factor as f64).to_bits(),
                ((p[1] + part.dy as f64 * 256.) / part.factor as f64).to_bits(),
            ];
            if let Some(old) = exact_heights.insert(exact, p[2]) {
                ensure!(old == p[2], "共有頂点の標高が一致しません");
            }
            *p = transform(*p);
            if let Some(&index) = exact_indices.get(&exact) {
                mapping.push(index);
                continue;
            }
            let mut key = [p[0].to_bits(), p[1].to_bits()];
            if let Some(&index) = lookup.get(&key) {
                if ranges[index][1].max(p[2]) - ranges[index][0].min(p[2]) > 10. {
                    let raw = [f64::from_bits(exact[0]), f64::from_bits(exact[1])];
                    let xy = separate_alias(raw, [p[0], p[1]], &lookup)?;
                    coordinate_shift = coordinate_shift.max((xy[0] - raw[0]).hypot(xy[1] - raw[1]));
                    p[0] = xy[0];
                    p[1] = xy[1];
                    key = [p[0].to_bits(), p[1].to_bits()];
                    eprintln!("標高差の大きい近接頂点を、保存精度内で別の座標として保持しました");
                }
            }
            let index = if let Some(&index) = lookup.get(&key) {
                ranges[index][0] = ranges[index][0].min(p[2]);
                ranges[index][1] = ranges[index][1].max(p[2]);
                ensure!(
                    ranges[index][1] - ranges[index][0] <= 10.,
                    "座標丸めで合流する頂点を、各標高の変更 5 m 以内で統一できません"
                );
                index
            } else {
                let index = mesh.vertices.len();
                mesh.vertices.push(*p);
                ranges.push([p[2], p[2]]);
                lookup.insert(key, index);
                index
            };
            exact_indices.insert(exact, index);
            mapping.push(index);
        }
        mesh.faces
            .extend(part.mesh.faces.iter().map(|f| f.map(|i| mapping[i])));
        protected.extend(part.protected.into_iter().map(|p| {
            let exact = [
                ((p[0] + part.dx as f64 * 256.) / part.factor as f64).to_bits(),
                ((p[1] + part.dy as f64 * 256.) / part.factor as f64).to_bits(),
            ];
            exact_indices
                .get(&exact)
                .map_or_else(|| transform(p), |i| mesh.vertices[*i])
        }));
        height = height.max(part.height);
        coast = coast.max(part.coast);
    }
    let mut adjustment = 0f64;
    let mut joined = 0;
    for (point, [lo, hi]) in mesh.vertices.iter_mut().zip(ranges) {
        if lo != hi {
            // f32 化で区別できなくなる座標だけを統一する。海岸の 0 m を優先する。
            point[2] = if lo == 0. && hi <= 5. {
                0.
            } else {
                ((lo + hi) / 10.).round_ties_even() * 5.
            };
            let change = (point[2] - lo).abs().max((point[2] - hi).abs());
            ensure!(change <= 5., "合流する頂点の標高変更が 5 m を超えています");
            adjustment = adjustment.max(change);
            joined += 1;
        }
    }
    if joined > 0 {
        eprintln!(
            "座標丸めで合流する頂点 {joined} 個の標高をそろえ、最大 {adjustment} m を誤差に加算しました"
        );
    }
    Ok((
        mesh,
        protected,
        height + adjustment,
        coast + coordinate_shift * meters,
    ))
}
fn separate_alias(
    raw: [f64; 2],
    rounded: [f64; 2],
    lookup: &BTreeMap<[u64; 2], usize>,
) -> Result<[f64; 2]> {
    let step = |v: f64, n: i32| {
        let mut f = v as f32;
        for _ in 0..n.abs() {
            f = if n > 0 { f.next_up() } else { f.next_down() };
        }
        f as f64
    };
    let mut candidates = Vec::new();
    for dx in -2i32..=2 {
        for dy in -2i32..=2 {
            let xy = [step(rounded[0], dx), step(rounded[1], dy)];
            if (0..2).any(|a| {
                xy[a] < 0. || xy[a] > 256. || ((raw[a] == 0. || raw[a] == 256.) && xy[a] != raw[a])
            }) || lookup.contains_key(&xy.map(f64::to_bits))
            {
                continue;
            }
            let distance = (xy[0] - raw[0]).powi(2) + (xy[1] - raw[1]).powi(2);
            let direction = -((xy[0] - rounded[0]) * (raw[0] - rounded[0])
                + (xy[1] - rounded[1]) * (raw[1] - rounded[1]));
            candidates.push((distance, direction, xy));
        }
    }
    candidates.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2[0].total_cmp(&b.2[0]))
            .then(a.2[1].total_cmp(&b.2[1]))
    });
    candidates
        .first()
        .map(|c| c.2)
        .ok_or_else(|| anyhow::anyhow!("保存精度内で異なる頂点を保持できません"))
}
pub struct Converted {
    pub level: u32,
    pub x: u32,
    pub y: u32,
    pub body: Option<Vec<u8>>,
    pub vertices: usize,
    pub triangles: usize,
    pub height: f64,
    pub coast: f64,
    pub protected: Vec<u8>,
    pub source: usize,
    pub quality: serde_json::Value,
}
fn adopt_reduced(
    source: &Mesh,
    physical: &Mesh,
    reduced: Mesh,
    meters: f64,
    budget: f64,
) -> Result<(Mesh, f64, serde_json::Value)> {
    let original: BTreeMap<_, _> = physical
        .vertices
        .iter()
        .zip(&source.vertices)
        .map(|(a, b)| (point_key(*a), *b))
        .collect();
    let mut candidate = Mesh {
        vertices: reduced
            .vertices
            .iter()
            .map(|p| original[&point_key(*p)])
            .collect(),
        faces: reduced.faces,
    };
    let before = candidate.faces.len();
    candidate
        .faces
        .retain(|f| crate::mesh::normal(&candidate.vertices, *f)[2] * meters * meters > 1e-10);
    candidate = candidate.compact();
    candidate.validate().context("削減結果の保存形式")?;
    let represented = Mesh {
        vertices: candidate
            .vertices
            .iter()
            .map(|p| [p[0] * meters, p[1] * meters, p[2]])
            .collect(),
        faces: candidate.faces.clone(),
    };
    let surface = Surface::new(&represented);
    let mut missing = 0;
    let mut maximum = 0f64;
    let probes = physical.probes();
    for p in &probes {
        if let Some(height) = surface.height([p[0], p[1]]) {
            maximum = maximum.max((height - p[2]).abs());
        } else {
            missing += 1;
        }
    }
    let report = serde_json::json!({"passed":missing==0&&maximum<=budget+0.001,"sampledMaxErrorMeters":maximum,"allowedStageErrorMeters":budget,"missingProbes":missing,"probeCount":probes.len(),"removedDegenerateFaces":before-candidate.faces.len(),"sourceTriangles":source.faces.len(),"triangles":candidate.faces.len(),"fallback":false});
    Ok((candidate, maximum, report))
}
pub fn convert(
    source: &Path,
    output: &Path,
    library: &Path,
    coasts: &Path,
    level: u32,
    x: u32,
    y: u32,
) -> Result<Converted> {
    let z = zoom(level);
    let latitude = (std::f64::consts::PI * (1. - 2. * (y as f64 + 0.5) / 2f64.powi(z as i32)))
        .sinh()
        .atan();
    let meters =
        2. * std::f64::consts::PI * 6_378_137. * latitude.cos() / (256. * 2f64.powi(z as i32));
    let mut parts = Vec::new();
    if level == 14 {
        let db = readonly(source)?;
        for dy in 0..4 {
            for dx in 0..4 {
                let grid = dem::read_grid(&db, 14, x * 4 + dx, y * 4 + dy)?;
                let mesh = dem::simplify(&grid, 5.)
                    .context("元 DEM の三角形化（代替の格子へは戻さない）")?;
                if !mesh.faces.is_empty() {
                    parts.push(Part {
                        mesh,
                        protected: dem::extrema(&grid, meters / 4.),
                        height: 5.,
                        coast: 0.,
                        dx,
                        dy,
                        factor: 4,
                    });
                }
            }
        }
    } else {
        let db = readonly(output)?;
        let factor = 1 << (zoom(level + 1) - z);
        for dy in 0..factor {
            for dx in 0..factor {
                let row:Option<(Vec<u8>,Vec<u8>,f64,f64)>=db.query_row("SELECT t.data,m.protected,m.height_error,m.coast_error FROM tiles t JOIN metrics m USING(z,x,y) WHERE t.z=?1 AND t.x=?2 AND t.y=?3",(level+1,x*factor+dx,y*factor+dy),|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
                if let Some((body, protected, height, coast)) = row {
                    parts.push(Part {
                        mesh: codec::decode_mesh(&body)?,
                        protected: protected_read(&protected)?,
                        height,
                        coast,
                        dx,
                        dy,
                        factor,
                    });
                }
            }
        }
    }
    let (mut mesh, protected, previous, coast_error) = merge_with_scale(parts, meters)?;
    if previous > ERRORS[(level - 4) as usize] + 0.001 {
        eprintln!(
            "{}",
            serde_json::json!({"state":"inherited-error-warning","level":level,"x":x,"y":y,"errorMeters":previous})
        );
    }
    mesh.faces
        .retain(|f| crate::mesh::normal(&mesh.vertices, *f)[2] * meters * meters > 1e-10);
    mesh = mesh.compact();
    if let Ok(path) = std::env::var("TERRAIN_DIAGNOSTIC_MESH") {
        std::fs::write(format!("{path}-merged.json"), serde_json::to_vec(&mesh)?)?;
    }
    if !mesh.faces.is_empty() {
        let api = Geos::open(library)?;
        let land = api.read(&std::fs::read(coasts.join(format!("{level}.wkb")))?)?;
        mesh = coast::clip(&api, &mesh, &land, level, x, y).context("海岸線の切り出し")?;
    }
    let source_count = mesh.faces.len();
    if let Ok(path) = std::env::var("TERRAIN_DIAGNOSTIC_MESH") {
        std::fs::write(format!("{path}-coast.json"), serde_json::to_vec(&mesh)?)?;
    }
    let mut stage_error = 0.;
    let mut quality = serde_json::json!({"passed":true,"missingProbes":0,"fallback":false,"sourceTriangles":0,"triangles":0});
    if !mesh.faces.is_empty() {
        let physical = Mesh {
            vertices: mesh
                .vertices
                .iter()
                .map(|p| [p[0] * meters, p[1] * meters, p[2]])
                .collect(),
            faces: mesh.faces.clone(),
        };
        let lookup: BTreeMap<_, _> = mesh
            .vertices
            .iter()
            .enumerate()
            .map(|(i, p)| (point_key(*p), i))
            .collect();
        let fixed: Vec<_> = protected
            .iter()
            .filter_map(|p| lookup.get(&point_key(*p)).copied())
            .collect();
        let probes = physical.probes();
        let mut q =
            Qem::new(physical.clone(), &fixed, Some(probes.clone())).context("削減器の初期化")?;
        let budget = (ERRORS[(level - 4) as usize] - previous).max(0.);
        let reduced = q.reduce(
            ((source_count as f64 * if level == 14 { 0.8 } else { 0.5 }) as usize).max(2),
            budget,
        )?;
        let (candidate, error, report) = adopt_reduced(&mesh, &physical, reduced, meters, budget)?;
        mesh = candidate;
        stage_error = error;
        quality = report;
        if quality["passed"] != true {
            eprintln!(
                "{}",
                serde_json::json!({"state":"quality-warning","level":level,"x":x,"y":y,"report":quality})
            );
        }
    }
    let present: BTreeSet<_> = mesh.vertices.iter().map(|p| point_key(*p)).collect();
    let protected: Vec<_> = protected
        .into_iter()
        .filter(|p| present.contains(&point_key(*p)))
        .collect();
    let body = if mesh.faces.is_empty() {
        None
    } else {
        let body = codec::encode_mesh(&mesh).context("地形の圧縮")?;
        let restored = codec::decode_mesh(&body).context("地形の復元")?;
        ensure!(
            restored.vertices == mesh.vertices && restored.faces == mesh.faces,
            "保存した頂点または面が一致しません"
        );
        Some(body)
    };
    Ok(Converted {
        level,
        x,
        y,
        body,
        vertices: mesh.vertices.len(),
        triangles: mesh.faces.len(),
        height: previous + stage_error,
        coast: coast_error,
        protected: protected_write(&protected)?,
        source: source_count,
        quality,
    })
}
fn progress(path: &Path, value: &serde_json::Value) -> Result<()> {
    let temp = path.with_extension("tmp.json");
    std::fs::write(&temp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(temp, path)?;
    println!("{value}");
    Ok(())
}
fn completed_levels(db: &Connection) -> Result<Vec<u32>> {
    let value: Option<String> = db
        .query_row(
            "SELECT value FROM metadata WHERE key='completedLevels'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match value {
        Some(value) => serde_json::from_str(&value)?,
        None => Vec::new(),
    })
}
#[cfg(windows)]
fn free_bytes(path: &Path) -> Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            path: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let folder = path.parent().unwrap_or(Path::new("."));
    let wide: Vec<_> = folder
        .canonicalize()?
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut available = 0;
    ensure!(
        unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } != 0,
        "空き容量を確認できません"
    );
    Ok(available)
}
#[cfg(not(windows))]
fn free_bytes(_path: &Path) -> Result<u64> {
    Ok(u64::MAX)
}
pub fn generate(
    source: &Path,
    output: &Path,
    library: &Path,
    coast_source: &Path,
    workers: usize,
    limit: Option<usize>,
) -> Result<()> {
    ensure!(source != output, "入力と出力に同じパスは指定できません");
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    ensure!(
        free_bytes(output)? >= 5 * 1024 * 1024 * 1024,
        "空き容量が 5 GiB 未満です"
    );
    let started = Instant::now();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()?;
    let src = readonly(source)?;
    let mut db = Connection::open(output)?;
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS tiles(z INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(z,x,y));CREATE TABLE IF NOT EXISTS metrics(z INTEGER,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER,height_error REAL,coast_error REAL,protected BLOB,source_triangles INTEGER,PRIMARY KEY(z,x,y));CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);")?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS quality(z INTEGER,x INTEGER,y INTEGER,report TEXT,PRIMARY KEY(z,x,y));")?;
    let identity = serde_json::json!({"generator":"rust-2-no-fallback","source":source.canonicalize()?.to_string_lossy(),"sourceBytes":std::fs::metadata(source)?.len(),"sourceModified":format!("{:?}",std::fs::metadata(source)?.modified()?),"coastBytes":std::fs::metadata(coast_source)?.len(),"coastModified":format!("{:?}",std::fs::metadata(coast_source)?.modified()?)});
    let old: Option<String> = db
        .query_row(
            "SELECT value FROM metadata WHERE key='rustIdentity'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(old) = old {
        ensure!(
            serde_json::from_str::<serde_json::Value>(&old)? == identity,
            "再開条件が異なります"
        );
    } else {
        let count: i64 = db.query_row("SELECT count(*) FROM tiles", [], |r| r.get(0))?;
        ensure!(count == 0, "別の生成器の出力には追記できません");
        db.execute(
            "INSERT INTO metadata VALUES ('rustIdentity',?1)",
            [identity.to_string()],
        )?;
    }
    let status_path = output.with_extension("progress.json");
    progress(
        &status_path,
        &serde_json::json!({"state":"preparing","generator":"rust","stage":"coastlines"}),
    )?;
    let folder = output.with_extension("coasts");
    std::fs::create_dir_all(&folder)?;
    let api = Geos::open(library)?;
    let land = api.read(&std::fs::read(coast_source)?)?;
    for level in 4..=14 {
        let file = folder.join(format!("{level}.wkb"));
        if !file.exists() {
            let simplified = land.simplify(COASTS[(level - 4) as usize])?;
            let temp = file.with_extension("part");
            std::fs::write(&temp, simplified.write()?)?;
            std::fs::rename(temp, file)?;
        }
    }
    let metadata = serde_json::json!({"schemaFormat":"yamamuki-terrain-mesh","schemaVersion":3,"generatorVersion":"rust-2-no-fallback","experimental":true,"fallbackEnabled":false,"qualityPolicy":"削減結果を採用し、標本の欠落と誤差超過は quality テーブルに記録する。保存形式や接続の不正は停止する。","encoding":"YMP4","levels":(4..=14).collect::<Vec<_>>(),"geometryZoomByLevel":(4..=14).map(|l|(l.to_string(),zoom(l))).collect::<BTreeMap<_,_>>(),"pixelOrigin":"highest-grid-corners","meshErrorBounds":(4..=14).map(|l|(l.to_string(),ERRORS[(l-4) as usize])).collect::<BTreeMap<_,_>>(),"coastlineToleranceMeters":(4..=14).map(|l|(l.to_string(),COASTS[(l-4) as usize])).collect::<BTreeMap<_,_>>(),"generationComplete":false,"coastOverlapNumericalToleranceMeters":0.001,"source":"国土地理院 DEM10B レベル 14 から QEM で段階的に削減","errorPolicy":"入力の頂点・面中心で測った段階誤差を累積。海岸移動部分と境界調整を除く。"});
    for (key, value) in metadata.as_object().unwrap() {
        db.execute(
            "INSERT OR REPLACE INTO metadata VALUES (?1,?2)",
            (key, value.to_string()),
        )?;
    }
    let mut handled = 0;
    let mut completed = completed_levels(&db)?;
    for level in (4..=14).rev() {
        if completed.contains(&level) {
            continue;
        }
        let factor = if level == 14 {
            4
        } else {
            1 << (zoom(level + 1) - zoom(level))
        };
        let coords = {
            let input = if level == 14 { &src } else { &db };
            let mut statement=input.prepare("SELECT DISTINCT CAST(x/?1 AS INTEGER),CAST(y/?1 AS INTEGER) FROM tiles WHERE z=?2 ORDER BY 1,2")?;
            statement
                .query_map((factor, if level == 14 { 14 } else { level + 1 }), |r| {
                    Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let done = {
            let mut statement = db.prepare("SELECT x,y FROM metrics WHERE z=?1")?;
            statement
                .query_map([level], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))?
                .collect::<rusqlite::Result<BTreeSet<_>>>()?
        };
        let pending: Vec<_> = coords
            .iter()
            .filter(|p| !done.contains(p))
            .copied()
            .collect();
        let mut count = done.len();
        for chunk in pending.chunks(if level <= 7 {
            1
        } else if level <= 10 {
            workers.min(2)
        } else {
            workers
        }) {
            if limit.is_some_and(|limit| handled >= limit) {
                progress(
                    &status_path,
                    &serde_json::json!({"state":"limited","generator":"rust","generatedThisRun":handled,"generationComplete":false}),
                )?;
                return Ok(());
            }
            let selected = &chunk[..chunk.len().min(limit.map_or(usize::MAX, |l| l - handled))];
            use rayon::prelude::*;
            let results: Vec<_> = pool.install(|| {
                selected
                    .par_iter()
                    .map(|&(x, y)| {
                        convert(source, output, library, &folder, level, x, y)
                            .with_context(|| format!("レベル {level} ({x}, {y})"))
                    })
                    .collect()
            });
            for result in results {
                let r = result?;
                let transaction = db.transaction()?;
                if let Some(body) = r.body {
                    transaction.execute(
                        "INSERT OR REPLACE INTO tiles VALUES (?1,?2,?3,?4)",
                        (r.level, r.x, r.y, body),
                    )?;
                }
                transaction.execute(
                    "INSERT OR REPLACE INTO metrics VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    (
                        r.level,
                        r.x,
                        r.y,
                        r.vertices,
                        r.triangles,
                        r.height,
                        r.coast,
                        r.protected,
                        r.source,
                    ),
                )?;
                transaction.execute(
                    "INSERT OR REPLACE INTO quality VALUES (?1,?2,?3,?4)",
                    (r.level, r.x, r.y, r.quality.to_string()),
                )?;
                transaction.commit()?;
                count += 1;
                handled += 1;
            }
            let free = free_bytes(output)?;
            let wal = PathBuf::from(format!("{}-wal", output.display()));
            let size =
                std::fs::metadata(output)?.len() + std::fs::metadata(wal).map_or(0, |m| m.len());
            progress(
                &status_path,
                &serde_json::json!({"state":"running","generator":"rust","level":level,"completedInLevel":count,"totalInLevel":coords.len(),"completedLevels":completed,"secondsThisRun":started.elapsed().as_secs_f64(),"sqliteBytes":size,"freeBytes":free}),
            )?;
            ensure!(
                free >= 2 * 1024 * 1024 * 1024,
                "空き容量が 2 GiB 未満のため保存して停止しました"
            );
        }
        stitch::stitch(&mut db, level)?;
        completed.push(level);
        db.execute(
            "INSERT OR REPLACE INTO metadata VALUES ('completedLevels',?1)",
            [serde_json::to_string(&completed)?],
        )?;
    }
    // 全国の面・共有辺の検証を通したあとでのみ完了にする。
    normalize_levels(&mut db, &api, &status_path)?;
    progress(
        &status_path,
        &serde_json::json!({"state":"verifying","generator":"rust","completedLevels":completed}),
    )?;
    let report = verify(output)?;
    db.execute(
        "INSERT OR REPLACE INTO metadata VALUES ('verification',?1)",
        [report.to_string()],
    )?;
    db.execute(
        "INSERT OR REPLACE INTO metadata VALUES ('qualityPassed',?1)",
        [report["qualityPassed"].to_string()],
    )?;
    db.execute(
        "INSERT OR REPLACE INTO metadata VALUES ('generationComplete','true')",
        [],
    )?;
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    progress(
        &status_path,
        &serde_json::json!({"state":"complete","generator":"rust","verification":report,"secondsThisRun":started.elapsed().as_secs_f64()}),
    )?;
    Ok(())
}
fn normalize_levels(db: &mut Connection, api: &Geos, status: &Path) -> Result<()> {
    for level in (4..=14).rev() {
        let marker = format!("representedNormalization{level}");
        let done: Option<String> = db
            .query_row("SELECT value FROM metadata WHERE key=?1", [&marker], |r| {
                r.get(0)
            })
            .optional()?;
        if done.is_some() {
            continue;
        }
        let coords = {
            let mut statement = db.prepare("SELECT x,y FROM tiles WHERE z=?1 ORDER BY x,y")?;
            statement
                .query_map([level], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut changed = 0;
        for (i, &(x, y)) in coords.iter().enumerate() {
            let body: Vec<u8> = db.query_row(
                "SELECT data FROM tiles WHERE z=?1 AND x=?2 AND y=?3",
                (level, x, y),
                |r| r.get(0),
            )?;
            let before = codec::decode_mesh(&body)?;
            let after = coast::normalize_stored(api, &before)
                .with_context(|| format!("レベル {level} ({x}, {y}) の保存頂点の整理"))?;
            if before.vertices != after.vertices || before.faces != after.faces {
                let tx = db.transaction()?;
                tx.execute(
                    "UPDATE tiles SET data=?1 WHERE z=?2 AND x=?3 AND y=?4",
                    (codec::encode_mesh(&after)?, level, x, y),
                )?;
                tx.execute(
                    "UPDATE metrics SET vertices=?1,triangles=?2 WHERE z=?3 AND x=?4 AND y=?5",
                    (after.vertices.len(), after.faces.len(), level, x, y),
                )?;
                tx.commit()?;
                changed += 1;
            }
            if i % 32 == 0 || i + 1 == coords.len() {
                progress(
                    status,
                    &serde_json::json!({"state":"normalizing","generator":"rust","level":level,"completedInLevel":i+1,"totalInLevel":coords.len(),"repairedMeshes":changed}),
                )?;
            }
        }
        stitch::stitch(db, level)?;
        db.execute(
            "INSERT OR REPLACE INTO metadata VALUES (?1,?2)",
            (
                &marker,
                serde_json::json!({"version":1,"meshes":coords.len(),"repairedMeshes":changed})
                    .to_string(),
            ),
        )?;
    }
    Ok(())
}
pub fn verify(path: &Path) -> Result<serde_json::Value> {
    let db = readonly(path)?;
    let check: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    ensure!(check == "ok", "SQLite の検査に失敗しました");
    let mut total = 0;
    let mut triangles = 0;
    let mut levels = Vec::new();
    let mut shared = 0;
    let mut warnings = BTreeSet::new();
    let mut uncovered = 0usize;
    let mut missing_probes = 0u64;
    for level in 4..=14 {
        let mut profiles = BTreeMap::new();
        let mut statement=db.prepare("SELECT t.x,t.y,t.data,m.height_error FROM tiles t JOIN metrics m USING(z,x,y) WHERE t.z=?1")?;
        let rows = statement.query_map([level], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, Vec<u8>>(2)?,
                r.get::<_, f64>(3)?,
            ))
        })?;
        for row in rows {
            let (x, y, body, error) = row?;
            let mesh = codec::decode_mesh(&body)?;
            mesh.validate_manifold()
                .with_context(|| format!("レベル {level} ({x}, {y}) の接続"))?;
            ensure!(error.is_finite(), "標高誤差が有限ではありません");
            if error > ERRORS[(level - 4) as usize] + 0.01 {
                warnings.insert((level, x, y));
            }
            ensure!(
                mesh.vertices
                    .iter()
                    .all(|p| p[0] >= 0. && p[0] <= 256. && p[1] >= 0. && p[1] <= 256.),
                "頂点が区画の外です"
            );
            for f in &mesh.faces {
                ensure!(
                    crate::mesh::normal(&mesh.vertices, *f)[2] > 0.,
                    "三角形の向きが不正です"
                );
            }
            triangles += mesh.faces.len();
            total += 1;
            profiles.insert((x, y), stitch::profiles(&mesh));
        }
        if profiles.is_empty() {
            continue;
        }
        levels.push(level);
        for (&(x, y), edges) in &profiles {
            for (neighbor, a, b) in [((x + 1, y), 1, 0), ((x, y + 1), 3, 2)] {
                if let Some(other) = profiles.get(&neighbor) {
                    let (a, b) = (&edges[a], &other[b]);
                    if a.is_empty() || b.is_empty() {
                        continue;
                    }
                    let lo = a[0][0].max(b[0][0]);
                    let hi = a.last().unwrap()[0].min(b.last().unwrap()[0]);
                    for p in a.iter().chain(b) {
                        if p[0] >= lo && p[0] <= hi {
                            ensure!(
                                (stitch::interpolate(a, p[0]) - stitch::interpolate(b, p[0])).abs()
                                    <= 0.01,
                                "共有辺の標高が一致しません"
                            );
                        }
                    }
                    shared += 1;
                }
            }
        }
    }
    ensure!(levels.len() == 11, "全レベルの生成が終わっていません");
    let has_quality: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='quality')",
        [],
        |r| r.get(0),
    )?;
    if has_quality {
        let mut statement = db.prepare("SELECT z,x,y,report FROM quality")?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (z, x, y, report) = row?;
            let report: serde_json::Value = serde_json::from_str(&report)?;
            let missing = report["missingProbes"].as_u64().unwrap_or(0);
            if missing > 0 {
                uncovered += 1;
                missing_probes += missing;
            }
            if report["passed"] == false {
                warnings.insert((z, x, y));
            }
        }
    }
    Ok(
        serde_json::json!({"levels":levels,"meshes":total,"triangles":triangles,"sharedBorders":shared,"sqliteCheck":check,"qualityPassed":warnings.is_empty(),"qualityWarningMeshes":warnings.len(),"uncoveredProbeMeshes":uncovered,"missingProbes":missing_probes,"experimental":true}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uncovered_candidate_is_not_replaced() {
        let source = square(0.);
        let reduced = Mesh {
            vertices: source.vertices.clone(),
            faces: vec![source.faces[0]],
        };
        let (result, _, report) = adopt_reduced(&source, &source, reduced, 1., 1.).unwrap();
        assert_eq!(result.faces.len(), 1);
        assert_eq!(report["passed"], false);
        assert!(report["missingProbes"].as_u64().unwrap() > 0);
        assert_eq!(report["fallback"], false);
    }
    #[test]
    fn excessive_error_is_recorded_without_fallback() {
        let source = Mesh {
            vertices: vec![
                [0., 0., 0.],
                [1., 0., 0.],
                [0., 1., 0.],
                [1., 1., 0.],
                [0.5, 0.5, 100.],
            ],
            faces: vec![[0, 1, 4], [1, 3, 4], [3, 2, 4], [2, 0, 4]],
        };
        let reduced = Mesh {
            vertices: source.vertices.clone(),
            faces: vec![[0, 1, 2], [1, 3, 2]],
        };
        let (result, error, report) = adopt_reduced(&source, &source, reduced, 1., 1.).unwrap();
        assert_eq!(result.faces.len(), 2);
        assert_eq!(error, 100.);
        assert_eq!(report["passed"], false);
    }
    #[test]
    fn degenerate_candidate_faces_are_removed_without_fallback() {
        let source = Mesh {
            vertices: vec![[0., 0., 0.], [1., 0., 0.], [2., 0., 0.], [0., 1., 0.]],
            faces: vec![[0, 1, 3], [1, 2, 3]],
        };
        let reduced = Mesh {
            vertices: source.vertices.clone(),
            faces: vec![[0, 1, 2], [0, 2, 3]],
        };
        let (result, _, report) = adopt_reduced(&source, &source, reduced, 1., 1.).unwrap();
        assert_eq!(result.faces.len(), 1);
        assert_eq!(report["removedDegenerateFaces"], 1);
        assert_eq!(report["passed"], true);
    }
    #[test]
    fn rounded_alias_height_is_budgeted() {
        let mut p = part(1, 1, 0.);
        p.mesh = Mesh {
            vertices: vec![
                [179.74998474121094, 227.75001525878906, 0.],
                [179.75, 227.75, 5.],
                [180., 227., 10.],
                [180., 228., 10.],
            ],
            faces: vec![[0, 2, 3], [1, 2, 3]],
        };
        p.height = 10.;
        let (m, _, error, _) = merge(vec![p]).unwrap();
        assert_eq!(error, 15.);
        assert_eq!(m.vertices[0][2], 0.);
        assert_eq!(m.vertices.len(), 3);
    }
    #[test]
    fn large_rounded_alias_preserved() {
        let mut p = part(1, 1, 0.);
        p.mesh.vertices[0] = [179.74998474121094, 227.75001525878906, 0.];
        p.mesh.vertices[1] = [179.75, 227.75, 20.];
        let (mesh, _, error, shift) = merge(vec![p]).unwrap();
        assert_eq!(error, 0.);
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.vertices[0][2], 0.);
        assert_eq!(mesh.vertices[1][2], 20.);
        assert_ne!(mesh.vertices[0][..2], mesh.vertices[1][..2]);
        assert!(shift > 0. && shift < 0.0001);
    }
    #[test]
    fn aliases_stay_on_outer_edge() {
        let rounded = [256., 128.];
        let mut lookup = BTreeMap::new();
        lookup.insert(rounded.map(f64::to_bits), 0);
        let raw = [256., 128. + 0.00000762939453125];
        let xy = separate_alias(raw, rounded, &lookup).unwrap();
        assert_eq!(xy[0], 256.);
        assert_ne!(xy[1], 128.);
    }
    #[test]
    fn ten_meter_alias_uses_midpoint() {
        let mut p = part(1, 1, 0.);
        p.mesh.vertices[0] = [200.5, 210., 10.];
        p.mesh.vertices[1] = [200.50001525878906, 209.99998474121094, 0.];
        p.height = 10.;
        let (mesh, _, error, _) = merge(vec![p]).unwrap();
        assert_eq!(mesh.vertices[0][2], 5.);
        assert_eq!(error, 15.);
    }
    #[test]
    fn completed_checkpoint() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)")
            .unwrap();
        assert!(completed_levels(&db).unwrap().is_empty());
        db.execute(
            "INSERT INTO metadata VALUES ('completedLevels','[14,13]')",
            [],
        )
        .unwrap();
        assert_eq!(completed_levels(&db).unwrap(), vec![14, 13]);
    }
    fn square(z: f64) -> Mesh {
        Mesh {
            vertices: vec![[0., 0., z], [256., 0., z], [0., 256., z], [256., 256., z]],
            faces: vec![[0, 1, 2], [1, 3, 2]],
        }
    }
    fn part(x: u32, y: u32, z: f64) -> Part {
        Part {
            mesh: square(z),
            protected: Vec::new(),
            height: 0.,
            coast: 0.,
            dx: x,
            dy: y,
            factor: 2,
        }
    }
    #[test]
    fn shared_heights() {
        assert!(merge(vec![part(0, 0, 100.), part(1, 0, 110.)]).is_err());
    }
    #[test]
    fn cross_distribution_borders() {
        let (mesh, _, _, _) = merge(vec![
            part(0, 0, 100.),
            part(1, 0, 100.),
            part(0, 1, 100.),
            part(1, 1, 100.),
        ])
        .unwrap();
        assert_eq!(mesh.vertices.len(), 9);
        let mut q = Qem::new(mesh, &[], None).unwrap();
        let reduced = q.reduce(4, 0.).unwrap();
        assert!(reduced.faces.len() < 8);
        assert!(reduced.faces.iter().any(|f| (0..2).any(|axis| {
            let values = f.map(|i| reduced.vertices[i][axis]);
            values.iter().copied().fold(f64::INFINITY, f64::min) < 128.
                && values.iter().copied().fold(f64::NEG_INFINITY, f64::max) > 128.
        })));
    }
    #[test]
    fn coordinate_chain() {
        let mut mesh = square(100.);
        let mut expected = None;
        for level in (4..=14).rev() {
            let factor = 1u32 << (14 - zoom(level));
            let p = Part {
                mesh,
                protected: Vec::new(),
                height: 0.,
                coast: 0.,
                dx: 0,
                dy: 0,
                factor: if level == 14 { 4 } else { 2 },
            };
            mesh = merge(vec![p]).unwrap().0;
            let max = mesh
                .vertices
                .iter()
                .map(|p| p[0] * factor as f64)
                .fold(0., f64::max);
            if let Some(old) = expected {
                assert_eq!(max, old)
            } else {
                expected = Some(max)
            }
            assert_eq!(mesh.vertices[0][2], 100.);
        }
    }
    #[test]
    fn levels_and_limits() {
        assert_eq!(zoom(14), 12);
        assert_eq!(zoom(4), 2);
        for i in 1..11 {
            assert!(ERRORS[i] <= ERRORS[i - 1] && COASTS[i] <= COASTS[i - 1]);
        }
    }
    #[test]
    fn protected_codec() {
        let p = vec![[1., 2., 100.], [4., 5., -10.]];
        assert_eq!(protected_read(&protected_write(&p).unwrap()).unwrap(), p);
    }
}
