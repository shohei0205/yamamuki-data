//! 全国共通点集合を誤差順に適合する。区間上界も使い全有効画素を監査する。
use anyhow::{Result,Context,ensure};
use std::{collections::{HashMap,VecDeque,BTreeSet},path::{Path,PathBuf},fs,sync::{Arc,Mutex},time::Instant};
use rusqlite::{Connection,OpenFlags,params,OptionalExtension};
use rayon::prelude::*;
use serde_json::json;
use serde::{Serialize,Deserialize};
use crate::{natural_catalog::{Catalog,Local,XY,Z,WORLD,quantize,point_id},natural_dem::{Source,STEP,load_lpf},natural_inventory::{hash,progress}};
const N:usize=1024;
const MARGIN:f64=0.02;
#[derive(Clone)] struct Candidate {q:[i32;2],score:f64,bounds:[f64;4]}
struct Scan {bx:u32,by:u32,maximum:f64,valid:u64,evaluated:u64,certified:u64,reused:u64,candidates:Vec<Candidate>,cache_path:PathBuf,cache_sha:String}
#[derive(Clone,Default,Serialize,Deserialize)]struct Cell {maximum:f64,valid:u64,evaluated:u64,certified:u64,q:[i32;2],score:f64}
pub struct LpfSampler {db:Connection,z:u32,blocks:HashMap<(i32,i32),Arc<Vec<f32>>>,order:VecDeque<(i32,i32)>}
impl LpfSampler {
 pub fn new(output:&Path,z:u32)->Result<Self>{Ok(Self{db:Connection::open_with_flags(output.join("work.sqlite3"),OpenFlags::SQLITE_OPEN_READ_ONLY)?,z,blocks:HashMap::new(),order:VecDeque::new()})}
 fn block(&mut self,x:i32,y:i32)->Result<Arc<Vec<f32>>> {
  if let Some(b)=self.blocks.get(&(x,y)){return Ok(b.clone());}
  let row:Option<(String,String)>=self.db.query_row("SELECT path,sha FROM artifacts WHERE stage='lpf' AND z=?1 AND x=?2 AND y=?3",params![self.z,x,y],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
  let v=if let Some((path,sha))=row {ensure!(hash(&fs::read(&path)?)==sha,"LPFキャッシュのハッシュ不一致です");let (n,_,data)=load_lpf(Path::new(&path))?;ensure!(n==N,"LPFの区画寸法が不一致です");data}else{vec![f32::NAN;N*N]};
  if self.blocks.len()>=32 {if let Some(k)=self.order.pop_front(){self.blocks.remove(&k);}}
  let v=Arc::new(v);self.order.push_back((x,y));self.blocks.insert((x,y),v.clone());Ok(v)
 }
 pub fn sample(&mut self,q:[i32;2])->Result<Option<f64>> {
  let x=q[0] as f64*XY/STEP-0.5;let y=q[1] as f64*XY/STEP-0.5;let ix=x.floor() as i32;let iy=y.floor() as i32;let a=x-ix as f64;let b=y-iy as f64;let mut sum=0.;let mut weight=0.;
  for (dx,dy,w) in [(0,0,(1.-a)*(1.-b)),(1,0,a*(1.-b)),(0,1,(1.-a)*b),(1,1,a*b)] {if w==0.{continue;}let x=ix+dx;let y=iy+dy;let values=self.block(x.div_euclid(N as i32),y.div_euclid(N as i32))?;let v=values[y.rem_euclid(N as i32) as usize*N+x.rem_euclid(N as i32) as usize];if v.is_finite(){sum+=v as f64*w;weight+=w;}}
  Ok((weight>1e-12).then_some(sum/weight))
 }
}
fn block_rect(x:u32,y:u32)->[f64;4] {[(x as f64*N as f64+0.5)*STEP,(y as f64*N as f64+0.5)*STEP,((x+1) as f64*N as f64-0.5)*STEP,((y+1) as f64*N as f64-0.5)*STEP]}
fn influence(local:&Local,q:[f64;2])->[f64;4] {
 let mut b=[WORLD,WORLD,0.,0.];
 for t in local.surface.faces_for_rect([q[0],q[1],q[0],q[1]]) {let c=local.surface.circles[t];let r=c[2].sqrt();b[0]=b[0].min(c[0]-r);b[1]=b[1].min(c[1]-r);b[2]=b[2].max(c[0]+r);b[3]=b[3].max(c[1]+r);}
 for v in &mut b {*v=v.clamp(0.,WORLD);}b
}
fn scan(c:&Catalog,source:&mut Source,z:u32,error:f64,bx:u32,by:u32,output:&Path,epoch:u64,force:bool)->Result<Scan> {
 let raw=source.raster(bx as i32*N as i32,by as i32*N as i32,N,N)?;
 let info:String=c.db.query_row("SELECT info FROM tasks WHERE stage='fit' AND z=?1 AND x=?2 AND y=?3",params![z,bx,by],|r|r.get(0))?;let info:serde_json::Value=serde_json::from_str(&info)?;
 let dirty:Option<Vec<[f64;3]>>=info.get("dirty_circles").map(|v|serde_json::from_value(v.clone())).transpose()?;
 let mut old=None;
 if !force && dirty.is_some() {
  let saved:Option<(String,String)>=c.db.query_row("SELECT path,sha FROM artifacts WHERE stage='scan-cells' AND z=?1 AND x=?2 AND y=?3",params![z,bx,by],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
  if let Some((path,sha))=saved {let blob=fs::read(path)?;ensure!(hash(&blob)==sha,"監査セルキャッシュのSHA-256が不一致です");let values:Vec<Cell>=serde_json::from_slice(&zstd::stream::decode_all(blob.as_slice())?)?;ensure!(values.len()==4096,"監査セル数が不正です");old=Some(values);}
 }
 let local=if raw.iter().any(|v|v.is_finite()){Some(c.local(z,block_rect(bx,by),250.)?)}else{None};
 let mut cells=vec![Cell::default();4096];let mut reused=0;let mut start=0;
 for cy in (0..N).step_by(16) {for cx in (0..N).step_by(16) {
  let index=cy/16*64+cx/16;let q0=[(bx as f64*N as f64+cx as f64+0.5)*STEP,(by as f64*N as f64+cy as f64+0.5)*STEP];let rect=[q0[0],q0[1],q0[0]+15.*STEP,q0[1]+15.*STEP];
  if let (Some(previous),Some(circles))=(&old,&dirty) {
   let changed=circles.iter().any(|c|{let dx=c[0]-c[0].clamp(rect[0],rect[2]);let dy=c[1]-c[1].clamp(rect[1],rect[3]);dx*dx+dy*dy<=c[2]*(1.+1e-12)+1e-6});
   if !changed {cells[index]=previous[index].clone();reused+=1;continue;}
  }
  let mut cell=Cell::default();for y in cy..cy+16 {for x in cx..cx+16 {cell.valid+=u64::from(raw[y*N+x].is_finite());}}
  if cell.valid==0 {continue;}
  let surface=&local.as_ref().context("有効画素の局所曲面がありません")?.surface;
  let center=[(rect[0]+rect[2])*0.5,(rect[1]+rect[3])*0.5];let (face,_)=surface.locate(center,start)?;start=face;
  let t=&surface.topology.triangles[3*face..3*face+3];let a=surface.points[t[0]];let b=surface.points[t[1]];let d=surface.points[t[2]];
  let ab=[b[0]-a[0],b[1]-a[1]];let ad=[d[0]-a[0],d[1]-a[1]];let det=ab[0]*ad[1]-ab[1]*ad[0];let gx=((b[2]-a[2])*ad[1]-(d[2]-a[2])*ab[1])/det;let gy=(ab[0]*(d[2]-a[2])-ad[0]*(b[2]-a[2]))/det;
  let plane=|x:f64,y:f64|a[2]+gx*(x-a[0])+gy*(y-a[1]);let mut lo=f64::INFINITY;let mut hi=f64::NEG_INFINITY;
  for f in surface.faces_for_rect(rect) {for &i in &surface.topology.triangles[3*f..3*f+3] {let p=surface.points[i];let e=p[2]-plane(p[0],p[1]);lo=lo.min(e);hi=hi.max(e);}}
  ensure!(lo.is_finite() && hi.is_finite(),"監査セルの支持点がありません");let mut raw_lo=f64::INFINITY;let mut raw_hi=f64::NEG_INFINITY;
  for y in cy..cy+16 {for x in cx..cx+16 {let v=raw[y*N+x];if v.is_finite(){let e=v as f64-plane(q0[0]+(x-cx) as f64*STEP,q0[1]+(y-cy) as f64*STEP);raw_lo=raw_lo.min(e);raw_hi=raw_hi.max(e);}}}
  let bound=(raw_lo-hi).abs().max((raw_hi-lo).abs())+MARGIN;
  if bound<=error {cell.certified=cell.valid;cell.maximum=bound;cells[index]=cell;continue;}
  for y in cy..cy+16 {for x in cx..cx+16 {let v=raw[y*N+x];if !v.is_finite(){continue;}let q=[(bx as f64*N as f64+x as f64+0.5)*STEP,(by as f64*N as f64+y as f64+0.5)*STEP];let e=(surface.evaluate(q,&mut start)?-v as f64).abs()+MARGIN;cell.evaluated+=1;cell.maximum=cell.maximum.max(e);if e>error && e>cell.score {cell.score=e;cell.q=[quantize(q[0],XY)?,quantize(q[1],XY)?];}}}
  cells[index]=cell;
 }}
 let mut result=Scan{bx,by,maximum:cells.iter().map(|v|v.maximum).fold(0.,f64::max),valid:cells.iter().map(|v|v.valid).sum(),evaluated:cells.iter().map(|v|v.evaluated).sum(),certified:cells.iter().map(|v|v.certified).sum(),reused,candidates:vec![],cache_path:output.join("scans").join(format!("z{z}")).join(format!("{bx}-{by}-e{epoch}-{}.nsc",if force{"full"}else{"local"})),cache_sha:String::new()};
 let mut candidates=cells.iter().filter(|c|c.score>error).collect::<Vec<_>>();candidates.sort_by(|a,b|b.score.total_cmp(&a.score).then(a.q.cmp(&b.q)));candidates.truncate(128);
 for c in candidates {result.candidates.push(Candidate{q:c.q,score:c.score,bounds:influence(local.as_ref().unwrap(),[c.q[0] as f64*XY,c.q[1] as f64*XY])});}
 let bytes=zstd::stream::encode_all(serde_json::to_vec(&cells)?.as_slice(),3)?;fs::create_dir_all(result.cache_path.parent().unwrap())?;let tmp=result.cache_path.with_extension("partial");fs::write(&tmp,&bytes)?;fs::rename(tmp,&result.cache_path)?;result.cache_sha=hash(&bytes);Ok(result)
}
fn meta(db:&Connection,key:&str)->Result<Option<String>> {Ok(db.query_row("SELECT value FROM metadata WHERE key=?1",[key],|r|r.get(0)).optional()?)}
fn set_meta(db:&Connection,key:&str,value:&str)->Result<()> {db.execute("INSERT OR REPLACE INTO metadata VALUES(?1,?2)",params![key,value])?;Ok(())}
fn seed(c:&Catalog,output:&Path,z:u32,sha:&Arc<HashMap<(u32,u32),String>>)->Result<()> {
 let key=format!("fit-seeded-z{z}");if meta(&c.db,&key)?.is_some(){return Ok(());}
 let mut lpf=LpfSampler::new(output,z)?;
 let tx=c.db.unchecked_transaction()?;c.guards(z)?;
 if z>8 {
  let mut s=c.db.prepare("SELECT p.qx,p.qy,l.flags FROM points p JOIN point_levels l ON l.id=p.id WHERE l.z=?1 ORDER BY CAST(p.qy/?2 AS INTEGER),CAST(p.qx/?2 AS INTEGER),p.id")?;
  for row in s.query_map(params![z-1,N as f64*STEP/XY],|r|Ok(([r.get::<_,i32>(0)?,r.get::<_,i32>(1)?],r.get::<_,u32>(2)?)))? {let (q,flags)=row?;if flags==1{continue;}
   let anchor=if flags==2 {let id=point_id(q)?;c.db.query_row("SELECT p.qx,p.qy FROM guard_anchors g JOIN points p ON p.id=g.anchor_id WHERE g.id=?1",[id],|r|Ok([r.get::<_,i32>(0)?,r.get::<_,i32>(1)?]))?}else{q};
   let h=lpf.sample(anchor)?.context("親制御点の元LPFが欠損しています")?;c.add(z,[q[0],q[1],quantize(h,Z)?],flags)?;}
 }else{
  let mut groups:HashMap<(u32,u32),Vec<(u32,u32)>>=HashMap::new();for &(x,y) in sha.keys(){groups.entry((x/16,y/16)).or_default().push((x,y));}
  let mut groups=groups.into_iter().collect::<Vec<_>>();groups.sort_by_key(|v|v.0);
  for (_,mut tiles) in groups {tiles.sort_unstable();for which in [tiles.len()/3,tiles.len()*2/3] {let (x,y)=tiles[which];for attempt in 0..64 {let ix=(attempt*137+73)%256;let iy=(attempt*197+149)%256;let q=[quantize((x as f64*256.+ix as f64+0.5)*STEP,XY)?,quantize((y as f64*256.+iy as f64+0.5)*STEP,XY)?];if let Some(h)=lpf.sample(q)? {c.add(z,[q[0],q[1],quantize(h,Z)?],0)?;break;}}}}
 }
 set_meta(&c.db,&key,"true")?;tx.commit()?;Ok(())
}
#[derive(Clone)]struct Bounds(rstar::AABB<[f64;2]>);
impl rstar::RTreeObject for Bounds {type Envelope=rstar::AABB<[f64;2]>;fn envelope(&self)->Self::Envelope{self.0}}
pub fn run(output:&Path,workers:usize)->Result<()> {
 ensure!(workers>0 && workers<=256,"並列数が不正です");let guard=Connection::open(output.join("run-lock.sqlite3"))?;guard.execute_batch("BEGIN EXCLUSIVE")?;
 let c=Catalog::open(output)?;for z in 8..=14 {ensure!(meta(&c.db,&format!("lpf-sigma-z{z}"))?.is_some(),"全国LPF監査が未完了です: z{z}");}
 c.db.execute_batch("CREATE TABLE IF NOT EXISTS natural_candidates(z INTEGER,bx INTEGER,by INTEGER,id INTEGER,qx INTEGER,qy INTEGER,score REAL,b0 REAL,b1 REAL,b2 REAL,b3 REAL,PRIMARY KEY(z,bx,by,id)) WITHOUT ROWID;
 CREATE INDEX IF NOT EXISTS candidate_priority ON natural_candidates(z,score DESC,id);")?;
 let identity:serde_json::Value=serde_json::from_str(&meta(&c.db,"identity")?.context("入力ID不明")?)?;let cache=PathBuf::from(identity["cache"].as_str().context("元PNGパス不明")?);
 let mut s=c.db.prepare("SELECT x,y,sha FROM inputs ORDER BY y,x")?;let sha=Arc::new(s.query_map([],|r|Ok(((r.get::<_,u32>(0)?,r.get::<_,u32>(1)?),r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<HashMap<_,_>>>()?);
 let blocks=sha.keys().map(|&(x,y)|(x/4,y/4)).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
 let threads=(0..workers).map(|_|Ok(Mutex::new((Catalog::readonly(output)?,Source::new(cache.clone(),sha.clone()))))).collect::<Result<Vec<_>>>()?;let pool=rayon::ThreadPoolBuilder::new().num_threads(workers).build()?;
 for (z,error) in [(8,800.),(9,400.),(10,200.),(11,100.),(12,50.),(13,25.),(14,12.5)] {
  let phase_key=format!("fit-phase-z{z}");if meta(&c.db,&phase_key)?.as_deref()==Some("complete"){continue;}
  seed(&c,output,z,&sha)?;let started=Instant::now();let mut epoch=meta(&c.db,&format!("fit-epoch-z{z}"))?.unwrap_or("0".into()).parse::<u64>()?;
  {let tx=c.db.unchecked_transaction()?;for &(x,y) in &blocks {c.db.execute("INSERT OR IGNORE INTO tasks VALUES('fit',?1,?2,?3,'pending','{}')",params![z,x,y])?;}tx.commit()?;}
  loop {
   let mut query=c.db.prepare("SELECT x,y FROM tasks WHERE stage='fit' AND z=?1 AND state='pending' ORDER BY y,x")?;
   let pending=query.query_map([z],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,u32>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;drop(query);
   let final_audit=meta(&c.db,&phase_key)?.as_deref()==Some("final-audit");let mut done=0;
   let points:u64=c.db.query_row("SELECT count(*) FROM point_levels WHERE z=?1",[z],|r|r.get(0))?;
   for batch in pending.chunks(workers) {
    let results=pool.install(||batch.par_iter().map(|&(bx,by)|{let i=rayon::current_thread_index().unwrap();let mut thread=threads[i].lock().map_err(|_|anyhow::anyhow!("作業キャッシュの排他エラー"))?;let (c,source)=&mut *thread;scan(c,source,z,error,bx,by,output,epoch,final_audit)}).collect::<Result<Vec<_>>>())?;
    let tx=c.db.unchecked_transaction()?;
    for result in results {c.db.execute("INSERT OR REPLACE INTO artifacts VALUES('scan-cells',?1,?2,?3,?4,?5)",params![z,result.bx,result.by,result.cache_path.to_string_lossy(),result.cache_sha])?;c.db.execute("DELETE FROM natural_candidates WHERE z=?1 AND bx=?2 AND by=?3",params![z,result.bx,result.by])?;
     for candidate in &result.candidates {let q=candidate.q;let b=candidate.bounds;c.db.execute("INSERT OR REPLACE INTO natural_candidates VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![z,result.bx,result.by,point_id(q)?,q[0],q[1],candidate.score,b[0],b[1],b[2],b[3]])?;}
     c.db.execute("UPDATE tasks SET state=?4,info=?5 WHERE stage='fit' AND z=?1 AND x=?2 AND y=?3",params![z,result.bx,result.by,if result.maximum<=error {"passed"}else{"needs-points"},json!({"maximum_error_bound":result.maximum,"valid":result.valid,"evaluated":result.evaluated,"certified":result.certified,"reused_cells":result.reused,"epoch":epoch}).to_string()])?;done+=1;
    }tx.commit()?;
    progress(output,json!({"stage":if final_audit{"final-quantized-audit"}else{"fit-global-controls"},"z":z,"error_m":error,"epoch":epoch,"done":done,"total":pending.len(),"points":points,"elapsed_seconds":started.elapsed().as_secs_f64(),"complete":false}))?;
   }
   let mut query=c.db.prepare("SELECT qx,qy,score,b0,b1,b2,b3 FROM natural_candidates WHERE z=?1 ORDER BY score DESC,id LIMIT 50000")?;
   let candidates=query.query_map([z],|r|Ok(Candidate{q:[r.get(0)?,r.get(1)?],score:r.get(2)?,bounds:[r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?]}))?.collect::<rusqlite::Result<Vec<_>>>()?;drop(query);
   if candidates.is_empty() {
    let failed:u64=c.db.query_row("SELECT count(*) FROM tasks WHERE stage='fit' AND z=?1 AND state<>'passed'",[z],|r|r.get(0))?;ensure!(failed==0,"誤差を超えていますが追加候補がありません");
    if final_audit {set_meta(&c.db,&format!("point-count-z{z}"),&points.to_string())?;set_meta(&c.db,&phase_key,"complete")?;break;}
    let tx=c.db.unchecked_transaction()?;set_meta(&c.db,&phase_key,"final-audit")?;c.db.execute("UPDATE tasks SET state='pending' WHERE stage='fit' AND z=?1",[z])?;tx.commit()?;continue;
   }
   let mut selected=Vec::new();let mut footprints=rstar::RTree::new();let mut unique=BTreeSet::new();
   for p in candidates {let b=p.bounds;let envelope=rstar::AABB::from_corners([b[0],b[1]],[b[2],b[3]]);if unique.insert(p.q) && footprints.locate_in_envelope_intersecting(&envelope).next().is_none(){footprints.insert(Bounds(envelope));selected.push(p);if selected.len()==2048{break;}}}
   let mut lpf=LpfSampler::new(output,z)?;let mut added=0;let mut inserted=Vec::new();
   {let tx=c.db.unchecked_transaction()?;
    let stalled=selected.iter().find(|v|c.db.query_row("SELECT 1 FROM point_levels WHERE z=?1 AND id=?2",params![z,point_id(v.q).unwrap()],|_|Ok(())).optional().ok().flatten().is_some());
    if let Some(candidate)=stalled {
     // 凸包側の細長いVoronoiセルでは、量子化の数cmで重みが大きく変わる。
     // 周囲の元画素位置へ支持点を足す。欠損側だけは中心点の高さで延長する。
     let anchor=point_id(candidate.q)?;let center=[(candidate.q[0] as f64*XY/STEP-0.5).round() as i32,(candidate.q[1] as f64*XY/STEP-0.5).round() as i32];
     for radius in 1i32..=4 {
      for dy in -radius..=radius {for dx in -radius..=radius {if dx.abs()!=radius && dy.abs()!=radius {continue;}
       let q=[quantize((center[0]+dx) as f64*STEP+STEP*0.5,XY)?,quantize((center[1]+dy) as f64*STEP+STEP*0.5,XY)?];
       let id=point_id(q)?;let exists:bool=c.db.query_row("SELECT 1 FROM point_levels WHERE z=?1 AND id=?2",params![z,id],|_|Ok(())).optional()?.is_some();if exists{continue;}
       let direct=lpf.sample(q)?;let (h,flags,anchor_id)=if let Some(h)=direct {(h,0,anchor)}else{
        let old_anchor:Option<i64>=c.db.query_row("SELECT anchor_id FROM guard_anchors WHERE id=?1",[id],|r|r.get(0)).optional()?;let owner=old_anchor.unwrap_or(anchor);let aq=c.db.query_row("SELECT qx,qy FROM points WHERE id=?1",[owner],|r|Ok([r.get::<_,i32>(0)?,r.get::<_,i32>(1)?]))?;(lpf.sample(aq)?.context("欠損側の支持点の参照元がありません")?,2,owner)
       };
       if c.add(z,[q[0],q[1],quantize(h,Z)?],flags)? {added+=1;inserted.push(q);if flags==2{c.db.execute("INSERT OR IGNORE INTO guard_anchors VALUES(?1,?2)",params![id,anchor_id])?;}}
      }}if added>0{break;}
     }
    }else{for candidate in &selected {let q=candidate.q;let h=lpf.sample(q)?.context("追加点の元LPFが欠損しています")?;if c.add(z,[q[0],q[1],quantize(h,Z)?],0)?{added+=1;inserted.push(q);}}}
    ensure!(added>0,"誤差を超えたまま追加可能な点がありません。支持点の補修も上限まで試しました");tx.commit()?;}
   let mut dirty:HashMap<(u32,u32),Vec<[f64;3]>>=HashMap::new();
   for inserted_q in inserted {let q=[inserted_q[0] as f64*XY,inserted_q[1] as f64*XY];let local=c.local(z,[q[0],q[1],q[0],q[1]],250.)?;
    let vertex=local.values.iter().position(|v|v[0]==inserted_q[0] && v[1]==inserted_q[1]).context("追加点が局所曲面にありません")?;
    for &face in &local.surface.incident[vertex] {let circle=local.surface.circles[face];for &(x,y) in &blocks {let rect=block_rect(x,y);let dx=circle[0]-circle[0].clamp(rect[0],rect[2]);let dy=circle[1]-circle[1].clamp(rect[1],rect[3]);if dx*dx+dy*dy<=circle[2]*(1.+1e-12)+1e-6 {dirty.entry((x,y)).or_default().push(circle);}}}
   }
   ensure!(!dirty.is_empty(),"追加点の影響領域がありません");epoch+=1;
   {let tx=c.db.unchecked_transaction()?;for (&(x,y),circles) in &dirty {c.db.execute("UPDATE tasks SET state='pending',info=?4 WHERE stage='fit' AND z=?1 AND x=?2 AND y=?3",params![z,x,y,json!({"dirty_circles":circles}).to_string()])?;c.db.execute("DELETE FROM natural_candidates WHERE z=?1 AND bx=?2 AND by=?3",params![z,x,y])?;}set_meta(&c.db,&phase_key,"fitting")?;set_meta(&c.db,&format!("fit-epoch-z{z}"),&epoch.to_string())?;tx.commit()?;}
   progress(output,json!({"stage":"insert-controls","z":z,"epoch":epoch,"added":added,"dirty_blocks":dirty.len(),"complete":false}))?;
  }
 }
 progress(output,json!({"stage":"controls-complete","next_stage":"pack-rice","complete":false,"distributable":false}))?;Ok(())
}
