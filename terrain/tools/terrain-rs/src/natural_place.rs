//! LPFの空間スケールと曲率から自由配置する。誤差探索による反復追加はしない。
use anyhow::{Result,Context,ensure};
use std::{path::Path,fs,time::Instant};
use rusqlite::{Connection,params};
use rstar::{RTree,RTreeObject,AABB};
use serde_json::json;
use crate::{natural_catalog::{Catalog,XY,Z,quantize},natural_fit::LpfSampler,natural_dem::STEP,natural_inventory::progress};
#[derive(Clone)]struct Site {q:[f64;2],r:f64}
impl RTreeObject for Site {type Envelope=AABB<[f64;2]>;fn envelope(&self)->Self::Envelope{AABB::from_corners([self.q[0]-self.r,self.q[1]-self.r],[self.q[0]+self.r,self.q[1]+self.r])}}
fn radius(lpf:&mut LpfSampler,q:[i32;2],h:f64,base:f64,quality:f64)->Result<f64>{
 let d=(base/XY).round() as i32;let mut bend=0.;
 for axis in 0..2 {let mut a=q;let mut b=q;a[axis]-=d;b[axis]+=d;if let (Some(a),Some(b))=(lpf.sample(a)?,lpf.sample(b)?){bend+=(a-2.*h+b).abs();}}
 Ok(base*(3./(1.+bend/(quality*0.05)).sqrt()).clamp(1.,3.))
}
fn compatible(tree:&RTree<Site>,q:[f64;2],r:f64)->bool{
 let box_=AABB::from_corners([q[0]-r,q[1]-r],[q[0]+r,q[1]+r]);
 tree.locate_in_envelope_intersecting(&box_).all(|p|{let dx=p.q[0]-q[0];let dy=p.q[1]-q[1];dx*dx+dy*dy>=r.max(p.r).powi(2)})
}
fn fixed_budget(c:&Catalog,output:&Path,z:u32,sigma:f64,quality:f64,bounds:[f64;4],budget:usize)->Result<()> {
 ensure!((4..=20000).contains(&budget),"比較の固定点数は4～20000です");
 let start=Instant::now();let mut lpf=LpfSampler::new(output,z)?;let metric=(if sigma==0.{quality*0.75}else{sigma*1.5}).max(STEP*4.);let mut candidates=Vec::new();let mut unique=std::collections::HashSet::new();
 for i in 0..budget*128 {
  let u=(0.5+(i+1) as f64*0.7548776662466927).fract();let v=(0.5+(i+1) as f64*0.5698402909980532).fract();let q=[quantize(bounds[0]+u*(bounds[2]-bounds[0]),XY)?,quantize(bounds[1]+v*(bounds[3]-bounds[1]),XY)?];
  if unique.insert(q){if let Some(h)=lpf.sample(q)? {let factor=radius(&mut lpf,q,h,metric,quality)?/metric;candidates.push((q,h,factor));}}
 }
 ensure!(candidates.len()>=budget,"有効な配置候補が不足しています");
 let area=(bounds[2]-bounds[0])*(bounds[3]-bounds[1]);let mut lo=0.;let mut hi=area.sqrt();let mut best=Vec::new();let mut passes=0;
 // 制御点数だけを調整する。曲面の誤差評価・再適合は行わない。
 for _ in 0..18 {
  passes+=1;let spacing=(lo+hi)*0.5;let mut tree=RTree::new();let mut selected=Vec::new();
  for (i,(q,_,factor)) in candidates.iter().enumerate(){let xy=[q[0] as f64*XY,q[1] as f64*XY];let r=spacing*factor;if compatible(&tree,xy,r){tree.insert(Site{q:xy,r});selected.push(i);if selected.len()==budget{break;}}}
  if selected.len()==budget{lo=spacing;best=selected;}else{hi=spacing;}
 }
 ensure!(best.len()==budget,"点数をそろえられませんでした");
 let tx=c.db.unchecked_transaction()?;c.guards(z)?;
 for i in best {let (q,h,_)=candidates[i];c.add(z,[q[0],q[1],quantize(h,Z)?],0)?;}
 c.db.execute("INSERT OR REPLACE INTO metadata VALUES(?1,'complete')",[format!("fit-phase-z{z}")])?;
 c.db.execute("INSERT OR REPLACE INTO metadata VALUES(?1,?2)",params![format!("point-count-z{z}"),(budget+4).to_string()])?;
 c.db.execute("INSERT OR REPLACE INTO metadata VALUES(?1,?2)",params![format!("placement-info-z{z}"),json!({"points":budget,"sigma":sigma,"spacing_scale":lo,"density_passes":passes,"candidates":candidates.len(),"seconds":start.elapsed().as_secs_f64(),"max_error_guaranteed":false}).to_string()])?;tx.commit()?;Ok(())
}
pub fn run(output:&Path)->Result<()> {
 let lock=Connection::open(output.join("run-lock.sqlite3"))?;lock.execute_batch("BEGIN EXCLUSIVE")?;
 let config:serde_json::Value=serde_json::from_slice(&fs::read(output.join("settings.json"))?)?;
 ensure!(config["placement_policy"].as_str()==Some("lpf-scale-poisson"),"スケール配置専用の設定が必要です");
 let c=Catalog::open(output)?;
 let bounds:[f64;4]=c.db.query_row("SELECT min(x)*256,min(y)*256,(max(x)+1)*256,(max(y)+1)*256 FROM inputs",[],|r|Ok([r.get::<_,f64>(0)?*STEP,r.get::<_,f64>(1)?*STEP,r.get::<_,f64>(2)?*STEP,r.get::<_,f64>(3)?*STEP]))?;
 let bounds=if let Some(v)=config.get("placement_bounds"){serde_json::from_value::<[f64;4]>(v.clone())?}else{bounds};
 ensure!(bounds.iter().all(|v|v.is_finite()) && bounds[0]<bounds[2] && bounds[1]<bounds[3],"配置範囲が不正です");
 let area=(bounds[2]-bounds[0])*(bounds[3]-bounds[1]);
 let only=config["placement_only_z"].as_u64();
 for (z,quality) in [(8,800.),(9,400.),(10,200.),(11,100.),(12,50.),(13,25.),(14,12.5)] {
  if only.is_some_and(|v|v!=z as u64){continue;}
  let key=format!("fit-phase-z{z}");let old:Option<String>=c.db.query_row("SELECT value FROM metadata WHERE key=?1",[&key],|r|r.get(0)).ok();if old.as_deref()==Some("complete"){continue;}
  let quality=config["placement_quality_m"].as_f64().unwrap_or(quality);ensure!(quality.is_finite() && quality>0.,"配置品質が不正です");
  let sigma: f64=c.db.query_row("SELECT value FROM metadata WHERE key=?1",[format!("lpf-sigma-z{z}")],|r|r.get::<_,String>(0))?.parse()?;
  if let Some(budget)=config["fixed_point_budget"].as_u64(){fixed_budget(&c,output,z,sigma,if config["lpf_policy"].as_str()==Some("none"){quality}else{sigma*2.},bounds,budget as usize)?;continue;}
  let raw=config["lpf_policy"].as_str()==Some("none");
  ensure!((raw && sigma==0.) || (sigma-quality/2.).abs()<1e-9,"LPF幅が設定と一致しません");
  let base=(if raw{quality*0.75}else{sigma*1.5}).max(STEP*4.);let candidates=(area/base.powi(2)*6.).ceil() as u64;ensure!(candidates<=20000000,"比較領域が大きすぎます");
  let start=Instant::now();let mut lpf=LpfSampler::new(output,z)?;let mut tree=RTree::new();let mut accepted=0u64;
  let tx=c.db.unchecked_transaction()?;c.guards(z)?;
  if z>8 {
   let mut stmt=c.db.prepare("SELECT p.qx,p.qy FROM points p JOIN point_levels l ON l.id=p.id WHERE l.z=?1 AND l.flags<>1 ORDER BY p.id")?;
   for row in stmt.query_map([z-1],|r|Ok([r.get::<_,i32>(0)?,r.get::<_,i32>(1)?]))? {let q=row?;let h=lpf.sample(q)?.context("親点がLPF領域外です")?;let r=radius(&mut lpf,q,h,base,quality)?;c.add(z,[q[0],q[1],quantize(h,Z)?],0)?;tree.insert(Site{q:[q[0] as f64*XY,q[1] as f64*XY],r});accepted+=1;}
  }
  for i in 0..candidates {
   // R2列: X/Yの行列格子を作らず、矩形全体を再現可能な順序で埋める。
   let u=(0.5+(i+1) as f64*0.7548776662466927).fract();let v=(0.5+(i+1) as f64*0.5698402909980532).fract();
   let q=[quantize(bounds[0]+u*(bounds[2]-bounds[0]),XY)?,quantize(bounds[1]+v*(bounds[3]-bounds[1]),XY)?];
   if let Some(h)=lpf.sample(q)? {let r=radius(&mut lpf,q,h,base,quality)?;let xy=[q[0] as f64*XY,q[1] as f64*XY];
    if compatible(&tree,xy,r) && c.add(z,[q[0],q[1],quantize(h,Z)?],0)? {tree.insert(Site{q:xy,r});accepted+=1;ensure!(accepted<=500000,"地域の安全上限50万点を超えました。完了にはしません");}
   }
   if i%100000==0 {progress(output,json!({"stage":"scale-placement","z":z,"quality_m":quality,"sigma_m":sigma,"base_spacing_m":base,"done":i,"total":candidates,"points":accepted,"elapsed_seconds":start.elapsed().as_secs_f64(),"error_is_guaranteed":false,"complete":false}))?;}
  }
  c.db.execute("INSERT OR REPLACE INTO metadata VALUES(?1,'complete')",[&key])?;c.db.execute("INSERT OR REPLACE INTO metadata VALUES(?1,?2)",params![format!("point-count-z{z}"),(accepted+4).to_string()])?;
  c.db.execute("INSERT OR REPLACE INTO metadata VALUES(?1,?2)",params![format!("placement-info-z{z}"),json!({"sigma":sigma,"base_spacing":base,"min_spacing":base,"max_spacing":base*3.,"candidates":candidates,"points":accepted,"seconds":start.elapsed().as_secs_f64(),"max_error_guaranteed":false}).to_string()])?;tx.commit()?;
  progress(output,json!({"stage":"scale-quality-complete","z":z,"points":accepted,"elapsed_seconds":start.elapsed().as_secs_f64(),"complete":false}))?;
 }
 progress(output,json!({"stage":"scale-placement-complete","error_is_guaranteed":false,"complete":false,"next_stage":"measure-errors-and-preview"}))?;Ok(())
}
#[cfg(test)]mod tests{use super::*;#[test]fn varying_radii_enforce_symmetric_distance(){let mut t=RTree::new();t.insert(Site{q:[0.,0.],r:2.});assert!(!compatible(&t,[1.,0.],0.5));assert!(!compatible(&t,[3.,0.],4.));assert!(compatible(&t,[4.,0.],4.));}}
