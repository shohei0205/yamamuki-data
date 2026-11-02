//! 比較曲面のXYZを独立Rice／上限付き差分Riceへ格納し、全件復号照合する。
#[allow(dead_code)] mod natural_codec;
#[allow(dead_code)] mod natural_surface;
#[allow(dead_code)] mod natural_regional_surface;
#[allow(dead_code)] mod natural_regional_gpu;
mod natural_local_gradient;
use anyhow::{Result,ensure};use clap::Parser;use serde_json::{Value,json};use rusqlite::{Connection,params};
use std::{fs,path::PathBuf,io::{Read,BufReader},collections::{HashMap,BTreeSet}};
#[derive(Parser)]struct Args{#[arg(long)]input:PathBuf,#[arg(long)]output:PathBuf,#[arg(long)]prepare:bool,#[arg(long)]rebuild:bool}
struct Tile{p:Vec<[i32;3]>,map:HashMap<(i32,i32),usize>,depth:usize,deps:BTreeSet<String>}
fn u32le(r:&mut impl Read)->Result<u32>{let mut b=[0;4];r.read_exact(&mut b)?;Ok(u32::from_le_bytes(b))}
fn main()->Result<()>{let a=Args::parse();if a.prepare{return prepare(&a.input,&a.output);}if a.rebuild{return rebuild(&a.input,&a.output);}fs::create_dir_all(&a.output)?;let fullpath=a.output.join("independent.sqlite3");let mixedpath=a.output.join("hybrid.sqlite3");ensure!(!fullpath.exists()&&!mixedpath.exists(),"既存の出力を上書きしません");
 let full=Connection::open(&fullpath)?;let mixed=Connection::open(&mixedpath)?;
 for c in [&full,&mixed]{c.execute_batch("CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID;CREATE TABLE tiles(region TEXT,error INTEGER,x INTEGER,y INTEGER,origin_x INTEGER,origin_y INTEGER,kind TEXT,parents TEXT,points INTEGER,payload BLOB,PRIMARY KEY(region,error,x,y)) WITHOUT ROWID;BEGIN;")?;c.execute("INSERT INTO metadata VALUES('scope',?1)",["三地点比較曲面のXYZ保存容量試験。GPU補助情報なし、全域C1傾きなし、元DEM再監査前"])?;}
 let mut r=BufReader::new(fs::File::open(a.input)?);let mut magic=[0;4];r.read_exact(&mut magic)?;ensure!(&magic==b"NPS1","入力形式が不正");let count=u32le(&mut r)?;
 let mut saved:HashMap<String,Tile>=HashMap::new();let mut index:HashMap<(i32,i32),Vec<(String,usize)>>=HashMap::new();let mut previous_group=String::new();let mut reports=vec![];
 for _ in 0..count {let length=u32le(&mut r)? as usize;let n=u32le(&mut r)? as usize;ensure!(length<8192&&n<=262144,"入力サイズが上限超過");let mut b=vec![0;length];r.read_exact(&mut b)?;let m:Value=serde_json::from_slice(&b)?;let region=m["region"].as_str().unwrap();let error=m["error"].as_i64().unwrap();let z=m["z"].as_i64().unwrap();let x=m["x"].as_i64().unwrap();let y=m["y"].as_i64().unwrap();let origin=[m["ox"].as_i64().unwrap() as i32,m["oy"].as_i64().unwrap() as i32];let key=format!("{region}/{error}/{x}/{y}");
  let parent=match error{400=>500,300=>400,200=>250,125=>200,25=>60,_=>0};let group=format!("{region}/{parent}");
  if group!=previous_group {index.clear();for(k,t)in &saved {if k.starts_with(&(group.clone()+"/")){for(i,p)in t.p.iter().enumerate(){index.entry((p[0],p[1])).or_default().push((k.clone(),i));}}}for v in index.values_mut(){v.sort();}previous_group=group;}
  let mut p=Vec::with_capacity(n);for _ in 0..n{let mut v=[0i32;3];for d in &mut v{*d=u32le(&mut r)? as i32;}p.push(v);}
  let local=p.iter().map(|p|[p[0]-origin[0],p[1]-origin[1],p[2]]).collect::<Vec<_>>();let independent=natural_codec::independent(&local)?;ensure!(natural_codec::decode_global(&independent,origin,&[])?==p,"独立復元が不一致");
  let mut scores:HashMap<String,usize>=HashMap::new();for p in &p {if let Some(v)=index.get(&(p[0],p[1])){for(k,_)in v{*scores.entry(k.clone()).or_default()+=1;}}}let mut candidates=scores.into_iter().collect::<Vec<_>>();candidates.sort_by(|a,b|b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
  let parents=candidates.into_iter().filter(|(k,_)|saved[k].depth<2).take(4).map(|v|v.0).collect::<Vec<_>>();
  let refs=p.iter().map(|p|parents.iter().enumerate().find_map(|(slot,k)|saved[k].map.get(&(p[0],p[1])).map(|&i|(slot,i)))).collect::<Vec<_>>();let ps=parents.iter().map(|k|saved[k].p.clone()).collect::<Vec<_>>();let mut deps=BTreeSet::new();for k in &parents{deps.insert(k.clone());deps.extend(saved[k].deps.iter().cloned());}let depth=parents.iter().map(|k|saved[k].depth+1).max().unwrap_or(0);
  let mut blob=independent.clone();let mut parent_json="[]".to_string();let mut kind="NQC1";let mut actual_depth=0;let mut actual_deps=BTreeSet::new();
  if !parents.is_empty()&&deps.len()<16&&depth<=2 {let delta=natural_codec::delta(&local,&refs,&ps)?;let names=serde_json::to_string(&parents)?;ensure!(natural_codec::decode_global(&delta,origin,&ps)?==p,"差分復元が不一致");if delta.len()+names.len()<independent.len()+2 {blob=delta;parent_json=names;kind="NDC1";actual_depth=depth;actual_deps=deps;}}
  for(c,k,par,data)in [(&full,"NQC1","[]",&independent),(&mixed,kind,parent_json.as_str(),&blob)]{c.execute("INSERT INTO tiles VALUES(?,?,?,?,?,?,?,?,?,?)",params![region,error,x,y,origin[0],origin[1],k,par,n as i64,data])?;}
  reports.push(json!({"region":region,"z":z,"error":error,"x":x,"y":y,"points":n,"independentBytes":independent.len(),"hybridBytes":blob.len(),"kind":kind,"depth":actual_depth,"dependencies":actual_deps.len()}));let map=p.iter().enumerate().map(|(i,p)|((p[0],p[1]),i)).collect();saved.insert(key,Tile{p,map,depth:actual_depth,deps:actual_deps});
 }
 for c in [&full,&mixed]{c.execute_batch("COMMIT;PRAGMA optimize;")?;ensure!(c.query_row("PRAGMA integrity_check",[],|r|r.get::<_,String>(0))?=="ok","SQLite検査失敗");}
 drop(full);drop(mixed);let result=json!({"tiles":count,"verified":count,"independentSqliteBytes":fs::metadata(fullpath)?.len(),"hybridSqliteBytes":fs::metadata(mixedpath)?.len(),"rows":reports});fs::write(a.output.join("report.json"),serde_json::to_vec_pretty(&result)?)?;Ok(())}

// GPU候補だけでなく、曲面と局所傾きを復元する支持点全体を配布用へ抽出する。
fn prepare(input:&PathBuf,output:&PathBuf)->Result<()>{
 use std::io::Write;use natural_regional_surface::Fast;
 let jobs:Vec<Value>=serde_json::from_slice(&fs::read(input)?)?;
 let count=jobs.iter().map(|j|{let n=128000/(j["side"].as_f64().unwrap() as u64);n*n}).sum::<u64>();
 let mut out=std::io::BufWriter::new(fs::File::create(output)?);out.write_all(b"NPS1")?;out.write_all(&(count as u32).to_le_bytes())?;let mut max_quant=0f64;
 for j in jobs {
  let raw=fs::read(j["input"].as_str().unwrap())?;let p=raw.chunks_exact(24).map(|p|std::array::from_fn(|i|f64::from_le_bytes(p[i*8..i*8+8].try_into().unwrap()))).collect::<Vec<[f64;3]>>();let m=Fast::new(p)?;drop(raw);
  let side=j["side"].as_f64().unwrap();let n=(128000./side) as usize;let origin=[j["origin"][0].as_f64().unwrap(),j["origin"][1].as_f64().unwrap()];
  for y in 0..n {for x in 0..n {
   let rect=[x as f64*side-2.,y as f64*side-2.,(x+1) as f64*side+2.,(y+1) as f64*side+2.];
   let mut ids=m.base.support_vertices(rect).into_iter().collect::<BTreeSet<_>>();for i in ids.iter().copied().collect::<Vec<_>>(){ids.extend(m.base.neighbors[i].iter().copied());}
   let ox=((origin[0]+x as f64*side)*16.+0.5).floor() as i32;let oy=((origin[1]+y as f64*side)*16.+0.5).floor() as i32;
   let meta=json!({"region":j["region"],"error":j["error"],"z":j["z"],"x":x,"y":y,"ox":ox,"oy":oy});let b=serde_json::to_vec(&meta)?;
   out.write_all(&(b.len() as u32).to_le_bytes())?;out.write_all(&(ids.len() as u32).to_le_bytes())?;out.write_all(&b)?;
   for i in ids {let p=m.base.points[i];let q=[((p[0]+origin[0])*16.+0.5).floor() as i32,((p[1]+origin[1])*16.+0.5).floor() as i32,(p[2]*8.+0.5).floor() as i32];
    max_quant=max_quant.max((q[0] as f64/16.-origin[0]-p[0]).abs()).max((q[1] as f64/16.-origin[1]-p[1]).abs());
    for v in q {out.write_all(&v.to_le_bytes())?;}
   }
  }}println!("抽出 {} {}",j["region"],j["error"]);
 }
 out.flush()?;fs::write(output.with_extension("quantization.json"),serde_json::to_vec_pretty(&json!({"maximumXYQuantizationDifferenceM":max_quant,"notes":"既存実点の整数座標を維持。試験番兵のみ共通格子へ量子化。DEM再監査前。"}))?)?;Ok(())
}

fn restore(c:&Connection,key:&str,cache:&mut HashMap<String,Vec<[i32;3]>>,stack:&mut BTreeSet<String>,bytes:&mut usize)->Result<Vec<[i32;3]>>{
 if let Some(p)=cache.get(key){return Ok(p.clone());}ensure!(stack.len()<3&&stack.insert(key.into()),"親参照が循環または深さ超過");
 let k=key.split('/').collect::<Vec<_>>();ensure!(k.len()==4,"不正なタイルキー");
 let (ox,oy,parents,blob):(i32,i32,String,Vec<u8>)=c.query_row("SELECT origin_x,origin_y,parents,payload FROM tiles WHERE region=? AND error=? AND x=? AND y=?",params![k[0],k[1].parse::<i64>()?,k[2].parse::<i64>()?,k[3].parse::<i64>()?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
 *bytes+=blob.len();let keys:Vec<String>=serde_json::from_str(&parents)?;ensure!(keys.len()<=4,"親が上限超過");let mut p=vec![];for key in keys {p.push(restore(c,&key,cache,stack,bytes)?);}
 let points=natural_codec::decode_global(&blob,[ox,oy],&p)?;stack.remove(key);cache.insert(key.into(),points.clone());Ok(points)
}
fn rebuild(input:&PathBuf,output:&PathBuf)->Result<()>{
 let job:Value=serde_json::from_slice(&fs::read(input)?)?;let start=std::time::Instant::now();let c=Connection::open_with_flags(job["database"].as_str().unwrap(),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
 let meta=|name:&str|->Result<Value>{let text:String=c.query_row("SELECT value FROM metadata WHERE key=?",[name],|r|r.get(0))?;Ok(serde_json::from_str(&text)?)};
 let region=job["region"].as_str().unwrap();let error=job["error"].as_i64().unwrap();let x=job["x"].as_i64().unwrap();let y=job["y"].as_i64().unwrap();
 let regions=meta("regions")?;let bounds=&regions.as_array().unwrap().iter().find(|r|r["region"]==region).ok_or_else(||anyhow::anyhow!("不正な地域"))?["bounds"];let origin=[bounds[0].as_f64().unwrap(),bounds[1].as_f64().unwrap()];
 let levels=meta("levels")?;let side=levels.as_array().unwrap().iter().find(|r|r["error"]==error).ok_or_else(||anyhow::anyhow!("不正な品質"))?["tileSide"].as_f64().unwrap();let core=[x as f64*side,y as f64*side,(x+1) as f64*side,(y+1) as f64*side];
 let key=format!("{region}/{error}/{x}/{y}");let mut cache=HashMap::new();let mut bytes=0;let integers=restore(&c,&key,&mut cache,&mut BTreeSet::new(),&mut bytes)?;let dependencies=cache.len();
 let points=integers.iter().map(|p|[p[0] as f64/16.-origin[0],p[1] as f64/16.-origin[1],p[2] as f64/8.]).collect();let decode_seconds=start.elapsed().as_secs_f64();
 let m=natural_regional_surface::Fast::new(points)?;let gradient=natural_local_gradient::local_gradients(&m);let native_seconds=start.elapsed().as_secs_f64();
 let (packet,info)=natural_regional_gpu::packet(&m,&gradient,core,json!({"origin":[0.,0.],"coreBounds":core,"worldFieldBounds":core,"sharedSurface":true,"allDemPixelsAudited":false,"source":"SQLite XYZ / Rust rebuild","compressedDependencyBytes":bytes,"dependencyTiles":dependencies,"decodeSeconds":decode_seconds,"topologyAndGradientSeconds":native_seconds-decode_seconds}))?;
 if let Some(parent)=output.parent(){fs::create_dir_all(parent)?;}let tmp=output.with_extension("next");fs::write(&tmp,&packet)?;fs::rename(tmp,output)?;
 fs::write(output.with_extension("json"),serde_json::to_vec_pretty(&json!({"seconds":start.elapsed().as_secs_f64(),"packetBytes":packet.len(),"info":info}))?)?;Ok(())
}
