//! Q4～Q12の標高取得・TIN生成・品質補修・検証・並列実行・保存をRustで完結する。
#[allow(dead_code)] mod adaptive_tin;
#[allow(dead_code)] mod codec;
#[allow(dead_code)] mod mesh;
use anyhow::{Context,Result,ensure};
use clap::Parser;
use flate2::{Compression,write::ZlibEncoder,read::ZlibDecoder};
use rusqlite::{Connection,OpenFlags,OptionalExtension,params};
use serde_json::{Value,json};
use std::{collections::{HashMap,VecDeque,BTreeSet},fs::{self,File,OpenOptions},io::{Read,Write},path::{Path,PathBuf},sync::{Arc,Mutex,mpsc},time::{Duration,Instant,SystemTime,UNIX_EPOCH}};
mod q_additional_store;
mod q_pool_bench;
mod q_pool;
mod q_hierarchy;
mod triangle_frontier;
const SPACING:f64=31.25;
const R:f64=6378137.;

fn default_workers()->usize {std::thread::available_parallelism().map(|n|(n.get()/2).max(1)).unwrap_or(1)}

#[derive(Parser,Clone)]
struct Args {
    #[arg(long,default_value="build/terrain-lod.sqlite3")] source:PathBuf,
    #[arg(long,default_value="build/terrain-error-national-base.sqlite3")] catalog:PathBuf,
    #[arg(long,default_value="build/terrain-q4-q12-basic.sqlite3")] output:PathBuf,
    #[arg(long,default_value_t=default_workers(),value_parser=clap::value_parser!(usize))] workers:usize,
    #[arg(long,default_value_t=8.)] min_free_gib:f64,
    #[arg(long)] limit:Option<usize>,
    #[arg(long)] verify:bool,
    #[arg(long, requires="base_input")] prepare_additional:bool,
    #[arg(long)] base_input:Option<PathBuf>,
    #[arg(long, requires="base_input")] benchmark_additional:bool,
    #[arg(long, requires="base_input")] additional:bool,
    #[arg(long,default_value_t=7200.)] max_additional_seconds:f64,
    #[arg(long,default_value_t=0.)] density_free_vertices:f64,
    #[arg(long)] field_cache_input:Option<PathBuf>,
    #[arg(long,allow_hyphen_values=true,value_delimiter=',',num_args=1)] sample:Option<Vec<i64>>,
    #[arg(long)] sample_output:Option<PathBuf>,
    #[arg(long)] stop_file:Option<PathBuf>,
    #[arg(long, requires="base_input", conflicts_with_all=["additional","verify","prepare_additional","benchmark_additional","sample"])] hierarchical:bool,
    #[arg(long,default_value_t=4)] minimum_q:i64,
    #[arg(long,default_value_t=12)] maximum_q:i64,
}
#[derive(Clone,Copy,Debug)]
struct Job {q:i64,x:i64,y:i64}
impl Job {
    fn width(self)->f64 {2000.*2f64.powi(self.q as i32-4)}
    fn n(self)->usize {(self.width()/SPACING) as usize+1}
    fn error(self)->f64 {12.5*2f64.powi(self.q as i32-4)}
    fn memory(self)->u64 {(self.n() as u64).pow(2)*40+256*1024*1024}
}
fn now()->f64 {SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64()}
fn read(path:&Path)->Result<Connection> {Ok(Connection::open_with_flags(path,OpenFlags::SQLITE_OPEN_READ_ONLY)?)}
fn zip(data:&[u8])->Result<Vec<u8>> {let mut out=ZlibEncoder::new(Vec::new(),Compression::new(6));out.write_all(data)?;Ok(out.finish()?)}

