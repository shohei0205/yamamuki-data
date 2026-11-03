//! 証明済みの互いに離れた領域で、削減候補をまとめて処理する。
use super::*;
pub(super) struct BlockTask {
    pub stripe:usize,
    pub patch:Option<Patch>,
    pub bounds:[f64;4],
    pub z:i64,
    pub snapshot:Option<crate::natural_shared_store::ReadSnapshot>,
    pub rows:Vec<(i64,i64,[f64;3],bool)>,
    pub rasters:Vec<(Input,Arc<Vec<f64>>)>,
    pub witnesses:Vec<[f64;3]>,
}
pub(super) fn prepare_and_evaluate(j:BlockTask,error:f64,root:&Path)->Result<BlockResult> {
    let patch=if let Some(p)=j.patch {p} else {
        let reader=j.snapshot.as_ref().context("並列準備のスナップショットがありません")?.open()?;
        reader.patch(j.z,j.bounds)?
    };
    evaluate(BlockJob{stripe:j.stripe,patch,rows:j.rows,rasters:j.rasters,witnesses:j.witnesses},error,root)
}
pub(super) struct BlockJob {
    pub stripe:usize,
    pub patch:Patch,
    pub rows:Vec<(i64,i64,[f64;3],bool)>,
    pub rasters:Vec<(Input,Arc<Vec<f64>>)>,
    pub witnesses:Vec<[f64;3]>,
}
pub(super) struct BlockResult {
    pub stripe:usize,
    pub patch:Patch,
    pub finished:Vec<(usize,i64)>,
    pub removed:Vec<Removal>,
    pub witnesses:Vec<[f64;3]>,
    pub blocked:Option<i64>,
    pub checked_inputs:Vec<(i32,i32)>,
}
pub(super) fn evaluate(mut j:BlockJob,error:f64,root:&Path)->Result<BlockResult> {
    let mut finished=Vec::new();let mut removed=Vec::new();let mut blocked=None;
    let source=j.rasters.iter().map(|(i,a)|((i.x,i.y),a)).collect::<HashMap<_,_>>();
    let started=Instant::now();
    let direct=std::env::var_os("TERRAIN_SKIP_DIRECT_PRUNE").is_none();
    let mut w=Work::at_face(j.patch.hint_face);
    let mut direct_work=crate::natural_regional_surface::RemovalWork::default();
    for (n,(sequence,id,q,active)) in j.rows.into_iter().enumerate() {
        if n>=16 && started.elapsed()>std::time::Duration::from_millis(250) {break;}
        if n%16==0 {ensure!(!root.join("stop-requested").exists(),"停止要求を受けました。局所バッチは未確定です");}
        if !active {finished.push((j.stripe,sequence));continue;}
        if !inside([q[0],q[1]],j.patch.certified_bounds) {break;}
        let Some(k)=j.patch.index_of(id) else {break;};
        if j.patch.model.base.topology.hull.contains(&k) {finished.push((j.stripe,sequence));continue;}
        let x=(q[0]/STEP-0.5).round() as i32;let y=(q[1]/STEP-0.5).round() as i32;
        let own=source.get(&(x.div_euclid(256),y.div_euclid(256))).and_then(|a|{
            let h=a[(y.rem_euclid(256)*256+x.rem_euclid(256)) as usize];
            h.is_finite().then_some([(x as f64+0.5)*STEP,(y as f64+0.5)*STEP,h])
        });
        if let Some(at)=own.filter(|_|direct) {
            if let Some(h)=j.patch.model.evaluate_without_vertex_work(k,[at[0]-j.patch.origin[0],at[1]-j.patch.origin[1]],&mut direct_work)? {
                if score(h-at[2])>error+1e-6 {remember_witness(&mut j.witnesses,at);finished.push((j.stripe,sequence));continue;}
            }
        }
        let cs=circles(&j.patch,k).into_iter().map(|c|[c[0],c[1],(c[2].sqrt()+STEP*2.).powi(2)]).collect::<Vec<_>>();
        let influence=j.patch.deletion_box(k,STEP*2.);
        let old_star=star(&j.patch,k);let old_hint=j.patch.hint_face;
        let undo=j.patch.model.remove_local(k)?;
        j.patch.hint_face=undo.start_face;j.patch.ids[k]=0;
        w.set_start(j.patch.hint_face);let mut failure=None;
        for at in own.into_iter().chain(j.witnesses.iter().copied().filter(|at|cs.iter().any(|&c|circle_hits_point(c,[at[0],at[1]])))) {
            if inside([at[0],at[1]],j.patch.certified_bounds) && score(j.patch.evaluate([at[0],at[1]],&mut w)?-at[2])>error+1e-8 {failure=Some(at);break;}
        }
        // 変更可能域が証明領域を越えるときは、元の一候補ずつの全域証明へ戻す。
        let contained=inside([influence[0],influence[1]],j.patch.certified_bounds)&&inside([influence[2],influence[3]],j.patch.certified_bounds);
        if failure.is_none() && !contained {
            j.patch.model.undo_remove_local(undo);j.patch.restore_id(k,id);j.patch.hint_face=old_hint;blocked=Some(id);break;
        }
        if failure.is_none() {
            for (i,raster) in &j.rasters {
                let r=sample_range(i,influence);
                if r[0]>=r[2]||r[1]>=r[3] {continue;}
                let b=[(i.x as f64*256.+r[0] as f64)*STEP,(i.y as f64*256.+r[1] as f64)*STEP,(i.x as f64*256.+r[2] as f64)*STEP,(i.y as f64*256.+r[3] as f64)*STEP];
                if !cs.iter().any(|&c|circle_hits(c,b)){continue;}
                if let Some(at)=region_within_changed_error_work(&j.patch,i,raster,r,error+1e-8,&cs,&mut w)? {failure=Some(at);break;}
            }
        }
        if let Some(at)=failure {
            j.patch.model.undo_remove_local(undo);j.patch.restore_id(k,id);j.patch.hint_face=old_hint;remember_witness(&mut j.witnesses,at);
        } else {
            j.patch.epoch+=1;removed.push(Removal{id,star:old_star});
        }
        finished.push((j.stripe,sequence));
    }
    let checked_inputs=if !removed.is_empty() && std::env::var_os("TERRAIN_PRUNE_BENCH_AUDIT").is_some() {j.rasters.iter().map(|(i,_)|(i.x,i.y)).collect()} else {Vec::new()};
    Ok(BlockResult{stripe:j.stripe,patch:j.patch,finished,removed,witnesses:j.witnesses,blocked,checked_inputs})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disjoint_batches_match_global_rebuild_and_stop_is_safe()->Result<()> {
        let s=Store::open(Path::new(":memory:"),8192)?;
        for (x,y) in [(-10000,-10000),(10000,-10000),(10000,10000),(-10000,10000)] {
            s.insert(13,[x*16,y*16,(x/16+y/8)],true)?;
        }
        for y in 0..33 {for x in 0..33 {
            let xm=(x+40)*16;let ym=(y+40)*16;
            s.insert(13,[xm*16,ym*16,xm/16+ym/8+(x*13+y*7)%5-2],false)?;
        }}
        s.cache_level(13)?;
        let root=std::env::temp_dir().join(format!("prune-block-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));fs::create_dir(&root)?;
        let input=Input{x:0,y:0,sha:String::new()};
        let raster=Arc::new((0..65536).map(|k|((k%256) as f64+0.5)*STEP/128.+((k/256) as f64+0.5)*STEP/64.).collect::<Vec<_>>());
        let mut jobs=Vec::new();
        for (stripe,b,x0,x1) in [(0,[600.,600.,900.,1250.],704,800),(1,[950.,600.,1250.,1250.],1008,1088)] {
            let mut stmt=s.db.prepare("SELECT id,x,y,h FROM points WHERE guard=0 AND x>=? AND x<=? AND y>=? AND y<=? ORDER BY y,x LIMIT 32")?;
            let pts=stmt.query_map(params![x0*16,x1*16,768*16,896*16],|r|Ok((r.get::<_,i64>(0)?,[r.get::<_,i32>(1)? as f64/16.,r.get::<_,i32>(2)? as f64/16.,r.get::<_,i32>(3)? as f64/8.])))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let rows=pts.into_iter().enumerate().map(|(k,(id,q))|(k as i64+1,id,q,true)).collect();
            jobs.push(BlockJob{stripe,patch:s.patch(13,b)?,rows,rasters:vec![(input.clone(),raster.clone())],witnesses:Vec::new()});
        }
        let results=jobs.into_par_iter().map(|j|evaluate(j,25.,&root)).collect::<Result<Vec<_>>>()?;
        for r in &results {assert!(r.removed.len()>=8,"removed={}",r.removed.len());}
        s.delete_batch(13,&results.iter().flat_map(|r|r.removed.iter().map(|v|v.id)).collect::<Vec<_>>())?;
        for r in results {
            let b=r.patch.certified_bounds;let whole=s.patch(13,b)?;let mut a=Work::default();let mut bw=Work::default();
            for y in 0..9 {for x in 0..9 {
                let q=[b[0]+(b[2]-b[0])*x as f64/8.,b[1]+(b[3]-b[1])*y as f64/8.];
                assert!((r.patch.evaluate(q,&mut a)?-whole.evaluate(q,&mut bw)?).abs()<1e-7);
            }}
        }
        let before=s.epoch(13)?;let patch=s.patch(13,[600.,600.,900.,1250.])?;
        fs::write(root.join("stop-requested"),b"stop")?;
        let stopped=evaluate(BlockJob{stripe:0,patch,rows:vec![(1,0,[0.,0.,0.],false)],rasters:Vec::new(),witnesses:Vec::new()},25.,&root);
        assert!(stopped.is_err());assert_eq!(s.epoch(13)?,before);
        fs::remove_dir_all(root)?;Ok(())
    }
}
