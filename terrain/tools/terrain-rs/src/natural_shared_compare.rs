//! 全域Sibson曲面を保存領域へ分割し、局所傾きと作業領域分割を比較する。
#[allow(dead_code)] mod natural_surface;
#[allow(dead_code)] mod natural_regional_surface;
#[allow(dead_code)] mod natural_regional_gpu;
use anyhow::{Result,ensure};
use clap::Parser;
use natural_regional_surface::{Fast,Work};
use serde_json::json;
use std::{fs,path::{Path,PathBuf},collections::{BTreeSet,HashMap},time::Instant};
#[derive(Parser)] struct Args {
 #[arg(long)] input:PathBuf,#[arg(long)] output:PathBuf,
 #[arg(long,default_value_t=128000.)] side:f64,
 #[arg(long)] local_only:bool,
 #[arg(long,default_value_t=32000.)] tile_side:f64,
}
mod natural_local_gradient;
use natural_local_gradient::local_gradients;
fn point_key(p:[f64;3])->(u64,u64) {(p[0].to_bits(),p[1].to_bits())}
fn expanded_support(m:&Fast,r:[f64;4])->Vec<usize> {
 let mut ids=m.base.support_vertices(r).into_iter().collect::<BTreeSet<_>>();
 // 支持点の傾きを復元する一周近傍も保持する。固定距離で打ち切らない。
 for i in ids.iter().copied().collect::<Vec<_>>() {ids.extend(m.base.neighbors[i].iter().copied());}
 ids.into_iter().collect()
}
fn write_packet(root:&Path,mode:&str,x:usize,y:usize,m:&Fast,g:&[[f64;2]],r:[f64;4])->Result<usize>{
 let dir=root.join(mode).join(format!("{x}-{y}"));fs::create_dir_all(&dir)?;
 let (bytes,info)=natural_regional_gpu::packet(m,g,r,json!({"sharedSurface":true,"gradientMode":mode,"allDemPixelsAudited":false,"origin":[0.,0.],"coreBounds":r,"worldFieldBounds":r,"noTileHeightBlending":true}))?;
 fs::write(dir.join("packet.bin"),&bytes)?;fs::write(dir.join("packet.json"),serde_json::to_vec_pretty(&info)?)?;Ok(bytes.len())
}
fn main()->Result<()> {
 let a=Args::parse();ensure!(a.side>0.&&a.tile_side>0.&&(a.side/a.tile_side).fract()==0.,"範囲と分割が不正");
 let start=Instant::now();let bytes=fs::read(&a.input)?;ensure!(bytes.len()%24==0,"入力XYZが不正");
 let points=bytes.chunks_exact(24).map(|p|std::array::from_fn(|i|f64::from_le_bytes(p[i*8..i*8+8].try_into().unwrap()))).collect::<Vec<_>>();drop(bytes);
 let m=Fast::new(points)?;fs::create_dir_all(&a.output)?;
 println!("全域接続完了 {}点",m.base.points.len());
 let t=Instant::now();let lg=local_gradients(&m);let local_seconds=t.elapsed().as_secs_f64();
 let t=Instant::now();let gg=if a.local_only {Vec::new()}else{natural_regional_gpu::gradient(&m,[0.,0.,a.side,a.side])?};let global_seconds=t.elapsed().as_secs_f64();
 ensure!((a.side/a.tile_side) as usize%2==0,"保存領域は作業領域の2分割へ整合する必要があります");
 let n=(a.side/a.tile_side) as usize;let mut total=[0usize;3];let mut max_h=0f64;let mut max_g=0f64;let mut checked=0;let mut region_support=0;
 // 保存領域とは異なる2×2の作業領域で再構築し、同じ傾き・補間を復元する。
 for by in 0..2 {for bx in 0..2 {
  let r=[bx as f64*a.side/2.,by as f64*a.side/2.,(bx+1) as f64*a.side/2.,(by+1) as f64*a.side/2.];
  let ids=expanded_support(&m,r);region_support+=ids.len();let block=Fast::new(ids.iter().map(|&i|m.base.points[i]).collect())?;let bg=local_gradients(&block);
  let block_lookup=block.base.points.iter().enumerate().map(|(i,&p)|(point_key(p),i)).collect::<HashMap<_,_>>();
  for y in by*n/2..(by+1)*n/2 {for x in bx*n/2..(bx+1)*n/2 {
   let r=[x as f64*a.tile_side,y as f64*a.tile_side,(x+1) as f64*a.tile_side,(y+1) as f64*a.tile_side];
   let mut w=Work::default();let mut bw=Work::default();
   for iy in 0..=8 {for ix in 0..=8 {
    let q=[r[0]+(r[2]-r[0])*ix as f64/8.,r[1]+(r[3]-r[1])*iy as f64/8.];
    let h=m.evaluate(q,&mut w)?;max_h=max_h.max((h-block.evaluate(q,&mut bw)?).abs());
    for (id,weight) in m.basis(q,&mut w)? {if weight<1e-12 {continue;}let j=*block_lookup.get(&point_key(m.base.points[id])).ok_or_else(||anyhow::anyhow!("作業領域の支持点が不足"))?;
     max_g=max_g.max((lg[id][0]-bg[j][0]).abs()).max((lg[id][1]-bg[j][1]).abs());
    }checked+=1;
   }}
   // 最詳細では全域セルをタイルごとに複製すると膨大になる。必要な支持だけを切り出す。
   let ti=expanded_support(&m,r);let tm=Fast::new(ti.iter().map(|&i|m.base.points[i]).collect())?;
   let tg=if a.local_only {Vec::new()}else{ti.iter().map(|&i|gg[i]).collect::<Vec<_>>()};let tl=ti.iter().map(|&i|lg[i]).collect::<Vec<_>>();
   let bi=expanded_support(&block,r);let bm=Fast::new(bi.iter().map(|&i|block.base.points[i]).collect())?;
   let bl=bi.iter().map(|&i|bg[i]).collect::<Vec<_>>();
   // 支持切出しそのものによっても高さが変わらないことを、辺を含めて確認する。
   let mut tw=Work::default();let mut bmw=Work::default();
   for iy in 0..=4 {for ix in 0..=4 {
    let q=[r[0]+(r[2]-r[0])*ix as f64/4.,r[1]+(r[3]-r[1])*iy as f64/4.];
    let h=m.evaluate(q,&mut w)?;max_h=max_h.max((h-tm.evaluate(q,&mut tw)?).abs()).max((h-bm.evaluate(q,&mut bmw)?).abs());
   }}
   if !a.local_only {total[0]+=write_packet(&a.output,"global",x,y,&tm,&tg,r)?;}
   total[1]+=write_packet(&a.output,"local",x,y,&tm,&tl,r)?;
   total[2]+=write_packet(&a.output,"partition",x,y,&bm,&bl,r)?;
   println!("完了 {x}-{y}");
  }}
 }}
 let report=json!({"gradientAlgorithm":"voronoi-neighbor-ls-v2","points":m.base.points.len(),"side":a.side,"tileSide":a.tile_side,"tiles":n*n,"globalGradientSeconds":global_seconds,"localGradientSeconds":local_seconds,"packetBytes":total,"maxPartitionHeightDifference":max_h,"maxPartitionGradientDifference":max_g,"checked":checked,"workRegionSupportReferences":region_support,"seconds":start.elapsed().as_secs_f64(),"source":"既存制御点を統合して全域曲面を再構築。元DEMへの再適合・誤差保証の引継ぎではない","complete":max_h<1e-6&&max_g<1e-8});
 fs::write(a.output.join("report.json"),serde_json::to_vec_pretty(&report)?)?;
 ensure!(max_h<1e-6&&max_g<1e-8,"作業領域によって曲面・傾きが変化しました: {max_h}, {max_g}");Ok(())
}
#[cfg(test)] mod tests {
 use super::*;
 #[test]fn plane_gradient_and_partition_are_consistent(){
  let mut p=vec![[-100.,-100.,-500.],[200.,-100.,100.],[-100.,200.,400.],[200.,200.,1000.]];
  for i in 0..120 {let x=(i as f64*0.754877666)%1.*100.;let y=(i as f64*0.569840291)%1.*100.;p.push([x,y,2.*x+3.*y]);}
  let m=Fast::new(p).unwrap();let g=local_gradients(&m);
  for i in 4..g.len() {assert!((g[i][0]-2.).abs()<1e-8);assert!((g[i][1]-3.).abs()<1e-8);}
  let ids=expanded_support(&m,[25.,25.,75.,75.]);let b=Fast::new(ids.iter().map(|&i|m.base.points[i]).collect()).unwrap();let bg=local_gradients(&b);
  for (j,&i) in ids.iter().enumerate(){if m.base.points[i][0]>30.&&m.base.points[i][0]<70.&&m.base.points[i][1]>30.&&m.base.points[i][1]<70.{assert!((bg[j][0]-g[i][0]).abs()<1e-8);}}
 }
}
