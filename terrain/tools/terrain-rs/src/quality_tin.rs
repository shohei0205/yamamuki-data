// 角度補修と全格子の誤差検証もRust内で行う。標高は元の格子から取得する。
use super::{Builder, Face, Point, orient};
use anyhow::{Result, ensure, bail};
use std::collections::{BTreeMap, HashSet};

pub struct QualityMesh {
    pub points: Vec<Point>,
    pub faces: Vec<Face>,
    pub max_error: f64,
    pub minimum_angle: f64,
    pub rounds: usize,
}

pub fn minimum_angle(p: &[Point], f: Face) -> f64 {
    let mut minimum = 180f64;
    for i in 0..3 {
        let a=p[f[i]];let b=p[f[(i+1)%3]];let c=p[f[(i+2)%3]];
        let u=[(b[0]-a[0]) as f64,(b[1]-a[1]) as f64];
        let v=[(c[0]-a[0]) as f64,(c[1]-a[1]) as f64];
        minimum=minimum.min(((u[0]*v[0]+u[1]*v[1])/(u[0].hypot(u[1])*v[0].hypot(v[1]))).clamp(-1.,1.).acos().to_degrees());
    }
    minimum
}

fn triangulate(p:&[Point],n:usize)->Result<Vec<Face>> {
    let xy=p.iter().map(|p|delaunator::Point{x:p[0] as f64,y:p[1] as f64}).collect::<Vec<_>>();
    let mesh=delaunator::triangulate(&xy);
    let mut area=0i128;let mut result=Vec::with_capacity(mesh.triangles.len()/3);
    for t in mesh.triangles.chunks_exact(3) {
        let mut f=[t[0],t[1],t[2]];let mut a=orient(p[f[0]],p[f[1]],p[f[2]]);
        if a<0 {f.swap(1,2);a=-a;}
        ensure!(a>0,"退化した面を検出しました");area+=a;result.push(f);
    }
    ensure!(area==2*(n-1).pow(2) as i128,"基本TINの被覆面積が一致しません");
    Ok(result)
}

// 各面で誤差最大の点を評価し、範囲を変更せず元の全格子と比較する。
pub fn audit(p:&[Point],faces:&[Face],grid:&[f64],n:usize,tolerance:f64)->Result<(f64,Vec<Point>)> {
    let mut maximum=0f64;let mut additions=Vec::new();
    for &f in faces {
        let [a,b,c]=f.map(|i|p[i]);let den=orient(a,b,c) as f64;
        ensure!(den>0.,"反転した三角形を検出しました");
        let za=grid[a[1] as usize*n+a[0] as usize];
        let db=grid[b[1] as usize*n+b[0] as usize]-za;let dc=grid[c[1] as usize*n+c[0] as usize]-za;
        let mut best=0f64;let mut sample=a;
        for y in a[1].min(b[1]).min(c[1])..=a[1].max(b[1]).max(c[1]) {
            let mut lo=f64::INFINITY;let mut hi=f64::NEG_INFINITY;
            for (u,v) in [(a,b),(b,c),(c,a)] {
                if y<u[1].min(v[1])||y>u[1].max(v[1]) {continue;}
                if u[1]==v[1] {lo=lo.min(u[0].min(v[0]) as f64);hi=hi.max(u[0].max(v[0]) as f64);}
                else {let x=u[0] as f64+(y-u[1]) as f64*(v[0]-u[0]) as f64/(v[1]-u[1]) as f64;lo=lo.min(x);hi=hi.max(x);}
            }
            for x in ((lo-1e-8).ceil() as i64).max(0)..=((hi+1e-8).floor() as i64).min(n as i64-1) {
                let q=[x,y];let wb=orient(c,a,q);let wc=orient(a,b,q);
                if wb<0||wc<0||wb+wc>den as i128 {continue;}
                let predicted=za+wb as f64/den*db+wc as f64/den*dc;
                let error=(predicted-grid[y as usize*n+x as usize]).abs();
                if error>best {best=error;sample=q;}
            }
        }
        maximum=maximum.max(best);if best>tolerance+1e-6 {additions.push(sample);}
    }
    Ok((maximum,additions))
}