#[cfg(windows)]
mod resources {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    #[repr(C)] struct Memory {length:u32,load:u32,total:u64,available:u64,page_total:u64,page_available:u64,virtual_total:u64,virtual_available:u64,extended:u64}
    #[link(name="kernel32")]
    unsafe extern "system" {fn GlobalMemoryStatusEx(p:*mut Memory)->i32;fn GetDiskFreeSpaceExW(path:*const u16,free:*mut u64,total:*mut u64,total_free:*mut u64)->i32;}
    pub fn memory()->Result<u64> {let mut m=Memory{length:64,load:0,total:0,available:0,page_total:0,page_available:0,virtual_total:0,virtual_available:0,extended:0};ensure!(unsafe{GlobalMemoryStatusEx(&mut m)}!=0,"空きメモリーを取得できません");Ok(m.available)}
    pub fn disk(path:&Path)->Result<u64> {let wide=path.canonicalize()?.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();let (mut free,mut total,mut all)=(0,0,0);ensure!(unsafe{GetDiskFreeSpaceExW(wide.as_ptr(),&mut free,&mut total,&mut all)}!=0,"空きディスク容量を取得できません");Ok(free)}
}
#[cfg(not(windows))]
mod resources {use super::*;pub fn memory()->Result<u64>{Ok(8*1024*1024*1024)}pub fn disk(_path:&Path)->Result<u64>{Ok(u64::MAX)}}

struct Source {db:Connection,cache:HashMap<(i64,i64),Option<codec::Grid>>,order:VecDeque<(i64,i64)>,nn:f64,ff:f64,rho0:f64}
impl Source {
    fn open(path:&Path)->Result<Self> {
        let (p1,p2)=(30f64.to_radians(),46f64.to_radians());let nn=(p1.cos()/p2.cos()).ln()/((std::f64::consts::FRAC_PI_4+p2/2.).tan()/(std::f64::consts::FRAC_PI_4+p1/2.).tan()).ln();
        let ff=p1.cos()*(std::f64::consts::FRAC_PI_4+p1/2.).tan().powf(nn)/nn;let rho0=R*ff/(std::f64::consts::FRAC_PI_4+36f64.to_radians()/2.).tan().powf(nn);
        Ok(Self{db:read(path)?,cache:HashMap::new(),order:VecDeque::new(),nn,ff,rho0})
    }
    fn pixel(&mut self,x:i64,y:i64)->Result<Option<f64>> {
        let key=(x.div_euclid(256),y.div_euclid(256));
        if !self.cache.contains_key(&key) {
            let body:Option<Vec<u8>>=self.db.prepare_cached("SELECT data FROM tiles WHERE z=14 AND x=?1 AND y=?2")?.query_row(params![key.0,key.1],|r|r.get(0)).optional()?;
            let grid=body.map(|b|codec::decode_grid(&b)).transpose()?;
            if let Some(g)=&grid {ensure!(g.rows==256&&g.columns==256,"元標高は256四方が必要です");}
            if self.cache.len()>=1024 {if let Some(old)=self.order.pop_front(){self.cache.remove(&old);}}
            self.order.push_back(key);self.cache.insert(key,grid);
        }
        Ok(self.cache[&key].as_ref().and_then(|g|{let z=g.values[(y.rem_euclid(256)*256+x.rem_euclid(256)) as usize];z.is_finite().then_some(z as f64)}))
    }
    fn at(&mut self,x:f64,y:f64)->Result<(f64,bool)> {
        let rho=x.hypot(self.rho0-y);let theta=x.atan2(self.rho0-y);
        let lon=(theta/self.nn+138f64.to_radians()).to_degrees();let lat=2.*(R*self.ff/rho).powf(1./self.nn).atan()-std::f64::consts::FRAC_PI_2;
        let px=(lon+180.)/360.*4194304.-0.5;let py=(1.-lat.tan().asinh()/std::f64::consts::PI)/2.*4194304.-0.5;
        let (ix,iy)=(px.floor() as i64,py.floor() as i64);let (fx,fy)=(px-ix as f64,py-iy as f64);let (mut value,mut weight)=(0.,0.);
        for (dx,dy,w) in [(0,0,(1.-fx)*(1.-fy)),(1,0,fx*(1.-fy)),(0,1,(1.-fx)*fy),(1,1,fx*fy)] {
            if let Some(z)=self.pixel(ix+dx,iy+dy)? {value+=z*w;weight+=w;}
        }
        let valid=weight>0.999999;
        // Python版のfloat32格納と0.25 m量子化を引き継ぐ。
        Ok((if valid {((value as f32*4.).round_ties_even()/4.) as f64}else{0.},valid))
    }
    fn grid(&mut self,job:Job)->Result<(Vec<f64>,Vec<u8>,usize)> {
        let n=job.n();let mut grid=vec![0.;n*n];let mut bits=vec![0;n.checked_mul(n).context("格子が大きすぎます")?.div_ceil(8)];let mut count=0;
        for y in 0..n {for x in 0..n {let i=y*n+x;let (z,valid)=self.at(job.x as f64*job.width()+x as f64*SPACING,job.y as f64*job.width()+y as f64*SPACING)?;grid[i]=z;if valid {bits[i/8]|=1<<(i%8);count+=1;}}}
        Ok((grid,bits,count))
    }
}

