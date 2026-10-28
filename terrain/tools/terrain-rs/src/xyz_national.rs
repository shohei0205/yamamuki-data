//! 全国を小さな作業領域で処理し、保存タイルとは別に境界を同期する。
use super::*;
use std::collections::{BTreeSet, HashSet};
use xyz_region_pack::V;
#[path = "xyz_national_pack.rs"]
mod packing;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Config {
    version: u32,
    source_sha256: String,
    source_bytes: u64,
    cells: usize,
    window: Vec<i64>,
    chunks: Vec<[i64; 2]>,
    settings: String,
    mesh_id: String,
}
fn catalog(args: &Args) -> Result<Config> {
    let cells = args.national_chunk_cells;
    ensure!(
        cells.is_power_of_two() && (128..=4096).contains(&cells),
        "作業領域は128～4096の2の累乗を指定してください"
    );
    let w = &args.national_window;
    ensure!(
        w.is_empty() || (w.len() == 4 && w[0] < w[2] && w[1] < w[3]),
        "作業領域の試験範囲が不正です"
    );
    let db = Connection::open_with_flags(&args.source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut query = db.prepare("SELECT x,y FROM tiles WHERE z=14 ORDER BY y,x")?;
    let mut chunks = BTreeSet::new();
    for row in query.query_map([], |r| Ok([r.get::<_, i64>(0)?, r.get::<_, i64>(1)?]))? {
        let [x, y] = row?;
        for cy in y * 256 / cells as i64..=(y * 256 + 255) / cells as i64 {
            for cx in x * 256 / cells as i64..=(x * 256 + 255) / cells as i64 {
                if w.is_empty() || (cx >= w[0] && cy >= w[1] && cx < w[2] && cy < w[3]) {
                    chunks.insert([cx, cy]);
                }
            }
        }
    }
    let mut chunks = chunks.into_iter().collect::<Vec<_>>();
    chunks.sort_by_key(|p| (p[1], p[0]));
    ensure!(!chunks.is_empty(), "対象の元標高がありません");
    let saved = args.output.join("national-config.json");
    let mesh_id = if saved.exists() {
        serde_json::from_slice::<Config>(&fs::read(saved)?)?.mesh_id
    } else {
        format!(
            "national-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        )
    };
    Ok(Config {
        version: 1,
        source_sha256: args.source_sha256.clone(),
        source_bytes: fs::metadata(&args.source)?.len(),
        cells,
        window: w.clone(),
        chunks,
        settings: include_str!("../../terrain/national_mesh_settings.json").into(),
        mesh_id,
    })
}
fn chunk_region(config: &Config, xy: [i64; 2]) -> Region {
    let spacing = web_mercator::width(14) / 256.;
    Region {
        name: format!("chunk-{}-{}", xy[0], xy[1]),
        latitude: 0.,
        longitude: 0.,
        n: config.cells + 1,
        px: xy[0] * config.cells as i64,
        py: xy[1] * config.cells as i64,
        spacing,
        extent: config.cells as f64 * spacing,
        source_sha256: config.source_sha256.clone(),
    }
}
fn chunk_dir(args: &Args, r: &Region) -> PathBuf {
    args.output.join("chunks").join(&r.name)
}
fn load_fields(
    source: &mut Source,
    r: &Region,
    dir: &Path,
    with_weights: bool,
) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>, usize, HashSet<(i64, i64)>)> {
    // 山頂検出・地形指標は作業領域の外まで参照する。DEMそのものは保存しない。
    let pad = (2000. / r.spacing).ceil() as usize;
    let n = r.n + pad * 2;
    let mut wide = Vec::with_capacity(n * n);
    let mut valid = HashSet::new();
    for y in 0..n {
        if y % 64 == 0 {
            progress(
                dir,
                json!({"stage":"read-source","processed":y,"total":n,"complete":false}),
            )?;
        }
        for x in 0..n {
            let gx = r.px + x as i64 - pad as i64;
            let gy = r.py + y as i64 - pad as i64;
            let (v, ok) = source.at(gx, gy)?;
            wide.push(v);
            if ok && x >= pad && y >= pad && x < pad + r.n - 1 && y < pad + r.n - 1 {
                valid.insert((gx.div_euclid(256), gy.div_euclid(256)));
            }
        }
    }
    progress(dir, json!({"stage":"terrain-importance","complete":false}))?;
    let weights = if with_weights {
        decimation_surface::importance(&wide, n, r.spacing)
    } else {
        Vec::new()
    };
    let (scales, peaks) = summit_policy::summit_scales(&wide, n, r.spacing);
    let crop = |field: &[f64]| {
        (0..r.n)
            .flat_map(|y| {
                field[(y + pad) * n + pad..(y + pad) * n + pad + r.n]
                    .iter()
                    .copied()
            })
            .collect::<Vec<_>>()
    };
    Ok((
        crop(&wide),
        if with_weights {
            crop(&weights)
        } else {
            Vec::new()
        },
        crop(&scales),
        peaks,
        valid,
    ))
}
fn register_valid(db: &Connection, valid: &HashSet<(i64, i64)>) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    {
        let mut q = tx.prepare_cached("INSERT OR IGNORE INTO valid_tiles VALUES(?1,?2,?3)")?;
        for &(x, y) in valid {
            for z in 6..=14 {
                let k = 1i64 << (14 - z);
                q.execute(params![z, x / k, y / k])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}
fn boundary(p: [i64; 2], cells: i64) -> bool {
    p[0] == 0 || p[1] == 0 || p[0] == cells || p[1] == cells
}
fn snapped(p: i64, step: i64) -> i64 {
    ((p as f64 / step as f64).round() as i64) * step
}
fn write_shape(
    path: &Path,
    p: &[[i64; 2]],
    f: &[[usize; 3]],
    grid: &[f64],
    n: usize,
) -> Result<()> {
    let tmp = path.with_extension("partial.mesh");
    let mut out = BufWriter::new(
        retry_shared_io(|| File::create(&tmp), std::time::Duration::from_millis(25))
            .with_context(|| format!("確定形状の一時保存: {}", tmp.display()))?,
    );
    out.write_all(b"XRG1")?;
    out.write_all(&(p.len() as u32).to_le_bytes())?;
    out.write_all(&(f.len() as u32).to_le_bytes())?;
    for xy in p {
        out.write_all(&(xy[0] as u32).to_le_bytes())?;
        out.write_all(&(xy[1] as u32).to_le_bytes())?;
        out.write_all(&grid[xy[1] as usize * n + xy[0] as usize].to_le_bytes())?;
    }
    for face in f {
        for &i in face {
            out.write_all(&(i as u32).to_le_bytes())?;
        }
    }
    out.flush()?;
    drop(out);
    retry_shared_io(
        || fs::rename(&tmp, path),
        std::time::Duration::from_millis(25),
    )
    .with_context(|| format!("確定形状の置換: {}", path.display()))?;
    Ok(())
}
fn status(args: &Args, value: serde_json::Value) -> Result<()> {
    progress(&args.output, value)
}
fn check_space(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide = path
            .canonicalize()?
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut available = 0u64;
        unsafe {
            let kernel = libloading::Library::new("kernel32.dll")?;
            let free: libloading::Symbol<
                unsafe extern "system" fn(*const u16, *mut u64, *mut u64, *mut u64) -> i32,
            > = kernel.get(b"GetDiskFreeSpaceExW\0")?;
            ensure!(
                free(
                    wide.as_ptr(),
                    &mut available,
                    std::ptr::null_mut(),
                    std::ptr::null_mut()
                ) != 0,
                "空き容量を取得できません"
            );
        }
        ensure!(
            available >= 1024 * 1024 * 1024,
            "空き容量が1 GiB未満のため保存済み形状を残して停止します"
        );
    }
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}

pub(super) fn run(args: &Args) -> Result<()> {
    // 同じ出力への二重起動をOSのファイル共有モードで防ぐ。異常終了でもロックは解放される。
    #[cfg(windows)]
    let _lock = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(args.output.join("generation.lock"))
            .context("同じ全国出力を別プロセスが使用中です")?
    };
    let result = run_locked(args);
    if let Err(error) = &result {
        let previous = fs::read(args.output.join("progress.json"))
            .ok()
            .and_then(|v| serde_json::from_slice::<serde_json::Value>(&v).ok());
        let _ = status(
            args,
            json!({"stage":"failed","complete":false,"generationStarted":previous.as_ref().and_then(|v|v["generationStarted"].as_bool()).unwrap_or(false),"error":format!("{error:#}"),"previous":previous}),
        );
    }
    result
}
fn run_locked(args: &Args) -> Result<()> {
    check_space(&args.output)?;
    let config = catalog(args)?;
    let config_path = args.output.join("national-config.json");
    if config_path.exists() {
        ensure!(
            serde_json::from_slice::<Config>(&fs::read(&config_path)?)? == config,
            "全国再開設定が保存済み設定と異なります"
        );
    } else {
        fs::write(&config_path, serde_json::to_vec_pretty(&config)?)?;
    }
    let db = Connection::open(args.output.join("national-work.sqlite3"))?;
    db.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL;CREATE TABLE IF NOT EXISTS valid_tiles(z INTEGER,x INTEGER,y INTEGER,PRIMARY KEY(z,x,y)) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS base_done(x INTEGER,y INTEGER,PRIMARY KEY(x,y)) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS boundary(z INTEGER,x INTEGER,y INTEGER,PRIMARY KEY(z,x,y)) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS synchronized(z INTEGER PRIMARY KEY);CREATE TABLE IF NOT EXISTS packed(z INTEGER PRIMARY KEY);")?;
    if args.prepare_only {
        return status(
            args,
            json!({"stage":"catalog-ready","chunks":config.chunks.len(),"complete":false,"generationStarted":false}),
        );
    }
    generate_base_parallel(args, &config, &db)?;
    for z in 6..=14u32 {
        if db
            .query_row("SELECT 1 FROM synchronized WHERE z=?1", [z], |_| Ok(()))
            .optional()?
            .is_none()
        {
            synchronize(args, &config, &db, z)?;
        }
        if db
            .query_row("SELECT 1 FROM packed WHERE z=?1", [z], |_| Ok(()))
            .optional()?
            .is_none()
        {
            pack_level(args, &config, &db, z)?;
        }
    }
    let output = Connection::open(args.output.join("terrain.sqlite3"))?;
    ensure!(
        output.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))? == "ok",
        "最終SQLiteの整合性検査が失敗しました"
    );
    let mut reports = Vec::new();
    for z in 6..=14 {
        let mut passed = true;
        let mut max_ratio = 0f64;
        let mut angle = 180f64;
        for &xy in &config.chunks {
            let r = chunk_region(&config, xy);
            let audit: serde_json::Value = serde_json::from_slice(&fs::read(
                chunk_dir(args, &r).join(format!("final-z{z}.audit.json")),
            )?)?;
            passed &= audit["passed"] == true;
            max_ratio = max_ratio.max(
                audit["maximumLocalErrorRatio"]
                    .as_f64()
                    .context("誤差検証値がありません")?,
            );
            angle = angle.min(
                audit["minimumAngleDegrees"]
                    .as_f64()
                    .context("角度検証値がありません")?,
            );
        }
        ensure!(passed, "量子化検証が完了していません");
        reports.push(json!({"z":z,"quantizedAudit":{"passed":passed,"maximumLocalErrorRatio":max_ratio,"minimumAngleDegrees":angle}}));
    }
    fs::write(
        args.output.join("packing-report.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    output.execute(
        "INSERT OR REPLACE INTO metadata VALUES('complete','true')",
        [],
    )?;
    output.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    status(
        args,
        json!({"stage":"generated","complete":true,"generationStarted":true,"distributable":false,"note":"生成と各タイルの復元照合済み。独立した全国検証・表示確認は別途必要です。"}),
    )
}

