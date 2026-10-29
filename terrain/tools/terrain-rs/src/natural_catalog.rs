//! 全品質の正規XYとZ。支持集合は全球点索引との照合で閉じる。
use anyhow::{Result,ensure};
use rusqlite::{Connection,params,OptionalExtension};
use std::{path::Path,collections::{BTreeMap,HashSet}};
use crate::{natural_inventory::open_work,natural_surface::Surface};
pub const XY:f64=0.0625;pub const Z:f64=0.125;pub const WORLD:f64=40075016.68557849;
pub fn point_id(q:[i32;2])->Result<i64> {ensure!(q.iter().all(|&v|v>=0 && v<=641200267),"全球整数座標が範囲外です");Ok(((q[0] as i64)<<32)|q[1] as i64)}
pub fn quantize(v:f64,step:f64)->Result<i32> {let x=(v/step+0.5).floor();ensure!(x.is_finite() && x>=i32::MIN as f64 && x<=i32::MAX as f64,"量子化がint32範囲外です");Ok(x as i32)}
pub struct Catalog {pub db:Connection}
impl Catalog {
 pub fn readonly(output:&Path)->Result<Self>{Ok(Self{db:Connection::open_with_flags(output.join("work.sqlite3"),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?})}

 pub fn open(output:&Path)->Result<Self> {
  let db=open_work(output)?;db.execute_batch("CREATE TABLE IF NOT EXISTS points(id INTEGER PRIMARY KEY,qx INTEGER NOT NULL,qy INTEGER NOT NULL);
  CREATE VIRTUAL TABLE IF NOT EXISTS point_index USING rtree_i32(id,min_x,max_x,min_y,max_y);
  CREATE TABLE IF NOT EXISTS point_levels(z INTEGER,id INTEGER REFERENCES points(id),qz INTEGER NOT NULL,flags INTEGER NOT NULL,PRIMARY KEY(z,id)) WITHOUT ROWID;
  CREATE INDEX IF NOT EXISTS point_level_world_guards ON point_levels(z,id) WHERE flags=1;
  CREATE TABLE IF NOT EXISTS guard_anchors(id INTEGER PRIMARY KEY REFERENCES points(id),anchor_id INTEGER NOT NULL REFERENCES points(id));")?;Ok(Self{db})
 }
 pub fn add(&self,z:u32,q:[i32;3],flags:u32)->Result<bool> {
  let id=point_id([q[0],q[1]])?;let old:Option<i32>=self.db.query_row("SELECT qz FROM point_levels WHERE z=?1 AND id=?2",params![z,id],|r|r.get(0)).optional()?;
  if let Some(old)=old {ensure!(old==q[2],"同じ品質・XYの高さが一致しません");return Ok(false);}
  self.db.execute("INSERT OR IGNORE INTO points VALUES(?1,?2,?3)",params![id,q[0],q[1]])?;
  self.db.execute("INSERT OR IGNORE INTO point_index VALUES(?1,?2,?2,?3,?3)",params![id,q[0],q[1]])?;
  self.db.execute("INSERT INTO point_levels VALUES(?1,?2,?3,?4)",params![z,id,q[2],flags])?;Ok(true)
 }
 pub fn guards(&self,z:u32)->Result<()> {let m=quantize(WORLD,XY)?;for (x,y) in [(0,0),(m,0),(0,m),(m,m)]{self.add(z,[x,y,0],1)?;}Ok(())}
 pub fn query(&self,z:u32,rect:[f64;4])->Result<Vec<(i64,[i32;3],u32)>> {
  let q=rect.map(|v|(v/XY).clamp(0.,641200267.) as i64);
  let mut s=self.db.prepare_cached("SELECT p.id,p.qx,p.qy,l.qz,l.flags FROM point_index r CROSS JOIN points p CROSS JOIN point_levels l WHERE p.id=r.id AND l.id=p.id AND l.z=?1 AND r.min_x<=?4 AND r.max_x>=?2 AND r.min_y<=?5 AND r.max_y>=?3 ORDER BY p.id")?;
  Ok(s.query_map(params![z,q[0]-2,q[1]-2,q[2]+2,q[3]+2],|r|Ok((r.get(0)?,[r.get(1)?,r.get(2)?,r.get(3)?],r.get(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
 }
 pub fn local(&self,z:u32,rect:[f64;4],halo:f64)->Result<Local> {
  let initial=[rect[0]-halo,rect[1]-halo,rect[2]+halo,rect[3]+halo];
  let mut rows=self.query(z,initial)?.into_iter().map(|r|(r.0,(r.1,r.2))).collect::<BTreeMap<_,_>>();
  let mut s=self.db.prepare("SELECT p.id,p.qx,p.qy,l.qz,l.flags FROM points p JOIN point_levels l ON l.id=p.id WHERE l.z=?1 AND l.flags=1")?;
  for r in s.query_map([z],|r|Ok((r.get::<_,i64>(0)?,([r.get::<_,i32>(1)?,r.get(2)?,r.get(3)?],r.get::<_,u32>(4)?))))? {let (id,v)=r?;rows.insert(id,v);}
  let mut checked=HashSet::new();
  for _ in 0..128 {
   ensure!(rows.len()<=4_000_000,"支持集合が内部メモリー上限を超えました。点を切り捨てず停止します");
   let ids=rows.keys().copied().collect::<Vec<_>>();let values=rows.values().map(|v|v.0).collect::<Vec<_>>();
   let surface=Surface::new(values.iter().map(|p|[p[0] as f64*XY,p[1] as f64*XY,p[2] as f64*Z]).collect())?;
   let mut faces=HashSet::new();
   for t in surface.faces_for_rect(rect) {for &v in &surface.topology.triangles[3*t..3*t+3] {faces.extend(surface.incident[v].iter().copied());}}
   let mut extra=BTreeMap::new();
   for t in faces {
    let triangle=&surface.topology.triangles[3*t..3*t+3];let mut key=[ids[triangle[0]],ids[triangle[1]],ids[triangle[2]]];key.sort_unstable();if !checked.insert(key) {continue;}
    let c=surface.circles[t];let r=c[2].sqrt();ensure!(r.is_finite(),"外接円の半径が不正です");
    for (id,q,flags) in self.query(z,[c[0]-r,c[1]-r,c[0]+r,c[1]+r])? {
     if rows.contains_key(&id) || extra.contains_key(&id) {continue;}
     if (q[0] as f64*XY-c[0]).powi(2)+(q[1] as f64*XY-c[1]).powi(2)<=c[2]*(1.+1e-12)+1e-6 {extra.insert(id,(q,flags));}
    }
   }
   if extra.is_empty() {return Ok(Local{surface,values});}
   rows.extend(extra);
  }
  anyhow::bail!("全球Delaunay支持集合が128回で閉じません")
 }
}
pub struct Local {pub surface:Surface,pub values:Vec<[i32;3]>}
#[cfg(test)]mod tests {use super::*;
 #[test]fn integer_id_and_quantization(){assert_eq!(point_id([16,32]).unwrap(),(16i64<<32)|32);assert_eq!(quantize(-0.1875,0.125).unwrap(),-1);assert!(point_id([-1,0]).is_err());}
 #[test]fn support_closure_matches_global(){
  let root=std::env::current_dir().unwrap().join("build").join(format!("natural-catalog-test-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));std::fs::create_dir_all(&root).unwrap();
  {let c=Catalog::open(&root).unwrap();c.guards(8).unwrap();for i in 0..300 {let x=1000+((i*131)%1000)*16;let y=1000+((i*197)%1000)*16;c.add(8,[x,y,(i*17)%500],0).unwrap();}
   let all=c.query(8,[0.,0.,WORLD,WORLD]).unwrap();let global=Surface::new(all.iter().map(|r|[r.1[0] as f64*XY,r.1[1] as f64*XY,r.1[2] as f64*Z]).collect()).unwrap();let local=c.local(8,[200.,200.,400.,400.],1.).unwrap();
   for y in 0..10 {for x in 0..10 {let q=[201.+19.*x as f64,201.+19.*y as f64];assert!((global.evaluate(q,&mut 0).unwrap()-local.surface.evaluate(q,&mut 0).unwrap()).abs()<1e-7);}}
  }for name in ["work.sqlite3","work.sqlite3-wal","work.sqlite3-shm"]{let p=root.join(name);if p.exists(){std::fs::remove_file(p).unwrap();}}std::fs::remove_dir(root).unwrap();
 }
}
