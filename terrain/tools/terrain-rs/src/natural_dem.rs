//! 元DEMと品質別LPF。欠損値を0 mへ置換せず、重みを正規化する。
use anyhow::{Result,Context,ensure};
use std::{collections::{HashMap,VecDeque,BTreeSet},fs,path::{Path,PathBuf},sync::{Arc,Mutex},time::Instant};
use rusqlite::{Connection,params,OptionalExtension};
use rayon::prelude::*;
use serde_json::json;
use crate::natural_inventory::{decode,hash,open_work,progress};
pub const STEP:f64=40075016.68557849/4194304.;
pub struct Source {cache:PathBuf,sha:Arc<HashMap<(u32,u32),String>>,tiles:HashMap<(i32,i32),Arc<Vec<f32>>>,order:VecDeque<(i32,i32)>}
impl Source {
 pub fn new(cache:PathBuf,sha:Arc<HashMap<(u32,u32),String>>)->Self {Self{cache,sha,tiles:HashMap::new(),order:VecDeque::new()}}
 fn tile(&mut self,x:i32,y:i32)->Result<Arc<Vec<f32>>> {
  if let Some(v)=self.tiles.get(&(x,y)){return Ok(v.clone());}
  let data=if let Some(expected)=self.sha.get(&(x as u32,y as u32)) {
   let bytes=fs::read(self.cache.join("14").join(x.to_string()).join(format!("{y}.png")))?;ensure!(&hash(&bytes)==expected,"元PNGが監査後に変更されました: {x}/{y}");decode(&bytes)?.into_iter().map(|v|v as f32).collect()
  }else{vec![f32::NAN;65536]};
  let v=Arc::new(data);if self.tiles.len()>=256 {if let Some(k)=self.order.pop_front(){self.tiles.remove(&k);}}
  self.order.push_back((x,y));self.tiles.insert((x,y),v.clone());Ok(v)
 }
 pub fn raster(&mut self,left:i32,top:i32,w:usize,h:usize)->Result<Vec<f32>> {
  let mut out=vec![f32::NAN;w*h];
  for ty in top.div_euclid(256)..=(top+h as i32-1).div_euclid(256) {for tx in left.div_euclid(256)..=(left+w as i32-1).div_euclid(256) {
   let tile=self.tile(tx,ty)?;let x0=left.max(tx*256);let y0=top.max(ty*256);let x1=(left+w as i32).min((tx+1)*256);let y1=(top+h as i32).min((ty+1)*256);
   for y in y0..y1 {let a=((y-top) as usize)*w+(x0-left) as usize;let b=((y-ty*256)*256+x0-tx*256) as usize;let n=(x1-x0) as usize;out[a..a+n].copy_from_slice(&tile[b..b+n]);}
  }}Ok(out)
 }
}
pub fn weights(sigma:f64)->Vec<f64> {let sigma=sigma/STEP;if sigma<0.01{return vec![1.];}let r=(4.*sigma+0.5).floor() as i32;let mut w=(-r..=r).map(|x|(-0.5*(x as f64/sigma).powi(2)).exp()).collect::<Vec<_>>();let sum=w.iter().sum::<f64>();for a in &mut w{*a/=sum;}w}
pub fn filter_direct(raw:&[f32],n:usize,weights:&[f64])->Result<(Vec<f32>,f64,u64)> {
 let r=weights.len()/2;let w=n+2*r;ensure!(raw.len()==w*w,"LPF入力のサイズが不正です");
 let mut a=vec![0f64;w*n];let mut b=vec![0f64;w*n];
 for y in 0..w {for (k,&v) in weights.iter().enumerate(){for x in 0..n{let z=raw[y*w+x+k];if z.is_finite(){a[y*n+x]+=v*z as f64;b[y*n+x]+=v;}}}}
 let mut value=vec![0f64;n*n];let mut total=vec![0f64;n*n];
 for (k,&v) in weights.iter().enumerate(){for y in 0..n{for x in 0..n{value[y*n+x]+=v*a[(y+k)*n+x];total[y*n+x]+=v*b[(y+k)*n+x];}}}
 let mut out=vec![f32::NAN;n*n];let mut max=0f64;let mut count=0;
 for y in 0..n{for x in 0..n{let i=y*n+x;let original=raw[(y+r)*w+x+r];if original.is_finite(){ensure!(total[i]>0.,"LPFの有効重みがありません");let z=(value[i]/total[i]) as f32;out[i]=z;max=max.max((z as f64-original as f64).abs());count+=1;}}}
 Ok((out,max,count))
}
// 2成分を実部・虚部に詰め、分子と有効重みを同じFFTで畳み込む。
#[derive(Clone,Copy,Default)] struct Complex {re:f64,im:f64}
fn fft(a:&mut [Complex], inverse:bool) {
 let n=a.len();let mut j=0;
 for i in 1..n {let mut bit=n>>1;while j&bit!=0 {j^=bit;bit>>=1;}j^=bit;if i<j{a.swap(i,j);}}
 let mut width=2;
 while width<=n {let theta=if inverse {2.}else{-2.}*std::f64::consts::PI/width as f64;let base=Complex{re:theta.cos(),im:theta.sin()};
  for begin in (0..n).step_by(width){let mut w=Complex{re:1.,im:0.};for j in 0..width/2 {let x=a[begin+j];let y=a[begin+j+width/2];let y=Complex{re:y.re*w.re-y.im*w.im,im:y.re*w.im+y.im*w.re};a[begin+j]=Complex{re:x.re+y.re,im:x.im+y.im};a[begin+j+width/2]=Complex{re:x.re-y.re,im:x.im-y.im};w=Complex{re:w.re*base.re-w.im*base.im,im:w.re*base.im+w.im*base.re};}}
  width*=2;
 }
 if inverse {for x in a {x.re/=n as f64;x.im/=n as f64;}}
}
fn convolve(a:&mut [Complex], kernel:&[Complex]) {fft(a,false);for (a,b) in a.iter_mut().zip(kernel){*a=Complex{re:a.re*b.re-a.im*b.im,im:a.re*b.im+a.im*b.re};}fft(a,true);}
pub fn filter_fft(raw:&[f32],n:usize,weights:&[f64])->Result<(Vec<f32>,f64,u64)> {
 let r=weights.len()/2;let w=n+2*r;ensure!(raw.len()==w*w,"LPF入力サイズ不一致");let len=(w+weights.len()-1).next_power_of_two();
 let mut kernel=vec![Complex::default();len];for (i,&v) in weights.iter().enumerate(){kernel[i].re=v;}fft(&mut kernel,false);
 let mut scratch=vec![Complex::default();len];let mut horizontal=vec![Complex::default();w*n];
 for y in 0..w {scratch.fill(Complex::default());for x in 0..w {let z=raw[y*w+x];if z.is_finite(){scratch[x]=Complex{re:z as f64,im:1.};}}convolve(&mut scratch,&kernel);horizontal[y*n..(y+1)*n].copy_from_slice(&scratch[2*r..2*r+n]);}
 let mut out=vec![f32::NAN;n*n];let mut maximum=0f64;let mut count=0;
 for x in 0..n {scratch.fill(Complex::default());for y in 0..w{scratch[y]=horizontal[y*n+x];}convolve(&mut scratch,&kernel);
  for y in 0..n{let original=raw[(y+r)*w+x+r];if original.is_finite(){let v=scratch[y+2*r];ensure!(v.im>0. && v.re.is_finite(),"FFTの有効重みが不正です");let z=(v.re/v.im) as f32;out[y*n+x]=z;maximum=maximum.max((z as f64-original as f64).abs());count+=1;}}
 }Ok((out,maximum,count))
}
pub fn filter(raw:&[f32],n:usize,weights:&[f64])->Result<(Vec<f32>,f64,u64)> {
 if weights.len()>33 {filter_fft(raw,n,weights)}else{filter_direct(raw,n,weights)}
}
pub fn load_lpf(path:&Path)->Result<(usize,f64,Vec<f32>)> {
 let b=zstd::stream::decode_all(fs::File::open(path)?)?;ensure!(b.len()>=16 && &b[..4]==b"NLP1","LPFキャッシュの形式が不正です");
 let n=u32::from_le_bytes(b[4..8].try_into()?) as usize;let sigma=f64::from_le_bytes(b[8..16].try_into()?);ensure!(n<=4096 && b.len()==16+4*n*n,"LPFキャッシュ長が不正です");
 Ok((n,sigma,b[16..].chunks_exact(4).map(|p|f32::from_le_bytes(p.try_into().unwrap())).collect()))
}
fn save_lpf(path:&Path,n:usize,sigma:f64,values:&[f32])->Result<String> {
 let mut raw=b"NLP1".to_vec();raw.extend((n as u32).to_le_bytes());raw.extend(sigma.to_le_bytes());for v in values{raw.extend(v.to_le_bytes());}
 let bytes=zstd::stream::encode_all(raw.as_slice(),3)?;let digest=hash(&bytes);fs::create_dir_all(path.parent().unwrap())?;let tmp=path.with_extension("partial");fs::write(&tmp,bytes)?;fs::rename(tmp,path)?;Ok(digest)
}
pub fn run(output:&Path,workers:usize)->Result<()> {
 ensure!(workers>0 && workers<=256,"並列数が不正です");let guard=Connection::open(output.join("run-lock.sqlite3"))?;guard.execute_batch("BEGIN EXCLUSIVE")?;
 let db=open_work(output)?;let ready:String=db.query_row("SELECT value FROM metadata WHERE key='inventory_complete'",[],|r|r.get(0)).context("入力検査が未完了です")?;ensure!(ready=="true","入力検査が未完了です");
 let identity:String=db.query_row("SELECT value FROM metadata WHERE key='identity'",[],|r|r.get(0))?;let identity:serde_json::Value=serde_json::from_str(&identity)?;let cache=PathBuf::from(identity["cache"].as_str().context("PNGパス不明")?);
 let mut query=db.prepare("SELECT x,y,sha FROM inputs ORDER BY y,x")?;
 let sha=Arc::new(query.query_map([],|r|Ok(((r.get::<_,u32>(0)?,r.get::<_,u32>(1)?),r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<HashMap<_,_>>>()?);
 let blocks=sha.keys().map(|&(x,y)|(x/4,y/4)).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
 let sources=(0..workers).map(|_|Mutex::new(Source::new(cache.clone(),sha.clone()))).collect::<Vec<_>>();let pool=rayon::ThreadPoolBuilder::new().num_threads(workers).build()?;
 for (z,error) in [(8,800.),(9,400.),(10,200.),(11,100.),(12,50.),(13,25.),(14,12.5)] {
  let key=format!("lpf-sigma-z{z}");let saved:Option<String>=db.query_row("SELECT value FROM metadata WHERE key=?1",[&key],|r|r.get(0)).optional()?;
  if saved.is_some(){continue;}
  let started=Instant::now();let mut sigma=error/2.;let mut passed=false;
  for attempt in 0..12 {
   let weights=weights(sigma);let r=weights.len()/2;let mut maximum=0f64;let mut count=0u64;let mut done=0;
   for batch in blocks.chunks(workers) {
    let cached=batch.iter().map(|&(x,y)|->Result<_>{
     let row:Option<(String,String,String)>=db.query_row("SELECT a.path,a.sha,t.info FROM artifacts a JOIN tasks t ON a.stage=t.stage AND a.z=t.z AND a.x=t.x AND a.y=t.y WHERE a.stage='lpf' AND a.z=?1 AND a.x=?2 AND a.y=?3",params![z,x,y],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
     if let Some((path,sha,info))=row {let info:serde_json::Value=serde_json::from_str(&info)?;if info["sigma"].as_f64()==Some(sigma) && Path::new(&path).is_file(){ensure!(hash(&fs::read(&path)?)==sha,"LPFキャッシュのSHA-256が不一致です");return Ok(Some((x,y,PathBuf::from(path),sha,info["max_error"].as_f64().context("LPF監査値不明")?,info["valid_pixels"].as_u64().context("LPF画素数不明")?)));}}
     Ok(None)
    }).collect::<Result<Vec<_>>>()?;
    let results=pool.install(||batch.par_iter().zip(&cached).map(|(&(x,y),old)|->Result<_>{if let Some(v)=old {return Ok(v.clone());}let i=rayon::current_thread_index().unwrap();let mut source=sources[i].lock().map_err(|_|anyhow::anyhow!("PNGキャッシュが破損しました"))?;
     let raw=source.raster(x as i32*1024-r as i32,y as i32*1024-r as i32,1024+2*r,1024+2*r)?;drop(source);let (values,max,valid)=filter(&raw,1024,&weights)?;
     let path=output.join("lpf").join(format!("z{z}-sigma-{sigma}" )).join(format!("{x}-{y}.nlp"));let sha=save_lpf(&path,1024,sigma,&values)?;Ok((x,y,path,sha,max,valid))}).collect::<Result<Vec<_>>>())?;
    let tx=db.unchecked_transaction()?;
    for (x,y,path,sha,max,valid) in results {maximum=maximum.max(max);count+=valid;done+=1;
     tx.execute("INSERT OR REPLACE INTO artifacts VALUES('lpf',?1,?2,?3,?4,?5)",params![z,x,y,path.to_string_lossy(),sha])?;
     tx.execute("INSERT OR REPLACE INTO tasks VALUES('lpf',?1,?2,?3,'audited',?4)",params![z,x,y,json!({"sigma":sigma,"max_error":max,"valid_pixels":valid}).to_string()])?;
    }tx.commit()?;
    progress(output,json!({"stage":"lpf-audit","z":z,"error_m":error,"sigma_m":sigma,"attempt":attempt,"done":done,"total":blocks.len(),"max_error":maximum,"limit":error/2.,"valid_pixels":count,"elapsed_seconds":started.elapsed().as_secs_f64(),"complete":false}))?;
    if maximum>error/2. {break;}
   }
   if done==blocks.len() && maximum<=error/2. {
    db.execute("INSERT INTO metadata VALUES(?1,?2)",params![key,sigma.to_string()])?;passed=true;break;
   }sigma*=0.5;
  }
  ensure!(passed,"LPFの全国共通sigmaが誤差条件を満たしません: z{z}");
 }
 progress(output,json!({"stage":"lpf-complete","next_stage":"fit-global-controls","complete":false,"distributable":false}))?;Ok(())
}
#[cfg(test)] mod tests {use super::*;
 #[test] fn constant_and_nodata(){let w=weights(25.);let n=8;let size=n+2*(w.len()/2);let mut a=vec![123f32;size*size];a[(size/2)*size+size/2]=f32::NAN;let (v,max,count)=filter(&a,n,&w).unwrap();assert_eq!(max,0.);assert_eq!(count,n as u64*n as u64-1);assert_eq!(v.iter().filter(|v|v.is_finite()).count(),count as usize);}
 #[test] fn lpf_is_identity_for_very_small_sigma(){let a=(0..64).map(|v|v as f32).collect::<Vec<_>>();let (v,e,n)=filter(&a,8,&weights(0.)).unwrap();assert_eq!(v,a);assert_eq!(e,0.);assert_eq!(n,64);}
}

#[cfg(test)] mod fft_tests {use super::*;
 #[test] fn gaussian_fft_matches_direct_with_nodata(){let n=32;let weights=weights(100.);let w=n+2*(weights.len()/2);let raw=(0..w*w).map(|i|if i%17==0{f32::NAN}else{((i*137)%350000) as f32*0.01}).collect::<Vec<_>>();let (a,ea,na)=filter_direct(&raw,n,&weights).unwrap();let (b,eb,nb)=filter_fft(&raw,n,&weights).unwrap();assert_eq!(na,nb);assert!((ea-eb).abs()<0.001);for (a,b) in a.iter().zip(b){assert!(a.is_nan() && b.is_nan() || (*a-b).abs()<0.001,"{a}/{b}");}}
}