struct Packed {vertices:usize,triangles:usize,mesh:Vec<u8>,validity:Vec<u8>,error:f64,angle:f64,rounds:usize}
fn generate(source:&mut Source,job:Job)->Result<Option<Packed>> {
    ensure!((job.n() as u64).pow(2)*40<resources::memory()?*7/10,"Q{}の格子評価には空きメモリーが不足しています。条件を変えず停止します",job.q);
    let (grid,bits,valid)=source.grid(job)?;if valid==0 {return Ok(None);}
    let mesh=adaptive_tin::quality_tin::generate(&grid,job.n(),job.error(),10.)?;
    let mut body=Vec::with_capacity(12+mesh.points.len()*12+mesh.faces.len()*12);body.extend(b"YQB1");body.extend((mesh.points.len() as u32).to_le_bytes());body.extend((mesh.faces.len() as u32).to_le_bytes());
    for p in &mesh.points {for value in [p[0] as f64*SPACING,p[1] as f64*SPACING,grid[p[1] as usize*job.n()+p[0] as usize]] {body.extend((value as f32).to_le_bytes());}}
    for face in &mesh.faces {for &id in face {body.extend((id as u32).to_le_bytes());}}
    Ok(Some(Packed{vertices:mesh.points.len(),triangles:mesh.faces.len(),mesh:zip(&body)?,validity:zip(&bits)?,error:mesh.max_error,angle:mesh.minimum_angle,rounds:mesh.rounds}))
}

