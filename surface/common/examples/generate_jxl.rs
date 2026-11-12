//! 全国地表をdistance 4の独立／段階式JPEG XLへ変換する比較用生成器。
#[cfg(windows)]
#[path = "compare_jxl.rs"]
#[allow(dead_code)]
mod comparison;
#[cfg(not(windows))]
fn main() {
    eprintln!("この実験はWindows版libjxlを使用します");
}
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    national::run()
}
#[cfg(windows)]
mod national {
    use super::comparison::experiment::{Api, metrics};
    use anyhow::{Context, Result, ensure};
    use image::{
        GrayImage,
        imageops::{self, FilterType},
    };
    use rusqlite::{Connection, OpenFlags, params};
    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};
    use std::{
        collections::{BTreeMap, VecDeque},
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::{Path, PathBuf},
        process::Command,
        sync::{Arc, Mutex, OnceLock, mpsc},
        time::{Duration, Instant},
    };
    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct Tile {
        dataset: i64,
        layer: i64,
        z: u32,
        x: u32,
        y: u32,
        side: u32,
    }
    #[derive(Clone, Debug)]
    struct Job {
        id: i64,
        anchor: Tile,
        finest: u32,
        tiles: Vec<Tile>,
    }
    #[derive(Clone)]
    struct Settings {
        source: PathBuf,
        raw: PathBuf,
        bin: PathBuf,
        out: PathBuf,
        jobs: usize,
        independent: bool,
    }
    struct Finished {
        job: Job,
        payload: Vec<u8>,
        stages: Vec<Value>,
        hash: String,
    }
    fn plan_independent(tiles: Vec<Tile>) -> Vec<Job> {
        tiles
            .into_iter()
            .enumerate()
            .map(|(i, t)| Job {
                id: i as i64 + 1,
                finest: t.z,
                anchor: t.clone(),
                tiles: vec![t],
            })
            .collect()
    }
    fn plan(tiles: Vec<Tile>) -> Vec<Job> {
        let mut buckets = BTreeMap::<(i64, i64, u32, u32, u32), Vec<Tile>>::new();
        for t in tiles {
            let z = t.z.min(10);
            let shift = t.z - z;
            buckets
                .entry((t.dataset, t.layer, z, t.x >> shift, t.y >> shift))
                .or_default()
                .push(t);
        }
        let mut jobs = Vec::new();
        for (_, mut tiles) in buckets {
            tiles.sort_by_key(|t| (t.z, t.x, t.y));
            let parent = &tiles[0];
            let complete = parent.z == 10
                && tiles.len() == 21
                && (10..=12).all(|z| {
                    let n = 1 << (z - 10);
                    (0..n).all(|dy| {
                        (0..n).all(|dx| {
                            tiles.iter().any(|t| {
                                t.z == z
                                    && t.x == parent.x * n + dx
                                    && t.y == parent.y * n + dy
                                    && t.side == 384
                            })
                        })
                    })
                });
            if complete {
                jobs.push(Job {
                    id: jobs.len() as i64 + 1,
                    anchor: parent.clone(),
                    finest: 12,
                    tiles,
                });
            } else {
                for t in tiles {
                    jobs.push(Job {
                        id: jobs.len() as i64 + 1,
                        anchor: t.clone(),
                        finest: t.z,
                        tiles: vec![t],
                    });
                }
            }
        }
        jobs
    }
    fn fingerprint(bytes: &[u8]) -> String {
        let mut hash = 0xcbf29ce484222325u64;
        for b in bytes {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{hash:016x}")
    }
    fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
        let temp = path.with_extension("tmp");
        fs::write(&temp, bytes)?;
        for retry in 0..20 {
            match fs::rename(&temp, path) {
                Ok(()) => return Ok(()),
                Err(e) if retry < 19 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(e) => return Err(e.into()),
            }
        }
        unreachable!()
    }
    fn read_tile(db: &Connection, t: &Tile, layer: &str) -> Result<GrayImage> {
        let table = match layer {
            "forest" => "tiles",
            "water" => "water_tiles",
            "hillshade" => "hillshade_tiles",
            _ => anyhow::bail!("未知のレイヤー"),
        };
        let bytes: Vec<u8> = db
            .query_row(
                &format!("SELECT payload FROM {table} WHERE z=? AND x=? AND y=?"),
                params![t.z + 1, t.x, t.y],
                |r| r.get(0),
            )
            .with_context(|| format!("元PNGなし: {layer} {}/{}/{}", t.z, t.x, t.y))?;
        ensure!(
            bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "圧縮前PNGを指定してください"
        );
        let img = image::load_from_memory(&bytes)?.to_luma8();
        ensure!(
            img.dimensions() == (t.side, t.side),
            "元PNGの寸法が一致しません"
        );
        Ok(img)
    }
    fn reference(db: &Connection, job: &Job, layer: &str, z: u32) -> Result<GrayImage> {
        let n = 1 << (z - job.anchor.z);
        let mut image = GrayImage::new(job.anchor.side * n, job.anchor.side * n);
        for t in job.tiles.iter().filter(|t| t.z == z) {
            let img = read_tile(db, t, layer)?;
            imageops::replace(
                &mut image,
                &img,
                ((t.x - job.anchor.x * n) * t.side) as i64,
                ((t.y - job.anchor.y * n) * t.side) as i64,
            );
        }
        Ok(image)
    }
    fn convert(
        db: &Connection,
        api: &Api,
        settings: &Settings,
        job: Job,
        layer: &str,
        scratch: &Path,
    ) -> Result<Finished> {
        let original = reference(db, &job, layer, job.finest)?;
        let side = original.width();
        let input = scratch.join("input.png");
        let encoded = scratch.join("output.jxl");
        // 単色タイルは同じ符号列になるため、同じ寸法・値の再圧縮を省く。
        static CONSTANTS: OnceLock<Mutex<BTreeMap<(u32, u8), Vec<u8>>>> = OnceLock::new();
        let constant = original
            .as_raw()
            .first()
            .copied()
            .filter(|&v| original.as_raw().iter().all(|&p| p == v));
        let cached = constant.and_then(|v| {
            CONSTANTS
                .get_or_init(Default::default)
                .lock()
                .unwrap()
                .get(&(side, v))
                .cloned()
        });
        let payload = if let Some(payload) = cached {
            payload
        } else {
            original.save(&input)?;

            if encoded.exists() {
                fs::remove_file(&encoded)?;
            }
            let mut command = Command::new(settings.bin.join("cjxl.exe"));
            command.arg(&input).arg(&encoded).args([
                "-d",
                "4",
                "-e",
                "7",
                "--num_threads=1",
                "--modular=0",
            ]);
            if !settings.independent {
                command.arg("-p");
            }
            let status = command.output()?;
            ensure!(
                status.status.success(),
                "圧縮失敗: {}",
                String::from_utf8_lossy(&status.stderr)
            );
            let payload = fs::read(&encoded)?;
            if let Some(v) = constant {
                CONSTANTS
                    .get_or_init(Default::default)
                    .lock()
                    .unwrap()
                    .insert((side, v), payload.clone());
            }
            payload
        };
        let mut stages = Vec::new();
        for z in job.anchor.z..=job.finest {
            let ratio = 1 << (job.finest - z);
            // 中間段階の通知を得られない画像は、原寸復号へ戻して範囲を保持する。
            let mut decoded = match api.decode(&payload, side, ratio as usize) {
                Ok(decoded) => decoded,
                Err(_) if ratio > 1 => api
                    .decode(&payload, side, 1)
                    .with_context(|| format!("原寸への復号: {layer} z{z} group{}", job.id))?,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("圧縮後の復号: {layer} z{z} group{}", job.id));
                }
            };
            ensure!(
                decoded.bytes > 0
                    && decoded.bytes <= payload.len()
                    && decoded.ratio <= ratio as usize,
                "段階復号の情報が不正です"
            );
            let replay_ratio = if decoded.ratio == 1 {
                1
            } else {
                ratio as usize
            };
            let replay = api.decode(&payload[..decoded.bytes], side, replay_ratio);
            if !replay
                .as_ref()
                .is_ok_and(|prefix| prefix.image == decoded.image)
            {
                decoded = api.decode(&payload, side, 1).with_context(|| {
                    format!("途中入力不一致からの原寸復号: {layer} z{z} group{}", job.id)
                })?;
                let prefix = api.decode(&payload[..decoded.bytes], side, 1)?;
                ensure!(
                    prefix.image == decoded.image,
                    "原寸入力の再検査が一致しません"
                );
            }
            let reference = if z == job.finest {
                original.clone()
            } else {
                reference(db, &job, layer, z)?
            };
            let display = imageops::resize(
                &decoded.image,
                reference.width(),
                reference.height(),
                FilterType::Triangle,
            );
            stages.push(json!({"z":z,"ratio":ratio,"actual_ratio":decoded.ratio,"prefix":decoded.bytes,"pixels":reference.width()as usize*reference.height()as usize,"peak":decoded.peak,"error":metrics(&reference,&display)}));
        }
        let hash = fingerprint(&payload);
        Ok(Finished {
            job,
            payload,
            stages,
            hash,
        })
    }
    fn regular(path: &Path) -> Result<PathBuf> {
        let p = fs::canonicalize(path)?;
        let s = p.to_string_lossy();
        Ok(PathBuf::from(s.strip_prefix("\\\\?\\").unwrap_or(&s)))
    }
    fn config(settings: &Settings) -> Result<Value> {
        let record = |p: &Path| -> Result<Value> {
            let meta = fs::metadata(p)?;
            Ok(
                json!({"path":p,"bytes":meta.len(),"modified":meta.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_secs()}),
            )
        };
        let v = Command::new(settings.bin.join("cjxl.exe"))
            .arg("--version")
            .output()?;
        ensure!(v.status.success(), "圧縮器が使えません");
        Ok(
            json!({"format":if settings.independent {"surface-jxl-independent-experiment-v1"} else {"surface-jxl-progressive-experiment-v1"},"independent":settings.independent,"input":record(&settings.source)?,"raw":record(&settings.raw)?,"encoder":format!("{}{}",String::from_utf8_lossy(&v.stdout),String::from_utf8_lossy(&v.stderr)),"distance":4,"mode":"VarDCT","effort":7,"threads_per_group":1,"anchor_z":10}),
        )
    }
    fn initialize(db: &Connection, source: &Path) -> Result<()> {
        db.execute_batch("PRAGMA page_size=4096; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA application_id=1498237528; PRAGMA user_version=1; PRAGMA foreign_keys=ON;")?;
        let uri = format!(
            "file:{}?mode=ro",
            source
                .to_string_lossy()
                .replace('\\', "/")
                .replace('%', "%25")
                .replace('#', "%23")
                .replace('?', "%3F")
        );
        db.execute("ATTACH ? AS source", [uri])?;
        db.execute_batch(r#"
        CREATE TABLE datasets AS SELECT * FROM source.datasets;
        CREATE UNIQUE INDEX dataset_pk ON datasets(dataset_key);
        CREATE TABLE surface_layers AS SELECT * FROM source.surface_layers;
        CREATE UNIQUE INDEX layer_pk ON surface_layers(dataset_key,layer_key);
        CREATE TABLE surface_levels AS SELECT * FROM source.surface_levels;
        CREATE UNIQUE INDEX level_pk ON surface_levels(dataset_key,layer_key,z);
        CREATE TABLE experiment_info(key TEXT PRIMARY KEY,value TEXT NOT NULL CHECK(json_valid(value))) WITHOUT ROWID,STRICT;
        CREATE TABLE jxl_groups(group_id INTEGER PRIMARY KEY,dataset_key INTEGER NOT NULL,layer_key INTEGER NOT NULL,z INTEGER NOT NULL,x INTEGER NOT NULL,y INTEGER NOT NULL,finest_z INTEGER NOT NULL,pixel_size INTEGER NOT NULL,distance REAL NOT NULL CHECK(distance=4),mode TEXT NOT NULL CHECK(mode='VarDCT'),source_tiles INTEGER NOT NULL,payload_hash TEXT NOT NULL,payload BLOB NOT NULL,UNIQUE(dataset_key,layer_key,z,x,y)) STRICT;
        CREATE TABLE surface_tiles(dataset_key INTEGER NOT NULL,layer_key INTEGER NOT NULL,z INTEGER NOT NULL,x INTEGER NOT NULL,y INTEGER NOT NULL,group_id INTEGER NOT NULL REFERENCES jxl_groups,downsample INTEGER NOT NULL,actual_ratio INTEGER NOT NULL,prefix_bytes INTEGER NOT NULL,left_px INTEGER NOT NULL,top_px INTEGER NOT NULL,tile_pixels INTEGER NOT NULL,PRIMARY KEY(dataset_key,layer_key,z,x,y),FOREIGN KEY(dataset_key,layer_key,z) REFERENCES surface_levels(dataset_key,layer_key,z)) WITHOUT ROWID,STRICT;
        CREATE TABLE jxl_stage_errors(group_id INTEGER NOT NULL REFERENCES jxl_groups,z INTEGER NOT NULL,rmse REAL NOT NULL,mae REAL NOT NULL,max_error INTEGER NOT NULL,pixels INTEGER NOT NULL,peak_decoder_bytes INTEGER NOT NULL,PRIMARY KEY(group_id,z)) WITHOUT ROWID,STRICT;
        DETACH source;
    "#)?;
        Ok(())
    }
    fn save(db: &Connection, done: &Finished) -> Result<()> {
        let j = &done.job;
        let a = &j.anchor;
        let side = a.side * (1 << (j.finest - a.z));
        db.execute(
            "INSERT INTO jxl_groups VALUES(?,?,?,?,?,?,?,?,4,'VarDCT',?,?,?)",
            params![
                j.id,
                a.dataset,
                a.layer,
                a.z,
                a.x,
                a.y,
                j.finest,
                side,
                j.tiles.len(),
                done.hash,
                done.payload
            ],
        )?;
        for stage in &done.stages {
            let z = stage["z"].as_u64().unwrap() as u32;
            let n = 1 << (z - a.z);
            let ratio = stage["ratio"].as_u64().unwrap();
            for t in j.tiles.iter().filter(|t| t.z == z) {
                db.execute(
                    "INSERT INTO surface_tiles VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                    params![
                        t.dataset,
                        t.layer,
                        t.z,
                        t.x,
                        t.y,
                        j.id,
                        ratio,
                        stage["actual_ratio"].as_u64(),
                        stage["prefix"].as_u64(),
                        (t.x - a.x * n) * t.side,
                        (t.y - a.y * n) * t.side,
                        t.side
                    ],
                )?;
            }
            db.execute(
                "INSERT INTO jxl_stage_errors VALUES(?,?,?,?,?,?,?)",
                params![
                    j.id,
                    z,
                    stage["error"]["rmse"].as_f64(),
                    stage["error"]["mae"].as_f64(),
                    stage["error"]["max"].as_u64(),
                    stage["pixels"].as_u64(),
                    stage["peak"].as_u64()
                ],
            )?;
        }
        Ok(())
    }
    fn statistics(db: &Connection, source: &Connection) -> Result<Value> {
        let mut layers = Vec::new();
        let mut query = db.prepare("SELECT layer_key,id FROM surface_layers ORDER BY layer_key")?;
        for row in query.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (key, id) = row?;
            let (bytes,groups):(i64,i64)=db.query_row("SELECT coalesce(sum(length(payload)),0),count(*) FROM jxl_groups WHERE layer_key=?",[key],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let baseline: i64 = source.query_row(
                "SELECT sum(length(payload)) FROM surface_tiles WHERE layer_key=?",
                [key],
                |r| r.get(0),
            )?;
            let mut levels = Vec::new();
            for z in 5..=12 {
                let count: i64 = db.query_row(
                    "SELECT count(*) FROM surface_tiles WHERE layer_key=? AND z=?",
                    params![key, z],
                    |r| r.get(0),
                )?;
                let (expected,baseline_bytes):(i64,i64)=source.query_row("SELECT count(*),coalesce(sum(length(payload)),0) FROM surface_tiles WHERE layer_key=? AND z=?",params![key,z],|r|Ok((r.get(0)?,r.get(1)?)))?;
                ensure!(
                    count == expected,
                    "タイル数不一致: {id} z{z} {count}/{expected}"
                );
                let prefix:i64=db.query_row("SELECT coalesce(sum(prefix_bytes),0) FROM (SELECT group_id,max(prefix_bytes) prefix_bytes FROM surface_tiles WHERE layer_key=? AND z=? GROUP BY group_id)",params![key,z],|r|r.get(0))?;
                let error:f64=db.query_row("SELECT coalesce(sum(e.rmse*e.rmse*e.pixels)/sum(e.pixels),0) FROM jxl_stage_errors e JOIN jxl_groups g USING(group_id) WHERE g.layer_key=? AND e.z=?",params![key,z],|r|r.get(0))?;
                let fallback:i64=db.query_row("SELECT count(DISTINCT group_id) FROM surface_tiles WHERE layer_key=? AND z=? AND downsample>1 AND actual_ratio=1",params![key,z],|r|r.get(0))?;
                levels.push(json!({"z":z,"tiles":count,"baseline_bytes":baseline_bytes,"unique_group_prefix_bytes":prefix,"full_decode_groups":fallback,"rmse":error.sqrt()}));
            }
            layers.push(json!({"id":id,"groups":groups,"payload_bytes":bytes,"baseline_payload_bytes":baseline,"levels":levels}));
        }
        Ok(json!({"layers":layers}))
    }
    fn html() -> &'static str {
        r#"<!doctype html><html lang=ja><meta charset=utf-8><title>全国JPEG XL生成</title><style>body{font:16px system-ui;max-width:1000px;margin:32px auto;background:#f3f5f4;color:#20352e}section{background:white;padding:24px;margin:16px 0}progress{width:100%;height:24px}pre{white-space:pre-wrap}</style><h1>全国JPEG XL生成・distance 4</h1><p>森林・水域・陰影／全レベルz5〜z12／VarDCT／z10〜z12は段階式で共有</p><section><h2 id=phase>確認中</h2><progress id=bar></progress><p id=counts></p><p id=bytes></p><p id=time></p><pre id=failures></pre></section><section><h2>出力</h2><p>比較用SQLite。現行配布版は変更せず、通常ビューワが誤読しない専用application_idで保存。</p><pre id=output></pre></section><script>async function update(){try{const p=await(await fetch('progress.json',{cache:'no-store'})).json();phase.textContent=p.phase;bar.max=p.total||1;bar.value=p.done;counts.textContent=`グループ ${p.done} / ${p.total}・論理タイル ${p.tiles}・失敗 ${p.failures.length}`;bytes.textContent=`圧縮画像 ${(p.payload_bytes/1048576).toFixed(1)} MiB`;time.textContent=`経過 ${(p.elapsed_seconds/60).toFixed(1)} 分　残り目安 ${p.eta_seconds==null?'計測中':(p.eta_seconds/60).toFixed(1)+' 分'}`;failures.textContent=p.failures.map(x=>JSON.stringify(x)).join('\n');output.textContent=p.output;}catch(e){phase.textContent='進捗を確認中: '+e.message}}update();setInterval(update,2000)</script></html>"#
    }
    fn serve(root: &Path, port: u16) -> Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        for stream in listener.incoming() {
            let mut stream = stream?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut data = [0; 4096];
            let n = stream.read(&mut data)?;
            let request = String::from_utf8_lossy(&data[..n]);
            let path = request
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .split('?')
                .next()
                .unwrap_or("/");
            let file = match path {
                "/" => Some(("progress.html", "text/html; charset=utf-8")),
                "/progress.json" => Some(("progress.json", "application/json; charset=utf-8")),
                "/summary.json" => Some(("summary.json", "application/json; charset=utf-8")),
                _ => None,
            };
            let (status, body, kind) = match file
                .and_then(|(name, kind)| fs::read(root.join(name)).ok().map(|b| (b, kind)))
            {
                Some((b, k)) => ("200 OK", b, k),
                None => ("404 Not Found", b"Not Found".to_vec(), "text/plain"),
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream
                .write_all(head.as_bytes())
                .and_then(|_| stream.write_all(&body));
        }
        Ok(())
    }
    fn extract(args: &[std::ffi::OsString]) -> Result<()> {
        ensure!(
            args.len() == 8,
            "--extract SQLite libjxlのbin 出力PNG レイヤー z x y"
        );
        let db = Connection::open_with_flags(&args[1], OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        ensure!(
            db.query_row("PRAGMA application_id", [], |r| r.get::<_, i64>(0))? == 1498237528,
            "比較用JPEG XLのSQLiteではありません"
        );
        let z: u32 = args[5].to_string_lossy().parse()?;
        let x: u32 = args[6].to_string_lossy().parse()?;
        let y: u32 = args[7].to_string_lossy().parse()?;
        let (bytes,side,downsample,actual,prefix,left,top,pixels):(Vec<u8>,u32,u32,usize,usize,u32,u32,u32)=db.query_row("SELECT g.payload,g.pixel_size,t.downsample,t.actual_ratio,t.prefix_bytes,t.left_px,t.top_px,t.tile_pixels FROM surface_tiles t JOIN jxl_groups g USING(group_id) JOIN surface_layers l USING(dataset_key,layer_key) WHERE l.id=? AND t.z=? AND t.x=? AND t.y=?",params![args[4].to_string_lossy(),z,x,y],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)))?;
        ensure!(
            prefix > 0 && prefix <= bytes.len() && downsample > 0,
            "画像の参照が不正です"
        );
        let api = Api::load(&Path::new(&args[2]).join("jxl.dll"))?;
        let decoded = api.decode(&bytes[..prefix], side, actual)?;
        let display = imageops::resize(
            &decoded.image,
            side / downsample,
            side / downsample,
            FilterType::Triangle,
        );
        ensure!(
            left + pixels <= display.width() && top + pixels <= display.height(),
            "切り出し範囲が不正です"
        );
        ensure!(
            !Path::new(&args[3]).exists(),
            "出力PNGは新しいファイルを指定してください"
        );
        imageops::crop_imm(&display, left, top, pixels, pixels)
            .to_image()
            .save(&args[3])?;
        Ok(())
    }
    pub fn run() -> Result<()> {
        let mut args: Vec<_> = std::env::args_os().skip(1).collect();
        let independent = args.first().is_some_and(|a| a == "--independent");
        if independent {
            args.remove(0);
        }
        if args.first().is_some_and(|a| a == "--extract") {
            return extract(&args);
        }
        if args.first().is_some_and(|a| a == "--serve") {
            ensure!(args.len() == 3, "--serve 出力ディレクトリ ポート");
            return serve(Path::new(&args[1]), args[2].to_string_lossy().parse()?);
        }
        ensure!(
            (4..=5).contains(&args.len()),
            "引数: 配布SQLite 圧縮前packedSQLite libjxlのbin 新しい出力先 [並列数]"
        );
        let settings = Settings {
            independent,
            source: regular(Path::new(&args[0]))?,
            raw: regular(Path::new(&args[1]))?,
            bin: regular(Path::new(&args[2]))?,
            out: std::path::absolute(&args[3])?,
            jobs: args
                .get(4)
                .map(|a| a.to_string_lossy().parse())
                .transpose()?
                .unwrap_or(8),
        };
        ensure!((1..=16).contains(&settings.jobs), "並列数は1〜16");
        let source =
            Connection::open_with_flags(&settings.source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let tiles:Vec<Tile>=source.prepare("SELECT t.dataset_key,t.layer_key,t.z,t.x,t.y,l.tile_pixels FROM surface_tiles t JOIN surface_levels l USING(dataset_key,layer_key,z) ORDER BY t.dataset_key,t.layer_key,t.z,t.x,t.y")?.query_map([],|r|Ok(Tile{dataset:r.get(0)?,layer:r.get(1)?,z:r.get(2)?,x:r.get(3)?,y:r.get(4)?,side:r.get(5)?}))?.collect::<rusqlite::Result<_>>()?;
        ensure!(
            tiles.iter().all(|t| (5..=12).contains(&t.z)),
            "対象はz5〜z12"
        );
        let tile_count = tiles.len();
        let jobs = if independent {
            plan_independent(tiles)
        } else {
            plan(tiles)
        };
        let total = jobs.len();
        fs::create_dir_all(&settings.out)?;
        let final_path = settings.out.join(if independent {
            "esa-worldcover-jxl-independent-d4.sqlite3"
        } else {
            "esa-worldcover-jxl-d4.sqlite3"
        });
        ensure!(!final_path.exists(), "生成済みの出力先です");
        let cfg = config(&settings)?;
        let cfg_path = settings.out.join("config.json");
        let partial = settings.out.join("generation.sqlite3.partial");
        if cfg_path.exists() {
            ensure!(
                serde_json::from_slice::<Value>(&fs::read(&cfg_path)?)? == cfg,
                "再開時の入力・圧縮設定が一致しません"
            );
            ensure!(partial.exists(), "再開用SQLiteがありません");
        } else {
            ensure!(!partial.exists(), "不明な途中出力があります");
            fs::write(&cfg_path, serde_json::to_vec_pretty(&cfg)?)?;
        }
        let fresh = !partial.exists();
        let db = Connection::open(&partial)?;
        if fresh {
            initialize(&db, &settings.source)?;
            db.execute(
                "INSERT INTO experiment_info VALUES('configuration',?)",
                [cfg.to_string()],
            )?;
        }
        db.execute_batch(
            "PRAGMA foreign_keys=ON;PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL;",
        )?;
        let completed: BTreeMap<i64, usize> = db
            .prepare("SELECT group_id,length(payload) FROM jxl_groups")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut done = completed.len();
        let initial_done = done;
        let mut payload_bytes: usize = completed.values().sum();
        let mut tiles_done: usize =
            db.query_row("SELECT count(*) FROM surface_tiles", [], |r| r.get(0))?;
        let queue = Arc::new(Mutex::new(
            jobs.into_iter()
                .filter(|j| !completed.contains_key(&j.id))
                .collect::<VecDeque<_>>(),
        ));
        let (sender, receiver) = mpsc::sync_channel::<(i64, Result<Finished>)>(settings.jobs * 2);
        let layer_names: BTreeMap<i64, String> = source
            .prepare("SELECT layer_key,id FROM surface_layers")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        fs::write(
            settings.out.join("progress.html"),
            if independent {
                html().replace("z10〜z12は段階式で共有", "全レベルの全タイルを独立圧縮")
            } else {
                html().to_string()
            },
        )?;
        let start = Instant::now();
        let mut failures = Vec::<Value>::new();
        let progress = |phase: &str,
                        done: usize,
                        tiles: usize,
                        bytes: usize,
                        failures: &Vec<Value>|
         -> Result<()> {
            let seconds = start.elapsed().as_secs_f64();
            atomic(
                &settings.out.join("progress.json"),
                &serde_json::to_vec_pretty(
                    &json!({"phase":phase,"done":done,"total":total,"tiles":tiles,"total_tiles":tile_count,"payload_bytes":bytes,"elapsed_seconds":seconds,"eta_seconds":if done>initial_done{Some(seconds*(total-done)as f64/(done-initial_done)as f64)}else{None},"failures":failures,"output":final_path}),
                )?,
            )?;
            Ok(())
        };
        progress(
            if independent {
                "独立圧縮と復号の検査"
            } else {
                "圧縮と段階復号の検査"
            },
            done,
            tiles_done,
            payload_bytes,
            &failures,
        )?;
        let mut workers = Vec::new();
        for worker in 0..settings.jobs {
            let queue = queue.clone();
            let tx = sender.clone();
            let settings = settings.clone();
            let names = layer_names.clone();
            workers.push(std::thread::spawn(move || -> Result<()> {
                let raw =
                    Connection::open_with_flags(&settings.raw, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                let api = Api::load(&settings.bin.join("jxl.dll"))?;
                let scratch = settings.out.join(format!("scratch-{worker}"));
                fs::create_dir_all(&scratch)?;
                loop {
                    let job = queue.lock().unwrap().pop_front();
                    let Some(job) = job else { break };
                    let id = job.id;
                    let layer = names.get(&job.anchor.layer).context("レイヤー不明")?;
                    let result = convert(&raw, &api, &settings, job, layer, &scratch);
                    if tx.send((id, result)).is_err() {
                        break;
                    }
                }
                Ok(())
            }));
        }
        drop(sender);
        db.execute_batch("BEGIN")?;
        let mut pending = 0;
        for (id, result) in receiver {
            match result {
                Ok(group) => {
                    save(&db, &group)?;
                    payload_bytes += group.payload.len();
                    tiles_done += group.job.tiles.len();
                    done += 1;
                    pending += 1;
                }
                Err(error) => failures.push(json!({"group_id":id,"error":format!("{error:#}")})),
            }
            if pending >= 32 {
                db.execute_batch("COMMIT;BEGIN")?;
                pending = 0;
            }
            if done % 8 == 0 || done == total || !failures.is_empty() {
                progress(
                    "圧縮と段階復号の検査",
                    done,
                    tiles_done,
                    payload_bytes,
                    &failures,
                )?;
            }
            if done % 128 == 0 {
                eprintln!(
                    "{done}/{total} グループ、{tiles_done}/{tile_count} タイル、{:.1} MiB",
                    payload_bytes as f64 / 1048576.
                );
            }
        }
        db.execute_batch("COMMIT")?;
        for worker in workers {
            match worker.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => failures.push(json!({"worker_error":format!("{e:#}")})),
                Err(_) => failures.push(json!({"worker_error":"worker panic"})),
            }
        }
        if !failures.is_empty() {
            progress(
                "失敗あり・再実行で不足分を再開",
                done,
                tiles_done,
                payload_bytes,
                &failures,
            )?;
            anyhow::bail!("{}件失敗。進捗JSONを確認してください", failures.len());
        }
        ensure!(
            done == total && tiles_done == tile_count,
            "生成件数が一致しません"
        );
        progress(
            "SQLite整合性と全BLOBの検査",
            done,
            tiles_done,
            payload_bytes,
            &failures,
        )?;
        let check: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        ensure!(check == "ok", "SQLite整合性エラー: {check}");
        let foreign: bool = db.prepare("PRAGMA foreign_key_check")?.exists([])?;
        ensure!(!foreign, "参照整合性エラー");
        for row in db
            .prepare("SELECT payload,payload_hash FROM jxl_groups")?
            .query_map([], |r| {
                Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?))
            })?
        {
            let (bytes, hash) = row?;
            ensure!(fingerprint(&bytes) == hash, "保存BLOBのハッシュ不一致");
        }
        let missing:bool=db.prepare("SELECT 1 FROM surface_tiles t JOIN jxl_groups g USING(group_id) WHERE prefix_bytes<=0 OR prefix_bytes>length(payload) OR t.dataset_key!=g.dataset_key OR t.layer_key!=g.layer_key LIMIT 1")?.exists([])?;
        ensure!(!missing, "タイル参照または途中入力が不正です");
        let summary = statistics(&db, &source)?;
        db.execute(
            "INSERT OR REPLACE INTO experiment_info VALUES('summary',?)",
            [summary.to_string()],
        )?;
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);PRAGMA journal_mode=DELETE;")?;
        drop(db);
        fs::rename(&partial, &final_path)?;
        let mut summary = summary;
        summary["file_bytes"] = json!(fs::metadata(&final_path)?.len());
        summary["payload_bytes"] = json!(payload_bytes);
        summary["groups"] = json!(total);
        summary["tiles"] = json!(tile_count);
        summary["elapsed_seconds"] = json!(start.elapsed().as_secs_f64());
        summary["output"] = json!(final_path);
        atomic(
            &settings.out.join("summary.json"),
            &serde_json::to_vec_pretty(&summary)?,
        )?;
        progress(
            "完了・全グループ検査済み",
            done,
            tiles_done,
            payload_bytes,
            &failures,
        )?;
        println!("{}", serde_json::to_string_pretty(&summary)?);
        Ok(())
    }
    #[test]
    fn families_preserve_every_tile() {
        let parent = Tile {
            dataset: 1,
            layer: 1,
            z: 10,
            x: 900,
            y: 400,
            side: 384,
        };
        let mut tiles = Vec::new();
        for z in 10..=12 {
            let n = 1 << (z - 10);
            for dy in 0..n {
                for dx in 0..n {
                    tiles.push(Tile {
                        z,
                        x: parent.x * n + dx,
                        y: parent.y * n + dy,
                        ..parent.clone()
                    });
                }
            }
        }
        let separate = plan_independent(tiles.clone());
        assert_eq!(separate.len(), 21);
        assert!(
            separate
                .iter()
                .all(|j| j.anchor.z == j.finest && j.tiles.len() == 1)
        );
        assert!(separate.iter().all(|j| j.anchor.side == 384));
        let grouped = plan(tiles.clone());
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].tiles.len(), 21);
        assert_eq!(grouped[0].finest, 12);
        tiles.pop();
        let independent = plan(tiles);
        assert_eq!(independent.len(), 20);
        assert!(
            independent
                .iter()
                .all(|j| j.anchor.z == j.finest && j.tiles.len() == 1)
        );
    }
}
