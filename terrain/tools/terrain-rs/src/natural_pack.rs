//! XYZタイルへの支持点切出し、整数差分・Rice保存と独立復元検査。
use anyhow::{Result,Context,ensure};
use rusqlite::{Connection,OpenFlags,params,OptionalExtension};
use sha2::{Digest,Sha256};
use serde_json::{Value,json};
use std::{collections::{BTreeSet,HashMap,VecDeque},fs,path::Path,time::Instant,sync::Mutex};
use crate::{natural_catalog::{Catalog,XY,Z,WORLD,quantize,point_id},natural_codec,natural_inventory::{hash,progress}};
use rayon::prelude::*;
type Key=(u32,u32,u32);
struct Prepared {key:Key,count:usize,owned:usize,min:i32,max:i32,dependencies:usize,sha:Vec<u8>,body:Vec<u8>,parents:Vec<Key>}
fn origin(k:Key)->Result<[i32;2]>{let w=WORLD/(1u64<<k.0) as f64;Ok([quantize(k.1 as f64*w,XY)?,quantize(k.2 as f64*w,XY)?])}
fn digest(points:&[[i32;3]])->Vec<u8> {let mut h=Sha256::new();for p in points{for v in p{h.update(v.to_le_bytes());}}h.finalize().to_vec()}
fn bytes(hex:&str)->Result<Vec<u8>> {ensure!(hex.len()==64,"SHA-256の長さが不正です");(0..64).step_by(2).map(|i|Ok(u8::from_str_radix(&hex[i..i+2],16)?)).collect()}
fn morton(x:u32,y:u32)->u64 {let mut out=0;for i in 0..32 {out|=((x as u64>>i)&1)<<(2*i);out|=((y as u64>>i)&1)<<(2*i+1);}out}
fn owned(p:[i32;3],k:Key)->bool {let w=WORLD/(1u64<<k.0) as f64;let x=p[0] as f64*XY;let y=p[1] as f64*XY;x>=k.1 as f64*w && y>=k.2 as f64*w && x<(k.1+1) as f64*w && y<(k.2+1) as f64*w}
fn parents(db:&Connection,k:Key)->Result<Vec<Key>> {let mut s=db.prepare_cached("SELECT parent_z,parent_x,parent_y FROM tile_parents WHERE z=?1 AND x=?2 AND y=?3 ORDER BY slot")?;Ok(s.query_map(params![k.0,k.1,k.2],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)}
fn ancestry(db:&Connection,k:Key,seen:&mut BTreeSet<Key>,stack:&mut BTreeSet<Key>)->Result<usize>{ensure!(stack.insert(k),"親参照が循環しています");seen.insert(k);ensure!(seen.len()<=16,"依存タイル数が16を超えました");let mut depth=0;for p in parents(db,k)? {ensure!(p.0+1==k.0,"親の品質が不正です");depth=depth.max(1+ancestry(db,p,seen,stack)?);}stack.remove(&k);ensure!(depth<=2,"親深度が2を超えました");Ok(depth)}
pub struct Loader {cache:HashMap<Key,Vec<[i32;3]>>,order:VecDeque<Key>,bytes:usize}
impl Loader {
 pub fn new()->Self{Self{cache:HashMap::new(),order:VecDeque::new(),bytes:0}}
 pub fn restore(&mut self,db:&Connection,k:Key)->Result<Vec<[i32;3]>> {self.inner(db,k,&mut BTreeSet::new())}
 fn inner(&mut self,db:&Connection,k:Key,stack:&mut BTreeSet<Key>)->Result<Vec<[i32;3]>> {
  if let Some(p)=self.cache.get(&k){return Ok(p.clone());}ensure!(stack.insert(k) && stack.len()<=3,"親の循環・深度超過です");
  let (blob,expected,n,generation):(Vec<u8>,Vec<u8>,usize,u64)=db.query_row("SELECT payload,xyz_sha256,point_count,generation_key FROM tiles WHERE z=?1 AND x=?2 AND y=?3",params![k.0,k.1,k.2],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;ensure!(generation==1,"生成世代が不一致です");
  let ps=parents(db,k)?;ensure!(ps.len()<=4 && ps.iter().all(|p|p.0+1==k.0),"親定義が不正です");let values=ps.into_iter().map(|p|self.inner(db,p,stack)).collect::<Result<Vec<_>>>()?;
  let points=natural_codec::decode_global(&blob,origin(k)?,&values)?;ensure!(points.len()==n && digest(&points)==expected,"整数XYZの点数・SHA-256が不一致です");stack.remove(&k);
  let size=points.len()*12;while self.bytes+size>256*1024*1024 {if let Some(key)=self.order.pop_front(){if let Some(p)=self.cache.remove(&key){self.bytes-=p.len()*12;}}else{break;}}
  self.bytes+=size;self.order.push_back(k);self.cache.insert(k,points.clone());Ok(points)
 }
}
fn metadata(db:&Connection,key:&str,value:Value)->Result<()> {db.execute("INSERT OR REPLACE INTO metadata VALUES(?1,?2)",params![key,value.to_string()])?;Ok(())}
fn exists(db:&Connection,k:Key)->Result<bool>{Ok(db.query_row("SELECT 1 FROM tiles WHERE z=?1 AND x=?2 AND y=?3",params![k.0,k.1,k.2],|_|Ok(())).optional()?.is_some())}
fn prepare_tile(c:&Catalog,db:&Connection,loader:&mut Loader,k:Key,parent_z:Option<u32>)->Result<Prepared> {
 let w=WORLD/(1u64<<k.0) as f64;let rect=[(k.1 as f64*w-251.).max(0.),(k.2 as f64*w-251.).max(0.),((k.1+1) as f64*w+251.).min(WORLD),((k.2+1) as f64*w+251.).min(WORLD)];
 let local=c.local(k.0,rect,250.)?;let mut points=local.surface.support_vertices(rect).into_iter().map(|i|local.values[i]).collect::<Vec<_>>();
 points.sort_by_key(|&p|(!owned(p,k),morton(p[0] as u32,p[1] as u32),p[2]));points.dedup();ensure!(!points.is_empty() && points.len()<=262144,"支持点込みで1タイル262144点を超えました");
 let owned_count=points.iter().take_while(|&&p|owned(p,k)).count();let o=origin(k)?;
 let local_ints=points.iter().map(|p|Ok([p[0].checked_sub(o[0]).context("局所原点のオーバーフロー")?,p[1].checked_sub(o[1]).context("局所原点のオーバーフロー")?,p[2]])).collect::<Result<Vec<_>>>()?;
 let independent=natural_codec::independent(&local_ints)?;let mut body=independent.clone();let mut selected:Vec<(Key,Vec<[i32;3]>)>=Vec::new();let mut dependency_count=1;
 if let Some(pz)=parent_z {
  let px=k.1/2;let py=k.2/2;let owner=(pz,px,py);let mut candidates=Vec::new();
  for y in py.saturating_sub(1)..=(py+1).min((1<<pz)-1){for x in px.saturating_sub(1)..=(px+1).min((1<<pz)-1){let key=(pz,x,y);if exists(db,key)? {candidates.push((key,loader.restore(db,key)?));}}}
  let ids=points.iter().map(|p|point_id([p[0],p[1]])).collect::<Result<BTreeSet<_>>>()?;let mut covered=BTreeSet::new();
  for _ in 0..4 {
   let best=candidates.iter().enumerate().map(|(i,(key,ps))|{let score=ps.iter().filter(|p|{let id=point_id([p[0],p[1]]).unwrap();ids.contains(&id) && !covered.contains(&id)}).count();(i,score,*key==owner,key.2,key.1)}).filter(|v|v.1>0).max_by_key(|v|(v.2,v.1,std::cmp::Reverse(v.3),std::cmp::Reverse(v.4)));
   if let Some((i,_,_,_,_))=best {let v=candidates.remove(i);for p in &v.1{covered.insert(point_id([p[0],p[1]])?);}selected.push(v);}else{break;}
  }
  selected.sort_by_key(|v|(v.0.0,v.0.2,v.0.1));
  if !selected.is_empty() {
   let mut deps=BTreeSet::from([k]);let mut depth=0;let mut permitted=true;
   for (p,_) in &selected {match ancestry(db,*p,&mut deps,&mut BTreeSet::new()){Ok(v)=>depth=depth.max(v+1),Err(_)=>{permitted=false;break;}}}
   permitted &= deps.len()<=16 && depth<=2;
   if permitted {
    let maps=selected.iter().map(|(_,p)|p.iter().enumerate().map(|(i,p)|(point_id([p[0],p[1]]).unwrap(),i)).collect::<HashMap<_,_>>()).collect::<Vec<_>>();
    let owning=selected.iter().position(|p|p.0==owner);
    let refs=points.iter().map(|p|{let id=point_id([p[0],p[1]]).unwrap();if let Some(slot)=owning {if let Some(&i)=maps[slot].get(&id){return Some((slot,i));}}maps.iter().enumerate().find_map(|(slot,m)|m.get(&id).map(|&i|(slot,i)))}).collect::<Vec<_>>();
    let parent_values=selected.iter().map(|v|v.1.clone()).collect::<Vec<_>>();let delta=natural_codec::delta(&local_ints,&refs,&parent_values)?;
    if delta.len()+12*selected.len()<independent.len(){body=delta;dependency_count=deps.len();}else{selected.clear();}
   }else{selected.clear();}
  }
 }
 let parent_values=selected.iter().map(|v|v.1.clone()).collect::<Vec<_>>();let restored=natural_codec::decode_global(&body,o,&parent_values)?;ensure!(restored==points,"圧縮前後の正規整数が不一致です");
 let subset=crate::natural_surface::Surface::new(restored.iter().map(|p|[p[0] as f64*XY,p[1] as f64*XY,p[2] as f64*Z]).collect())?;
 for y in 0..=4 {for x in 0..=4 {let q=[rect[0]+(rect[2]-rect[0])*x as f64/4.,rect[1]+(rect[3]-rect[1])*y as f64/4.];ensure!((subset.evaluate(q,&mut 0)?-local.surface.evaluate(q,&mut 0)?).abs()<0.001,"支持点切出しで曲面が変化しました");}}
 let min=points.iter().map(|p|p[2]).min().unwrap();let max=points.iter().map(|p|p[2]).max().unwrap();
 Ok(Prepared{key:k,count:points.len(),owned:owned_count,min,max,dependencies:dependency_count,sha:digest(&points),body,parents:selected.into_iter().map(|v|v.0).collect()})
}
fn write_tile(db:&Connection,p:Prepared)->Result<()> {
 let k=p.key;let tx=db.unchecked_transaction()?;db.execute("INSERT INTO tiles VALUES(?1,?2,?3,1,?4,?5,?6,?7,?8,?9,?10)",params![k.0,k.1,k.2,p.count,p.owned,p.min,p.max,p.dependencies,p.sha,p.body])?;
 for (slot,parent) in p.parents.iter().enumerate(){db.execute("INSERT INTO tile_parents VALUES(?1,?2,?3,?4,?5,?6,?7)",params![k.0,k.1,k.2,slot,parent.0,parent.1,parent.2])?;}tx.commit()?;Ok(())
}
fn coverage(c:&Catalog,db:&Connection,output:&Path)->Result<u64>{
 let proof_path=output.join("coverage-source.sqlite3");let proof=if proof_path.exists(){Some(Connection::open_with_flags(proof_path,OpenFlags::SQLITE_OPEN_READ_ONLY)?)}else{None};
 let mut query=c.db.prepare("SELECT x,y,valid FROM inputs ORDER BY y,x")?;let input=query.query_map([],|r|Ok(((r.get::<_,u32>(0)?,r.get::<_,u32>(1)?),r.get::<_,u32>(2)?)))?.collect::<rusqlite::Result<HashMap<_,_>>>()?;
 let roots=input.keys().map(|&(x,y)|(x>>6,y>>6)).collect::<BTreeSet<_>>();let mut unknown=0;
 for &(x,y) in &roots {let mut cells=vec![2u8;4096];
  for dy in 0..64 {for dx in 0..64 {let key=(x*64+dx,y*64+dy);let value=if let Some(&n)=input.get(&key){u8::from(n>0)}else if let Some(p)=&proof {let state:Option<String>=p.query_row("SELECT state FROM cells WHERE x=?1 AND y=?2",params![key.0,key.1],|r|r.get(0)).optional()?;if state.as_deref()==Some("verified404"){0}else{2}}else{2};cells[(dy*64+dx) as usize]=value;unknown+=u64::from(value==2);}}
  let integers=cells.iter().map(|&v|v as i64).collect::<Vec<_>>();let encoded=natural_codec::stream(&integers)?;let mut body=b"NCV1".to_vec();body.extend([8,14,0,0]);body.extend(4096u32.to_le_bytes());body.extend((encoded.len() as u32).to_le_bytes());body.extend(encoded);
  db.execute("INSERT OR REPLACE INTO coverage VALUES(?1,?2,1,?3,?4)",params![x,y,Sha256::digest(&cells).to_vec(),body])?;
 }metadata(db,"coverage_roots",json!(roots))?;metadata(db,"coverage_unresolved",json!(unknown))?;Ok(unknown)
}
pub fn run(output:&Path,workers:usize)->Result<()> {
 ensure!(workers>0 && workers<=256,"並列数が不正です");
 let guard=Connection::open(output.join("run-lock.sqlite3"))?;guard.execute_batch("BEGIN EXCLUSIVE")?;let c=Catalog::open(output)?;
 for z in 8..=14 {let phase:String=c.db.query_row("SELECT value FROM metadata WHERE key=?1",[format!("fit-phase-z{z}")],|r|r.get(0)).context("制御点の最終監査が未完了です")?;ensure!(phase=="complete","制御点の最終監査が未完了です: z{z}");}
 let settings=fs::read(output.join("settings.json"))?;let config:Value=serde_json::from_slice(&settings)?;
 let identity:String=c.db.query_row("SELECT value FROM metadata WHERE key='identity'",[],|r|r.get(0))?;let identity:Value=serde_json::from_str(&identity)?;ensure!(Some(hash(&settings).as_str())==identity["settings_sha256"].as_str(),"固定済み生成設定が変更されています");
 let source_hash:String=c.db.query_row("SELECT value FROM metadata WHERE key='input_manifest_sha256'",[],|r|r.get(0))?;let config_hash=hash(&settings);
 let mut mixed=bytes(&config_hash)?;mixed.extend(bytes(&source_hash)?);let id_hash=hash(&mixed);
 let path=output.join("distribution.partial.sqlite3");let was_existing=path.exists();let db=Connection::open(&path)?;
 if !was_existing {db.execute_batch(include_str!("../../terrain/national_natural.schema.sql"))?;db.execute_batch("PRAGMA journal_mode=WAL")?;
  let utc:String=db.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')",[],|r|r.get(0))?;let stamp:String=db.query_row("SELECT strftime('%Y%m%dT%H%M%f','now')||'000Z'",[],|r|r.get(0))?;let id=format!("nt1:{stamp}:{id_hash}");
  let tx=db.unchecked_transaction()?;db.execute("INSERT INTO generations VALUES(1,?1,?2,?3,?4)",params![id,bytes(&config_hash)?,bytes(&source_hash)?,utc])?;
  for l in config["levels"].as_array().context("品質設定が不正です")?{db.execute("INSERT INTO levels VALUES(?1,?2,?3)",params![l["z"].as_u64(),l["error_m"].as_f64(),l["parent_z"].as_u64()])?;}
  for (key,v) in [("format",json!("YNN")),("schema_version",json!(1)),("generation_key",json!(1)),("complete",json!(false)),("distributable",json!(false)),("crs",config["crs"].clone()),("quantization",config["quantization"].clone()),("codec_profile",config["codec"].clone()),("xyz_hash_mode",json!("sha256-le-i32-global-xyz-ordered")),("attribution",json!({"source":"国土地理院 地理院タイル DEM10B","url":"https://maps.gsi.go.jp/development/ichiran.html"})),("source_manifest",json!({"version":1,"source_z":14,"input_manifest_sha256":source_hash,"kind":"gsi-dem-png"}))] {metadata(&db,key,v)?;}
  db.execute("INSERT INTO metadata VALUES('generation_settings',?1)",[String::from_utf8(settings.clone())?])?;tx.commit()?;
 }else{let (ch,sh):(Vec<u8>,Vec<u8>)=db.query_row("SELECT config_sha256,source_sha256 FROM generations WHERE generation_key=1",[],|r|Ok((r.get(0)?,r.get(1)?)))?;ensure!(ch==bytes(&config_hash)? && sh==bytes(&source_hash)?,"既存の配布作業DBは異なる生成世代です");}
 db.execute_batch("PRAGMA foreign_keys=ON")?;
 let mut sigmas=serde_json::Map::new();for z in 8..=14 {let sigma:String=c.db.query_row("SELECT value FROM metadata WHERE key=?1",[format!("lpf-sigma-z{z}")],|r|r.get(0))?;sigmas.insert(z.to_string(),json!(sigma.parse::<f64>()?));}metadata(&db,"interpolation",json!({"algorithm":"sibson-voronoi-area-v1","sigma_m":sigmas,"audit":"all-valid-source-pixel-centres-after-quantization","audit_method":"positive-area-affine-residual-intervals-or-direct-sibson","numerical_margin_m":0.02,"hull_stabilization":"source-pixel-support-ring-and-missing-side-anchor-extension"}))?;
 metadata(&db,"generator_runs",json!([{"implementation":"terrain-natural-national-rust-v1","packing_executable_sha256":hash(&fs::read(std::env::current_exe()?)?)}]))?;
 let input=c.db.prepare("SELECT x,y FROM inputs WHERE valid>0 ORDER BY y,x")?.query_map([],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,u32>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;let threads=(0..workers).map(|_|Ok(Mutex::new((Catalog::readonly(output)?,Connection::open_with_flags(&path,OpenFlags::SQLITE_OPEN_READ_ONLY)?,Loader::new())))).collect::<Result<Vec<_>>>()?;let pool=rayon::ThreadPoolBuilder::new().num_threads(workers).build()?;let started=Instant::now();let mut total_done=0;let mut counts=serde_json::Map::new();
 for z in 8..=14 {let keys=input.iter().map(|&(x,y)|(z,x>>(14-z),y>>(14-z))).collect::<BTreeSet<_>>();let parent_z=match z{10=>Some(9),11=>Some(10),13=>Some(12),14=>Some(13),_=>None};let total=keys.len();let mut done=0;
  let mut pending=Vec::new();for k in keys {if !exists(&db,k)? {pending.push(k);}else{done+=1;total_done+=1;}}
  for batch in pending.chunks(workers*2) {
   let prepared=pool.install(||batch.par_iter().map(|&k|{let i=rayon::current_thread_index().unwrap();let mut worker=threads[i].lock().map_err(|_|anyhow::anyhow!("圧縮作業の排他エラー"))?;let (c,parent_db,loader)=&mut *worker;prepare_tile(c,parent_db,loader,k,parent_z)}).collect::<Result<Vec<_>>>())?;
   for p in prepared {write_tile(&db,p)?;done+=1;total_done+=1;}
   progress(output,json!({"stage":"pack-rice","z":z,"done":done,"total":total,"total_done":total_done,"workers":workers,"elapsed_seconds":started.elapsed().as_secs_f64(),"complete":false}))?;
  }counts.insert(z.to_string(),json!(total));
 }
 metadata(&db,"tile_counts",json!(counts))?;let unknown=coverage(&c,&db,output)?;db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
 progress(output,json!({"stage":"packed-awaiting-verification","tiles":total_done,"coverage_unresolved":unknown,"complete":false,"distributable":false}))?;Ok(())
}
