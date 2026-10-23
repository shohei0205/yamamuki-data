// 基本形状とは別の追加枠を、地形誤差と距離密度で一頂点ずつ配分する。
struct DetailState {
    step:usize, columns:usize, rows:usize, radius:f64, threshold:f64, free_density:f64, max_error:f64, minimum_height_error:f64, minimum_priority:f64, normal_spacing:Option<f64>,
    density:Vec<f64>, density_changes:Vec<([f64;2],[f64;2])>, blocked:HashSet<usize>, rejected_bins:HashMap<(usize,usize),HashSet<usize>>, version:usize, versions:HashMap<usize,usize>, minimum:f64,
}
impl DetailState {
    fn count(&self,x:usize,y:usize)->f64 {
        let u=x as f64/self.step as f64;let v=y as f64/self.step as f64;let a=u.floor() as usize;let b=v.floor() as usize;let (s,t)=(u-a as f64,v-b as f64);
        let at=|x:usize,y:usize|self.density[y.min(self.rows-1)*self.columns+x.min(self.columns-1)];
        at(a,b)*(1.-s)*(1.-t)+at(a+1,b)*s*(1.-t)+at(a,b+1)*(1.-s)*t+at(a+1,b+1)*s*t
    }
    fn add(&mut self,p:Point) {
        let r=self.radius/self.step as f64;let x=p[0] as f64/self.step as f64;let y=p[1] as f64/self.step as f64;
        let lo_x=((x-r).floor() as isize).max(0) as usize;let hi_x=((x+r).ceil() as usize).min(self.columns-1);
        let lo_y=((y-r).floor() as isize).max(0) as usize;let hi_y=((y+r).ceil() as usize).min(self.rows-1);
        for yy in lo_y..=hi_y {for xx in lo_x..=hi_x {if (xx as f64-x).powi(2)+(yy as f64-y).powi(2)<=r*r {self.density[yy*self.columns+xx]+=1.;}}}
    }
}
fn normal_distance(a:[f64;2],b:[f64;2])->f64 {
    let dot=1.+a[0]*b[0]+a[1]*b[1];
    (2.*(1.-(dot/((1.+a[0]*a[0]+a[1]*a[1])*(1.+b[0]*b[0]+b[1]*b[1])).sqrt()).clamp(-1.,1.))).max(0.).sqrt()
}
fn detail_angle(a:Point,b:Point,c:Point)->f64 {
    let mut minimum=180f64;for (p,q,r) in [(a,b,c),(b,c,a),(c,a,b)] {let u=[(q[0]-p[0]) as f64,(q[1]-p[1]) as f64];let v=[(r[0]-p[0]) as f64,(r[1]-p[1]) as f64];minimum=minimum.min(((u[0]*v[0]+u[1]*v[1])/(u[0].hypot(u[1])*v[0].hypot(v[1]))).clamp(-1.,1.).acos().to_degrees());}minimum
}
impl<'a> Builder<'a> {
    // 復元の途中では候補を評価せず、地形・境界・密度が揃ってから一度だけ評価する。
    pub fn seeded_detail_owned(grid:Vec<f64>,n:usize,vertices:&[[usize;2]],faces:&[[usize;3]],limits:&[f64],max_error:f64,step:usize,density:Vec<f64>)->Result<Builder<'static>> {
        Self::seeded_detail_owned_free(grid,n,vertices,faces,limits,max_error,step,density,0.)
    }
    pub fn seeded_detail_owned_free(grid:Vec<f64>,n:usize,vertices:&[[usize;2]],faces:&[[usize;3]],limits:&[f64],max_error:f64,step:usize,density:Vec<f64>,free_density:f64)->Result<Builder<'static>> {
        ensure!(free_density.is_finite()&&free_density>=0.,"密度の無減点範囲が不正です");
        let mut b=Builder::new_cow(std::borrow::Cow::Owned(grid),n,n,true)?.with_error_limits(limits)?.with_seed_mesh(vertices,faces)?.with_locked_boundary()?.start_detail_with_density((n-1) as f64/2.,4096.,10.,max_error,step,Some(density))?.with_detail_priority_floor(0.1)?;
        b.detail.as_mut().unwrap().free_density=free_density;b.defer_evaluation=false;b.detail_refresh();Ok(b)
    }
    pub fn with_seed_mesh(mut self,vertices:&[[usize;2]],faces:&[[usize;3]])->Result<Self> {
        ensure!(vertices.len()>=4&&!faces.is_empty(),"初期メッシュが空です");
        self.points.clear();self.selected.fill(false);self.triangles.clear();self.edges.clear();self.heap.clear();self.next_id=0;
        for &[x,y] in vertices {ensure!(x<self.width&&y<self.height&&!self.selected[y*self.width+x],"初期メッシュの頂点が不正です");self.selected[y*self.width+x]=true;self.points.push([x as i64,y as i64]);}
        let mut area=0i128;
        for &f in faces {ensure!(f.iter().all(|&i|i<self.points.len()),"初期メッシュの参照が不正です");area+=orient(self.points[f[0]],self.points[f[1]],self.points[f[2]]);self.add(f)?;}
        ensure!(area==2*(self.width-1) as i128*(self.height-1) as i128,"初期メッシュの被覆面積が一致しません");
        for (&(a,b),&id) in &self.edges {if a>b {continue;}if let Some(&other)=self.edges.get(&(b,a)) {let f=self.triangles[&id].face;let opposite=*self.triangles[&other].face.iter().find(|&&i|i!=a&&i!=b).unwrap();ensure!(incircle(self.points[f[0]],self.points[f[1]],self.points[f[2]],self.points[opposite])<=0,"初期メッシュがDelaunay条件を満たしません");}}
        Ok(self)
    }
    pub fn start_detail(self,radius:f64,threshold:f64,minimum:f64,max_error:f64)->Result<Self> { self.start_detail_with_step(radius,threshold,minimum,max_error,16) }
    pub fn start_detail_with_step(self,radius:f64,threshold:f64,minimum:f64,max_error:f64,step:usize)->Result<Self> {
        self.start_detail_with_density(radius,threshold,minimum,max_error,step,None)
    }
    fn start_detail_with_density(mut self,radius:f64,threshold:f64,minimum:f64,max_error:f64,step:usize,density:Option<Vec<f64>>)->Result<Self> {
        ensure!(max_error.is_finite()&&max_error>0.&&radius>=1.&&threshold>0.&&minimum>0.&&minimum<30.,"追加の密度・角度条件が不正です");
        ensure!(step>0,"密度評価間隔が不正です");let columns=(self.width-1).div_ceil(step)+1;let rows=(self.height-1).div_ceil(step)+1;
        let mut s=DetailState{step,columns,rows,radius,threshold,free_density:0.,max_error,minimum_height_error:0.,minimum_priority:1e-8,normal_spacing:None,density:vec![0.;columns*rows],density_changes:Vec::new(),blocked:HashSet::new(),rejected_bins:HashMap::new(),version:0,versions:HashMap::new(),minimum};
        if let Some(counts)=density {
            ensure!(counts.len()==columns*rows&&counts.iter().all(|v|v.is_finite()&&*v>=0.),"周辺密度が不正です");s.density=counts;
        } else {for &p in &self.points {s.add(p);}}
        self.detail=Some(s);self.detail_refresh();Ok(self)
    }
    pub fn replace_detail_density(&mut self,counts:Vec<f64>)->Result<()> {
        let s=self.detail.as_mut().ok_or_else(||anyhow::anyhow!("追加生成を先に開始してください"))?;
        ensure!(counts.len()==s.density.len()&&counts.iter().all(|v|v.is_finite()&&*v>=0.),"周辺密度が不正です");
        if s.free_density>0. && counts.iter().chain(&s.density).all(|&n|n<=s.free_density) {s.density=counts;return Ok(());}
        let decreased=counts.iter().zip(&s.density).any(|(&a,&b)|a<b);
        let mut lo=[f64::INFINITY;2];let mut hi=[f64::NEG_INFINITY;2];
        for (i,(&a,&b)) in counts.iter().zip(&s.density).enumerate() {if a!=b {let p=[(i%s.columns) as f64*s.step as f64,(i/s.columns) as f64*s.step as f64];for k in 0..2 {lo[k]=lo[k].min(p[k]-s.step as f64);hi[k]=hi[k].max(p[k]+s.step as f64);}}}
        s.density=counts;
        if lo[0].is_finite() {s.density_changes.push((lo,hi));s.version+=1;if decreased {self.detail_refresh();}}
        Ok(())
    }
    pub fn detail_point_count(&self)->usize { self.points.len() }
    pub fn with_detail_priority_floor(mut self,priority:f64)->Result<Self> {
        ensure!(priority.is_finite()&&priority>=0.,"優先度下限が不正です");
        self.detail.as_mut().ok_or_else(||anyhow::anyhow!("追加生成を先に開始してください"))?.minimum_priority=priority;
        Ok(self)
    }
    pub fn with_detail_minimum_error(mut self,error:f64)->Result<Self> {
        ensure!(error.is_finite()&&error>=0.,"追加点の最低標高誤差が不正です");
        self.detail.as_mut().ok_or_else(||anyhow::anyhow!("追加生成を先に開始してください"))?.minimum_height_error=error;
        self.detail_refresh();Ok(self)
    }
    // 保存タイルによらず、元DEMと現在の三角形の斜面方向を比較する。
    pub fn with_detail_normals(mut self,spacing:f64)->Result<Self> {
        ensure!(spacing.is_finite()&&spacing>0.,"法線評価の格子間隔が不正です");
        self.detail.as_mut().ok_or_else(||anyhow::anyhow!("追加生成を先に開始してください"))?.normal_spacing=Some(spacing);
        self.detail_refresh();Ok(self)
    }
    fn detail_refresh(&mut self) {
        self.heap.clear();let mut faces:Vec<_>=self.triangles.iter().map(|(&id,t)|(id,t.face)).collect();faces.sort_by_key(|v|v.0);
        for (id,face) in faces {let triangle=self.evaluate(face);self.heap.push(Entry{error:triangle.error,triangle:id,sample:triangle.sample});self.triangles.insert(id,triangle);let s=self.detail.as_mut().unwrap();s.versions.insert(id,s.version);}
    }
    fn detail_safe(&mut self,entry:Entry)->bool {
        let p=[(entry.sample%self.width) as i64,(entry.sample/self.width) as i64];let mut cavity=HashSet::new();let mut visited=HashSet::new();let mut stack=vec![entry.triangle];
        while let Some(id)=stack.pop() {if !visited.insert(id){continue;}let f=self.triangles[&id].face;
            if id!=entry.triangle&&incircle(self.points[f[0]],self.points[f[1]],self.points[f[2]],p)<=0{continue;}
            cavity.insert(id);for (a,b) in [(f[0],f[1]),(f[1],f[2]),(f[2],f[0])] {if let Some(&other)=self.edges.get(&(b,a)){stack.push(other);}}
        }
        let mut candidate_faces=Vec::new();
        for &id in &cavity {let f=self.triangles[&id].face;for (a,b) in [(f[0],f[1]),(f[1],f[2]),(f[2],f[0])] {
            if self.edges.get(&(b,a)).is_some_and(|other|cavity.contains(other)){continue;}
            let (aa,bb)=(self.points[a],self.points[b]);if orient(aa,bb,p)>0 {if detail_angle(aa,bb,p)<self.detail.as_ref().unwrap().minimum-1e-7{return false;}candidate_faces.push([a,b,self.points.len()]);}
        }}
        self.points.push(p);
        let error_ok=candidate_faces.into_iter().all(|f|self.evaluate(f).absolute_error<=self.detail.as_ref().unwrap().max_error+1e-6);
        self.points.pop();error_ok
    }
    fn detail_entry(&mut self)->Entry {
        loop {let top=self.worst();let s=self.detail.as_ref().unwrap();
            if s.versions.get(&top.triangle)!=Some(&s.version) {
                let known=s.versions.get(&top.triangle).copied();let previous=known.unwrap_or(0);
                let face=self.triangles[&top.triangle].face;
                let lo=[face.iter().map(|&i|self.points[i][0]).min().unwrap() as f64,face.iter().map(|&i|self.points[i][1]).min().unwrap() as f64];
                let hi=[face.iter().map(|&i|self.points[i][0]).max().unwrap() as f64,face.iter().map(|&i|self.points[i][1]).max().unwrap() as f64];
                let affected=known.is_none()||s.density_changes[previous..].iter().any(|(a,b)|a[0]<=hi[0]&&a[1]<=hi[1]&&b[0]>=lo[0]&&b[1]>=lo[1]);
                if !affected {let s=self.detail.as_mut().unwrap();s.versions.insert(top.triangle,s.version);continue;}
                self.heap.pop();let t=self.evaluate(face);self.heap.push(Entry{error:t.error,triangle:top.triangle,sample:t.sample});self.triangles.insert(top.triangle,t);let s=self.detail.as_mut().unwrap();s.versions.insert(top.triangle,s.version);continue;
            }
            if top.error<=self.detail.as_ref().unwrap().minimum_priority||self.detail_safe(top){return top;}
            let s=self.detail.as_mut().unwrap();s.blocked.insert(top.sample);s.rejected_bins.entry(((top.sample%self.width)/s.step,(top.sample/self.width)/s.step)).or_default().insert(top.sample);s.versions.remove(&top.triangle);
        }
    }
    pub fn detail_next(&mut self)->(f64,usize) {let e=self.detail_entry();(e.error,e.sample)}
    pub fn detail_next_height_error(&mut self)->f64 {
        let e=self.detail_entry();if e.error<=1e-8{return 0.;}
        let [a,b,c]=self.triangles[&e.triangle].face.map(|i|self.points[i]);
        let p=[(e.sample%self.width) as i64,(e.sample/self.width) as i64];let area=orient(a,b,c) as f64;
        let za=self.height_at(a);let delta=orient(c,a,p) as f64/area*(self.height_at(b)-za)+orient(a,b,p) as f64/area*(self.height_at(c)-za);
        ((self.grid[e.sample]-za)-delta).abs()
    }
    pub fn detail_step(&mut self)->Result<bool> {
        let e=self.detail_entry();if e.error<=self.detail.as_ref().unwrap().minimum_priority{return Ok(false);}let first=self.next_id;self.insert(e)?;
        let p=*self.points.last().unwrap();let s=self.detail.as_mut().unwrap();s.add(p);
        if s.free_density==0. || s.density.iter().any(|&n|n>s.free_density) {let radius=s.radius+s.step as f64;s.density_changes.push(([p[0] as f64-radius,p[1] as f64-radius],[p[0] as f64+radius,p[1] as f64+radius]));s.version+=1;}
        let changed:Vec<_>=(first..self.next_id).flat_map(|id|self.triangles[&id].face.map(|i|self.points[i])).collect();
        let lo=[changed.iter().map(|p|p[0]).min().unwrap(),changed.iter().map(|p|p[1]).min().unwrap()];let hi=[changed.iter().map(|p|p[0]).max().unwrap(),changed.iter().map(|p|p[1]).max().unwrap()];
        let mut restored=Vec::new();
        for y in lo[1] as usize/s.step..=hi[1] as usize/s.step {for x in lo[0] as usize/s.step..=hi[0] as usize/s.step {
            if let Some(bin)=s.rejected_bins.get_mut(&(x,y)) {bin.retain(|&sample|{let px=(sample%self.width) as i64;let py=(sample/self.width) as i64;let remove=px>=lo[0]&&px<=hi[0]&&py>=lo[1]&&py<=hi[1];if remove {restored.push(sample);}!remove});}
        }}
        for sample in &restored {s.blocked.remove(sample);}
        let mut refresh:HashSet<usize>=(first..self.next_id).collect();
        for sample in restored {
            let p=[(sample%self.width) as i64,(sample/self.width) as i64];let mut id=self.next_id-1;
            for _ in 0..self.triangles.len() {let f=self.triangles[&id].face;let edges=[(f[0],f[1]),(f[1],f[2]),(f[2],f[0])];
                if let Some((a,b))=edges.into_iter().find(|&(a,b)|orient(self.points[a],self.points[b],p)<0) {id=*self.edges.get(&(b,a)).unwrap();}
                else {refresh.insert(id);for (a,b) in edges {if orient(self.points[a],self.points[b],p)==0 {if let Some(&other)=self.edges.get(&(b,a)){refresh.insert(other);}}}break;}
            }
        }
        let mut refresh:Vec<_>=refresh.into_iter().collect();refresh.sort_unstable();
        for id in refresh {let f=self.triangles[&id].face;let t=self.evaluate(f);self.heap.push(Entry{error:t.error,triangle:id,sample:t.sample});self.triangles.insert(id,t);let s=self.detail.as_mut().unwrap();s.versions.insert(id,s.version);}
        Ok(true)
    }
    pub fn detail_geometry(&self)->(Vec<[f64;3]>,Vec<Face>) {
        let vertices=self.points.iter().map(|&p|[p[0] as f64,p[1] as f64,self.height_at(p)]).collect();
        let mut faces:Vec<_>=self.triangles.iter().map(|(&id,t)|(id,t.face)).collect();faces.sort_unstable_by_key(|v|v.0);
        (vertices,faces.into_iter().map(|v|v.1).collect())
    }
    pub fn detail_snapshot(&mut self)->Result<Mesh> {let count=self.points.len();self.advance_with_minimum(count,1.,4)}
}
#[cfg(test)]
mod detail_tests {
    use super::*;
    #[test]
    fn density_free_range_skips_penalty_and_invalidates_when_crossed() {
        let n=33;let z:Vec<_>=(0..n*n).map(|i|((i*97)%101) as f64/10.).collect();
        let seed=Builder::new(&z,n,n).unwrap().generate(100,1.).unwrap();let p:Vec<_>=seed.vertices.iter().map(|v|[v[0] as usize,v[1] as usize]).collect();let limits=vec![25.;z.len()];
        let mut base=Builder::seeded_detail_owned(z.clone(),n,&p,&seed.triangles,&limits,25.,4,vec![0.;81]).unwrap();
        let mut free=Builder::seeded_detail_owned_free(z,n,&p,&seed.triangles,&limits,25.,4,vec![100.;81],4096.).unwrap();
        let expected=base.detail_next();assert_eq!(free.detail_next(),expected);
        let before=free.samples;let version=free.detail.as_ref().unwrap().version;free.replace_detail_density(vec![4096.;81]).unwrap();assert_eq!(before,free.samples);assert_eq!(version,free.detail.as_ref().unwrap().version);assert_eq!(free.detail_next(),expected);
        free.replace_detail_density(vec![8192.;81]).unwrap();let got=free.detail_next();assert!((got.0-expected.0/2.).abs()<1e-12);assert_eq!(got.1,expected.1);
        free.replace_detail_density(vec![2000.;81]).unwrap();assert_eq!(free.detail_next(),expected);
    }
    #[test]
    fn free_density_lazy_updates_match_full_refresh_after_each_insertion() {
        let n=33;let z:Vec<_>=(0..n*n).map(|i|((i*97)%101) as f64/10.).collect();let seed=Builder::new(&z,n,n).unwrap().generate(100,1.).unwrap();let p:Vec<_>=seed.vertices.iter().map(|v|[v[0] as usize,v[1] as usize]).collect();let limits=vec![25.;z.len()];
        let make=||Builder::seeded_detail_owned_free(z.clone(),n,&p,&seed.triangles,&limits,25.,4,vec![200.;81],4096.).unwrap().with_detail_priority_floor(0.).unwrap();
        let (mut lazy,mut full)=(make(),make());
        for _ in 0..20 {full.detail_refresh();assert_eq!(lazy.detail_next(),full.detail_next());let a=lazy.detail_step().unwrap();let b=full.detail_step().unwrap();assert_eq!(a,b);if !a{break;}}
        assert_eq!(lazy.detail_geometry(),full.detail_geometry());
    }
    #[test]
    fn supplied_density_preserves_priority_and_rejects_bad_arrays() {
        let n=33;let z:Vec<_>=(0..n*n).map(|i|((i*97)%101) as f64/10.).collect();let seed=Builder::new(&z,n,n).unwrap().generate(100,1.).unwrap();let p:Vec<_>=seed.vertices.iter().map(|v|[v[0] as usize,v[1] as usize]).collect();let limits=vec![25.;z.len()];
        let mut old=Builder::new(&z,n,n).unwrap().with_error_limits(&limits).unwrap().with_seed_mesh(&p,&seed.triangles).unwrap().with_locked_boundary().unwrap().start_detail_with_step(16.,4096.,10.,25.,4).unwrap().with_detail_priority_floor(0.1).unwrap();
        let counts=old.detail.as_ref().unwrap().density.iter().map(|v|v+128.).collect::<Vec<_>>();old.replace_detail_density(counts.clone()).unwrap();
        let mut fast=Builder::seeded_detail_owned(z.clone(),n,&p,&seed.triangles,&limits,25.,4,counts.clone()).unwrap();assert_eq!(fast.detail.as_ref().unwrap().density,counts);assert_eq!(old.detail_next(),fast.detail_next());
        assert!(Builder::seeded_detail_owned(z.clone(),n,&p,&seed.triangles,&limits,25.,4,vec![0.]).is_err());let mut invalid=counts;invalid[0]=f64::NAN;assert!(Builder::seeded_detail_owned(z,n,&p,&seed.triangles,&limits,25.,4,invalid).is_err());
    }
    #[test]
    fn geometry_only_export_matches_snapshot_without_evaluating_samples() {
        let z:Vec<_>=(0..33*33).map(|i|((i*97)%101) as f64/10.).collect();let mut b=Builder::new(&z,33,33).unwrap().start_detail(8.,4096.,10.,25.).unwrap();
        for _ in 0..20 {if !b.detail_step().unwrap(){break;}}
        let samples=b.samples;let (p,f)=b.detail_geometry();assert_eq!(samples,b.samples);let m=b.detail_snapshot().unwrap();assert_eq!(p,m.vertices);assert_eq!(f,m.triangles);
    }
    #[test]
    fn deferred_restore_preserves_all_priorities_and_insertion_order() {
        let n=33;let z:Vec<_>=(0..n*n).map(|i|((i*97)%101) as f64/10.).collect();
        let seed=Builder::new(&z,n,n).unwrap().generate(120,0.5).unwrap();let p:Vec<_>=seed.vertices.iter().map(|p|[p[0] as usize,p[1] as usize]).collect();let limits=vec![25.;z.len()];
        let mut old=Builder::new(&z,n,n).unwrap().with_error_limits(&limits).unwrap().with_seed_mesh(&p,&seed.triangles).unwrap().with_locked_boundary().unwrap().start_detail_with_step(16.,4096.,10.,25.,4).unwrap().with_detail_priority_floor(0.1).unwrap();
        let density=old.detail.as_ref().unwrap().density.clone();
        let mut fast=Builder::seeded_detail_owned(z.clone(),n,&p,&seed.triangles,&limits,25.,4,density).unwrap();assert!(fast.samples<old.samples);
        for _ in 0..20 {assert_eq!(old.detail_next(),fast.detail_next());let a=old.detail_step().unwrap();let b=fast.detail_step().unwrap();assert_eq!(a,b);if !a {break;}}
        let a=old.detail_snapshot().unwrap();let b=fast.detail_snapshot().unwrap();assert_eq!(a.vertices,b.vertices);assert_eq!(a.triangles,b.triangles);
    }
    #[test]
    fn external_density_changes_match_full_recalculation_without_eager_rescan() {
        let z:Vec<_>=(0..33*33).map(|i|((i*97)%101) as f64/10.).collect();
        let mut b=Builder::new(&z,33,33).unwrap().start_detail_with_step(8.,32.,10.,25.,4).unwrap();
        for iteration in 0..8 {let mut counts=b.detail.as_ref().unwrap().density.clone();counts[(iteration*11)%81]+=1.;let before=b.samples;b.replace_detail_density(counts).unwrap();assert_eq!(b.samples,before);
            let score=b.detail_next().0;let faces:Vec<_>=b.triangles.values().map(|t|t.face).collect();let mut expected=0f64;for f in faces {expected=expected.max(b.evaluate(f).error);}assert!((score-expected).abs()<1e-8);
            if !b.detail_step().unwrap(){break;}
        }
    }
    #[test]
    fn priority_floor_skips_geometry_rejection_without_changing_candidates_above_floor() {
        let mut z=vec![0.;33*33];z[16*33+16]=0.05;
        let mut b=Builder::new(&z,33,33).unwrap().start_detail(8.,4096.,10.,25.).unwrap().with_detail_priority_floor(0.1).unwrap();
        let before=b.samples;assert!(!b.detail_step().unwrap());assert_eq!(b.samples,before);assert!(b.detail.as_ref().unwrap().blocked.is_empty());assert_eq!(b.points.len(),4);
        z[16*33+16]=5.;
        let mut a=Builder::new(&z,33,33).unwrap().start_detail(8.,4096.,10.,25.).unwrap();
        let mut b=Builder::new(&z,33,33).unwrap().start_detail(8.,4096.,10.,25.).unwrap().with_detail_priority_floor(0.1).unwrap();
        assert_eq!(a.detail_next(),b.detail_next());assert!(a.detail_step().unwrap()&&b.detail_step().unwrap());assert_eq!(a.points,b.points);
    }
    #[test]
    fn minimum_error_excludes_small_error_before_priority_weights() {
        let mut z=vec![0.;33*33];z[16*33+8]=9.;z[16*33+24]=15.;
        let mut limits=vec![1.;z.len()];limits[16*33+8]=0.01;
        let b=Builder::new(&z,33,33).unwrap().with_error_limits(&limits).unwrap().start_detail(8.,8.,10.,25.).unwrap();
        let mut b=b.with_detail_minimum_error(10.).unwrap();
        assert_eq!(b.detail_next().1,16*33+24);
        assert!(b.detail_step().unwrap());assert!(!b.selected[16*33+8]);
    }
    #[test]
    fn minimum_error_is_strict_and_stops_without_spending_budget() {
        for height in [9.,10.,11.] {
            let mut z=vec![0.;33*33];z[16*33+16]=height;
            let mut b=Builder::new(&z,33,33).unwrap().start_detail(8.,8.,10.,25.).unwrap().with_detail_minimum_error(10.).unwrap();
            assert_eq!(b.detail_step().unwrap(),height>10.);
        }
    }
    #[test]
    fn normal_distance_detects_direction_changes_not_just_steepness() {
        assert!(normal_distance([0.3,0.1],[0.3,0.1])<1e-7);
        assert!(normal_distance([0.3,0.],[-0.3,0.])>0.5);
        assert!(normal_distance([0.,0.],[0.,0.])<1e-7);
    }
    #[test]
    fn detail_normals_do_not_add_points_to_an_exact_plane() {
        let z:Vec<_>=(0..33*33).map(|i|(i%33) as f64*0.5+(i/33) as f64*0.2).collect();
        let mut b=Builder::new(&z,33,33).unwrap().start_detail(8.,8.,10.,25.).unwrap().with_detail_normals(31.25).unwrap();
        assert!(b.detail_next().0<1e-7);
        assert!(!b.detail_step().unwrap());
    }
    #[test]
    fn density_is_monotone_and_does_not_change_with_storage_tile_boundaries() {
        let z=vec![0.;65*65];let mut b=Builder::new(&z,65,65).unwrap().start_detail(16.,8.,10.,25.).unwrap();
        let before=b.detail.as_ref().unwrap().count(32,32);b.detail.as_mut().unwrap().add([32,32]);let s=b.detail.as_ref().unwrap();
        assert!(s.count(32,32)>before);assert!(s.count(31,32)>before);assert!(s.count(33,32)>before);
    }
    #[test]
    fn detail_preserves_angle_and_height_and_updates_priority() {
        let z:Vec<_>=(0..33*33).map(|i|((i*97)%101) as f64/10.).collect();let limits=vec![1.;z.len()];
        let mut b=Builder::new(&z,33,33).unwrap().with_error_limits(&limits).unwrap().start_detail(8.,8.,10.,25.).unwrap();
        for _ in 0..40 {
            let (score,_)=b.detail_next();let faces:Vec<_>=b.triangles.values().map(|t|t.face).collect();let mut expected=0f64;
            for f in faces {let t=b.evaluate(f);expected=expected.max(t.error);}assert!((score-expected).abs()<1e-8);
            if !b.detail_step().unwrap(){break;}
            for t in b.triangles.values(){let [a,c,d]=t.face.map(|i|b.points[i]);assert!(detail_angle(a,c,d)>=10.-1e-7);assert!(t.absolute_error<=25.+1e-6);}
        }
    }
    #[test]
    fn detail_uses_the_selected_quality_error_instead_of_fixed_25m() {
        let mut z=vec![0.;33*33];z[16*33+16]=40.;let limits=vec![1.;z.len()];
        let mut b=Builder::new(&z,33,33).unwrap().with_error_limits(&limits).unwrap().start_detail(8.,8.,10.,50.).unwrap();
        assert!(b.detail_step().unwrap());assert!(b.selected[16*33+16]);
        let maximum=b.triangles.values().map(|t|t.absolute_error).fold(0f64,f64::max);
        assert!(maximum>25.&&maximum<=50.);
    }

}