fn quality_candidates(p:&[Point],faces:&[Face],n:usize,minimum:f64)->Vec<Point> {
    let existing=p.iter().copied().collect::<HashSet<_>>();let mut segments=Vec::new();
    for (axis,value) in [(0,0),(0,n as i64-1),(1,0),(1,n as i64-1)] {
        let mut edge=p.iter().copied().filter(|p|p[axis]==value).collect::<Vec<_>>();edge.sort_by_key(|p|p[1-axis]);
        segments.extend(edge.windows(2).map(|s|(s[0],s[1])));
    }
    let mut bad=faces.iter().copied().map(|f|(minimum_angle(p,f),f)).filter(|v|v.0<minimum-1e-7).collect::<Vec<_>>();
    bad.sort_by(|a,b|a.0.total_cmp(&b.0).then_with(||a.1.cmp(&b.1)));
    let mut candidates=BTreeMap::<Point,f64>::new();
    for &(angle,f) in &bad {
        let [a,b,c]=f.map(|i|p[i]);let ux=(b[0]-a[0]) as f64;let uy=(b[1]-a[1]) as f64;
        let vx=(c[0]-a[0]) as f64;let vy=(c[1]-a[1]) as f64;let det=2.*(ux*vy-uy*vx);
        let (uu,vv)=(ux*ux+uy*uy,vx*vx+vy*vy);
        let mut q=[a[0] as f64+(uu*vy-vv*uy)/det,a[1] as f64+(ux*vv-vx*uu)/det];
        let encroached=segments.iter().enumerate().filter(|(_, (a,b))|(q[0]-a[0] as f64)*(q[0]-b[0] as f64)+(q[1]-a[1] as f64)*(q[1]-b[1] as f64)< -1e-8).map(|v|v.0).collect::<Vec<_>>();
        if !encroached.is_empty()||q.iter().any(|&v|v<=0.||v>=(n-1) as f64) {
            let selected=if encroached.is_empty() {
                segments.iter().enumerate().min_by(|(_, (a,b)),(_, (c,d))| {
                    let distance=|(a,b):&(Point,Point)| {let u=[(b[0]-a[0]) as f64,(b[1]-a[1]) as f64];let t=(((q[0]-a[0] as f64)*u[0]+(q[1]-a[1] as f64)*u[1])/(u[0]*u[0]+u[1]*u[1])).clamp(0.,1.);(q[0]-a[0] as f64-t*u[0]).hypot(q[1]-a[1] as f64-t*u[1])};
                    distance(&(*a,*b)).total_cmp(&distance(&(*c,*d)))
                }).map(|v|v.0).unwrap()
            } else {*encroached.iter().max_by_key(|&&i| {let (a,b)=segments[i];(b[0]-a[0]).pow(2)+(b[1]-a[1]).pow(2)}).unwrap()};
            let (a,b)=segments[selected];q=[(a[0]+b[0]) as f64/2.,(a[1]+b[1]) as f64/2.];
        }
        let xy=q.map(|v|v.round_ties_even() as i64);
        if xy.iter().all(|&v|v>=0&&v<n as i64)&&!existing.contains(&xy) {candidates.entry(xy).and_modify(|v|*v=v.min(angle)).or_insert(angle);}
        if candidates.len()>=2048 {break;}
    }
    if candidates.is_empty() {
        for (_,f) in bad {
            let edges=[(p[f[0]],p[f[1]]),(p[f[1]],p[f[2]]),(p[f[2]],p[f[0]])];
            let (a,b)=edges.into_iter().max_by_key(|(a,b)|(a[0]-b[0]).pow(2)+(a[1]-b[1]).pow(2)).unwrap();
            let xy=[((a[0]+b[0]) as f64/2.).round_ties_even() as i64,((a[1]+b[1]) as f64/2.).round_ties_even() as i64];
            if !existing.contains(&xy) {candidates.insert(xy,0.);}
        }
    }
    let mut out=candidates.into_iter().collect::<Vec<_>>();out.sort_by(|a,b|a.1.total_cmp(&b.1).then_with(||a.0.cmp(&b.0)));out.into_iter().map(|v|v.0).collect()
}

