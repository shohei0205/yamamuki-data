//! 生成前の全域曲面構築と分割時の実メモリー・接続保持を測定する。
#[allow(dead_code)] mod natural_surface;
#[allow(dead_code)] mod natural_regional_surface;
#[allow(dead_code)] mod natural_regional_gpu;
use anyhow::{Result,ensure};
use clap::Parser;
use serde_json::json;
use natural_regional_surface::{Fast,Work};
use std::{fs,path::PathBuf,time::Instant};
#[derive(Parser)] struct Args {
 #[arg(long)] input:PathBuf,
 #[arg(long)] output:PathBuf,
 #[arg(long)] side:f64,
 #[arg(long)] auxiliary:bool,
 #[arg(long,default_value_t=8000.)] tile_side:f64,
 #[arg(long,default_value_t=4096)] queries:usize,
 #[arg(long,default_value_t=4000000)] max_points:usize,
}
#[cfg(windows)] fn memory()->serde_json::Value {
 #[repr(C)] #[derive(Default)] struct Counters {cb:u32,faults:u32,peak:usize,working:usize,pp:usize,p:usize,pnp:usize,np:usize,page:usize,peakpage:usize,private:usize}
 #[link(name="psapi")] unsafe extern "system" {fn GetProcessMemoryInfo(process:*mut std::ffi::c_void,counters:*mut Counters,size:u32)->i32;}
 #[link(name="kernel32")] unsafe extern "system" {fn GetCurrentProcess()->*mut std::ffi::c_void;}
 let mut c=Counters::default();c.cb=std::mem::size_of::<Counters>() as u32;
 let ok=unsafe{GetProcessMemoryInfo(GetCurrentProcess(),&mut c,std::mem::size_of::<Counters>() as u32)};
 assert_ne!(ok,0,"メモリー計測に失敗");json!({"working_bytes":c.working,"peak_working_bytes":c.peak,"private_bytes":c.private,"peak_commit_bytes":c.peakpage})
}
#[cfg(not(windows))] fn memory()->serde_json::Value {json!({"unavailable":true})}
fn main()->Result<()> {
 let a=Args::parse();ensure!(a.side.is_finite()&&a.side>0.&&a.tile_side.is_finite()&&a.tile_side>0.,"範囲が不正");
 let start=Instant::now();ensure!(fs::metadata(&a.input)?.len()<=a.max_points as u64*24,"入力点数が上限を超えています");let bytes=fs::read(&a.input)?;ensure!(bytes.len()%24==0&&bytes.len()/24<=a.max_points,"入力点数が上限を超えています");
 let points=bytes.chunks_exact(24).map(|p|std::array::from_fn(|i|f64::from_le_bytes(p[i*8..i*8+8].try_into().unwrap()))).collect::<Vec<[f64;3]>>();drop(bytes);
 let n=points.len();let mut stages=vec![];stages.push(json!({"stage":"loaded","seconds":start.elapsed().as_secs_f64(),"memory":memory()}));
 let tick=Instant::now();let model=Fast::new(points)?;
 stages.push(json!({"stage":"global_topology","seconds":tick.elapsed().as_secs_f64(),"memory":memory()}));
 println!("全域接続構築: {}点 {:0.3}秒",n,tick.elapsed().as_secs_f64());
 if a.auxiliary {
  let bounds=[0.,0.,a.side,a.side];let tick=Instant::now();
  let gradient=natural_regional_gpu::gradient(&model,bounds)?;
  stages.push(json!({"stage":"global_c1","seconds":tick.elapsed().as_secs_f64(),"memory":memory()}));
  println!("全域傾き調整: {:.3}秒",tick.elapsed().as_secs_f64());
  let tick=Instant::now();let (packet,info)=natural_regional_gpu::packet(&model,&gradient,bounds,json!({}))?;
  stages.push(json!({"stage":"gpu_packet","seconds":tick.elapsed().as_secs_f64(),"memory":memory()}));
  fs::write(a.output.with_extension("packet.bin"),&packet)?;
  fs::write(&a.output,serde_json::to_vec_pretty(&json!({"points":n,"side_m":a.side,"packet_bytes":packet.len(),"packet_metadata":info,"stages":stages,"total_seconds":start.elapsed().as_secs_f64(),"scope":"固定した既存制御点で全域C1・GPU補助情報を構築。DEM適合・削減は未計測"}))?)?;
  return Ok(());
 }
 let mut work=Work::default();let tick=Instant::now();let mut checksum=0.;
 for i in 0..a.queries {let q=[a.side*((i as f64*0.754877666)%0.98+0.01),a.side*((i as f64*0.569840291)%0.98+0.01)];checksum+=model.evaluate(q,&mut work)?;}
 drop(work);stages.push(json!({"stage":"queries","count":a.queries,"checksum":checksum,"seconds":tick.elapsed().as_secs_f64(),"memory":memory()}));
 // 各タイルの補間に影響する外接円と、その頂点のセルを構成する隣接点を保持。
 let tick=Instant::now();let count=(a.side/a.tile_side).ceil() as usize;
 let mut refs=0usize;let mut max_support=0;let mut comparisons=0;let mut max_difference=0f64;
 let mut subset_bytes=0u64;
 for y in 0..count {for x in 0..count {
  let r=[x as f64*a.tile_side,y as f64*a.tile_side,((x+1) as f64*a.tile_side).min(a.side),((y+1) as f64*a.tile_side).min(a.side)];
  let ids=model.base.support_vertices(r);refs+=ids.len();max_support=max_support.max(ids.len());
  // 同じ全域接続から切り出した点集合で曲面を復元し、再近似なしで一致するか実測。
  let local=ids.iter().map(|&i|model.base.points[i]).collect::<Vec<_>>();subset_bytes+=(local.len()*24) as u64;
  let local=Fast::new(local)?;let mut global_work=Work::default();let mut local_work=Work::default();
  for uv in [[0.001,0.5],[0.999,0.5],[0.5,0.001],[0.5,0.999],[0.5,0.5]] {
   let q=[r[0]+uv[0]*(r[2]-r[0]),r[1]+uv[1]*(r[3]-r[1])];
   let delta=(model.evaluate(q,&mut global_work)?-local.evaluate(q,&mut local_work)?).abs();max_difference=max_difference.max(delta);comparisons+=1;
  }
 }}
 stages.push(json!({"stage":"split_support","seconds":tick.elapsed().as_secs_f64(),"memory":memory()}));
 let result=json!({"scope":"既存制御点の統合・全域接続・補間・保存分割の規模試験。DEM適合・頂点削減・全域C1最適化・GPUパケット生成は未計測","points":n,"side_m":a.side,"tile_side_m":a.tile_side,"tiles":count*count,"support_references":refs,"support_multiplier":refs as f64/n as f64,"largest_tile_support":max_support,"uncompressed_split_xyz_bytes":subset_bytes,"comparison_samples":comparisons,"max_split_height_difference_m":max_difference,"triangles":model.base.topology.triangles.len()/3,"stages":stages,"total_seconds":start.elapsed().as_secs_f64()});
 fs::write(a.output,serde_json::to_vec_pretty(&result)?)?;ensure!(max_difference<1e-5,"分割によって補間結果が変化しました");Ok(())
}

#[cfg(test)] mod scale_tests {
 use super::*;
 #[test] fn split_support_preserves_irregular_surface_at_edges() {
  let mut points=vec![[-10.,-10.,0.],[110.,-10.,0.],[-10.,110.,0.],[110.,110.,0.]];
  for i in 0..200 {let x=(i as f64*0.754877666)%1.*100.;let y=(i as f64*0.569840291)%1.*100.;points.push([x,y,(x*0.1).sin()*30.+(y*0.2).cos()*10.]);}
  let global=Fast::new(points).unwrap();
  for r in [[0.,0.,50.,50.],[50.,0.,100.,50.],[0.,50.,50.,100.],[50.,50.,100.,100.]] {
   let ids=global.base.support_vertices(r);let local=Fast::new(ids.iter().map(|&i|global.base.points[i]).collect()).unwrap();
   let mut a=Work::default();let mut b=Work::default();
   for y in 0..=12 {for x in 0..=12 {let q=[r[0]+(r[2]-r[0])*x as f64/12.,r[1]+(r[3]-r[1])*y as f64/12.];assert!((global.evaluate(q,&mut a).unwrap()-local.evaluate(q,&mut b).unwrap()).abs()<1e-8);}}
  }
 }
}
