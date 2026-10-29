//! DBを独立に復元・検証し、合格した作業ファイルだけ確定する。
use anyhow::{Result,Context,ensure};
use rusqlite::{Connection,params};
use std::{path::Path,fs,collections::{BTreeSet,HashSet,HashMap}};
use serde_json::{Value,json};
use sha2::{Digest,Sha256};
use crate::{natural_pack::Loader,natural_codec,natural_catalog::{Catalog,XY,WORLD,point_id},natural_inventory::{progress,hash_file}};
type Key=(u32,u32,u32);
fn value(db:&Connection,key:&str)->Result<Value>{let text:String=db.query_row("SELECT value FROM metadata WHERE key=?1",[key],|r|r.get(0))?;Ok(serde_json::from_str(&text)?)}
fn ancestry(db:&Connection,k:Key,seen:&mut BTreeSet<Key>,stack:&mut BTreeSet<Key>)->Result<usize> {
 ensure!(stack.insert(k),"親参照が循環しています");seen.insert(k);ensure!(seen.len()<=16,"依存タイル数が16を超えました");
 let mut s=db.prepare("SELECT slot,parent_z,parent_x,parent_y FROM tile_parents WHERE z=?1 AND x=?2 AND y=?3 ORDER BY slot")?;
 let p=s.query_map(params![k.0,k.1,k.2],|r|Ok((r.get::<_,usize>(0)?,(r.get::<_,u32>(1)?,r.get::<_,u32>(2)?,r.get::<_,u32>(3)?))))?.collect::<rusqlite::Result<Vec<_>>>()?;
 ensure!(p.len()<=4 && p.iter().enumerate().all(|(i,v)|v.0==i && v.1.0+1==k.0),"親のスロット・品質・件数が不正です");
 let mut depth=0;for (_,key) in p {depth=depth.max(ancestry(db,key,seen,stack)?+1);}stack.remove(&k);ensure!(depth<=2,"親深度が2を超えました");Ok(depth)
}
pub fn contents(c:&Catalog,db:&Connection,output:&Path)->Result<Value> {
 ensure!(db.query_row("PRAGMA application_id",[],|r|r.get::<_,i64>(0))?==0x594e4e31 && db.query_row("PRAGMA user_version",[],|r|r.get::<_,i64>(0))?==1,"DB形式の識別子が不正です");
 ensure!(db.query_row("PRAGMA integrity_check",[],|r|r.get::<_,String>(0))?=="ok","SQLite整合性に問題があります");
 let fk:Option<String>=db.prepare("PRAGMA foreign_key_check")?.query_map([],|r|r.get(0))?.next().transpose()?;ensure!(fk.is_none(),"DBに外部キー不整合があります");
 ensure!(value(db,"format")?==json!("YNN") && value(db,"schema_version")?==json!(1),"metadataの形式が不正です");
 let settings:String=db.query_row("SELECT value FROM metadata WHERE key='generation_settings'",[],|r|r.get(0))?;
 let (config_sha,source_sha):(Vec<u8>,Vec<u8>)=db.query_row("SELECT config_sha256,source_sha256 FROM generations WHERE generation_key=1",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
 ensure!(config_sha==Sha256::digest(settings.as_bytes()).to_vec(),"設定本文のSHA-256が一致しません");
 let expected_source:String=c.db.query_row("SELECT value FROM metadata WHERE key='input_manifest_sha256'",[],|r|r.get(0))?;ensure!(source_sha.iter().map(|v|format!("{v:02x}")).collect::<String>()==expected_source,"作業と配布の入力世代が異なります");
 let config:Value=serde_json::from_str(&settings)?;let declared=config["levels"].as_array().context("設定の品質が不正です")?;
 let rows=db.prepare("SELECT z,error_m,parent_z FROM levels ORDER BY z")?.query_map([],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,f64>(1)?,r.get::<_,Option<u32>>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
 ensure!(rows.len()==7 && rows.iter().zip(declared).all(|(r,l)|Some(r.0 as u64)==l["z"].as_u64() && Some(r.1)==l["error_m"].as_f64() && r.2.map(|v|v as u64)==l["parent_z"].as_u64()),"品質表が設定と不一致です");
 let source=c.db.prepare("SELECT x,y,valid FROM inputs ORDER BY y,x")?.query_map([],|r|Ok(((r.get::<_,u32>(0)?,r.get::<_,u32>(1)?),r.get::<_,u64>(2)?)))?.collect::<rusqlite::Result<HashMap<_,_>>>()?;
 let valid_total=source.values().sum::<u64>();let mut expected=BTreeSet::new();
 for (&(x,y),&valid) in &source {if valid>0 {for z in 8..=14 {expected.insert((z,x>>(14-z),y>>(14-z)));}}}
 let keys=db.prepare("SELECT z,x,y FROM tiles ORDER BY z,y,x")?.query_map([],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,u32>(1)?,r.get::<_,u32>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
 ensure!(expected==keys.iter().copied().collect(),"有効な元DEMに基づく必要タイルと一致しません");
 let mut loader=Loader::new();let mut done=0;let mut bytes=0u64;let mut max_dep=0;let mut max_depth=0;let mut maximum_points=0;let mut level_sizes=serde_json::Map::new();
 for &k in &keys {
  let points=loader.restore(db,k)?;
  let (owned,min,max,dependency,size):(usize,i32,i32,usize,u64)=db.query_row("SELECT owned_point_count,min_qz,max_qz,dependency_count,length(payload) FROM tiles WHERE z=?1 AND x=?2 AND y=?3",params![k.0,k.1,k.2],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
  let w=WORLD/(1u64<<k.0) as f64;let owns=|p:[i32;3]|{let x=p[0] as f64*XY;let y=p[1] as f64*XY;x>=k.1 as f64*w && x<(k.1+1) as f64*w && y>=k.2 as f64*w && y<(k.2+1) as f64*w};
  ensure!(points.len()<=262144 && size<=16777216 && owned<=points.len() && points[..owned].iter().all(|&p|owns(p)) && points[owned..].iter().all(|&p|!owns(p)),"本体・余白の配置または上限が不正です");
  ensure!(points.iter().map(|p|p[2]).min()==Some(min) && points.iter().map(|p|p[2]).max()==Some(max),"高さ範囲が不正です");
  let mut unique=HashSet::new();for p in &points {ensure!(unique.insert(point_id([p[0],p[1]])?),"同じXYの重複点があります");}
  let mut deps=BTreeSet::new();let depth=ancestry(db,k,&mut deps,&mut BTreeSet::new())?;ensure!(deps.len()==dependency,"依存タイル件数が不正です");max_dep=max_dep.max(dependency);max_depth=max_depth.max(depth);maximum_points=maximum_points.max(points.len());bytes+=size;done+=1;
  let key=k.0.to_string();let old=level_sizes.get(&key).and_then(|v|v.as_u64()).unwrap_or(0);level_sizes.insert(key,json!(old+size));
  if done%32==0 || done==keys.len() {progress(output,json!({"stage":"verify-packed-integers","done":done,"total":keys.len(),"complete":false}))?;}
 }
 for z in 8..=14 {
  let mut count=0u64;for row in c.db.prepare("SELECT state,info FROM tasks WHERE stage='fit' AND z=?1")?.query_map([z],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))? {let (state,info)=row?;let info:Value=serde_json::from_str(&info)?;ensure!(state=="passed","最終誤差監査が未完了です");ensure!(info.get("reused_cells").and_then(|v|v.as_u64()).unwrap_or(0)==0,"最終監査がキャッシュを再利用しています");count+=info["valid"].as_u64().context("有効画素数がありません")?;let ratio=config["fit_error_ratio"].as_f64().unwrap_or(1.);ensure!(ratio.is_finite() && ratio>0. && ratio<=1.,"近似誤差率が不正です");let error=rows.iter().find(|r|r.0==z).unwrap().1*ratio;ensure!(info["maximum_error_bound"].as_f64().context("誤差の記録がありません")?<=error,"設定された誤差基準の条件が未達です");}
  ensure!(count==valid_total,"最終監査の有効画素数が一致しません: z{z}");
 }
 let roots=source.keys().map(|&(x,y)|(x>>6,y>>6)).collect::<BTreeSet<_>>();let mut checked_roots=BTreeSet::new();let mut unresolved=0u64;
 for row in db.prepare("SELECT root_x,root_y,coverage_sha256,payload FROM coverage ORDER BY root_y,root_x")?.query_map([],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,u32>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,Vec<u8>>(3)?)))? {
  let (x,y,sha,b)=row?;checked_roots.insert((x,y));ensure!(b.len()>=16 && &b[..4]==b"NCV1" && b[4..8]==[8,14,0,0] && u32::from_le_bytes(b[8..12].try_into()?)==4096 && u32::from_le_bytes(b[12..16].try_into()?) as usize==b.len()-16,"coverageヘッダーが不正です");
  let cells=natural_codec::decode_stream(&b[16..],4096)?;ensure!(cells.len()==4096 && cells.iter().all(|v|(0..=2).contains(v)),"coverageの値が不正です");let raw=cells.iter().map(|&v|v as u8).collect::<Vec<_>>();ensure!(Sha256::digest(&raw).to_vec()==sha,"coverageのSHA-256が不一致です");
  for dy in 0..64 {for dx in 0..64 {let value=cells[(dy*64+dx) as usize];if let Some(&valid)=source.get(&(x*64+dx,y*64+dy)){ensure!(value==i64::from(valid>0),"coverageと元DEMの収録状況が不一致です");}unresolved+=u64::from(value==2);}}
 }
 ensure!(roots==checked_roots,"coverageのルート範囲が不足・過剰です");
 Ok(json!({"error_reference":config["fit_reference"].as_str().unwrap_or("original-dem"),"lpf_policy":config["lpf_policy"].as_str().unwrap_or("global-max-adaptive"),"verified_tiles":keys.len(),"geometry_bytes":bytes,"level_geometry_bytes":level_sizes,"max_dependency_tiles":max_dep,"max_parent_depth":max_depth,"max_points_per_tile":maximum_points,"coverage_unresolved":unresolved,"valid_source_pixels_per_quality":valid_total,"complete":false,"distributable":false}))
}
pub fn run(output:&Path,finalize:bool)->Result<()> {
 let lock=Connection::open(output.join("run-lock.sqlite3"))?;lock.execute_batch("BEGIN EXCLUSIVE")?;let c=Catalog::open(output)?;let file=output.join("distribution.partial.sqlite3");
 if !file.exists() {
  let mut old:Value=serde_json::from_slice(&fs::read(output.join("verification.json"))?)?;ensure!(old["complete"]==json!(true),"作業DBがありません");let path=std::path::PathBuf::from(old["path"].as_str().context("確定DBのパスがありません")?);ensure!(path.canonicalize()?.parent()==Some(output.canonicalize()?.as_path()),"確定DBが作業先の外にあります");ensure!(Some(hash_file(&path)?.as_str())==old["file_sha256"].as_str(),"確定済みDBのSHA-256が不一致です");let existing=Connection::open_with_flags(&path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;let report=contents(&c,&existing,output)?;ensure!(report["coverage_unresolved"]==json!(0),"確定DBのcoverageが未完了です");old["reverified"]=json!(true);fs::write(output.join("verification.json"),serde_json::to_vec_pretty(&old)?)?;progress(output,json!({"stage":"complete","report":old,"complete":true}))?;return Ok(());
 }
 let db=Connection::open(&file)?;
 let mut report=contents(&c,&db,output)?;
 if finalize {
  ensure!(report["coverage_unresolved"].as_u64()==Some(0),"未確認のcoverageが残っています。完了にはしません");
  let generation:String=db.query_row("SELECT generation_id FROM generations WHERE generation_key=1",[],|r|r.get(0))?;let fields=generation.split(':').collect::<Vec<_>>();ensure!(fields.len()==3,"生成IDが不正です");
  let final_path=output.join(format!("terrain-natural-{}-{}.sqlite3",fields[1],&fields[2][..16]));ensure!(!final_path.exists(),"確定済みファイルは上書きしません");
  let settings:String=db.query_row("SELECT value FROM metadata WHERE key='generation_settings'",[],|r|r.get(0))?;let config:Value=serde_json::from_str(&settings)?;
  // 形式案の段階では、数学的な完成と公開適否を分ける。
  let distributable=config["production_ready"].as_bool()==Some(true);
  {let tx=db.unchecked_transaction()?;db.execute("INSERT OR REPLACE INTO metadata VALUES('complete','true')",[])?;db.execute("INSERT OR REPLACE INTO metadata VALUES('distributable',?1)",[if distributable{"true"}else{"false"}])?;tx.commit()?;}
  db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);PRAGMA journal_mode=DELETE;")?;drop(db);fs::rename(file,&final_path)?;
  let sha=hash_file(&final_path)?;report["complete"]=json!(true);report["distributable"]=json!(distributable);report["path"]=json!(final_path);report["file_sha256"]=json!(sha);
 }
 fs::write(output.join("verification.json"),serde_json::to_vec_pretty(&report)?)?;progress(output,json!({"stage":if finalize{"complete"}else{"verified-awaiting-finalization"},"report":report,"complete":finalize}))?;Ok(())
}