pub fn generate(grid:&[f64],n:usize,tolerance:f64,minimum:f64)->Result<QualityMesh> {
    let mesh=Builder::new(grid,n,n)?.generate_with_minimum(grid.len(),tolerance,4)?;
    ensure!(mesh.tolerance_reached,"初期TINの誤差条件を満たしません");
    let mut p=mesh.vertices.iter().map(|p|[p[0] as i64,p[1] as i64]).collect::<Vec<_>>();
    let mut selected=p.iter().copied().collect::<HashSet<_>>();
    for round in 0..10000 {
        let f=triangulate(&p,n)?;let angle=f.iter().map(|&f|minimum_angle(&p,f)).fold(180f64,f64::min);
        let additions=if angle<minimum-1e-7 {quality_candidates(&p,&f,n,minimum)} else {
            let (error,extra)=audit(&p,&f,grid,n,tolerance)?;
            if error<=tolerance+1e-6 {return Ok(QualityMesh{points:p,faces:f,max_error:error,minimum_angle:angle,rounds:round});}
            extra
        };
        let before=p.len();for xy in additions {if selected.insert(xy) {p.push(xy);}}
        ensure!(p.len()>before,"誤差・角度の補修候補がなくなりました（最小角{angle}度）");
    }
    bail!("誤差と角度の補修が収束しません")
}

// 親・共有辺・元の頂点を保持したまま、追加だけで品質を補修する。
pub fn refine_seeded(grid:&[f64],n:usize,mut p:Vec<Point>,tolerance:f64,minimum:f64,stop:impl Fn()->bool)->Result<QualityMesh> {
    ensure!(grid.len()==n*n && n>=2 && p.len()>=4,"階層補修の格子・頂点が不正です");
    let mut selected=p.iter().copied().collect::<HashSet<_>>();
    ensure!(selected.len()==p.len() && p.iter().all(|p|p.iter().all(|&v|v>=0&&v<n as i64)),"階層補修の頂点が重複または範囲外です");
    for round in 0..10000 {
        ensure!(!stop(),"階層補修を停止しました");
        ensure!(p.len()<=65535,"階層補修が16ビットの頂点上限に達しました");
        let f=triangulate(&p,n)?;
        let angle=f.iter().map(|&f|minimum_angle(&p,f)).fold(180f64,f64::min);
        let additions=if angle<minimum-1e-7 {quality_candidates(&p,&f,n,minimum)}else{
            let (error,extra)=audit(&p,&f,grid,n,tolerance)?;
            if error<=tolerance+1e-6 {return Ok(QualityMesh{points:p,faces:f,max_error:error,minimum_angle:angle,rounds:round});}
            extra
        };
        let before=p.len();
        for xy in additions {if selected.insert(xy) {ensure!(p.len()<65535,"階層補修が16ビットの頂点上限に達しました");p.push(xy);}}
        ensure!(p.len()>before,"階層補修の候補がありません（最小角{angle}度）");
    }
    bail!("階層補修が収束しません")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn plane_keeps_four_vertices() {let grid=(0..33*33).map(|i|(i%33+i/33) as f64).collect::<Vec<_>>();let m=generate(&grid,33,1.,10.).unwrap();assert_eq!(m.points.len(),4);assert!(m.max_error<1e-8);}
    #[test] fn peaks_and_random_fields_meet_both_conditions() {
        for seed in 0..8 {
            let grid=(0..33*33).map(|i| {let x=i%33;let y=i/33;((i*(97+seed*2))%101) as f64*0.25+if (x as i32-16).pow(2)+(y as i32-16).pow(2)<36 {30.}else{0.}}).collect::<Vec<_>>();
            let m=generate(&grid,33,12.5,10.).unwrap();assert!(m.minimum_angle>=10.-1e-7);assert!(audit(&m.points,&m.faces,&grid,33,12.5).unwrap().0<=12.5+1e-6);
        }
    }
}
