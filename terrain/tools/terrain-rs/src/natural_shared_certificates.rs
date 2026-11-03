//! 削除専用期間の閉空円証明。追加・復元時は必ず失効させる。
use std::{collections::HashSet,sync::{RwLock,atomic::{AtomicI64,AtomicUsize,AtomicU64,Ordering::Relaxed}}};
pub(super) struct Certificates {
    z:AtomicI64,
    shards:Vec<RwLock<HashSet<[i64;3]>>>,
    count:AtomicUsize,
    hits:AtomicU64,
    misses:AtomicU64,
}
impl Certificates {
    pub(super) fn new()->Self {Self{z:AtomicI64::new(-1),shards:(0..64).map(|_|RwLock::new(HashSet::new())).collect(),count:AtomicUsize::new(0),hits:AtomicU64::new(0),misses:AtomicU64::new(0)}}
    pub(super) fn activate(&self,z:i64) {self.invalidate();self.z.store(z,Relaxed);}
    pub(super) fn invalidate(&self) {
        if self.z.swap(-1,Relaxed)==-1 {return;}
        for shard in &self.shards {shard.write().unwrap().clear();}
        self.count.store(0,Relaxed);
    }
    fn slot(key:[i64;3])->usize {((key[0] as u64).wrapping_mul(0x9e3779b97f4a7c15)^(key[1] as u64).rotate_left(23)^(key[2] as u64).wrapping_mul(0xbf58476d1ce4e5b9)) as usize&63}
    pub(super) fn contains(&self,z:i64,key:[i64;3])->bool {
        if self.z.load(Relaxed)!=z {return false;}
        let yes=self.shards[Self::slot(key)].read().unwrap().contains(&key);
        if yes {self.hits.fetch_add(1,Relaxed);}else{self.misses.fetch_add(1,Relaxed);}yes
    }
    pub(super) fn insert(&self,z:i64,key:[i64;3]) {
        if self.z.load(Relaxed)!=z {return;}
        let mut shard=self.shards[Self::slot(key)].write().unwrap();
        if self.count.load(Relaxed)>=500000 {let n=shard.len();shard.clear();self.count.fetch_sub(n,Relaxed);}
        if shard.insert(key) {self.count.fetch_add(1,Relaxed);}
    }
    pub(super) fn stats(&self)->serde_json::Value {serde_json::json!({"z":self.z.load(Relaxed),"entries":self.count.load(Relaxed),"hits":self.hits.load(Relaxed),"misses":self.misses.load(Relaxed),"limit":500000})}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generations_do_not_reuse_other_levels_or_reinsertions() {
        let c=Certificates::new();c.activate(13);c.insert(13,[1,2,3]);
        assert!(c.contains(13,[1,2,3]));assert!(!c.contains(12,[1,2,3]));
        c.invalidate();assert!(!c.contains(13,[1,2,3]));
        c.activate(13);assert!(!c.contains(13,[1,2,3]));
        c.insert(13,[1,2,3]);c.activate(12);assert!(!c.contains(13,[1,2,3]));
    }
}
