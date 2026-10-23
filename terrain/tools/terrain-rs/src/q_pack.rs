//! 完了済みSQLiteを保持し、別SQLiteへ16ビット形状を書き出す。
#[allow(dead_code)] mod triangle_frontier;
use anyhow::{Result,ensure,Context};
use clap::Parser;
use flate2::{read::ZlibDecoder,write::ZlibEncoder,Compression};
use rusqlite::{Connection,OpenFlags,params};
use std::{path::PathBuf,io::{Read,Write}};
#[derive(Parser)]
struct Args {#[arg(long)] input:PathBuf,#[arg(long)] output:PathBuf}
fn pack(raw:&[u8])->Result<Vec<u8>> {
 ensure!(raw.len()>=12 && &raw[..4]==b"YQB1","未対応の形状形式です");
 let nv=u32::from_le_bytes(raw[4..8].try_into()?) as usize;
 let nf=u32::from_le_bytes(raw[8..12].try_into()?) as usize;
 ensure!(nv<=65536 && raw.len()==12+12*(nv+nf),"頂点上限または形状サイズが不正です");
 let mut out=b"YQP1".to_vec();out.extend(&raw[4..12]);
 for p in raw[12..12+nv*12].chunks_exact(12) {
  for k in 0..3 {let v=f32::from_le_bytes(p[k*4..k*4+4].try_into()?) as f64;ensure!(v.is_finite(),"非有限の座標です");
   if k<2 {let q=v/15.625;ensure!(q==q.round() && (0.0..=65535.0).contains(&q),"水平座標を損失なく16ビットにできません");out.extend((q as u16).to_le_bytes());}
   else {let q=(v*8.).round();ensure!((-32768.0..=32767.0).contains(&q),"標高が16ビットの範囲外です");out.extend((q as i16).to_le_bytes());}
  }
 }
 for p in raw[12+nv*12..].chunks_exact(4) {let id=u32::from_le_bytes(p.try_into()?);ensure!((id as usize)<nv,"頂点番号が範囲外です");out.extend((id as u16).to_le_bytes());}
 Ok(out)
}
fn verify(raw:&[u8],packed:&[u8])->Result<()> {
 let nv=u32::from_le_bytes(raw[4..8].try_into()?) as usize;
 ensure!(packed.len()==12+(raw.len()-12)/2,"圧縮形状サイズが不正です");
 for i in 0..nv {for k in 0..3 {let old=f32::from_le_bytes(raw[12+i*12+k*4..16+i*12+k*4].try_into()?) as f64;let b: [u8;2]=packed[12+i*6+k*2..14+i*6+k*2].try_into()?;let new=if k<2 {u16::from_le_bytes(b) as f64*15.625}else{i16::from_le_bytes(b) as f64/8.};ensure!((old-new).abs()<=if k<2 {0.}else{0.0625},"座標の復元誤差が上限を超えました");}}
 for (a,b) in raw[12+nv*12..].chunks_exact(4).zip(packed[12+nv*6..].chunks_exact(2)) {ensure!(u32::from_le_bytes(a.try_into()?)==u16::from_le_bytes(b.try_into()?) as u32,"三角形が変化しました");}Ok(())
}
fn main()->Result<()> {
 let args=Args::parse();ensure!(!args.output.exists(),"出力先が既に存在します");
 let src=Connection::open_with_flags(&args.input,OpenFlags::SQLITE_OPEN_READ_ONLY)?;
 for key in ["complete","distributable"] {let v:String=src.query_row("SELECT value FROM metadata WHERE key=?1",[key],|r|r.get(0)).context("全工程の完了記録がありません")?;ensure!(v=="true","全工程が完了していません: {key}");}
 let mut stmt=src.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name IN ('basic','additional','meshes')")?;
 let tables=stmt.query_map([],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;ensure!(!tables.is_empty(),"対応する形状テーブルがありません");
 src.execute("VACUUM INTO ?1",[args.output.to_str().context("出力パスが不正です")?]).context("SQLiteの複製に失敗しました")?;
 let mut dst=Connection::open(&args.output)?;
 dst.execute_batch("UPDATE metadata SET value='false' WHERE key IN ('complete','distributable')")?;
 let result=(||->Result<()> {let tx=dst.transaction()?;let mut count=0u64;
 for table in &tables {let mut select=src.prepare(&format!("SELECT q,x,y,mesh FROM {table}"))?;let mut rows=select.query([])?;
 while let Some(row)=rows.next()? {let blob:Vec<u8>=row.get(3)?;let mut raw=Vec::new();ZlibDecoder::new(&blob[..]).read_to_end(&mut raw)?;let packed=pack(&raw)?;verify(&raw,&packed)?;
 let mut enc=ZlibEncoder::new(Vec::new(),Compression::new(6));enc.write_all(&packed)?;let compressed=enc.finish()?;
 tx.execute(&format!("UPDATE {table} SET mesh=?1 WHERE q=?2 AND x=?3 AND y=?4"),params![compressed,row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?])?;count+=1;}}
 tx.execute("INSERT OR REPLACE INTO metadata VALUES('mesh_encoding',?1)",[r#"{"format":"YQP1","xyStepMeters":15.625,"zStepMeters":0.125,"zSigned":true,"indexBits":16,"compression":"zlib6"}"#])?;
 tx.commit()?;dst.execute_batch("VACUUM; UPDATE metadata SET value='true' WHERE key IN ('complete','distributable')")?;println!("16ビット保存・復元検証完了: {count}メッシュ");Ok(())})();
 if let Err(e)=result {eprintln!("未完成の出力です。元SQLiteは変更していません");return Err(e);}Ok(())
}
#[cfg(test)] mod tests {use super::*;
 fn sample()->Vec<u8> {let mut r=b"YQB1".to_vec();r.extend(3u32.to_le_bytes());r.extend(1u32.to_le_bytes());for v in [0f32,0.,-12.53,512000.,0.,3776.12,0.,512000.,1.] {r.extend(v.to_le_bytes());}for i in [0u32,1,2] {r.extend(i.to_le_bytes());}r}
 #[test]fn roundtrip(){let r=sample();let p=pack(&r).unwrap();verify(&r,&p).unwrap();}
 #[test]fn rejects_off_grid(){let mut r=sample();r[12..16].copy_from_slice(&1f32.to_le_bytes());assert!(pack(&r).is_err());}
 #[test]fn rejects_index(){let mut r=sample();let n=r.len();r[n-4..].copy_from_slice(&3u32.to_le_bytes());assert!(pack(&r).is_err());}
}
