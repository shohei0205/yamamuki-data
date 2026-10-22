// 保存境界で面を切らず、頂点の所属セルごとに予算を管理する試作。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct TileBudgetState {
    pub pixels: usize,
    pub offset: usize,
    pub baseline: Vec<usize>,
    pub columns: usize,
    pub rows: usize,
    pub base: usize,
    pub radius: usize,
    pub counts: Vec<usize>,
    pub quotas: Vec<usize>,
    pub errors: Vec<f64>,
    pub angles: Vec<f64>,
    pub received: Vec<usize>,
    pub donated: Vec<usize>,
    pub transfers: usize,
    pub rounds: usize,
    pub window_limit: usize,
    #[serde(skip)]
    pool_phase: bool,
    #[serde(skip)]
    blocked: Vec<bool>,
}
impl TileBudgetState {
    fn cell(&self,x:usize,y:usize)->usize {
        ((y+self.offset)/self.pixels).min(self.rows-1)*self.columns+((x+self.offset)/self.pixels).min(self.columns-1)
    }
    fn can_insert(&self,cell:usize)->bool {
        if self.pool_phase {!self.blocked[cell]} else {self.counts[cell]<self.quotas[cell]}
    }
    fn window_load(&self,x:isize,y:isize)->usize {
        let mut sum=0;
        for yy in y-1..=y+1 {for xx in x-1..=x+1 {
            sum+=if xx<0 || yy<0 || xx>=self.columns as isize || yy>=self.rows as isize {self.base} else {self.counts[yy as usize*self.columns+xx as usize]};
        }}sum
    }
    fn update_window_mask(&mut self,changed:Option<usize>) {
        if self.window_limit==0 {self.blocked.fill(false);return;}
        let (x0,x1,y0,y1)=if let Some(c)=changed {let x=c%self.columns;let y=c/self.columns;(x.saturating_sub(2),(x+2).min(self.columns-1),y.saturating_sub(2),(y+2).min(self.rows-1))} else {(0,self.columns-1,0,self.rows-1)};
        for y in y0..=y1 {for x in x0..=x1 {
            let full=(y as isize-1..=y as isize+1).any(|cy|(x as isize-1..=x as isize+1).any(|cx|self.window_load(cx,cy)>=self.window_limit));
            self.blocked[y*self.columns+x]=full;
        }}
    }
    pub fn maximum_window_load(&self)->usize {
        (-1..=self.rows as isize).flat_map(|y|(-1..=self.columns as isize).map(move |x|self.window_load(x,y))).max().unwrap_or(0)
    }
    fn redistribute(&mut self)->usize {
        let mut offers=vec![Vec::<(usize,usize)>::new();self.quotas.len()];
        for donor in 0..self.quotas.len() {
            if self.errors[donor]>1. || self.angles[donor]<10. {continue;}
            let supply=self.quotas[donor]-self.counts[donor];if supply==0 {continue;}
            let mut targets=Vec::new();
            for recv in 0..self.quotas.len() {
                let dx=(donor%self.columns).abs_diff(recv%self.columns);let dy=(donor/self.columns).abs_diff(recv/self.columns);
                let distance=((dx*dx+dy*dy) as f64).sqrt();
                if distance>0. && distance<=self.radius as f64 && self.errors[recv]>1. && self.counts[recv]>=self.quotas[recv] && self.quotas[recv]<self.baseline[recv]*2 {
                    targets.push((recv,self.errors[recv]/(1.+distance)));
                }
            }
            let weight: f64=targets.iter().map(|v|v.1).sum();if weight==0. {continue;}
            let mut amounts:Vec<_>=targets.iter().map(|v|(v.0,(supply as f64*v.1/weight).floor() as usize,supply as f64*v.1/weight%1.)).collect();
            let rest=supply-amounts.iter().map(|v|v.1).sum::<usize>();
            amounts.sort_by(|a,b|b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
            for (i,(recv,amount,_)) in amounts.into_iter().enumerate() {offers[recv].push((donor,amount+usize::from(i<rest)));}
        }
        let mut moved=0;
        for (recv,list) in offers.into_iter().enumerate() {
            let sum:usize=list.iter().map(|v|v.1).sum();if sum==0 {continue;}
            // 一度に受け取る枠を小さくし、再評価後に次の交換を行う。
            let room=(self.baseline[recv]*2-self.quotas[recv]).min(32).min(sum);
            let mut accepted:Vec<_>=list.iter().map(|&(d,v)|(d,room*v/sum,(room*v)%sum)).collect();
            let rest=room-accepted.iter().map(|v|v.1).sum::<usize>();accepted.sort_by(|a,b|b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
            for (i,(donor,amount,_)) in accepted.into_iter().enumerate() {
                let amount=amount+usize::from(i<rest);self.quotas[donor]-=amount;self.quotas[recv]+=amount;self.donated[donor]+=amount;self.received[recv]+=amount;moved+=amount;
            }
        }
        self.transfers+=moved;moved
    }
}
impl<'a> Builder<'a> {
    fn refresh_tile_errors(&mut self) {
        if let Some(s)=&mut self.tile_budget {s.errors.fill(0.);s.angles.fill(180.);}
        self.heap.clear();let mut faces:Vec<_>=self.triangles.iter().map(|(&id,t)|(id,t.face)).collect();faces.sort_by_key(|v|v.0);
        for (id,face) in faces {
            let q=face.map(|i|self.points[i]);let mut angle=180f64;
            for k in 0..3 {
                let a=q[k];let b=q[(k+1)%3];let c=q[(k+2)%3];let u=[(b[0]-a[0]) as f64,(b[1]-a[1]) as f64];let v=[(c[0]-a[0]) as f64,(c[1]-a[1]) as f64];
                angle=angle.min(((u[0]*v[0]+u[1]*v[1])/(u[0].hypot(u[1])*v[0].hypot(v[1]))).clamp(-1.,1.).acos().to_degrees());
            }
            if let Some(s)=&mut self.tile_budget {
                let lo=s.cell(q.iter().map(|p|p[0]).min().unwrap() as usize,q.iter().map(|p|p[1]).min().unwrap() as usize);
                let hi=s.cell(q.iter().map(|p|p[0]).max().unwrap() as usize,q.iter().map(|p|p[1]).max().unwrap() as usize);
                for y in lo/s.columns..=hi/s.columns {for x in lo%s.columns..=hi%s.columns {let c=y*s.columns+x;s.angles[c]=s.angles[c].min(angle);}}
            }
            let triangle=self.evaluate(face);self.heap.push(Entry{error:triangle.error,triangle:id,sample:triangle.sample});self.triangles.insert(id,triangle);
        }
    }
    fn prepare_tile_budget(mut self,pixels:usize,base:usize,radius:usize,offset:usize)->Result<Self> {
        ensure!(pixels>0 && base>=4 && radius>0,"セル幅・基準予算・近傍範囲が不正です");
        ensure!((self.width-1)%pixels==0 && (self.height-1)%pixels==0,"生成範囲はセル幅で割り切れる必要があります");
        ensure!(offset<pixels,"配分原点のずれがセル幅以上です");
        // 地形がある場合だけ疎な共通骨格を置く。内部の辺は拘束しない。
        if self.worst().error>1. {
            let axis=|length:usize| {let mut out:Vec<_>=(pixels-offset..length).step_by(pixels).collect();out.extend([0,length-1]);out.sort_unstable();out.dedup();out};
            let xs=axis(self.width);let ys=axis(self.height);let mut seeds:Vec<_>=ys.into_iter().flat_map(|y|xs.iter().map(move |&x|[x,y])).collect();
            seeds.sort_by_key(|p|((p[0] as u64*2654435761)^(p[1] as u64*2246822519))&0xffffffff);
            let total=base*((self.width-1)/pixels)*((self.height-1)/pixels);
            self=self.with_seed_points(&seeds,total)?;
        }
        let columns=(self.width-1+offset).div_ceil(pixels);let rows=(self.height-1+offset).div_ceil(pixels);let cells=columns*rows;
        let mut baseline=Vec::new();
        for y in 0..rows {for x in 0..columns {
            let left=(x*pixels).saturating_sub(offset);let right=((x+1)*pixels-offset).min(self.width-1);
            let bottom=(y*pixels).saturating_sub(offset);let top=((y+1)*pixels-offset).min(self.height-1);
            let numerator=base*(right-left)*(top-bottom);ensure!(numerator%(pixels*pixels)==0,"部分セルの面積比例予算が整数になりません");baseline.push(numerator/(pixels*pixels));
        }}
        let mut state=TileBudgetState{pixels,offset,baseline:baseline.clone(),columns,rows,base,radius,counts:vec![0;cells],quotas:baseline,errors:vec![0.;cells],angles:vec![180.;cells],received:vec![0;cells],donated:vec![0;cells],transfers:0,rounds:0,window_limit:0,pool_phase:false,blocked:vec![false;cells]};
        for p in &self.points {let cell=state.cell(p[0] as usize,p[1] as usize);state.counts[cell]+=1;}
        ensure!(state.counts.iter().zip(&state.quotas).all(|(a,b)|a<=b),"初期点がセルの基準予算を超えています");
        self.tile_budget=Some(state);
        Ok(self)
    }
    pub fn start_global_tile_pool(mut self,pixels:usize,base:usize,window_limit:usize)->Result<Self> {
        ensure!(window_limit==0 || window_limit>=9*base,"3×3上限は初期の基準予算9枚分以上が必要です");
        self=self.prepare_tile_budget(pixels,base,2,0)?;
        let total:usize=self.tile_budget.as_ref().unwrap().baseline.iter().sum();self.refresh_tile_errors();
        while self.points.len()<total {
            let top=self.worst();if top.error<=1. {break;}
            let cell=self.tile_budget.as_ref().unwrap().cell(top.sample%self.width,top.sample/self.width);
            self.insert(top)?;self.tile_budget.as_mut().unwrap().counts[cell]+=1;
        }
        self.refresh_tile_errors();let s=self.tile_budget.as_mut().unwrap();
        for i in 0..s.counts.len() {s.donated[i]=s.baseline[i]-s.counts[i];s.quotas[i]=s.counts[i];}
        s.pool_phase=true;s.window_limit=window_limit;s.update_window_mask(None);self.refresh_tile_errors();Ok(self)
    }
    pub fn global_pool_state(&self)->TileBudgetState {self.tile_budget.as_ref().unwrap().clone()}
    pub fn global_pool_next(&mut self,max_vertices:usize)->(f64,usize) {
        if self.points.len()>=max_vertices {return (0.,0);}
        let top=self.worst();(top.error,top.sample)
    }
    pub fn global_pool_step(&mut self,max_vertices:usize)->Result<bool> {
        let (score,_)=self.global_pool_next(max_vertices);if score<=1. {return Ok(false);}
        let top=self.worst();let cell=self.tile_budget.as_ref().unwrap().cell(top.sample%self.width,top.sample/self.width);
        self.insert(top)?;let s=self.tile_budget.as_mut().unwrap();s.counts[cell]+=1;s.quotas[cell]+=1;s.received[cell]+=1;s.transfers+=1;s.update_window_mask(Some(cell));Ok(true)
    }
    pub fn global_pool_snapshot(&mut self)->Result<(Mesh,TileBudgetState)> {
        self.refresh_tile_errors();let state=self.global_pool_state();let count=self.points.len();let mut mesh=self.advance_with_minimum(count,1.,4)?;
        if !mesh.tolerance_reached {mesh.stop_reason="globalPoolOrWindowLimit";}Ok((mesh,state))
    }
    pub fn generate_tile_budgets(self,pixels:usize,base:usize,radius:usize,sharing:bool)->Result<(Mesh,TileBudgetState)> {
        self.generate_tile_budgets_offset(pixels,base,radius,sharing,0)
    }
    pub fn generate_tile_budgets_offset(mut self,pixels:usize,base:usize,radius:usize,sharing:bool,offset:usize)->Result<(Mesh,TileBudgetState)> {
        self=self.prepare_tile_budget(pixels,base,radius,offset)?;
        let total=self.tile_budget.as_ref().unwrap().quotas.iter().sum();
        loop {
            self.refresh_tile_errors();self.tile_budget.as_mut().unwrap().rounds+=1;
            let mut inserted=0;
            while self.points.len()<total {
                let top=self.worst();if top.error<=1. {break;}
                let cell=self.tile_budget.as_ref().unwrap().cell(top.sample%self.width,top.sample/self.width);
                self.insert(top)?;self.tile_budget.as_mut().unwrap().counts[cell]+=1;inserted+=1;
            }
            self.refresh_tile_errors();
            let moved=if sharing {self.tile_budget.as_mut().unwrap().redistribute()} else {0};
            if moved==0 {break;}
            if inserted==0 && self.tile_budget.as_ref().unwrap().rounds>total {break;}
        }
        let state=self.tile_budget.take().unwrap();
        // 制限を外して真の残存誤差を報告する。点の追加は行わない。
        self.refresh_tile_errors();let mut mesh=self.advance_with_minimum(self.points.len(),1.,4)?;
        if !mesh.tolerance_reached {mesh.stop_reason="tileBudgets";}
        Ok((mesh,state))
    }
}
#[cfg(test)]
mod tile_budget_tests {
    use super::*;
    #[test]
    fn plane_keeps_unused_credits_and_counts_each_vertex_once() {
        let z=vec![0.;33*33];let (mesh,s)=Builder::new(&z,33,33).unwrap().generate_tile_budgets(8,16,2,true).unwrap();
        assert_eq!(mesh.vertices.len(),4);assert_eq!(s.counts.iter().sum::<usize>(),4);assert_eq!(s.quotas.iter().sum::<usize>(),256);assert_eq!(s.transfers,0);
    }
    #[test]
    fn credits_are_conserved_local_and_bounded() {
        let mut s=TileBudgetState{pixels:8,offset:0,baseline:vec![16;5],columns:5,rows:1,base:16,radius:1,counts:vec![0,0,16,0,0],quotas:vec![16;5],errors:vec![0.,0.,10.,0.,0.],angles:vec![45.;5],received:vec![0;5],donated:vec![0;5],transfers:0,rounds:0,window_limit:0,pool_phase:false,blocked:vec![false;5]};
        assert_eq!(s.redistribute(),16);assert_eq!(s.quotas[2],32);assert_eq!(s.donated[0],0);assert_eq!(s.donated[4],0);assert_eq!(s.quotas.iter().sum::<usize>(),80);
    }
    #[test]
    fn half_cell_offset_preserves_area_budget_and_outer_coverage() {
        let z=vec![0.;33*33];let (mesh,s)=Builder::new(&z,33,33).unwrap().generate_tile_budgets_offset(8,16,2,true,4).unwrap();
        assert_eq!(s.columns,5);assert_eq!(s.rows,5);assert_eq!(s.baseline.iter().sum::<usize>(),256);assert_eq!(s.quotas.iter().sum::<usize>(),256);assert_eq!(mesh.vertices.len(),4);
        assert_eq!(s.baseline[0],4);assert_eq!(s.baseline[6],16);
    }
    #[test]
    fn insertion_respects_per_cell_limits_and_is_deterministic() {
        let z:Vec<_>=(0..33*33).map(|i|if i%33>8 && i%33<24 && i/33>8 && i/33<24 {((i*97)%101) as f64} else {0.}).collect();
        let run=||Builder::new(&z,33,33).unwrap().generate_tile_budgets(8,12,2,true).unwrap();let (a,s)=run();let (b,t)=run();
        assert_eq!(a.vertices,b.vertices);assert_eq!(s.quotas,t.quotas);assert_eq!(s.counts.iter().sum::<usize>(),a.vertices.len());assert!(s.counts.iter().zip(&s.quotas).all(|(a,b)|a<=b));assert!(s.quotas.iter().all(|&v|v<=24));assert_eq!(s.quotas.iter().sum::<usize>(),192);
        assert!(s.transfers>0);assert!(a.vertices.len()<=192);
    }
}

#[cfg(test)]
mod global_pool_tests {
    use super::*;
    #[test]
    fn every_overlapping_window_is_checked_and_boundaries_have_reserve() {
        let z:Vec<_>=(0..33*33).map(|i|((i*97)%101) as f64).collect();
        let mut b=Builder::new(&z,33,33).unwrap().start_global_tile_pool(8,16,216).unwrap();
        for _ in 0..500 {
            let old=b.points.len();if !b.global_pool_step(500).unwrap(){break;}
            let s=b.global_pool_state();assert_eq!(s.counts.iter().sum::<usize>(),old+1);assert!(s.maximum_window_load()<=216);
            assert_eq!(s.quotas.iter().sum::<usize>(),old+1);
        }
        validate_local(&b);
    }
    fn validate_local(b:&Builder<'_>){
        let area:i128=b.triangles.values().map(|t|{let [a,c,d]=t.face.map(|i|b.points[i]);assert!(orient(a,c,d)>0);for &p in &b.points {assert!(incircle(a,c,d,p)<=0);}orient(a,c,d)}).sum();
        assert_eq!(area,2*(b.width-1) as i128*(b.height-1) as i128);
    }
    #[test]
    fn current_global_priority_is_recomputed_after_each_insertion() {
        let z:Vec<_>=(0..17*17).map(|i|((i*173+19)%251) as f64).collect();
        let mut b=Builder::new(&z,17,17).unwrap().start_global_tile_pool(4,6,81).unwrap();
        for _ in 0..25 {
            let mut expected=0f64;
            for y in 0..17 {for x in 0..17 {
                if b.selected[y*17+x] || !b.tile_budget.as_ref().unwrap().can_insert(b.tile_budget.as_ref().unwrap().cell(x,y)){continue;}
                let p=[x as i64,y as i64];
                for t in b.triangles.values() {
                    let [a,c,d]=t.face.map(|i|b.points[i]);let (u,v,w)=(orient(c,d,p),orient(d,a,p),orient(a,c,p));if u<0||v<0||w<0{continue;}
                    let predicted=(b.height_at(a)*u as f64+b.height_at(c)*v as f64+b.height_at(d)*w as f64)/orient(a,c,d) as f64;
                    expected=expected.max((z[y*17+x]-predicted).abs());break;
                }
            }}
            let (actual,_)=b.global_pool_next(300);assert!((actual-expected).abs()<1e-8);
            if !b.global_pool_step(300).unwrap(){break;}
        }
    }
    #[test]
    fn donors_release_only_unused_initial_credits() {
        let z=vec![0.;33*33];let b=Builder::new(&z,33,33).unwrap().start_global_tile_pool(8,16,216).unwrap();let s=b.global_pool_state();
        assert_eq!(s.donated.iter().sum::<usize>(),252);assert_eq!(s.counts.iter().sum::<usize>(),4);assert_eq!(s.quotas.iter().sum::<usize>(),4);assert_eq!(s.received.iter().sum::<usize>(),0);
    }
    #[test]
    fn zero_window_limit_does_not_block_insertions() {
        let mut seed=17u64;let z:Vec<_>=(0..33*33).map(|_|{seed=seed.wrapping_mul(6364136223846793005).wrapping_add(1);(seed>>32) as f64/1000000.}).collect();
        let mut b=Builder::new(&z,33,33).unwrap().start_global_tile_pool(8,16,0).unwrap();
        while b.global_pool_step(800).unwrap() {}
        let state=b.global_pool_state();
        assert_eq!(state.window_limit,0);assert!(state.blocked.iter().all(|v|!*v));
        assert!(state.maximum_window_load()>216);
        assert_eq!(state.counts.iter().sum::<usize>(),b.points.len());
    }

}
