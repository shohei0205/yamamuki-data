//! 適合済み入力の証明を保持し、挿入前後の支持円に触れる入力だけを無効化する。
use super::*;
pub(super) struct Verified {flags:Vec<u8>,count:usize,index:rstar::RTree<InputBox>,lookup:HashMap<(i32,i32),usize>,pub enabled:bool}
impl Verified {
    pub fn empty(inputs: &[Input]) -> Self {
        Self { flags: vec![0; inputs.len().div_ceil(8)], count: 0,
            index: input_index(inputs),
            lookup: inputs.iter().enumerate().map(|(k,i)| ((i.x,i.y),k)).collect(),
            enabled: true }
    }
    pub fn clear(&mut self) { self.flags.fill(0); self.count = 0; }

    pub fn load(s:&Store,inputs:&[Input],signature:&str)->Result<Self> {
        s.db.execute_batch("CREATE TABLE IF NOT EXISTS fit_verified(id INTEGER PRIMARY KEY CHECK(id=1),signature TEXT,epoch INTEGER,flags BLOB)")?;
        let disabled=s.db.query_row("SELECT 1 FROM meta WHERE key='benchmark_disable_fit_verified' AND EXISTS(SELECT 1 FROM meta WHERE key='benchmark_only')",[],|_|Ok(())).optional()?.is_some();
        let enabled=std::env::var_os("TERRAIN_FIT_NO_VALID_CACHE").is_none()&&!disabled;let cache_signature=format!("fit-valid-v1:E25:up4:before-after:{signature}");let len=inputs.len().div_ceil(8);
        let row=s.db.query_row("SELECT signature,epoch,flags FROM fit_verified WHERE id=1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,Vec<u8>>(2)?))).optional()?;
        let flags=match row {Some((sig,epoch,flags)) if enabled&&sig==cache_signature&&epoch==s.epoch(13)?&&flags.len()==len=>flags,_=>vec![0;len]};
        let count=flags.iter().map(|v|v.count_ones() as usize).sum();
        Ok(Self{flags,count,index:input_index(inputs),lookup:inputs.iter().enumerate().map(|(k,i)|((i.x,i.y),k)).collect(),enabled})
    }
    pub fn count(&self)->usize {self.count}
    pub fn seed_from_audit(&mut self,s:&Store)->Result<()> {
        let mut stmt=s.db.prepare("SELECT x,y,valid,up,down FROM audit WHERE z=13 AND epoch=?")?;
        let rows=stmt.query_map([s.epoch(13)?],|r|Ok((r.get::<_,i32>(0)?,r.get::<_,i32>(1)?,r.get::<_,u64>(2)?,r.get::<_,f64>(3)?,r.get::<_,f64>(4)?)))?;
        for row in rows {let (x,y,_valid,up,down)=row?;if up.is_finite()&&down.is_finite()&&(up*4.).max(down)<=25.+1e-8 {if let Some(&k)=self.lookup.get(&(x,y)){if !self.has(k){self.flags[k/8]|=1<<(k%8);self.count+=1;}}}}
        Ok(())
    }
    pub fn has(&self,k:usize)->bool {self.enabled&&(self.flags[k/8]&(1<<(k%8)))!=0}
    pub fn mark(&mut self,i:&Input){if !self.enabled{return;}let k=self.lookup[&(i.x,i.y)];if !self.has(k){self.flags[k/8]|=1<<(k%8);self.count+=1;}}
    pub fn invalidate(&mut self,circles:&[[f64;3]]) {
        if !self.enabled||self.count==0{return;}
        for c in circles {
            if !c.iter().all(|x|x.is_finite())||c[2]<0. {self.flags.fill(0);self.count=0;return;}
            let r=c[2].sqrt()*(1.+1e-10)+STEP*4.;
            for item in self.index.locate_within_distance([c[0],c[1]],r*r) {
                let k=item.index;if self.has(k){self.flags[k/8]&=!(1<<(k%8));self.count-=1;}
            }
        }
    }
    pub fn save(&self,s:&Store,signature:&str)->Result<()> {
        s.db.execute("INSERT OR REPLACE INTO fit_verified VALUES(1,?,?,?)",params![format!("fit-valid-v1:E25:up4:before-after:{signature}"),s.epoch(13)?,self.flags])?;Ok(())
    }
}
