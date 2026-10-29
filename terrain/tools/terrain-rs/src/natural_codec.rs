//! Rice・FOR・ゼロ連続をブロックごとに比較するNQC1/NDC1エンコーダー。
use anyhow::{Result,ensure};
pub fn var(mut v:u64,out:&mut Vec<u8>) {while v>=128 {out.push(v as u8&127|128);v>>=7;}out.push(v as u8);}
fn zig(v:i64)->u64 {((v as u64)<<1)^((v>>63) as u64)}
fn put_bits(out:&mut Vec<u8>,cursor:&mut usize,v:u64,width:usize) {
 for i in 0..width {if v&(1<<i)!=0 {out[*cursor/8]|=1<<(*cursor%8);}*cursor+=1;}
}
pub fn stream(values:&[i64])->Result<Vec<u8>> {
 let mut out=Vec::new();var(values.len() as u64,&mut out);
 for v in values.chunks(128) {
  ensure!(v.iter().all(|&x|x>-(1i64<<62) && x<(1i64<<62)),"整数の符号化範囲外です");
  let base=*v.iter().min().unwrap();let max=*v.iter().max().unwrap();let width=(64-(max-base).leading_zeros()) as usize;
  let mut chosen=vec![0];var(zig(base),&mut chosen);chosen.push(width as u8);
  let begin=chosen.len();chosen.resize(begin+(v.len()*width).div_ceil(8),0);let mut cursor=begin*8;
  for &x in v {put_bits(&mut chosen,&mut cursor,(x-base) as u64,width);}
  let z=v.iter().map(|&x|zig(x)).collect::<Vec<_>>();
  let mut run=vec![1];let mut i=0;
  while i<v.len() {if z[i]==0 {let begin=i;while i<v.len() && z[i]==0 {i+=1;}run.push(0);var((i-begin) as u64,&mut run);}else{var(z[i]+1,&mut run);i+=1;}}
  if run.len()<chosen.len() {chosen=run;}
  let mean=(z.iter().map(|&x|x as u128).sum::<u128>()/z.len() as u128) as u64;
  let guess=(64-mean.leading_zeros()).saturating_sub(1) as usize;
  for k in guess.saturating_sub(1)..=(guess+1).min(32) {
   let len=z.iter().map(|v|(v>>k) as u128).sum::<u128>()+v.len() as u128*(k+1) as u128;
   if len>chosen.len() as u128*8 {continue;}
   let len=len as usize;let mut rice=vec![2,k as u8];var(len as u64,&mut rice);let begin=rice.len();rice.resize(begin+len.div_ceil(8),0);let mut cursor=begin*8;
   for &x in &z {cursor+=(x>>k) as usize;put_bits(&mut rice,&mut cursor,1,1);put_bits(&mut rice,&mut cursor,x,k);}
   if rice.len()<chosen.len() {chosen=rice;}
  }
  var(chosen.len() as u64,&mut out);out.extend(chosen);
 }
 Ok(out)
}
fn pack(magic:&[u8;4],n:usize,parents:usize,fresh:usize,columns:&[Vec<i64>])->Result<Vec<u8>> {
 ensure!(n<=262144 && parents<=4 && fresh<=n,"全国圧縮プロファイルの点数・親数を超過しています");
 let mut out=magic.to_vec();for v in [n as u32,parents as u32,fresh as u32,0.0625f32.to_bits(),0.125f32.to_bits()] {out.extend(v.to_le_bytes());}
 for c in columns {let b=stream(c)?;out.extend((b.len() as u32).to_le_bytes());out.extend(b);}
 ensure!(out.len()<=16777216,"圧縮BLOBが上限を超えました");Ok(out)
}
pub fn independent(points:&[[i32;3]])->Result<Vec<u8>> {
 let mut c=vec![Vec::new();3];let mut previous=[0i64;3];for p in points {for i in 0..3 {c[i].push(p[i] as i64-previous[i]);previous[i]=p[i] as i64;}}
 pack(b"NQC1",points.len(),0,points.len(),&c)
}
pub fn delta(points:&[[i32;3]], refs:&[Option<(usize,usize)>], parents:&[Vec<[i32;3]>])->Result<Vec<u8>> {
 ensure!(points.len()==refs.len() && parents.len()<=4,"親参照の長さが不正です");
 let mut c=vec![Vec::new();7];let mut last=vec![0i64;parents.len()];let mut prev=[0i64;3];let mut fresh=0;
 for (p,r) in points.iter().zip(refs) {match *r {
  Some((slot,index))=> {ensure!(slot<parents.len() && index<parents[slot].len() && index<(1<<24),"親参照が範囲外です");c[0].push(0);c[1].push(slot as i64);c[2].push(index as i64-last[slot]);last[slot]=index as i64;c[3].push(p[2] as i64-parents[slot][index][2] as i64);}
  None=> {c[0].push(1);fresh+=1;for i in 0..3 {c[4+i].push(p[i] as i64-prev[i]);prev[i]=p[i] as i64;}}
 }}
 pack(b"NDC1",points.len(),parents.len(),fresh,&c)
}
#[cfg(test)] mod tests {use super::*;
 #[test] fn zero_rice_matches_reference(){assert_eq!(stream(&[0,0,0]).unwrap(),vec![3,3,0,0,0]);}
 #[test] fn independent_header(){let b=independent(&[[1,2,3],[-2,0,4]]).unwrap();assert_eq!(&b[..4],b"NQC1");assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()),2);}
 #[test] fn invalid_references_are_errors(){assert!(delta(&[[0;3]],&[Some((0,0))],&[]).is_err());assert!(stream(&[i64::MIN]).is_err());}
}

#[path="sibson_native_decoder.rs"]
#[allow(dead_code)] mod reader;
pub fn decode_global(bytes:&[u8],origin:[i32;2],parents:&[Vec<[i32;3]>])->Result<Vec<[i32;3]>> {reader::decode_global(bytes,origin,parents).map_err(|_|anyhow::anyhow!("全国整数BLOBの復元に失敗しました"))}
#[cfg(test)]mod integer_tests {use super::*;
 #[test]fn origin_and_parent_z_are_exact(){let parent=vec![[350000001,150000003,24000],[350000017,150000019,24016]];let p=vec![[17,19,24019],[33,35,24020]];let b=delta(&p,&[Some((0,1)),None],&[parent.clone()]).unwrap();let actual=decode_global(&b,[350000000,150000000],&[parent]).unwrap();assert_eq!(actual,vec![[350000017,150000019,24019],[350000033,150000035,24020]]);}
 #[test]fn truncated_integer_packet_is_rejected(){let b=independent(&[[1,2,3]]).unwrap();for n in 0..b.len(){assert!(decode_global(&b[..n],[0,0],&[]).is_err());}}
}

pub fn decode_stream(bytes:&[u8],limit:usize)->Result<Vec<i64>> {reader::limited_stream(bytes,limit).map_err(|_|anyhow::anyhow!("整数ストリームの復元に失敗しました"))}
