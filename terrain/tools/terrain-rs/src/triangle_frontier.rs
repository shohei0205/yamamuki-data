//! TFC1。頂点番号を変えず、共有辺を優先した面順で保存する。外側の圧縮は使わない。
use anyhow::{Result, ensure};
use std::collections::HashMap;

#[derive(Default)]
struct State { edges: Vec<[usize; 2]>, vertices: Vec<usize>, last: usize }
fn zig(v: i64) -> u32 { if v >= 0 { (v * 2) as u32 } else { (-v * 2 - 1) as u32 } }
fn signed(v: u32) -> i64 { if v & 1 == 0 { (v / 2) as i64 } else { -((v as i64 + 1) / 2) } }
fn write(out: &mut Vec<u8>, mut v: u32) { while v >= 128 { out.push((v as u8 & 127) | 128); v >>= 7; } out.push(v as u8); }
fn read(src: &[u8], at: &mut usize) -> Result<u32> {
    let mut v = 0;
    for shift in [0, 7, 14] { ensure!(*at < src.len(), "TFC1の途中切断"); let b = src[*at]; *at += 1; v |= ((b & 127) as u32) << shift; if b < 128 { return Ok(v); } }
    anyhow::bail!("TFC1の整数超過")
}
impl State {
    fn update(&mut self, [a,b,c]: [usize;3]) {
        self.edges.splice(0..0, [[b,a],[c,b],[a,c]]); self.edges.truncate(15);
        for v in [a,b,c] { if let Some(i) = self.vertices.iter().position(|&x| x == v) { self.vertices.remove(i); } self.vertices.insert(0,v); self.vertices.truncate(12); }
        self.last = c;
    }
    fn token(&self, a: usize, b: usize, c: usize) -> (usize,u32) {
        if let Some(i) = self.vertices.iter().position(|&v| v == c) { return (i,0); }
        for (tag,base) in [(12,a),(13,b),(14,self.last)] { if c == base + 1 { return (tag,0); } }
        let from_b = c.abs_diff(b) < c.abs_diff(self.last);
        (15, zig(c as i64 - if from_b { b } else { self.last } as i64) * 2 + u32::from(from_b))
    }
}
pub fn encode(faces: &[[usize;3]]) -> Result<Vec<u8>> {
    let mut adjacency = HashMap::new();
    for (i,f) in faces.iter().enumerate() {
        ensure!(f.iter().all(|&v| v <= 65535) && f[0]!=f[1] && f[1]!=f[2] && f[2]!=f[0], "TFC1の三角形が不正");
        for j in 0..3 { ensure!(adjacency.insert([f[j], f[(j+1)%3]], (i,f[(j+2)%3])).is_none(), "TFC1の有向辺が重複"); }
    }
    let mut state = State::default(); let mut visited = vec![false;faces.len()]; let mut first = 0; let mut out = vec![0xc1];
    for _ in 0..faces.len() {
        let mut best = None;
        for (e,&[a,b]) in state.edges.iter().enumerate() {
            let Some(&(id,c)) = adjacency.get(&[a,b]) else { continue }; if visited[id] { continue; }
            let (tag,payload) = state.token(a,b,c);
            let bytes = if payload < 128 {1} else if payload < 16384 {2} else {3};
            let f = faces[id]; let live = (0..3).filter(|&j| adjacency.get(&[f[(j+1)%3],f[j]]).is_some_and(|&(k,_)| !visited[k])).count();
            let score = (1 + if tag==15 {bytes} else {0})*100000 + live*100 + e;
            if best.as_ref().is_none_or(|&(old,_,_,_,_,_)| score < old) { best = Some((score,id,[a,b,c],e,tag,payload)); }
        }
        let (id,face,reference) = if let Some((_,id,f,e,tag,payload)) = best { (id,f,Some((e,tag,payload))) } else {
            while first < faces.len() && visited[first] { first += 1; }
            (first,faces[first],None)
        };
        visited[id] = true;
        if let Some((e,tag,payload)) = reference { out.push((e*16+tag) as u8); if tag==15 {write(&mut out,payload);} }
        else { out.push(240); let mut previous = state.last; for v in face { write(&mut out,zig(v as i64-previous as i64)); previous=v; } }
        state.update(face);
    }
    Ok(out)
}
pub fn decode(src: &[u8], count: usize, vertices: usize) -> Result<Vec<[usize;3]>> {
    ensure!(src.first()==Some(&0xc1) && (1..=65536).contains(&vertices) && count<=2_000_000, "TFC1の形式・個数が不正");
    let mut state=State::default(); let mut at=1; let mut out=Vec::with_capacity(count);
    for _ in 0..count {
        ensure!(at<src.len(), "TFC1の面が不足"); let code=src[at] as usize; at+=1;
        let face = if code==240 { let mut previous=state.last as i64; let mut f=[0;3]; for v in &mut f { previous+=signed(read(src,&mut at)?); ensure!(previous>=0 && previous<vertices as i64,"TFC1の頂点番号超過"); *v=previous as usize; } f }
        else {
            let (e,t)=(code>>4,code&15); ensure!(code<240 && e<state.edges.len(), "TFC1の辺参照不正"); let [a,b]=state.edges[e];
            let c = match t { 0..=11 => {ensure!(t<state.vertices.len(),"TFC1の頂点参照不正"); state.vertices[t] as i64},12=>a as i64+1,13=>b as i64+1,14=>state.last as i64+1,_=>{let p=read(src,&mut at)?; (if p&1==0 {state.last} else {b}) as i64+signed(p>>1)} };
            ensure!(c>=0 && c<vertices as i64,"TFC1の頂点番号超過"); [a,b,c as usize]
        };
        ensure!(face[0]!=face[1] && face[1]!=face[2] && face[2]!=face[0],"TFC1の退化面"); state.update(face); out.push(face);
    }
    ensure!(at==src.len(),"TFC1の余分なデータ"); Ok(out)
}
#[cfg(test)] mod tests {
    use super::*;
    fn canonical(mut f:[usize;3])->[usize;3] {let i=(0..3).min_by_key(|&i|f[i]).unwrap(); f.rotate_left(i); f}
    #[test] fn roundtrip() {for faces in [vec![],vec![[0,1,2],[2,1,3],[65535,10,9]]] {let data=encode(&faces).unwrap(); let mut got=decode(&data,faces.len(),65536).unwrap().into_iter().map(canonical).collect::<Vec<_>>();let mut want=faces.into_iter().map(canonical).collect::<Vec<_>>();got.sort();want.sort();assert_eq!(got,want);} }
    #[test] fn malformed() {for data in [&[0xc1,240,128][..],&[0xc1,255][..],&[0xc1,0][..]] {assert!(decode(data,1,4).is_err());} assert!(encode(&[[0,1,2],[0,1,3]]).is_err());}
    #[test] fn javascript_reference() {
        let Ok(path)=std::env::var("TERRAIN_HIERARCHY_TFC_REFERENCE") else {return};
        let value:serde_json::Value=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let faces:Vec<[usize;3]>=serde_json::from_value(value["faces"].clone()).unwrap();
        // 依存を増やさず、試験用のBase64だけを復元する。
        let alphabet=b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut expected=Vec::new();let(mut bits,mut available)=(0u32,0);
        for c in value["data"].as_str().unwrap().bytes().take_while(|&c|c!=b'=') {
            bits=(bits<<6)|alphabet.iter().position(|&a|a==c).unwrap() as u32; available+=6;
            if available>=8 {available-=8;expected.push((bits>>available) as u8);bits &= (1<<available)-1;}
        }
        assert_eq!(encode(&faces).unwrap(),expected);
    }
}
