#[path="natural_shared_fit_cache.rs"]
mod fit_cache;
#[path = "natural_shared_prune.rs"]
mod parallel_prune;
use crate::{
    natural_codec, natural_inventory,
    natural_regional_surface::Work,
    natural_shared_store::{Patch, Store},
};
use anyhow::{Context, Result, ensure};
use rayon::prelude::*;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
const WORLD: f64 = 40075016.68557849;
const STEP: f64 = WORLD / 4194304.;
const LEVELS: [(i64, f64); 8] = [
    (13, 25.),
    (12, 60.),
    (11, 125.),
    (10, 200.),
    (9, 250.),
    (8, 300.),
    (7, 400.),
    (6, 500.),
];
#[derive(Clone)]
struct Input {
    x: i32,
    y: i32,
    sha: String,
}
static DEM_CACHE_HITS: std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(0);
static DEM_CACHE_MISSES: std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(0);
static DEM_READ_NS: std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(0);
fn dem_profile()->Value {use std::sync::atomic::Ordering::Relaxed;json!({"hits":DEM_CACHE_HITS.load(Relaxed),"misses":DEM_CACHE_MISSES.load(Relaxed),"read_decode_seconds":DEM_READ_NS.load(Relaxed) as f64/1e9,"tile_limit":std::env::var("TERRAIN_DEM_CACHE_TILES").ok().and_then(|s|s.parse::<usize>().ok()).unwrap_or(4096).clamp(128,4096)})}
struct Dem {
    root: PathBuf,
    cache: HashMap<(i32, i32), Arc<Vec<f64>>>,
    order: VecDeque<(i32, i32)>,
}
impl Dem {
    fn read_many(&mut self,inputs:&[Input])->Result<Vec<Arc<Vec<f64>>>> {
        let mut result=vec![None;inputs.len()];let mut missing=Vec::new();
        for (k,i) in inputs.iter().enumerate() {
            if let Some(a)=self.cache.get(&(i.x,i.y)) {DEM_CACHE_HITS.fetch_add(1,std::sync::atomic::Ordering::Relaxed);result[k]=Some(a.clone());}
            else {missing.push((k,i.clone()));}
        }
        let root=self.root.clone();
        let loaded=missing.into_par_iter().map(|(k,i)| -> Result<_> {
            let mut reader=Dem{root:root.clone(),cache:HashMap::new(),order:VecDeque::new()};
            let a=reader.read(&i)?;Ok((k,i,a))
        }).collect::<Result<Vec<_>>>()?;
        let limit=std::env::var("TERRAIN_DEM_CACHE_TILES").ok().and_then(|s|s.parse::<usize>().ok()).unwrap_or(4096).clamp(128,4096);
        for (k,i,a) in loaded {
            let key=(i.x,i.y);
            if !self.cache.contains_key(&key) {
                if self.cache.len()>=limit {if let Some(old)=self.order.pop_front(){self.cache.remove(&old);}}
                self.cache.insert(key,a.clone());self.order.push_back(key);
            }
            result[k]=Some(a);
        }
        Ok(result.into_iter().map(Option::unwrap).collect())
    }
    fn read(&mut self, i: &Input) -> Result<Arc<Vec<f64>>> {
        let key = (i.x, i.y);
        if let Some(v) = self.cache.get(&key) {
            DEM_CACHE_HITS.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
            return Ok(v.clone());
        }
        DEM_CACHE_MISSES.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
        let read_started=Instant::now();
        let path = self
            .root
            .join("14")
            .join(i.x.to_string())
            .join(format!("{}.png", i.y));
        let bytes = fs::read(&path).with_context(|| format!("PNG読込 {}", path.display()))?;
        ensure!(
            natural_inventory::hash(&bytes) == i.sha,
            "固定済みPNGが変更されました"
        );
        let a = Arc::new(natural_inventory::decode(&bytes)?);
        DEM_READ_NS.fetch_add(read_started.elapsed().as_nanos() as u64,std::sync::atomic::Ordering::Relaxed);
        let cache_limit=std::env::var("TERRAIN_DEM_CACHE_TILES").ok().and_then(|s|s.parse::<usize>().ok()).unwrap_or(4096).clamp(128,4096);
        if self.cache.len() >= cache_limit {
            if let Some(k) = self.order.pop_front() {
                self.cache.remove(&k);
            }
        }
        self.cache.insert(key, a.clone());
        self.order.push_back(key);
        Ok(a)
    }
}
fn rect(i: &Input) -> [f64; 4] {
    let side = WORLD / 16384.;
    [
        i.x as f64 * side,
        i.y as f64 * side,
        (i.x + 1) as f64 * side,
        (i.y + 1) as f64 * side,
    ]
}
fn quant(q: [f64; 3]) -> Result<[i32; 3]> {
    let a = [
        (q[0] * 16. + 0.5).floor(),
        (q[1] * 16. + 0.5).floor(),
        (q[2] * 8. + 0.5).floor(),
    ];
    ensure!(
        a.iter()
            .all(|&v| v >= i32::MIN as f64 && v <= i32::MAX as f64),
        "整数座標が範囲外"
    );
    Ok(a.map(|v| v as i32))
}
fn score(delta: f64) -> f64 {
    if delta > 0. { delta * 4. } else { -delta }
}
#[derive(Clone)]
struct InputBox {
    index: usize,
    bounds: rstar::AABB<[f64; 2]>,
}
impl rstar::RTreeObject for InputBox {
    type Envelope = rstar::AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.bounds
    }
}
impl rstar::PointDistance for InputBox {
    fn distance_2(&self,p:&[f64;2])->f64 {
        let lo=self.bounds.lower();let hi=self.bounds.upper();
        let x=p[0].clamp(lo[0],hi[0]);let y=p[1].clamp(lo[1],hi[1]);(p[0]-x).powi(2)+(p[1]-y).powi(2)
    }
}
fn input_index(inputs: &[Input]) -> rstar::RTree<InputBox> {
    rstar::RTree::bulk_load(
        inputs
            .iter()
            .enumerate()
            .map(|(index, i)| {
                let b = rect(i);
                InputBox {
                    index,
                    bounds: rstar::AABB::from_corners([b[0], b[1]], [b[2], b[3]]),
                }
            })
            .collect(),
    )
}
fn affected_inputs(inputs: &[Input], index: &rstar::RTree<InputBox>, b: [f64; 4]) -> Vec<Input> {
    let envelope = rstar::AABB::from_corners([b[0], b[1]], [b[2], b[3]]);
    let mut ids = index
        .locate_in_envelope_intersecting(&envelope)
        .map(|v| v.index)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.into_iter().map(|i| inputs[i].clone()).collect()
}
fn nearest_fine(s: &Store, z: i64, q: [f64; 2]) -> Result<Option<i64>> {
    let max_radius = q[0].abs() + q[1].abs() + WORLD * 2.;
    let mut radius = STEP * 2.;
    let mut st=s.db.prepare("SELECT p.id,((p.x-?7)*(p.x-?7)+(p.y-?8)*(p.y-?8))/256. FROM point_index i CROSS JOIN points p ON p.id=i.id CROSS JOIN active a ON a.id=p.id AND a.z=?1 WHERE i.x1>=?3 AND i.y1>=?4 AND i.x0<=?5 AND i.y0<=?6 AND p.guard=0 AND NOT EXISTS(SELECT 1 FROM active b WHERE b.z=?2 AND b.id=p.id) ORDER BY 2,p.id LIMIT 1")?;
    loop {
        let found = st
            .query_row(
                params![
                    z + 1,
                    z,
                    (q[0] - radius) * 16.,
                    (q[1] - radius) * 16.,
                    (q[0] + radius) * 16.,
                    (q[1] + radius) * 16.,
                    q[0] * 16.,
                    q[1] * 16.
                ],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)),
            )
            .optional()?;
        if let Some((id, distance)) = found {
            if distance.sqrt() <= radius {
                return Ok(Some(id));
            }
        }
        if radius >= max_radius {
            return Ok(None);
        }
        radius = (radius * 2.).min(max_radius);
    }
}
fn state(s: &Store, k: &str) -> Result<Option<Value>> {
    Ok(s.db
        .query_row(
            "SELECT details FROM stages WHERE name=? AND state='done'",
            [k],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .map(|v| serde_json::from_str(&v))
        .transpose()?)
}
fn done(s: &Store, k: &str, v: Value) -> Result<()> {
    s.db.execute(
        "INSERT OR REPLACE INTO stages VALUES(?,'done',?)",
        params![k, v.to_string()],
    )?;
    Ok(())
}
fn progress(root: &Path, mut v: Value) -> Result<()> {
    ensure!(
        !root.join("stop-requested").exists(),
        "停止要求を受けました。保存済み点・削減候補から再開できます"
    );
    #[cfg(not(test))]
    crate::natural_shared_resources::check(root, 100 * 1024u64.pow(3))?;
    v["resources"] = crate::natural_shared_resources::report();
    natural_inventory::progress(root, v)
}
fn initialize(root: &Path, limit: usize, window: &[u32]) -> Result<(Store, Value, Vec<Input>)> {
    initialize_internal(root, limit, window, false)
}
fn initialize_internal(
    root: &Path,
    limit: usize,
    window: &[u32],
    allow_benchmark: bool,
) -> Result<(Store, Value, Vec<Input>)> {
    let job: Value = serde_json::from_slice(&fs::read(root.join("job.json"))?)?;
    ensure!(
        job["format"] == "national-shared-prepared-v1" && job["inputs_verified"] == true,
        "入力の全件検証が必要です"
    );
    let config = &job["settings"];
    ensure!(
        config["profile"] == "sibson-shared-no-lpf-z6-z13" && config["generation"]["lpf"] == false,
        "旧設定との混在です"
    );
    ensure!(
        window.is_empty()
            || window.len() == 4
                && window[0] < window[2]
                && window[1] < window[3]
                && window[2] <= 16384
                && window[3] <= 16384,
        "z14窓は左上・右下の半開区間です"
    );
    let configured = config["levels"].as_array().context("品質表がありません")?;
    ensure!(configured.len() == 8, "品質数が違います");
    for &(z, e) in &LEVELS {
        let level = configured
            .iter()
            .find(|v| v["z"].as_i64() == Some(z))
            .context("品質が不足")?;
        ensure!(
            level["error_down_m"].as_f64() == Some(e)
                && level["error_up_m"].as_f64() == Some(e / 4.)
                && level["anchor"].as_bool() == Some([6, 9, 12].contains(&z)),
            "品質表がエンジンと不一致 z{z}"
        );
    }
    ensure!(
        config["codec"]["integer_profile"] == "rice-hybrid-v1"
            && config["codec"]["xy_step_m"] == 0.0625
            && config["codec"]["z_step_m"] == 0.125
            && config["codec"]["outer_zlib"] == false,
        "整数形式がエンジンと不一致"
    );
    let s = Store::open(&root.join("work.sqlite3"), limit)?;
    ensure!(
        allow_benchmark
            || s.db
                .query_row(
                    "SELECT value FROM meta WHERE key='benchmark_only'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .optional()?
                .is_none(),
        "性能試験用DBを全国本番へ流用できません"
    );
    let identity = json!({"generation":job["generation_id"],"window":window,"algorithm":"shared-certified-v1"});
    if let Some(old) =
        s.db.query_row("SELECT value FROM meta WHERE key='identity'", [], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
    {
        ensure!(
            serde_json::from_str::<Value>(&old)? == identity,
            "異なる作業範囲・世代では再開できません"
        );
    } else {
        s.db.execute(
            "INSERT INTO meta VALUES('identity',?)",
            [identity.to_string()],
        )?;
    }
    let c = Connection::open_with_flags(
        root.join("inputs.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut st = c.prepare(
        "SELECT x,y,sha256 FROM inputs WHERE state='cached' AND verified=1 ORDER BY y,x",
    )?;
    let inputs = st
        .query_map([], |r| {
            Ok(Input {
                x: r.get(0)?,
                y: r.get(1)?,
                sha: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|i| {
            window.is_empty()
                || i.x >= window[0] as i32
                    && i.x < (window[2] as i32)
                    && i.y >= window[1] as i32
                    && i.y < (window[3] as i32)
        })
        .collect::<Vec<_>>();
    ensure!(!inputs.is_empty(), "有効な入力候補がありません");
    if state(&s, "guards")?.is_none() {
        for p in [
            [0., 0., 0.],
            [WORLD, 0., 0.],
            [WORLD, WORLD, 0.],
            [0., WORLD, 0.],
        ] {
            s.insert(13, quant(p)?, true)?;
        }
        done(&s, "guards", json!({"count":4}))?;
    }
    Ok((s, job, inputs))
}
struct Audit {
    max: f64,
    up: f64,
    down: f64,
    valid: u64,
    candidate: Option<[i32; 3]>,
}
fn audit_patch(p: &Patch, i: &Input, raster: &[f64]) -> Result<Audit> {
    let blocks = (0..16)
        .into_par_iter()
        .map(|k| audit_region(p, i, raster, [0, k * 16, 256, (k + 1) * 16]))
        .collect::<Result<Vec<_>>>()?;
    let mut out = Audit {
        max: 0.,
        up: 0.,
        down: 0.,
        valid: 0,
        candidate: None,
    };
    for a in blocks {
        out.valid += a.valid;
        out.up = out.up.max(a.up);
        out.down = out.down.max(a.down);
        if a.max > out.max {
            out.max = a.max;
            out.candidate = a.candidate;
        }
    }
    Ok(out)
}
fn audit_region(p: &Patch, i: &Input, raster: &[f64], range: [usize; 4]) -> Result<Audit> {
    audit_region_until(p, i, raster, range, f64::INFINITY)
}
fn audit_region_until(
    p: &Patch,
    i: &Input,
    raster: &[f64],
    range: [usize; 4],
    limit: f64,
) -> Result<Audit> {
    let mut w = Work::at_face(p.hint_face);
    let mut a = Audit {
        max: 0.,
        up: 0.,
        down: 0.,
        valid: 0,
        candidate: None,
    };
    for y in range[1]..range[3] {
        for x in range[0]..range[2] {
            let index = y * 256 + x;
            let h = raster[index];
            if !h.is_finite() {
                continue;
            }
            let q = [
                (i.x as f64 * 256. + (index % 256) as f64 + 0.5) * STEP,
                (i.y as f64 * 256. + (index / 256) as f64 + 0.5) * STEP,
            ];
            let delta = p.evaluate(q, &mut w)? - h;
            a.valid += 1;
            a.up = a.up.max(delta);
            a.down = a.down.max(-delta);
            let v = score(delta);
            if v > a.max {
                a.max = v;
                a.candidate = Some(quant([q[0], q[1], h])?);
            }
            if a.max > limit {
                return Ok(a);
            }
        }
    }
    Ok(a)
}
fn region_bound(
    p: &Patch,
    i: &Input,
    raster: &[f64],
    range: [usize; 4],
) -> Result<Option<(f64, u64)>> {
    let mut count = 0u64;
    let samples = (range[1]..range[3])
        .flat_map(|y| (range[0]..range[2]).map(move |x| (x, y)))
        .filter_map(|(x, y)| {
            let h = raster[y * 256 + x];
            h.is_finite().then_some((
                [
                    (i.x as f64 * 256. + x as f64 + 0.5) * STEP - p.origin[0],
                    (i.y as f64 * 256. + y as f64 + 0.5) * STEP - p.origin[1],
                ],
                h,
            ))
        });
    for y in range[1]..range[3] {
        for x in range[0]..range[2] {
            count += u64::from(raster[y * 256 + x].is_finite());
        }
    }
    if count == 0 {
        return Ok(None);
    }
    let b = p.local_rect([
        (i.x as f64 * 256. + range[0] as f64) * STEP,
        (i.y as f64 * 256. + range[1] as f64) * STEP,
        (i.x as f64 * 256. + range[2] as f64) * STEP,
        (i.y as f64 * 256. + range[3] as f64) * STEP,
    ]);
    let [up, down] = p.model.affine_error_bounds_from(b, samples, p.hint_face)?;
    Ok(Some(((up * 4.).max(down), count)))
}
fn audit_fit_region(p: &Patch, i: &Input, raster: &[f64], range: [usize; 4]) -> Result<Audit> {
    match region_bound(p, i, raster, range)? {
        None => Ok(Audit {
            max: 0.,
            up: 0.,
            down: 0.,
            valid: 0,
            candidate: None,
        }),
        Some((bound, count)) if bound <= 25. => Ok(Audit {
            max: bound,
            up: bound / 4.,
            down: bound,
            valid: count,
            candidate: None,
        }),
        _ => audit_region(p, i, raster, range),
    }
}
fn region_within_error(
    p: &Patch,
    i: &Input,
    raster: &[f64],
    range: [usize; 4],
    error: f64,
) -> Result<Option<[f64; 3]>> {
    for y in (range[1]..range[3]).step_by(16) {
        for x in (range[0]..range[2]).step_by(16) {
            let r = [x, y, (x + 16).min(range[2]), (y + 16).min(range[3])];
            if region_bound(p, i, raster, r)?.is_none_or(|(b, _)| b <= error) {
                continue;
            }
            let a = audit_region_until(p, i, raster, r, error)?;
            if a.max > error {
                let q = a.candidate.context("反例画素がありません")?;
                let gx = (q[0] as f64 / 16. / STEP - 0.5).round() as i32;
                let gy = (q[1] as f64 / 16. / STEP - 0.5).round() as i32;
                let h = raster[(gy.rem_euclid(256) * 256 + gx.rem_euclid(256)) as usize];
                return Ok(Some([
                    (gx as f64 + 0.5) * STEP,
                    (gy as f64 + 0.5) * STEP,
                    h,
                ]));
            }
        }
    }
    Ok(None)
}
fn remember_witness(points: &mut Vec<[f64; 3]>, p: [f64; 3]) {
    if points.len() >= 64 {
        points.remove(0);
    }
    points.push(p);
}
fn audit_all(s:&Store,z:i64,error:f64,inputs:&[Input],dem:&mut Dem,root:&Path,save:bool)->Result<(bool,f64,u64)> {
    audit_all_policy(s,z,error,inputs,dem,root,save,!save)
}
fn audit_all_policy(s:&Store,z:i64,error:f64,inputs:&[Input],dem:&mut Dem,root:&Path,save:bool,stop_on_error:bool)->Result<(bool,f64,u64)> {
    let epoch=s.epoch(z)?;let mut good=true;let mut maximum=0f64;let mut valid=0u64;let mut reused=0usize;
    let workers=rayon::current_num_threads().clamp(1,10);let started=Instant::now();
    let mut prepare_seconds=0.;let mut evaluate_seconds=0.;let mut save_seconds=0.;
    for (batch,group) in inputs.chunks(workers).enumerate() {
        let start=batch*workers;let phase=Instant::now();
        ensure!(!root.join("stop-requested").exists(),"監査を停止。保存済みの同世代監査は再利用できます");
        let rasters=dem.read_many(group)?;
        let cached={let mut stmt=s.db.prepare_cached("SELECT valid,up,down FROM audit WHERE z=? AND x=? AND y=? AND epoch=?")?;
            group.iter().map(|i|stmt.query_row(params![z,i.x,i.y,epoch],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,f64>(1)?,r.get::<_,f64>(2)?))).optional()).collect::<rusqlite::Result<Vec<_>>>()?};
        let missing=cached.iter().enumerate().filter_map(|(k,v)|v.is_none().then_some(k)).collect::<Vec<_>>();
        let patches=s.patches(z,&missing.iter().map(|&k|rect(&group[k])).collect::<Vec<_>>())?;
        prepare_seconds+=phase.elapsed().as_secs_f64();let phase=Instant::now();
        let computed=missing.into_par_iter().zip(patches.into_par_iter()).map(|(k,p)|Ok((k,audit_patch(&p,&group[k],&rasters[k])?))).collect::<Result<Vec<_>>>()?;
        evaluate_seconds+=phase.elapsed().as_secs_f64();ensure!(s.epoch(z)?==epoch,"監査中に点集合が変わりました");
        let mut results=cached.iter().map(|v|v.map(|(valid,up,down)|Audit{max:(up*4.).max(down),up,down,valid,candidate:None})).collect::<Vec<_>>();
        for (k,a) in computed {results[k]=Some(a);}
        let phase=Instant::now();
        if save {let tx=s.db.unchecked_transaction()?;{
            let mut stmt=tx.prepare_cached("INSERT OR REPLACE INTO audit VALUES(?,?,?,?,?,?,?)")?;
            for (k,i) in group.iter().enumerate() {if cached[k].is_none() {let a=results[k].as_ref().context("監査結果なし")?;stmt.execute(params![z,i.x,i.y,epoch,a.valid,a.up,a.down])?;}}
        }tx.commit()?;}
        save_seconds+=phase.elapsed().as_secs_f64();
        for (k,a) in results.into_iter().enumerate() {let a=a.context("監査結果なし")?;maximum=maximum.max(a.max);valid+=a.valid;reused+=usize::from(cached[k].is_some());good&=a.max<=error+1e-8;
            if stop_on_error&&!good {progress(root,json!({"stage":"audit","z":z,"done":start+k+1,"total":inputs.len(),"maximum_directional_error":maximum,"valid_pixels":valid,"reused_inputs":reused,"fresh_inputs":start+k+1-reused,"early_failure":true,"workers":workers,"complete":false}))?;return Ok((false,maximum,valid));}
        }
        if batch%4==0 || start+group.len()==inputs.len() {progress(root,json!({"stage":"audit","z":z,"done":start+group.len(),"total":inputs.len(),"maximum_directional_error":maximum,"valid_pixels":valid,"reused_inputs":reused,"fresh_inputs":start+group.len()-reused,"workers":workers,"profile":{"prepare_seconds":prepare_seconds,"evaluate_seconds":evaluate_seconds,"save_seconds":save_seconds,"elapsed_seconds":started.elapsed().as_secs_f64()},"complete":false}))?;}
    }Ok((good,maximum,valid))
}
fn audit_all_serial_reference(
    s: &Store,
    z: i64,
    error: f64,
    inputs: &[Input],
    dem: &mut Dem,
    root: &Path,
    save: bool,
) -> Result<(bool, f64, u64)> {
    let mut good = true;
    let mut maximum = 0f64;
    let mut valid = 0;
    let mut reused = 0u64;
    let epoch = s.epoch(z)?;
    for (n, i) in inputs.iter().enumerate() {
        // 元PNGの検証は省略しない。同じ点集合の世代で計測済みなら数値だけ再利用する。
        let r = dem.read(i)?;
        let cached =
            s.db.query_row(
                "SELECT valid,up,down FROM audit WHERE z=? AND x=? AND y=? AND epoch=?",
                params![z, i.x, i.y, epoch],
                |r| {
                    Ok((
                        r.get::<_, u64>(0)?,
                        r.get::<_, f64>(1)?,
                        r.get::<_, f64>(2)?,
                    ))
                },
            )
            .optional()?;
        let a = if let Some((valid, up, down)) = cached {
            reused += 1;
            Audit {
                max: (up * 4.).max(down),
                up,
                down,
                valid,
                candidate: None,
            }
        } else {
            let p = s.patch(z, rect(i))?;
            audit_patch(&p, i, &r)?
        };
        maximum = maximum.max(a.max);
        valid += a.valid;
        good &= a.max <= error + 1e-8;
        if save && cached.is_none() {
            s.db.execute(
                "INSERT OR REPLACE INTO audit VALUES(?,?,?,?,?,?,?)",
                params![z, i.x, i.y, epoch, a.valid, a.up, a.down],
            )?;
        }
        if n % 32 == 0 || n + 1 == inputs.len() {
            progress(
                root,
                json!({"stage":"audit","z":z,"done":n+1,"total":inputs.len(),"maximum_directional_error":maximum,"valid_pixels":valid,"reused_inputs":reused,"fresh_inputs":n+1-reused as usize,"complete":false}),
            )?;
        }
        if !save && !good {
            return Ok((false, maximum, valid));
        }
    }
    Ok((good, maximum, valid))
}
fn audit_affected_cached(
    s: &Store,
    z: i64,
    error: f64,
    inputs: &[Input],
    dem: &mut Dem,
    influence: [f64; 4],
    cached: Option<&Patch>,
    mut witnesses: Option<&mut Vec<[f64; 3]>>,
) -> Result<bool> {
    for input in inputs {
        let range = [
            ((influence[0] / STEP - input.x as f64 * 256. - 1.)
                .floor()
                .clamp(0., 256.)) as usize,
            ((influence[1] / STEP - input.y as f64 * 256. - 1.)
                .floor()
                .clamp(0., 256.)) as usize,
            ((influence[2] / STEP - input.x as f64 * 256. + 1.)
                .ceil()
                .clamp(0., 256.)) as usize,
            ((influence[3] / STEP - input.y as f64 * 256. + 1.)
                .ceil()
                .clamp(0., 256.)) as usize,
        ];
        if range[0] >= range[2] || range[1] >= range[3] {
            continue;
        }
        let b = [
            (input.x as f64 * 256. + range[0] as f64) * STEP,
            (input.y as f64 * 256. + range[1] as f64) * STEP,
            (input.x as f64 * 256. + range[2] as f64) * STEP,
            (input.y as f64 * 256. + range[3] as f64) * STEP,
        ];
        let local;
        let p = if let Some(p) = cached.filter(|p| {
            p.epoch == s.epoch(z).unwrap_or(-1)
                && b[0] >= p.certified_bounds[0]
                && b[1] >= p.certified_bounds[1]
                && b[2] <= p.certified_bounds[2]
                && b[3] <= p.certified_bounds[3]
        }) {
            p
        } else {
            local = s.patch(z, b)?;
            &local
        };
        let raster = dem.read(input)?;
        if let Some(q) = region_within_error(p, input, &raster, range, error + 1e-8)? {
            if let Some(w) = witnesses.as_mut() {
                remember_witness(w, q);
            }
            return Ok(false);
        }
    }
    Ok(true)
}
/// 未生成地域の巨大な空円を避ける初期点。各PNGの実在する最高点を一つ置く。
/// 初期点は固定せず、通常の誤差条件による削減対象とする。
fn seed_global(s: &Store, inputs: &[Input], dem: &Dem, root: &Path) -> Result<()> {
    if state(s, "seed-global-v1")?.is_some() {
        return Ok(());
    }
    let mut cursor =
        s.db.query_row(
            "SELECT value FROM meta WHERE key='seed-global-cursor'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(|| "0".into())
        .parse::<usize>()?;
    let mut added = 0;
    let cache_root = dem.root.clone();
    while cursor < inputs.len() {
        let end = (cursor + 128).min(inputs.len());
        let points = inputs[cursor..end]
            .par_iter()
            .map(|i| -> Result<Option<[i32; 3]>> {
                let mut local = Dem {
                    root: cache_root.clone(),
                    cache: HashMap::new(),
                    order: VecDeque::new(),
                };
                let raster = local.read(i)?;
                let mut best = None;
                for (k, &h) in raster.iter().enumerate() {
                    if h.is_finite() && best.is_none_or(|(_, old)| h > old) {
                        best = Some((k, h));
                    }
                }
                best.map(|(k, h)| {
                    quant([
                        (i.x as f64 * 256. + (k % 256) as f64 + 0.5) * STEP,
                        (i.y as f64 * 256. + (k / 256) as f64 + 0.5) * STEP,
                        h,
                    ])
                })
                .transpose()
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        added += s.insert_batch(13, &points)?;
        cursor = end;
        s.db.execute(
            "INSERT OR REPLACE INTO meta VALUES('seed-global-cursor',?)",
            [cursor.to_string()],
        )?;
        progress(
            root,
            json!({"stage":"seed-global","done":cursor,"total":inputs.len(),"added_this_run":added,"complete":false}),
        )?;
    }
    done(
        s,
        "seed-global-v1",
        json!({"inputs":inputs.len(),"added_this_run":added,"epoch":s.epoch(13)?,"policy":"one actual highest DEM pixel per input; removable"}),
    )
}
struct FitProposal {
    input: Input,
    bounds: [f64; 4],
    points: Vec<[i32; 3]>,
    circles: Vec<[f64; 3]>,
}
fn fit_bounds(i: &Input) -> [f64; 4] {
    let b = rect(i);
    [
        b[0] - STEP * 2.,
        b[1] - STEP * 2.,
        b[2] + STEP * 2.,
        b[3] + STEP * 2.,
    ]
}
fn circle_hits(c: [f64; 3], b: [f64; 4]) -> bool {
    let x = c[0].clamp(b[0], b[2]);
    let y = c[1].clamp(b[1], b[3]);
    (c[0] - x).powi(2) + (c[1] - y).powi(2) <= c[2] * (1. + 1e-12) + 1e-8
}
fn proposal_insert(
    q: [i32; 3],
    p: &mut Patch,
    points: &mut Vec<[i32; 3]>,
    circles: &mut Vec<[f64; 3]>,
    known: &mut HashMap<[i32; 2], i32>,
) -> Result<bool> {
    if let Some(&h) = known.get(&[q[0], q[1]]) {
        ensure!(h == q[2], "提案内の同じXYで標高が不一致です");
        return Ok(false);
    }
    let xy = [q[0] as f64 / 16., q[1] as f64 / 16.];
    let b = p.certified_bounds;
    ensure!(
        xy[0] >= b[0] && xy[1] >= b[1] && xy[0] <= b[2] && xy[1] <= b[3],
        "適合提案が証明範囲外へはみ出しました"
    );
    ensure!(
        p.ids.len() < p.point_limit,
        "適合提案の局所点数予算を超過しました"
    );
    let local = [xy[0] - p.origin[0], xy[1] - p.origin[1]];
    for face in p.model.base.faces_for_point(local)? {
        let mut c = p.model.base.circles[face];
        c[0] += p.origin[0];
        c[1] += p.origin[1];
        circles.push(c);
    }
    p.model
        .insert_local_from([local[0], local[1], q[2] as f64 / 8.], p.hint_face)?;
    for &face in p.model.base.incident.last().context("挿入頂点なし")? {
        let mut c=p.model.base.circles[face];c[0]+=p.origin[0];c[1]+=p.origin[1];circles.push(c);
    }
    p.hint_face = *p
        .model
        .base
        .incident
        .last()
        .and_then(|v| v.first())
        .context("適合提案の挿入面がありません")?;
    p.id_index.take();
    p.ids.push(-1 - points.len() as i64);
    p.epoch += 1;
    points.push(q);
    known.insert([q[0], q[1]], q[2]);
    Ok(true)
}
fn fit_proposal(
    mut p: Patch,
    i: &Input,
    raster: &[f64],
    source: &HashMap<(i32, i32), &Input>,
    dem: &mut Dem,
    root: &Path,
) -> Result<FitProposal> {
    let mut additions = Vec::new();
    let mut circles = Vec::new();
    let mut known = HashMap::new();
    let mut added = 0;
    for k in 0..p.ids.len() {
        let q = quant(p.point(k))?;
        known.insert([q[0], q[1]], q[2]);
    }
    let ranges = (0..256)
        .map(|k| {
            [
                (k % 16) * 16,
                (k / 16) * 16,
                (k % 16 + 1) * 16,
                (k / 16 + 1) * 16,
            ]
        })
        .collect::<Vec<_>>();
    let mut blocks = ranges
        .par_iter()
        .map(|&r| audit_fit_region(&p, i, &raster, r))
        .collect::<Result<Vec<_>>>()?;
    loop {
        let index = (0..blocks.len())
            .max_by(|&a, &b| blocks[a].max.total_cmp(&blocks[b].max))
            .unwrap();
        let a = &blocks[index];
        if a.max <= 25. + 1e-8 {
            break;
        }
        if a.max > 25. + 1e-8 {
            let q = a.candidate.context("違反画素がありません")?;
            let local = [
                q[0] as f64 / 16. - p.origin[0],
                q[1] as f64 / 16. - p.origin[1],
            ];
            let changed_box = p.model.insertion_box(local);
            let mut influence = [
                changed_box[0] + p.origin[0],
                changed_box[1] + p.origin[1],
                changed_box[2] + p.origin[0],
                changed_box[3] + p.origin[1],
            ];
            if proposal_insert(q, &mut p, &mut additions, &mut circles, &mut known)? {
                added += 1;
            } else {
                influence = rect(i);
                // 量子化点の外側へ元画素中心がわずかにはみ出すと、遠い外周支持点が
                // 自然近傍の重みに入る。元DEMの端を一画素だけ延長し、0mを捏造しない。

                let gx = (q[0] as f64 / 16. / STEP - 0.5).round() as i32;
                let gy = (q[1] as f64 / 16. / STEP - 0.5).round() as i32;
                let mut padded = 0;
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let (x, y) = (gx + dx, gy + dy);
                    let nearest =
                        if let Some(input) = source.get(&(x.div_euclid(256), y.div_euclid(256))) {
                            dem.read(input)?[(y.rem_euclid(256) * 256 + x.rem_euclid(256)) as usize]
                        } else {
                            f64::NAN
                        };
                    let h = if nearest.is_finite() {
                        nearest
                    } else {
                        q[2] as f64 / 8.
                    };
                    if proposal_insert(
                        quant([(x as f64 + 0.5) * STEP, (y as f64 + 0.5) * STEP, h])?,
                        &mut p,
                        &mut additions,
                        &mut circles,
                        &mut known,
                    )? {
                        padded += 1;
                    }
                }
                ensure!(
                    padded > 0,
                    "量子化後の元画素誤差を解消できません: {}/{} score={}",
                    i.x,
                    i.y,
                    a.max
                );
                added += padded;
            }
            if added % 32 == 0 {
                ensure!(
                    !root.join("stop-requested").exists(),
                    "停止要求を受けました。確定済み提案は保存済みです"
                );
            }
            for (k, r) in ranges.iter().copied().enumerate() {
                let b = [
                    (i.x as f64 * 256. + r[0] as f64) * STEP,
                    (i.y as f64 * 256. + r[1] as f64) * STEP,
                    (i.x as f64 * 256. + r[2] as f64) * STEP,
                    (i.y as f64 * 256. + r[3] as f64) * STEP,
                ];
                if b[0] <= influence[2]
                    && b[1] <= influence[3]
                    && b[2] >= influence[0]
                    && b[3] >= influence[1]
                {
                    blocks[k] = audit_fit_region(&p, i, &raster, r)?;
                }
            }
        }
    }
    Ok(FitProposal {
        input: i.clone(),
        bounds: p.certified_bounds,
        points: additions,
        circles,
    })
}
fn fit_finest(s: &Store, inputs: &[Input], dem: &mut Dem, root: &Path) -> Result<()> {
    let workers = if inputs.len() < 512 {
        1
    } else {
        rayon::current_num_threads().max(1)
    };
    fit_finest_workers(s, inputs, dem, root, workers)
}
fn fit_finest_workers(
    s: &Store,
    inputs: &[Input],
    dem: &mut Dem,
    root: &Path,
    workers: usize,
) -> Result<()> {
    fit_finest_budget(s, inputs, dem, root, workers, None)
}
fn fit_finest_budget(
    s: &Store,
    inputs: &[Input],
    dem: &mut Dem,
    root: &Path,
    workers: usize,
    batches: Option<usize>,
) -> Result<()> {
    if state(s, "fit-13")?.is_some_and(|v| v["epoch"].as_i64() == Some(s.epoch(13).unwrap_or(-1))) {
        return Ok(());
    }
    let source = inputs
        .iter()
        .map(|i| ((i.x, i.y), i))
        .collect::<HashMap<_, _>>();
    s.cache_level(13)?;
    let mut index_base = s.cached_points(13).unwrap_or(0);
    let mut index_rebuilds = 0usize;
    // 小範囲の試験は順次処理。全国では緯度方向の離れた帯を同じバッチへ置く。
    let workers = workers.max(1).min(inputs.len());
    let mut total_fallback = 0;
    let stride = inputs.len().div_ceil(workers);
    let mut signature = Sha256::new();
    for i in inputs {
        signature.update(i.x.to_le_bytes());
        signature.update(i.y.to_le_bytes());
        signature.update(i.sha.as_bytes());
    }
    let signature = signature
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mut resume = if batches.is_none() {
        s.db.query_row(
            "SELECT value FROM meta WHERE key='fit-cursor-v1'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .map(|v| serde_json::from_str::<Value>(&v))
        .transpose()?
    } else {
        None
    };
    if resume.as_ref().is_some_and(|v| {
        v["signature"] != signature
            || v["total"].as_u64() != Some(inputs.len() as u64)
            || v["workers"].as_u64() != Some(workers as u64)
            || v["epoch"].as_i64() != Some(s.epoch(13).unwrap_or(-1))
    }) {
        eprintln!("適合カーソルの世代・入力・並列数が変わったため、全域の確認走査から再開します");
        resume = None;
    }
    if batches.is_some() {
        let offset =
            s.db.query_row(
                "SELECT value FROM meta WHERE key='benchmark_start_offset'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(|| "0".into())
            .parse::<usize>()?;
        resume = Some(
            json!({"pass":1,"next_offset":offset,"completed":0,"added":0,"fallback":0,"elapsed_seconds":0.}),
        );
    }
    let serial_prepare=s.db.query_row("SELECT 1 FROM meta WHERE key='benchmark_serial_fit_prepare' AND EXISTS(SELECT 1 FROM meta WHERE key='benchmark_only')",[],|_|Ok(())).optional()?.is_some();
    let mut verified=fit_cache::Verified::load(s,inputs,&signature)?;
    let repair_active=s.db.query_row("SELECT 1 FROM meta WHERE key='certify-repair-fit-13-v1'",[],|_|Ok(())).optional()?.is_some();
    if verified.enabled&&verified.count()==0&&repair_active&&batches.is_none() {
        progress(root,json!({"stage":"fit-verify-seed","z":13,"total":inputs.len(),"note":"全画素の実測で、再利用できる適合済み領域を分類","complete":false}))?;
        let (ok,max,valid)=audit_all(s,13,25.,inputs,dem,root,true)?;
        verified.seed_from_audit(s)?;verified.save(s,&signature)?;
        if ok {
            done(s,"fit-13",json!({"epoch":s.epoch(13)?,"max":max,"valid":valid,"workers":workers,"verified_seed_complete":true}))?;
            s.db.execute("DELETE FROM meta WHERE key='fit-cursor-v1'",[])?;return Ok(());
        }
    }
    let mut reused_total=0usize;
    let mut pass = resume
        .as_ref()
        .and_then(|v| v["pass"].as_u64())
        .unwrap_or(1)
        - 1;
    loop {
        pass += 1;
        let saved = resume.take();
        let offset_start = saved
            .as_ref()
            .and_then(|v| v["next_offset"].as_u64())
            .unwrap_or(0) as usize;
        ensure!(offset_start <= stride, "適合カーソルが走査範囲外です");
        let mut added = saved
            .as_ref()
            .and_then(|v| v["added"].as_u64())
            .unwrap_or(0) as usize;
        let mut completed = saved
            .as_ref()
            .and_then(|v| v["completed"].as_u64())
            .unwrap_or(0) as usize;
        let resumed_completed = completed;
        let elapsed_before = saved
            .as_ref()
            .and_then(|v| v["elapsed_seconds"].as_f64())
            .unwrap_or(0.);
        let mut fallback = saved
            .as_ref()
            .and_then(|v| v["fallback"].as_u64())
            .unwrap_or(0) as usize;
        let started = Instant::now();
        let mut prepare_seconds = 0.;
        let mut proposal_seconds = 0.;
        let mut commit_seconds = 0.;
        for offset in offset_start..stride {
            let indices = (0..workers)
                .map(|w| w * stride + offset)
                .filter(|&k| k < inputs.len())
                .collect::<Vec<_>>();
            ensure!(
                !root.join("stop-requested").exists(),
                "停止要求を受けました。確定済みバッチ位置から再開できます"
            );
            let prepare_started = Instant::now();
            let mut pending = Vec::new();
            for &k in &indices {
                let i = &inputs[k];
                let raster = dem.read(i)?;
                if verified.has(k) {reused_total+=1;continue;}
                if raster.iter().any(|h| h.is_finite()) {pending.push((i.clone(),raster));}
                else {verified.mark(i);}

            }
            let dem_root = dem.root.clone();
            let use_fused = !serial_prepare
                && std::env::var_os("TERRAIN_SEPARATE_FIT_STAGES").is_none();
            let proposals = if use_fused {
                let snapshot = s.read_snapshot(13)?;
                if let Some(snapshot) = snapshot {
                    // 同世代の準備と適合を一仕事にする。遅い準備の完了待ちで全CPUを止めない。
                    let result = pending.into_par_iter().map(|(i, raster)| {
                        let local_store = snapshot.open()?;
                        let support_started = Instant::now();
                        let patch = local_store.patch(13, fit_bounds(&i))?;
                        let support_seconds = support_started.elapsed().as_secs_f64();
                        let fit_started = Instant::now();
                        let mut local_dem = Dem { root: dem_root.clone(), cache: HashMap::new(), order: VecDeque::new() };
                        let result = fit_proposal(patch, &i, &raster, &source, &mut local_dem, root);
                        if std::env::var_os("TERRAIN_FIT_PROFILE").is_some() {
                            eprintln!("fit-profile {}/{} support={:.6} fit={:.6}", i.x, i.y, support_seconds, fit_started.elapsed().as_secs_f64());
                        }
                        result
                    }).collect::<Result<Vec<_>>>()?;
                    prepare_seconds += prepare_started.elapsed().as_secs_f64();
                    result
                } else {
                    let mut result = Vec::new();
                    for (i, raster) in pending {
                        let patch = s.patch(13, fit_bounds(&i))?;
                        result.push(fit_proposal(patch, &i, &raster, &source, dem, root)?);
                    }
                    prepare_seconds += prepare_started.elapsed().as_secs_f64();
                    result
                }
            } else {
            let patches = if serial_prepare {
                pending
                    .iter()
                    .map(|(i, _)| s.patch(13, fit_bounds(i)))
                    .collect::<Result<Vec<_>>>()?
            } else {
                s.patches(
                    13,
                    &pending
                        .iter()
                        .map(|(i, _)| fit_bounds(i))
                        .collect::<Vec<_>>(),
                )?
            };
            let jobs = pending
                .into_iter()
                .zip(patches)
                .map(|((i, raster), patch)| (i, patch, raster))
                .collect::<Vec<_>>();
            prepare_seconds += prepare_started.elapsed().as_secs_f64();
            let proposal_started = Instant::now();
            let dem_root = dem.root.clone();
            let proposals = jobs
                .into_par_iter()
                .map(|(i, p, raster)| {
                    let mut local = Dem {
                        root: dem_root.clone(),
                        cache: HashMap::new(),
                        order: VecDeque::new(),
                    };
                    fit_proposal(p, &i, &raster, &source, &mut local, root)
                })
                .collect::<Result<Vec<_>>>()?;
                proposal_seconds += proposal_started.elapsed().as_secs_f64();
                proposals
            };
            let commit_started = Instant::now();
            let mut committed = Vec::<[f64; 3]>::new();
            for mut proposal in proposals {
                // 先行する提案の影響円が届く場合だけ、現在の全国点集合から再計算する。
                if committed.iter().any(|&c| circle_hits(c, proposal.bounds)) {
                    fallback += 1;
                    total_fallback += 1;
                    let i = &proposal.input;
                    let raster = dem.read(i)?;
                    proposal =
                        fit_proposal(s.patch(13, fit_bounds(i))?, i, &raster, &source, dem, root)?;
                }
                let n = s.insert_batch(13, &proposal.points)?;
                ensure!(
                    n == proposal.points.len(),
                    "確定前の提案点集合が変化しました"
                );
                added += n;
                verified.invalidate(&proposal.circles);
                if n==0 {verified.mark(&proposal.input);}
                committed.extend(proposal.circles);
            }
            commit_seconds += commit_started.elapsed().as_secs_f64();
            completed += indices.len();
            if batches.is_none() {
                let cursor = json!({"format":"fit-cursor-v1","signature":signature,"total":inputs.len(),"workers":workers,"pass":pass,"next_offset":offset+1,"completed":completed,"added":added,"fallback":fallback,"elapsed_seconds":elapsed_before+started.elapsed().as_secs_f64(),"epoch":s.epoch(13)?});
                let tx=s.db.unchecked_transaction()?;
                s.db.execute("INSERT OR REPLACE INTO meta VALUES('fit-cursor-v1',?)",[cursor.to_string()])?;
                verified.save(s,&signature)?;tx.commit()?;
            }
            if batches.is_some() {verified.save(s,&signature)?;}
            if index_base > 0
                && s.cached_points(13)
                    .is_some_and(|n| n >= index_base + index_base.div_ceil(4))
            {
                s.rebuild_cache(13)?;
                index_base = s.cached_points(13).unwrap_or(0);
                index_rebuilds += 1;
            }
            if offset % 8 == 0 || offset + 1 == stride {
                progress(
                    root,
                    json!({"stage":"fit-batch","z":13,"pass":pass,"done":completed,"total":inputs.len(),"workers":workers,"fallback_tiles":fallback,"added_this_pass":added,"pass_seconds":elapsed_before+started.elapsed().as_secs_f64(),"resumed_completed_inputs":resumed_completed,"inputs_this_run":completed-resumed_completed,"run_seconds":started.elapsed().as_secs_f64(),"index_rebuilds_this_run":index_rebuilds,"verified_inputs_reused":reused_total,"profile":{"prepare_seconds":prepare_seconds,"proposal_seconds":proposal_seconds,"commit_seconds":commit_seconds},"complete":false}),
                )?;
            }
            if batches.is_some_and(|n| offset - offset_start + 1 >= n) {
                let report = json!({"scope":"fit-only / national snapshot","processed_inputs":completed,"input_offset":offset_start,"total_inputs":inputs.len(),"workers":workers,"seconds":started.elapsed().as_secs_f64(),"conflicting_proposals_recomputed":fallback,"added_points":added,"verified_inputs_reused":reused_total,"inputs_per_second":completed as f64/started.elapsed().as_secs_f64(),"preparation_seconds":prepare_seconds,"proposal_seconds":proposal_seconds,"commit_seconds":commit_seconds,"national_complete":false,"distributable":false});
                fs::write(
                    root.join("benchmark-fit.json"),
                    serde_json::to_vec_pretty(&report)?,
                )?;
                progress(
                    root,
                    json!({"stage":"benchmark-complete","complete":false,"benchmark":report}),
                )?;
                return Ok(());
            }
        }
        if added == 0 {
            let (ok, max, valid) = audit_all(s, 13, 25., inputs, dem, root, true)?;
            if !ok {s.db.execute("DELETE FROM fit_verified",[])?;}
            ensure!(ok, "最終監査に失敗。適合領域キャッシュは破棄しました");
            done(
                s,
                "fit-13",
                json!({"epoch":s.epoch(13)?,"max":max,"valid":valid,"passes":pass,"workers":workers,"conflicting_proposals_recomputed":total_fallback,"resumed_completed_inputs":resumed_completed,"verified_inputs_reused":reused_total}),
            )?;
            s.db.execute("DELETE FROM meta WHERE key='fit-cursor-v1'", [])?;
            break;
        }
        if batches.is_none() {
            let cursor = json!({"format":"fit-cursor-v1","signature":signature,"total":inputs.len(),"workers":workers,"pass":pass+1,"next_offset":0,"completed":0,"added":0,"fallback":0,"elapsed_seconds":0.,"epoch":s.epoch(13)?});
            s.db.execute(
                "INSERT OR REPLACE INTO meta VALUES('fit-cursor-v1',?)",
                [cursor.to_string()],
            )?;
        }
    }
    Ok(())
}
fn prune(
    s: &Store,
    z: i64,
    fine: i64,
    error: f64,
    inputs: &[Input],
    dem: &mut Dem,
    root: &Path,
) -> Result<()> {
    if inputs.len() > 1 {
        parallel_prune::run(
            s,
            z,
            fine,
            error,
            inputs,
            dem,
            root,
            rayon::current_num_threads().min(inputs.len()),
        )
    } else {
        prune_serial(s, z, fine, error, inputs, dem, root)
    }
}
fn prune_serial(
    s: &Store,
    z: i64,
    fine: i64,
    error: f64,
    inputs: &[Input],
    dem: &mut Dem,
    root: &Path,
) -> Result<()> {
    let source = inputs
        .iter()
        .map(|i| ((i.x, i.y), i))
        .collect::<HashMap<_, _>>();
    let spatial_inputs = input_index(inputs);
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
    s.cache_level(z)?;
    // 再開時は未確定の削除をDBトランザクションが戻す。試行済みの候補は保持する。
    let cursor_key = format!("cursor-{z}");
    let mut cursor =
        s.db.query_row("SELECT value FROM meta WHERE key=?", [&cursor_key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .unwrap_or_else(|| "0".into())
        .parse::<i64>()?;
    let mut patch_cache: Option<Patch> = None;
    let mut witnesses = Vec::new();
    loop {
        let id=s.db.query_row("SELECT a.id FROM active a JOIN points p ON p.id=a.id WHERE a.z=? AND a.id>? AND p.guard=0 ORDER BY a.id LIMIT 1",params![z,cursor],|r|r.get::<_,i64>(0)).optional()?;
        let Some(id) = id else { break };
        let q = s.point(id)?;
        let refresh = patch_cache.as_ref().is_none_or(|p| {
            p.epoch != s.epoch(z).unwrap_or(-1)
                || q[0] < p.certified_bounds[0]
                || q[1] < p.certified_bounds[1]
                || q[0] > p.certified_bounds[2]
                || q[1] > p.certified_bounds[3]
        });
        if refresh {
            let side = STEP * 256.;
            let x = (q[0] / side).floor() * side;
            let y = (q[1] / side).floor() * side;
            patch_cache = Some(s.patch(
                z,
                [
                    x - STEP * 2.,
                    y - STEP * 2.,
                    x + side + STEP * 2.,
                    y + side + STEP * 2.,
                ],
            )?);
        }
        let before = patch_cache.as_mut().unwrap();
        let local_id = before
            .ids
            .iter()
            .position(|&v| v == id)
            .context("削除点の接続がありません")?;
        if before.model.base.topology.hull.contains(&local_id) {
            cursor = id;
            continue;
        }
        let influence = before.deletion_box(local_id, STEP * 2.);
        let affected = affected_inputs(inputs, &spatial_inputs, influence);
        let old_hint = before.hint_face;
        let undo = before.model.remove_local(local_id)?;
        before.hint_face = undo.start_face;
        before.ids[local_id] = 0;
        before.epoch += 1;
        let mut witness_failed = false;
        let bounds = before.certified_bounds;
        let mut hint = Work::at_face(before.hint_face);
        for q in &witnesses {
            let q: &[f64; 3] = q;
            if q[0] >= influence[0]
                && q[1] >= influence[1]
                && q[0] <= influence[2]
                && q[1] <= influence[3]
                && q[0] >= bounds[0]
                && q[1] >= bounds[1]
                && q[0] <= bounds[2]
                && q[1] <= bounds[3]
                && score(before.evaluate([q[0], q[1]], &mut hint)? - q[2]) > error + 1e-8
            {
                witness_failed = true;
                break;
            }
        }
        if witness_failed {
            before.model.undo_remove_local(undo);
            before.hint_face = old_hint;
            before.restore_id(local_id,id);
            before.epoch -= 1;
            cursor = id;
            continue;
        }
        // 削除点の元画素だけでも違反する場合、周囲を監査する必要はない。
        let gx = (q[0] / STEP - 0.5).round() as i32;
        let gy = (q[1] / STEP - 0.5).round() as i32;
        if let Some(input) = source.get(&(gx.div_euclid(256), gy.div_euclid(256))) {
            let h = dem.read(input)?[(gy.rem_euclid(256) * 256 + gx.rem_euclid(256)) as usize];
            if h.is_finite() {
                let at = [(gx as f64 + 0.5) * STEP, (gy as f64 + 0.5) * STEP];
                if score(before.evaluate(at, &mut Work::at_face(before.hint_face))? - h)
                    > error + 1e-8
                {
                    remember_witness(&mut witnesses, [at[0], at[1], h]);
                    before.model.undo_remove_local(undo);
                    before.hint_face = old_hint;
                    before.restore_id(local_id,id);
                    before.epoch -= 1;
                    cursor = id;
                    if cursor % 64 == 0 {
                        s.db.execute(
                            "INSERT OR REPLACE INTO meta VALUES(?,?)",
                            params![cursor_key, cursor.to_string()],
                        )?;
                    }
                    continue;
                }
            }
        }
        // 点を削除した旧外接円の領域だけが自然近傍へ影響する。全日本を毎点再走査しない。
        s.db.execute_batch("SAVEPOINT removal")?;
        s.db.execute("DELETE FROM active WHERE z=? AND id=?", params![z, id])?;
        s.bump(z)?;
        s.refresh_cached_point(z, id)?;
        let result = audit_affected_cached(
            s,
            z,
            error,
            &affected,
            dem,
            influence,
            Some(before),
            Some(&mut witnesses),
        );
        match result {
            Ok(true) => {
                s.db.execute("INSERT OR IGNORE INTO removed VALUES(?,?)", params![z, id])?;
                s.db.execute_batch("RELEASE removal")?;
            }
            Ok(false) => {
                s.db.execute_batch("ROLLBACK TO removal;RELEASE removal")?;
                s.refresh_cached_point(z, id)?;
                before.model.undo_remove_local(undo);
                before.hint_face = old_hint;
                before.restore_id(local_id,id);
                before.epoch -= 1;
            }
            Err(e) => {
                s.db.execute_batch("ROLLBACK TO removal;RELEASE removal")?;
                s.refresh_cached_point(z, id)?;
                before.model.undo_remove_local(undo);
                before.hint_face = old_hint;
                before.restore_id(local_id,id);
                before.epoch -= 1;
                return Err(e);
            }
        }
        cursor = id;
        s.db.execute(
            "INSERT OR REPLACE INTO meta VALUES(?,?)",
            params![cursor_key, cursor.to_string()],
        )?;
    }
    let (ok, max, valid) = audit_all(s, z, error, inputs, dem, root, true)?;
    ensure!(ok, "粗品質の元DEM監査に失敗");
    done(
        s,
        &name,
        json!({"max":max,"valid":valid,"epoch":s.epoch(z)?}),
    )?;
    Ok(())
}
fn triangle_intersects(v: &[[f64; 3]], b: [f64; 4]) -> bool {
    let r = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]];
    let mut axes = vec![[1., 0.], [0., 1.]];
    for k in 0..3 {
        axes.push([v[(k + 1) % 3][1] - v[k][1], v[k][0] - v[(k + 1) % 3][0]]);
    }
    for a in axes {
        let projection = |x: f64, y: f64| a[0] * x + a[1] * y;
        let lo = v
            .iter()
            .map(|p| projection(p[0], p[1]))
            .fold(f64::INFINITY, f64::min);
        let hi = v
            .iter()
            .map(|p| projection(p[0], p[1]))
            .fold(f64::NEG_INFINITY, f64::max);
        let rlo = r
            .iter()
            .map(|p| projection(p[0], p[1]))
            .fold(f64::INFINITY, f64::min);
        let rhi = r
            .iter()
            .map(|p| projection(p[0], p[1]))
            .fold(f64::NEG_INFINITY, f64::max);
        if hi < rlo || rhi < lo {
            return false;
        }
    }
    true
}
fn minimum_angle(p: &Patch, b: [f64; 4]) -> f64 {
    minimum_angle_surface(&p.model.base, p.local_rect(b))
}
// 三角形に接する外接円は矩形に必ず接する。索引の再構築なしで完全探索する。
fn worst_angle_surface(surface:&crate::natural_surface::Surface,b:[f64;4],origin:[f64;2])->(f64,Option<[[f64;3];3]>) {
    let local=[b[0]-origin[0],b[1]-origin[1],b[2]-origin[0],b[3]-origin[1]];
    let mut faces=surface.faces_for_rect_walk(local).unwrap_or_else(|_|(0..surface.circles.len()).collect());
    faces.sort_unstable();faces.dedup();
    let mut maximum=-1f64;let mut worst=None;
    for face in faces {
        let t=&surface.topology.triangles[face*3..face*3+3];
        let v=[t[0],t[1],t[2]].map(|k|{let p=surface.points[k];[p[0]+origin[0],p[1]+origin[1],p[2]]});
        if !triangle_intersects(&v,b) {continue;}
        for k in 0..3 {
            let a=[v[(k+1)%3][0]-v[k][0],v[(k+1)%3][1]-v[k][1]];
            let c=[v[(k+2)%3][0]-v[k][0],v[(k+2)%3][1]-v[k][1]];
            let dot=((a[0]*c[0]+a[1]*c[1])/(a[0].hypot(a[1])*c[0].hypot(c[1]))).clamp(-1.,1.);
            if dot>maximum {maximum=dot;worst=Some(v);}
        }
    }
    (maximum.acos().to_degrees(),worst)
}
fn minimum_angle_surface(surface:&crate::natural_surface::Surface,b:[f64;4])->f64 {worst_angle_surface(surface,b,[0.,0.]).0}
#[cfg(test)]
fn minimum_angle_surface_reference(surface: &crate::natural_surface::Surface, b: [f64; 4]) -> f64 {
    let mut minimum = 180f64;
    for t in surface.topology.triangles.chunks_exact(3) {
        let v = [
            surface.points[t[0]],
            surface.points[t[1]],
            surface.points[t[2]],
        ];
        if !triangle_intersects(&v, b) {
            continue;
        }
        for k in 0..3 {
            let a = [v[(k + 1) % 3][0] - v[k][0], v[(k + 1) % 3][1] - v[k][1]];
            let c = [v[(k + 2) % 3][0] - v[k][0], v[(k + 2) % 3][1] - v[k][1]];
            let dot = (a[0] * c[0] + a[1] * c[1]) / (a[0].hypot(a[1]) * c[0].hypot(c[1]));
            minimum = minimum.min(dot.clamp(-1., 1.).acos().to_degrees());
        }
    }
    minimum
}
fn certify_cursor(s:&Store,z:i64,inputs:usize)->Result<Option<Value>> {
    let value=s.db.query_row("SELECT value FROM meta WHERE key=?",[format!("certify-cursor-{z}-v1")],|r|r.get::<_,String>(0)).optional()?;
    let Some(value)=value else{return Ok(None);};let mut v:Value=serde_json::from_str(&value)?;
    ensure!(v["inputs"].as_u64()==Some(inputs as u64),"角度補修カーソルの入力数が異なります");
    ensure!(v["next_input"].as_u64().is_some_and(|n|n<=inputs as u64),"角度補修カーソルの位置が不正です");
    let saved=v["epoch"].as_i64().context("補修カーソル世代なし")?;
    ensure!(saved<=s.epoch(z)?,"補修カーソルが点集合より新しいです");
    if saved<s.epoch(z)? {v["changes"]=json!(v["changes"].as_u64().unwrap_or(0).max(1));}
    Ok(Some(v))
}
fn save_certify_cursor(s:&Store,z:i64,inputs:usize,pass:u64,next:usize,changes:u64,min:f64)->Result<()> {
    s.db.execute("INSERT OR REPLACE INTO meta VALUES(?,?)",params![format!("certify-cursor-{z}-v1"),json!({"inputs":inputs,"pass":pass,"next_input":next,"changes":changes,"minimum_angle":min,"epoch":s.epoch(z)?}).to_string()])?;Ok(())
}
fn repair_certify_fit(s:&Store,inputs:&[Input],dem:&mut Dem,root:&Path)->Result<()> {
    s.db.execute("INSERT OR REPLACE INTO meta VALUES('certify-repair-fit-13-v1',?)",[json!({"inputs":inputs.len(),"epoch":s.epoch(13)?}).to_string()])?;
    fit_finest(s,inputs,dem,root)?;
    s.db.execute("DELETE FROM meta WHERE key='certify-repair-fit-13-v1'",[])?;Ok(())
}
// 挿入空洞とその頂点の一周近傍を含め、傾き復元が変わる範囲も無効化する。
fn repair_support_circles(p: &Patch, xy: [f64; 2]) -> Result<Vec<[f64; 3]>> {
    let faces = p.model.base.faces_for_point([xy[0]-p.origin[0], xy[1]-p.origin[1]])?;
    let mut selected = BTreeSet::new();
    for face in faces {
        selected.insert(face);
        for &vertex in &p.model.base.topology.triangles[face*3..face*3+3] {
            selected.extend(p.model.base.incident[vertex].iter().copied());
        }
    }
    Ok(selected.into_iter().map(|face| {
        let mut c = p.model.base.circles[face];
        c[0] += p.origin[0]; c[1] += p.origin[1]; c
    }).collect())
}
fn repair_insert(s: &Store, z: i64, q: [i32; 3], p: &mut Patch,
    angles: &mut fit_cache::Verified, errors: &mut fit_cache::Verified) -> Result<bool> {
    let xy = [q[0] as f64/16., q[1] as f64/16.];
    let mut circles = repair_support_circles(p, xy)?;
    let changed = s.insert_cached(z, q, p)?;
    if changed {
        circles.extend(repair_support_circles(p, xy)?);
        angles.invalidate(&circles);
        errors.invalidate(&circles);
    }
    Ok(changed)
}
fn certify(s: &Store, z: i64, inputs: &[Input], dem: &mut Dem, root: &Path) -> Result<()> {
    s.cache_level(z)?;
    if z==13&&s.db.query_row("SELECT 1 FROM meta WHERE key='certify-repair-fit-13-v1'",[],|_|Ok(())).optional()?.is_some(){repair_certify_fit(s,inputs,dem,root)?;}
    let spatial_inputs = input_index(inputs);
    let local_checks = z < 13 && std::env::var_os("TERRAIN_FULL_CERTIFY").is_none();
    let mut angle_proofs = fit_cache::Verified::empty(inputs);
    let mut error_proofs = fit_cache::Verified::empty(inputs);
    let key = format!("certify-{z}");
    if state(s, &key)?.is_some_and(|v| v["epoch"].as_i64() == Some(s.epoch(z).unwrap_or(-1))) {
        return Ok(());
    }
    let source = inputs
        .iter()
        .map(|i| ((i.x, i.y), i))
        .collect::<HashMap<_, _>>();
    let fine = if z < 13 { Some(z + 1) } else { None };
    let mut resume=certify_cursor(s,z,inputs.len())?;
    let mut pass=resume.as_ref().and_then(|v|v["pass"].as_u64()).unwrap_or(1)-1;
    let virtual_delete=std::env::var_os("TERRAIN_ANGLE_LEGACY_DELETE").is_none();
    let mut virtual_checks=0u64;
    let mut recertifications=0u64;
    let mut recertification_changes=0u64;
    let prefetch_limit=std::thread::available_parallelism().map(|n|(n.get()/2).max(1).min(10)).unwrap_or(1);
    loop {
        pass += 1;
        let cursor=resume.take();
        let mut changes=cursor.as_ref().and_then(|v|v["changes"].as_u64()).unwrap_or(0);
        let mut min=cursor.as_ref().and_then(|v|v["minimum_angle"].as_f64()).unwrap_or(180.);
        let repair_key = format!("certify-repair-{z}");
        let repairing = z < 13 && s.db.query_row(
            "SELECT 1 FROM meta WHERE key=?", [&repair_key], |_| Ok(())
        ).optional()?.is_some();
        let next = if repairing { changes = 0; inputs.len() } else {
            cursor.as_ref().and_then(|v|v["next_input"].as_u64()).unwrap_or(0) as usize
        };
        save_certify_cursor(s,z,inputs.len(),pass,next,changes,min)?;
        let mut prefetched=VecDeque::new();let mut unchanged_streak=0usize;
        for (n, i) in inputs.iter().enumerate().skip(next) {
            if local_checks && angle_proofs.has(n) { continue; }
            let input_epoch = s.epoch(z)?;
            let before=changes;
            if prefetched.is_empty() && unchanged_streak>=8 && prefetch_limit>1 {
                let bounds=inputs[n..(n+prefetch_limit).min(inputs.len())].iter().map(rect).collect::<Vec<_>>();
                prefetched=s.patches(z,&bounds)?.into();
            }
            let mut p=if let Some(p)=prefetched.pop_front(){p}else{s.patch(z,rect(i))?};
            let mut rebuilt_at = changes;
            loop {
                // 長い差分挿入列は正本点集合から再構築して接続を再認証する。
                if changes.saturating_sub(rebuilt_at) >= 32 {
                    let before_angle = minimum_angle(&p, rect(i));
                    let rebuilt = s.patch(z, rect(i))?;
                    let after_angle = minimum_angle(&rebuilt, rect(i));
                    recertifications += 1;
                    if (before_angle-after_angle).abs() > 1e-7 {
                        recertification_changes += 1;
                        angle_proofs.clear();
                        error_proofs.clear();
                    }
                    p = rebuilt;
                    rebuilt_at = changes;
                    prefetched.clear();
                    progress(root,json!({"stage":"certify-local","z":z,"pass":pass,
                        "input":n+1,"inputs":inputs.len(),"changes":changes,
                        "recertifications":recertifications,"recertification_changes":recertification_changes,
                        "angle_before_rebuild":before_angle,"angle_after_rebuild":after_angle,
                        "note":"正本の点集合から局所接続を再構築して角度を再認証","complete":false}))?;
                }
                if p.epoch != s.epoch(z)? {
                    p = s.patch(z, rect(i))?;
                }
                let (angle,worst) = worst_angle_surface(&p.model.base,rect(i),p.origin);
                min = min.min(angle);
                if angle >= 10. - 1e-8 {
                    break;
                }
                if angle < 10. - 1e-8 {
                    // 最悪三角形の最長辺を細分化する。粗品質は細品質の点を戻すだけ。
                    let b = rect(i);
                    let value=angle;
                    let v = worst.context("鋭角面が見つかりません")?;
                    // 密集した同高の冗長点が鋭角を作る場合は、誤差を守れる削除を先に試す。
                    let mut deleted = false;
                    for vertex in v.into_iter().filter(|_| z == 13) {
                        let qp = quant(vertex)?;
                        let id =
                            s.db.query_row(
                                "SELECT id FROM points WHERE x=? AND y=? AND guard=0 AND NOT EXISTS(SELECT 1 FROM angle_banned b WHERE b.z=? AND b.id=points.id)",
                                params![qp[0], qp[1], z],
                                |r| r.get::<_, i64>(0),
                            )
                            .optional()?;
                        let Some(id) = id else { continue };
                        let idx = p
                            .ids
                            .iter()
                            .position(|&v| v == id)
                            .context("接続点がありません")?;
                        let influence = p.deletion_box(idx, STEP * 2.);
                        let affected = affected_inputs(inputs, &spatial_inputs, influence);
                        let local = vertex[0] >= b[0]
                            && vertex[1] >= b[1]
                            && vertex[0] <= b[2]
                            && vertex[1] <= b[3];
                        let old_hint = p.hint_face;
                        let undo = if local {
                            let u = p.model.remove_local(idx)?;
                            p.hint_face = u.start_face;
                            p.ids[idx] = 0;
                            p.epoch += 1;
                            Some(u)
                        } else {
                            None
                        };
                        // 旧影響域と照会画素の余裕まで証明済みなら、DBを試行変更する必要はない。
                        let cb=p.certified_bounds;
                        let contained=local && influence[0]-STEP*2.>=cb[0] && influence[1]-STEP*2.>=cb[1]
                            && influence[2]+STEP*2.<=cb[2] && influence[3]+STEP*2.<=cb[3];
                        if virtual_delete && contained {
                            virtual_checks+=1;
                            let tested=(||->Result<bool>{
                                if minimum_angle(&p,b)<=value+1e-7 {return Ok(false);}
                                let error=LEVELS.iter().find(|v|v.0==z).unwrap().1;
                                for input in &affected {
                                    let range=parallel_prune::sample_range(input,influence);
                                    if range[0]>=range[2]||range[1]>=range[3] {continue;}
                                    // 変更円だけには限定しない。旧経路と同じ矩形の全有効画素を確認する。
                                    let raster=dem.read(input)?;
                                    if region_within_error(&p,input,&raster,range,error+1e-8)?.is_some(){return Ok(false);}
                                }Ok(true)
                            })();
                            if matches!(tested,Ok(true)) {
                                let tx=s.db.unchecked_transaction()?;
                                ensure!(tx.execute("DELETE FROM active WHERE z=? AND id=?",params![z,id])?==1,"角度削減点がありません");
                                s.bump(z)?;
                                tx.execute("INSERT OR IGNORE INTO angle_banned VALUES(?,?)",params![z,id])?;
                                tx.commit()?;s.refresh_cached_point(z,id)?;
                                deleted=true;break;
                            }
                            if let Some(u)=undo {p.model.undo_remove_local(u);p.hint_face=old_hint;p.restore_id(idx,id);p.epoch-=1;}
                            tested?;continue;
                        }
                        s.db.execute_batch("SAVEPOINT angle_delete")?;
                        s.db.execute("DELETE FROM active WHERE z=? AND id=?", params![z, id])?;
                        s.bump(z)?;
                        s.refresh_cached_point(z, id)?;
                        let tested = (|| -> Result<bool> {
                            let external;
                            let after = if local {
                                &p
                            } else {
                                external = s.patch(z, b)?;
                                &external
                            };
                            if minimum_angle(after, b) <= value + 1e-7 {
                                return Ok(false);
                            }
                            let error = LEVELS.iter().find(|v| v.0 == z).unwrap().1;
                            audit_affected_cached(
                                s,
                                z,
                                error,
                                &affected,
                                dem,
                                influence,
                                Some(after),
                                None,
                            )
                        })();
                        match tested {
                            Ok(true) => {
                                s.db.execute(
                                    "INSERT OR IGNORE INTO angle_banned VALUES(?,?)",
                                    params![z, id],
                                )?;
                                s.db.execute_batch("RELEASE angle_delete")?;
                                if !local {
                                    p = s.patch(z, b)?;
                                }
                                deleted = true;
                                break;
                            }
                            Ok(false) => {
                                s.db.execute_batch(
                                    "ROLLBACK TO angle_delete;RELEASE angle_delete",
                                )?;
                                s.refresh_cached_point(z, id)?;
                                if let Some(u) = undo {
                                    p.model.undo_remove_local(u);
                                    p.hint_face = old_hint;
                                    p.restore_id(idx,id);
                                    p.epoch -= 1;
                                }
                            }
                            Err(e) => {
                                s.db.execute_batch(
                                    "ROLLBACK TO angle_delete;RELEASE angle_delete",
                                )?;
                                s.refresh_cached_point(z, id)?;
                                if let Some(u) = undo {
                                    p.model.undo_remove_local(u);
                                    p.hint_face = old_hint;
                                    p.restore_id(idx,id);
                                    p.epoch -= 1;
                                }
                                return Err(e);
                            }
                        }
                    }
                    if deleted {
                        changes += 1;
                        progress(
                            root,
                            json!({"stage":"angle-prune","z":z,"pass":pass,"input":n+1,"inputs":inputs.len(),"changes":changes,"virtual_checks":virtual_checks,"recertifications":recertifications,"recertification_changes":recertification_changes,"complete":false}),
                        )?;
                        continue;
                    }
                    let a = [v[1][0] - v[0][0], v[1][1] - v[0][1]];
                    let d = [v[2][0] - v[0][0], v[2][1] - v[0][1]];
                    let det = 2. * (a[0] * d[1] - a[1] * d[0]);
                    let aa = a[0] * a[0] + a[1] * a[1];
                    let dd = d[0] * d[0] + d[1] * d[1];
                    let center = [
                        v[0][0] + (aa * d[1] - dd * a[1]) / det,
                        v[0][1] + (a[0] * dd - d[0] * aa) / det,
                    ];
                    let mut candidates = vec![center];
                    for edge in 0..3 {
                        for weight in [0.5, 0.25, 0.75] {
                            candidates.push([
                                v[edge][0] * weight + v[(edge + 1) % 3][0] * (1. - weight),
                                v[edge][1] * weight + v[(edge + 1) % 3][1] * (1. - weight),
                            ]);
                        }
                    }
                    candidates.push([
                        (v[0][0] + v[1][0] + v[2][0]) / 3.,
                        (v[0][1] + v[1][1] + v[2][1]) / 3.,
                    ]);
                    let mut chosen = None;
                    for q in candidates {
                        if !q.iter().all(|&v| v >= 0. && v <= WORLD) {
                            continue;
                        }
                        let key = quant([q[0], q[1], 0.])?;
                        let exists=s.db.query_row("SELECT 1 FROM points p WHERE p.x=? AND p.y=? AND EXISTS(SELECT 1 FROM active a WHERE a.z=? AND a.id=p.id)",params![key[0],key[1],z],|r|r.get::<_,i64>(0)).optional()?.is_some();
                        if !exists {
                            chosen = Some([key[0] as f64 / 16., key[1] as f64 / 16.]);
                            break;
                        }
                    }
                    let q = if fine.is_some() {
                        if center.iter().all(|v| v.is_finite()) {
                            center
                        } else {
                            [
                                (v[0][0] + v[1][0] + v[2][0]) / 3.,
                                (v[0][1] + v[1][1] + v[2][1]) / 3.,
                            ]
                        }
                    } else {
                        chosen
                            .with_context(|| format!("角度補修の候補を使い切りました。量子化精度を維持して停止: z={z} source={}/{} angle={value} triangle={v:?}", i.x, i.y))?
                    };
                    if fine.is_some() {
                        let id = nearest_fine(s, z, q)?
                            .context("細品質に角度補修候補がありません。品質を緩めず停止")?;
                        repair_insert(s,z,quant(s.point(id)?)?,&mut p,&mut angle_proofs,&mut error_proofs)?;
                    } else {
                        let px = q[0] / STEP - 0.5;
                        let py = q[1] / STEP - 0.5;
                        let x = px.floor() as i32;
                        let y = py.floor() as i32;
                        let mut h = 0.;
                        for (dx, dy, w) in [
                            (0, 0, (1. - px.fract()) * (1. - py.fract())),
                            (1, 0, px.fract() * (1. - py.fract())),
                            (0, 1, (1. - px.fract()) * py.fract()),
                            (1, 1, px.fract() * py.fract()),
                        ] {
                            if w <= 1e-15 {
                                continue;
                            }
                            let (xx, yy) = (x + dx, y + dy);
                            let value = if let Some(input) =
                                source.get(&(xx.div_euclid(256), yy.div_euclid(256)))
                            {
                                let raster = dem.read(input)?;
                                raster[(yy.rem_euclid(256) * 256 + xx.rem_euclid(256)) as usize]
                            } else {
                                f64::NAN
                            };
                            if value.is_finite() {
                                h += value * w;
                            } else {
                                // 欠損を0mとして追加しない。幾何補助点には現在の曲面値を引き継ぐ。
                                h += p.evaluate(q, &mut Work::default())? * w;
                            }
                        }
                        let mut point = quant([q[0], q[1], h])?;
                        // 過去の幾何補助点を復帰させる場合も、全国共通のXYZは動かさない。
                        if let Some(h) =
                            s.db.query_row(
                                "SELECT h FROM points WHERE x=? AND y=?",
                                params![point[0], point[1]],
                                |r| r.get::<_, i32>(0),
                            )
                            .optional()?
                        {
                            point[2] = h;
                        }
                        ensure!(
                            s.insert_cached(z, point, &mut p)?,
                            "角度補修が量子化限界に達しました: q={q:?} angle={value} triangle={v:?}"
                        );
                    }
                    changes += 1;
                    if changes % 32 == 0 {
                        progress(
                            root,
                            json!({"stage":"certify-local","z":z,"pass":pass,"input":n+1,"inputs":inputs.len(),"changes":changes,"virtual_checks":virtual_checks,"recertifications":recertifications,"recertification_changes":recertification_changes,"complete":false}),
                        )?;
                    }
                }
            }
            if local_checks && input_epoch == s.epoch(z)? { angle_proofs.mark(i); }
            if changes==before {unchanged_streak+=1;}else{unchanged_streak=0;prefetched.clear();}
            if n%32==0 || n+1==inputs.len() {save_certify_cursor(s,z,inputs.len(),pass,n+1,changes,min)?;}
            if n % 32 == 0 {
                progress(
                    root,
                    json!({"stage":"certify","z":z,"pass":pass,"done":n+1,"total":inputs.len(),"minimum_angle":min,"added":changes,"angle_verified":angle_proofs.count(),"error_verified":error_proofs.count(),"complete":false}),
                )?;
            }
        }
        if changes == 0 {
            let error = LEVELS.iter().find(|v| v.0 == z).unwrap().1;
            let (ok, max, valid) = if local_checks && error_proofs.count() < inputs.len() {
                (false, 0., 0)
            } else {
                audit_all_policy(s, z, error, inputs, dem, root, true, true)?
            };
            if !ok {
                if z == 13 {
                    repair_certify_fit(s, inputs, dem, root)?;
                    continue;
                }
                if local_checks && error_proofs.count() == inputs.len() { error_proofs.clear(); }
                s.db.execute("INSERT OR REPLACE INTO meta VALUES(?,?)",
                    params![&repair_key, s.epoch(z)?.to_string()])?;
                let mut repaired = false;
                let started = Instant::now();
                let mut restored = 0usize;
                for (batch, group) in inputs.chunks(prefetch_limit).enumerate() {
                    ensure!(!root.join("stop-requested").exists(), "誤差修復を停止。復帰済み点は保存されています");
                    let selected = group.iter().enumerate().filter(|(offset,_)|
                        !local_checks || !error_proofs.has(batch*prefetch_limit+offset)
                    ).map(|(_,input)|input.clone()).collect::<Vec<_>>();
                    let group = selected.as_slice();
                    let patches = s.patches(z, &group.iter().map(rect).collect::<Vec<_>>())?;
                    let rasters = group.iter().map(|input| dem.read(input)).collect::<Result<Vec<_>>>()?;
                    let audits = patches.par_iter().zip(group).zip(&rasters)
                        .map(|((patch, input), raster)| audit_patch(patch, input, raster))
                        .collect::<Result<Vec<_>>>()?;
                    for (((input, patch), raster), audit) in group.iter().zip(&patches).zip(&rasters).zip(audits) {
                        // 先行する復帰で変わった曲面は、現世代で評価し直してから点を選ぶ。
                        let audit = if patch.epoch != s.epoch(z)? {
                            audit_patch(&s.patch(z, rect(input))?, input, raster)?
                        } else {
                            audit
                        };
                        if audit.max <= error {
                            if local_checks { error_proofs.mark(input); }
                            continue;
                        }
                        let q = audit.candidate.context("誤差補修点なし")?;
                        let id = nearest_fine(s, z, [q[0] as f64 / 16., q[1] as f64 / 16.])?
                            .context("誤差補修の細品質点がありません")?;
                        let mut current = s.patch(z, rect(input))?;
                        repair_insert(s,z,quant(s.point(id)?)?,&mut current,&mut angle_proofs,&mut error_proofs)?;
                        restored += 1;
                        repaired = true;
                    }
                    progress(root, json!({"stage":"certify-repair","z":z,"pass":pass,
                        "done":((batch+1)*prefetch_limit).min(inputs.len()),"total":inputs.len(),
                        "angle_verified":angle_proofs.count(),"error_verified":error_proofs.count(),
                        "restored":restored,"workers":prefetch_limit,
                        "elapsed_seconds":started.elapsed().as_secs_f64(),"complete":false}))?;
                }
                ensure!(repaired || (local_checks && error_proofs.count()==inputs.len()), "角度補修後の誤差を解消できません");
                s.db.execute("DELETE FROM meta WHERE key=?", [&repair_key])?;
                continue;
            }
            s.db.execute("DELETE FROM meta WHERE key=?", [&repair_key])?;
            let page_workers=rayon::current_num_threads().clamp(1,10);
            for (batch,group) in inputs.chunks(page_workers).enumerate() {
                let patches=s.patches(z,&group.iter().map(rect).collect::<Vec<_>>())?;
                s.persist_patches(z,&group.iter().map(|i|format!("{}/{}",i.x,i.y)).collect::<Vec<_>>(),&patches)?;
                if batch%4==0 || (batch+1)*page_workers>=inputs.len() {
                    progress(root,json!({"stage":"certify-pages","z":z,"done":batch*page_workers+group.len(),"total":inputs.len(),"workers":page_workers,"complete":false}))?;
                }
            }
            done(
                s,
                &key,
                json!({"minimum_angle":min,"epoch":s.epoch(z)?,"maximum_directional_error":max,"valid":valid,"passes":pass,"virtual_checks":virtual_checks}),
            )?;
            s.db.execute("DELETE FROM meta WHERE key=?",[format!("certify-cursor-{z}-v1")])?;
            return Ok(());
        }
    }
}
#[derive(Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Debug)]
struct Key(i64, i64, i64);
fn restore(
    db: &Connection,
    key: Key,
    cache: &mut BTreeMap<Key, Vec<[i32; 3]>>,
    stack: &mut BTreeSet<Key>,
) -> Result<Vec<[i32; 3]>> {
    if let Some(p) = cache.get(&key) {
        return Ok(p.clone());
    }
    ensure!(
        stack.len() < 3 && stack.insert(key),
        "親参照が循環または深さ超過"
    );
    let (ox, oy, blob): (i32, i32, Vec<u8>) = db.query_row(
        "SELECT origin_x,origin_y,payload FROM tiles WHERE z=? AND x=? AND y=?",
        params![key.0, key.1, key.2],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let parents = {
        let mut st=db.prepare("SELECT parent_z,parent_x,parent_y FROM parents WHERE z=? AND x=? AND y=? ORDER BY slot")?;
        st.query_map(params![key.0, key.1, key.2], |r| {
            Ok(Key(r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    ensure!(parents.len() <= 4, "直接親の上限超過");
    let p = parents
        .iter()
        .map(|&k| restore(db, k, cache, stack))
        .collect::<Result<Vec<_>>>()?;
    let points = natural_codec::decode_global(&blob, [ox, oy], &p)?;
    ensure!(points.len() <= 262144, "復元点数上限超過");
    stack.remove(&key);
    cache.insert(key, points.clone());
    ensure!(cache.len() <= 16, "推移依存上限超過");
    Ok(points)
}
fn distribution(path: &Path, generation: &Value) -> Result<Connection> {
    let c = Connection::open(path)?;
    c.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=FULL;PRAGMA cache_size=-65536;
 CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID;
 CREATE TABLE IF NOT EXISTS tiles(z INTEGER,x INTEGER,y INTEGER,origin_x INTEGER,origin_y INTEGER,kind TEXT,points INTEGER,xyz_sha256 TEXT,payload BLOB,depth INTEGER,PRIMARY KEY(z,x,y)) WITHOUT ROWID;
 CREATE TABLE IF NOT EXISTS parents(z INTEGER,x INTEGER,y INTEGER,slot INTEGER,parent_z INTEGER,parent_x INTEGER,parent_y INTEGER,PRIMARY KEY(z,x,y,slot)) WITHOUT ROWID;")?;
    let id = generation.to_string();
    let previous = c
        .query_row(
            "SELECT value FROM metadata WHERE key='generation_id'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    ensure!(
        previous.as_ref().is_none_or(|v| v == &id),
        "異世代の配布DBです"
    );
    c.execute(
        "INSERT OR IGNORE INTO metadata VALUES('generation_id',?)",
        [id],
    )?;
    Ok(c)
}
fn point_digest(p: &[[i32; 3]]) -> String {
    let mut h = Sha256::new();
    for p in p {
        for v in p {
            h.update(v.to_le_bytes());
        }
    }
    h.finalize().iter().map(|v| format!("{v:02x}")).collect()
}
fn pack(s: &Store, job: &Value, inputs: &[Input], root: &Path, window: &[u32]) -> Result<()> {
    let out = root.join("terrain-shared.partial.sqlite3");
    let c = distribution(&out, &job["generation_id"])?;
    // 親点検索用は作業DBへ置く。配布DBに冗長な点一覧を入れない。
    s.db.execute_batch("CREATE TABLE IF NOT EXISTS packed_points(z INTEGER,x INTEGER,y INTEGER,id INTEGER,idx INTEGER,PRIMARY KEY(z,x,y,id)) WITHOUT ROWID;CREATE INDEX IF NOT EXISTS packed_id ON packed_points(z,id);")?;
    for &(z, _) in LEVELS.iter().rev() {
        s.cache_level(z)?;
        let mut sums=s.db.prepare("SELECT (x >> ?2),(y >> ?2),sum(valid) FROM audit WHERE z=?1 GROUP BY (x >> ?2),(y >> ?2)")?;
        let valid_counts = sums
            .query_map(params![z, 14 - z], |r| {
                Ok((
                    (r.get::<_, i64>(0)?, r.get::<_, i64>(1)?),
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
        let coords = inputs
            .iter()
            .map(|i| ((i.x as i64) >> (14 - z), (i.y as i64) >> (14 - z)))
            .collect::<BTreeSet<_>>();
        for (n, (x, y)) in coords.iter().copied().enumerate() {
            let valid = valid_counts.get(&(x, y)).copied().unwrap_or(0);
            if valid == 0 {
                continue;
            }
            let side = WORLD / 2f64.powi(z as i32);
            let b = [
                x as f64 * side,
                y as f64 * side,
                (x + 1) as f64 * side,
                (y + 1) as f64 * side,
            ];
            let patch = s.patch(z, b)?;
            let selected = patch
                .model
                .base
                .support_vertices(patch.local_rect(b))
                .into_iter()
                .flat_map(|i| {
                    std::iter::once(i).chain(patch.model.base.neighbors[i].iter().copied())
                })
                .collect::<BTreeSet<_>>();
            ensure!(
                selected.len() <= 262144,
                "タイル支持点数が上限超過。支持点を省略しません"
            );
            let ids = selected.iter().map(|&i| patch.ids[i]).collect::<Vec<_>>();
            let points = selected
                .iter()
                .map(|&i| quant(patch.point(i)))
                .collect::<Result<Vec<_>>>()?;
            let hash = point_digest(&points);
            let origin = quant([b[0], b[1], 0.])?;
            if let Some(old) = c
                .query_row(
                    "SELECT xyz_sha256 FROM tiles WHERE z=? AND x=? AND y=?",
                    params![z, x, y],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
            {
                ensure!(old == hash, "既存パックの点集合が変化しました");
            } else {
                let local = points
                    .iter()
                    .map(|p| [p[0] - origin[0], p[1] - origin[1], p[2]])
                    .collect::<Vec<_>>();
                let mut blob = natural_codec::independent(&local)?;
                let mut kind = "NQC1";
                let mut parent_keys = Vec::new();
                let mut depth = 0;
                if ![6, 9, 12].contains(&z) {
                    let mut counts = BTreeMap::<Key, usize>::new();
                    if c.query_row(
                        "SELECT 1 FROM tiles WHERE z=? AND x=? AND y=?",
                        params![z - 1, x / 2, y / 2],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()?
                    .is_some()
                    {
                        counts.insert(Key(z - 1, x / 2, y / 2), 0);
                    }
                    let mut lookup=s.db.prepare("SELECT x,y FROM packed_points WHERE z=? AND id=? AND EXISTS(SELECT 1 FROM points p WHERE p.id=packed_points.id AND p.guard=0) ORDER BY y,x")?;
                    for &id in &ids {
                        for row in lookup.query_map(params![z - 1, id], |r| {
                            Ok(Key(z - 1, r.get(0)?, r.get(1)?))
                        })? {
                            *counts.entry(row?).or_default() += 1;
                        }
                    }
                    let mut ranked = counts.into_iter().collect::<Vec<_>>();
                    ranked.sort_by_key(|(k, count)| (std::cmp::Reverse(*count), *k));
                    let candidates = ranked.into_iter().take(4).map(|v| v.0).collect::<Vec<_>>();
                    let mut cache = BTreeMap::new();
                    let mut parents = Vec::new();
                    let mut parent_depth = 0;
                    for &k in &candidates {
                        let mut one = BTreeMap::new();
                        let decoded = restore(&c, k, &mut one, &mut BTreeSet::new())?;
                        cache.extend(one);
                        if cache.len() >= 16 {
                            parents.clear();
                            break;
                        }
                        parents.push(decoded);
                        let d: i64 = c.query_row(
                            "SELECT depth FROM tiles WHERE z=? AND x=? AND y=?",
                            params![k.0, k.1, k.2],
                            |r| r.get(0),
                        )?;
                        parent_depth = parent_depth.max(d + 1);
                    }
                    if !parents.is_empty() && cache.len() < 16 && parent_depth < 3 {
                        let indices = parents
                            .iter()
                            .map(|p| {
                                p.iter()
                                    .enumerate()
                                    .map(|(i, p)| ((p[0], p[1]), i))
                                    .collect::<HashMap<_, _>>()
                            })
                            .collect::<Vec<_>>();
                        let refs = points
                            .iter()
                            .map(|p| {
                                indices
                                    .iter()
                                    .enumerate()
                                    .find_map(|(slot, m)| m.get(&(p[0], p[1])).map(|&i| (slot, i)))
                            })
                            .collect::<Vec<_>>();
                        let delta = natural_codec::delta(&local, &refs, &parents)?;
                        ensure!(
                            natural_codec::decode_global(&delta, [origin[0], origin[1]], &parents)?
                                == points,
                            "差分復号不一致"
                        );
                        if delta.len() + candidates.len() * 24 < blob.len() {
                            blob = delta;
                            kind = "NDC1";
                            parent_keys = candidates;
                            depth = parent_depth;
                        }
                    }
                }
                if kind == "NQC1" {
                    ensure!(
                        natural_codec::decode_global(&blob, [origin[0], origin[1]], &[])? == points,
                        "独立復号不一致"
                    );
                }
                let tx = c.unchecked_transaction()?;
                tx.execute(
                    "INSERT INTO tiles VALUES(?,?,?,?,?,?,?,?,?,?)",
                    params![
                        z,
                        x,
                        y,
                        origin[0],
                        origin[1],
                        kind,
                        points.len(),
                        hash,
                        blob,
                        depth
                    ],
                )?;
                for (slot, k) in parent_keys.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO parents VALUES(?,?,?,?,?,?,?)",
                        params![z, x, y, slot, k.0, k.1, k.2],
                    )?;
                }
                tx.commit()?;
            }
            // 配布DBのcommit後に停止しても、再開時に同じハッシュから検索表を再構築する。
            let tx = s.db.unchecked_transaction()?;
            {
                let mut insert =
                    tx.prepare_cached("INSERT OR REPLACE INTO packed_points VALUES(?,?,?,?,?)")?;
                for (idx, id) in ids.iter().enumerate() {
                    insert.execute(params![z, x, y, id, idx])?;
                }
            }
            tx.commit()?;
            if n % 8 == 0 {
                progress(
                    root,
                    json!({"stage":"pack","z":z,"done":n+1,"total":coords.len(),"complete":false}),
                )?;
            }
        }
    }
    for (k, v) in [
        ("format", json!("national-shared-xyz-v1")),
        ("algorithm", json!("shared-certified-local-v1")),
        ("crs", json!("EPSG:3857")),
        ("world_width_m", json!(WORLD)),
        ("common_origin_epsg3857", json!([-WORLD / 2., WORLD / 2.])),
        ("axes", json!(["east", "south", "up"])),
        (
            "quantization",
            json!({"xy_step_m":0.0625,"z_step_m":0.125,"rounding":"floor(v/step+0.5)"}),
        ),
        ("levels", job["settings"]["levels"].clone()),
        ("tile_scheme", json!("XYZ")),
        ("settings", job["settings"].clone()),
        ("source_window_z14", json!(window)),
        (
            "error_domain",
            json!("valid original DEM pixel centers; base Sibson; display C1 excluded"),
        ),
        (
            "scope",
            json!(if window.is_empty() {
                "national"
            } else {
                "window-test"
            }),
        ),
        ("complete", json!(false)),
        ("gpu_auxiliary_in_distribution", json!(false)),
    ] {
        c.execute(
            "INSERT OR REPLACE INTO metadata VALUES(?,?)",
            params![k, v.to_string()],
        )?;
    }
    c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    Ok(())
}
fn distribution_path(root: &Path) -> PathBuf {
    let partial = root.join("terrain-shared.partial.sqlite3");
    if partial.exists() {
        partial
    } else {
        root.join("terrain-shared.sqlite3")
    }
}
fn finalize(root: &Path, window: &[u32], report: &Value) -> Result<()> {
    let partial = root.join("terrain-shared.partial.sqlite3");
    let target = root.join("terrain-shared.sqlite3");
    ensure!(!target.exists(), "完成SQLiteを上書きしません");
    {
        let c = Connection::open(&partial)?;
        let tx = c.unchecked_transaction()?;
        for (k, v) in [
            ("complete", json!(true)),
            ("national_complete", json!(window.is_empty())),
            ("verification", report.clone()),
        ] {
            tx.execute(
                "INSERT OR REPLACE INTO metadata VALUES(?,?)",
                params![k, v.to_string()],
            )?;
        }
        tx.commit()?;
        c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);PRAGMA journal_mode=DELETE")?;
    }
    for attempt in 0..40 {
        match fs::rename(&partial, &target) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 39 && matches!(e.raw_os_error(), Some(5 | 32 | 33)) => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}
fn verify_distribution(root: &Path) -> Result<Value> {
    let c = Connection::open_with_flags(
        distribution_path(root),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    ensure!(
        c.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))? == "ok",
        "配布DBが破損"
    );
    let mut st = c.prepare("SELECT z,x,y,xyz_sha256 FROM tiles ORDER BY z,y,x")?;
    let mut rows = st.query([])?;
    let mut count = 0;
    let mut max_deps = 0;
    while let Some(r) = rows.next()? {
        let k = Key(r.get(0)?, r.get(1)?, r.get(2)?);
        let expected: String = r.get(3)?;
        let mut cache = BTreeMap::new();
        let p = restore(&c, k, &mut cache, &mut BTreeSet::new())?;
        ensure!(
            point_digest(&p) == expected,
            "整数復号ハッシュ不一致: {k:?}"
        );
        max_deps = max_deps.max(cache.len());
        count += 1;
    }
    Ok(json!({"tiles":count,"maximum_dependency_tiles":max_deps,"integer_roundtrip":true}))
}
fn audit_saved(root: &Path, inputs: &[Input], dem: &mut Dem) -> Result<Value> {
    let c = Connection::open_with_flags(
        distribution_path(root),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let work = Connection::open(root.join("verified.sqlite3"))?;
    work.execute_batch("CREATE TABLE IF NOT EXISTS audits(z INTEGER,x INTEGER,y INTEGER,hash TEXT,valid INTEGER,up REAL,down REAL,PRIMARY KEY(z,x,y)) WITHOUT ROWID")?;
    let mut tested = 0u64;
    let mut maximum_up = 0f64;
    let mut maximum_down = 0f64;
    for &(z, error) in LEVELS.iter().rev() {
        let mut curves: BTreeMap<Key, crate::natural_surface::Surface> = BTreeMap::new();
        let mut curve_order = VecDeque::new();
        let mut curve_points = 0usize;
        for (n, i) in inputs.iter().enumerate() {
            let key = Key(z, (i.x as i64) >> (14 - z), (i.y as i64) >> (14 - z));
            let checksum = c
                .query_row(
                    "SELECT xyz_sha256 FROM tiles WHERE z=? AND x=? AND y=?",
                    params![key.0, key.1, key.2],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            let Some(checksum) = checksum else {
                ensure!(
                    dem.read(i)?.iter().all(|v| !v.is_finite()),
                    "有効画素を含む保存タイルがありません"
                );
                continue;
            };
            let audit_key = format!("direct-Surface-certified-angle-v3:{checksum}");
            if let Some(old) = work
                .query_row(
                    "SELECT hash FROM audits WHERE z=? AND x=? AND y=?",
                    params![z, i.x, i.y],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
            {
                if old == audit_key {
                    continue;
                }
            }
            if !curves.contains_key(&key) {
                let p = restore(&c, key, &mut BTreeMap::new(), &mut BTreeSet::new())?;
                let side = WORLD / 2f64.powi(key.0 as i32);
                let points = p
                    .into_iter()
                    .map(|p| {
                        [
                            p[0] as f64 / 16. - key.1 as f64 * side,
                            p[1] as f64 / 16. - key.2 as f64 * side,
                            p[2] as f64 / 8.,
                        ]
                    })
                    .collect();
                let model = crate::natural_surface::Surface::certified(points)?;
                while curve_points + model.points.len() > 500_000 && !curves.is_empty() {
                    if let Some(old) = curve_order.pop_front() {
                        if let Some(m) = curves.remove(&old) {
                            curve_points -= m.points.len();
                        }
                    }
                }
                curve_points += model.points.len();
                curves.insert(key, model);
                curve_order.push_back(key);
            }
            let model = &curves[&key];
            let side = WORLD / 2f64.powi(z as i32);
            let b = rect(i);
            let local = [
                b[0] - key.1 as f64 * side,
                b[1] - key.2 as f64 * side,
                b[2] - key.1 as f64 * side,
                b[3] - key.2 as f64 * side,
            ];
            ensure!(
                minimum_angle_surface(model, local) >= 10. - 1e-7,
                "保存後の最小角監査が未達 z{z}"
            );
            let raster = dem.read(i)?;
            let blocks = (0..16)
                .into_par_iter()
                .map(|block| -> Result<(f64, f64, u64)> {
                    let mut w = crate::natural_surface::Workspace::default();
                    let mut start = 0;
                    let mut up = 0f64;
                    let mut down = 0f64;
                    let mut valid = 0;
                    for index in block * 4096..(block + 1) * 4096 {
                        let h = raster[index];
                        if !h.is_finite() {
                            continue;
                        }
                        let q = [
                            (i.x as f64 * 256. + (index % 256) as f64 + 0.5) * STEP,
                            (i.y as f64 * 256. + (index / 256) as f64 + 0.5) * STEP,
                        ];
                        let delta = model.evaluate_reusing(
                            [q[0] - key.1 as f64 * side, q[1] - key.2 as f64 * side],
                            &mut start,
                            &mut w,
                        )? - h;
                        up = up.max(delta);
                        down = down.max(-delta);
                        valid += 1;
                    }
                    Ok((up, down, valid))
                })
                .collect::<Result<Vec<_>>>()?;
            let up = blocks.iter().map(|v| v.0).fold(0f64, f64::max);
            let down = blocks.iter().map(|v| v.1).fold(0f64, f64::max);
            let valid: u64 = blocks.iter().map(|v| v.2).sum();
            ensure!(
                up <= error / 4. + 1e-7 && down <= error + 1e-7,
                "保存後の全画素監査未達 z{z} {}/{} up={up} down={down}",
                i.x,
                i.y
            );
            work.execute(
                "INSERT OR REPLACE INTO audits VALUES(?,?,?,?,?,?,?)",
                params![z, i.x, i.y, audit_key, valid, up, down],
            )?;
            tested += valid;
            maximum_up = maximum_up.max(up);
            maximum_down = maximum_down.max(down);
            if n % 8 == 0 {
                progress(
                    root,
                    json!({"stage":"verify-saved-dem","z":z,"done":n+1,"total":inputs.len(),"pixels_this_run":tested,"complete":false}),
                )?;
            }
        }
    }
    let(count,total,up,down):(i64,i64,f64,f64)=work.query_row("SELECT count(*),coalesce(sum(valid),0),coalesce(max(up),0),coalesce(max(down),0) FROM audits",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    Ok(
        json!({"audit_blocks":count,"total_audited_pixels":total,"maximum_up":up,"maximum_down":down,"newly_tested_pixels":tested,"maximum_up_this_run":maximum_up,"maximum_down_this_run":maximum_down,"evaluator":"direct-Surface / decoded-distribution-XYZ","all_valid_pixels":true}),
    )
}
fn verify_seams(root: &Path) -> Result<Value> {
    let c = Connection::open_with_flags(
        distribution_path(root),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut st = c.prepare("SELECT z,x,y FROM tiles ORDER BY z,y,x")?;
    let keys = st
        .query_map([], |r| Ok(Key(r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    let mut maximum_h = 0f64;
    let mut maximum_g = 0f64;
    let mut checked = 0;
    for &key in &keys {
        for axis in [0, 1] {
            let other = Key(
                key.0,
                key.1 + if axis == 0 { 1 } else { 0 },
                key.2 + if axis == 1 { 1 } else { 0 },
            );
            if !keys.contains(&other) {
                continue;
            }
            let side = WORLD / 2f64.powi(key.0 as i32);
            let origin = [key.1 as f64 * side, key.2 as f64 * side];
            let models = [key, other].map(|k| -> Result<_> {
                let points = restore(&c, k, &mut BTreeMap::new(), &mut BTreeSet::new())?
                    .iter()
                    .map(|p| {
                        [
                            p[0] as f64 / 16. - origin[0],
                            p[1] as f64 / 16. - origin[1],
                            p[2] as f64 / 8.,
                        ]
                    })
                    .collect();
                crate::natural_regional_surface::Fast::certified(points)
            });
            let [a, b] = models;
            let (a, b) = (a?, b?);
            let (ag, bg) = (
                crate::natural_local_gradient::local_gradients(&a),
                crate::natural_local_gradient::local_gradients(&b),
            );
            let mut aw = Work::default();
            let mut bw = Work::default();
            let side = WORLD / 2f64.powi(key.0 as i32);
            for n in 0..=16 {
                let u = n as f64 / 16.;
                let q = if axis == 0 {
                    [(key.1 + 1) as f64 * side, (key.2 as f64 + u) * side]
                } else {
                    [(key.1 as f64 + u) * side, (key.2 + 1) as f64 * side]
                };
                let q = [q[0] - origin[0], q[1] - origin[1]];
                maximum_h =
                    maximum_h.max((a.evaluate(q, &mut aw)? - b.evaluate(q, &mut bw)?).abs());
                let mut grad = [[0.; 2]; 2];
                for (k, (m, g, w)) in [(&a, &ag, &mut aw), (&b, &bg, &mut bw)]
                    .into_iter()
                    .enumerate()
                {
                    for (i, weight) in m.basis(q, w)? {
                        for axis in 0..2 {
                            grad[k][axis] += weight * g[i][axis];
                        }
                    }
                }
                maximum_g = maximum_g
                    .max((grad[0][0] - grad[1][0]).abs())
                    .max((grad[0][1] - grad[1][1]).abs());
                checked += 1;
            }
        }
    }
    ensure!(
        maximum_h < 1e-6 && maximum_g < 1e-6,
        "同品質の保存境界が不一致: height={maximum_h} gradient={maximum_g}"
    );
    Ok(
        json!({"samples":checked,"maximum_height_difference":maximum_h,"maximum_gradient_difference":maximum_g,"scope":"same-quality / decoded-XYZ"}),
    )
}
/// 保存済み全国キューの先頭だけを複製環境で測る。元キューは別名で保持する。
/// 同じ全国スナップショットの複製で、監査だけを比較する。配布不可。
pub fn benchmark_fit_cache(root:&Path,snapshot:&Path,output:&Path,batches:usize,offset:usize)->Result<()> {
    ensure!(batches>0,"バッチ数が必要です");fs::create_dir(output)?;
    for name in ["job.json","settings.json","inputs.sqlite3"]{fs::copy(root.join(name),output.join(name))?;}
    fs::copy(snapshot,output.join("work.sqlite3"))?;
    let(s,job,inputs)=initialize_internal(output,131072,&[],true)?;
    s.db.execute("INSERT OR REPLACE INTO meta VALUES('benchmark_only','true')",[])?;
    s.db.execute("INSERT OR REPLACE INTO meta VALUES('benchmark_start_offset',?)",[offset.to_string()])?;
    s.db.execute("DELETE FROM stages WHERE name='fit-13'",[])?;
    let workers=std::thread::available_parallelism().map(|n|(n.get()/2).max(1)).unwrap_or(1);let _=rayon::ThreadPoolBuilder::new().num_threads(workers).build_global();
    let stride=inputs.len().div_ceil(workers);ensure!(offset+batches<=stride,"試験範囲外");
    let scope=(offset..offset+batches).flat_map(|k|(0..workers).map(move|w|w*stride+k)).filter(|&k|k<inputs.len()).map(|k|inputs[k].clone()).collect::<Vec<_>>();
    let mut dem=Dem{root:PathBuf::from(job["identity"]["cache"].as_str().context("cache")?),cache:HashMap::new(),order:VecDeque::new()};
    let mut stable=false;
    for _ in 0..4 {fit_finest_budget(&s,&inputs,&mut dem,output,workers,Some(batches))?;
        let report:Value=serde_json::from_slice(&fs::read(output.join("benchmark-fit.json"))?)?;
        if report["added_points"].as_u64()==Some(0)&&audit_all_policy(&s,13,25.,&scope,&mut dem,output,false,false)?.0 {stable=true;break;}}
    ensure!(stable,"試験範囲が収束しません");let epoch=s.epoch(13)?;
    fit_finest_budget(&s,&inputs,&mut dem,output,workers,Some(batches))?;
    let cached:Value=serde_json::from_slice(&fs::read(output.join("benchmark-fit.json"))?)?;
    s.db.execute("INSERT INTO meta VALUES('benchmark_disable_fit_verified','true')",[])?;
    fit_finest_budget(&s,&inputs,&mut dem,output,workers,Some(batches))?;
    let baseline:Value=serde_json::from_slice(&fs::read(output.join("benchmark-fit.json"))?)?;
    ensure!(s.epoch(13)?==epoch&&cached["added_points"].as_u64()==Some(0)&&baseline["added_points"].as_u64()==Some(0),"比較中に点集合が変化しました");
    let audit=audit_all_policy(&s,13,25.,&scope,&mut dem,output,false,false)?;ensure!(audit.0,"比較後の全画素監査未達");
    let report=json!({"scope":"stable limited real fitting / same epoch / warm PNG cache in both","inputs":scope.len(),"cached":cached,"uncached":baseline,"same_point_epoch":true,"maximum_error":audit.1,"valid_pixels":audit.2,"national_complete":false,"distributable":false});
    fs::write(output.join("benchmark-fit-cache.json"),serde_json::to_vec_pretty(&report)?)?;println!("{}",serde_json::to_string_pretty(&report)?);Ok(())
}
pub fn benchmark_audit(root:&Path,snapshot:&Path,output:&Path,offset:usize,count:usize)->Result<()> {
    ensure!(count>0,"監査入力数が必要です");fs::create_dir(output)?;
    for name in ["job.json","settings.json","inputs.sqlite3"]{fs::copy(root.join(name),output.join(name))?;}
    fs::copy(snapshot,output.join("work.sqlite3"))?;
    let (s,job,all)=initialize_internal(output,131072,&[],true)?;
    ensure!(offset<all.len(),"監査開始位置が範囲外です");let inputs=&all[offset..(offset+count).min(all.len())];
    s.db.execute("INSERT OR REPLACE INTO meta VALUES('benchmark_only','true')",[])?;
    s.db.execute("INSERT OR REPLACE INTO meta VALUES('benchmark_audit_only','true')",[])?;
    let workers=std::thread::available_parallelism().map(|n|(n.get()/2).max(1)).unwrap_or(1);
    let _=rayon::ThreadPoolBuilder::new().num_threads(workers).build_global();s.cache_level(13)?;
    let mut outcomes=Vec::new();
    for parallel in [false,true] {
        // ベンチマーク複製だけを変更する。両方式とも同じ世代・未監査状態から開始する。
        ensure!(s.db.query_row("SELECT value FROM meta WHERE key='benchmark_audit_only'",[],|r|r.get::<_,String>(0))?=="true","試験用DBではありません");
        s.db.execute("DELETE FROM audit WHERE z=13",[])?;
        let mut dem=Dem{root:PathBuf::from(job["identity"]["cache"].as_str().context("cache")?),cache:HashMap::new(),order:VecDeque::new()};
        let started=Instant::now();let result=if parallel {audit_all_policy(&s,13,25.,inputs,&mut dem,output,true,false)?}else{audit_all_serial_reference(&s,13,25.,inputs,&mut dem,output,true)?};
        outcomes.push((started.elapsed().as_secs_f64(),result));
    }
    ensure!(outcomes[0].1==outcomes[1].1,"逐次監査と並列監査が一致しません");
    let report=json!({"scope":"limited real national audit / clone only","inputs":inputs.len(),"offset":offset,"serial_seconds":outcomes[0].0,"parallel_seconds":outcomes[1].0,"results_equal":true,"valid_pixels":outcomes[1].1.2,"maximum_directional_error":outcomes[1].1.1,"within_error":outcomes[1].1.0,"workers":workers,"distributable":false,"national_complete":false,"resources":crate::natural_shared_resources::report()});
    fs::write(output.join("benchmark-audit.json"),serde_json::to_vec_pretty(&report)?)?;println!("{}",serde_json::to_string_pretty(&report)?);Ok(())
}
pub fn benchmark_prune(root:&Path,snapshot:&Path,output:&Path,count:usize,skip:usize,zoom:u8)->Result<()> {
    let z = i64::from(zoom);
    let error = LEVELS.iter().find(|(level, _)| *level == z).context("unsupported benchmark zoom")?.1;
    let queue_name = format!("prune-queue-{z}");
    ensure!(count>0,"候補数が必要です");
    fs::create_dir(output)?;
    for name in ["job.json","settings.json","inputs.sqlite3"] {fs::copy(root.join(name),output.join(name))?;}
    fs::copy(snapshot,output.join("work.sqlite3"))?;
    let (s,job,inputs)=initialize_internal(output,131072,&[],true)?;
    s.db.execute("INSERT OR REPLACE INTO meta VALUES('benchmark_only','true')",[])?;
    s.db.execute("INSERT OR REPLACE INTO meta VALUES('benchmark_prune_only','true')",[])?;
    s.db.execute_batch("ALTER TABLE prune_queue RENAME TO benchmark_original_prune_queue; CREATE TABLE prune_queue(z INTEGER,stripe INTEGER,sequence INTEGER,id INTEGER,PRIMARY KEY(z,stripe,sequence)) WITHOUT ROWID")?;
    let queue=state(&s,&queue_name)?.context("全国削減キューがありません")?;
    let workers=queue["workers"].as_u64().context("workers")? as usize;
    for stripe in 0..workers {
        s.db.execute("INSERT INTO prune_queue SELECT * FROM benchmark_original_prune_queue WHERE z=? AND stripe=? ORDER BY sequence LIMIT ? OFFSET ?",params![z,stripe,count.div_ceil(workers),skip])?;
    }
    if std::env::var_os("TERRAIN_BENCH_PRUNE_ORIGINAL_ORDER").is_some() {
        s.db.execute_batch("ALTER TABLE prune_queue RENAME TO benchmark_selected_prune_queue; CREATE TABLE prune_queue(z INTEGER,stripe INTEGER,sequence INTEGER,id INTEGER,PRIMARY KEY(z,stripe,sequence)) WITHOUT ROWID; INSERT INTO prune_queue SELECT q.z,q.stripe,row_number() OVER(PARTITION BY q.z,q.stripe ORDER BY p.y,p.x,q.id),q.id FROM benchmark_selected_prune_queue q JOIN points p ON p.id=q.id")?;
    }
    let n:i64=s.db.query_row("SELECT count(*) FROM prune_queue",[],|r|r.get(0))?;
    s.db.execute("UPDATE stages SET details=? WHERE name=?",[json!({"workers":workers,"candidates":n}).to_string(),queue_name])?;
    let _=rayon::ThreadPoolBuilder::new().num_threads(configured_threads(workers)).build_global();
    let mut dem=Dem{root:PathBuf::from(job["identity"]["cache"].as_str().context("cache")?),cache:HashMap::new(),order:VecDeque::new()};
    parallel_prune::run(&s,z,13,error,&inputs,&mut dem,output,workers)
}
/// 全国の入力台帳と点集合を複製した作業領域で、離れた入力の適合速度を実測する。
pub fn benchmark(
    root: &Path,
    snapshot: &Path,
    output: &Path,
    batches: usize,
    workers: usize,
    start_batch: usize,
    serial_prepare: bool,
) -> Result<()> {
    ensure!(
        batches > 0 && workers > 0,
        "バッチ数・並列数は正の値が必要です"
    );
    fs::create_dir(output).context("新しいベンチマーク出力が必要です")?;
    for name in ["job.json", "inputs.sqlite3", "settings.json"] {
        let p = root.join(name);
        if p.exists() {
            fs::copy(p, output.join(name))?;
        }
    }
    fs::copy(snapshot, output.join("work.sqlite3"))?;
    let (s, job, inputs) = initialize_internal(output, 131072, &[], true)?;
    s.db.execute(
        "INSERT OR REPLACE INTO meta VALUES('benchmark_only','true')",
        [],
    )?;
    let mut dem = Dem {
        root: PathBuf::from(job["identity"]["cache"].as_str().context("cache")?),
        cache: HashMap::new(),
        order: VecDeque::new(),
    };
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build_global();
    timed(output, "seed-global", || {
        seed_global(&s, &inputs, &dem, output)
    })?;
    ensure!(
        start_batch < inputs.len().div_ceil(workers),
        "開始バッチが範囲外です"
    );
    s.db.execute(
        "INSERT OR REPLACE INTO meta VALUES('benchmark_start_offset',?)",
        [start_batch.to_string()],
    )?;
    if serial_prepare {
        s.db.execute(
            "INSERT OR REPLACE INTO meta VALUES('benchmark_serial_fit_prepare','true')",
            [],
        )?;
    }
    let last: i64 =
        s.db.query_row("SELECT max(id) FROM points", [], |r| r.get(0))?;
    s.db.execute(
        "INSERT OR REPLACE INTO meta VALUES('benchmark_prune_min_id',?)",
        [last.to_string()],
    )?;
    fit_finest_budget(&s, &inputs, &mut dem, output, workers, Some(batches))
}
/// 適合済みの全国複製から、測定した入力だけを対象に全品質の処理時間を測る。
pub fn benchmark_levels(root: &Path) -> Result<()> {
    let report: Value = serde_json::from_slice(&fs::read(root.join("benchmark-fit.json"))?)?;
    ensure!(
        report["scope"] == "fit-only / national snapshot",
        "適合ベンチマークの出力が必要です"
    );
    let (s, job, inputs) = initialize_internal(root, 131072, &[], true)?;
    s.db.execute(
        "INSERT OR REPLACE INTO meta VALUES('benchmark_only','true')",
        [],
    )?;
    let workers = report["workers"].as_u64().context("並列数がありません")? as usize;
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build_global();
    let n = report["processed_inputs"]
        .as_u64()
        .context("入力数がありません")? as usize;
    let batches = n.div_ceil(workers);
    let stride = inputs.len().div_ceil(workers);
    let input_offset = report["input_offset"].as_u64().unwrap_or(0) as usize;
    let scope = (input_offset..input_offset + batches)
        .flat_map(|k| (0..workers).map(move |w| w * stride + k))
        .filter(|&k| k < inputs.len())
        .map(|k| inputs[k].clone())
        .collect::<Vec<_>>();
    let min_id = if let Some(v) =
        s.db.query_row(
            "SELECT value FROM meta WHERE key='benchmark_prune_min_id'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        v.parse::<i64>()?
    } else {
        let (count, max): (i64, i64) =
            s.db.query_row("SELECT count(*),max(id) FROM points", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        ensure!(
            count == max && count == s.epoch(13)?,
            "初期点IDを確定できないため測定を中止します"
        );
        let v = state(&s, "seed-global-v1")?.context("初期点がありません")?["epoch"]
            .as_i64()
            .context("初期点世代がありません")?;
        s.db.execute(
            "INSERT INTO meta VALUES('benchmark_prune_min_id',?)",
            [v.to_string()],
        )?;
        v
    };
    let mut dem = Dem {
        root: PathBuf::from(job["identity"]["cache"].as_str().context("cache")?),
        cache: HashMap::new(),
        order: VecDeque::new(),
    };
    s.cache_level(13)?;
    if state(&s, "benchmark-fitted")?.is_none() {
        for attempt in 0..8 {
            if audit_all(&s, 13, 25., &scope, &mut dem, root, false)?.0 {
                break;
            }
            fit_finest_budget(&s, &inputs, &mut dem, root, workers, Some(batches))?;
            ensure!(attempt < 7, "測定範囲の適合が収束しません");
        }
        let (ok, max, valid) = audit_all(&s, 13, 25., &scope, &mut dem, root, true)?;
        ensure!(ok, "測定範囲の適合が未達です");
        done(
            &s,
            "fit-13",
            json!({"scope":"benchmark-only","epoch":s.epoch(13)?,"max":max,"valid":valid}),
        )?;
        done(&s, "benchmark-fitted", json!({"inputs":scope.len()}))?;
    }
    let started = Instant::now();
    let mut levels = Vec::new();
    for (k, &(z, e)) in LEVELS.iter().enumerate() {
        let fine = if k == 0 { 13 } else { LEVELS[k - 1].0 };
        let phase = Instant::now();
        let prune_reused = state(&s, &format!("prune-{z}"))?.is_some();
        timed(root, &format!("benchmark-prune-{z}"), || {
            parallel_prune::run(&s, z, fine, e, &scope, &mut dem, root, workers)
        })?;
        let certify_reused = state(&s, &format!("certify-{z}"))?
            .is_some_and(|v| v["epoch"].as_i64() == Some(s.epoch(z).unwrap_or(-1)));
        timed(root, &format!("benchmark-certify-{z}"), || {
            certify(&s, z, &scope, &mut dem, root)
        })?;
        if z < 13 {
            let missing:i64=s.db.query_row("SELECT count(*) FROM active a WHERE a.z=? AND NOT EXISTS(SELECT 1 FROM active b WHERE b.z=? AND b.id=a.id)",params![z,z+1],|r|r.get(0))?;
            ensure!(missing == 0, "粗品質の点が細品質に含まれていません: z{z}");
        }
        let points: i64 = s.db.query_row(
            "SELECT count(*) FROM active WHERE z=? AND id>?",
            params![z, min_id],
            |r| r.get(0),
        )?;
        levels.push(
            json!({"z":z,"seconds_this_run":phase.elapsed().as_secs_f64(),"scope_points":points,"prune_reused":prune_reused,"certify_reused":certify_reused}),
        );
        fs::write(
            root.join("benchmark-levels.json"),
            serde_json::to_vec_pretty(
                &json!({"scope":"benchmark-only / source inputs subset; outside context fixed","inputs":scope.len(),"levels":levels,"complete":false,"national_complete":false,"distributable":false}),
            )?,
        )?;
    }
    let summary = json!({"scope":"benchmark-only / source inputs subset; outside context fixed","inputs":scope.len(),"seconds_this_run":started.elapsed().as_secs_f64(),"levels":levels,"complete":true,"national_complete":false,"distributable":false});
    fs::write(
        root.join("benchmark-levels.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    progress(
        root,
        json!({"stage":"benchmark-complete","complete":false,"benchmark":summary}),
    )
}
/// 読取用スナップショットを複製し、同じ挿入列で照合の有無だけを比較する。
pub fn profile(root: &Path, snapshot: &Path, count: usize) -> Result<()> {
    let workers = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).max(1))
        .unwrap_or(1);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build_global();
    let job: Value = serde_json::from_slice(&fs::read(root.join("job.json"))?)?;
    let c = Connection::open_with_flags(
        root.join("inputs.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let input = c.query_row(
        "SELECT x,y,sha256 FROM inputs WHERE state='cached' ORDER BY y,x LIMIT 1 OFFSET 123",
        [],
        |r| {
            Ok(Input {
                x: r.get(0)?,
                y: r.get(1)?,
                sha: r.get(2)?,
            })
        },
    )?;
    let mut dem = Dem {
        root: PathBuf::from(job["identity"]["cache"].as_str().context("cache")?),
        cache: HashMap::new(),
        order: VecDeque::new(),
    };
    let raster = dem.read(&input)?;
    let bounds = rect(&input);
    let bounds = [
        bounds[0] - 2. * STEP,
        bounds[1] - 2. * STEP,
        bounds[2] + 2. * STEP,
        bounds[3] + 2. * STEP,
    ];
    let mut sequence = Vec::new();
    let mut results = Vec::new();
    for cached in [false, true] {
        let path = root.join(format!(
            "profile-{}.sqlite3",
            if cached { "cached" } else { "baseline" }
        ));
        ensure!(
            !path.exists(),
            "実測用ファイルが既にあります: {}",
            path.display()
        );
        fs::copy(snapshot, &path)?;
        let store = Store::open(&path, 131072)?;
        if cached {
            store.cache_level(13)?;
        }
        let start = Instant::now();
        let mut patch = store.patch(13, bounds)?;
        let initial = start.elapsed().as_secs_f64();
        let mut audit_seconds = 0.;
        let mut update_seconds = 0.;
        let mut inserted = 0;
        for k in 0..count {
            let start = Instant::now();
            let candidate = if cached {
                sequence.get(k).copied()
            } else {
                let a = audit_patch(&patch, &input, &raster)?;
                if a.max <= 25. + 1e-8 {
                    None
                } else {
                    a.candidate
                }
            };
            audit_seconds += start.elapsed().as_secs_f64();
            let Some(q) = candidate else { break };
            let start = Instant::now();
            let changed = if cached {
                store.insert_cached(13, q, &mut patch)?
            } else {
                let changed = store.insert(13, q, false)?;
                patch = store.patch(13, bounds)?;
                changed
            };
            update_seconds += start.elapsed().as_secs_f64();
            if !changed {
                break;
            }
            if !cached {
                sequence.push(q);
            }
            inserted += 1;
        }
        let full = store.patch(13, bounds)?;
        let mut a = Work::default();
        let mut b = Work::default();
        let mut difference = 0f64;
        for y in 0..33 {
            for x in 0..33 {
                let q = [
                    bounds[0] + (bounds[2] - bounds[0]) * x as f64 / 32.,
                    bounds[1] + (bounds[3] - bounds[1]) * y as f64 / 32.,
                ];
                difference =
                    difference.max((patch.evaluate(q, &mut a)? - full.evaluate(q, &mut b)?).abs());
            }
        }
        ensure!(
            difference < 1e-6,
            "キャッシュ更新の照合不一致: {difference}"
        );
        results.push(json!({"cached":cached,"input":[input.x,input.y],"inserted":inserted,"initial_patch_seconds":initial,"audit_seconds":audit_seconds,"update_seconds":update_seconds,"maximum_difference":difference,"local_points":patch.ids.len()}));
        eprintln!("{}", results.last().unwrap());
    }
    fs::write(
        root.join("profile-comparison.json"),
        serde_json::to_vec_pretty(&results)?,
    )?;
    Ok(())
}
fn timed<T>(root: &Path, name: &str, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let start = Instant::now();
    let result = operation();
    let path = root.join("timings.json");
    let mut values: Value = fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(json!({}));
    let seconds = start.elapsed().as_secs_f64();
    let previous = values[name]["accumulated_seconds"]
        .as_f64()
        .or_else(|| values[name]["seconds"].as_f64())
        .unwrap_or(0.);
    values[name] =
        json!({"seconds":seconds,"accumulated_seconds":previous+seconds,"ok":result.is_ok()});
    fs::write(path, serde_json::to_vec_pretty(&values)?)?;
    result
}
// 削減キューは誤差適合の完了後にだけ確定する。初期再開では適合を繰り返さない。
// certify中に監査違反を検出した場合の明示的fit_finest呼出しは、この判定を通らない。
fn initial_fit_required(s:&Store)->Result<bool> {
    Ok(state(s,"fit-13")?.is_none()
        || (state(s,"prune-queue-13")?.is_none() && state(s,"prune-13")?.is_none()))
}
fn configured_threads(default: usize) -> usize {
    let available = std::thread::available_parallelism().map(|n|n.get()).unwrap_or(1);
    std::env::var("TERRAIN_PRUNE_THREADS").ok().and_then(|v|v.parse().ok())
        .unwrap_or(default).clamp(1,available)
}
pub fn execute(root: &Path, limit: usize, mode: &str, window: Vec<u32>) -> Result<()> {
    let workers = std::thread::available_parallelism()
        .map(|v| (v.get() / 2).max(1))
        .unwrap_or(1);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(configured_threads(workers))
        .build_global();
    if mode == "run" && window.is_empty() {
        let proof: Value = serde_json::from_slice(
            &fs::read(root.join("ready.json"))
                .context("全国実行の事前検証が必要です: qualify_national_shared.ps1")?,
        )?;
        let job: Value = serde_json::from_slice(&fs::read(root.join("job.json"))?)?;
        ensure!(
            proof["ready_to_generate"] == true
                && job["ready_to_generate"] == true
                && proof["generation_id"] == job["generation_id"],
            "全国実行の検証世代が違います"
        );
        let hash = natural_inventory::hash_file(&std::env::current_exe()?)?;
        ensure!(
            proof["engine_sha256"] == hash,
            "検証後に生成器が変化しました。事前検証を再実行してください"
        );
    }
    let lock = Connection::open(root.join("engine-lock.sqlite3"))?;
    lock.busy_timeout(std::time::Duration::ZERO)?;
    lock.execute_batch("CREATE TABLE IF NOT EXISTS lock(id INTEGER);BEGIN EXCLUSIVE")?;
    let window = if mode == "verify" && window.is_empty() {
        let c = Connection::open_with_flags(
            root.join("work.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let raw: String = c.query_row("SELECT value FROM meta WHERE key='identity'", [], |r| {
            r.get(0)
        })?;
        let v: Value = serde_json::from_str(&raw)?;
        serde_json::from_value(v["window"].clone())?
    } else {
        window
    };
    let started = Instant::now();
    let result = (|| -> Result<()> {
        let (s, job, inputs) = initialize(root, limit, &window)?;
        if mode == "init" {
            progress(
                root,
                json!({"stage":"initialized","inputs":inputs.len(),"complete":false}),
            )?;
            return Ok(());
        }
        let cache = job["identity"]["cache"]
            .as_str()
            .context("元DEMキャッシュなし")?;
        let mut dem = Dem {
            root: cache.into(),
            cache: HashMap::new(),
            order: VecDeque::new(),
        };
        // 初期点準備もPNG原本のSHA256と復号を全件検査する。
        let seeding = mode == "run"
            && !root.join("terrain-shared.sqlite3").exists()
            && state(&s, "seed-global-v1")?.is_none()
            && state(&s, "fit-13")?.is_none();
        if seeding {
            timed(root, "seed-global", || seed_global(&s, &inputs, &dem, root))?;
        }
        // 再開時も原本変更を見逃さない。前回の監査結果だけで入力を信用しない。
        for (n, input) in inputs.iter().enumerate().filter(|_| !seeding) {
            dem.read(input)?;
            if n % 32 == 0 {
                progress(
                    root,
                    json!({"stage":"check-inputs","done":n+1,"total":inputs.len(),"complete":false}),
                )?;
            }
        }
        if mode == "verify" || root.join("terrain-shared.sqlite3").exists() {
            let report = verify_distribution(root)?;
            let audit = audit_saved(root, &inputs, &mut dem)?;
            let seams = verify_seams(root)?;
            let result = json!({"integers":report,"saved_dem":audit,"seams":seams});
            fs::write(
                root.join("verify.json"),
                serde_json::to_vec_pretty(&result)?,
            )?;
            progress(
                root,
                json!({"stage":"complete","scope":if window.is_empty(){"national"}else{"window-test"},"complete":true,"national_complete":window.is_empty(),"verification":result}),
            )?;
            return Ok(());
        }
        if initial_fit_required(&s)? {
            timed(root, "fit-13", || fit_finest(&s, &inputs, &mut dem, root))?;
        }
        timed(root, "prune-13", || {
            prune(&s, 13, 13, 25., &inputs, &mut dem, root)
        })?;
        timed(root, "certify-13", || {
            certify(&s, 13, &inputs, &mut dem, root)
        })?;
        for pair in LEVELS.windows(2) {
            let (fine, _) = pair[0];
            let (z, e) = pair[1];
            timed(root, &format!("prune-{z}"), || {
                prune(&s, z, fine, e, &inputs, &mut dem, root)
            })?;
            timed(root, &format!("certify-{z}"), || {
                certify(&s, z, &inputs, &mut dem, root)
            })?;
        }
        for &(z, _) in &LEVELS {
            if z == 13 {
                continue;
            }
            let missing:i64=s.db.query_row("SELECT count(*) FROM active a WHERE a.z=? AND NOT EXISTS(SELECT 1 FROM active b WHERE b.z=? AND b.id=a.id)",params![z,z+1],|r|r.get(0))?;
            ensure!(missing == 0, "粗品質の点が細品質に含まれていません: z{z}");
        }
        timed(root, "pack", || pack(&s, &job, &inputs, root, &window))?;
        let seams = verify_seams(root)?;
        fs::write(root.join("seams.json"), serde_json::to_vec_pretty(&seams)?)?;
        let saved_audit = timed(root, "verify-saved", || {
            audit_saved(root, &inputs, &mut dem)
        })?;
        fs::write(
            root.join("saved-audit.json"),
            serde_json::to_vec_pretty(&saved_audit)?,
        )?;
        let report = verify_distribution(root)?;
        fs::write(
            root.join("verify.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        let verification = json!({"integer_roundtrip":report,"saved_dem":saved_audit,"same_quality_seams":seams,"minimum_angle_degrees":10,"support_certified":true});
        finalize(root, &window, &verification)?;
        progress(
            root,
            json!({"stage":"complete","scope":if window.is_empty(){"national"}else{"window-test"},"seconds":started.elapsed().as_secs_f64(),"complete":true,"national_complete":window.is_empty(),"distributable":true,"verification":verification}),
        )?;
        Ok(())
    })();
    if let Err(e) = &result {
        let _ = natural_inventory::progress(
            root,
            json!({"stage":"paused-error","error":format!("{e:#}"),"complete":false,"distributable":false,"checkpoint":"work.sqlite3"}),
        );
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    fn temporary() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "natural-pipeline-{}-{}-{}",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn fixture(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("cache/14/14500"))?;
        let mut raw = Vec::new();
        {
            let mut e = png::Encoder::new(&mut raw, 256, 256);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()?
                .write_image_data(&vec![0; 256 * 256 * 3])?;
        }
        fs::write(root.join("cache/14/14500/6400.png"), &raw)?;
        let job = json!({"format":"national-shared-prepared-v1","inputs_verified":true,"generation_id":"test","settings":serde_json::from_str::<Value>(include_str!("../../../NATIONAL_SHARED_SETTINGS.json"))?,"identity":{"cache":root.join("cache").to_string_lossy()}});
        fs::write(root.join("job.json"), serde_json::to_vec(&job)?)?;
        let c = Connection::open(root.join("inputs.sqlite3"))?;
        c.execute_batch(
            "CREATE TABLE inputs(x INTEGER,y INTEGER,state TEXT,sha256 TEXT,verified INTEGER)",
        )?;
        c.execute(
            "INSERT INTO inputs VALUES(14500,6400,'cached',?,1)",
            [natural_inventory::hash(&raw)],
        )?;
        fs::create_dir_all(root.join("cache/14/14502"))?;
        fs::write(root.join("cache/14/14502/6400.png"), &raw)?;
        c.execute(
            "INSERT INTO inputs VALUES(14502,6400,'cached',?,1)",
            [natural_inventory::hash(&raw)],
        )?;
        Ok(())
    }
    #[test]
    fn parallel_proposals_recompute_interfering_input() -> Result<()> {
        let root = temporary();
        fixture(&root)?;
        let mut bytes = Vec::new();
        {
            let mut e = png::Encoder::new(&mut bytes, 256, 256);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()?
                .write_image_data(&[0u8, 78, 32].repeat(256 * 256))?;
        }
        for x in [14500, 14502] {
            fs::write(root.join(format!("cache/14/{x}/6400.png")), &bytes)?;
        }
        {
            let c = Connection::open(root.join("inputs.sqlite3"))?;
            c.execute(
                "UPDATE inputs SET sha256=?",
                [natural_inventory::hash(&bytes)],
            )?;
        }
        let (s, job, inputs) = initialize(&root, 4096, &[14500, 6400, 14503, 6401])?;
        let mut dem = Dem {
            root: PathBuf::from(job["identity"]["cache"].as_str().unwrap()),
            cache: HashMap::new(),
            order: VecDeque::new(),
        };
        fit_finest_workers(&s, &inputs, &mut dem, &root, 2)?;
        let report = state(&s, "fit-13")?.unwrap();
        assert_eq!(report["workers"], 2);
        assert!(report["conflicting_proposals_recomputed"].as_u64().unwrap() > 0);
        assert!(audit_all(&s, 13, 25., &inputs, &mut dem, &root, false)?.0);
        parallel_prune::run(&s, 12, 13, 60., &inputs, &mut dem, &root, 2)?;
        assert!(audit_all(&s, 12, 60., &inputs, &mut dem, &root, false)?.0);
        drop(s);
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn parallel_pruning_commits_disjoint_jobs_and_resumes() -> Result<()> {
        let root = temporary();
        fixture(&root)?;
        let (s, job, inputs) = initialize(&root, 4096, &[14500, 6400, 14503, 6401])?;
        for i in &inputs {
            let b = rect(i);
            for y in 0..9 {
                for x in 0..9 {
                    let q = quant([
                        b[0] + STEP * (60. + x as f64 * 16.),
                        b[1] + STEP * (60. + y as f64 * 16.),
                        0.,
                    ])?;
                    s.insert(13, q, x == 0 || y == 0 || x == 8 || y == 8)?;
                }
            }
        }
        let mut dem = Dem {
            root: PathBuf::from(job["identity"]["cache"].as_str().unwrap()),
            cache: HashMap::new(),
            order: VecDeque::new(),
        };
        parallel_prune::run(&s, 12, 13, 60., &inputs, &mut dem, &root, 2)?;
        let report = state(&s, "prune-12")?.unwrap();
        assert!(report["maximum_parallel_jobs"].as_u64().unwrap() > 1);
        assert!(audit_all(&s, 12, 60., &inputs, &mut dem, &root, false)?.0);
        let epoch = s.epoch(12)?;
        parallel_prune::run(&s, 12, 13, 60., &inputs, &mut dem, &root, 2)?;
        assert_eq!(epoch, s.epoch(12)?);
        drop(s);
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn batch_insert_rolls_back_conflicting_height() -> Result<()> {
        let s = Store::open(Path::new(":memory:"), 4096)?;
        s.insert_batch(13, &[[0, 0, 0], [160, 0, 80], [0, 160, 0]])?;
        let epoch = s.epoch(13)?;
        s.cache_level(13)?;
        assert!(s.insert_batch(13, &[[32, 32, 8], [160, 0, 81]]).is_err());
        assert_eq!(s.epoch(13)?, epoch);
        let n: i64 =
            s.db.query_row("SELECT count(*) FROM points WHERE x=32 AND y=32", [], |r| {
                r.get(0)
            })?;
        assert_eq!(n, 0);
        assert_eq!(s.insert_batch(13, &[[32, 32, 8], [64, 64, 16]])?, 2);
        assert_eq!(s.epoch(13)?, epoch + 2);
        Ok(())
    }
    #[test]
    fn proposal_circle_conflict_checks_exact_distance() {
        assert!(circle_hits([0., 0., 4.], [-0.1, -0.1, 0.1, 0.1]));
        assert!(!circle_hits([0., 0., 4.], [1.8, 1.8, 2., 2.]));
        assert!(circle_hits([0., 0., 4.], [2., 0., 3., 1.]));
    }
    #[test]
    fn batched_dem_decode_is_bit_exact_and_reuses_cache()->Result<()> {
        let root=temporary();fixture(&root)?;
        let (_s,job,inputs)=initialize(&root,4096,&[14500,6400,14503,6401])?;
        let path=PathBuf::from(job["identity"]["cache"].as_str().unwrap());
        let mut serial=Dem{root:path.clone(),cache:HashMap::new(),order:VecDeque::new()};
        let mut parallel=Dem{root:path,cache:HashMap::new(),order:VecDeque::new()};
        let expected=inputs.iter().map(|i|serial.read(i)).collect::<Result<Vec<_>>>()?;
        let actual=parallel.read_many(&inputs)?;let cached=parallel.read_many(&inputs)?;
        for ((a,b),c) in expected.iter().zip(&actual).zip(&cached) {
            assert!(a.iter().zip(b.iter()).all(|(x,y)|x.to_bits()==y.to_bits()));assert!(Arc::ptr_eq(b,c));
        }
        drop(_s);fs::remove_dir_all(root)?;Ok(())
    }
    #[test]
    fn prune_resume_skips_initial_fit_but_not_unfitted_input() -> Result<()> {
        let s=Store::open(Path::new(":memory:"),4096)?;
        assert!(initial_fit_required(&s)?);
        done(&s,"fit-13",json!({"epoch":0}))?;
        assert!(initial_fit_required(&s)?);
        done(&s,"prune-queue-13",json!({"workers":10,"candidates":100}))?;
        assert!(!initial_fit_required(&s)?);
        s.bump(13)?;
        assert!(!initial_fit_required(&s)?);
        s.db.execute("DELETE FROM stages WHERE name='fit-13'",[])?;
        assert!(initial_fit_required(&s)?);
        Ok(())
    }
    #[test]
    fn all_levels_roundtrip_and_resume() -> Result<()> {
        let root = temporary();
        fixture(&root)?;
        let w = vec![14500, 6400, 14503, 6401];
        execute(&root, 4096, "run", w.clone())?;
        let first = verify_distribution(&root)?;
        assert_eq!(first["tiles"], 9);
        assert!(verify_seams(&root)?["samples"].as_u64().unwrap() > 0);
        assert!(first["maximum_dependency_tiles"].as_u64().unwrap() <= 3);
        execute(&root, 4096, "run", w)?;
        assert_eq!(first, verify_distribution(&root)?);
        let path = root.join("cache/14/14500/6400.png");
        let original = fs::read(&path)?;
        let mut changed = original.clone();
        changed[40] ^= 1;
        fs::write(&path, changed)?;
        assert!(execute(&root, 4096, "verify", Vec::new()).is_err());
        fs::write(path, original)?;
        {
            let c = Connection::open(distribution_path(&root))?;
            c.execute("UPDATE tiles SET xyz_sha256='bad' WHERE z=13", [])?;
        }
        assert!(verify_distribution(&root).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn committed_fit_cursor_resumes_and_rejects_wrong_epoch() -> Result<()> {
        let root = temporary();
        fixture(&root)?;
        let (s, job, inputs) = initialize(&root, 4096, &[14500, 6400, 14503, 6401])?;
        let mut dem = Dem {
            root: PathBuf::from(job["identity"]["cache"].as_str().unwrap()),
            cache: HashMap::new(),
            order: VecDeque::new(),
        };
        let mut hash = Sha256::new();
        for i in &inputs {
            hash.update(i.x.to_le_bytes());
            hash.update(i.y.to_le_bytes());
            hash.update(i.sha.as_bytes());
        }
        let signature = hash
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let mut cursor = json!({"format":"fit-cursor-v1","signature":signature,"total":inputs.len(),"workers":2,"pass":1,"next_offset":1,"completed":2,"added":0,"fallback":0,"elapsed_seconds":0.,"epoch":s.epoch(13)?});
        s.db.execute(
            "INSERT INTO meta VALUES('fit-cursor-v1',?)",
            [cursor.to_string()],
        )?;
        fit_finest_workers(&s, &inputs, &mut dem, &root, 2)?;
        assert_eq!(
            state(&s, "fit-13")?.unwrap()["resumed_completed_inputs"].as_u64(),
            Some(2)
        );
        assert!(
            s.db.query_row(
                "SELECT value FROM meta WHERE key='fit-cursor-v1'",
                [],
                |r| r.get::<_, String>(0)
            )
            .optional()?
            .is_none()
        );
        s.db.execute("DELETE FROM stages WHERE name='fit-13'", [])?;
        cursor["epoch"] = json!(-1);
        s.db.execute(
            "INSERT INTO meta VALUES('fit-cursor-v1',?)",
            [cursor.to_string()],
        )?;
        fit_finest_workers(&s, &inputs, &mut dem, &root, 2)?;
        assert_eq!(
            state(&s, "fit-13")?.unwrap()["resumed_completed_inputs"].as_u64(),
            Some(0)
        );
        drop(s);
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn audit_reuse_requires_same_epoch_and_valid_source() -> Result<()> {
        let root = temporary();
        fixture(&root)?;
        let (s, job, inputs) = initialize(&root, 4096, &[14500, 6400, 14503, 6401])?;
        let mut dem = Dem {
            root: PathBuf::from(job["identity"]["cache"].as_str().unwrap()),
            cache: HashMap::new(),
            order: VecDeque::new(),
        };
        let first = audit_all(&s, 13, 25., &inputs, &mut dem, &root, true)?;
        assert!(first.0);
        assert_eq!(
            first,
            audit_all(&s, 13, 25., &inputs, &mut dem, &root, true)?
        );
        let progress: Value = serde_json::from_slice(&fs::read(root.join("progress.json"))?)?;
        assert_eq!(
            progress["reused_inputs"].as_u64(),
            Some(inputs.len() as u64)
        );
        let i = &inputs[0];
        s.insert(
            13,
            quant([
                (i.x as f64 * 256. + 128.5) * STEP,
                (i.y as f64 * 256. + 128.5) * STEP,
                1000.,
            ])?,
            false,
        )?;
        assert!(!audit_all(&s, 13, 25., &inputs, &mut dem, &root, false)?.0);
        let path = dem
            .root
            .join("14")
            .join(i.x.to_string())
            .join(format!("{}.png", i.y));
        let mut bytes = fs::read(&path)?;
        bytes[40] ^= 1;
        fs::write(&path, bytes)?;
        dem.cache.clear();
        dem.order.clear();
        assert!(audit_all(&s, 13, 25., &inputs, &mut dem, &root, true).is_err());
        drop(s);
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn interrupted_delete_rolls_back() -> Result<()> {
        let root = temporary();
        fs::create_dir_all(&root)?;
        let path = root.join("work.sqlite3");
        {
            let s = Store::open(&path, 32)?;
            s.insert(13, [0, 0, 0], true)?;
            s.db.execute_batch("SAVEPOINT interrupted;DELETE FROM active")?;
        }
        {
            let s = Store::open(&path, 32)?;
            assert_eq!(
                s.db.query_row("SELECT count(*) FROM active", [], |r| r.get::<_, i64>(0))?,
                1
            );
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn indexed_fine_candidate_matches_nearest_and_excludes_existing() -> Result<()> {
        let root = temporary();
        fs::create_dir_all(&root)?;
        let store = Store::open(&root.join("work.sqlite3"), 256)?;
        for p in [[0, 0, 0], [1600, 0, 0], [0, 16000, 0]] {
            store.insert(13, p, false)?;
        }
        let id: i64 = store
            .db
            .query_row("SELECT id FROM points WHERE x=0 AND y=0", [], |r| r.get(0))?;
        store.set_active(12, id, true)?;
        let expected: i64 = store
            .db
            .query_row("SELECT id FROM points WHERE x=1600", [], |r| r.get(0))?;
        assert_eq!(nearest_fine(&store, 12, [40., 0.])?, Some(expected));
        store.db.execute(
            "INSERT OR IGNORE INTO active SELECT 12,id FROM active WHERE z=13",
            [],
        )?;
        assert!(nearest_fine(&store, 12, [40., 0.])?.is_none());
        drop(store);
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn angular_walk_matches_full_scan_at_world_offsets() -> Result<()> {
        for origin in [[0.,0.],[35000000.,14000000.]] {
            let mut points=vec![[-5000.,-5000.,0.],[5000.,-5000.,0.],[5000.,5000.,0.],[-5000.,5000.,0.]];
            for y in 0..32 {for x in 0..32 {points.push([x as f64*17.+((x*3+y*7)%11) as f64*0.0625,y as f64*19.+((x*5+y*3)%13) as f64*0.0625,0.]);}}
            for p in &mut points {p[0]+=origin[0];p[1]+=origin[1];}
            let model=crate::natural_regional_surface::Fast::certified(points)?;
            for size in [1.,25.,140.,800.] {for k in 0..16 {
                let x=origin[0]+k as f64*31.;let y=origin[1]+k as f64*23.;let b=[x-size,y-size,x+size,y+size];
                let fast=worst_angle_surface(&model.base,b,[0.,0.]).0;
                let reference=minimum_angle_surface_reference(&model.base,b);
                assert!((fast-reference).abs()<1e-12,"fast={fast} reference={reference} bounds={b:?}");
            }}
        }Ok(())
    }
    #[test]
    fn certify_resume_repeats_partial_input_and_forces_full_followup() -> Result<()> {
        let s=Store::open(Path::new(":memory:"),8192)?;s.insert(13,[0,0,0],true)?;
        save_certify_cursor(&s,13,100,2,32,0,10.)?;
        let v=certify_cursor(&s,13,100)?.unwrap();assert_eq!(v["next_input"],32);assert_eq!(v["changes"],0);
        s.insert(13,[16,16,8],false)?;
        let v=certify_cursor(&s,13,100)?.unwrap();assert_eq!(v["next_input"],32);assert_eq!(v["changes"],1);
        assert!(certify_cursor(&s,13,101).is_err());assert!(state(&s,"certify-13")?.is_none());
        Ok(())
    }

    #[test]
    fn parallel_audit_matches_serial_cache_and_repair_failure() -> Result<()> {
        let root=temporary();fixture(&root)?;let (s,job,inputs)=initialize(&root,4096,&[14500,6400,14503,6401])?;
        let mut dem=Dem{root:PathBuf::from(job["identity"]["cache"].as_str().unwrap()),cache:HashMap::new(),order:VecDeque::new()};
        s.cache_level(13)?;
        let serial=audit_all_serial_reference(&s,13,25.,&inputs,&mut dem,&root,true)?;
        let rows=|s:&Store|->Result<Vec<(i64,i64,i64,u64,f64,f64)>> {Ok(s.db.prepare("SELECT x,y,epoch,valid,up,down FROM audit WHERE z=13 ORDER BY y,x")?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)};
        let expected=rows(&s)?;s.db.execute("DELETE FROM audit WHERE z=13",[])?;
        assert_eq!(serial,audit_all(&s,13,25.,&inputs,&mut dem,&root,true)?);assert_eq!(rows(&s)?,expected);
        assert_eq!(serial,audit_all(&s,13,25.,&inputs,&mut dem,&root,true)?);
        let p:Value=serde_json::from_slice(&fs::read(root.join("progress.json"))?)?;assert_eq!(p["reused_inputs"],inputs.len());
        let b=rect(&inputs[0]);s.insert(13,quant([(b[0]+b[2])*0.5,(b[1]+b[3])*0.5,1000.])?,false)?;
        assert!(!audit_all_policy(&s,13,25.,&inputs,&mut dem,&root,true,true)?.0);
        let p:Value=serde_json::from_slice(&fs::read(root.join("progress.json"))?)?;assert_eq!(p["early_failure"],true);assert!(state(&s,"certify-13")?.is_none());
        assert!(!audit_all(&s,13,25.,&inputs,&mut dem,&root,true)?.0);
        drop(s);fs::remove_dir_all(root)?;Ok(())
    }
    #[test]
    fn parallel_page_encoding_is_byte_identical_and_rejects_stale_epoch() -> Result<()> {
        let root=temporary();fixture(&root)?;let (s,_job,inputs)=initialize(&root,4096,&[14500,6400,14503,6401])?;s.cache_level(13)?;
        let patches=s.patches(13,&inputs.iter().map(rect).collect::<Vec<_>>())?;
        for (k,p) in patches.iter().enumerate(){s.persist_patch(13,&format!("serial{k}"),p)?;}
        let keys=(0..patches.len()).map(|k|format!("parallel{k}")).collect::<Vec<_>>();s.persist_patches(13,&keys,&patches)?;
        for k in 0..patches.len(){let a:Vec<u8>=s.db.query_row("SELECT payload FROM pages WHERE z=13 AND key=?",[format!("serial{k}")],|r|r.get(0))?;let b:Vec<u8>=s.db.query_row("SELECT payload FROM pages WHERE z=13 AND key=?",[format!("parallel{k}")],|r|r.get(0))?;assert_eq!(a,b);}
        s.insert(13,quant([rect(&inputs[0])[0]+STEP,rect(&inputs[0])[1]+STEP,1.])?,false)?;
        assert!(s.persist_patches(13,&keys,&patches).is_err());drop(s);fs::remove_dir_all(root)?;Ok(())
    }

    #[test]
    fn verified_fit_cache_reuses_proofs_and_rejects_external_epochs() -> Result<()> {
        let root=temporary();fixture(&root)?;let(s,job,inputs)=initialize(&root,4096,&[14500,6400,14503,6401])?;
        let mut cache=fit_cache::Verified::load(&s,&inputs,"test-signature")?;
        for i in &inputs {cache.mark(i);}cache.save(&s,"test-signature")?;
        let b=rect(&inputs[0]);cache.invalidate(&[[(b[0]+b[2])*0.5,(b[1]+b[3])*0.5,STEP*STEP]]);
        assert!(!cache.has(0));assert!(cache.has(1));cache.save(&s,"test-signature")?;
        let restored=fit_cache::Verified::load(&s,&inputs,"test-signature")?;assert!(!restored.has(0));assert!(restored.has(1));
        assert_eq!(fit_cache::Verified::load(&s,&inputs,"different")?.count(),0);
        s.insert(13,quant([b[0]+STEP,b[1]+STEP,0.])?,false)?;
        assert_eq!(fit_cache::Verified::load(&s,&inputs,"test-signature")?.count(),0);
        let mut dem=Dem{root:PathBuf::from(job["identity"]["cache"].as_str().unwrap()),cache:HashMap::new(),order:VecDeque::new()};
        s.db.execute("INSERT INTO meta VALUES('benchmark_only','true')",[])?;
        fit_finest_budget(&s,&inputs,&mut dem,&root,1,Some(1))?;
        fit_finest_budget(&s,&inputs,&mut dem,&root,1,Some(1))?;
        let report:Value=serde_json::from_slice(&fs::read(root.join("benchmark-fit.json"))?)?;
        assert_eq!(report["verified_inputs_reused"],1);assert_eq!(report["added_points"],0);
        assert!(audit_all(&s,13,25.,&inputs,&mut dem,&root,true)?.0);
        drop(s);fs::remove_dir_all(root)?;Ok(())
    }
    #[test]
    fn insertion_before_after_circles_enclose_all_observed_field_changes() -> Result<()> {
        let root=temporary();fixture(&root)?;let(s,_job,inputs)=initialize(&root,8192,&[14500,6400,14503,6401])?;
        let b=fit_bounds(&inputs[0]);
        for y in 0..17 {for x in 0..17 {s.insert(13,quant([b[0]+STEP*(x as f64*16.+0.5),b[1]+STEP*(y as f64*16.+0.5),((x*7+y*11)%31) as f64])?,false)?;}}
        s.cache_level(13)?;let before=s.patch(13,b)?;let mut after=s.patch(13,b)?;
        let mut points=Vec::new();let mut circles=Vec::new();let mut known=HashMap::new();
        proposal_insert(quant([b[0]+STEP*127.37,b[1]+STEP*129.71,500.])?,&mut after,&mut points,&mut circles,&mut known)?;
        let mut a=Work::default();let mut c=Work::default();let mut outside=0;
        for y in 1..64 {for x in 1..64 {let q=[b[0]+(b[2]-b[0])*x as f64/64.,b[1]+(b[3]-b[1])*y as f64/64.];
            if !circles.iter().any(|&v|circle_hits(v,[q[0],q[1],q[0],q[1]])) {outside+=1;assert!((before.evaluate(q,&mut a)?-after.evaluate(q,&mut c)?).abs()<1e-7);}
        }}assert!(outside>1000);drop(s);fs::remove_dir_all(root)?;Ok(())
    }
    #[test]
    fn repair_fit_marker_is_retained_on_failure_and_cleared_after_verified_success() -> Result<()> {
        let root=temporary();fixture(&root)?;let(s,job,inputs)=initialize(&root,4096,&[14500,6400,14503,6401])?;
        let mut dem=Dem{root:PathBuf::from(job["identity"]["cache"].as_str().unwrap()),cache:HashMap::new(),order:VecDeque::new()};
        repair_certify_fit(&s,&inputs,&mut dem,&root)?;
        assert!(state(&s,"fit-13")?.is_some());assert!(s.db.query_row("SELECT 1 FROM meta WHERE key='certify-repair-fit-13-v1'",[],|_|Ok(())).optional()?.is_none());
        s.db.execute("DELETE FROM stages WHERE name='fit-13'",[])?;
        let path=dem.root.join("14").join(inputs[0].x.to_string()).join(format!("{}.png",inputs[0].y));
        fs::write(path,b"corrupted source")?;dem.cache.clear();dem.order.clear();
        assert!(repair_certify_fit(&s,&inputs,&mut dem,&root).is_err());
        assert!(s.db.query_row("SELECT 1 FROM meta WHERE key='certify-repair-fit-13-v1'",[],|_|Ok(())).optional()?.is_some());
        assert!(state(&s,"fit-13")?.is_none());
        drop(s);fs::remove_dir_all(root)?;Ok(())
    }

    #[test]
    fn broad_insert_invalidation_preserves_unaffected_verified_regions() -> Result<()> {
        let s=Store::open(Path::new(":memory:"),8192)?;s.insert(13,[0,0,0],true)?;
        let inputs=(12000..12600).map(|x|Input{x,y:6400,sha:String::new()}).collect::<Vec<_>>();
        let mut cache=fit_cache::Verified::load(&s,&inputs,"broad-test")?;for i in &inputs{cache.mark(i);}
        let b=rect(&inputs[0]);let side=b[2]-b[0];
        cache.invalidate(&[[(b[0]+b[2])*0.5,(b[1]+b[3])*0.5,(side*340.).powi(2)]]);
        assert!(!cache.has(0));assert!(!cache.has(300));assert!(cache.has(599));Ok(())
    }

    #[test]
    fn repair_local_invalidation_encloses_changed_surface_and_keeps_remote_proofs() -> Result<()> {
        let root=temporary(); fixture(&root)?;
        let(s,_,inputs)=initialize(&root,8192,&[14500,6400,14503,6401])?;
        let b=fit_bounds(&inputs[0]);
        for y in 0..17 { for x in 0..17 {
            s.insert(12,quant([b[0]+STEP*(x as f64*16.+0.5),
                b[1]+STEP*(y as f64*16.+0.5),((x*7+y*11)%31) as f64])?,false)?;
        }}
        let before=s.patch(12,b)?;
        let mut after=s.patch(12,b)?;
        let q=quant([b[0]+STEP*127.37,b[1]+STEP*129.71,500.])?;
        let xy=[q[0] as f64/16.,q[1] as f64/16.];
        let mut circles=repair_support_circles(&before,xy)?;
        let mut angles=fit_cache::Verified::empty(&inputs);
        let mut errors=fit_cache::Verified::empty(&inputs);
        for i in &inputs { angles.mark(i); errors.mark(i); }
        assert!(repair_insert(&s,12,q,&mut after,&mut angles,&mut errors)?);
        circles.extend(repair_support_circles(&after,xy)?);
        assert!(!angles.has(0)); assert!(!errors.has(0));
        let mut a=Work::default();let mut c=Work::default();let mut outside=0;
        for y in 1..64 {for x in 1..64 {
            let q=[b[0]+(b[2]-b[0])*x as f64/64.,b[1]+(b[3]-b[1])*y as f64/64.];
            if !circles.iter().any(|&v|circle_hits(v,[q[0],q[1],q[0],q[1]])) {
                outside+=1;
                assert!((before.evaluate(q,&mut a)?-after.evaluate(q,&mut c)?).abs()<1e-7);
            }
        }}
        assert!(outside>1000);
        let remote=Input{x:15000,y:6400,sha:String::new()};
        let mut proof=fit_cache::Verified::empty(&[inputs[0].clone(),remote.clone()]);
        proof.mark(&inputs[0]);proof.mark(&remote);proof.invalidate(&circles);
        assert!(!proof.has(0));assert!(proof.has(1));
        drop(s);fs::remove_dir_all(root)?;Ok(())
    }

}
