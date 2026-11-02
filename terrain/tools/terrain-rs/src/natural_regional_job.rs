//! 元PNGのブロック監査、点の適合と削減。標高をLPFや別曲面へ置換しない。
use crate::{
    natural_codec,
    natural_regional_surface::{Fast, Work},
    now, write, write_bytes,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeMap, BinaryHeap, HashMap, HashSet},
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
pub const WORLD: f64 = 40075016.68557849;
pub const STEP: f64 = WORLD / 4194304.;
// 最詳細の2万点制限とは別に、複数の子タイルを統合する粗曲面の安全上限。
const MAX_HIERARCHY_POINTS: usize = 131072;
pub const LEVELS: &[(u32, f64)] = &[
    (6, 640.),
    (7, 500.),
    (8, 400.),
    (9, 320.),
    (10, 240.),
    (11, 160.),
    (12, 80.),
    (13, 40.),
    (14, 20.),
];
// 正の残差は地形の膨張。非対称ノルムは和の誤差にも三角不等式が成り立つ。
fn directional_error(delta: f64, upward_ratio: f64) -> f64 {
    if delta > 0. { delta / upward_ratio } else { -delta }
}
type Key = (i32, i32);
pub struct Inputs {
    cache: PathBuf,
    states: HashMap<Key, (String, String)>,
}
impl Inputs {
    pub fn load(db: &Path, cache: &Path) -> Result<Self> {
        let db = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut s = db.prepare("SELECT x,y,state,coalesce(sha,'') FROM cells")?;
        let states = s
            .query_map([], |r| Ok(((r.get(0)?, r.get(1)?), (r.get(2)?, r.get(3)?))))?
            .collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok(Self {
            cache: cache.into(),
            states,
        })
    }
    fn read(&self, key: Key) -> Result<Option<Vec<f64>>> {
        let Some((state, sha)) = self.states.get(&key) else {
            return Ok(None);
        };
        if state != "cached" {
            ensure!(
                state == "verified404",
                "入力状態が未確定です: {key:?} {state}"
            );
            return Ok(None);
        }
        let path = self
            .cache
            .join("14")
            .join(key.0.to_string())
            .join(format!("{}.png", key.1));
        let bytes = fs::read(&path)
            .with_context(|| format!("記録済みPNGがありません: {}", path.display()))?;
        if !sha.is_empty() {
            let actual = Sha256::digest(&bytes)
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect::<String>();
            ensure!(
                &actual == sha,
                "PNGが取得記録から変更されています: {}",
                path.display()
            );
        }
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut r = decoder.read_info()?;
        ensure!(
            r.info().width == 256 && r.info().height == 256,
            "PNG寸法が不正です"
        );
        let mut b = vec![0; r.output_buffer_size().context("PNGサイズ不明")?];
        let info = r.next_frame(&mut b)?;
        ensure!(
            info.bit_depth == png::BitDepth::Eight,
            "PNGの深さが不正です"
        );
        let stride = match info.color_type {
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            _ => anyhow::bail!("PNGがRGB/RGBAではありません"),
        };
        Ok(Some(
            b[..info.buffer_size()]
                .chunks_exact(stride)
                .map(|p| {
                    let v = ((p[0] as i32) << 16) | ((p[1] as i32) << 8) | p[2] as i32;
                    if v == 8388608 || stride == 4 && p[3] == 0 {
                        f64::NAN
                    } else {
                        (if v >= 8388608 { v - 16777216 } else { v }) as f64 * 0.01
                    }
                })
                .collect(),
        ))
    }
}
#[derive(Clone)]
struct Block {
    cell_bounds: std::sync::OnceLock<Vec<[f64; 4]>>,
    q: Vec<[f64; 2]>,
    z: Vec<f64>,
    cells: Vec<Vec<usize>>,
}
struct Reference {
    upward_ratio: f64,
    inputs: Arc<Inputs>,
    height_samples: HashMap<Key, (Option<Arc<Vec<f64>>>, u64)>,
    domains: Vec<[f64; 4]>,
    world: [f64; 2],
    lo: [i32; 2],
    hi: [i32; 2],
    keys: Vec<Key>,
    samples: HashMap<Key, (Arc<Block>, u64)>,
    bytes: usize,
    tick: u64,
    residuals: BTreeMap<Key, Vec<f64>>,
    rows: BTreeMap<Key, Row>,
    optimized: bool,
    interval_bounds: bool,
    evaluated: u64,
    certified: u64,
    block_seconds: f64,
}
#[derive(Clone, Copy, Debug)]
struct ErrorCandidate {
    score: f64,
    point: [f64; 3],
}
impl PartialEq for ErrorCandidate {
    fn eq(&self, b: &Self) -> bool {
        self.cmp(b) == Ordering::Equal
    }
}
impl Eq for ErrorCandidate {}
impl PartialOrd for ErrorCandidate {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl Ord for ErrorCandidate {
    fn cmp(&self, b: &Self) -> Ordering {
        self.score
            .total_cmp(&b.score)
            .then(b.point[1].total_cmp(&self.point[1]))
            .then(b.point[0].total_cmp(&self.point[0]))
    }
}
fn sort_candidates(v: &mut [(f64, [f64; 3])]) {
    v.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(a.1[1].total_cmp(&b.1[1]))
            .then(a.1[0].total_cmp(&b.1[0]))
    });
}
#[derive(Clone, Default)]
struct Row {
    max: f64,
    physical_sumsq: f64,
    above: f64,
    below: f64,
    sumsq: f64,
    count: u64,
    candidate: Vec<(f64, [f64; 3])>,
}
struct Audit {
    rows: BTreeMap<Key, Row>,
    residuals: BTreeMap<Key, Vec<f64>>,
}
impl Reference {
    fn new(inputs: Arc<Inputs>, world: [f64; 2], half: f64) -> Self {
        let lo = [
            ((world[0] - half) / STEP - 0.5).ceil() as i32,
            ((world[1] - half) / STEP - 0.5).ceil() as i32,
        ];
        let hi = [
            ((world[0] + half) / STEP - 0.5).floor() as i32,
            ((world[1] + half) / STEP - 0.5).floor() as i32,
        ];
        let keys = (lo[1].div_euclid(256)..=hi[1].div_euclid(256))
            .flat_map(|y| (lo[0].div_euclid(256)..=hi[0].div_euclid(256)).map(move |x| (x, y)))
            .collect();
        Self {
            inputs,
            height_samples: HashMap::new(),
            upward_ratio: 1.,
            domains: Vec::new(),
            world,
            lo,
            hi,
            keys,
            samples: HashMap::new(),
            bytes: 0,
            tick: 0,
            residuals: BTreeMap::new(),
            rows: BTreeMap::new(),
            optimized: true,
            interval_bounds: false,
            evaluated: 0,
            certified: 0,
            block_seconds: 0.,
        }
    }
    fn block(&mut self, key: Key) -> Result<Arc<Block>> {
        self.tick += 1;
        if let Some((v, t)) = self.samples.get_mut(&key) {
            *t = self.tick;
            return Ok(v.clone());
        }
        let mut b = Block {
            cell_bounds: std::sync::OnceLock::new(),
            q: Vec::new(),
            z: Vec::new(),
            cells: vec![Vec::new(); 256],
        };
        if let Some(values) = self.inputs.read(key)? {
            for y in self.lo[1].max(key.1 * 256)..(self.hi[1] + 1).min((key.1 + 1) * 256) {
                for x in self.lo[0].max(key.0 * 256)..(self.hi[0] + 1).min((key.0 + 1) * 256) {
                    let z = values[((y - key.1 * 256) * 256 + x - key.0 * 256) as usize];
                    let global = [(x as f64 + 0.5) * STEP, (y as f64 + 0.5) * STEP];
                    if z.is_finite()
                        && (self.domains.is_empty()
                            || self.domains.iter().any(|&r| inside(global, r)))
                    {
                        b.cells[(y.rem_euclid(256) / 16 * 16 + x.rem_euclid(256) / 16) as usize]
                            .push(b.z.len());
                        b.q.push([
                            (x as f64 + 0.5) * STEP - self.world[0],
                            (y as f64 + 0.5) * STEP - self.world[1],
                        ]);
                        b.z.push(z);
                    }
                }
            }
        }
        let size = b.z.len() * 32;
        while self.bytes + size > 128 * 1024 * 1024 || self.samples.len() >= 256 {
            let Some(k) = self.samples.iter().min_by_key(|v| v.1.1).map(|v| *v.0) else {
                break;
            };
            let old = self.samples.remove(&k).unwrap();
            self.bytes -= old.0.z.len() * 32;
        }
        let b = Arc::new(b);
        self.samples.insert(key, (b.clone(), self.tick));
        self.bytes += size;
        Ok(b)
    }
    // 評価位置の局所DEM参照を一度だけ作る。格子点上では元画素値そのもの。
    fn height_at(&mut self, global: [f64; 2]) -> Result<Option<f64>> {
        // 大きな世界座標との往復誤差で、元画素を微小な画素間位置と誤認しない。
        let snap=|v:f64| if (v-v.round()).abs()<1e-7 {v.round()} else {v};
        let gx=snap(global[0]/STEP-0.5);let gy=snap(global[1]/STEP-0.5);
        let x0=gx.floor() as i32;let y0=gy.floor() as i32;
        let tx=gx-x0 as f64;let ty=gy-y0 as f64;let mut value=0.;
        for (dy,wy) in [(0,1.-ty),(1,ty)] { for (dx,wx) in [(0,1.-tx),(1,tx)] {
            let weight=wx*wy;if weight<=1e-12 {continue;}
            let x=x0+dx;let y=y0+dy;let key=(x.div_euclid(256),y.div_euclid(256));
            let world=[(x as f64+0.5)*STEP,(y as f64+0.5)*STEP];
            if x<self.lo[0] || y<self.lo[1] || x>self.hi[0] || y>self.hi[1]
                || (!self.domains.is_empty() && !self.domains.iter().any(|&d|inside(world,d))) { return Ok(None); }
            self.tick+=1;
            if !self.height_samples.contains_key(&key) {
                // 局所参照にはXYZ一覧やセル索引は不要。PNG復号値だけを512 MiBまで保持。
                if self.height_samples.len()>=1024 {
                    let old=*self.height_samples.iter().min_by_key(|(_,v)|v.1).unwrap().0;
                    self.height_samples.remove(&old);
                }
                let values=self.inputs.read(key)?.map(Arc::new);
                self.height_samples.insert(key,(values,self.tick));
            }
            let (values,tick)=self.height_samples.get_mut(&key).unwrap();*tick=self.tick;
            let Some(values)=values else {return Ok(None);};
            let z=values[(y.rem_euclid(256)*256+x.rem_euclid(256)) as usize];
            if !z.is_finite() {return Ok(None);}
            value+=weight*z;
        }}
        Ok(Some(value))
    }
    // 削除位置に最も近い元DEM画素を先に直接検査する。
    // この画素も通常監査の対象内なので、超過時だけ安全に早期棄却できる。
    fn rejects_near(&mut self, m: &Fast, q: [f64; 2], rect: [f64; 4], limit: f64) -> Result<bool> {
        let gx = ((q[0]+self.world[0])/STEP).floor() as i32;
        let gy = ((q[1]+self.world[1])/STEP).floor() as i32;
        let key = (gx.div_euclid(256),gy.div_euclid(256));
        if !self.keys.contains(&key) { return Ok(false); }
        let block = self.block(key)?;
        let cell = (gy.rem_euclid(256)/16*16+gx.rem_euclid(256)/16) as usize;
        let Some(ids) = block.cells.get(cell) else { return Ok(false); };
        let nearest = ids.iter().copied().filter(|&i|inside(block.q[i],rect))
            .min_by(|&a,&b| {
                let d = |i:usize| (block.q[i][0]-q[0]).powi(2)+(block.q[i][1]-q[1]).powi(2);
                d(a).total_cmp(&d(b))
            });
        let Some(i) = nearest else { return Ok(false); };
        self.evaluated += 1;
        Ok(directional_error(m.evaluate(block.q[i],&mut Work::default())?-block.z[i],self.upward_ratio)>limit)
    }
    fn rect(&self, key: Key) -> [f64; 4] {
        [
            key.0 as f64 * 256. * STEP - self.world[0],
            key.1 as f64 * 256. * STEP - self.world[1],
            (key.0 + 1) as f64 * 256. * STEP - self.world[0],
            (key.1 + 1) as f64 * 256. * STEP - self.world[1],
        ]
    }
    fn audit(
        &mut self,
        m: &Fast,
        region: Option<[f64; 4]>,
        limit: Option<f64>,
        error: f64,
        direct: bool,
        progress: &mut dyn FnMut(usize, usize, u64, u64) -> Result<()>,
    ) -> Result<Option<Audit>> {
        let keys = self
            .keys
            .iter()
            .copied()
            .filter(|&k| {
                region.is_none_or(|b| intersects(b, self.rect(k)))
                    && self
                        .rows
                        .get(&k)
                        .is_none_or(|r| r.count > 0 || region.is_none())
            })
            .collect::<Vec<_>>();
        let mut rows = BTreeMap::new();
        let mut residuals = BTreeMap::new();
        let mut residual_bytes = 0;
        let mut w = Work::default();
        for (done, key) in keys.iter().enumerate() {
            let read_start = Instant::now();
            let block = self.block(*key)?;
            self.block_seconds += read_start.elapsed().as_secs_f64();
            let old = if region.is_some() {
                self.residuals.get(key)
            } else {
                None
            };
            // 棄却試行では大きな残差配列のコピーと範囲外の全画素走査をしない。
            // 受理できた試行だけ既存配列へ局所差分を適用する。
            let mut checked_updates = None;
            if let (Some(_), Some(rect), Some(threshold)) = (old, region, limit) {
                let bounds = block.cell_bounds.get_or_init(|| block.cells.iter().map(|ids| {
                    ids.iter().fold([f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY],|r,&i| {
                        let q=block.q[i]; [r[0].min(q[0]),r[1].min(q[1]),r[2].max(q[0]),r[3].max(q[1])]
                    })
                }).collect());
                let mut updates = Vec::new();
                for (cell,bounds) in block.cells.iter().zip(bounds) {
                    if cell.is_empty() || !intersects(*bounds,rect) { continue; }
                    for &i in cell {
                        if !inside(block.q[i],rect) { continue; }
                        let e = directional_error(m.evaluate(block.q[i], &mut w)?-block.z[i],self.upward_ratio);
                        self.evaluated += 1;
                        if e > threshold { return Ok(None); }
                        updates.push((i,e));
                    }
                }
                checked_updates = Some(updates);
            }
            let mut e = old.cloned().unwrap_or_else(|| vec![0.; block.z.len()]);
            if let Some(updates) = &checked_updates { for &(i,value) in updates { e[i] = value; } }

            let mut row = Row::default();
            let mut top = BinaryHeap::<Reverse<ErrorCandidate>>::new();
            let mut exact = true;
            // 誤差の証明は追加中だけ。最終監査と削除の採否は全画素を直接評価する。
            let bound_enabled = self.optimized
                && self.upward_ratio == 1.
                && self.interval_bounds
                && !direct
                && limit.is_none()
                && old.is_none();
            for cell in &block.cells {
                if cell.is_empty() {
                    continue;
                }
                if bound_enabled {
                    let threshold = if top.len() == 128 {
                        top.peek().unwrap().0.score.max(error)
                    } else {
                        error
                    };
                    let rect = cell.iter().fold(
                        [
                            f64::INFINITY,
                            f64::INFINITY,
                            f64::NEG_INFINITY,
                            f64::NEG_INFINITY,
                        ],
                        |a, &i| {
                            [
                                a[0].min(block.q[i][0]),
                                a[1].min(block.q[i][1]),
                                a[2].max(block.q[i][0]),
                                a[3].max(block.q[i][1]),
                            ]
                        },
                    );
                    let bound =
                        m.affine_error_bound(rect, cell.iter().map(|&i| (block.q[i], block.z[i])))?;
                    if bound <= threshold {
                        row.max = row.max.max(bound);
                        row.sumsq += bound * bound * cell.len() as f64;
                        row.count += cell.len() as u64;
                        self.certified += cell.len() as u64;
                        exact = false;
                        continue;
                    }
                }
                for &i in cell {
                    if checked_updates.is_none() && (old.is_none() || region.is_none_or(|b| inside(block.q[i], b))) {
                        let delta = m.evaluate(block.q[i], &mut w)? - block.z[i];
                        row.physical_sumsq += delta * delta;
                        row.above = row.above.max(delta);
                        row.below = row.below.max(-delta);
                        e[i] = directional_error(delta, self.upward_ratio);
                        self.evaluated += 1;
                    }
                    // 棄却試行は1画素でも誤差超過した時点で終了できる。
                    if self.optimized && limit.is_some_and(|limit| e[i] > limit) {
                        return Ok(None);
                    }
                    row.max = row.max.max(e[i]);
                    row.sumsq += e[i] * e[i];
                    row.count += 1;
                    if e[i] > error {
                        let candidate = ErrorCandidate {
                            score: e[i],
                            point: [block.q[i][0], block.q[i][1], block.z[i]],
                        };
                        if self.optimized {
                            if top.len() < 128 {
                                top.push(Reverse(candidate));
                            } else if candidate > top.peek().unwrap().0 {
                                *top.peek_mut().unwrap() = Reverse(candidate);
                            }
                        } else {
                            row.candidate.push((candidate.score, candidate.point));
                            if row.candidate.len() >= 256 {
                                sort_candidates(&mut row.candidate);
                                row.candidate.truncate(128);
                            }
                        }
                    }
                }
            }
            if self.optimized {
                row.candidate = top.into_iter().map(|v| (v.0.score, v.0.point)).collect();
            }
            sort_candidates(&mut row.candidate);
            row.candidate.truncate(128);
            if limit.is_some_and(|limit| row.max > limit) {
                return Ok(None);
            }
            if exact && (region.is_some() || limit.is_some()) {
                while residual_bytes + e.len() * 8 > 32 * 1024 * 1024 && !residuals.is_empty() {
                    let first = *residuals.keys().next().unwrap();
                    let old: Vec<f64> = residuals.remove(&first).unwrap();
                    residual_bytes -= old.len() * 8;
                }
                residual_bytes += e.len() * 8;
                residuals.insert(*key, e);
            }
            rows.insert(*key, row);
            if done % 16 == 0 || done + 1 == keys.len() {
                progress(done + 1, keys.len(), self.evaluated, self.certified)?;
            }
        }
        Ok(Some(Audit { rows, residuals }))
    }
    fn commit(&mut self, a: Audit) {
        for (k, v) in a.rows {
            self.rows.insert(k, v);
            self.residuals.remove(&k);
        }
        for (k, v) in a.residuals {
            self.residuals.insert(k, v);
        }
        let mut size = self.residuals.values().map(|v| v.len() * 8).sum::<usize>();
        while size > 32 * 1024 * 1024 {
            let k = *self.residuals.keys().next().unwrap();
            size -= self.residuals.remove(&k).unwrap().len() * 8;
        }
    }
    fn summary(&self) -> (f64, f64, u64) {
        let count = self.rows.values().map(|r| r.count).sum();
        let max = self.rows.values().map(|r| r.max).fold(0., f64::max);
        let rmse = if count > 0 {
            (self.rows.values().map(|r| r.sumsq).sum::<f64>() / count as f64).sqrt()
        } else {
            0.
        };
        (max, rmse, count)
    }
}
fn intersects(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[2] && a[2] >= b[0] && a[1] <= b[3] && a[3] >= b[1]
}
fn inside(p: [f64; 2], b: [f64; 4]) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
}
#[derive(Clone, Copy)]
struct Rank {
    score: f64,
    id: usize,
    version: u64,
}
impl PartialEq for Rank {
    fn eq(&self, b: &Self) -> bool {
        self.id == b.id && self.score == b.score && self.version == b.version
    }
}
impl Eq for Rank {}
impl PartialOrd for Rank {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl Ord for Rank {
    fn cmp(&self, b: &Self) -> Ordering {
        b.score
            .total_cmp(&self.score)
            .then(b.id.cmp(&self.id))
            .then(b.version.cmp(&self.version))
    }
}
pub struct Job {
    root: PathBuf,
    z: u32,
    x: u32,
    y: u32,
    error: f64,
    side: f64,
    world: [f64; 2],
    folder: PathBuf,
    reference: Reference,
    started: Instant,
    last: Instant,
}
impl Job {
    pub fn folder(root: &Path, z: u32, x: u32, y: u32) -> PathBuf {
        root.join("tiles")
            .join(z.to_string())
            .join(x.to_string())
            .join(y.to_string())
    }
    pub fn new(root: &Path, z: u32, x: u32, y: u32, inputs: Arc<Inputs>) -> Result<Self> {
        let mut error = LEVELS.iter().find(|v| v.0 == z).context("不正な品質")?.1;
        let side = WORLD / (1u64 << z) as f64;
        let world = [(x as f64 + 0.5) * side, (y as f64 + 0.5) * side];
        let folder = Self::folder(root, z, x, y);
        fs::create_dir_all(&folder)?;
        let mut reference = Reference::new(inputs, world, side / 2. + 250.);
        if root.join("settings.json").exists() {
            let cfg: Value = serde_json::from_slice(&fs::read(root.join("settings.json"))?)?;
            reference.upward_ratio = cfg["upwardErrorRatio"].as_f64().unwrap_or(1.);
            ensure!(reference.upward_ratio > 0. && reference.upward_ratio <= 1., "上方向誤差比が不正です");
            if let Some(levels) = cfg["levels"].as_array() {
                error = levels.iter().find(|v| v["z"].as_u64()==Some(z as u64)).context("設定にないレベルです")?["error_m"].as_f64().context("許容誤差が不正です")?;
            }
            if let Some(domains) = cfg.get("domainBounds") {
                reference.domains = serde_json::from_value(domains.clone())?;
                let d = reference.domains.clone();
                reference.keys.retain(|&(x, y)| {
                    let r = [
                        x as f64 * 256. * STEP,
                        y as f64 * 256. * STEP,
                        (x + 1) as f64 * 256. * STEP,
                        (y + 1) as f64 * 256. * STEP,
                    ];
                    d.iter().any(|&d| intersects(r, d))
                });
            }
        }
        Ok(Self {
            root: root.into(),
            z,
            x,
            y,
            error,
            side,
            world,
            folder,
            reference,
            started: Instant::now(),
            last: Instant::now() - std::time::Duration::from_secs(5),
        })
    }
    fn status(&mut self, stage: &str, points: usize, extra: Value) -> Result<()> {
        if self.last.elapsed().as_millis() < 500 {
            return Ok(());
        }
        self.last = Instant::now();
        let mut v = json!({"engine":"Rust","stage":stage,"z":self.z,"x":self.x,"y":self.y,"error":self.error,"fineError":self.initial_fit_error()?,"points":points.saturating_sub(4),"totalSeconds":self.started.elapsed().as_secs_f64(),"updated":now()});
        if let Some(e) = extra.as_object() {
            for (k, vv) in e {
                v[k] = vv.clone();
            }
        }
        write(&self.folder.join("progress.json"), &v)
    }
    fn scan(
        &mut self,
        m: &Fast,
        rect: Option<[f64; 4]>,
        limit: Option<f64>,
        error: f64,
        stage: &str,
    ) -> Result<Option<Audit>> {
        let folder = self.folder.clone();
        let (z, x, y, e) = (self.z, self.x, self.y, self.error);
        let points = m.base.points.len() - 4;
        let start = self.started;
        let mut last = Instant::now() - std::time::Duration::from_secs(3);
        self.reference.audit(m,rect,limit,error,stage!="rust-fine-fit",&mut |done,total,evaluated,certified|{if last.elapsed().as_millis()>500||done==total{last=Instant::now();write(&folder.join("progress.json"),&json!({"engine":"Rust","stage":stage,"z":z,"x":x,"y":y,"error":e,"points":points,"blocksDone":done,"blocksTotal":total,"evaluatedPixels":evaluated,"certifiedPixels":certified,"totalSeconds":start.elapsed().as_secs_f64()}))?;}Ok(())})
    }
    fn coverage(&self) -> Value {
        let mut recorded = 0;
        let mut cached = 0;
        let mut missing = 0;
        for k in &self.reference.keys {
            if let Some((s, _)) = self.reference.inputs.states.get(k) {
                recorded += 1;
                if s == "cached" {
                    cached += 1;
                }
                if s == "verified404" {
                    missing += 1;
                }
            }
        }
        json!({"recorded":recorded,"expected":self.reference.keys.len(),"cached":cached,"verified404":missing,"unrecordedOutsideVerifiedDomain":self.reference.keys.len()-recorded})
    }
    fn seed(&mut self) -> Result<Vec<[f64; 3]>> {
        let corners = [
            [-self.side / 2., -self.side / 2.],
            [self.side / 2., -self.side / 2.],
            [-self.side / 2., self.side / 2.],
            [self.side / 2., self.side / 2.],
        ];
        let mut best = [f64::INFINITY; 4];
        let mut p = [[0.; 3]; 4];
        let keys = self.reference.keys.clone();
        for (done, k) in keys.iter().enumerate() {
            let block = self.reference.block(*k)?;
            for (xy, &z) in block.q.iter().zip(&block.z) {
                for i in 0..4 {
                    let d = (xy[0] - corners[i][0]).powi(2) + (xy[1] - corners[i][1]).powi(2);
                    if d < best[i] {
                        best[i] = d;
                        p[i] = [xy[0], xy[1], z];
                    }
                }
            }
            if done % 64 == 0 {
                self.status(
                    "rust-seed",
                    4,
                    json!({"blocksDone":done,"blocksTotal":keys.len()}),
                )?;
            }
        }
        if !best[0].is_finite() {
            return Ok(Vec::new());
        }
        let mut out = vec![
            [-4. * self.side, -4. * self.side, 0.],
            [4. * self.side, -4. * self.side, 0.],
            [-4. * self.side, 4. * self.side, 0.],
            [4. * self.side, 4. * self.side, 0.],
        ];
        let mut known = HashSet::new();
        for p in p {
            let p = self.quantize(p);
            if known.insert((p[0].to_bits(), p[1].to_bits())) {
                out.push(p);
            }
        }
        Ok(out)
    }
    fn quantize(&self, p: [f64; 3]) -> [f64; 3] {
        [
            ((p[0] + self.world[0]) / 0.0625).round_ties_even() * 0.0625 - self.world[0],
            ((p[1] + self.world[1]) / 0.0625).round_ties_even() * 0.0625 - self.world[1],
            (p[2] / 0.125).round_ties_even() * 0.125,
        ]
    }
    fn save(&self, stage: u32, p: &[[f64; 3]], active: &[bool]) -> Result<()> {
        let mut b = b"RXT1".to_vec();
        b.extend(stage.to_le_bytes());
        b.extend((p.len() as u32).to_le_bytes());
        for p in p {
            for v in p {
                b.extend(v.to_le_bytes());
            }
        }
        b.extend(active.iter().map(|&v| u8::from(v)));
        let digest = Sha256::digest(&b);
        b.extend(digest);
        write_bytes(&self.folder.join("rust-state.bin"), &b)
    }
    fn restore(&self) -> Result<Option<(u32, Vec<[f64; 3]>, Vec<bool>)>> {
        let path = self.folder.join("rust-state.bin");
        if !path.exists() {
            return Ok(None);
        }
        let b = fs::read(path)?;
        ensure!(
            b.len() >= 44 && &b[..4] == b"RXT1",
            "再開データの形式が不正です"
        );
        let stage = u32::from_le_bytes(b[4..8].try_into()?);
        let n = u32::from_le_bytes(b[8..12].try_into()?) as usize;
        ensure!(
            stage <= 2 && n >= 4 && n <= MAX_HIERARCHY_POINTS + 4 && b.len() == 12 + n * 25 + 32,
            "再開データの点数・長さが不正です"
        );
        ensure!(
            Sha256::digest(&b[..b.len() - 32]).as_slice() == &b[b.len() - 32..],
            "再開データのSHAが不一致です"
        );
        let p = b[12..12 + 24 * n]
            .chunks_exact(24)
            .map(|r| {
                [
                    f64::from_le_bytes(r[..8].try_into().unwrap()),
                    f64::from_le_bytes(r[8..16].try_into().unwrap()),
                    f64::from_le_bytes(r[16..24].try_into().unwrap()),
                ]
            })
            .collect();
        let active = b[12 + 24 * n..12 + 25 * n]
            .iter()
            .map(|&v| v != 0)
            .collect::<Vec<_>>();
        ensure!(active[..4].iter().all(|&v| v), "外周ガードが無効です");
        Ok(Some((stage, p, active)))
    }
    fn initial_fit_error(&self) -> Result<f64> {
        let cfg:Value=serde_json::from_slice(&fs::read(self.root.join("settings.json"))?)?;
        Ok(if cfg["fineFitFullBudget"].as_bool()==Some(true) {self.error} else {self.error/2.})
    }
    fn fit(&mut self, mut p: Vec<[f64; 3]>) -> Result<Vec<[f64; 3]>> {
        let mut rect = None;
        let error = self.initial_fit_error()?;
        let mut full = false;
        for iteration in 0..20000 {
            self.status("rust-fine-fit", p.len(), json!({"iteration":iteration}))?;
            let model = Fast::new(p.clone())?;
            let audit = self
                .scan(
                    &model,
                    rect,
                    None,
                    error,
                    if full {
                        "rust-fit-final-audit"
                    } else {
                        "rust-fine-fit"
                    },
                )?
                .unwrap();
            self.reference.commit(audit);
            let (max, _, valid) = self.reference.summary();
            ensure!(valid > 0, "有効DEMがありません");
            if max <= error {
                if full {
                    for v in &mut p {
                        v[0] = ((v[0] + self.world[0]) / 0.0625 + 0.5).floor() * 0.0625
                            - self.world[0];
                        v[1] = ((v[1] + self.world[1]) / 0.0625 + 0.5).floor() * 0.0625
                            - self.world[1];
                        v[2] = (v[2] / 0.125 + 0.5).floor() * 0.125;
                    }
                    return Ok(p);
                }
                rect = None;
                full = true;
                continue;
            }
            full = false;
            let known = p
                .iter()
                .map(|p| {
                    (
                        ((p[0] + self.world[0]) / 0.0625).round() as i64,
                        ((p[1] + self.world[1]) / 0.0625).round() as i64,
                    )
                })
                .collect::<HashSet<_>>();
            let mut candidates = self
                .reference
                .rows
                .values()
                .flat_map(|r| r.candidate.iter().copied())
                .collect::<Vec<_>>();
            candidates.sort_by(|a, b| {
                b.0.total_cmp(&a.0)
                    .then(a.1[1].total_cmp(&b.1[1]))
                    .then(a.1[0].total_cmp(&b.1[0]))
            });
            let next = candidates.iter().map(|v| self.quantize(v.1)).find(|v| {
                !known.contains(&(
                    ((v[0] + self.world[0]) / 0.0625).round() as i64,
                    ((v[1] + self.world[1]) / 0.0625).round() as i64,
                ))
            });
            let next = if let Some(v) = next {
                v
            } else {
                let mut support = None;
                'outer: for (_, q) in candidates.iter().take(128) {
                    let ix = ((q[0] + self.world[0]) / STEP).floor() as i32;
                    let iy = ((q[1] + self.world[1]) / STEP).floor() as i32;
                    for r in [1, 2, 4] {
                        for (dx, dy) in [
                            (1, 0),
                            (-1, 0),
                            (0, 1),
                            (0, -1),
                            (1, 1),
                            (1, -1),
                            (-1, 1),
                            (-1, -1),
                        ] {
                            let (x, y) = (ix + dx * r, iy + dy * r);
                            let key = (x.div_euclid(256), y.div_euclid(256));
                            if let Some(raw) = self.reference.inputs.read(key)? {
                                let z = raw[(y.rem_euclid(256) * 256 + x.rem_euclid(256)) as usize];
                                if z.is_finite() {
                                    let v = self.quantize([
                                        (x as f64 + 0.5) * STEP - self.world[0],
                                        (y as f64 + 0.5) * STEP - self.world[1],
                                        z,
                                    ]);
                                    if inside(
                                        [v[0], v[1]],
                                        [
                                            -self.side / 2. - 500.,
                                            -self.side / 2. - 500.,
                                            self.side / 2. + 500.,
                                            self.side / 2. + 500.,
                                        ],
                                    ) && !known.contains(&(
                                        ((v[0] + self.world[0]) / 0.0625).round() as i64,
                                        ((v[1] + self.world[1]) / 0.0625).round() as i64,
                                    )) {
                                        support = Some(v);
                                        break 'outer;
                                    }
                                }
                            }
                        }
                    }
                }
                support.context("誤差超過に対する未登録支持点がありません")?
            };
            rect = Some(model.insertion_box([next[0], next[1]]));
            p.push(next);
            ensure!(p.len() <= 20004, "安全上限20000点を超えました");
            self.save(0, &p, &vec![true; p.len()])?;
        }
        anyhow::bail!("適合反復上限に到達しました")
    }
    fn enqueue(
        &mut self,
        p: &[[f64; 3]],
        active: &[bool],
        fixed: &[bool],
        model: &Fast,
        ids: &[usize],
        versions: &mut [u64],
        heap: &mut BinaryHeap<Rank>,
    ) -> Result<()> {
        let current = active
            .iter()
            .enumerate()
            .filter_map(|(i, &a)| a.then_some(i))
            .collect::<Vec<_>>();
        let mut index = vec![usize::MAX; p.len()];
        for (i, &k) in current.iter().enumerate() {
            index[k] = i;
        }
        for &k in ids {
            versions[k] += 1;
            if !active[k] || fixed[k] {
                continue;
            }
            let i = index[k];
            let neighbors = &model.base.neighbors[i];
            let q = [p[k][0], p[k][1]];
            let mut value = None;
            if neighbors.len() >= 3 {
                if let Ok(local) =
                    Fast::new(neighbors.iter().map(|&j| model.base.points[j]).collect())
                {
                    if let Ok(v) = local.evaluate(q, &mut Work::default()) {
                        value = Some(v);
                    }
                }
            }
            let v = if let Some(v) = value {
                v
            } else {
                Fast::new(
                    current
                        .iter()
                        .copied()
                        .filter(|&id| id != k)
                        .map(|id| p[id])
                        .collect(),
                )?
                .evaluate(q, &mut Work::default())?
            };
            heap.push(Rank {
                score: (v - p[k][2]).abs(),
                id: k,
                version: versions[k],
            });
        }
        Ok(())
    }
    fn prune(&mut self, p: &[[f64; 3]], mut active: Vec<bool>) -> Result<(Vec<bool>, Fast, Value)> {
        let fixed = p
            .iter()
            .enumerate()
            .map(|(i, p)| {
                i < 4 || p[0].abs() >= self.side / 2. - STEP || p[1].abs() >= self.side / 2. - STEP
            })
            .collect::<Vec<_>>();
        let mut model = Fast::new(
            p.iter()
                .zip(&active)
                .filter_map(|(&p, &a)| a.then_some(p))
                .collect(),
        )?;
        let a = self
            .scan(
                &model,
                None,
                Some(self.error),
                self.error,
                "rust-prune-initial-audit",
            )?
            .context("削減元が許容誤差を超えています")?;
        self.reference.commit(a);
        let mut heap = BinaryHeap::new();
        let mut versions = vec![0; p.len()];
        let ids = (0..p.len()).collect::<Vec<_>>();
        self.status("rust-rank", p.len(), json!({}))?;
        self.enqueue(p, &active, &fixed, &model, &ids, &mut versions, &mut heap)?;
        let mut final_pass = false;
        let mut attempted = 0;
        loop {
            if heap.is_empty() {
                if final_pass {
                    break;
                }
                final_pass = true;
                self.enqueue(p, &active, &fixed, &model, &ids, &mut versions, &mut heap)?;
                continue;
            }
            let r = heap.pop().unwrap();
            if !active[r.id] || r.version != versions[r.id] {
                continue;
            }
            let current = active
                .iter()
                .enumerate()
                .filter_map(|(i, &a)| a.then_some(i))
                .collect::<Vec<_>>();
            let i = current.iter().position(|&id| id == r.id).unwrap();
            let box_ = model.deletion_box(i, STEP * 2.);
            let trial = model.without_point(i)?;
            attempted += 1;
            if self.reference.rejects_near(&trial, [p[r.id][0],p[r.id][1]], box_, self.error)? {
                continue;
            }
            if let Some(a) = self.scan(
                &trial,
                Some(box_),
                Some(self.error),
                self.error,
                "rust-prune-local-audit",
            )? {
                let mut near = HashSet::from([i]);
                for _ in 0..2 {
                    let old = near.iter().copied().collect::<Vec<_>>();
                    for j in old {
                        near.extend(model.base.neighbors[j].iter().copied());
                    }
                }
                let mut rerank = near.into_iter().map(|j| current[j]).collect::<Vec<_>>();
                rerank.sort_unstable();
                active[r.id] = false;
                model = trial;
                self.reference.commit(a);
                final_pass = false;
                self.enqueue(
                    p,
                    &active,
                    &fixed,
                    &model,
                    &rerank,
                    &mut versions,
                    &mut heap,
                )?;
                self.save(1, p, &active)?;
            }
            let (max, rmse, valid) = self.reference.summary();
            self.status("rust-prune",active.iter().filter(|&&v|v).count(),json!({"attempted":attempted,"removed":active.iter().filter(|&&v|!v).count(),"max":max,"rmse":rmse,"validPixels":valid}))?;
        }
        let a = self
            .scan(
                &model,
                None,
                Some(self.error),
                self.error,
                "rust-final-full-audit",
            )?
            .context("最終全画素監査で誤差超過")?;
        self.reference.commit(a);
        let (max, rmse, valid) = self.reference.summary();
        let audit = json!({"max":max,"rmse":rmse,"validPixels":valid,"initialPoints":p.len()-4,"finalPoints":active.iter().filter(|&&v|v).count()-4,"fixedPoints":fixed.iter().filter(|&&v|v).count()-4,"removed":active.iter().filter(|&&v|!v).count(),"attempted":attempted,"noFeasibleSingleRemoval":true,"allDemPixelsAudited":true,"quantizedControls":true});
        Ok((active, model, audit))
    }
    pub fn run(&mut self) -> Result<Value> {
        if self.folder.join("selection.json").exists() {
            return Ok(serde_json::from_slice(&fs::read(
                self.folder.join("selection.json"),
            )?)?);
        }
        let cfg: Value = serde_json::from_slice(&fs::read(self.root.join("settings.json"))?)?;
        let finest = cfg["levels"].as_array().context("レベル定義がありません")?.iter().filter_map(|v|v["z"].as_u64()).max().context("レベル定義が空です")?;
        if self.z < finest as u32 && cfg["generationMode"].as_str() == Some("fine-to-coarse") {
            return self.run_hierarchy();
        }
        let (stage, p, mask) = if let Some(v) = self.restore()? {
            v
        } else {
            let p = self.seed()?;
            if p.is_empty() {
                ensure!(
                    self.coverage()["unrecordedOutsideVerifiedDomain"].as_u64() == Some(0),
                    "有効DEMなし・未確認入力あり。海とは判定しない"
                );
                let audit = json!({"empty":true,"verifiedNoData":true,"finalPoints":0,"max":0,"allDemPixelsAudited":true,"z":self.z,"x":self.x,"y":self.y});
                write(&self.folder.join("packet.json"), &audit)?;
                write(&self.folder.join("selection.json"), &audit)?;
                return Ok(audit);
            }
            let mask = vec![true; p.len()];
            (0, p, mask)
        };
        let (p, mask) = if stage == 0 {
            let p = self.fit(p)?;
            let mask = vec![true; p.len()];
            self.save(1, &p, &mask)?;
            (p, mask)
        } else {
            (p, mask)
        };
        let (mask, model, audit) = self.prune(&p, mask)?;
        self.save(2, &p, &mask)?;
        self.finish(model, audit)
    }
    fn finish(&mut self, model: Fast, mut audit: Value) -> Result<Value> {
        if self.reference.upward_ratio != 1. {
            let above = self.reference.rows.values().map(|r| r.above).fold(0., f64::max);
            let below = self.reference.rows.values().map(|r| r.below).fold(0., f64::max);
            audit["weightedMax"] = audit["max"].clone();
            audit["max"] = json!(above.max(below));
            audit["maxAboveDEM"] = json!(above);
            audit["maxBelowDEM"] = json!(below);
            audit["upwardErrorRatio"] = json!(self.reference.upward_ratio);
            audit["upwardErrorTarget"] = json!(self.error * self.reference.upward_ratio);
            audit["weightedRmse"] = audit["rmse"].clone();
            let count = self.reference.rows.values().map(|r| r.count).sum::<u64>();
            let sumsq = self.reference.rows.values().map(|r| r.physical_sumsq).sum::<f64>();
            audit["rmse"] = json!(if count > 0 { (sumsq / count as f64).sqrt() } else { 0. });
        }

        let points = model.base.points.clone();
        self.status("rust-C1-and-GPU-pack", points.len(), json!({}))?;
        let half = self.side / 2. + 250.;
        let bounds = [-half, -half, half, half];
        let gradient = crate::natural_regional_gpu::gradient(&model, bounds)?;
        let core = [
            self.x as f64 * self.side,
            self.y as f64 * self.side,
            (self.x + 1) as f64 * self.side,
            (self.y + 1) as f64 * self.side,
        ];
        let meta = json!({"region":"xyz","error":self.error,"z":self.z,"x":self.x,"y":self.y,"side":self.side,"origin":self.world,"coreBounds":core,"worldFieldBounds":[core[0]-250.,core[1]-250.,core[2]+250.,core[3]+250.],"halo":500,"lpf":false,"curve":"XYZ-Sibson-error-tested-pruning","generator":"Rust","sourceGeometryBytes":points.len()*12,"allDemPixelsAudited":true,"auditedMaxError":audit["max"],"auditedErrorTarget":self.error,"maxErrorGuaranteed":false,"auditScope":"all valid available source pixel centres; excludes C1/render/blending and unrecorded source domain","sourceCoverage":self.coverage()});
        let mut meta = meta;
        meta["upwardErrorRatio"] = json!(self.reference.upward_ratio);
        meta["auditedUpwardErrorTarget"] = json!(self.error * self.reference.upward_ratio);
        if self.reference.upward_ratio != 1. {
            meta["auditedMaxAboveDEM"] = audit["maxAboveDEM"].clone();
            meta["auditedMaxBelowDEM"] = audit["maxBelowDEM"].clone();
            meta["auditedWeightedMaxError"] = audit["weightedMax"].clone();
        }
        let cfg: Value = serde_json::from_slice(&fs::read(self.root.join("settings.json"))?)?;
        if let Some(domains) = cfg.get("domainBounds") {
            meta["validDomainBounds"] = domains.clone();
            meta["generationMode"] = cfg["generationMode"].clone();
        }
        let (packet, mut info) =
            crate::natural_regional_gpu::packet(&model, &gradient, bounds, meta)?;
        let origin = [
            (core[0] / 0.0625 + 0.5).floor() as i32,
            (core[1] / 0.0625 + 0.5).floor() as i32,
        ];
        let integers = points
            .iter()
            .map(|p| {
                let x = ((p[0] + self.world[0]) / 0.0625 + 0.5).floor() as i64 - origin[0] as i64;
                let y = ((p[1] + self.world[1]) / 0.0625 + 0.5).floor() as i64 - origin[1] as i64;
                ensure!(
                    i32::try_from(x).is_ok() && i32::try_from(y).is_ok(),
                    "局所整数座標がint32範囲外です"
                );
                Ok([x as i32, y as i32, (p[2] / 0.125 + 0.5).floor() as i32])
            })
            .collect::<Result<Vec<_>>>()?;
        let blob = natural_codec::independent(&integers)?;
        let expected = integers
            .iter()
            .map(|p| [p[0] + origin[0], p[1] + origin[1], p[2]])
            .collect::<Vec<_>>();
        ensure!(
            natural_codec::decode_global(&blob, origin, &[])? == expected,
            "Rice往復結果が不一致です"
        );
        audit["z"] = json!(self.z);
        audit["x"] = json!(self.x);
        audit["y"] = json!(self.y);
        audit["errorTarget"] = json!(self.error);
        audit["fineError"] = json!(self.initial_fit_error()?);
        audit["sourceCoverage"] = self.coverage();
        audit["seconds"] = json!(self.started.elapsed().as_secs_f64());
        audit["riceBytes"] = json!(blob.len());
        audit["gpuBytes"] = json!(packet.len());
        audit["engine"] = json!("Rust");
        info["pruningAudit"] = audit.clone();
        write_bytes(&self.folder.join("packet.bin"), &packet)?;
        write_bytes(&self.folder.join("control-xyz.nqc1"), &blob)?;
        write(
            &self.folder.join("control-gradient.json"),
            &json!({"points":points,"gradient":gradient}),
        )?;
        write(&self.folder.join("packet.json"), &info)?;
        write(&self.folder.join("selection.json"), &audit)?;
        let mut progress = audit.clone();
        progress["stage"] = json!("complete");
        write(&self.folder.join("progress.json"), &progress)?;
        Ok(audit)
    }
}
/// 稼働中の生成器に触れず、同じ点集合・元PNGで監査の旧経路と高速経路を比較する。
pub fn benchmark(input: &Value, inputs: Arc<Inputs>) -> Result<Value> {
    let points: Vec<[f64; 3]> = serde_json::from_value(input["points"].clone())?;
    let world: [f64; 2] = serde_json::from_value(input["world"].clone())?;
    let half = input["half"].as_f64().context("halfがありません")?;
    let error = input["error"].as_f64().context("errorがありません")?;
    let model = Fast::new(points)?;
    let mut reports = Vec::new();
    let mut expected = None;
    for optimized in [false, true] {
        let mut reference = Reference::new(inputs.clone(), world, half);
        reference.optimized = optimized;
        reference.interval_bounds = input["interval"].as_bool().unwrap_or(false);
        if let Some(keys) = input.get("keys") {
            let keys: Vec<[i32; 2]> = serde_json::from_value(keys.clone())?;
            let keys = keys.into_iter().map(|v| (v[0], v[1])).collect::<Vec<_>>();
            ensure!(
                keys.iter().all(|k| reference.keys.contains(k)),
                "比較区画が領域外です"
            );
            reference.keys = keys;
        }
        let start = Instant::now();
        let a = reference
            .audit(
                &model,
                None,
                None,
                error,
                input["direct"].as_bool().unwrap_or(false),
                &mut |_, _, _, _| Ok(()),
            )?
            .unwrap();
        reference.commit(a);
        let seconds = start.elapsed().as_secs_f64();
        let mut candidates = reference
            .rows
            .values()
            .flat_map(|r| r.candidate.iter().copied())
            .collect::<Vec<_>>();
        sort_candidates(&mut candidates);
        if let Some(old) = &expected {
            let old: &Vec<(f64, [f64; 3])> = old;
            ensure!(
                candidates.len() == old.len(),
                "誤差超過候補の数が変化しました"
            );
            for (a, b) in candidates.iter().zip(old) {
                ensure!(
                    a.1 == b.1 && (a.0 - b.0).abs() < 1e-6,
                    "上位候補の順位が変化しました"
                );
            }
        } else {
            expected = Some(candidates.clone());
        }
        let (max, rmse, count) = reference.summary();
        reports.push(json!({"optimized":optimized,"seconds":seconds,"sourceSeconds":reference.block_seconds,"max":max,"rmseOrBound":rmse,"validPixels":count,"evaluatedPixels":reference.evaluated,"certifiedPixels":reference.certified,"candidateCount":candidates.len(),"topCandidates":candidates.iter().take(8).collect::<Vec<_>>()}));
    }
    Ok(
        json!({"sameCandidates":true,"speedup":reports[0]["seconds"].as_f64().unwrap()/reports[1]["seconds"].as_f64().unwrap(),"runs":reports}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn synthetic_reference() -> Reference {
        let inputs = Arc::new(Inputs {
            cache: PathBuf::new(),
            states: HashMap::new(),
        });
        let mut r = Reference::new(inputs, [128., 128.], 128.);
        r.keys = vec![(0, 0)];
        let q = (0..8)
            .flat_map(|y| (0..8).map(move |x| [x as f64, y as f64]))
            .collect::<Vec<_>>();
        let block = Arc::new(Block {
            cell_bounds: std::sync::OnceLock::new(),
            q,
            z: vec![3.; 64],
            cells: vec![(0..64).collect()],
        });
        r.samples.insert((0, 0), (block, 0));
        r
    }
    #[test]
    fn rejected_candidate_stops_without_committing() {
        let m = Fast::new(vec![
            [-100., -100., 0.],
            [100., -100., 0.],
            [-100., 100., 0.],
            [100., 100., 0.],
            [0., 0., 0.],
        ])
        .unwrap();
        for optimized in [false, true] {
            let mut r = synthetic_reference();
            r.optimized = optimized;
            assert!(
                r.audit(&m, None, Some(1.), 1., true, &mut |_, _, _, _| Ok(()))
                    .unwrap()
                    .is_none()
            );
            assert!(r.rows.is_empty());
            assert_eq!(r.evaluated, if optimized { 1 } else { 64 });
        }
    }
    #[test]
    fn final_direct_audit_replaces_interval_bound() {
        let m = Fast::new(vec![
            [-100., -100., 0.],
            [100., -100., 0.],
            [-100., 100., 0.],
            [100., 100., 0.],
            [0., 0., 0.],
        ])
        .unwrap();
        let mut r = synthetic_reference();
        r.interval_bounds = true;
        let a = r
            .audit(&m, None, None, 5., false, &mut |_, _, _, _| Ok(()))
            .unwrap()
            .unwrap();
        r.commit(a);
        assert_eq!(r.certified, 64);
        assert_eq!(r.evaluated, 0);
        assert!(r.summary().0 >= 3.);
        let a = r
            .audit(&m, None, None, 5., true, &mut |_, _, _, _| Ok(()))
            .unwrap()
            .unwrap();
        r.commit(a);
        assert_eq!(r.evaluated, 64);
        assert!((r.summary().0 - 3.).abs() < 1e-8);
    }
    #[test]
    fn asymmetric_audit_rejects_inflation_and_keeps_depression_budget() {
        for height in [6., 0.] {
            let m = Fast::new(vec![[-100.,-100.,height],[100.,-100.,height],
                [-100.,100.,height],[100.,100.,height],[0.,0.,height]]).unwrap();
            let mut r = synthetic_reference();
            r.upward_ratio = 0.25;
            let trial = r.audit(&m,None,Some(5.),5.,true,&mut |_,_,_,_|Ok(())).unwrap();
            assert_eq!(trial.is_some(),height == 0.);
            let audit = r.audit(&m,None,None,5.,true,&mut |_,_,_,_|Ok(())).unwrap().unwrap();
            r.commit(audit);
            let expected = if height == 6. {12.} else {3.};
            assert!((r.summary().0-expected).abs()<1e-8);
            let row = r.rows.values().next().unwrap();
            assert!((row.above-(height-3.).max(0.)).abs()<1e-8);
            assert!((row.below-(3.-height).max(0.)).abs()<1e-8);
        }
    }
    #[test]
    fn cached_local_audit_matches_direct_acceptance_and_residuals() {
        for height in [5.,7.] {
            let model=|h| Fast::new(vec![[-100.,-100.,h],[100.,-100.,h],[-100.,100.,h],[100.,100.,h],[0.,0.,h]]).unwrap();
            let mut cached=synthetic_reference();cached.upward_ratio=0.25;
            let initial=cached.audit(&model(3.),None,Some(13.),13.,true,&mut |_,_,_,_|Ok(())).unwrap().unwrap();cached.commit(initial);
            let mut direct=synthetic_reference();direct.upward_ratio=0.25;
            let a=cached.audit(&model(height),Some([-10.,-10.,10.,10.]),Some(13.),13.,true,&mut |_,_,_,_|Ok(())).unwrap();
            let b=direct.audit(&model(height),None,Some(13.),13.,true,&mut |_,_,_,_|Ok(())).unwrap();
            assert_eq!(a.is_some(),b.is_some());
            if let (Some(a),Some(b))=(a,b) {
                cached.commit(a);direct.commit(b);
                assert_eq!(cached.summary(),direct.summary());
                assert_eq!(cached.residuals,direct.residuals);
            }
        }
    }
    #[test]
    fn nearest_probe_only_rejects_a_real_audited_violation() {
        let m = Fast::new(vec![[-100.,-100.,6.],[100.,-100.,6.],[-100.,100.,6.],[100.,100.,6.],[0.,0.,6.]]).unwrap();
        let mut r=synthetic_reference();r.upward_ratio=0.25;
        assert!(r.rejects_near(&m,[3.,3.],[-10.,-10.,10.,10.],5.).unwrap());
        assert!(r.audit(&m,None,Some(5.),5.,true,&mut |_,_,_,_|Ok(())).unwrap().is_none());
        assert!(!r.rejects_near(&m,[3.,3.],[20.,20.,30.,30.],5.).unwrap());
        assert!(!r.rejects_near(&m,[3.,3.],[-10.,-10.,10.,10.],13.).unwrap());
        assert!(r.rows.is_empty());
    }
    #[test]
    fn local_dem_reference_is_exact_on_pixels_and_affine_between_pixels() {
        let input=Arc::new(Inputs {cache:PathBuf::new(),states:HashMap::new()});
        let mut r=Reference::new(input,[0.;2],100.);
        let q=(0..3).flat_map(|y|(0..3).map(move |x|[(x as f64+0.5)*STEP,(y as f64+0.5)*STEP])).collect::<Vec<_>>();
        let z=q.iter().map(|q|3.+q[0]*2.+q[1]*0.5).collect::<Vec<_>>();
        let mut cells=vec![Vec::new();256];cells[0]=(0..9).collect();
        let mut raw=vec![f64::NAN;65536];
        for (i,&value) in z.iter().enumerate() {raw[(i/3)*256+i%3]=value;}
        r.height_samples.insert((0,0),(Some(Arc::new(raw)),0));
        r.samples.insert((0,0),(Arc::new(Block {cell_bounds:std::sync::OnceLock::new(),q,z,cells}),0));
        for q in [[STEP*0.5,STEP*0.5],[STEP,STEP],[STEP*1.3,STEP*1.7]] {
            assert!((r.height_at(q).unwrap().unwrap()-(3.+q[0]*2.+q[1]*0.5)).abs()<1e-9);
        }
        assert!(r.height_at([STEP*3.,STEP*3.]).unwrap().is_none());
    }
    #[test]
    fn directional_error_composes_across_levels() {
        for a in [-100.,-1.,0.,1.,100.] { for b in [-100.,-1.,0.,1.,100.] {
            assert!(directional_error(a+b,0.25) <= directional_error(a,0.25)+directional_error(b,0.25));
        }}
    }
    #[test]
    fn rank_prefers_small_score_then_original_index() {
        let mut h = BinaryHeap::new();
        h.push(Rank {
            score: 2.,
            id: 0,
            version: 0,
        });
        h.push(Rank {
            score: 1.,
            id: 2,
            version: 0,
        });
        h.push(Rank {
            score: 1.,
            id: 1,
            version: 0,
        });
        assert_eq!(h.pop().unwrap().id, 1);
        assert_eq!(h.pop().unwrap().id, 2);
    }
    #[test]
    fn xyz_definition_is_monotone() {
        for pair in LEVELS.windows(2) {
            assert_eq!(pair[1].0, pair[0].0 + 1);
            assert!(pair[1].1 < pair[0].1);
        }
        assert_eq!(LEVELS.len(), 9);
    }
}

include!("natural_hierarchy.rs");
