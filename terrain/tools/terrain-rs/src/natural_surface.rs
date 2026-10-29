//! 倍精度のSibson面積座標。固定近傍数による切捨てや別補間への置換はしない。
use anyhow::{Result, ensure, bail};
use delaunator::{Point, Triangulation, EMPTY};
use std::collections::HashSet;

#[derive(Debug)]
pub struct Surface {
    pub points: Vec<[f64; 3]>,
    pub topology: Triangulation,
    pub circles: Vec<[f64; 3]>,
    pub neighbors: Vec<Vec<usize>>,
    pub incident: Vec<Vec<usize>>,
    pub circle_index: rstar::RTree<CircleEntry>,
}
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 { a[0]*b[1]-a[1]*b[0] }
fn clip(poly: &[[f64; 2]], a: f64, b: f64, c: f64) -> Vec<[f64; 2]> {
    let mut out=Vec::with_capacity(poly.len()+1);
    if poly.is_empty() { return out; }
    let mut s=poly[poly.len()-1]; let mut ds=a*s[0]+b*s[1]-c;
    for &e in poly {
        let de=a*e[0]+b*e[1]-c;
        if (ds<=0.) != (de<=0.) { let t=ds/(ds-de); out.push([s[0]+t*(e[0]-s[0]),s[1]+t*(e[1]-s[1])]); }
        if de<=0. { out.push(e); }
        s=e; ds=de;
    }
    out
}
fn area(poly: &[[f64; 2]]) -> f64 {
    if poly.len()<3 { return 0.; }
    let o=poly[0];
    (1..poly.len()-1).map(|i|cross([poly[i][0]-o[0],poly[i][1]-o[1]],[poly[i+1][0]-o[0],poly[i+1][1]-o[1]])).sum::<f64>().abs()*0.5
}
#[derive(Clone,Debug)] pub struct CircleEntry {pub id:usize,box_:rstar::AABB<[f64;2]>}
impl rstar::RTreeObject for CircleEntry {type Envelope=rstar::AABB<[f64;2]>;fn envelope(&self)->Self::Envelope {self.box_}}
impl Surface {
    pub fn new(points: Vec<[f64; 3]>) -> Result<Self> {
        ensure!(points.len()>=3 && points.iter().flatten().all(|v|v.is_finite()), "制御点が不正です");
        let mut unique=HashSet::with_capacity(points.len());
        for p in &points { ensure!(unique.insert((if p[0]==0.{0}else{p[0].to_bits()},if p[1]==0.{0}else{p[1].to_bits()})), "XYが重複しています"); }
        let xy=points.iter().map(|v|Point{x:v[0],y:v[1]}).collect::<Vec<_>>();
        let topology=delaunator::triangulate(&xy);
        ensure!(!topology.triangles.is_empty(), "制御点が一直線です");
        let mut circles=Vec::with_capacity(topology.triangles.len()/3);
        let mut neighbors=vec![Vec::new();points.len()];
        let mut incident=vec![Vec::new();points.len()];
        for (face,t) in topology.triangles.chunks_exact(3).enumerate() {
            for &v in t {incident[v].push(face);}
            let [p,a,b]=[points[t[0]],points[t[1]],points[t[2]]];
            let a=[a[0]-p[0],a[1]-p[1]]; let b=[b[0]-p[0],b[1]-p[1]];
            let det=2.*cross(a,b); ensure!(det!=0. && det.is_finite(), "面積ゼロのDelaunay面です");
            let u=a[0]*a[0]+a[1]*a[1];let v=b[0]*b[0]+b[1]*b[1];
            let dx=(u*b[1]-v*a[1])/det;let dy=(a[0]*v-b[0]*u)/det;
            circles.push([p[0]+dx,p[1]+dy,dx*dx+dy*dy]);
            for i in 0..3 { for j in 0..3 { if i!=j { neighbors[t[i]].push(t[j]); } } }
        }
        for n in &mut neighbors { n.sort_unstable();n.dedup(); }
        let circle_index=rstar::RTree::bulk_load(circles.iter().enumerate().map(|(id,c)|{let r=c[2].sqrt();CircleEntry{id,box_:rstar::AABB::from_corners([c[0]-r,c[1]-r],[c[0]+r,c[1]+r])}}).collect());
        Ok(Self{points,topology,circles,neighbors,incident,circle_index})
    }
    pub fn weights(&self, t: usize, q: [f64; 2]) -> [f64; 3] {
        let ids=&self.topology.triangles[3*t..3*t+3];
        let [a,b,c]=[self.points[ids[0]],self.points[ids[1]],self.points[ids[2]]];
        let ab=[b[0]-a[0],b[1]-a[1]];let ac=[c[0]-a[0],c[1]-a[1]];let aq=[q[0]-a[0],q[1]-a[1]];
        let d=cross(ab,ac);let v=cross(aq,ac)/d;let w=cross(ab,aq)/d;[1.-v-w,v,w]
    }
    pub fn faces_for_rect(&self,rect:[f64;4])->Vec<usize> {
        let bounds=rstar::AABB::from_corners([rect[0],rect[1]],[rect[2],rect[3]]);
        self.circle_index.locate_in_envelope_intersecting(&bounds).filter_map(|v|{let c=self.circles[v.id];let x=c[0].clamp(rect[0],rect[2]);let y=c[1].clamp(rect[1],rect[3]);(((c[0]-x).powi(2)+(c[1]-y).powi(2))<=c[2]*(1.+1e-12)+1e-8).then_some(v.id)}).collect()
    }
    pub fn support_vertices(&self,rect:[f64;4])->Vec<usize> {
        let mut ids=std::collections::BTreeSet::new();
        for t in self.faces_for_rect(rect) {for &v in &self.topology.triangles[3*t..3*t+3] {ids.insert(v);for &u in &self.neighbors[v]{ids.insert(u);}}}
        ids.into_iter().collect()
    }
    pub fn locate(&self,q:[f64;2], start:usize) -> Result<(usize,[f64;3])> {
        ensure!(q.iter().all(|v|v.is_finite()), "照会点が不正です");
        let mut t=start.min(self.circles.len()-1);
        for _ in 0..self.circles.len().min(100000) {
            let w=self.weights(t,q);
            let i=(0..3).min_by(|&i,&j|w[i].total_cmp(&w[j])).unwrap();
            if w[i]>=-1e-12 { return Ok((t,w)); }
            let h=self.topology.halfedges[3*t+(i+1)%3];
            ensure!(h!=EMPTY,"照会点が制御点の凸包外です");t=h/3;
        }
        bail!("面探索が収束しません")
    }
    pub fn evaluate(&self,q:[f64;2], start:&mut usize) -> Result<f64> {
        let (st,w)=self.locate(q,*start)?;*start=st;
        let ids=&self.topology.triangles[3*st..3*st+3];
        // 凸包辺上ではSibson座標は辺の線形座標になる。
        for i in 0..3 {
            let edge=3*st+(i+1)%3;
            if w[i].abs()<1e-11 && self.topology.halfedges[edge]==EMPTY {
                return Ok((0..3).map(|j|w[j]*self.points[ids[j]][2]).sum());
            }
        }
        let mut stack=vec![st];let mut seen=HashSet::new();let mut vertices=Vec::new();let mut vertex_set=HashSet::new();
        while let Some(t)=stack.pop() {
            if !seen.insert(t) { continue; }
            ensure!(seen.len()<=65536,"自然近傍の探索領域が上限を超えました");
            let c=self.circles[t];let d=(c[0]-q[0]).powi(2)+(c[1]-q[1]).powi(2);
            if t!=st && d>c[2]*(1.+1e-12) { continue; }
            for e in 3*t..3*t+3 {
                let v=self.topology.triangles[e];if vertex_set.insert(v) { vertices.push(v); }
                let h=self.topology.halfedges[e];if h!=EMPTY { stack.push(h/3); }
            }
        }
        for &v in &vertices { let p=self.points[v];if (p[0]-q[0]).powi(2)+(p[1]-q[1]).powi(2)<1e-18 { return Ok(p[2]); } }
        let mut bound=128000.;let mut cell=Vec::new();let mut closed=false;
        for _ in 0..40 {
            cell=vec![[-bound,-bound],[bound,-bound],[bound,bound],[-bound,bound]];
            for &v in &vertices {
                let p=self.points[v];let dx=p[0]-q[0];let dy=p[1]-q[1];cell=clip(&cell,dx,dy,(dx*dx+dy*dy)*0.5);
            }
            if cell.iter().all(|p|p[0].abs()<bound*0.999999 && p[1].abs()<bound*0.999999) { closed=true;break; }
            bound*=2.;
        }
        ensure!(closed && !cell.is_empty(),"Voronoiセルが閉じません");
        let mut total=0.;let mut value=0.;
        for &v in &vertices {
            let p=self.points[v];let vx=p[0]-q[0];let vy=p[1]-q[1];let mut poly=cell.clone();
            for &u in &self.neighbors[v] {
                let p=self.points[u];let ux=p[0]-q[0];let uy=p[1]-q[1];
                poly=clip(&poly,ux-vx,uy-vy,(ux*ux+uy*uy-vx*vx-vy*vy)*0.5);
                if poly.is_empty() { break; }
            }
            let weight=area(&poly);total+=weight;value+=weight*self.points[v][2];
        }
        ensure!(total>1e-15 && value.is_finite(),"自然近傍の面積計算に失敗しました");
        Ok(value/total)
    }
}
#[cfg(test)] mod tests {
    use super::*;
    fn points()->Vec<[f64;3]> {vec![[0.,0.,1.],[10.,0.,21.],[10.,10.,51.],[0.,10.,31.],[3.,4.,19.],[8.,6.,35.]]}
    #[test] fn plane_and_walk() {let s=Surface::new(points()).unwrap();let mut start=0;for y in 0..=20 {for x in 0..=20 {let q=[x as f64*0.5,y as f64*0.5];let h=s.evaluate(q,&mut start).unwrap();assert!((h-(1.+2.*q[0]+3.*q[1])).abs()<1e-8,"{q:?}: {h}");}}}
    #[test] fn exact_points() {let s=Surface::new(points()).unwrap();let mut start=0;for p in &s.points {assert!((s.evaluate([p[0],p[1]],&mut start).unwrap()-p[2]).abs()<1e-10);}}
    #[test] fn global_offset_keeps_precision() {let mut p=points();for v in &mut p {v[0]+=35000000.;v[1]+=15000000.;}let s=Surface::new(p).unwrap();let mut t=0;assert!((s.evaluate([35000004.25,15000005.5],&mut t).unwrap()-26.).abs()<1e-7);}
    #[test] fn bad_input_is_not_a_fallback_surface() {assert!(Surface::new(vec![[0.;3];4]).is_err());let s=Surface::new(points()).unwrap();assert!(s.evaluate([-1.,5.],&mut 0).is_err());assert!(s.evaluate([f64::NAN,0.],&mut 0).is_err());}
}
