//! レベルごとの全国共通キュー。基本SQLiteを書き換えず、更新と挿入履歴を別DBへ保存する。
use super::*;
use std::collections::{BinaryHeap,HashSet};
use rayon::prelude::*;
type Key=(i64,i64);
struct Density {counts:HashMap<Key,u32>,stride:i64,step:i64}
impl Density {
    fn new(job:Job)->Self {let stride=job.n() as i64-1;Self{counts:HashMap::new(),stride,step:(stride/8).max(1)}}
    fn add(&mut self,p:Key) {
        let x=p.0 as f64/self.step as f64;let y=p.1 as f64/self.step as f64;let r=self.stride as f64/(2.*self.step as f64);
        for yy in (y-r).ceil() as i64..=(y+r).floor() as i64 {for xx in (x-r).ceil() as i64..=(x+r).floor() as i64 {
            if (xx as f64-x).powi(2)+(yy as f64-y).powi(2)<=r*r+1e-9 {*self.counts.entry((xx,yy)).or_default()+=1;}
        }}
    }
    fn tile(&self,key:Key)->Vec<f64> {let cells=self.stride/self.step;let mut out=Vec::new();for y in 0..=cells {for x in 0..=cells {out.push(*self.counts.get(&(key.0*cells+x,key.1*cells+y)).unwrap_or(&0) as f64);}}out}
    fn world(&self,key:Key,sample:usize,n:usize)->Key {(key.0*self.stride+(sample%n) as i64,key.1*self.stride+(sample/n) as i64)}
}
#[derive(Clone,Copy,Debug)] struct Candidate {score:f64,key:Key}
impl PartialEq for Candidate {fn eq(&self,o:&Self)->bool {self.score==o.score&&self.key==o.key}}
impl Eq for Candidate {}
impl PartialOrd for Candidate {fn partial_cmp(&self,o:&Self)->Option<std::cmp::Ordering>{Some(self.cmp(o))}}
impl Ord for Candidate {fn cmp(&self,o:&Self)->std::cmp::Ordering {self.score.total_cmp(&o.score).then_with(||o.key.cmp(&self.key))}}
struct Tile {builder:adaptive_tin::Builder<'static>,base:usize,version:u64,dirty:bool,load_parts:[f64;3]}

fn packed(builder:&mut adaptive_tin::Builder<'static>)->Result<(usize,usize,Vec<u8>)> {
    let (vertices,triangles)=builder.detail_geometry();let mut body=Vec::new();body.extend(b"YQB1");body.extend((vertices.len() as u32).to_le_bytes());body.extend((triangles.len() as u32).to_le_bytes());
    for p in &vertices {for v in [p[0]*SPACING,p[1]*SPACING,p[2]] {body.extend((v as f32).to_le_bytes());}}
    for f in &triangles {for &v in f {body.extend((v as u32).to_le_bytes());}}
    let mut zipped=b"YQW1".to_vec();zipped.extend(zstd::stream::encode_all(&body[..],1)?);Ok((vertices.len(),triangles.len(),zipped))
}
fn unpack_working(blob:&[u8])->Result<Vec<u8>> {
    if blob.starts_with(b"YQW1") {Ok(zstd::stream::decode_all(&blob[4..])?)}else{let mut raw=Vec::new();ZlibDecoder::new(blob).read_to_end(&mut raw)?;Ok(raw)}
}
fn load_mesh(input:&Connection,output:&Connection,q:i64,key:Key,n:usize)->Result<(Vec<[usize;2]>,Vec<[usize;3]>,usize)> {
    let base:usize=input.query_row("SELECT vertices FROM basic WHERE q=?1 AND x=?2 AND y=?3",params![q,key.0,key.1],|r|r.get(0))?;
    let override_blob:Option<Vec<u8>>=output.query_row("SELECT mesh FROM working WHERE q=?1 AND x=?2 AND y=?3",params![q,key.0,key.1],|r|r.get(0)).optional()?;
    let blob=match override_blob {Some(b)=>b,None=>input.query_row("SELECT mesh FROM basic WHERE q=?1 AND x=?2 AND y=?3",params![q,key.0,key.1],|r|r.get(0))?};
    let raw=unpack_working(&blob)?;let (p,f)=q_pool_bench::decode(&raw,n)?;Ok((p,f,base))
}
// 約125 mの平滑化と250 m周囲の勾配を評価。海の欠損を追加対象にしない。
fn gain(source:&mut Source,job:Job)->Result<(Vec<f64>,Vec<f64>)> {
    let halo=32;let n=job.n();let w=n+halo*2;let mut z=vec![0.;w*w];let mut valid=vec![false;w*w];
    for y in 0..w {for x in 0..w {let (v,b)=source.at(job.x as f64*job.width()+(x as f64-halo as f64)*SPACING,job.y as f64*job.width()+(y as f64-halo as f64)*SPACING)?;z[y*w+x]=v;valid[y*w+x]=b;}}
    let kernel=(-16i32..=16).map(|x|(-(x*x) as f64/32.).exp()).collect::<Vec<_>>();let sum=kernel.iter().sum::<f64>();
    let mut temp=vec![0.;w*w];let mut smooth=vec![0.;w*w];
    for y in 0..w {for x in 0..w {temp[y*w+x]=kernel.iter().enumerate().map(|(k,&v)|z[y*w+(x as isize+k as isize-16).clamp(0,w as isize-1) as usize]*v).sum::<f64>()/sum;}}
    for y in 0..w {for x in 0..w {smooth[y*w+x]=kernel.iter().enumerate().map(|(k,&v)|temp[(y as isize+k as isize-16).clamp(0,w as isize-1) as usize*w+x]*v).sum::<f64>()/sum;}}
    drop(temp);let mut slope=vec![0.;w*w];
    for y in 1..w-1 {for x in 1..w-1 {slope[y*w+x]=((smooth[y*w+x+1]-smooth[y*w+x-1])/(2.*SPACING)).hypot((smooth[(y+1)*w+x]-smooth[(y-1)*w+x])/(2.*SPACING));}}
    // 分離した単調キューにより、最大値フィルターを画素数に比例した時間で行う。
    for vertical in [false,true] {let mut filtered=vec![0.;w*w];for line in 0..w {let index=|a:usize|if vertical {a*w+line}else{line*w+a};let mut queue:VecDeque<usize>=VecDeque::new();let mut next=0;
        for at in 0..w {let hi=(at+8).min(w-1);while next<=hi {while queue.back().is_some_and(|&v|slope[index(v)]<=slope[index(next)]) {queue.pop_back();}queue.push_back(next);next+=1;}while queue.front().is_some_and(|&v|v+8<at) {queue.pop_front();}filtered[index(at)]=slope[index(*queue.front().unwrap())];}
    }slope=filtered;}
    let (low,high)=(1f64.to_radians().tan(),8f64.to_radians().tan());let mut grid=Vec::with_capacity(n*n);let mut limits=Vec::with_capacity(n*n);
    for y in halo..halo+n {for x in halo..halo+n {let i=y*w+x;let t=((slope[i]-low)/(high-low)).clamp(0.,1.);let convex=((4.*smooth[i]-smooth[i-8]-smooth[i+8]-smooth[i-8*w]-smooth[i+8*w])/50.).clamp(0.,1.);
        let weight=if valid[i] {(0.02+0.98*t*t*(3.-2.*t))*(1.+0.5*convex)}else{0.02};grid.push(z[i]);limits.push(job.error()/weight);
    }}Ok((grid,limits))
}
fn decode_fields(raw:&[u8],n:usize)->Result<(Vec<f64>,Vec<f64>)> {
    let count=n*n;ensure!(raw.len()==count*16,"標高キャッシュの長さが不正です");let read=|b:&[u8]|b.chunks_exact(8).map(|v|f64::from_le_bytes(v.try_into().unwrap())).collect::<Vec<_>>();Ok((read(&raw[..count*8]),read(&raw[count*8..])))
}
fn field_blob(db:&Connection,cache:Option<&Connection>,table:&str,job:Job)->Result<Option<Vec<u8>>> {
    let sql=format!("SELECT data FROM {table} WHERE q=?1 AND x=?2 AND y=?3");
    let local=db.query_row(&sql,params![job.q,job.x,job.y],|r|r.get(0)).optional()?;
    if local.is_some(){return Ok(local);}
    cache.map(|c|c.query_row(&sql,params![job.q,job.x,job.y],|r|r.get(0)).optional()).transpose().map(|v|v.flatten()).map_err(Into::into)
}
fn cached_gain(source:&mut Source,db:&Connection,job:Job,cache:Option<&Connection>)->Result<(Vec<f64>,Vec<f64>)> {
    let fast:Option<Vec<u8>>=field_blob(db,cache,"fields_fast",job)?;
    if let Some(blob)=fast {let raw=zstd::stream::decode_all(&blob[..])?;return decode_fields(&raw,job.n());}
    let cached:Option<Vec<u8>>=field_blob(db,cache,"fields",job)?;
    if let Some(blob)=cached {let mut raw=Vec::new();ZlibDecoder::new(&blob[..]).read_to_end(&mut raw)?;db.execute("INSERT OR IGNORE INTO fields_fast VALUES(?1,?2,?3,?4)",params![job.q,job.x,job.y,zstd::stream::encode_all(&raw[..],1)?])?;return decode_fields(&raw,job.n());}
    let (grid,limits)=gain(source,job)?;
    if job.q<=8 {let mut raw=Vec::with_capacity(grid.len()*16);for &v in grid.iter().chain(&limits){raw.extend(v.to_le_bytes());}db.execute("INSERT INTO fields VALUES(?1,?2,?3,?4)",params![job.q,job.x,job.y,zip(&raw)?])?;}
    Ok((grid,limits))
}
struct PreparedCandidate {key:Key,score:f64,fields:Option<Vec<u8>>,fast:Option<Vec<u8>>}
fn prepare_candidate(source:&mut Source,input:&Connection,output:&Connection,job:Job,density:&Density,free_density:f64,cache:Option<&Connection>)->Result<PreparedCandidate> {
    let key=(job.x,job.y);let (p,f,_)=load_mesh(input,output,job.q,key,job.n())?;
    let fast:Option<Vec<u8>>=field_blob(output,cache,"fields_fast",job)?;
    let old:Option<Vec<u8>>=if fast.is_none(){field_blob(output,cache,"fields",job)?}else{None};
    let mut fields=None;let mut new_fast=None;
    let (grid,limits)=if let Some(blob)=fast {decode_fields(&zstd::stream::decode_all(&blob[..])?,job.n())?}
        else if let Some(blob)=old {let mut raw=Vec::new();ZlibDecoder::new(&blob[..]).read_to_end(&mut raw)?;let values=decode_fields(&raw,job.n())?;new_fast=Some(zstd::stream::encode_all(&raw[..],1)?);values}
        else {let values=gain(source,job)?;if job.q<=8 {let mut raw=Vec::with_capacity(values.0.len()*16);for v in values.0.iter().chain(&values.1){raw.extend(v.to_le_bytes());}fields=Some(zip(&raw)?);new_fast=Some(zstd::stream::encode_all(&raw[..],1)?);}values};
    let mut builder=adaptive_tin::Builder::seeded_detail_owned_free(grid,job.n(),&p,&f,&limits,job.error(),density.step as usize,density.tile(key),free_density)?;
    Ok(PreparedCandidate{key,score:builder.detail_next().0,fields,fast:new_fast})
}
fn flush(db:&Connection,q:i64,key:Key,tile:&mut Tile)->Result<()> {
    if tile.dirty {let (nv,nf,body)=packed(&mut tile.builder)?;let ids=zip(&(tile.base..nv).flat_map(|v|(v as u32).to_le_bytes()).collect::<Vec<_>>())?;
        db.execute("INSERT OR REPLACE INTO working VALUES(?1,?2,?3,?4,?5,?6,?7)",params![q,key.0,key.1,nv,nf,body,ids])?;tile.dirty=false;
    }Ok(())
}
fn time_exceeded(limit:f64,seconds:f64)->bool {limit>0. && seconds>limit}

fn status(args:&Args,stage:&str,q:i64,prepared:usize,total:usize,used:u64,budget:u64,start:Instant)->Result<()> {
    fs::write(args.output.with_extension("progress.json"),serde_json::to_vec_pretty(&json!({"engine":"rust","stage":stage,"q":q,"preparedTiles":prepared,"totalTiles":total,"addedVertices":used,"budget":budget,"pid":std::process::id(),"updated":now(),"elapsedSeconds":start.elapsed().as_secs_f64(),"complete":false,"distributable":false}))?)?;Ok(())
}
pub fn run(args:&Args)->Result<()> {
    ensure!(args.max_additional_seconds.is_finite()&&args.max_additional_seconds>=0.,"時間上限が不正です");
    let base=args.base_input.as_ref().context("基本入力が必要です")?;q_additional_store::prepare(base,&args.output)?;
    let mut options=OpenOptions::new();options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)] {use std::os::windows::fs::OpenOptionsExt;options.share_mode(0);}
    let _lock=options.open(args.output.with_extension("lock")).context("同じ追加出力の生成が動いています")?;
    let input=read(base)?;let db=Connection::open(&args.output)?;db.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL;CREATE TABLE IF NOT EXISTS candidates(q INTEGER,x INTEGER,y INTEGER,score REAL NOT NULL,PRIMARY KEY(q,x,y));CREATE TABLE IF NOT EXISTS working(q INTEGER,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER,mesh BLOB,ids BLOB,PRIMARY KEY(q,x,y));CREATE TABLE IF NOT EXISTS trace(q INTEGER,seq INTEGER,x INTEGER,y INTEGER,sample INTEGER,priority REAL,PRIMARY KEY(q,seq));CREATE TABLE IF NOT EXISTS fields(q INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(q,x,y));CREATE TABLE IF NOT EXISTS fields_fast(q INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(q,x,y));")?;
    let manifest:String=input.query_row("SELECT value FROM metadata WHERE key='manifest'",[],|r|r.get(0))?;let manifest:Value=serde_json::from_str(&manifest)?;
    ensure!(Path::new(manifest["source"].as_str().context("元標高の記録がありません")?).canonicalize()?==args.source.canonicalize()?,"元標高が基本生成時と異なります");
    ensure!(manifest["sourceSize"].as_u64()==Some(fs::metadata(&args.source)?.len()),"元標高のサイズが基本生成時と異なります");
    ensure!(manifest["sourceMtimeNs"].as_u64()==Some(fs::metadata(&args.source)?.modified()?.duration_since(UNIX_EPOCH)?.as_nanos() as u64),"元標高の更新日時が基本生成時と異なります");
    ensure!(args.density_free_vertices.is_finite()&&args.density_free_vertices>=0.,"密度の無減点範囲が不正です");
    let legacy="national-pool-v1-gaussian125-slope250-convex250-density-width-div8-floor01";
    let policy=if args.density_free_vertices==0. {legacy.to_string()}else{format!("{legacy}-free{}",args.density_free_vertices)};
    let old:Option<String>=db.query_row("SELECT value FROM metadata WHERE key='pool_policy'",[],|r|r.get(0)).optional()?;if let Some(old)=old {ensure!(old==policy,"追加方式が異なるため別の出力名が必要です");}else {db.execute("INSERT INTO metadata VALUES('pool_policy',?1)",[&policy])?;}
    let start=Instant::now();let mut source=Source::open(&args.source)?;
    let field_cache=args.field_cache_input.as_deref().map(read).transpose()?;
    if let Some(cache)=&field_cache {
        let config:String=cache.query_row("SELECT value FROM metadata WHERE key='additional_config'",[],|r|r.get(0))?;
        ensure!(serde_json::from_str::<Value>(&config)?["base"]["manifest"]==manifest,"標高キャッシュの生成元が一致しません");
        ensure!(args.field_cache_input.as_ref().unwrap().canonicalize()?!=args.output.canonicalize()?,"キャッシュ元と出力は分けてください");
    }
    let restore_pool=rayon::ThreadPoolBuilder::new().num_threads(args.workers).build()?;
    for q in 4..=12 {
        let done:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM metadata WHERE key=?1)",[format!("q{q}-complete")],|r|r.get(0))?;if done {continue;}
        status(args,"indexing-density",q,0,0,0,0,start)?;
        let mut density=Density::new(Job{q,x:0,y:0});let mut shared=HashSet::new();let mut unique=0u64;
        let mut query=input.prepare("SELECT x,y,mesh FROM basic WHERE q=?1 ORDER BY y,x")?;let mut rows=query.query([q])?;let mut keys=Vec::new();
        while let Some(row)=rows.next()? {let key=(row.get::<_,i64>(0)?,row.get::<_,i64>(1)?);let blob:Vec<u8>=row.get(2)?;let mut raw=Vec::new();ZlibDecoder::new(&blob[..]).read_to_end(&mut raw)?;let (p,_)=q_pool_bench::decode(&raw,density.stride as usize+1)?;
            for p in p {let world=(key.0*density.stride+p[0] as i64,key.1*density.stride+p[1] as i64);let edge=p[0]==0||p[1]==0||p[0]==density.stride as usize||p[1]==density.stride as usize;if !edge||shared.insert(world) {density.add(world);unique+=1;}}
            keys.push(key);if keys.len()%4096==0 {println!("全国密度 Q{q} {}タイル",keys.len());status(args,"indexing-density",q,keys.len(),0,0,0,start)?;}
        }drop(rows);drop(query);drop(shared);let budget=unique/5;
        let mut trace=db.prepare("SELECT x,y,sample FROM trace WHERE q=?1 ORDER BY seq")?;let mut rows=trace.query([q])?;let mut used=0u64;let mut versions=HashMap::<Key,u64>::new();
        while let Some(row)=rows.next()? {let key=(row.get::<_,i64>(0)?,row.get::<_,i64>(1)?);let sample:usize=row.get(2)?;density.add(density.world(key,sample,density.stride as usize+1));used+=1;for dy in -1..=1 {for dx in -1..=1 {*versions.entry((key.0+dx,key.1+dy)).or_default()+=1;}}}drop(rows);drop(trace);
        // 空き容量による枠はレベル開始時に決め、キャッシュ自身の消費で縮小させない。
        let cache_bytes=(8u64*1073741824).min(resources::memory()?*2/3) as usize;
        let mut heap=BinaryHeap::new();let mut cache=HashMap::<Key,Tile>::new();
        let prepare_started=Instant::now();
        let job=Job{q,x:0,y:0};
        let prepare_workers=args.workers.max(1).min((resources::memory()?/(job.memory()+512*1024*1024)).max(1) as usize);
        let mut pending=Vec::new();let mut prepared=0usize;
        for &key in &keys {
            let old:Option<f64>=db.query_row("SELECT score FROM candidates WHERE q=?1 AND x=?2 AND y=?3",params![q,key.0,key.1],|r|r.get(0)).optional()?;
            let bound:Option<f64>=input.query_row("SELECT max_error FROM basic_stats WHERE q=?1 AND x=?2 AND y=?3",params![q,key.0,key.1],|r|r.get::<_,f64>(0)).optional().unwrap_or(None).map(|e|e*1.5/job.error());
            if let Some(score)=old {if score>0.1 {heap.push(Candidate{score,key});}prepared+=1;}
            else if bound.is_some_and(|v|v<=0.1) {db.execute("INSERT INTO candidates VALUES(?1,?2,?3,?4)",params![q,key.0,key.1,bound.unwrap()])?;prepared+=1;}
            else {pending.push(key);}
        }
        status(args,"preparing-candidates",q,prepared,keys.len(),used,budget,start)?;
        // 共有カウンターから次のタイルを取得する。完了待ちのバッチ境界を設けない。
        // 結果キューはワーカー数までに制限し、計算中も主スレッドが逐次確定保存する。
        use std::sync::atomic::{AtomicUsize,AtomicBool,Ordering};
        let next=AtomicUsize::new(0);let cancel=AtomicBool::new(false);
        let stopped=std::thread::scope(|scope|->Result<bool> {
            let (send,receive)=mpsc::sync_channel::<Result<PreparedCandidate>>(prepare_workers);
            for _ in 0..prepare_workers.min(pending.len()) {
                let send=send.clone();let pending=&pending;let density=&density;let next=&next;let cancel=&cancel;
                scope.spawn(move|| {
                    let run=||->Result<()> {
                        let input=read(base)?;let output=read(&args.output)?;let mut source=Source::open(&args.source)?;let field_cache=args.field_cache_input.as_deref().map(read).transpose()?;
                        while !cancel.load(Ordering::Relaxed) {
                            let i=next.fetch_add(1,Ordering::Relaxed);let Some(&key)=pending.get(i) else {break};
                            let value=prepare_candidate(&mut source,&input,&output,Job{q,x:key.0,y:key.1},density,args.density_free_vertices,field_cache.as_ref())?;
                            if send.send(Ok(value)).is_err(){break;}
                        }Ok(())
                    };
                    if let Err(e)=run(){cancel.store(true,Ordering::Relaxed);let _=send.send(Err(e));}
                });
            }drop(send);
            let mut stopped=false;let mut failure=None;
            for result in receive {
                match result {
                    Ok(value) if failure.is_none()=> {
                        let save=||->Result<()> {let tx=db.unchecked_transaction()?;
                            if let Some(blob)=&value.fields {tx.execute("INSERT OR IGNORE INTO fields VALUES(?1,?2,?3,?4)",params![q,value.key.0,value.key.1,blob])?;}
                            if let Some(blob)=&value.fast {tx.execute("INSERT OR IGNORE INTO fields_fast VALUES(?1,?2,?3,?4)",params![q,value.key.0,value.key.1,blob])?;}
                            tx.execute("INSERT INTO candidates VALUES(?1,?2,?3,?4)",params![q,value.key.0,value.key.1,value.score])?;tx.commit()?;Ok(())};
                        if let Err(e)=save(){failure=Some(e);cancel.store(true,Ordering::Relaxed);continue;}
                        if value.score>0.1 {heap.push(Candidate{score:value.score,key:value.key});}prepared+=1;
                        let remaining=(keys.len()-prepared) as f64*prepare_started.elapsed().as_secs_f64()/prepared as f64;
                        if args.stop_file.as_ref().is_some_and(|p|p.exists())||time_exceeded(args.max_additional_seconds,start.elapsed().as_secs_f64())||(prepared>=128&&time_exceeded(args.max_additional_seconds,start.elapsed().as_secs_f64()+remaining)) {stopped=true;cancel.store(true,Ordering::Relaxed);}
                        if prepared%16==0||prepared==keys.len() {
                            let report=||->Result<()> {ensure!(resources::disk(args.output.parent().unwrap_or(Path::new(".")))? as f64>=args.min_free_gib*1073741824.,"追加工程の空き容量が不足しました");println!("全国追加候補 Q{q} {prepared}/{} 連続並列{prepare_workers}",keys.len());status(args,"preparing-candidates",q,prepared,keys.len(),used,budget,start)};
                            if let Err(e)=report(){failure=Some(e);cancel.store(true,Ordering::Relaxed);}
                        }
                    }
                    Err(e)=>{failure=Some(e);cancel.store(true,Ordering::Relaxed);}
                    _=>{}
                }
            }
            if let Some(e)=failure {return Err(e);}
            ensure!(stopped||prepared==keys.len(),"候補準備が途中で終了しました");Ok(stopped)
        })?;
        if stopped {status(args,"paused-for-optimization",q,prepared,keys.len(),used,budget,start)?;return Ok(());}
        // 全国キューへ進む前に読込用キャッシュをまとめて変換し、点ごとの復号を軽くする。
        let mut fields=db.prepare("SELECT f.x,f.y,f.data FROM fields f LEFT JOIN fields_fast z ON z.q=f.q AND z.x=f.x AND z.y=f.y WHERE f.q=?1 AND z.q IS NULL")?;
        let mut field_rows=fields.query([q])?;let mut warmed=0usize;db.execute_batch("BEGIN IMMEDIATE")?;
        while let Some(row)=field_rows.next()? {let body:Vec<u8>=row.get(2)?;let mut raw=Vec::new();ZlibDecoder::new(&body[..]).read_to_end(&mut raw)?;
            db.execute("INSERT OR IGNORE INTO fields_fast VALUES(?1,?2,?3,?4)",params![q,row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,zstd::stream::encode_all(&raw[..],1)?])?;warmed+=1;
            if warmed%512==0 {db.execute_batch("COMMIT;BEGIN IMMEDIATE")?;status(args,"preparing-read-cache",q,warmed,keys.len(),used,budget,start)?;}
        }drop(field_rows);drop(fields);db.execute_batch("COMMIT")?;
        db.execute_batch("BEGIN IMMEDIATE")?;let mut since_commit=0;let addition_started=Instant::now();let initial_used=used;let mut profile=[0f64;8];let mut loads=0u64;
        while used<budget {let Some(mut top)=heap.pop() else {break};if top.score<=0.1 {break;}
            if args.limit.is_some_and(|n|used-initial_used>=n as u64)||time_exceeded(args.max_additional_seconds,start.elapsed().as_secs_f64())||args.stop_file.as_ref().is_some_and(|p|p.exists()) {
                for (&key,tile) in &mut cache {flush(&db,q,key,tile)?;}db.execute_batch("COMMIT")?;status(args,"paused-for-optimization",q,keys.len(),keys.len(),used,budget,start)?;return Ok(());
            }
            let version=*versions.get(&top.key).unwrap_or(&0);
            let loaded_at=Instant::now();
            if !cache.contains_key(&top.key) {
                let capacity=(cache_bytes/(Job{q,x:0,y:0}.n().pow(2)*24+1048576)).clamp(1,16384);
                let mut batch=vec![top.key];let mut seen=HashSet::from([top.key]);
                // BinaryHeapのiterは順位順ではない。上位候補だけを並列復元する。
                let mut ranked=heap.iter().copied().collect::<Vec<_>>();ranked.sort_unstable_by(|a,b|b.cmp(a));
                for candidate in &ranked {if batch.len()>=capacity.min(args.workers.max(1)){break;}if !cache.contains_key(&candidate.key)&&seen.insert(candidate.key) {batch.push(candidate.key);}}
                // 次に使う順位が低いものから退避し、高優先度の常駐メッシュを保持する。
                let scores=ranked.iter().map(|c|(c.key,*c)).collect::<HashMap<_,_>>();
                while cache.len()+batch.len()>capacity {
                    let key=*cache.keys().min_by(|a,b| {
                        let ca=scores.get(a).copied().unwrap_or(Candidate{score:f64::NEG_INFINITY,key:**a});
                        let cb=scores.get(b).copied().unwrap_or(Candidate{score:f64::NEG_INFINITY,key:**b});ca.cmp(&cb)
                    }).unwrap();
                    let mut tile=cache.remove(&key).unwrap();flush(&db,q,key,&mut tile)?;
                }
                // SQLiteの読込は同じトランザクション内で行い、独立した復元だけを並列化する。
                let mut prepared=Vec::with_capacity(batch.len());
                for key in batch {let job=Job{q,x:key.0,y:key.1};ensure!(job.memory()<resources::memory()?*8/10,"追加工程の作業メモリーが不足しています");
                    let t=Instant::now();let (p,f,base)=load_mesh(&input,&db,q,key,job.n())?;let a=t.elapsed().as_secs_f64();let t=Instant::now();let (grid,limits)=cached_gain(&mut source,&db,job,field_cache.as_ref())?;let b=t.elapsed().as_secs_f64();
                    prepared.push((key,job,p,f,base,grid,limits,density.tile(key),*versions.get(&key).unwrap_or(&0),[a,b,0.]));
                }
                let step=density.step as usize;
                let restored=restore_pool.install(||prepared.into_par_iter().map(|(key,job,p,f,base,grid,limits,counts,version,mut parts)|->Result<_> {
                    let t=Instant::now();let builder=adaptive_tin::Builder::seeded_detail_owned_free(grid,job.n(),&p,&f,&limits,job.error(),step,counts,args.density_free_vertices)?;parts[2]=t.elapsed().as_secs_f64();
                    Ok((key,Tile{builder,base,version,dirty:false,load_parts:parts}))
                }).collect::<Vec<_>>());
                for result in restored {let (key,tile)=result?;loads+=1;for k in 0..3 {profile[k+5]+=tile.load_parts[k];}cache.insert(key,tile);}
            }
            profile[0]+=loaded_at.elapsed().as_secs_f64();let next_at=Instant::now();
            let tile=cache.get_mut(&top.key).unwrap();
            if tile.builder.detail_point_count()>=65535 {db.execute("UPDATE candidates SET score=0 WHERE q=?1 AND x=?2 AND y=?3",params![q,top.key.0,top.key.1])?;continue;}
            if tile.version!=version {tile.builder.replace_detail_density(density.tile(top.key))?;tile.version=version;}
            let (score,sample)=tile.builder.detail_next();profile[1]+=next_at.elapsed().as_secs_f64();
            if score<top.score-1e-12 {top.score=score;db.execute("UPDATE candidates SET score=?1 WHERE q=?2 AND x=?3 AND y=?4",params![score,q,top.key.0,top.key.1])?;if score>0.1 {heap.push(top);}continue;}
            let step_at=Instant::now();
            ensure!(tile.builder.detail_step()?,"確定した追加候補を挿入できません");profile[2]+=step_at.elapsed().as_secs_f64();tile.dirty=true;used+=1;density.add(density.world(top.key,sample,density.stride as usize+1));
            for dy in -1..=1 {for dx in -1..=1 {*versions.entry((top.key.0+dx,top.key.1+dy)).or_default()+=1;}}
            tile.version=*versions.get(&top.key).unwrap();let next_at=Instant::now();let next=tile.builder.detail_next().0;profile[1]+=next_at.elapsed().as_secs_f64();let db_at=Instant::now();
            db.execute("INSERT INTO trace VALUES(?1,?2,?3,?4,?5,?6)",params![q,used,top.key.0,top.key.1,sample,score])?;
            db.execute("UPDATE candidates SET score=?1 WHERE q=?2 AND x=?3 AND y=?4",params![next,q,top.key.0,top.key.1])?;if next>0.1 {heap.push(Candidate{score:next,key:top.key});}
            profile[3]+=db_at.elapsed().as_secs_f64();since_commit+=1;
            if since_commit>=128 {let flush_at=Instant::now();for (&key,tile) in &mut cache {flush(&db,q,key,tile)?;}db.execute_batch("COMMIT;BEGIN IMMEDIATE")?;since_commit=0;profile[4]+=flush_at.elapsed().as_secs_f64();fs::write(args.output.with_extension("profile.json"),serde_json::to_vec_pretty(&json!({"q":q,"inserted":used-initial_used,"loads":loads,"loadSeconds":profile[0],"meshReadSeconds":profile[5],"fieldReadSeconds":profile[6],"restoreSeconds":profile[7],"prioritySeconds":profile[1],"stepSeconds":profile[2],"sqlSeconds":profile[3],"flushSeconds":profile[4],"elapsedSeconds":addition_started.elapsed().as_secs_f64()}))?)?;println!("全国追加 Q{q} {used}/{budget}");status(args,"adding-vertices",q,keys.len(),keys.len(),used,budget,start)?;
                if used-initial_used>=1024 {let remaining=(budget-used) as f64*addition_started.elapsed().as_secs_f64()/(used-initial_used) as f64;
                    if time_exceeded(args.max_additional_seconds,start.elapsed().as_secs_f64()+remaining) {db.execute_batch("COMMIT")?;status(args,"paused-for-optimization",q,keys.len(),keys.len(),used,budget,start)?;println!("最適化のため停止: Q{q}残り推計 {:.1}時間",remaining/3600.);return Ok(());}
                }
            }
        }
        for (&key,tile) in &mut cache {flush(&db,q,key,tile)?;}
        let mut publish=db.prepare("SELECT x,y,vertices,triangles,mesh,ids FROM working WHERE q=?1")?;let mut rows=publish.query([q])?;
        while let Some(row)=rows.next()? {let body:Vec<u8>=row.get(4)?;let raw=unpack_working(&body)?;
            db.execute("INSERT OR REPLACE INTO additional VALUES(?1,?2,?3,?4,?5,?6,?7)",params![q,row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,usize>(2)?,row.get::<_,usize>(3)?,zip(&raw)?,row.get::<_,Vec<u8>>(5)?])?;
        }drop(rows);drop(publish);
        db.execute("UPDATE jobs SET state=CASE WHEN EXISTS(SELECT 1 FROM additional a WHERE a.q=jobs.q AND a.x=jobs.x AND a.y=jobs.y) THEN 1 ELSE 2 END WHERE q=?1",[q])?;
        db.execute("INSERT INTO metadata VALUES(?1,?2)",params![format!("q{q}-complete"),json!({"uniqueBasicVertices":unique,"budget":budget,"used":used}).to_string()])?;db.execute_batch("COMMIT")?;
        println!("全国追加 Q{q} 完了 {used}/{budget}");
    }
    db.execute("INSERT OR REPLACE INTO metadata VALUES('stage','additional-ready-for-boundary')",[])?;status(args,"additional-ready-for-boundary",12,0,0,0,0,start)?;Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn working_decoder_supports_both_generations() {
        let raw=b"YQB1-same-geometry";let old=zip(raw).unwrap();let mut new=b"YQW1".to_vec();new.extend(zstd::stream::encode_all(&raw[..],1).unwrap());assert_eq!(unpack_working(&old).unwrap(),raw);assert_eq!(unpack_working(&new).unwrap(),raw);assert_eq!(zip(&unpack_working(&new).unwrap()).unwrap(),old);
    }
    #[test] fn fast_field_codec_preserves_every_float_bit() {
        let values=[-100.25f64,0.,0.25,3776.,12.5,25.,50.,625.];let raw=values.iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>();let encoded=zstd::stream::encode_all(&raw[..],1).unwrap();let decoded=zstd::stream::decode_all(&encoded[..]).unwrap();let (a,b)=decode_fields(&decoded,2).unwrap();assert_eq!(a,values[..4]);assert_eq!(b,values[4..]);assert!(decode_fields(&decoded[..63],2).is_err());
    }
    #[test] fn density_crosses_tiles_and_is_monotone(){let mut d=Density::new(Job{q:4,x:0,y:0});d.add((64,32));let a=d.tile((0,0));let b=d.tile((1,0));assert_eq!(a[4*9+8],b[4*9]);let old=d.counts.clone();d.add((63,32));assert!(old.iter().all(|(k,v)|d.counts[k]>=*v));}
    #[test] fn queue_is_global_and_deterministic(){let mut heap=BinaryHeap::new();heap.push(Candidate{score:2.,key:(2,0)});heap.push(Candidate{score:3.,key:(8,0)});heap.push(Candidate{score:2.,key:(1,0)});assert_eq!(heap.pop().unwrap().key,(8,0));assert_eq!(heap.pop().unwrap().key,(1,0));}
}

#[cfg(test)] mod time_limit_tests {use super::*;#[test] fn zero_disables_elapsed_and_estimated_limits(){assert!(!time_exceeded(0.,f64::MAX));assert!(!time_exceeded(7200.,7199.));assert!(time_exceeded(7200.,7201.));}}