fn levels()->Value {
    json!((4..=12).map(|q|{let job=Job{q,x:0,y:0};let radius=job.width()/2.;let area=std::f64::consts::PI*(radius/1000.).powi(2);json!({"level":q,"name":format!("Q{q}"),"maxErrorMeters":job.error(),"widthMeters":job.width() as u64,"sampleSpacingMeters":SPACING,"minimumAngleDegrees":10,"additionalFraction":0.2,"densityRadiusMeters":radius,"densityReferenceVertices":4096,"densityReferenceVerticesPerKm2":4096./area,"densityReferenceAreaKm2":area})}).collect::<Vec<_>>())
}
fn open_output(args:&Args)->Result<Connection> {
    let db=Connection::open(&args.output)?;db.busy_timeout(Duration::from_secs(30))?;
    db.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL;CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS jobs(q INTEGER,x INTEGER,y INTEGER,state INTEGER DEFAULT 0,failure TEXT,PRIMARY KEY(q,x,y));CREATE TABLE IF NOT EXISTS basic(q INTEGER,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER,mesh BLOB,source_validity BLOB,PRIMARY KEY(q,x,y));CREATE TABLE IF NOT EXISTS basic_stats(q INTEGER,x INTEGER,y INTEGER,engine TEXT,max_error REAL,min_angle REAL,rounds INTEGER,PRIMARY KEY(q,x,y));")?;
    let previous:Option<String>=db.query_row("SELECT value FROM metadata WHERE key='manifest'",[],|r|r.get(0)).optional()?;
    if let Some(previous)=previous {
        let manifest:Value=serde_json::from_str(&previous)?;
        ensure!(Path::new(manifest["source"].as_str().context("元標高の記録がありません")?).canonicalize()?==args.source.canonicalize()?,"元標高のパスが既存DBと異なります");
        ensure!(manifest["sourceSize"].as_u64()==Some(fs::metadata(&args.source)?.len()),"元標高のサイズが変わっています");
        let modified=fs::metadata(&args.source)?.modified()?.duration_since(UNIX_EPOCH)?.as_nanos() as u64;
        ensure!(manifest["sourceMtimeNs"].as_u64()==Some(modified),"元標高の更新日時が変わっています");
        let previous_levels=manifest["levels"].as_array().context("レベル定義がありません")?;
        ensure!(previous_levels.len()==9,"9レベルの定義が必要です");
        for (old,new) in previous_levels.iter().zip(levels().as_array().unwrap()) {for key in ["level","widthMeters","maxErrorMeters","sampleSpacingMeters","minimumAngleDegrees"] {ensure!(old[key].as_f64()==new[key].as_f64(),"既存DBのレベル定義が異なります: {key}");}}
    } else {
        let metadata=fs::metadata(&args.source)?;
        let config=json!({"version":1,"levels":levels(),"source":args.source.canonicalize()?.to_string_lossy(),"sourceSize":metadata.len(),"sourceMtimeNs":metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos() as u64,"stage":"basic-before-national-pool-and-boundary-connection","coastlineInMesh":false,"demSlopeHints":false,"demVertexNormals":false,"pruningEnabled":false,"additionalScope":"national-global-pool","additionalFraction":0.2,"distributable":false,"boundaryConnectionComplete":false,"additionalComplete":false});
        db.execute("INSERT INTO metadata VALUES('manifest',?1)",[config.to_string()])?;
    }
    let count:u64=db.query_row("SELECT count(*) FROM jobs",[],|r|r.get(0))?;
    if count==0 {
        let catalog=read(&args.catalog)?;let mut query=catalog.prepare("SELECT x,y FROM jobs WHERE error=800 AND state=1")?;
        let parents=query.query_map([],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;ensure!(!parents.is_empty(),"全国目録が空です");
        let tx=db.unchecked_transaction()?;
        for q in 4..=12 {let width=Job{q,x:0,y:0}.width() as i64;let mut keys=BTreeSet::new();for &(x,y) in &parents {
            if width<8000 {let k=8000/width;for dx in 0..k {for dy in 0..k {keys.insert((x*k+dx,y*k+dy));}}}
            else {let k=width/8000;keys.insert((x.div_euclid(k),y.div_euclid(k)));}
        }for (x,y) in keys {tx.execute("INSERT INTO jobs(q,x,y) VALUES(?1,?2,?3)",params![q,x,y])?;}}
        tx.commit()?;
    }
    Ok(db)
}

fn save(db:&Connection,job:Job,result:Option<Packed>)->Result<()> {
    let tx=db.unchecked_transaction()?;let mut count=0;
    if let Some(p)=result {count=p.vertices;tx.execute("INSERT INTO basic VALUES(?1,?2,?3,?4,?5,?6,?7)",params![job.q,job.x,job.y,p.vertices,p.triangles,p.mesh,p.validity])?;tx.execute("INSERT INTO basic_stats VALUES(?1,?2,?3,'rust',?4,?5,?6)",params![job.q,job.x,job.y,p.error,p.angle,p.rounds])?;}
    tx.execute("UPDATE jobs SET state=?1,failure=NULL WHERE q=?2 AND x=?3 AND y=?4",params![if count>0 {1}else{2},job.q,job.x,job.y])?;tx.commit()?;
    println!("基本タイル保存 (Q{}, {}, {}) 頂点 {} Rust",job.q,job.x,job.y,count);Ok(())
}

fn progress(db:&Connection,args:&Args,stage:&str,start:Instant,offset:f64,active:usize,reserved:u64,budget:u64)->Result<()> {
    let mut query=db.prepare("SELECT q,state,count(*) FROM jobs GROUP BY q,state ORDER BY q,state")?;let jobs=query.query_map([],|r|Ok([r.get::<_,u64>(0)?,r.get(1)?,r.get(2)?]))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut query=db.prepare("SELECT q,count(*),sum(vertices),sum(triangles),sum(length(mesh)),sum(length(source_validity)) FROM basic GROUP BY q ORDER BY q")?;let sizes=query.query_map([],|r|Ok([r.get::<_,u64>(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?]))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let free=resources::disk(args.output.parent().unwrap_or(Path::new(".")))?;
    let state=json!({"stage":stage,"engine":"rust","pid":std::process::id(),"updated":now(),"elapsedSeconds":offset+start.elapsed().as_secs_f64(),"runElapsedSeconds":start.elapsed().as_secs_f64(),"levels":levels(),"workerLimit":args.workers,"activeWorkers":active,"reservedWorkerBytes":reserved,"workerMemoryBudgetBytes":budget,"jobs":jobs,"sizes":sizes,"databaseBytes":fs::metadata(&args.output)?.len(),"freeBytes":free,"complete":false,"distributable":false,"pendingStages":["national-global-pool-20-percent","cross-tile-and-cross-level-boundary-connection","final-verification"]});
    let path=args.output.with_extension("progress.json");let temporary=path.with_extension("tmp");fs::write(&temporary,serde_json::to_vec_pretty(&state)?)?;fs::rename(temporary,path)?;
    ensure!(free as f64>=args.min_free_gib*1073741824.,"空きディスク容量が停止条件を下回りました");Ok(())
}

fn run(args:&Args)->Result<()> {
    ensure!(args.workers>0,"並列数は1以上が必要です");
    if args.hierarchical {let result=q_hierarchy::run(args);if result.is_err()&&args.stop_file.as_ref().is_some_and(|p|p.exists()){fs::write(args.output.with_extension("progress.json"),serde_json::to_vec_pretty(&json!({"engine":"rust","stage":"paused","updated":now(),"pid":std::process::id(),"complete":false,"distributable":false}))?)?;return Ok(());}return result;}
    if let Some(sample)=&args.sample {
        ensure!(sample.len()==3&&(4..=12).contains(&sample[0]),"sampleはQ番号,x,yが必要です");let job=Job{q:sample[0],x:sample[1],y:sample[2]};let (grid,bits,_)=Source::open(&args.source)?.grid(job)?;
        let mut file=File::create(args.sample_output.as_ref().context("sample-outputが必要です")?)?;file.write_all(b"YQG1")?;file.write_all(&(job.n() as u32).to_le_bytes())?;for z in grid {file.write_all(&(z as f32).to_le_bytes())?;}file.write_all(&bits)?;return Ok(());
    }
    if args.additional {return q_pool::run(args);}
    if args.benchmark_additional { return q_pool_bench::run(args); }
    if args.prepare_additional {
        let base=args.base_input.as_ref().unwrap();
        q_additional_store::prepare(base,&args.output)?;
        println!("追加工程の別SQLiteを準備しました（頂点追加は未実行）: {}",args.output.display());return Ok(());
    }
    ensure!(args.base_input.is_none(),"base-inputはprepare-additionalと併用してください");
    if args.verify {return verify(args);}
    fs::create_dir_all(args.output.parent().unwrap_or(Path::new(".")))?;
    let mut options=OpenOptions::new();options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)] {use std::os::windows::fs::OpenOptionsExt;options.share_mode(0);}
    let _lock=options.open(args.output.with_extension("lock")).context("同じ出力の生成が動いています")?;
    let db=open_output(args)?;let started=Instant::now();let mut last_status=Instant::now();
    let offset=fs::read(args.output.with_extension("progress.json")).ok().and_then(|b|serde_json::from_slice::<Value>(&b).ok()).and_then(|v|v["elapsedSeconds"].as_f64()).unwrap_or(0.);
    let budget=resources::memory()?*55/100;
    let run_info=json!({"engine":"rust","executable":std::env::current_exe()?.to_string_lossy(),"started":now(),"workers":args.workers,"allBasicStagesInRust":true});
    db.execute("INSERT OR REPLACE INTO metadata VALUES('runtime',?1)",[run_info.to_string()])?;
    progress(&db,args,"basic-generating",started,offset,0,0,budget)?;
    let mut query=db.prepare("SELECT q,x,y FROM jobs WHERE state=0 ORDER BY q,y,x")?;
    let mut jobs=query.query_map([],|r|Ok(Job{q:r.get(0)?,x:r.get(1)?,y:r.get(2)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;drop(query);if let Some(limit)=args.limit {jobs.truncate(limit);}
    let mut stopped=false;
    std::thread::scope(|scope|->Result<()> {
        let (send,receive)=mpsc::sync_channel::<Job>(args.workers);let receive=Arc::new(Mutex::new(receive));
        let (result_send,result_receive)=mpsc::sync_channel::<(Job,Result<Option<Packed>>)>(args.workers*2);
        for _ in 0..args.workers {let receive=receive.clone();let output=result_send.clone();let path=args.source.clone();scope.spawn(move|| {
            let mut source=Source::open(&path);
            loop {let job=match receive.lock().unwrap().recv(){Ok(job)=>job,Err(_)=>break};let result=match &mut source {Ok(source)=>generate(source,job),Err(error)=>Err(anyhow::anyhow!("元標高の初期化に失敗: {error:#}"))};if output.send((job,result)).is_err(){break;}}
        });}drop(result_send);
        let mut cursor=0;let mut active=HashMap::<(i64,i64,i64),u64>::new();
        let work=(||->Result<()> {loop {
            if args.stop_file.as_ref().is_some_and(|p|p.exists()) {stopped=true;}
            while !stopped&&cursor<jobs.len()&&active.len()<args.workers {
                let job=jobs[cursor];let estimate=job.memory();let reserved=active.values().sum::<u64>();if !active.is_empty()&&reserved+estimate>budget {break;}
                send.send(job)?;active.insert((job.q,job.x,job.y),estimate);cursor+=1;
            }
            if active.is_empty(){break;}
            match result_receive.recv_timeout(Duration::from_secs(2)) {
                Ok((job,result))=>{active.remove(&(job.q,job.x,job.y));match result {Ok(p)=>save(&db,job,p)?,Err(error)=>{db.execute("UPDATE jobs SET state=3,failure=?1 WHERE q=?2 AND x=?3 AND y=?4",params![format!("{error:#}"),job.q,job.x,job.y])?;progress(&db,args,"basic-failed",started,offset,active.len(),active.values().sum(),budget)?;return Err(error.context(format!("Q{}/{}/{}",job.q,job.x,job.y)));}}}
                Err(mpsc::RecvTimeoutError::Timeout)=>{},Err(error)=>return Err(error.into()),
            }
            if last_status.elapsed()>=Duration::from_secs(2) {progress(&db,args,"basic-generating",started,offset,active.len(),active.values().sum(),budget)?;last_status=Instant::now();}
        }Ok(())})();drop(send);drop(result_receive);work
    })?;
    let remaining:u64=db.query_row("SELECT count(*) FROM jobs WHERE state IN (0,3)",[],|r|r.get(0))?;
    progress(&db,args,if stopped {"basic-stopped"}else if remaining>0 {"basic-partial"}else{"basic-ready-for-national-pool"},started,offset,0,0,budget)?;Ok(())
}

fn verify(args:&Args)->Result<()> {
    let db=read(&args.output)?;let mut source=Source::open(&args.source)?;
    let mut query=db.prepare("SELECT q,x,y,vertices,triangles,mesh,source_validity FROM basic ORDER BY q,x,y")?;
    let mut rows=query.query([])?;let mut count=0;
    while let Some(row)=rows.next()? {
        if args.limit.is_some_and(|n|count>=n){break;}
        let job=Job{q:row.get(0)?,x:row.get(1)?,y:row.get(2)?};let nv:usize=row.get(3)?;let nf:usize=row.get(4)?;let body:Vec<u8>=row.get(5)?;let validity:Vec<u8>=row.get(6)?;
        let mut raw=Vec::new();ZlibDecoder::new(&body[..]).take((12+nv*12+nf*12+1) as u64).read_to_end(&mut raw)?;
        ensure!(raw.len()==12+nv*12+nf*12&&&raw[..4]==b"YQB1","メッシュの形式が不正です");
        let mut p=Vec::with_capacity(nv);
        let (grid,bits,_)=source.grid(job)?;let mut decoded_bits=Vec::new();ZlibDecoder::new(&validity[..]).read_to_end(&mut decoded_bits)?;ensure!(bits==decoded_bits,"元標高の有効域が一致しません");
        for i in 0..nv {let point:[f32;3]=std::array::from_fn(|k|f32::from_le_bytes(raw[12+i*12+k*4..16+i*12+k*4].try_into().unwrap()));ensure!(point.iter().all(|v|v.is_finite()),"非有限の頂点があります");let xy=[(point[0] as f64/SPACING).round() as i64,(point[1] as f64/SPACING).round() as i64];ensure!(xy.iter().all(|&v|v>=0&&v<job.n() as i64),"格子外の頂点です");ensure!((grid[xy[1] as usize*job.n()+xy[0] as usize]-point[2] as f64).abs()<1e-5,"頂点の標高が一致しません");p.push(xy);}
        let mut faces=Vec::with_capacity(nf);for i in 0..nf {let f:[usize;3]=std::array::from_fn(|k|u32::from_le_bytes(raw[12+nv*12+i*12+k*4..16+nv*12+i*12+k*4].try_into().unwrap()) as usize);ensure!(f.iter().all(|&i|i<nv),"頂点参照が範囲外です");faces.push(f);}
        let area:i128=faces.iter().map(|f|{let [a,b,c]=f.map(|i|p[i]);(b[0]-a[0]) as i128*(c[1]-a[1]) as i128-(b[1]-a[1]) as i128*(c[0]-a[0]) as i128}).sum();
        ensure!(area==2*(job.n()-1).pow(2) as i128,"被覆面積が一致しません");
        let error=adaptive_tin::quality_tin::audit(&p,&faces,&grid,job.n(),job.error())?.0;let angle=faces.iter().map(|&f|adaptive_tin::quality_tin::minimum_angle(&p,f)).fold(180f64,f64::min);
        ensure!(error<=job.error()+1e-6&&angle>=10.-1e-6,"誤差または角度が条件外: {error} m / {angle}度");count+=1;
    }
    println!("検証成功 {count} タイル Rust");Ok(())
}

fn main()->Result<()> {
    let args=Args::parse();ensure!(args.workers>0,"並列数は1以上にしてください");let result=run(&args);
    if let Err(error)=&result {eprintln!("生成失敗: {error:#}");if !args.verify&&!args.prepare_additional&&!args.benchmark_additional&&!args.additional&&args.sample.is_none()&&!format!("{error:#}").contains("同じ出力の生成") {let _=fs::write(args.output.with_extension("failure.json"),json!({"engine":"rust","error":format!("{error:#}"),"updated":now()}).to_string());}}
    result
}

#[cfg(test)] mod tests {use super::*;
    #[test] fn levels_match_q4_to_q12() {let all=levels();assert_eq!(all.as_array().unwrap().len(),9);assert_eq!(all[0]["widthMeters"],2000);assert_eq!(all[8]["widthMeters"],512000);assert_eq!(all[8]["maxErrorMeters"],3200.);}
    #[test] fn negative_catalog_coordinates_use_floor() {assert_eq!((-1i64).div_euclid(2),-1);assert_eq!(Job{q:4,x:0,y:0}.n(),65);assert_eq!(Job{q:12,x:0,y:0}.n(),16385);}
}
