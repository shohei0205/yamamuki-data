//! 入力PNGを復号・監査し、再開可能な全国作業DBへ固定する。
use anyhow::{Result,Context,ensure};
use rusqlite::{Connection,OpenFlags,params,OptionalExtension};
use rayon::prelude::*;
use serde::{Serialize,Deserialize};
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use std::{fs,io::Cursor,path::Path,time::{Instant,Duration}};
#[derive(Clone,Serialize,Deserialize)]
pub struct Input {pub x:u32,pub y:u32,pub sha:String,pub bytes:u64,pub valid:u32,pub min_cm:i32,pub max_cm:i32}
pub fn hash(b:&[u8])->String {Sha256::digest(b).iter().map(|v|format!("{v:02x}")).collect()}
pub fn progress(output:&Path,value:Value)->Result<()> {
 let tmp=output.join("progress.next.json");fs::write(&tmp,serde_json::to_vec_pretty(&value)?)?;
 for attempt in 0..40 {match fs::rename(&tmp,output.join("progress.json")) {Ok(())=>return Ok(()),Err(e) if attempt<39 && matches!(e.raw_os_error(),Some(5|32|33))=>std::thread::sleep(Duration::from_millis(50)),Err(e)=>return Err(e.into())}}unreachable!()
}
pub fn decode(data:&[u8])->Result<Vec<f64>> {
 let mut decoder=png::Decoder::new(Cursor::new(data));decoder.set_transformations(png::Transformations::EXPAND);
 let mut reader=decoder.read_info()?;ensure!(reader.info().width==256 && reader.info().height==256,"PNGは256×256が必要です");
 let mut out=vec![0;reader.output_buffer_size().context("PNGサイズ不明")?];let info=reader.next_frame(&mut out)?;
 ensure!(info.bit_depth==png::BitDepth::Eight,"PNGのビット深度が不正です");
 let stride=match info.color_type {png::ColorType::Rgb=>3,png::ColorType::Rgba=>4,_=>anyhow::bail!("標高PNGがRGB/RGBAではありません")};
 let mut z=Vec::with_capacity(65536);
 for p in out[..info.buffer_size()].chunks_exact(stride) {let code=((p[0] as i32)<<16)|((p[1] as i32)<<8)|p[2] as i32;
 z.push(if code==0x800000 || stride==4 && p[3]==0 {f64::NAN}else{(if code>=0x800000 {code-0x1000000}else{code}) as f64*0.01});}
 ensure!(z.len()==65536,"PNG画素数が不正です");Ok(z)
}
fn inspect(cache:&Path,x:u32,y:u32,previous:Option<&Input>)->Result<Input> {
 let file=cache.join("14").join(x.to_string()).join(format!("{y}.png"));let data=fs::read(&file).with_context(||format!("必要なPNGがありません: {}",file.display()))?;
 let sha=hash(&data);
 if let Some(old)=previous {ensure!(old.sha==sha && old.bytes==data.len() as u64,"固定済みPNGが変更されました: {x}/{y}");return Ok(old.clone());}
 let values=decode(&data).with_context(||format!("PNG復号失敗: {x}/{y}"))?;let mut valid=0;let mut min_cm=i32::MAX;let mut max_cm=i32::MIN;
 for z in values {if z.is_finite(){let cm=(z*100.).round() as i32;valid+=1;min_cm=min_cm.min(cm);max_cm=max_cm.max(cm);}}
 if valid==0 {min_cm=0;max_cm=0;}
 Ok(Input{x,y,sha,bytes:data.len() as u64,valid,min_cm,max_cm})
}
pub fn open_work(output:&Path)->Result<Connection> {
 let db=Connection::open(output.join("work.sqlite3"))?;db.execute_batch("PRAGMA journal_mode=WAL;PRAGMA foreign_keys=ON;
 CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
 CREATE TABLE IF NOT EXISTS inputs(x INTEGER,y INTEGER,sha TEXT NOT NULL,bytes INTEGER,valid INTEGER,min_cm INTEGER,max_cm INTEGER,PRIMARY KEY(x,y)) WITHOUT ROWID;
 CREATE TABLE IF NOT EXISTS tasks(stage TEXT,z INTEGER,x INTEGER,y INTEGER,state TEXT,info TEXT,PRIMARY KEY(stage,z,x,y)) WITHOUT ROWID;
 CREATE TABLE IF NOT EXISTS artifacts(stage TEXT,z INTEGER,x INTEGER,y INTEGER,path TEXT,sha TEXT,PRIMARY KEY(stage,z,x,y)) WITHOUT ROWID;")?;Ok(db)
}
pub fn run(source:&Path,cache:&Path,output:&Path,settings:&Path,workers:usize,resume:bool,window:&[u32])->Result<()> {
 ensure!(workers>0 && workers<=256,"並列数が不正です");ensure!(window.is_empty() || window.len()==4 && window[0]<window[2] && window[1]<window[3],"入力範囲が不正です");
 fs::create_dir_all(output)?;
 let lock=Connection::open(output.join("run-lock.sqlite3"))?;lock.execute_batch("CREATE TABLE IF NOT EXISTS lock(id INTEGER);BEGIN EXCLUSIVE;")?;
 let config_bytes=fs::read(settings)?;let config:Value=serde_json::from_slice(&config_bytes)?;
 let expected=[800.,400.,200.,100.,50.,25.,12.5];let levels=config["levels"].as_array().context("品質設定がありません")?;
 ensure!(levels.len()==7 && levels.iter().enumerate().all(|(i,l)|l["z"].as_u64()==Some((i+8) as u64) && l["error_m"].as_f64()==Some(expected[i])),"対象はz8～z14の7品質です");
 let source=source.canonicalize()?;let cache=cache.canonicalize()?;let db=open_work(output)?;
 let identity=json!({"source":source,"cache":cache,"settings_sha256":hash(&config_bytes),"window":window});
 let saved:Option<String>=db.query_row("SELECT value FROM metadata WHERE key='identity'",[],|r|r.get(0)).optional()?;
 if let Some(saved)=saved {ensure!(resume,"既存の作業には--resumeが必要です");ensure!(serde_json::from_str::<Value>(&saved)?==identity,"保存済み設定・入力パスが異なります");}
 else {db.execute("INSERT INTO metadata VALUES('identity',?1)",[identity.to_string()])?;fs::write(output.join("settings.json"),config_bytes)?;}
 let inputs=Connection::open_with_flags(source,OpenFlags::SQLITE_OPEN_READ_ONLY)?;
 let mut query=inputs.prepare("SELECT x,y FROM tiles WHERE z=14 ORDER BY y,x")?;
 let all=query.query_map([],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,u32>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
 let xy=all.into_iter().filter(|&(x,y)|window.is_empty() || x>=window[0]&&y>=window[1]&&x<window[2]&&y<window[3]).collect::<Vec<_>>();
 ensure!(!xy.is_empty(),"対象のz14入力がありません");ensure!(xy.windows(2).all(|w|w[0]!=w[1]) && xy.iter().all(|&(x,y)|x<16384 && y<16384),"元索引の重複または範囲外です");
 let index_hash=hash(&serde_json::to_vec(&xy)?);
 let old_index:Option<String>=db.query_row("SELECT value FROM metadata WHERE key='index_sha256'",[],|r|r.get(0)).optional()?;
 if let Some(old)=old_index {ensure!(old==index_hash,"固定済み元索引が変更されています");}else{db.execute("INSERT INTO metadata VALUES('index_sha256',?1)",[&index_hash])?;}
 let pool=rayon::ThreadPoolBuilder::new().num_threads(workers).build()?;let started=Instant::now();let mut done=0;let mut valid=0u64;let mut digest=Sha256::new();
 progress(output,json!({"stage":"inventory","done":0,"total":xy.len(),"workers":workers,"complete":false}))?;
 for batch in xy.chunks(workers*8) {
  let previous=batch.iter().map(|&(x,y)|db.query_row("SELECT sha,bytes,valid,min_cm,max_cm FROM inputs WHERE x=?1 AND y=?2",params![x,y],|r|Ok(Input{x,y,sha:r.get(0)?,bytes:r.get(1)?,valid:r.get(2)?,min_cm:r.get(3)?,max_cm:r.get(4)?})).optional()).collect::<rusqlite::Result<Vec<_>>>()?;
  let result=pool.install(||batch.par_iter().zip(&previous).map(|(&(x,y),old)|inspect(&cache,x,y,old.as_ref())).collect::<Result<Vec<_>>>());
  let result=match result {Ok(v)=>v,Err(e)=> {progress(output,json!({"stage":"failed","error":format!("{e:#}"),"done":done,"total":xy.len(),"complete":false}))?;return Err(e);}};
  let tx=db.unchecked_transaction()?;
  for r in result {tx.execute("INSERT OR IGNORE INTO inputs VALUES(?1,?2,?3,?4,?5,?6,?7)",params![r.x,r.y,r.sha,r.bytes,r.valid,r.min_cm,r.max_cm])?;digest.update(serde_json::to_vec(&r)?);valid+=r.valid as u64;done+=1;}
  tx.commit()?;
  progress(output,json!({"stage":"inventory","done":done,"total":xy.len(),"valid_pixels":valid,"elapsed_seconds":started.elapsed().as_secs_f64(),"workers":workers,"complete":false}))?;
 }
 let manifest=digest.finalize().iter().map(|v|format!("{v:02x}")).collect::<String>();
 db.execute("INSERT OR REPLACE INTO metadata VALUES('input_manifest_sha256',?1)",[manifest])?;
 db.execute("INSERT OR REPLACE INTO metadata VALUES('inventory_complete','true')",[])?;
 let mut counts=serde_json::Map::new();
 for z in 8..=14 {let mut seen=std::collections::BTreeSet::new();for &(x,y) in &xy {seen.insert((x>>(14-z),y>>(14-z)));}counts.insert(z.to_string(),json!(seen.len()));}
 let report=json!({"stage":"inventory-complete","input_tiles":xy.len(),"valid_pixels":valid,"candidate_tiles":counts,"candidate_counts_are_coverage":false,"complete":false,"distributable":false,"elapsed_seconds":started.elapsed().as_secs_f64(),"next_stage":"lpf-audit"});
 fs::write(output.join("inventory.json"),serde_json::to_vec_pretty(&report)?)?;progress(output,report)?;db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;Ok(())
}
#[cfg(test)] mod tests {use super::*;
 #[test] fn png_signed_and_nodata(){let mut b=Vec::new();{let mut e=png::Encoder::new(&mut b,256,256);e.set_color(png::ColorType::Rgb);e.set_depth(png::BitDepth::Eight);let mut w=e.write_header().unwrap();let mut rgb=vec![0u8;256*256*3];rgb[..9].copy_from_slice(&[0,0,100,255,255,156,128,0,0]);w.write_image_data(&rgb).unwrap();}let z=decode(&b).unwrap();assert_eq!(z[0],1.);assert_eq!(z[1],-1.);assert!(z[2].is_nan());}
 #[test] fn bad_png_is_not_empty_ocean(){assert!(decode(b"bad").is_err());}
}

pub fn hash_file(path:&Path)->Result<String> {
 use std::io::Read;
 let mut file=fs::File::open(path)?;let mut h=Sha256::new();let mut buffer=vec![0u8;1024*1024];
 loop {let n=file.read(&mut buffer)?;if n==0 {break;}h.update(&buffer[..n]);}
 Ok(h.finalize().iter().map(|v|format!("{v:02x}")).collect())
}