enum BaseEvent {
    Started(usize, [i64; 2]),
    Finished(usize, [i64; 2], HashSet<(i64, i64)>),
    Failed(anyhow::Error),
}
fn generate_base_parallel(args: &Args, config: &Config, db: &Connection) -> Result<()> {
    use std::collections::BTreeMap;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    };
    ensure!(
        args.national_workers > 0,
        "作業領域の並列数は1以上を指定してください"
    );
    let completed = db
        .prepare("SELECT x,y FROM base_done")?
        .query_map([], |r| Ok([r.get::<_, i64>(0)?, r.get::<_, i64>(1)?]))?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    let jobs = config
        .chunks
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, xy)| !completed.contains(xy))
        .collect::<Vec<_>>();
    if jobs.is_empty() {
        return Ok(());
    }
    // Windowsで親ディレクトリの同時作成が競合しないよう、作業先を先に用意する。
    for &(_, xy) in &jobs {
        let dir = chunk_dir(args, &chunk_region(config, xy));
        retry_shared_io(
            || fs::create_dir_all(&dir),
            std::time::Duration::from_millis(25),
        )
        .with_context(|| format!("作業領域の作成: {}", dir.display()))?;
    }
    let cpus = std::thread::available_parallelism()
        .map(|v| (v.get() / 2).max(1))
        .unwrap_or(1);
    let workers = args.national_workers.min(cpus).min(jobs.len());
    let next = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let (sender, receiver) = mpsc::sync_channel(workers * 2);
    std::thread::scope(|scope| -> Result<()> {
        let mut handles = Vec::new();
        for _ in 0..workers {
            let sender = sender.clone();
            let jobs = &jobs;
            let next = &next;
            let cancelled = &cancelled;
            handles.push(scope.spawn(move || {
                let result = (|| -> Result<()> {
                    // 接続と元標高キャッシュは各作業スレッド専用。出力DBへの書込は管理側だけ。
                    let mut source =
                        Source::open(&args.source).context("並列作業用の元標高を開く")?;
                    loop {
                        if cancelled.load(Ordering::Relaxed) {
                            break;
                        }
                        let job = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&(index, xy)) = jobs.get(job) else {
                            break;
                        };
                        let r = chunk_region(config, xy);
                        let dir = chunk_dir(args, &r);
                        check_space(&args.output)?;
                        sender
                            .send(BaseEvent::Started(index, xy))
                            .context("進捗管理との接続が切れました")?;
                        let (grid, weights, scales, peaks, valid) =
                            load_fields(&mut source, &r, &dir, true)
                                .with_context(|| format!("元標高と地形指標: {}", r.name))?;
                        generate_fields(&r, &dir, grid, weights, scales, peaks)
                            .with_context(|| format!("基本形状の生成と保存: {}", r.name))?;
                        sender
                            .send(BaseEvent::Finished(index, xy, valid))
                            .context("保存管理との接続が切れました")?;
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    cancelled.store(true, Ordering::Relaxed);
                    let _ = sender.send(BaseEvent::Failed(error));
                }
            }));
        }
        drop(sender);
        let mut active = BTreeMap::<usize, [i64; 2]>::new();
        let mut count = completed.len();
        let mut error = None;
        for event in receiver {
            let event_result = (|| -> Result<()> {
                match event {
                    BaseEvent::Started(index, xy) => {
                        active.insert(index, xy);
                    }
                    BaseEvent::Finished(index, xy, valid) => {
                        register_valid(db, &valid)?;
                        db.execute("INSERT INTO base_done VALUES(?1,?2)", params![xy[0], xy[1]])?;
                        active.remove(&index);
                        count += 1;
                    }
                    BaseEvent::Failed(e) => {
                        return Err(e);
                    }
                }
                let active=active.iter().map(|(&index,&xy)|{let r=chunk_region(config,xy);json!({"index":index,"chunk":r.name,"childProgress":chunk_dir(args,&r).join("progress.json")})}).collect::<Vec<_>>();
                status(
                    args,
                    json!({"stage":"generate-base","parallelWorkers":workers,"activeChunks":active,"chunkIndex":count,"completedChunks":count,"totalChunks":config.chunks.len(),"generationStarted":true,"complete":false}),
                )
            })();
            if let Err(e) = event_result {
                cancelled.store(true, Ordering::Relaxed);
                if error.is_none() {
                    error = Some(e);
                }
            }
        }
        // 失敗時も通知を受け取り続け、保存中の別領域を途中で打ち切らない。
        for handle in handles {
            if handle.join().is_err() && error.is_none() {
                error = Some(anyhow::anyhow!("作業領域の生成スレッドが異常終了しました"));
            }
        }
        if let Some(error) = error {
            return Err(error);
        }
        ensure!(
            count == config.chunks.len(),
            "基本メッシュの完了数が一致しません"
        );
        Ok(())
    })
}

