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
struct Dem {
    root: PathBuf,
    cache: HashMap<(i32, i32), Arc<Vec<f64>>>,
    order: VecDeque<(i32, i32)>,
}
impl Dem {
    fn read(&mut self, i: &Input) -> Result<Arc<Vec<f64>>> {
        let key = (i.x, i.y);
        if let Some(v) = self.cache.get(&key) {
            return Ok(v.clone());
        }
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
        if self.cache.len() >= 128 {
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
    let mut w = Work::default();
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
        }
    }
    Ok(a)
}
fn audit_all(
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
    let epoch = s.epoch(z)?;
    for (n, i) in inputs.iter().enumerate() {
        let p = s.patch(z, rect(i))?;
        let r = dem.read(i)?;
        let a = audit_patch(&p, i, &r)?;
        maximum = maximum.max(a.max);
        valid += a.valid;
        good &= a.max <= error + 1e-8;
        if save {
            s.db.execute(
                "INSERT OR REPLACE INTO audit VALUES(?,?,?,?,?,?,?)",
                params![z, i.x, i.y, epoch, a.valid, a.up, a.down],
            )?;
        }
        if n % 32 == 0 {
            progress(
                root,
                json!({"stage":"audit","z":z,"done":n+1,"total":inputs.len(),"maximum_directional_error":maximum,"valid_pixels":valid,"complete":false}),
            )?;
        }
        if !save && !good {
            return Ok((false, maximum, valid));
        }
    }
    Ok((good, maximum, valid))
}
fn audit_affected(
    s: &Store,
    z: i64,
    error: f64,
    inputs: &[Input],
    dem: &mut Dem,
    influence: [f64; 4],
) -> Result<bool> {
    for input in inputs {
        let p = s.patch(z, rect(input))?;
        let raster = dem.read(input)?;
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
        if audit_region(&p, input, &raster, range)?.max > error + 1e-8 {
            return Ok(false);
        }
    }
    Ok(true)
}
fn fit_finest(s: &Store, inputs: &[Input], dem: &mut Dem, root: &Path) -> Result<()> {
    if state(s, "fit-13")?.is_some_and(|v| v["epoch"].as_i64() == Some(s.epoch(13).unwrap_or(-1))) {
        return Ok(());
    }
    let source = inputs
        .iter()
        .map(|i| ((i.x, i.y), i))
        .collect::<HashMap<_, _>>();
    let mut pass = 0;
    loop {
        pass += 1;
        let mut added = 0;
        for (n, i) in inputs.iter().enumerate() {
            let raster = dem.read(i)?;
            let mut p = s.patch(13, rect(i))?;
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
                .map(|&r| audit_region(&p, i, &raster, r))
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
                    if s.insert(13, q, false)? {
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
                            let nearest = if let Some(input) =
                                source.get(&(x.div_euclid(256), y.div_euclid(256)))
                            {
                                dem.read(input)?
                                    [(y.rem_euclid(256) * 256 + x.rem_euclid(256)) as usize]
                            } else {
                                f64::NAN
                            };
                            let h = if nearest.is_finite() {
                                nearest
                            } else {
                                q[2] as f64 / 8.
                            };
                            if s.insert(
                                13,
                                quant([(x as f64 + 0.5) * STEP, (y as f64 + 0.5) * STEP, h])?,
                                false,
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
                        progress(
                            root,
                            json!({"stage":"fit-local","z":13,"pass":pass,"input":n+1,"inputs":inputs.len(),"added_this_pass":added,"complete":false}),
                        )?;
                    }
                    p = s.patch(13, rect(i))?;
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
                            blocks[k] = audit_region(&p, i, &raster, r)?;
                        }
                    }
                }
            }
            if n % 16 == 0 {
                progress(
                    root,
                    json!({"stage":"fit","z":13,"pass":pass,"done":n+1,"total":inputs.len(),"added_this_pass":added,"complete":false}),
                )?;
            }
        }
        if added == 0 {
            let (ok, max, valid) = audit_all(s, 13, 25., inputs, dem, root, true)?;
            ensure!(ok, "最終監査に失敗");
            done(
                s,
                "fit-13",
                json!({"epoch":s.epoch(13)?,"max":max,"valid":valid,"passes":pass}),
            )?;
            break;
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
    // 再開時は未確定の削除をDBトランザクションが戻す。試行済みの候補は保持する。
    let cursor_key = format!("cursor-{z}");
    let mut cursor =
        s.db.query_row("SELECT value FROM meta WHERE key=?", [&cursor_key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .unwrap_or_else(|| "0".into())
        .parse::<i64>()?;
    loop {
        let id=s.db.query_row("SELECT a.id FROM active a JOIN points p ON p.id=a.id WHERE a.z=? AND a.id>? AND p.guard=0 ORDER BY a.id LIMIT 1",params![z,cursor],|r|r.get::<_,i64>(0)).optional()?;
        let Some(id) = id else { break };
        let q = s.point(id)?;
        let before = s.patch(z, [q[0] - 0.125, q[1] - 0.125, q[0] + 0.125, q[1] + 0.125])?;
        let local_id = before
            .ids
            .iter()
            .position(|&v| v == id)
            .context("削除点の接続がありません")?;
        let influence = before.deletion_box(local_id, STEP * 2.);
        let affected = affected_inputs(inputs, &spatial_inputs, influence);
        // 点を削除した旧外接円の領域だけが自然近傍へ影響する。全日本を毎点再走査しない。
        s.db.execute_batch("SAVEPOINT removal")?;
        s.db.execute("DELETE FROM active WHERE z=? AND id=?", params![z, id])?;
        s.bump(z)?;
        let result = audit_affected(s, z, error, &affected, dem, influence);
        match result {
            Ok(true) => {
                s.db.execute("INSERT OR IGNORE INTO removed VALUES(?,?)", params![z, id])?;
                s.db.execute_batch("RELEASE removal")?;
            }
            Ok(false) => {
                s.db.execute_batch("ROLLBACK TO removal;RELEASE removal")?;
            }
            Err(e) => {
                s.db.execute_batch("ROLLBACK TO removal;RELEASE removal")?;
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
fn minimum_angle_surface(surface: &crate::natural_surface::Surface, b: [f64; 4]) -> f64 {
    let mut minimum = 180f64;
    for t in surface.topology.triangles.chunks_exact(3) {
        let v = t.iter().map(|&i| surface.points[i]).collect::<Vec<_>>();
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
fn certify(s: &Store, z: i64, inputs: &[Input], dem: &mut Dem, root: &Path) -> Result<()> {
    let spatial_inputs = input_index(inputs);
    let key = format!("certify-{z}");
    if state(s, &key)?.is_some_and(|v| v["epoch"].as_i64() == Some(s.epoch(z).unwrap_or(-1))) {
        return Ok(());
    }
    let source = inputs
        .iter()
        .map(|i| ((i.x, i.y), i))
        .collect::<HashMap<_, _>>();
    let fine = if z < 13 { Some(z + 1) } else { None };
    let mut pass = 0;
    loop {
        pass += 1;
        let mut changes = 0;
        let mut min = 180f64;
        for (n, i) in inputs.iter().enumerate() {
            let p = s.patch(z, rect(i))?;
            let angle = minimum_angle(&p, rect(i));
            min = min.min(angle);
            if angle < 10. - 1e-8 {
                // 最悪三角形の最長辺を細分化する。粗品質は細品質の点を戻すだけ。
                let b = rect(i);
                let mut worst = None;
                let mut value = 180f64;
                for t in p.model.base.topology.triangles.chunks_exact(3) {
                    let v = [p.point(t[0]), p.point(t[1]), p.point(t[2])];
                    if !triangle_intersects(&v, b) {
                        continue;
                    }
                    for k in 0..3 {
                        let a = [v[(k + 1) % 3][0] - v[k][0], v[(k + 1) % 3][1] - v[k][1]];
                        let d = [v[(k + 2) % 3][0] - v[k][0], v[(k + 2) % 3][1] - v[k][1]];
                        let angle = ((a[0] * d[0] + a[1] * d[1])
                            / (a[0].hypot(a[1]) * d[0].hypot(d[1])))
                        .clamp(-1., 1.)
                        .acos()
                        .to_degrees();
                        if angle < value {
                            value = angle;
                            worst = Some(v);
                        }
                    }
                }
                let v = worst.context("鋭角面が見つかりません")?;
                // 密集した同高の冗長点が鋭角を作る場合は、誤差を守れる削除を先に試す。
                let mut deleted = false;
                for vertex in v.into_iter().filter(|_| z == 13) {
                    let qp = quant(vertex)?;
                    let id =
                        s.db.query_row(
                            "SELECT id FROM points WHERE x=? AND y=? AND guard=0",
                            params![qp[0], qp[1]],
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
                    s.db.execute_batch("SAVEPOINT angle_delete")?;
                    s.db.execute("DELETE FROM active WHERE z=? AND id=?", params![z, id])?;
                    s.bump(z)?;
                    let tested = (|| -> Result<bool> {
                        let after = s.patch(z, b)?;
                        if minimum_angle(&after, b) < value - 1e-7 {
                            return Ok(false);
                        }
                        let error = LEVELS.iter().find(|v| v.0 == z).unwrap().1;
                        audit_affected(s, z, error, &affected, dem, influence)
                    })();
                    match tested {
                        Ok(true) => {
                            s.db.execute(
                                "INSERT OR IGNORE INTO angle_banned VALUES(?,?)",
                                params![z, id],
                            )?;
                            s.db.execute_batch("RELEASE angle_delete")?;
                            deleted = true;
                            break;
                        }
                        Ok(false) => {
                            s.db.execute_batch("ROLLBACK TO angle_delete;RELEASE angle_delete")?
                        }
                        Err(e) => {
                            s.db.execute_batch("ROLLBACK TO angle_delete;RELEASE angle_delete")?;
                            return Err(e);
                        }
                    }
                }
                if deleted {
                    changes += 1;
                    progress(
                        root,
                        json!({"stage":"angle-prune","z":z,"pass":pass,"changes":changes,"complete":false}),
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
                    let exists=s.db.query_row("SELECT 1 FROM points p WHERE p.x=? AND p.y=? AND (EXISTS(SELECT 1 FROM active a WHERE a.z=? AND a.id=p.id) OR EXISTS(SELECT 1 FROM angle_banned b WHERE b.z=? AND b.id=p.id))",params![key[0],key[1],z,z],|r|r.get::<_,i64>(0)).optional()?.is_some();
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
                    chosen.context("角度補修の候補を使い切りました。量子化精度を維持して停止")?
                };
                if let Some(fine) = fine {
                    let id = nearest_fine(s, z, q)?
                        .context("細品質に角度補修候補がありません。品質を緩めず停止")?;
                    s.set_active(z, id, true)?;
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
                    ensure!(
                        s.insert(z, quant([q[0], q[1], h])?, false)?,
                        "角度補修が量子化限界に達しました: q={q:?} angle={value} triangle={v:?}"
                    );
                }
                changes += 1;
            }
            if n % 32 == 0 {
                progress(
                    root,
                    json!({"stage":"certify","z":z,"pass":pass,"done":n+1,"total":inputs.len(),"minimum_angle":min,"added":changes,"complete":false}),
                )?;
            }
        }
        if changes == 0 {
            let error = LEVELS.iter().find(|v| v.0 == z).unwrap().1;
            let (ok, max, valid) = audit_all(s, z, error, inputs, dem, root, true)?;
            if !ok {
                if z == 13 {
                    fit_finest(s, inputs, dem, root)?;
                    continue;
                }
                let mut repaired = false;
                for input in inputs {
                    let patch = s.patch(z, rect(input))?;
                    let a = audit_patch(&patch, input, &dem.read(input)?)?;
                    if a.max <= error {
                        continue;
                    }
                    let q = a.candidate.context("誤差補修点なし")?;
                    let id = nearest_fine(s, z, [q[0] as f64 / 16., q[1] as f64 / 16.])?
                        .context("誤差補修の細品質点がありません")?;
                    s.set_active(z, id, true)?;
                    repaired = true;
                }
                ensure!(repaired, "角度補修後の誤差を解消できません");
                continue;
            }
            for i in inputs {
                let p = s.patch(z, rect(i))?;
                s.persist_patch(z, &format!("{}/{}", i.x, i.y), &p)?;
            }
            done(
                s,
                &key,
                json!({"minimum_angle":min,"epoch":s.epoch(z)?,"maximum_directional_error":max,"valid":valid,"passes":pass}),
            )?;
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
        let coords = inputs
            .iter()
            .map(|i| ((i.x as i64) >> (14 - z), (i.y as i64) >> (14 - z)))
            .collect::<BTreeSet<_>>();
        for (n, (x, y)) in coords.iter().copied().enumerate() {
            let valid: i64 = s.db.query_row(
                "SELECT coalesce(sum(valid),0) FROM audit WHERE z=? AND (x >> ?)=? AND (y >> ?)=?",
                params![z, 14 - z, x, 14 - z, y],
                |r| r.get(0),
            )?;
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
            for (idx, id) in ids.iter().enumerate() {
                tx.execute(
                    "INSERT OR REPLACE INTO packed_points VALUES(?,?,?,?,?)",
                    params![z, x, y, id, idx],
                )?;
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
        let mut previous = None;
        let mut curve = None;
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
            if previous != Some(key) {
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
                curve = Some(crate::natural_surface::Surface::certified(points)?);
                previous = Some(key);
            }
            let model = curve.as_ref().unwrap();
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
            let mut w = crate::natural_surface::Workspace::default();
            let mut start = 0;
            let mut up = 0f64;
            let mut down = 0f64;
            let mut valid = 0;
            for (index, &h) in raster.iter().enumerate() {
                if !h.is_finite() {
                    continue;
                }
                let q = [
                    (i.x as f64 * 256. + (index % 256) as f64 + 0.5) * STEP,
                    (i.y as f64 * 256. + (index / 256) as f64 + 0.5) * STEP,
                ];
                let side = WORLD / 2f64.powi(key.0 as i32);
                let delta = model.evaluate_reusing(
                    [q[0] - key.1 as f64 * side, q[1] - key.2 as f64 * side],
                    &mut start,
                    &mut w,
                )? - h;
                up = up.max(delta);
                down = down.max(-delta);
                valid += 1;
            }
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
pub fn execute(root: &Path, limit: usize, mode: &str, window: Vec<u32>) -> Result<()> {
    let workers = std::thread::available_parallelism()
        .map(|v| (v.get() / 2).max(1))
        .unwrap_or(1);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
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
        // 再開時も原本変更を見逃さない。前回の監査結果だけで入力を信用しない。
        for (n, input) in inputs.iter().enumerate() {
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
        fit_finest(&s, &inputs, &mut dem, root)?;
        prune(&s, 13, 13, 25., &inputs, &mut dem, root)?;
        certify(&s, 13, &inputs, &mut dem, root)?;
        for pair in LEVELS.windows(2) {
            let (fine, _) = pair[0];
            let (z, e) = pair[1];
            prune(&s, z, fine, e, &inputs, &mut dem, root)?;
            certify(&s, z, &inputs, &mut dem, root)?;
        }
        for &(z, _) in &LEVELS {
            if z == 13 {
                continue;
            }
            let missing:i64=s.db.query_row("SELECT count(*) FROM active a WHERE a.z=? AND NOT EXISTS(SELECT 1 FROM active b WHERE b.z=? AND b.id=a.id)",params![z,z+1],|r|r.get(0))?;
            ensure!(missing == 0, "粗品質の点が細品質に含まれていません: z{z}");
        }
        pack(&s, &job, &inputs, root, &window)?;
        let seams = verify_seams(root)?;
        fs::write(root.join("seams.json"), serde_json::to_vec_pretty(&seams)?)?;
        let saved_audit = audit_saved(root, &inputs, &mut dem)?;
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
        std::env::temp_dir().join(format!(
            "natural-pipeline-{}-{}",
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
}
