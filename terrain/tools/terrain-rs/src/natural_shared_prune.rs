//! 独立した影響円の頂点削減を並列に評価し、確定後にまとめて保存する。
use super::*;
#[path="natural_shared_prune_block.rs"]
mod blocks;

fn expanded(i: [f64; 3], z: i64) -> [f64; 4] {
    // 元PNGの大きさではなく、処理中のWebタイルの大きさで領域を再利用する。
    // 粗品質ほど広い支持を持つため、証明済み領域も同じ比率で広げる。
    let side = WORLD / 2f64.powi(z as i32);
    let x = (i[0] / side).floor() * side;
    let y = (i[1] / side).floor() * side;
    [
        x - side * 0.25,
        y - side * 0.25,
        x + side * if z==13 && std::env::var_os("TERRAIN_FORWARD_PRUNE_CACHE").is_some() {2.25} else {1.25},
        y + side * 1.25,
    ]
}
fn inside(q: [f64; 2], b: [f64; 4]) -> bool {
    q[0] >= b[0] && q[1] >= b[1] && q[0] <= b[2] && q[1] <= b[3]
}
fn overlap(a: [f64; 3], b: [f64; 3]) -> bool {
    let r = a[2].sqrt() + b[2].sqrt();
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) <= r * r * (1. + 1e-12) + 1e-8
}
fn circle_hits_point(c:[f64;3],q:[f64;2])->bool {
    (c[0]-q[0]).powi(2)+(c[1]-q[1]).powi(2)<=c[2]*(1.+1e-12)+1e-8
}
fn circles(p: &Patch, id: usize) -> Vec<[f64; 3]> {
    p.model.base.incident[id]
        .iter()
        .map(|&f| {
            let mut c = p.model.base.circles[f];
            c[0] += p.origin[0];
            c[1] += p.origin[1];
            c
        })
        .collect()
}
#[derive(Clone)]
struct Removal {
    id: i64,
    star: BTreeSet<[i64; 3]>,
}
fn star(p: &Patch, k: usize) -> BTreeSet<[i64; 3]> {
    p.model.base.incident[k]
        .iter()
        .map(|&f| {
            let t = &p.model.base.topology.triangles[f * 3..f * 3 + 3];
            let mut ids = [p.ids[t[0]], p.ids[t[1]], p.ids[t[2]]];
            ids.sort_unstable();
            ids
        })
        .collect()
}
#[cfg(test)]
fn sync(s: &Store, z: i64, p: &mut Option<Patch>, removed: &[Removal]) -> Result<()> {
    sync_epoch(s.epoch(z)?, p, removed)
}
fn sync_epoch(epoch: i64, p: &mut Option<Patch>, removed: &[Removal]) -> Result<()> {
    let Some(m) = p.as_mut() else { return Ok(()) };
    for r in removed {
        if let Some(k) = m.index_of(r.id) {
            let q = m.point(k);
            if !inside([q[0], q[1]], m.certified_bounds) {
                let faces = m
                    .model
                    .base
                    .faces_for_rect_walk_from(m.local_rect(m.certified_bounds), m.hint_face)?;
                let mut relevant = BTreeSet::new();
                for f in faces {
                    for &v in &m.model.base.topology.triangles[f * 3..f * 3 + 3] {
                        relevant.insert(v);
                        relevant.extend(m.model.base.neighbors[v].iter().copied());
                    }
                }
                // 問い合わせと、その内点を削除した後の支持にも使わない余剰点は保持してよい。
                if !relevant.contains(&k) {
                    continue;
                }
                // 範囲外でも削除点の全域の星形接続が同一と証明できれば局所更新する。
                if r.star.is_empty() || star(m, k) != r.star {
                    *p = None;
                    return Ok(());
                }
            }
            if m.model.base.topology.hull.contains(&k) {
                *p = None;
                return Ok(());
            }
            let u = m.model.remove_local(k)?;
            m.hint_face = u.start_face;
            m.ids[k] = 0;
        }
    }
    if let Some(m) = p.as_mut() {
        m.epoch = epoch;
    }
    Ok(())
}
struct RemovalJournal {
    floor:i64,
    batches:VecDeque<(i64,Vec<Removal>)>,
    count:usize,
    limit:usize,
}
impl RemovalJournal {
    fn new(epoch:i64)->Self {Self{floor:epoch,batches:VecDeque::new(),count:0,limit:131072}}
    fn restore(&self,epoch:i64,p:&mut Option<Patch>)->Result<()> {
        let Some(initial)=p.as_ref().map(|p|p.epoch) else{return Ok(());};
        if initial<self.floor {*p=None;return Ok(());}
        for (at,removed) in &self.batches {if *at>initial {sync_epoch(*at,p,removed)?;if p.is_none(){return Ok(());}}}
        sync_epoch(epoch,p,&[])
    }
    fn record(&mut self,epoch:i64,removed:Vec<Removal>,parked:&mut [Vec<Option<Patch>>]) {
        self.count+=removed.len();self.batches.push_back((epoch,removed));
        let minimum=parked.iter().flatten().filter_map(|p|p.as_ref().map(|p|p.epoch)).min().unwrap_or(epoch);
        while self.batches.front().is_some_and(|(at,_)|*at<=minimum)||self.count>self.limit {
            let Some((at,removed))=self.batches.pop_front() else{break;};
            self.floor=at;self.count-=removed.len();
        }
        for stripe in parked {stripe.retain(|p|p.as_ref().is_some_and(|p|p.epoch>=self.floor));}
    }
}
struct Job {
    stripe: usize,
    sequence: i64,
    id: i64,
    patch: Patch,
    snapshot: Option<crate::natural_shared_store::ReadSnapshot>,
    z: i64,
    index: usize,
    samples: Vec<(Input, Option<Arc<Vec<f64>>>, [usize; 4], Option<Patch>)>,
    dem_root: PathBuf,
    changed_circles: Vec<[f64;3]>,
    own: Option<[f64; 3]>,
    witnesses: Vec<[f64; 3]>,
    star: BTreeSet<[i64; 3]>,
}
struct Decision {
    stripe: usize,
    sequence: i64,
    id: i64,
    patch: Patch,
    accepted: bool,
    checked_inputs:Vec<(i32,i32)>,
    witness: Option<[f64; 3]>,
    star: BTreeSet<[i64; 3]>,
}
fn evaluate(mut j: Job, error: f64, root: &Path) -> Result<Decision> {
    ensure!(
        !root.join("stop-requested").exists(),
        "停止要求を受けました"
    );
    if j.patch.model.base.topology.hull.contains(&j.index) {
        return Ok(Decision {
            stripe: j.stripe,
            sequence: j.sequence,
            id: j.id,
            patch: j.patch,
            accepted: false,
            checked_inputs:Vec::new(),
            witness: None,
            star: j.star,
        });
    }
    let hint = j.patch.hint_face;
    let undo = j.patch.model.remove_local(j.index)?;
    j.patch.hint_face = undo.start_face;
    j.patch.ids[j.index] = 0;
    j.patch.epoch += 1;
    // 同じPatch・同じ世代で初期判定済み。自点と既知反例を二重に評価しない。
    let mut witness = None;
    if witness.is_none() {
        // 近傍が広い候補でも領域を4件ずつ並列評価する。
        // 全領域を先読みせず、反例が見つかった区切りで残りを打ち切る。
        for group in j.samples.chunks(4) {
            ensure!(
                !root.join("stop-requested").exists(),
                "停止要求を受けました"
            );
            let results = group
                .par_iter()
                .map(|(i, raster, range, proved)| -> Result<Option<[f64; 3]>> {
                    let b = [
                        (i.x as f64 * 256. + range[0] as f64) * STEP,
                        (i.y as f64 * 256. + range[1] as f64) * STEP,
                        (i.x as f64 * 256. + range[2] as f64) * STEP,
                        (i.y as f64 * 256. + range[3] as f64) * STEP,
                    ];
                    let external;
                    let p = if let Some(p) = proved.as_ref() {
                        p
                    } else if inside([b[0], b[1]], j.patch.certified_bounds)
                        && inside([b[2], b[3]], j.patch.certified_bounds)
                    {
                        &j.patch
                    } else {
                        let reader = j
                            .snapshot
                            .as_ref()
                            .context("領域外の証明スナップショットがありません")?
                            .open()?;
                        external = reader.patch_omitting(j.z, b, Some(j.id))?;
                        &external
                    };
                    let decoded;
                    let raster = if let Some(raster) = raster.as_ref() {
                        raster.as_ref()
                    } else {
                        let mut dem=Dem{root:j.dem_root.clone(),cache:HashMap::new(),order:VecDeque::new()};
                        decoded=dem.read(i)?;
                        decoded.as_ref()
                    };
                    if std::env::var_os("TERRAIN_FULL_REMOVAL_PIXELS").is_some() {
                        region_within_error(p, i, raster, *range, error + 1e-8)
                    } else {
                        region_within_changed_error(p,i,raster,*range,error+1e-8,&j.changed_circles)
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            if let Some(q) = results.into_iter().flatten().next() {
                witness = Some(q);
                break;
            }
        }
    }
    if witness.is_some() {
        j.patch.model.undo_remove_local(undo);
        j.patch.hint_face = hint;
        j.patch.restore_id(j.index,j.id);
        j.patch.epoch -= 1;
    }
    Ok(Decision {
        stripe: j.stripe,
        sequence: j.sequence,
        id: j.id,
        patch: j.patch,
        accepted: witness.is_none(),
        checked_inputs:if witness.is_none() && std::env::var_os("TERRAIN_PRUNE_BENCH_AUDIT").is_some() {j.samples.iter().map(|(i,_,_,_)|(i.x,i.y)).collect()} else {Vec::new()},
        witness,
        star: j.star,
    })
}
/// 旧頂点の影響円の外ではSibson曲面は不変。範囲内の全有効画素を同じ誤差で確認する。
fn region_within_changed_error(p:&Patch,i:&Input,raster:&[f64],range:[usize;4],error:f64,circles:&[[f64;3]]) -> Result<Option<[f64;3]>> {
    let mut work=Work::at_face(p.hint_face);
    region_within_changed_error_work(p,i,raster,range,error,circles,&mut work)
}
fn region_within_changed_error_work(p:&Patch,i:&Input,raster:&[f64],range:[usize;4],error:f64,circles:&[[f64;3]],work:&mut Work) -> Result<Option<[f64;3]>> {
    work.set_start(p.hint_face);
    let min_bound_pixels=std::env::var("TERRAIN_PRUNE_BOUND_MIN_PIXELS").ok().and_then(|v|v.parse::<usize>().ok()).unwrap_or(0);
    for y in (range[1]..range[3]).step_by(16) {for x in (range[0]..range[2]).step_by(16) {
        let r=[x,y,(x+16).min(range[2]),(y+16).min(range[3])];
        let b=[(i.x as f64*256.+r[0] as f64)*STEP,(i.y as f64*256.+r[1] as f64)*STEP,(i.x as f64*256.+r[2] as f64)*STEP,(i.y as f64*256.+r[3] as f64)*STEP];
        if !circles.iter().any(|&c|circle_hits(c,b)) {continue;}
        if (r[2]-r[0])*(r[3]-r[1])>min_bound_pixels && region_bound(p,i,raster,r)?.is_none_or(|(v,_)|v<=error) {continue;}
        for yy in r[1]..r[3] {for xx in r[0]..r[2] {
            let h=raster[yy*256+xx];if !h.is_finite(){continue;}
            let q=[(i.x as f64*256.+xx as f64+0.5)*STEP,(i.y as f64*256.+yy as f64+0.5)*STEP];
            if !circles.iter().any(|&c|circle_hits_point(c,q)) {continue;}
            if score(p.evaluate(q,work)?-h)>error {return Ok(Some([q[0],q[1],h]));}
        }}
    }}
    Ok(None)
}
pub(super) fn sample_range(i: &Input, b: [f64; 4]) -> [usize; 4] {
    [
        ((b[0] / STEP - i.x as f64 * 256. - 1.)
            .floor()
            .clamp(0., 256.)) as usize,
        ((b[1] / STEP - i.y as f64 * 256. - 1.)
            .floor()
            .clamp(0., 256.)) as usize,
        ((b[2] / STEP - i.x as f64 * 256. + 1.)
            .ceil()
            .clamp(0., 256.)) as usize,
        ((b[3] / STEP - i.y as f64 * 256. + 1.)
            .ceil()
            .clamp(0., 256.)) as usize,
    ]
}
fn source_at(
    q: [f64; 3],
    source: &HashMap<(i32, i32), &Input>,
    dem: &mut Dem,
) -> Result<Option<[f64; 3]>> {
    let x = (q[0] / STEP - 0.5).round() as i32;
    let y = (q[1] / STEP - 0.5).round() as i32;
    let Some(i) = source.get(&(x.div_euclid(256), y.div_euclid(256))) else {
        return Ok(None);
    };
    let h = dem.read(i)?[(y.rem_euclid(256) * 256 + x.rem_euclid(256)) as usize];
    Ok(h.is_finite()
        .then_some([(x as f64 + 0.5) * STEP, (y as f64 + 0.5) * STEP, h]))
}
fn serial(
    s: &Store,
    z: i64,
    id: i64,
    error: f64,
    p: &mut Patch,
    affected: &[Input],
    influence: [f64; 4],
    dem: &mut Dem,
) -> Result<bool> {
    let k = p
        .ids
        .iter()
        .position(|&v| v == id)
        .context("削減点がありません")?;
    if p.model.base.topology.hull.contains(&k) {
        return Ok(false);
    }
    let hint = p.hint_face;
    let undo = p.model.remove_local(k)?;
    p.hint_face = undo.start_face;
    p.ids[k] = 0;
    p.epoch += 1;
    s.db.execute_batch("SAVEPOINT parallel_fallback")?;
    s.db.execute("DELETE FROM active WHERE z=? AND id=?", params![z, id])?;
    s.bump(z)?;
    s.refresh_cached_point(z, id)?;
    let tested = audit_affected_cached(s, z, error, affected, dem, influence, Some(p), None);
    match tested {
        Ok(true) => {
            s.db.execute("INSERT OR IGNORE INTO removed VALUES(?,?)", params![z, id])?;
            s.db.execute_batch("RELEASE parallel_fallback")?;
            Ok(true)
        }
        result => {
            s.db.execute_batch("ROLLBACK TO parallel_fallback;RELEASE parallel_fallback")?;
            s.refresh_cached_point(z, id)?;
            p.model.undo_remove_local(undo);
            p.hint_face = hint;
            p.restore_id(k,id);
            p.epoch -= 1;
            result
        }
    }
}
fn dequeue(s: &Store, z: i64, rows: &[(usize, i64)]) -> Result<()> {
    let tx = s.db.unchecked_transaction()?;
    {
        let mut q = tx.prepare("DELETE FROM prune_queue WHERE z=? AND stripe=? AND sequence=?")?;
        for &(stripe, sequence) in rows {
            q.execute(params![z, stripe, sequence])?;
        }
    }
    tx.commit()?;
    Ok(())
}
/// 残り候補を同じWebタイルごとにまとめる。旧キューは保存し、点集合は変更しない。
fn localize_queue(s:&Store,z:i64,root:&Path)->Result<()> {
    if std::env::var_os("TERRAIN_LEGACY_PRUNE_ORDER").is_some() { return Ok(()); }
    let key=format!("prune-locality-{z}-v1");
    if s.db.query_row("SELECT 1 FROM meta WHERE key=?",[&key],|_|Ok(())).optional()?.is_some() {return Ok(());}
    ensure!((6..=13).contains(&z),"品質範囲外です");
    progress(root,json!({"stage":"prune-reorder","z":z,"complete":false,"note":"残りの候補をWebタイル順へ整理。旧キューと点集合は保持"}))?;
    let old=format!("prune_queue_before_locality_z{z}");
    let tx=s.db.unchecked_transaction()?;
    tx.execute_batch(&format!("ALTER TABLE prune_queue RENAME TO {old}; CREATE TABLE prune_queue(z INTEGER,stripe INTEGER,sequence INTEGER,id INTEGER,PRIMARY KEY(z,stripe,sequence)) WITHOUT ROWID"))?;
    tx.execute(&format!("INSERT INTO prune_queue SELECT * FROM {old} WHERE z<>?"),[z])?;
    let side=WORLD/2f64.powi(z as i32)*16.;
    tx.execute(&format!("INSERT INTO prune_queue SELECT q.z,q.stripe,row_number() OVER(PARTITION BY q.z,q.stripe ORDER BY CAST(p.y/?1 AS INTEGER),CAST(p.x/?1 AS INTEGER),p.y,p.x,q.sequence),q.id FROM {old} q JOIN points p ON p.id=q.id WHERE q.z=?2"),params![side,z])?;
    let before:i64=tx.query_row(&format!("SELECT count(*) FROM {old} WHERE z=?"),[z],|r|r.get(0))?;
    let after:i64=tx.query_row("SELECT count(*) FROM prune_queue WHERE z=?",[z],|r|r.get(0))?;
    ensure!(before==after,"キュー候補数が変化しました");
    tx.execute("INSERT INTO meta VALUES(?,?)",params![key,json!({"scheme":"WebMercator tile major, then y,x,old sequence","z":z,"remaining":after}).to_string()])?;
    tx.commit()?;
    Ok(())
}
/// 作業レーンを分け、物理スレッドを増やさず重い地域の待ちを隠す。
fn split_queue(s:&Store,z:i64,threads:usize,root:&Path)->Result<()> {
    if z!=13 {return Ok(());}
    let factor=std::env::var("TERRAIN_PRUNE_LANE_FACTOR").ok().and_then(|v|v.parse::<usize>().ok()).unwrap_or(2).clamp(1,4);
    let key=format!("prune-lanes-{z}-v1");
    if let Some(value)=s.db.query_row("SELECT value FROM meta WHERE key=?",[&key],|r|r.get::<_,String>(0)).optional()? {
        let m:Value=serde_json::from_str(&value)?;
        ensure!(m["threads"].as_u64()==Some(threads as u64),"保存済みレーンの実スレッド数が異なります");return Ok(());
    }
    if factor==1 {return Ok(());}
    let name=format!("prune-queue-{z}");let mut q=state(s,&name)?.context("削減キューがありません")?;
    ensure!(q["workers"].as_u64()==Some(threads as u64),"分割前のキュー数が異なります");
    progress(root,json!({"stage":"prune-reorder","z":z,"complete":false,"note":"独立作業レーンを追加。実スレッド数と候補は保持"}))?;
    let old=format!("prune_queue_before_lanes_z{z}");let tx=s.db.unchecked_transaction()?;
    tx.execute_batch(&format!("ALTER TABLE prune_queue RENAME TO {old}; CREATE TABLE prune_queue(z INTEGER,stripe INTEGER,sequence INTEGER,id INTEGER,PRIMARY KEY(z,stripe,sequence)) WITHOUT ROWID"))?;
    tx.execute(&format!("INSERT INTO prune_queue SELECT * FROM {old} WHERE z<>?"),[z])?;
    tx.execute(&format!("INSERT INTO prune_queue SELECT z,stripe*?1+ntile(?1) OVER(PARTITION BY stripe ORDER BY sequence)-1,sequence,id FROM {old} WHERE z=?2"),params![factor as i64,z])?;
    let before:i64=tx.query_row(&format!("SELECT count(*) FROM {old} WHERE z=?"),[z],|r|r.get(0))?;
    let after:i64=tx.query_row("SELECT count(*) FROM prune_queue WHERE z=?",[z],|r|r.get(0))?;ensure!(before==after,"レーン分割で候補数が変化しました");
    q["workers"]=json!(threads*factor);q["threads"]=json!(threads);
    tx.execute("UPDATE stages SET details=? WHERE name=?",params![q.to_string(),name])?;
    tx.execute("INSERT INTO meta VALUES(?,?)",params![key,json!({"threads":threads,"factor":factor,"remaining":after}).to_string()])?;
    tx.commit()?;Ok(())
}
pub(super) fn run(
    s: &Store,
    z: i64,
    fine: i64,
    error: f64,
    inputs: &[Input],
    dem: &mut Dem,
    root: &Path,
    workers: usize,
) -> Result<()> {
    let name = format!("prune-{z}");
    if state(s, &name)?.is_some() {
        return Ok(());
    }
    let seed = format!("seed-{z}");
    if z != fine && state(s, &seed)?.is_none() {
        let tx = s.db.unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO active SELECT ?,id FROM active WHERE z=?",
            params![z, fine],
        )?;
        tx.execute("INSERT OR REPLACE INTO epochs VALUES(?,1)", [z])?;
        tx.execute(
            "INSERT OR REPLACE INTO stages VALUES(?,'done','{}')",
            [seed],
        )?;
        tx.commit()?;
    }
    let workers = workers.max(1);
    s.cache_level(z)?;
    s.db.execute_batch("CREATE TABLE IF NOT EXISTS prune_queue(z INTEGER,stripe INTEGER,sequence INTEGER,id INTEGER,PRIMARY KEY(z,stripe,sequence)) WITHOUT ROWID")?;
    let queued = format!("prune-queue-{z}");
    if state(s, &queued)?.is_none() {
        let old_cursor =
            s.db.query_row(
                "SELECT value FROM meta WHERE key=?",
                [format!("cursor-{z}")],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(|| "0".into())
            .parse::<i64>()?;
        let bench_cursor=s.db.query_row("SELECT value FROM meta WHERE key='benchmark_prune_min_id' AND EXISTS(SELECT 1 FROM meta WHERE key='benchmark_only')",[],|r|r.get::<_,String>(0)).optional()?.unwrap_or_else(||"0".into()).parse::<i64>()?;
        let old_cursor = old_cursor.max(bench_cursor);
        let tx = s.db.unchecked_transaction()?;
        tx.execute("INSERT INTO prune_queue SELECT ?1,ntile(?2) OVER(ORDER BY p.y,p.x,a.id)-1,row_number() OVER(ORDER BY p.y,p.x,a.id),a.id FROM active a JOIN points p ON p.id=a.id WHERE a.z=?1 AND p.guard=0 AND a.id>?3",params![z,workers,old_cursor])?;
        let n: i64 = tx.query_row("SELECT count(*) FROM prune_queue WHERE z=?", [z], |r| {
            r.get(0)
        })?;
        tx.execute(
            "INSERT INTO stages VALUES(?,'done',?)",
            params![
                queued,
                json!({"workers":workers,"candidates":n}).to_string()
            ],
        )?;
        tx.commit()?;
    }
    localize_queue(s,z,root)?;
    split_queue(s,z,workers,root)?;
    let physical_threads=workers;
    let queue = state(s, &queued)?.unwrap();
    let workers = queue["workers"].as_u64().context("並列数がありません")? as usize;
    let total = queue["candidates"].as_u64().unwrap();
    let mut remaining =
        s.db.query_row("SELECT count(*) FROM prune_queue WHERE z=?", [z], |r| {
            r.get::<_, u64>(0)
        })?;
    let spatial = input_index(inputs);
    let source = inputs
        .iter()
        .map(|i| ((i.x, i.y), i))
        .collect::<HashMap<_, _>>();
    let mut caches = (0..workers).map(|_| None).collect::<Vec<Option<Patch>>>();
    // 空間順の走査がタイル列を往復しても、証明済みの周辺メッシュを再利用する。
    let mut parked = (0..workers)
        .map(|_| Vec::<Option<Patch>>::new())
        .collect::<Vec<_>>();
    let mut witnesses = (0..workers)
        .map(|_| Vec::new())
        .collect::<Vec<Vec<[f64; 3]>>>();
    let mut turn = 0usize;
    let mut accepted = 0u64;
    let mut maximum_parallel_jobs = 0usize;
    let initial_remaining = remaining;
    for counter in [&DEM_CACHE_HITS,&DEM_CACHE_MISSES,&DEM_READ_NS] {counter.store(0,std::sync::atomic::Ordering::Relaxed);}
    let started = Instant::now();
    let mut prepare_seconds = 0.;
    let mut evaluate_seconds = 0.;
    let mut save_seconds = 0.;
    let mut sync_seconds = 0.;
    let mut cache_rebuilds = 0u64;
    let mut preflight_rejections = 0u64;
    let mut serial_candidates = 0u64;
    // z13は実測で優位を確認した仮想削除を標準にする。粗品質は従来経路を維持。
    let virtual_proofs = (z == 13 && std::env::var_os("TERRAIN_SERIAL_REMOVAL_PROOFS").is_none())
        || std::env::var_os("TERRAIN_PARALLEL_REMOVAL_PROOFS").is_some();
    if z==13 && virtual_proofs && std::env::var_os("TERRAIN_NO_SHARED_CIRCLES").is_none() {s.enable_deletion_certificates(z);}
    let mut expanded_candidates = 0u64;
    let mut unaffected_regions = 0u64;
    let mut early_rejections = 0u64;
    let mut evaluated_candidates = 0u64;
    let deferred_sync=std::env::var_os("TERRAIN_EAGER_PARKED_SYNC").is_none();
    let mut journal=RemovalJournal::new(s.epoch(z)?);
    let blocks_enabled=z==13 && std::env::var_os("TERRAIN_SINGLE_CANDIDATE_PRUNE").is_none();
    let block_size=std::env::var("TERRAIN_PRUNE_BLOCK_SIZE").ok().and_then(|v|v.parse::<usize>().ok()).unwrap_or(1024).clamp(16,4096);
    let mut blocked=vec![None;workers];
    let mut block_candidates=0u64;
    let benchmark_audit=std::env::var_os("TERRAIN_PRUNE_BENCH_AUDIT").is_some();
    let mut touched_inputs=BTreeSet::new();
    let mut queue_head = s.db.prepare_cached(
        "SELECT sequence,id FROM prune_queue WHERE z=? AND stripe=? ORDER BY sequence LIMIT 1",
    )?;
    let mut is_active = s.db.prepare_cached("SELECT 1 FROM active WHERE z=? AND id=?")?;
    while remaining > 0 {
        ensure!(!root.join("stop-requested").exists(), "停止要求を受けました。確定済みキューは保持されています");
        let mut phase = Instant::now();
        let mut jobs = Vec::new();
        let mut outside = Vec::new();
        let snapshot = if virtual_proofs {
            s.read_snapshot(z)?
        } else {
            None
        };
        let mut selected_circles = Vec::new();
        let mut finished = Vec::new();
        let mut removed = Vec::new();
        let mut pending = Vec::new();
        let mut missing = Vec::new();
        let epoch = s.epoch(z)?;
        for j in 0..workers {
            let stripe = (j + turn) % workers;
            let row=queue_head.query_row(params![z,stripe],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).optional()?;
            let Some((sequence, id)) = row else { continue };
            let active =
                is_active.query_row(
                    params![z, id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !active {
                finished.push((stripe, sequence));
                continue;
            }
            let q = s.point(id)?;
            if caches[stripe].as_ref().is_none_or(|p| {
                p.epoch != epoch || !inside([q[0], q[1]], p.certified_bounds)
            }) {
                if let Some(previous) = caches[stripe].take() {
                    while !parked[stripe].is_empty()
                        && (parked[stripe].len() >= (16*physical_threads/workers).max(1)
                            || parked[stripe]
                                .iter()
                                .flatten()
                                .map(|p| p.ids.len())
                                .sum::<usize>()
                                + previous.ids.len()
                                > 131_072*physical_threads/workers)
                    {
                        parked[stripe].remove(0);
                    }
                    parked[stripe].push(Some(previous));
                }
                let found = parked[stripe].iter().position(|p| {
                    p.as_ref().is_some_and(|p| {
                        (deferred_sync || p.epoch == epoch) && inside([q[0], q[1]], p.certified_bounds)
                    })
                });
                let mut restored=found.and_then(|k|parked[stripe].remove(k));
                if deferred_sync {journal.restore(epoch,&mut restored)?;}
                caches[stripe] = if restored.is_some() {restored} else {
                    cache_rebuilds += 1;
                    missing.push((stripe, s.cache_region(z, [q[0], q[1]], expanded(q, z))?));
                    None
                };
            }
            pending.push((stripe, sequence, id, q));
        }
        if z == 13 && !missing.is_empty() && std::env::var_os("TERRAIN_LEGACY_PRUNE_PREPARE").is_none() && !blocks_enabled {
            let mut probes = Vec::new();
            for &(stripe, _) in &missing {
                let &(_, sequence, id, q) = pending.iter().find(|p| p.0 == stripe).unwrap();
                if let Some(own) = source_at(q, &source, dem)? {
                    let d = STEP * 2.;
                    probes.push((stripe, sequence, id, own, [q[0]-d,q[1]-d,q[0]+d,q[1]+d]));
                }
            }
            let small = s.patches(z, &probes.iter().map(|p| p.4).collect::<Vec<_>>())?;
            let rejected = probes.into_par_iter().zip(small).map(|((stripe, sequence, id, own, _), mut p)| -> Result<Option<(usize,i64,[f64;3])>> {
                let k=p.ids.iter().position(|&v|v==id).context("事前判定点がありません")?;
                if p.model.base.topology.hull.contains(&k) { return Ok(None); }
                if let Some(h)=p.model.evaluate_without_vertex(k,[own[0]-p.origin[0],own[1]-p.origin[1]])? {
                    if score(h-own[2])>error+1e-6 {return Ok(Some((stripe,sequence,own)));}
                }
                let u=p.model.remove_local(k)?;
                let mut work=Work::at_face(u.start_face);
                // 受理には使わない。自点で既存誤差条件に反する候補だけを早期棄却する。
                if score(p.evaluate([own[0],own[1]], &mut work)?-own[2]) > error+1e-8 {
                    Ok(Some((stripe,sequence,own)))
                } else { Ok(None) }
            }).collect::<Result<Vec<_>>>()?;
            let mut rejected_stripes=BTreeSet::new();
            for (stripe,sequence,own) in rejected.into_iter().flatten() {
                preflight_rejections+=1;early_rejections+=1;
                rejected_stripes.insert(stripe);finished.push((stripe,sequence));
                remember_witness(&mut witnesses[stripe],own);
            }
            missing.retain(|(stripe,_)|!rejected_stripes.contains(stripe));
            pending.retain(|(stripe,_,_,_)|!rejected_stripes.contains(stripe));
        }
        let late_requests=missing.iter().copied().collect::<HashMap<_,_>>();
        let defer_prepare=blocks_enabled && std::env::var_os("TERRAIN_ALTERNATE_PRUNE_BLOCKS").is_none_or(|_|turn%2==0) && snapshot.is_some();
        let rebuilt = if defer_prepare {Vec::new()} else {s.patches(z, &missing.iter().map(|(_, b)| *b).collect::<Vec<_>>())?};
        for ((stripe, _), patch) in missing.into_iter().zip(rebuilt) {
            caches[stripe] = Some(patch);
        }
        if blocks_enabled && std::env::var_os("TERRAIN_ALTERNATE_PRUNE_BLOCKS").is_none_or(|_|turn%2==0) {
            let mut chosen=Vec::<[f64;4]>::new();let mut block_jobs=Vec::new();
            for &(stripe,sequence,id,_) in &pending {
                if blocked[stripe]==Some(id) {continue;}
                let b=if let Some(p)=caches[stripe].as_ref() {p.certified_bounds} else if let Some(b)=late_requests.get(&stripe) {*b} else {continue;};
                if chosen.iter().any(|a|a[0]<=b[2]&&a[2]>=b[0]&&a[1]<=b[3]&&a[3]>=b[1]) {continue;}
                let mut stmt=s.db.prepare_cached("SELECT q.sequence,q.id,p.x,p.y,p.h,a.id IS NOT NULL FROM prune_queue q JOIN points p ON p.id=q.id LEFT JOIN active a ON a.z=q.z AND a.id=q.id WHERE q.z=? AND q.stripe=? AND q.sequence>=? ORDER BY q.sequence LIMIT ?4")?;
                let rows=stmt.query_map(params![z,stripe,sequence,block_size],|r|Ok((r.get(0)?,r.get(1)?,[r.get::<_,i32>(2)? as f64/16.,r.get::<_,i32>(3)? as f64/16.,r.get::<_,i32>(4)? as f64/8.],r.get(5)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                let area=[b[0]-STEP*2.,b[1]-STEP*2.,b[2]+STEP*2.,b[3]+STEP*2.];
                let region_inputs=affected_inputs(inputs,&spatial,area);
                let arrays=dem.read_many(&region_inputs)?;
                let rasters=region_inputs.into_iter().zip(arrays).collect();
                chosen.push(b);block_jobs.push(blocks::BlockTask{stripe,patch:caches[stripe].take(),bounds:b,z,snapshot:snapshot.clone(),rows,rasters,witnesses:witnesses[stripe].clone()});
            }
            prepare_seconds+=phase.elapsed().as_secs_f64();
            let block_started=Instant::now();
            let results=block_jobs.into_par_iter().map(|j|blocks::prepare_and_evaluate(j,error,root)).collect::<Result<Vec<_>>>()?;
            evaluate_seconds+=block_started.elapsed().as_secs_f64();
            let mut block_removed=Vec::new();let mut block_finished=Vec::new();
            for result in results {
                if benchmark_audit {touched_inputs.extend(result.checked_inputs);}
                blocked[result.stripe]=result.blocked;
                witnesses[result.stripe]=result.witnesses;caches[result.stripe]=Some(result.patch);
                block_removed.extend(result.removed);block_finished.extend(result.finished);
            }
            if !block_finished.is_empty() {
                block_candidates+=block_finished.len() as u64;
                finished.extend(block_finished);drop(snapshot);
                let save_started=Instant::now();
                s.delete_batch(z,&block_removed.iter().map(|r|r.id).collect::<Vec<_>>())?;
                accepted+=block_removed.len() as u64;
                dequeue(s,z,&finished)?;remaining-=finished.len() as u64;
                save_seconds+=save_started.elapsed().as_secs_f64();
                let sync_started=Instant::now();
                if !block_removed.is_empty() {
                    let epoch=s.epoch(z)?;
                    if deferred_sync {
                        caches.par_iter_mut().try_for_each(|p|sync_epoch(epoch,p,&block_removed))?;
                        journal.record(epoch,block_removed,&mut parked);
                    } else {
                        caches.par_iter_mut().chain(parked.par_iter_mut().flat_map_iter(|v|v.iter_mut())).try_for_each(|p|sync_epoch(epoch,p,&block_removed))?;
                        for v in &mut parked {v.retain(Option::is_some);}
                    }
                }
                sync_seconds+=sync_started.elapsed().as_secs_f64();turn=(turn+1)%workers;
                progress(root,json!({"stage":"prune-parallel","z":z,"done":total-remaining,"total":total,"accepted":accepted,"workers":physical_threads,"lanes":workers,"complete":false,"profile":{"prepare_seconds":prepare_seconds,"evaluate_seconds":evaluate_seconds,"save_seconds":save_seconds,"sync_seconds":sync_seconds,"cache_rebuilds":cache_rebuilds,"dem":dem_profile(),"closed_circles":s.certificate_stats(),"block_candidates":block_candidates,"block_size":block_size,"blocks_enabled":blocks_enabled,"fused_preparation":true,"deferred_sync":deferred_sync,"elapsed_seconds":started.elapsed().as_secs_f64()}}))?;
                continue;
            }
            phase=Instant::now();
        }
        let late=pending.iter().filter(|p|caches[p.0].is_none()).map(|p|Ok((p.0,*late_requests.get(&p.0).context("未準備領域がありません")?))).collect::<Result<Vec<_>>>()?;
        let late_patches=s.patches(z,&late.iter().map(|(_,b)|*b).collect::<Vec<_>>())?;
        for ((stripe,_),p) in late.into_iter().zip(late_patches) {caches[stripe]=Some(p);}
        let mut prechecks=Vec::new();
        for (stripe,sequence,id,q) in pending {
            let p=caches[stripe].take().unwrap();
            let k=p.index_of(id).context("並列削減の点がありません")?;
            if p.model.base.topology.hull.contains(&k) {
                caches[stripe]=Some(p);finished.push((stripe,sequence));continue;
            }
            let own=source_at(q,&source,dem)?;
            prechecks.push((stripe,sequence,id,q,p,k,own,witnesses[stripe].clone()));
        }
        let prechecks=prechecks.into_par_iter().map(|(stripe,sequence,id,q,mut p,k,own,witnesses)| -> Result<_> {
            if let Some(at)=own.filter(|_|std::env::var_os("TERRAIN_SKIP_DIRECT_PRUNE").is_none()) {
                if let Some(h)=p.model.evaluate_without_vertex(k,[at[0]-p.origin[0],at[1]-p.origin[1]])? {
                    if score(h-at[2])>error+1e-6 {return Ok((stripe,sequence,id,q,p,k,own,Some(at),Vec::new()));}
                }
            }
            let cs=circles(&p,k);
            let padded=cs.iter().map(|c|[c[0],c[1],(c[2].sqrt()+STEP*2.).powi(2)]).collect::<Vec<_>>();
            let u=p.model.remove_local(k)?;
            let mut work=Work::at_face(u.start_face);let mut failed=None;
            for at in own.into_iter().chain(witnesses.into_iter().filter(|at|padded.iter().any(|&c|circle_hits_point(c,[at[0],at[1]])))) {
                if inside([at[0],at[1]],p.certified_bounds) && score(p.evaluate([at[0],at[1]],&mut work)?-at[2])>error+1e-8 {failed=Some(at);break;}
            }
            p.model.undo_remove_local(u);
            Ok((stripe,sequence,id,q,p,k,own,failed,cs))
        }).collect::<Result<Vec<_>>>()?;
        for (stripe,sequence,id,_q,mut p,k,own,failed,cs) in prechecks {
            if let Some(at) = failed {
                early_rejections += 1;
                remember_witness(&mut witnesses[stripe], at);
                caches[stripe] = Some(p);
                finished.push((stripe, sequence));
                continue;
            }
            if cs
                .iter()
                .any(|&c| selected_circles.iter().any(|&d| overlap(c, d)))
            {
                caches[stripe] = Some(p);
                continue;
            }
            let influence = p.deletion_box(k, STEP * 2.);
            let affected = affected_inputs(inputs, &spatial, influence);
            let mut samples = Vec::new();
            let mut eligible = true;
            let mut filtered = Vec::new();
            for i in &affected {
                let r = sample_range(i, influence);
                if r[0] >= r[2] || r[1] >= r[3] {
                    continue;
                }
                let b = [
                    (i.x as f64 * 256. + r[0] as f64) * STEP,
                    (i.y as f64 * 256. + r[1] as f64) * STEP,
                    (i.x as f64 * 256. + r[2] as f64) * STEP,
                    (i.y as f64 * 256. + r[3] as f64) * STEP,
                ];
                // 矩形包絡だけでは、外接円の間の空白まで再評価してしまう。
                // Sibsonの支持が変わり得る旧外接円群（既存と同じ2画素の余裕）と照合する。
                if !cs
                    .iter()
                    .any(|&c| circle_hits([c[0], c[1], (c[2].sqrt() + STEP * 2.).powi(2)], b))
                {
                    unaffected_regions += 1;
                    continue;
                }
                if !inside([b[0], b[1]], p.certified_bounds)
                    || !inside([b[2], b[3]], p.certified_bounds)
                {
                    if !virtual_proofs {
                        eligible = false;
                    } else if snapshot.is_none() {
                        outside.push((jobs.len(), samples.len(), b, Some(id)));
                    }
                    expanded_candidates += 1;
                }
                filtered.push(i.clone());
                if eligible || virtual_proofs {
                    // 小さな影響域は共有DEMキャッシュで先読みし、広域候補だけ4領域ずつ遅延読込する。
                    samples.push((i.clone(), if virtual_proofs && samples.len() >= 64 { None } else { Some(dem.read(i)?) }, r, None));
                }
            }
            if !eligible {
                if !jobs.is_empty() {
                    caches[stripe] = Some(p);
                    continue;
                }
                serial_candidates += 1;
                let old_star = star(&p, k);
                if serial(s, z, id, error, &mut p, &filtered, influence, dem)? {
                    accepted += 1;
                    removed.push(Removal { id, star: old_star });
                }
                caches[stripe] = Some(p);
                finished.push((stripe, sequence));
                break;
            }
            selected_circles.extend(cs.iter().copied());
            let old_star = star(&p, k);
            jobs.push(Job {
                stripe,
                sequence,
                id,
                patch: p,
                snapshot: snapshot.clone(),
                z,
                index: k,
                samples,
                dem_root:dem.root.clone(),
                changed_circles:cs.into_iter().map(|c|[c[0],c[1],(c[2].sqrt()+STEP*2.).powi(2)]).collect(),
                own,
                witnesses: witnesses[stripe].clone(),
                star: old_star,
            });
        }
        // 各候補の仮想削除を独立した読み取り専用ビューで証明する。
        // 巨大な外接円全体を矩形化せず、必要な元DEM領域だけを構築する。
        let proofs = s.patches_omitting(
            z,
            &outside
                .iter()
                .map(|&(_, _, b, omit)| (b, omit))
                .collect::<Vec<_>>(),
        )?;
        for ((job, sample, _, _), proof) in outside.into_iter().zip(proofs) {
            jobs[job].samples[sample].3 = Some(proof);
        }
        prepare_seconds += phase.elapsed().as_secs_f64();
        let phase = Instant::now();
        evaluated_candidates += jobs.len() as u64;
        maximum_parallel_jobs = maximum_parallel_jobs.max(jobs.len());
        let decisions = jobs
            .into_par_iter()
            .map(|j| evaluate(j, error, root))
            .collect::<Result<Vec<_>>>()?;
        // ワーカー終了後は共有参照を解放し、確定時のRTree全複製を防ぐ。
        drop(snapshot);
        evaluate_seconds += phase.elapsed().as_secs_f64();
        let phase = Instant::now();
        if benchmark_audit {touched_inputs.extend(decisions.iter().filter(|d|d.accepted).flat_map(|d|d.checked_inputs.iter().copied()));}
        let accepted_ids = decisions
            .iter()
            .filter(|d| d.accepted)
            .map(|d| d.id)
            .collect::<Vec<_>>();
        s.delete_batch(z, &accepted_ids)?;
        accepted += accepted_ids.len() as u64;
        removed.extend(decisions.iter().filter(|d| d.accepted).map(|d| Removal {
            id: d.id,
            star: d.star.clone(),
        }));
        for d in decisions {
            finished.push((d.stripe, d.sequence));
            if let Some(q) = d.witness {
                remember_witness(&mut witnesses[d.stripe], q);
            }
            caches[d.stripe] = Some(d.patch);
        }
        ensure!(!finished.is_empty(), "独立した削減候補を選べませんでした");
        dequeue(s, z, &finished)?;
        remaining -= finished.len() as u64;
        save_seconds += phase.elapsed().as_secs_f64();
        let phase = Instant::now();
        if !removed.is_empty() {
            // 同期対象は互いに独立したキャッシュ。DBの世代は一度だけ読む。
            let epoch = s.epoch(z)?;
            if deferred_sync {
                caches.par_iter_mut().try_for_each(|p|sync_epoch(epoch,p,&removed))?;
                journal.record(epoch,removed,&mut parked);
            } else {
            caches
                .par_iter_mut()
                .chain(
                    parked
                        .par_iter_mut()
                        .flat_map_iter(|stripe| stripe.iter_mut()),
                )
                .try_for_each(|cache| sync_epoch(epoch, cache, &removed))?;
            for stripe in &mut parked {
                stripe.retain(Option::is_some);
            }
            }
        }
        sync_seconds += phase.elapsed().as_secs_f64();
        turn = (turn + 1) % workers;
        if (total - remaining) % 32 < workers as u64 || remaining == 0 {
            progress(
                root,
                json!({"stage":"prune-parallel","z":z,"done":total-remaining,"total":total,"accepted":accepted,"workers":physical_threads,"lanes":workers,"complete":false,"profile":{"prepare_seconds":prepare_seconds,"evaluate_seconds":evaluate_seconds,"save_seconds":save_seconds,"sync_seconds":sync_seconds,"cache_rebuilds":cache_rebuilds,"dem":dem_profile(),"closed_circles":s.certificate_stats(),"preflight_rejections":preflight_rejections,"serial_candidates":serial_candidates,"expanded_candidates":expanded_candidates,"unaffected_regions":unaffected_regions,"early_rejections":early_rejections,"evaluated_candidates":evaluated_candidates,"elapsed_seconds":started.elapsed().as_secs_f64()}}),
            )?;
        }
    }
    let benchmark = s.db.query_row("SELECT 1 FROM meta WHERE key='benchmark_prune_only' AND EXISTS(SELECT 1 FROM meta WHERE key='benchmark_only')",[],|_|Ok(())).optional()?.is_some();
    if benchmark {
        let core_seconds=started.elapsed().as_secs_f64();
        let audit_started=Instant::now();
        let quality=if benchmark_audit {
            let scope=inputs.iter().filter(|i|touched_inputs.contains(&(i.x,i.y))).cloned().collect::<Vec<_>>();
            let (ok,max,valid)=audit_all(s,z,error,&scope,dem,root,true)?;
            ensure!(ok,"限定キューの削減後独立監査が未達です");
            json!({"all_valid_pixels":true,"inputs":scope.len(),"pixels":valid,"maximum_error":max,"seconds":audit_started.elapsed().as_secs_f64()})
        } else {Value::Null};
        let report=json!({"scope":"prune-only / limited real queue / no final audit","workers":physical_threads,"lanes":workers,"block_candidates":block_candidates,"block_size":block_size,"blocks_enabled":blocks_enabled,"fused_preparation":true,"deferred_sync":deferred_sync,"processed_candidates":initial_remaining,"accepted":accepted,"seconds":core_seconds,"quality_audit":quality,"prepare_seconds":prepare_seconds,"evaluate_seconds":evaluate_seconds,"save_seconds":save_seconds,"sync_seconds":sync_seconds,"preflight_rejections":preflight_rejections,"cache_rebuilds":cache_rebuilds,"dem":dem_profile(),"closed_circles":s.certificate_stats(),"national_complete":false,"distributable":false});
        fs::write(root.join("benchmark-prune.json"),serde_json::to_vec_pretty(&report)?)?;
        return Ok(());
    }
    let (ok, max, valid) = audit_all(s, z, error, inputs, dem, root, true)?;
    ensure!(ok, "並列削減後の全画素監査が未達です");
    done(
        s,
        &name,
        json!({"max":max,"valid":valid,"epoch":s.epoch(z)?,"workers":workers,"maximum_parallel_jobs":maximum_parallel_jobs,"processed_this_run":initial_remaining,"seconds_this_run":started.elapsed().as_secs_f64()}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deferred_removal_journal_matches_eager_sync() -> Result<()> {
        let s=Store::open(Path::new(":memory:"),4096)?;
        for q in [[-16000,-16000,0],[16000,-16000,0],[16000,16000,0],[-16000,16000,0]] {s.insert(13,q,true)?;}
        for y in 0..21 {for x in 0..21 {s.insert(13,[x*160,y*160,(x*13+y*17)%31],false)?;}}
        s.cache_level(13)?;let b=[40.,40.,160.,160.];
        let mut eager=Some(s.patch(13,b)?);let mut parked=vec![vec![Some(s.patch(13,b)?)]];
        let mut journal=RemovalJournal::new(s.epoch(13)?);
        for xy in [[80*16,80*16],[90*16,90*16],[100*16,100*16]] {
            let id:i64=s.db.query_row("SELECT id FROM points WHERE x=? AND y=?",params![xy[0],xy[1]],|r|r.get(0))?;
            let p=s.patch(13,b)?;let k=p.index_of(id).unwrap();let r=Removal{id,star:star(&p,k)};
            s.delete_batch(13,&[id])?;let epoch=s.epoch(13)?;
            sync_epoch(epoch,&mut eager,std::slice::from_ref(&r))?;
            journal.record(epoch,vec![r],&mut parked);
        }
        let mut lazy=parked[0].pop().unwrap();journal.restore(s.epoch(13)?,&mut lazy)?;
        assert_eq!(lazy.is_some(),eager.is_some());
        let (a,bp)=(lazy.unwrap(),eager.unwrap());let mut aw=Work::default();let mut bw=Work::default();
        for y in 0..9 {for x in 0..9 {let q=[45.+x as f64*13.,45.+y as f64*13.];assert!((a.evaluate(q,&mut aw)?-bp.evaluate(q,&mut bw)?).abs()<1e-8);}}
        let mut expired=vec![vec![Some(s.patch(13,b)?)]];let mut small=RemovalJournal::new(s.epoch(13)?);small.limit=0;
        small.record(s.epoch(13)?+1,vec![Removal{id:0,star:BTreeSet::new()}],&mut expired);
        assert!(expired[0].is_empty());Ok(())
    }
    #[test]
    fn locality_keeps_queue_members_and_epoch_and_is_resumable() -> Result<()> {
        let s=Store::open(Path::new(":memory:"),4096)?;
        s.db.execute_batch("CREATE TABLE prune_queue(z INTEGER,stripe INTEGER,sequence INTEGER,id INTEGER,PRIMARY KEY(z,stripe,sequence)) WITHOUT ROWID")?;
        for k in 0..48i32 {
            s.insert(13,[100000+(k%4)*90000,100000+(k/4)*160,k*8],false)?;
        }
        let ids=s.db.prepare("SELECT id FROM points ORDER BY id")?.query_map([],|r|r.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for (i,id) in ids.iter().enumerate() {s.db.execute("INSERT INTO prune_queue VALUES(13,0,?,?)",params![i+1,id])?;}
        let epoch=s.epoch(13)?;
        let root=std::env::temp_dir().join(format!("queue-locality-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        fs::create_dir(&root)?;
        localize_queue(&s,13,&root)?;
        assert_eq!(s.epoch(13)?,epoch);
        let after=s.db.prepare("SELECT id FROM prune_queue ORDER BY id")?.query_map([],|r|r.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        assert_eq!(after,ids);
        let before:i64=s.db.query_row("SELECT count(*) FROM prune_queue_before_locality_z13",[],|r|r.get(0))?;
        assert_eq!(before,ids.len() as i64);
        localize_queue(&s,13,&root)?;
        assert_eq!(s.epoch(13)?,epoch);
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn small_preflight_matches_global_virtual_deletion() -> Result<()> {
        let s=Store::open(Path::new(":memory:"),8192)?;
        for p in [[-160000,-160000,0],[160000,-160000,0],[160000,160000,0],[-160000,160000,0]] {s.insert(13,p,true)?;}
        for y in 0..33 {for x in 0..33 {
            s.insert(13,[x*160+(x*7+y*3)%11,y*160+(x*3+y*5)%13,(x*19+y*23)%113],false)?;
        }}
        s.cache_level(13)?;
        let ids=s.db.prepare("SELECT id FROM points WHERE guard=0 ORDER BY id LIMIT 12 OFFSET 400")?
            .query_map([],|r|r.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for id in ids {
            let q=s.point(id)?;let d=STEP*2.;let b=[q[0]-d,q[1]-d,q[0]+d,q[1]+d];
            let mut tiny=s.patch(13,b)?;
            let k=tiny.ids.iter().position(|&v|v==id).unwrap();
            let u=tiny.model.remove_local(k)?;
            let full=s.patch_omitting(13,b,Some(id))?;
            let mut a=Work::at_face(u.start_face);let mut bw=Work::default();
            for dy in [-0.04,0.,0.04] {for dx in [-0.04,0.,0.04] {
                let at=[q[0]+dx,q[1]+dy];
                assert!((tiny.evaluate(at,&mut a)?-full.evaluate(at,&mut bw)?).abs()<1e-8);
            }}
        }
        Ok(())
    }
    #[test]
    fn sibson_is_unchanged_outside_deleted_vertex_circles() -> Result<()> {
        let s = Store::open(Path::new(":memory:"), 4096)?;
        for p in [
            [-16000, -16000, 0],
            [16000, -16000, 0],
            [16000, 16000, 0],
            [-16000, 16000, 0],
        ] {
            s.insert(13, p, true)?;
        }
        for y in 0..21 {
            for x in 0..21 {
                s.insert(
                    13,
                    [
                        x * 160 + (y * 3 + x * 7) % 31,
                        y * 160 + (x * 5 + y * 11) % 29,
                        ((x * 7 + y * 11) % 31) * 8,
                    ],
                    false,
                )?;
            }
        }
        let mut p = s.patch(13, [20., 20., 180., 180.])?;
        let k = p
            .ids
            .iter()
            .position(|&id| {
                s.point(id)
                    .is_ok_and(|q| q[0] > 98. && q[0] < 103. && q[1] > 98. && q[1] < 103.)
            })
            .unwrap();
        let cs = circles(&p, k);
        let mut queries = Vec::new();
        let mut work = Work::default();
        for y in 0..41 {
            for x in 0..41 {
                let q = [20. + x as f64 * 4., 20. + y as f64 * 4.];
                if cs
                    .iter()
                    .all(|&c| !circle_hits(c, [q[0], q[1], q[0], q[1]]))
                {
                    queries.push((q, p.evaluate(q, &mut work)?));
                }
            }
        }
        assert!(queries.len() > 1000);
        p.model.remove_local(k)?;
        let mut work = Work::default();
        for (q, h) in queries {
            assert!((p.evaluate(q, &mut work)? - h).abs() < 1e-8);
        }
        Ok(())
    }
    #[test]
    fn outside_proved_star_updates_without_dropping_cache() -> Result<()> {
        let s = Store::open(Path::new(":memory:"), 4096)?;
        for q in [
            [-16000, -16000, 0],
            [16000, -16000, 0],
            [16000, 16000, 0],
            [-16000, 16000, 0],
        ] {
            s.insert(13, q, true)?;
        }
        for y in 0..49 {
            for x in 0..49 {
                s.insert(
                    13,
                    [
                        x * 160 + (x * 3 + y * 7) % 13,
                        y * 160 + (x * 11 + y * 5) % 17,
                        (x * 13 + y * 17) % 31,
                    ],
                    false,
                )?;
            }
        }
        s.cache_level(13)?;
        let b = [90., 90., 150., 150.];
        let local = s.patch(13, b)?;
        let relevant = local.model.base.support_vertices(local.local_rect(b));
        let mut selected = None;
        for k in relevant {
            let q = local.point(k);
            if inside([q[0], q[1]], b) || local.model.base.topology.hull.contains(&k) {
                continue;
            }
            let full = s.patch(13, [q[0] - 0.1, q[1] - 0.1, q[0] + 0.1, q[1] + 0.1])?;
            let j = full.ids.iter().position(|&id| id == local.ids[k]).unwrap();
            let proof = star(&full, j);
            if star(&local, k) == proof {
                selected = Some(Removal {
                    id: local.ids[k],
                    star: proof,
                });
                break;
            }
        }
        let r = selected.context("範囲外の証明済み星形接続がありません")?;
        let mut cache = Some(local);
        s.delete_batch(13, &[r.id])?;
        sync(&s, 13, &mut cache, &[r])?;
        let local = cache
            .as_ref()
            .context("証明済みの範囲外削除は再利用できます")?;
        let full = s.patch(13, b)?;
        let mut a = Work::default();
        let mut c = Work::default();
        for y in 0..21 {
            for x in 0..21 {
                let q = [90. + x as f64 * 3., 90. + y as f64 * 3.];
                assert!((local.evaluate(q, &mut a)? - full.evaluate(q, &mut c)?).abs() < 1e-8);
            }
        }
        Ok(())
    }
    #[test]
    fn cached_regions_match_global_after_far_and_near_removals() -> Result<()> {
        let s = Store::open(Path::new(":memory:"), 4096)?;
        for q in [
            [-16000, -16000, 0],
            [16000, -16000, 0],
            [16000, 16000, 0],
            [-16000, 16000, 0],
        ] {
            s.insert(13, q, true)?;
        }
        for y in 0..49 {
            for x in 0..49 {
                s.insert(13, [x * 160, y * 160, (x * 13 + y * 17) % 31], false)?;
            }
        }
        s.cache_level(13)?;
        let b = [90., 90., 150., 150.];
        let mut cache = Some(s.patch(13, b)?);
        let far =
            s.db.prepare("SELECT id FROM active WHERE z=13")?
                .query_map([], |r| r.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .find(|id| !cache.as_ref().unwrap().ids.contains(id))
                .context("領域外の未読点がありません")?;
        let near = cache
            .as_ref()
            .unwrap()
            .ids
            .iter()
            .copied()
            .filter(|&id| {
                let p = s.point(id).unwrap();
                p[0] > 95. && p[0] < 145. && p[1] > 95. && p[1] < 145.
            })
            .take(12)
            .collect::<Vec<_>>();
        let local = cache.as_ref().unwrap();
        let relevant = local.model.base.support_vertices(local.local_rect(b));
        let extra = local
            .ids
            .iter()
            .enumerate()
            .filter(|(k, _)| !relevant.contains(k))
            .map(|(_, &id)| id)
            .filter(|&id| s.point(id).unwrap()[0] > 0.)
            .take(4)
            .collect::<Vec<_>>();
        for id in std::iter::once(far).chain(extra).chain(near) {
            s.delete_batch(13, &[id])?;
            sync(
                &s,
                13,
                &mut cache,
                &[Removal {
                    id,
                    star: BTreeSet::new(),
                }],
            )?;
            let local = cache.as_ref().context("領域内更新は再利用できるはずです")?;
            let full = s.patch(13, b)?;
            let mut a = Work::default();
            let mut c = Work::default();
            for y in 0..21 {
                for x in 0..21 {
                    let q = [90. + x as f64 * 3., 90. + y as f64 * 3.];
                    assert!((local.evaluate(q, &mut a)? - full.evaluate(q, &mut c)?).abs() < 1e-8);
                }
            }
        }
        Ok(())
    }
    #[test]
    fn lane_split_preserves_members_epoch_and_restart() -> Result<()> {
        let s=Store::open(Path::new(":memory:"),8192)?;
        s.db.execute_batch("CREATE TABLE prune_queue(z INTEGER,stripe INTEGER,sequence INTEGER,id INTEGER,PRIMARY KEY(z,stripe,sequence)) WITHOUT ROWID")?;
        for k in 0..32i64 {let id=s.insert(13,[k as i32*16,32,0],false)?;s.db.execute("INSERT INTO prune_queue VALUES(13,?,?,?)",params![k%2,k+1,id])?;}
        s.db.execute("INSERT INTO stages VALUES('prune-queue-13','done',?)",[json!({"workers":2,"candidates":32}).to_string()])?;
        let epoch=s.epoch(13)?;
        let root=std::env::temp_dir().join(format!("prune-lanes-{}",std::process::id()));fs::create_dir_all(&root)?;
        split_queue(&s,13,2,&root)?;split_queue(&s,13,2,&root)?;
        assert_eq!(s.epoch(13)?,epoch);assert_eq!(state(&s,"prune-queue-13")?.unwrap()["workers"],4);
        assert_eq!(s.db.query_row("SELECT count(*) FROM prune_queue",[],|r|r.get::<_,i64>(0))?,32);
        assert_eq!(s.db.query_row("SELECT count(*) FROM (SELECT id FROM prune_queue_before_lanes_z13 EXCEPT SELECT id FROM prune_queue)",[],|r|r.get::<_,i64>(0))?,0);
        assert_eq!(s.db.query_row("SELECT count(*) FROM (SELECT id FROM prune_queue EXCEPT SELECT id FROM prune_queue_before_lanes_z13)",[],|r|r.get::<_,i64>(0))?,0);
        fs::remove_dir_all(root)?;Ok(())
    }

}