fn synchronize(args: &Args, config: &Config, db: &Connection, z: u32) -> Result<()> {
    let k = 1i64 << (14 - z);
    let step = (k / 64).max(1);
    // 全領域の基本境界を先に登録する。後続の補修で増えた境界も収束まで共有する。
    let tx = db.unchecked_transaction()?;
    for &xy in &config.chunks {
        let r = chunk_region(config, xy);
        let dir = chunk_dir(args, &r);
        let (p, _, _) = read_level(&dir.join(format!("z{z}.mesh")))?;
        for corner in [
            [0, 0],
            [config.cells as i64, 0],
            [0, config.cells as i64],
            [config.cells as i64, config.cells as i64],
        ] {
            ensure!(
                p.contains(&corner),
                "並列境界同期に必要な固定隅点がありません: {}",
                r.name
            );
        }
        for p in p {
            if boundary(p, config.cells as i64) {
                tx.execute(
                    "INSERT OR IGNORE INTO boundary VALUES(?1,?2,?3)",
                    params![z, r.px + snapped(p[0], step), r.py + snapped(p[1], step)],
                )?;
            }
        }
    }
    tx.commit()?;
    ensure!(
        args.national_boundary_workers > 0,
        "境界同期の並列数は1以上を指定してください"
    );
    let cpus = std::thread::available_parallelism()
        .map(|v| (v.get() / 2).max(1))
        .unwrap_or(1);
    let workers = args
        .national_boundary_workers
        .min(cpus)
        .min(config.chunks.len());
    let batches = sync_batches(&config.chunks, workers);
    let mut pass = 0;
    loop {
        pass += 1;
        let mut added = 0;
        let mut processed = 0;
        for batch in &batches {
            check_space(&args.output)?;
            let mut tasks = Vec::new();
            for &(index, xy) in batch {
                let r = chunk_region(config, xy);
                let mut query=db.prepare("SELECT x,y FROM boundary WHERE z=?1 AND x>=?2 AND y>=?3 AND x<=?4 AND y<=?5 ORDER BY y,x")?;
                let shared = query
                    .query_map(
                        params![
                            z,
                            r.px,
                            r.py,
                            r.px + config.cells as i64,
                            r.py + config.cells as i64
                        ],
                        |row| Ok([row.get::<_, i64>(0)?, row.get::<_, i64>(1)?]),
                    )?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                tasks.push((index, xy, shared));
            }
            let active=tasks.iter().map(|&(index,xy,_)|{let r=chunk_region(config,xy);json!({"index":index,"chunk":r.name,"childProgress":chunk_dir(args,&r).join("progress.json")})}).collect::<Vec<_>>();
            status(
                args,
                json!({"stage":"synchronize-boundaries","parallelWorkers":workers,"activeChunks":active,"z":z,"pass":pass,"chunkIndex":processed,"totalChunks":config.chunks.len(),"generationStarted":true,"complete":false}),
            )?;
            // 同じ波面では共有辺がなく、隅点は最初から全領域に固定されている。
            // 境界登録はバッチ全体の計算後に管理側で行う。
            let results = std::thread::scope(|scope| {
                let handles = tasks
                    .into_iter()
                    .map(|(_, xy, shared)| {
                        scope.spawn(move || {
                            synchronize_chunk(args, config, z, step, xy, shared)
                                .with_context(|| format!("境界同期 z{z}: {xy:?}"))
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join().unwrap_or_else(|_| {
                            Err(anyhow::anyhow!("境界同期スレッドが異常終了しました"))
                        })
                    })
                    .collect::<Vec<_>>()
            });
            for result in results {
                let points = result?;
                let tx = db.unchecked_transaction()?;
                for p in points {
                    added += tx.execute(
                        "INSERT OR IGNORE INTO boundary VALUES(?1,?2,?3)",
                        params![z, p[0], p[1]],
                    )?;
                }
                tx.commit()?;
                processed += 1;
            }
        }
        if added == 0 {
            break;
        }
    }
    db.execute("INSERT INTO synchronized VALUES(?1)", [z])?;
    Ok(())
}
// 1並列は比較用に従来の行順を維持する。複数並列は北・西からの依存を波面で守る。
fn sync_batches(chunks: &[[i64; 2]], workers: usize) -> Vec<Vec<(usize, [i64; 2])>> {
    if workers == 1 {
        return chunks
            .iter()
            .copied()
            .enumerate()
            .map(|p| vec![p])
            .collect();
    }
    let mut waves = std::collections::BTreeMap::<i64, Vec<(usize, [i64; 2])>>::new();
    for (i, &xy) in chunks.iter().enumerate() {
        waves.entry(xy[0] + xy[1]).or_default().push((i, xy));
    }
    waves
        .values()
        .flat_map(|wave| wave.chunks(workers).map(|part| part.to_vec()))
        .collect()
}
fn synchronize_chunk(
    args: &Args,
    config: &Config,
    z: u32,
    step: i64,
    xy: [i64; 2],
    shared: Vec<[i64; 2]>,
) -> Result<Vec<[i64; 2]>> {
    let k = 1i64 << (14 - z);
    let r = chunk_region(config, xy);
    let dir = chunk_dir(args, &r);
    let target = dir.join(format!("final-z{z}.mesh"));
    let mut source = Source::open(&args.source)?;
    let (grid, _, scales, _, _) = load_fields(&mut source, &r, &dir, false)?;
    progress(
        &dir,
        json!({"stage":"quantization-repair","z":z,"complete":false}),
    )?;
    let (base, _, _) = read_level(&dir.join(format!("z{z}.mesh")))?;
    let mut seeds = base
        .into_iter()
        .map(|p| [snapped(p[0], step) / step, snapped(p[1], step) / step])
        .collect::<BTreeSet<_>>();
    for file in [
        if z > 6 {
            Some(dir.join(format!("final-z{}.mesh", z - 1)))
        } else {
            None
        },
        if target.exists() {
            Some(target.clone())
        } else {
            None
        },
    ]
    .into_iter()
    .flatten()
    {
        for p in read_level(&file)?.0 {
            ensure!(
                p[0] % step == 0 && p[1] % step == 0,
                "継承頂点の量子化格子が不一致"
            );
            seeds.insert([p[0] / step, p[1] / step]);
        }
    }
    for p in shared {
        seeds.insert([(p[0] - r.px) / step, (p[1] - r.py) / step]);
    }
    let n = (r.n - 1) / step as usize + 1;
    let down = |v: &[f64]| {
        (0..n)
            .flat_map(|y| (0..n).map(move |x| v[y * step as usize * r.n + x * step as usize]))
            .collect::<Vec<_>>()
    };
    let low = down(&grid);
    let low_scales = down(&scales);
    let tolerance = 12.5 * k as f64;
    let mut rounds = 0;
    let (points, faces) = loop {
        rounds += 1;
        ensure!(rounds <= 100, "面の幅補修が収束しません");
        let fixed = adaptive_tin::quality_tin::refine_seeded_with_limits(
            &low,
            n,
            seeds.iter().copied().collect(),
            tolerance - 0.125,
            10.,
            || false,
            if z == 14 { None } else { Some(&low_scales) },
        )?;
        let mut extra = BTreeSet::new();
        for face in &fixed.faces {
            for j in 0..3 {
                let a = fixed.points[face[j]];
                let b = fixed.points[face[(j + 1) % 3]];
                if (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) * step
                    > (256. * k as f64 * 0.49).floor() as i64
                {
                    extra.insert([(a[0] + b[0]) / 2, (a[1] + b[1]) / 2]);
                }
            }
        }
        if extra.is_empty() {
            break (
                fixed
                    .points
                    .into_iter()
                    .map(|p| [p[0] * step, p[1] * step])
                    .collect::<Vec<_>>(),
                fixed.faces,
            );
        }
        seeds.extend(fixed.points);
        let old = seeds.len();
        seeds.extend(extra);
        ensure!(seeds.len() > old, "面の幅補修に必要な格子点がありません");
    };
    let vertices = points
        .iter()
        .map(|p| {
            let h = grid[p[1] as usize * r.n + p[0] as usize];
            ensure!((-12000. ..=10383.75).contains(&h), "標高範囲外");
            Ok(V {
                x: (r.px + p[0]) * 64,
                y: (r.py + p[1]) * 64,
                z: (24000. + h / if h < 0. { 0.5 } else { 0.25 }).round() as u16,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    progress(
        &dir,
        json!({"stage":"quantized-audit","z":z,"complete":false}),
    )?;
    let audit = xyz_region_pack::quantized_audit(&r, &vertices, &faces, &grid, &scales, z);
    ensure!(
        audit["passed"] == true,
        "全国量子化検証失敗 {} z{}: {}",
        r.name,
        z,
        audit
    );
    write_shape(&target, &points, &faces, &grid, r.n)?;
    let audit_path = dir.join(format!("final-z{z}.audit.json"));
    let audit_bytes = serde_json::to_vec_pretty(&audit)?;
    retry_shared_io(
        || fs::write(&audit_path, &audit_bytes),
        std::time::Duration::from_millis(25),
    )
    .with_context(|| format!("量子化検証結果の保存: {}", audit_path.display()))?;

    Ok(points
        .into_iter()
        .filter(|&p| boundary(p, config.cells as i64))
        .map(|p| [r.px + p[0], r.py + p[1]])
        .collect())
}

// 最終保存の索引はSQLiteに置き、全国の面や親頂点を一括でメモリーへ載せない。
fn pack_level(args: &Args, config: &Config, work: &Connection, z: u32) -> Result<()> {
    packing::pack_level(args, config, work, z)
}

#[cfg(test)]
mod parallel_sync_tests {
    use super::*;
    #[test]
    fn waves_keep_north_west_dependencies_and_do_not_share_edges() {
        let chunks = (0..5)
            .flat_map(|y| {
                (0..6).filter_map(move |x| if (x + y) % 4 == 0 { None } else { Some([x, y]) })
            })
            .collect::<Vec<_>>();
        let batches = sync_batches(&chunks, 4);
        let mut where_at = HashMap::new();
        for (batch, jobs) in batches.iter().enumerate() {
            assert!(jobs.len() <= 4);
            for &(index, xy) in jobs {
                assert_eq!(chunks[index], xy);
                assert!(where_at.insert(xy, batch).is_none());
                for &(_, other) in jobs {
                    assert_ne!((xy[0] - other[0]).abs() + (xy[1] - other[1]).abs(), 1);
                }
            }
        }
        assert_eq!(where_at.len(), chunks.len());
        for &xy in &chunks {
            for previous in [[xy[0] - 1, xy[1]], [xy[0], xy[1] - 1]] {
                if let Some(p) = where_at.get(&previous) {
                    assert!(*p < where_at[&xy]);
                }
            }
        }
    }
    #[test]
    fn one_worker_keeps_the_original_row_order() {
        let chunks = vec![[3, 1], [4, 1], [1, 2], [2, 2], [4, 2]];
        assert_eq!(
            sync_batches(&chunks, 1)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            chunks.into_iter().enumerate().collect::<Vec<_>>()
        );
    }
}
