//! 全国自然近傍生成の入口。各段階の検証を通してから次へ進む。
mod natural_surface;
mod natural_codec;
mod natural_inventory;
mod natural_dem;
mod natural_catalog;
mod natural_fit;
mod natural_place;
mod natural_pack;
mod natural_verify;
use anyhow::Result;
use clap::{Parser,ArgGroup};
use std::{fs,path::PathBuf};
#[derive(Parser)]
#[command(group(ArgGroup::new("mode").required(true).multiple(false).args(["evaluate","pack","inventory","lpf","fit","place_scale","pack_tiles","verify"])))]
struct Args {
    /// 制御点と照会点のJSONを読み、RustでSibson評価する検査モード。
    #[arg(long)] evaluate:Option<PathBuf>,
    #[arg(long)] pack:Option<PathBuf>,
    #[arg(long)] output:PathBuf,
    #[arg(long)] inventory:bool,
    #[arg(long)] lpf:bool,
    #[arg(long)] fit:bool,
    #[arg(long)] place_scale:bool,
    #[arg(long)] pack_tiles:bool,
    #[arg(long)] verify:bool,
    #[arg(long,requires="verify")] finalize:bool,
    #[arg(long,default_value="build/terrain-lod.sqlite3")] source:PathBuf,
    #[arg(long,default_value="build/terrain-cache")] cache:PathBuf,
    #[arg(long,default_value="NATIONAL_NATURAL_SETTINGS.json")] settings:PathBuf,
    #[arg(long)] resume:bool,
    #[arg(long)] workers:Option<usize>,
    #[arg(long,num_args=4,requires="inventory")] window:Vec<u32>,
}
fn execute(a:Args)->Result<()> {
 if a.place_scale{return natural_place::run(&a.output);}
 if a.verify {return natural_verify::run(&a.output,a.finalize);}
 if a.pack_tiles {return natural_pack::run(&a.output,a.workers.unwrap_or_else(||std::thread::available_parallelism().map(|v|v.get()/2).unwrap_or(1).max(1)));}
 if a.fit {return natural_fit::run(&a.output,a.workers.unwrap_or_else(||std::thread::available_parallelism().map(|v|v.get()/2).unwrap_or(1).max(1)));}
 if a.lpf {return natural_dem::run(&a.output,a.workers.unwrap_or_else(||std::thread::available_parallelism().map(|v|v.get()/2).unwrap_or(1).max(1)));}
 if a.inventory {return natural_inventory::run(&a.source,&a.cache,&a.output,&a.settings,a.workers.unwrap_or_else(||std::thread::available_parallelism().map(|v|v.get()/2).unwrap_or(1).max(1)),a.resume,&a.window);}
 if let Some(input)=a.pack {let v:serde_json::Value=serde_json::from_slice(&fs::read(input)?)?;let p:Vec<[i32;3]>=serde_json::from_value(v["points"].clone())?;
 let blob=if v.get("parents").is_some() {let parents:Vec<Vec<[i32;3]>>=serde_json::from_value(v["parents"].clone())?;let refs:Vec<Option<(usize,usize)>>=serde_json::from_value(v["refs"].clone())?;natural_codec::delta(&p,&refs,&parents)?}else{natural_codec::independent(&p)?};fs::write(a.output,blob)?;return Ok(());}
 let v:serde_json::Value=serde_json::from_slice(&fs::read(a.evaluate.ok_or_else(||anyhow::anyhow!("--evaluate または --pack が必要です"))?)?)?;
 let p:Vec<[f64;3]>=serde_json::from_value(v["points"].clone())?;
 let q:Vec<[f64;2]>=serde_json::from_value(v["queries"].clone())?;
 let surface=natural_surface::Surface::new(p)?;let mut start=0;let mut workspace=natural_surface::Workspace::default();
 let values=q.into_iter().map(|p|surface.evaluate_reusing(p,&mut start,&mut workspace)).collect::<Result<Vec<_>>>()?;
 fs::write(a.output,serde_json::to_vec(&values)?)?;Ok(())
}

fn main()->Result<()> {
 let a=Args::parse();let output=a.output.clone();let numerical=a.place_scale||a.inventory||a.lpf||a.fit||a.pack_tiles||a.verify;
 if let Err(error)=execute(a) {
  let busy=error.chain().any(|e|matches!(e.downcast_ref::<rusqlite::Error>(),Some(rusqlite::Error::SqliteFailure(code,_)) if matches!(code.code,rusqlite::ErrorCode::DatabaseBusy|rusqlite::ErrorCode::DatabaseLocked)));
  if numerical && !busy && output.is_dir() {let previous=fs::read(output.join("progress.json")).ok().and_then(|b|serde_json::from_slice::<serde_json::Value>(&b).ok());let _=natural_inventory::progress(&output,serde_json::json!({"stage":"failed","error":format!("{error:#}"),"previous":previous,"complete":false,"distributable":false}));}
  return Err(error);
 }Ok(())
}
