// 基本形状とは別の追加枠を、地形誤差と距離密度で一頂点ずつ配分する。
struct DetailState {
    step:usize, columns:usize, rows:usize, radius:f64, threshold:f64, max_error:f64, minimum_height_error:f64, normal_spacing:Option<f64>,
    density:Vec<f64>, blocked:HashSet<usize>, rejected_bins:HashMap<(usize,usize),HashSet<usize>>, version:usize, versions:HashMap<usize,usize>, minimum:f64,
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
    pub fn with_seed_mesh(mut self,vertices:&[[usize;2]],faces:&[[usize;3]])->Result<Self> {
        ensure!(vertices.len()>=4&&!faces.is_empty(),"初期メッシュが空です");
        self.points.clear();self.selected.fill(false);self.triangles.clear();self.edges.clear();self.heap.clear();self.next_id=0;
        for &[x,y] in vertices {ensure!(x<self.width&&y<self.height&&!self.selected[y*self.width+x],"初期メッシュの頂点が不正です");self.selected[y*self.width+x]=true;self.points.push([x as i64,y as i64]);}
        let mut area=0i128;
        for &f in faces {ensure!(f.iter().all(|&i|i<self.points.len()),"初期メッシュの参照が不正です");area+=orient(self.points[f[0]],self.points[f[1]],self.points[f[2]]);self.add(f)?;}
        ensure!(area==2*(self.width-1) as i128*(self.height-1) as i128,"初期メッシュの被覆面積が一致しません");
        for (&(a,b),&id) in &self.edges {if let Some(&other)=self.edges.get(&(b,a)) {let f=self.triangles[&id].face;let opposite=*self.triangles[&other].face.iter().find(|&&i|i!=a&&i!=b).unwrap();ensure!(incircle(self.points[f[0]],self.points[f[1]],self.points[f[2]],self.points[opposite])<=0,"初期メッシュがDelaunay条件を満たしません");}}
        Ok(self)
    }
    pub fn start_detail(mut self,radius:f64,threshold:f64,minimum:f64,max_error:f64)->Result<Self> {
        ensure!(max_error.is_finite()&&max_error>0.&&radius>=1.&&threshold>0.&&minimum>0.&&minimum<30.,"追加の密度・角度条件が不正です");
        let step=16;let columns=(self.width-1).div_ceil(step)+1;let rows=(self.height-1).div_ceil(step)+1;
        let mut s=DetailState{step,columns,rows,radius,threshold,max_error,minimum_height_error:0.,normal_spacing:None,density:vec![0.;columns*rows],blocked:HashSet::new(),rejected_bins:HashMap::new(),version:0,versions:HashMap::new(),minimum};
        for &p in &self.points {s.add(p);}self.detail=Some(s);self.detail_refresh();Ok(self)
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
                self.heap.pop();let face=self.triangles[&top.triangle].face;let t=self.evaluate(face);self.heap.push(Entry{error:t.error,triangle:top.triangle,sample:t.sample});self.triangles.insert(top.triangle,t);let s=self.detail.as_mut().unwrap();s.versions.insert(top.triangle,s.version);continue;
            }
            if top.error<=1e-8||self.detail_safe(top){return top;}
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
        let e=self.detail_entry();if e.error<=1e-8{return Ok(false);}let first=self.next_id;self.insert(e)?;
        let p=*self.points.last().unwrap();let s=self.detail.as_mut().unwrap();s.add(p);s.version+=1;
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
    pub fn detail_snapshot(&mut self)->Result<Mesh> {let count=self.points.len();self.advance_with_minimum(count,1.,4)}
}
#[cfg(test)]
mod detail_tests {
    use super::*;
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
