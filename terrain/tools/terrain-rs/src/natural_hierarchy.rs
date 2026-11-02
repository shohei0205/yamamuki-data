// 細レベルの曲面を参照し、元DEMは完成後の検査だけに使う階層生成。
const HIER_HALO: f64 = 500.;
const FIELD_HALO: f64 = 250.;
// 子曲面の総点数を目安にし、粗タイルへ最詳細用の固定上限を流用しない。
fn hierarchy_point_limit(child_points: usize) -> usize {
    child_points.clamp(20000, MAX_HIERARCHY_POINTS) + 4
}
fn hier_bounds(z: u32, x: u32, y: u32) -> [f64; 4] {
    let s = WORLD / (1u64 << z) as f64;
    [
        x as f64 * s,
        y as f64 * s,
        (x + 1) as f64 * s,
        (y + 1) as f64 * s,
    ]
}
fn hier_intersection(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let r = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    (r[0] < r[2] && r[1] < r[3]).then_some(r)
}
fn hier_children(z: u32, x: u32, y: u32, domains: &[[f64; 4]]) -> Vec<(u32, u32, u32)> {
    let b = hier_bounds(z, x, y);
    let s = WORLD / (1u64 << (z + 1)) as f64;
    let r = [
        b[0] - 2. * FIELD_HALO,
        b[1] - 2. * FIELD_HALO,
        b[2] + 2. * FIELD_HALO,
        b[3] + 2. * FIELD_HALO,
    ];
    let mut out = Vec::new();
    for cy in
        (r[1] / s).floor().max(0.) as u32..(r[3] / s).ceil().min((1u64 << (z + 1)) as f64) as u32
    {
        for cx in (r[0] / s).floor().max(0.) as u32
            ..(r[2] / s).ceil().min((1u64 << (z + 1)) as f64) as u32
        {
            if domains
                .iter()
                .any(|&d| hier_intersection(hier_bounds(z + 1, cx, cy), d).is_some())
            {
                out.push((z + 1, cx, cy));
            }
        }
    }
    out
}
pub fn prepare_hierarchy(root: &Path, only: &[u32], reuse: Option<&Path>) -> Result<()> {
    prepare_hierarchy_with_levels(root, only, reuse, LEVELS)
}
pub fn prepare_hierarchy_with_levels(root: &Path, only: &[u32], reuse: Option<&Path>, levels: &[(u32,f64)]) -> Result<()> {
    prepare_hierarchy_with_policy(root, only, reuse, levels, 1.)
}
pub fn prepare_hierarchy_with_policy(root: &Path, only: &[u32], reuse: Option<&Path>, levels: &[(u32,f64)], upward_ratio: f64) -> Result<()> {
    prepare_hierarchy_with_budget(root, only, reuse, levels, upward_ratio, false)
}
pub fn prepare_hierarchy_with_budget(root: &Path, only: &[u32], reuse: Option<&Path>, levels: &[(u32,f64)], upward_ratio: f64, local_budget: bool) -> Result<()> {
    prepare_hierarchy_with_options(root, only, reuse, levels, upward_ratio, local_budget, false)
}
pub fn prepare_hierarchy_with_options(root: &Path, only: &[u32], reuse: Option<&Path>, levels: &[(u32,f64)], upward_ratio: f64, local_budget: bool, full_fit: bool) -> Result<()> {
    ensure!(upward_ratio == 1. || reuse.is_none(), "非対称誤差では旧データの再利用はできません");
    ensure!(upward_ratio > 0. && upward_ratio <= 1., "上方向誤差比が不正です");
    let &(finest, finest_error) = levels.last().context("レベル定義が空です")?;
    let mut regions = Vec::new();
    let mut domains = Vec::new();
    for (name, lat, lon) in [
        ("fuji", 35.3606f64, 138.7274f64),
        ("yari", 36.3419, 137.6476),
        ("tsukuba", 36.2254, 140.1067),
    ] {
        let c = [
            (lon + 180.) / 360. * WORLD,
            (1. - lat.to_radians().tan().asinh() / std::f64::consts::PI) * 0.5 * WORLD,
        ];
        let r = [c[0] - 64000., c[1] - 64000., c[0] + 64000., c[1] + 64000.];
        regions.push(json!({"region":name,"center":c,"bounds":r}));
        domains.push([
            r[0] - HIER_HALO,
            r[1] - HIER_HALO,
            r[2] + HIER_HALO,
            r[3] + HIER_HALO,
        ]);
    }
    let mut settings = json!({"generationMode":"fine-to-coarse","regions":regions,"domainBounds":domains,"domainHalo":HIER_HALO,"fieldHalo":FIELD_HALO,"levels":levels.iter().map(|&(z,e)|json!({"z":z,"error_m":e,"side_m":WORLD/(1u64<<z) as f64,"fine_error_m":e/2.})).collect::<Vec<_>>(),"lpf":false,"previewSpan":128000,"tileScheme":"standard-XYZ-EPSG3857","productionReady":false,"pointBudget":null,"codec":"NQC1 Rice independent","reference":"blended immediate finer-level Sibson fields; source DEM only for finest fit and final validation"});
    if upward_ratio != 1. { settings["upwardErrorRatio"] = json!(upward_ratio); }
    if local_budget {
        settings["hierarchyErrorPolicy"] = json!("local-dem-envelope");
        settings["localGuideReference"] = json!("finer-curve-subpixel + exact-dem-pixels");
    }
    if full_fit {
        settings["fineFitFullBudget"] = json!(true);
        for level in settings["levels"].as_array_mut().unwrap() { level["fine_error_m"] = level["error_m"].clone(); }
    }
    if root.join("settings.json").exists() {
        let old: Value = serde_json::from_slice(&fs::read(root.join("settings.json"))?)?;
        ensure!(
            old == settings,
            "既存の設定は階層生成と異なります。別の出力先が必要です"
        );
    } else {
        write(&root.join("settings.json"), &settings)?;
    }
    let c = Connection::open(root.join("queue.sqlite3"))?;
    c.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE IF NOT EXISTS jobs(z INT,x INT,y INT,state TEXT,priority REAL,info TEXT,PRIMARY KEY(z,x,y)) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS region_tiles(region TEXT,z INT,x INT,y INT,PRIMARY KEY(region,z,x,y)) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS tile_dependencies(z INT,x INT,y INT,cz INT,cx INT,cy INT,PRIMARY KEY(z,x,y,cz,cx,cy)) WITHOUT ROWID;CREATE INDEX IF NOT EXISTS hierarchy_jobs ON jobs(state,z DESC,priority,y,x);")?;
    let mut jobs = std::collections::BTreeSet::new();
    if only.is_empty() {
        for region in settings["regions"].as_array().unwrap() {
            let r: [f64; 4] = serde_json::from_value(region["bounds"].clone())?;
            let d = [
                r[0] - HIER_HALO,
                r[1] - HIER_HALO,
                r[2] + HIER_HALO,
                r[3] + HIER_HALO,
            ];
            for &(z, _) in levels {
                let s = WORLD / (1u64 << z) as f64;
                for y in (d[1] / s).floor() as u32..(d[3] / s).ceil() as u32 {
                    for x in (d[0] / s).floor() as u32..(d[2] / s).ceil() as u32 {
                        jobs.insert((z, x, y));
                        c.execute(
                            "INSERT OR IGNORE INTO region_tiles VALUES(?1,?2,?3,?4)",
                            rusqlite::params![region["region"].as_str(), z, x, y],
                        )?;
                    }
                }
            }
        }
    } else {
        ensure!(
            only.len() == 3 && levels.iter().any(|v| v.0 == only[0]),
            "XYZ指定が不正です"
        );
        jobs.insert((only[0], only[1], only[2]));
    }
    let mut todo = jobs.iter().copied().collect::<Vec<_>>();
    let mut deps = Vec::new();
    while let Some((z, x, y)) = todo.pop() {
        if z == finest {
            continue;
        }
        for child in hier_children(z, x, y, &domains) {
            deps.push(((z, x, y), child));
            if jobs.insert(child) {
                todo.push(child);
            }
        }
    }
    let tx = c.unchecked_transaction()?;
    for &(z, x, y) in &jobs {
        let s = WORLD / (1u64 << z) as f64;
        let priority = settings["regions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let v = r["center"].as_array().unwrap();
                ((x as f64 + 0.5 - v[0].as_f64().unwrap() / s).powi(2)
                    + (y as f64 + 0.5 - v[1].as_f64().unwrap() / s).powi(2))
                .sqrt()
            })
            .fold(f64::INFINITY, f64::min);
        c.execute(
            "INSERT OR IGNORE INTO jobs VALUES(?1,?2,?3,'pending',?4,'{}')",
            rusqlite::params![z, x, y, priority],
        )?;
    }
    for (p, q) in deps {
        c.execute(
            "INSERT OR IGNORE INTO tile_dependencies VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![p.0, p.1, p.2, q.0, q.1, q.2],
        )?;
    }
    tx.commit()?;
    let mut reused = 0;
    if let Some(old) = reuse {
        ensure!(
            old.canonicalize()? != root.canonicalize()?,
            "元と新しい出力先が同じです"
        );
        for &(z, x, y) in &jobs {
            if z != finest {
                continue;
            }
            let ready: String = c.query_row(
                "SELECT state FROM jobs WHERE z=?1 AND x=?2 AND y=?3",
                rusqlite::params![z, x, y],
                |r| r.get(0),
            )?;
            if ready == "ready" {
                continue;
            }
            let src = Job::folder(old, z, x, y);
            let Ok(bytes) = fs::read(src.join("selection.json")) else {
                let state = src.join("rust-state.bin");
                let dst = Job::folder(root, z, x, y);
                if state.exists() && !dst.join("rust-state.bin").exists() {
                    fs::create_dir_all(&dst)?;
                    fs::copy(state, dst.join("rust-state.bin"))?;
                }
                continue;
            };
            let a: Value = serde_json::from_slice(&bytes)?;
            if a["empty"].as_bool() != Some(true) {
                ensure!(
                    a["allDemPixelsAudited"].as_bool() == Some(true)
                        && a["max"].as_f64().is_some_and(|v| v <= finest_error),
                    "再利用する最詳細の監査が未完了です"
                );
            }
            let dst = Job::folder(root, z, x, y);
            fs::create_dir_all(&dst)?;
            for name in [
                "packet.bin",
                "packet.json",
                "control-xyz.nqc1",
                "control-gradient.json",
                "selection.json",
                "progress.json",
            ] {
                if src.join(name).exists() {
                    fs::copy(src.join(name), dst.join(name))?;
                }
            }
            c.execute(
                "UPDATE jobs SET state='ready',info=?1 WHERE z=?2 AND x=?3 AND y=?4",
                rusqlite::params![a.to_string(), z, x, y],
            )?;
            reused += 1;
        }
    }
    let previous = fs::read(root.join("hierarchy.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .map(|v| {
            v["reusedFinestTotal"]
                .as_u64()
                .or_else(|| v["reusedFinestThisStart"].as_u64())
                .unwrap_or(0)
        })
        .unwrap_or(0);
    write(
        &root.join("hierarchy.json"),
        &json!({"generationMode":"fine-to-coarse","reusedFinestThisStart":reused,"reusedFinestTotal":previous+reused,"total":jobs.len(),"reuseSource":reuse,"started":now(),"domainHalo":HIER_HALO,"finalDemAudit":true}),
    )?;
    Ok(())
}
struct FinePatch {
    work: std::cell::RefCell<Work>,
    model: Fast,
    world: [f64; 2],
    core: [f64; 4],
    bound: f64,
}
struct FineMosaic {
    dem: Option<std::cell::RefCell<Reference>>,
    patches: Vec<FinePatch>,
    domains: Vec<[f64; 4]>,
    parent: [f64; 4],
}
fn hier_weight(q: f64, lo: f64, hi: f64) -> f64 {
    let d = if q < lo {
        lo - q
    } else if q > hi {
        q - hi
    } else {
        0.
    };
    let t = (1. - d / FIELD_HALO).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
impl FineMosaic {
    fn value(&self, q: [f64; 2]) -> Result<Option<f64>> {
        if !inside(q, self.parent) || !self.domains.iter().any(|&d| inside(q, d)) {
            return Ok(None);
        }
        self.support_value(q)
    }
    // 保存用の量子化点は評価範囲を半刻みだけ囲むことがある。
    // 範囲判定を省いても、子の余白内の正の重みだけを使う。
    fn support_value(&self, q: [f64; 2]) -> Result<Option<f64>> {
        let mut sum = 0.;
        let mut total = 0.;
        for p in &self.patches {
            let a =
                hier_weight(q[0], p.core[0], p.core[2]) * hier_weight(q[1], p.core[1], p.core[3]);
            if a == 0. {
                continue;
            }
            let v = p.model.evaluate(
                [q[0] - p.world[0], q[1] - p.world[1]],
                &mut p.work.borrow_mut(),
            )?;
            sum += a * v;
            total += a;
        }
        Ok((total > 1e-12).then_some(sum / total))
    }
    // 子の制御点が範囲外でも、切り取った細い余白内に評価点を確保する。
    fn seed_samples(&self, samples: &mut CurveSamples) -> Result<()> {
        for patch in &self.patches {
            let field = [patch.core[0]-FIELD_HALO+0.0625, patch.core[1]-FIELD_HALO+0.0625,
                patch.core[2]+FIELD_HALO-0.0625, patch.core[3]+FIELD_HALO-0.0625];
            for &domain in &self.domains {
                if let Some(r) = hier_intersection(field, domain)
                    .and_then(|r| hier_intersection(r, self.parent)) {
                    for ty in [0., 0.5, 1.] {
                        for tx in [0., 0.5, 1.] {
                            samples.add(self, [r[0]+(r[2]-r[0])*tx, r[1]+(r[3]-r[1])*ty])?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn error_bound(&self) -> f64 {
        self.patches.iter().map(|p| p.bound).fold(0., f64::max)
    }
}
struct CurveSamples {
    upward_ratio: f64,
    q: Vec<[f64; 2]>,
    target: Vec<f64>,
    known: HashMap<(u64, u64), Option<usize>>,
    world: [f64; 2],
}
impl CurveSamples {
    fn add(&mut self, m: &FineMosaic, q: [f64; 2]) -> Result<bool> {
        let key = (q[0].to_bits(), q[1].to_bits());
        if self.known.contains_key(&key) {
            return Ok(false);
        }
        let Some(fine) = m.value(q)? else {
            self.known.insert(key, None);
            return Ok(false);
        };
        // 元画素の間に仮定した線形面と、引き継ぐ細曲面の頂点高さは一致しない。
        // 画素間は細曲面をガイドとし、実画素の制約だけを元DEMへ厳密に合わせる。
        let pixel=|v:f64| ((v/STEP-0.5)-(v/STEP-0.5).round()).abs()<1e-7;
        let v = if pixel(q[0]) && pixel(q[1]) {
            if let Some(dem)=&m.dem {dem.borrow_mut().height_at(q)?.unwrap_or(fine)} else {fine}
        } else {fine};
        self.known.insert(key, Some(self.q.len()));
        ensure!(
            self.q.len() < 2_000_000,
            "曲面参照の評価点が安全上限を超えました"
        );
        self.q.push([q[0] - self.world[0], q[1] - self.world[1]]);
        self.target.push(v);
        Ok(true)
    }
    fn value(&mut self, m: &FineMosaic, q: [f64; 2]) -> Result<Option<f64>> {
        self.add(m, q)?;
        Ok(self.known[&(q[0].to_bits(), q[1].to_bits())].map(|i| self.target[i]))
    }
    fn residuals(&self, m: &Fast) -> Result<Vec<f64>> {
        let mut w = Work::default();
        self.q
            .iter()
            .zip(&self.target)
            .map(|(&q, &z)| Ok(directional_error(m.evaluate(q, &mut w)? - z, self.upward_ratio)))
            .collect()
    }
    fn adaptive(&mut self, source: &FineMosaic, coarse: &Fast, budget: f64) -> Result<bool> {
        let mut todo = Vec::new();
        for patch in &source.patches {
            for &d in &source.domains {
                if let Some(r) = hier_intersection(patch.core, d)
                    .and_then(|r| hier_intersection(r, source.parent))
                {
                    todo.push((r, 0usize));
                }
            }
        }
        let mut bad = false;
        let min_size = (source
            .patches
            .first()
            .map(|p| p.core[2] - p.core[0])
            .unwrap_or(1.)
            / 512.)
            .max(STEP);

        let mut w = Work::default();
        while let Some((r, depth)) = todo.pop() {
            let (mx, my) = ((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5);
            let points = [
                [r[0], r[1]],
                [r[2], r[1]],
                [r[0], r[3]],
                [r[2], r[3]],
                [mx, r[1]],
                [mx, r[3]],
                [r[0], my],
                [r[2], my],
                [mx, my],
            ];
            let mut e = [0.; 9];
            for (i, &q) in points.iter().enumerate() {
                if let Some(z) = self.value(source, q)? {
                    let delta = coarse.evaluate([q[0] - self.world[0], q[1] - self.world[1]], &mut w)? - z;
                    e[i] = if delta > 0. { delta / self.upward_ratio } else { delta };
                }
            }
            let max = e.iter().map(|v| v.abs()).fold(0., f64::max);
            if max > budget {
                bad = true;
                // 全領域の超過箇所を同じ回で集める。16か所ごとの再適合を避ける。
                continue;
            }
            let curvature = (e[8] - (e[0] + e[1] + e[2] + e[3]) * 0.25)
                .abs()
                .max((e[4] - (e[0] + e[1]) * 0.5).abs())
                .max((e[5] - (e[2] + e[3]) * 0.5).abs())
                .max((e[6] - (e[0] + e[2]) * 0.5).abs())
                .max((e[7] - (e[1] + e[3]) * 0.5).abs());
            if depth < 7
                && (r[2] - r[0]).max(r[3] - r[1]) > min_size
                && (max + 4. * curvature > budget * 0.75 || curvature > budget * 0.1)
            {
                todo.extend([
                    ([r[0], r[1], mx, my], depth + 1),
                    ([mx, r[1], r[2], my], depth + 1),
                    ([r[0], my, mx, r[3]], depth + 1),
                    ([mx, my, r[2], r[3]], depth + 1),
                ]);
            }
        }
        Ok(bad)
    }
}
impl Job {
    fn hierarchy_control(&self, source: &FineMosaic, q: [f64; 2], occupied: &[[f64; 3]]) -> Result<[f64; 3]> {
        let mut best = None;
        let mut distance = f64::INFINITY;
        for &domain in &source.domains {
            // 評価範囲の内側だけへ丸めると、端の評価点が実制御点の凸包外に残る。
            // 2刻みの配置余白で端を囲み、巨大な計算用ガードの影響を避ける。
            let expand = |r: [f64; 4]| [r[0]-0.125,r[1]-0.125,r[2]+0.125,r[3]+0.125];
            let Some(r) = hier_intersection(expand(domain), expand(source.parent)) else {
                continue;
            };
            for patch in &source.patches {
                let f = [
                    patch.core[0] - FIELD_HALO + 0.0625,
                    patch.core[1] - FIELD_HALO + 0.0625,
                    patch.core[2] + FIELD_HALO - 0.0625,
                    patch.core[3] + FIELD_HALO - 0.0625,
                ];
                let Some(r) = hier_intersection(r, f) else {
                    continue;
                };
                let lo = [
                    (r[0] / 0.0625).ceil() * 0.0625,
                    (r[1] / 0.0625).ceil() * 0.0625,
                ];
                let hi = [
                    (r[2] / 0.0625).floor() * 0.0625,
                    (r[3] / 0.0625).floor() * 0.0625,
                ];
                if lo[0] > hi[0] || lo[1] > hi[1] {
                    continue;
                }
                // 最近点が既に存在する場合は、隣接する量子化位置も候補にする。
                // 半刻みの評価点を片側へ丸め続けて同じ点を再追加しない。
                for dy in [0., -1., 1.] {
                    for dx in [0., -1., 1.] {
                        let xy = [
                            (((q[0] / 0.0625 + 0.5).floor()+dx) * 0.0625).clamp(lo[0], hi[0]),
                            (((q[1] / 0.0625 + 0.5).floor()+dy) * 0.0625).clamp(lo[1], hi[1]),
                        ];
                        if occupied.iter().any(|p| p[0] == xy[0]-self.world[0] && p[1] == xy[1]-self.world[1]) {
                            continue;
                        }
                        let d = (q[0]-xy[0]).powi(2)+(q[1]-xy[1]).powi(2);
                        if d >= distance { continue; }
                        if let Some(z) = source.support_value(xy)? {
                            distance = d;
                            best = Some([xy[0]-self.world[0],xy[1]-self.world[1],(z/0.125+0.5).floor()*0.125]);
                        }
                    }
                }
            }
        }
        ensure!(
            distance < 1.,
            "曲面の量子化支持点が有効範囲から離れすぎています: q={:?}, distance2={}, nearest={:?}", q, distance, best
        );
        best.context("曲面の量子化支持点がありません")
    }
    fn hierarchy_source(&self) -> Result<(FineMosaic, CurveSamples, Vec<(u32, u32, u32)>)> {
        let c = Connection::open(self.root.join("queue.sqlite3"))?;
        let mut stmt = c.prepare(
            "SELECT cz,cx,cy FROM tile_dependencies WHERE z=?1 AND x=?2 AND y=?3 ORDER BY cy,cx",
        )?;
        let children = stmt
            .query_map(rusqlite::params![self.z, self.x, self.y], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, u32>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut patches = Vec::new();
        for &(z, x, y) in &children {
            let path = Job::folder(&self.root, z, x, y);
            let a: Value = serde_json::from_slice(&fs::read(path.join("selection.json"))?)?;
            ensure!(
                a["allDemPixelsAudited"].as_bool() == Some(true),
                "細レベルの元DEM検査が未完了です"
            );
            if a["empty"].as_bool() == Some(true) {
                continue;
            }
            let core = hier_bounds(z, x, y);
            let world = [(core[0] + core[2]) * 0.5, (core[1] + core[3]) * 0.5];
            let origin = [
                (core[0] / 0.0625 + 0.5).floor() as i32,
                (core[1] / 0.0625 + 0.5).floor() as i32,
            ];
            let ints = natural_codec::decode_global(
                &fs::read(path.join("control-xyz.nqc1"))?,
                origin,
                &[],
            )?;
            let points = ints
                .iter()
                .map(|p| {
                    [
                        p[0] as f64 * 0.0625 - world[0],
                        p[1] as f64 * 0.0625 - world[1],
                        p[2] as f64 * 0.125,
                    ]
                })
                .collect();
            patches.push(FinePatch {
                work: std::cell::RefCell::new(Work::default()),
                model: Fast::new(points)?,
                world,
                core,
                bound: a.get("weightedMax").unwrap_or(&a["max"]).as_f64().context("細レベルの誤差が不明です")?,
            });
        }
        let core = hier_bounds(self.z, self.x, self.y);
        let parent = [
            core[0] - FIELD_HALO,
            core[1] - FIELD_HALO,
            core[2] + FIELD_HALO,
            core[3] + FIELD_HALO,
        ];
        let cfg:Value=serde_json::from_slice(&fs::read(self.root.join("settings.json"))?)?;
        let dem=if cfg["hierarchyErrorPolicy"].as_str()==Some("local-dem-envelope") {
            let mut r=Reference::new(self.reference.inputs.clone(),self.world,self.side/2.+FIELD_HALO);
            r.domains=self.reference.domains.clone();Some(std::cell::RefCell::new(r))
        } else {None};
        let m = FineMosaic {
            dem,
            patches,
            domains: self.reference.domains.clone(),
            parent,
        };
        let mut samples = CurveSamples {
            upward_ratio: self.reference.upward_ratio,
            q: Vec::new(),
            target: Vec::new(),
            known: HashMap::new(),
            world: self.world,
        };
        for patch in &m.patches {
            for p in patch.model.base.points.iter().skip(4) {
                samples.add(&m, [p[0] + patch.world[0], p[1] + patch.world[1]])?;
            }
            for t in patch.model.base.topology.triangles.chunks_exact(3) {
                let p = t
                    .iter()
                    .map(|&i| patch.model.base.points[i])
                    .collect::<Vec<_>>();
                for weights in [[1. / 3.; 3], [0.5, 0.5, 0.], [0., 0.5, 0.5], [0.5, 0., 0.5]] {
                    let q = [
                        (0..3).map(|i| weights[i] * p[i][0]).sum::<f64>() + patch.world[0],
                        (0..3).map(|i| weights[i] * p[i][1]).sum::<f64>() + patch.world[1],
                    ];
                    samples.add(&m, q)?;
                }
            }
        }
        m.seed_samples(&mut samples)?;
        Ok((m, samples, children))
    }
    fn hierarchy_fit(
        &mut self,
        source: &FineMosaic,
        samples: &CurveSamples,
        mut p: Vec<[f64; 3]>,
        budget: f64,
        target: f64,
    ) -> Result<Vec<[f64; 3]>> {
        let mut errors = Vec::new();
        let mut dirty = None;
        let limit = hierarchy_point_limit(source.patches.iter().map(|p|p.model.base.points.len().saturating_sub(4)).sum());
        ensure!(p.len() <= limit, "保存済み点数が階層曲面の上限を超えています");
        for iteration in 0..limit {
            let m = Fast::new(p.clone())?;
            if errors.is_empty() {
                errors = samples.residuals(&m)?;
            } else {
                let mut w = Work::default();
                for (i, (&q, &z)) in samples.q.iter().zip(&samples.target).enumerate() {
                    if dirty.is_none_or(|r| inside(q, r)) {
                        errors[i] = directional_error(m.evaluate(q, &mut w)? - z, samples.upward_ratio);
                    }
                }
            }
            let (worst, &max) = errors
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .context("細曲面の評価点がありません")?;
            self.status("curve-fit-no-DEM",p.len(),json!({"iteration":iteration,"curveSamples":samples.q.len(),"curveMax":max,"curveBudget":budget,"pointLimit":limit-4}))?;
            if max <= target {
                // 局所更新範囲は高速化の目安なので、収束時には全評価点を再検査する。
                let full = samples.residuals(&m)?;
                if full.iter().all(|&v| v <= target) {
                    return Ok(p);
                }
                errors = full;
                dirty = None;
                continue;
            }
            let q = samples.q[worst];
            let point =
                self.hierarchy_control(source, [q[0] + self.world[0], q[1] + self.world[1]], &p)?;
            ensure!(
                !p.iter().any(|v| v[0] == point[0] && v[1] == point[1]),
                "量子化済み点の残差が収束しません: q={:?}, point={:?}, max={}, target={}, existing={:?}", q, point, max, target, p.iter().find(|v| v[0] == point[0] && v[1] == point[1])
            );
            dirty = Some(m.insertion_box([point[0], point[1]]));
            ensure!(p.len() < limit, "階層曲面の安全上限{}点で未収束です: 最大誤差{}、目標{}", limit-4, max, target);
            p.push(point);
            self.save(0, &p, &vec![true; p.len()])?;
        }
        anyhow::bail!("曲面適合の反復上限です")
    }
    fn hierarchy_prune(
        &mut self,
        p: &[[f64; 3]],
        samples: &CurveSamples,
        mut active: Vec<bool>,
        budget: f64,
    ) -> Result<(Vec<bool>, Fast, f64)> {
        let fixed = p
            .iter()
            .enumerate()
            .map(|(i, p)| {
                i < 8 || p[0].abs() >= self.side / 2. - STEP || p[1].abs() >= self.side / 2. - STEP
            })
            .collect::<Vec<_>>();
        let mut model = Fast::new(
            p.iter()
                .zip(&active)
                .filter_map(|(&p, &a)| a.then_some(p))
                .collect(),
        )?;
        let mut errors = samples.residuals(&model)?;
        ensure!(
            errors.iter().all(|&v| v <= budget),
            "曲面削減元が誤差超過です"
        );
        let ids = (0..p.len()).collect::<Vec<_>>();
        let mut heap = BinaryHeap::new();
        let mut versions = vec![0; p.len()];
        self.enqueue(p, &active, &fixed, &model, &ids, &mut versions, &mut heap)?;
        let mut final_pass = false;
        let mut attempts = 0;
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
                .filter_map(|(i, &v)| v.then_some(i))
                .collect::<Vec<_>>();
            let index = current.iter().position(|&id| id == r.id).unwrap();
            let bounds = model.deletion_box(index, STEP * 2.);
            let trial = model.without_point(index)?;
            let mut updates = Vec::new();
            let mut ok = true;
            let mut w = Work::default();
            for (i, (&q, &z)) in samples.q.iter().zip(&samples.target).enumerate() {
                if !inside(q, bounds) { continue; }
                let e = directional_error(trial.evaluate(q, &mut w)? - z, samples.upward_ratio);
                if e > budget {
                    ok = false;
                    break;
                }
                updates.push((i, e));
            }
            attempts += 1;
            if ok {
                let mut near = HashSet::from([index]);
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
                for (i, e) in updates {
                    errors[i] = e;
                }
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
            self.status(
                "curve-prune-no-DEM",
                current.len(),
                json!({"curveSamples":samples.q.len(),"attempted":attempts,"curveBudget":budget}),
            )?;
        }
        let errors = samples.residuals(&model)?;
        let maximum = errors.iter().copied().fold(0., f64::max);
        ensure!(maximum <= budget, "曲面の最終評価点検査で誤差超過です");
        Ok((active, model, maximum))
    }
    fn run_hierarchy(&mut self) -> Result<Value> {
        let (source, mut samples, children) = self.hierarchy_source()?;
        if source.patches.is_empty() {
            let audit = json!({"z":self.z,"x":self.x,"y":self.y,"empty":true,"verifiedNoData":true,"allDemPixelsAudited":true,"inheritedNoDataProof":children,"max":0.,"finalPoints":0,"generationMode":"fine-to-coarse"});
            write(&self.folder.join("packet.json"), &audit)?;
            write(&self.folder.join("selection.json"), &audit)?;
            return Ok(audit);
        }
        let bound = source.error_bound();
        let local_budget=source.dem.is_some();
        let budget = if local_budget {self.error} else {self.error - bound - 0.125};
        ensure!(budget > 0., "細曲面の誤差が粗レベルの予算を消費しています");
        let extras = self.folder.join("curve-extra-queries.json");
        if extras.exists() {
            let q: Vec<[f64; 2]> = serde_json::from_slice(&fs::read(&extras)?)?;
            for q in q {
                samples.add(&source, q)?;
            }
        }
        let mut p = if let Some((_, points, mask)) = self.restore()? {
            points
                .into_iter()
                .zip(mask)
                .filter_map(|(p, a)| a.then_some(p))
                .collect::<Vec<_>>()
        } else {
            let mut p = vec![
                [-4. * self.side, -4. * self.side, 0.],
                [4. * self.side, -4. * self.side, 0.],
                [-4. * self.side, 4. * self.side, 0.],
                [4. * self.side, 4. * self.side, 0.],
            ];
            for corner in [[-1., -1.], [1., -1.], [-1., 1.], [1., 1.]] {
                let i = (0..samples.q.len())
                    .min_by(|&a, &b| {
                        let d = |i: usize| {
                            (samples.q[i][0] - corner[0] * self.side / 2.).powi(2)
                                + (samples.q[i][1] - corner[1] * self.side / 2.).powi(2)
                        };
                        d(a).total_cmp(&d(b))
                    })
                    .context("細曲面の支持点がありません")?;
                let q = samples.q[i];
                let v =
                    self.hierarchy_control(&source, [q[0] + self.world[0], q[1] + self.world[1]], &p)?;
                if !p.iter().any(|p| p[0] == v[0] && p[1] == v[1]) {
                    p.push(v);
                }
            }
            p
        };
        let mut extra_queries = if extras.exists() {
            serde_json::from_slice::<Vec<[f64; 2]>>(&fs::read(&extras)?)?
        } else {
            Vec::new()
        };
        let mut dem_audits = 0;
        let mut dem_corrections = 0;
        for repair in 0..12 {
            p = self.hierarchy_fit(
                &source,
                &samples,
                p,
                budget,
                // 半分の誤差への過剰適合を避け、削減・適応評価と同じ予算を使う。
                budget,
            )?;
            let mask = vec![true; p.len()];
            // 初回で点を削減し、検証で必要になった補修点は再削減しない。
            // 補修のたびに削減して別の超過を作る反復を避ける。
            let (active, model) = if repair == 0 {
                let (active, model, _) = self.hierarchy_prune(&p, &samples, mask, budget)?;
                (active, model)
            } else {
                (mask, Fast::new(p.clone())?)
            };
            if samples.adaptive(&source, &model, budget)? {
                p = model.base.points.clone();
                self.save(0, &p, &vec![true; p.len()])?;
                continue;
            }
            let curve_max = samples.residuals(&model)?.into_iter().fold(0., f64::max);
            ensure!(curve_max <= budget, "追加した曲面評価点で誤差超過です");
            dem_audits += 1;
            let audit = self
                .scan(&model, None, None, self.error, "hierarchy-final-DEM-audit")?
                .unwrap();
            self.reference.commit(audit);
            let (max, rmse, valid) = self.reference.summary();
            if max > self.error {
                dem_corrections += 1;
                let mut bad = self
                    .reference
                    .rows
                    .values()
                    .flat_map(|r| r.candidate.iter().copied())
                    .collect::<Vec<_>>();
                sort_candidates(&mut bad);
                let mut added = 0;
                // 局所DEM方式では、一度の全画素監査で見つけた超過をまとめて補修する。
                // 旧方式は比較用に従来の16候補ずつを維持する。
                let count=if local_budget {bad.len()} else {16.min(bad.len())};
                for (_, q) in bad.iter().take(count) {
                    let global = [q[0] + self.world[0], q[1] + self.world[1]];
                    let fine = source
                        .value(global)?
                        .context("有効DEM画素を覆う細曲面がありません")?;
                    ensure!(
                        directional_error(fine - q[2], self.reference.upward_ratio) <= bound + 1e-6,
                        "細曲面が継承誤差を超えています"
                    );
                    if samples.add(&source, global)? {
                        extra_queries.push(global);
                        added += 1;
                    }
                }
                ensure!(
                    added > 0,
                    "最終DEM誤差を補修する新しい曲面評価点がありません"
                );
                write(&extras, &json!(extra_queries))?;
                p = model.base.points.clone();
                self.save(0, &p, &vec![true; p.len()])?;
                continue;
            }
            self.save(2, &p, &active)?;
            let audit = json!({"generationMode":"fine-to-coarse","fineReferences":children,"fineReferenceMaxError":bound,"curveErrorBudget":budget,"curveSampleMaxError":curve_max,"curveSamples":samples.q.len(),"curveReference":if local_budget {"finer-curve subpixel guides + exact DEM pixel constraints"} else {"finer curves"},"curveAudit":"adaptive samples; not a continuous absolute error bound","sourceAuditPasses":dem_audits,"finalPoints":model.base.points.len()-4,"initialPoints":p.len()-4,"removed":active.iter().filter(|&&v|!v).count(),"max":max,"rmse":rmse,"validPixels":valid,"allDemPixelsAudited":true,"quantizedControls":true,"DEMUsedDuringCurveFit":local_budget,"errorBudgetPolicy":if local_budget {"local-dem-envelope"} else {"inherited-global-bound"},"PointHeightsFromFineCurves":true,"finalDEMDrivenRefinementPasses":dem_corrections,"adaptiveCorrectionUsed":repair>0});
            return self.finish(model, audit);
        }
        anyhow::bail!("曲面階層の適応評価・補修の反復上限です")
    }
}
#[cfg(test)]
mod hierarchy_tests {
    use super::*;
    #[test]
    fn hierarchy_graph_keeps_only_immediate_finer_dependencies() {
        let root = std::env::temp_dir().join(format!(
            "yamamuki-hier-graph-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        prepare_hierarchy(&root, &[13, 7252, 3234], None).unwrap();
        let c = Connection::open(root.join("queue.sqlite3")).unwrap();
        let jobs: u64 = c
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))
            .unwrap();
        let deps: u64 = c
            .query_row(
                "SELECT count(*) FROM tile_dependencies WHERE cz=z+1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(jobs, 17);
        assert_eq!(deps, 16);
        drop(c);
        let cfg: Value =
            serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
        let d = &cfg["domainBounds"][0];
        assert!((d[2].as_f64().unwrap() - d[0].as_f64().unwrap() - 129000.).abs() < 1e-6);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn z13_profile_has_no_z14_dependencies_and_uses_32m() {
        let root=std::env::temp_dir().join(format!("yamamuki-z13-profile-{}-{}",std::process::id(),now()));
        let levels=&[(6,640.),(7,500.),(8,400.),(9,320.),(10,240.),(11,160.),(12,80.),(13,32.)];
        prepare_hierarchy_with_levels(&root,&[12,3626,1617],None,levels).unwrap();
        let c=Connection::open(root.join("queue.sqlite3")).unwrap();
        let max: u32=c.query_row("SELECT max(z) FROM jobs",[],|r|r.get(0)).unwrap();
        assert_eq!(max,13);
        let count:u32=c.query_row("SELECT count(*) FROM jobs",[],|r|r.get(0)).unwrap();
        assert_eq!(count,17);
        let input=Arc::new(Inputs {cache:PathBuf::new(),states:HashMap::new()});
        let job=Job::new(&root,13,7252,3234,input).unwrap();
        assert_eq!(job.error,32.);
        let cfg:Value=serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
        assert_eq!(cfg["levels"].as_array().unwrap().len(),8);
        assert_eq!(cfg["levels"][7]["fine_error_m"],16.);
        drop(job);drop(c);fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn dem_reference_crops_pixels_to_declared_domain() {
        let root = std::env::temp_dir().join(format!(
            "yamamuki-hier-domain-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("14/0")).unwrap();
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 256, 256);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut w = encoder.write_header().unwrap();
            w.write_image_data(&vec![0u8, 0, 100].repeat(65536))
                .unwrap();
        }
        fs::write(root.join("14/0/0.png"), &bytes).unwrap();
        let sha = Sha256::digest(&bytes)
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>();
        let input = Arc::new(Inputs {
            cache: root.clone(),
            states: HashMap::from([((0, 0), ("cached".into(), sha))]),
        });
        let mut r = Reference::new(input, [128. * STEP; 2], 128. * STEP);
        r.domains = vec![[0., 0., 2. * STEP, 2. * STEP]];
        let block = r.block((0, 0)).unwrap();
        assert_eq!(block.z.len(), 4);
        assert!(block.z.iter().all(|&z| z == 1.));
        drop(block);
        drop(r);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn blending_is_positive_and_continuous() {
        let make = |core: [f64; 4], z: f64| FinePatch {
            work: std::cell::RefCell::new(Work::default()),
            world: [0., 0.],
            core,
            bound: z.abs(),
            model: Fast::new(vec![
                [-1000., -1000., z],
                [1000., -1000., z],
                [-1000., 1000., z],
                [1000., 1000., z],
                [0., 0., z],
            ])
            .unwrap(),
        };
        let m = FineMosaic {
            dem:None,
            patches: vec![
                make([-500., -500., 0., 500.], 1.),
                make([0., -500., 500., 500.], -1.),
            ],
            domains: vec![[-500., -500., 500., 500.]],
            parent: [-500., -500., 500., 500.],
        };
        for x in -300..300 {
            let v = m.value([x as f64, 0.]).unwrap().unwrap();
            assert!((-1. - 1e-8..=1. + 1e-8).contains(&v));
        }
        assert!(
            (m.value([-1e-5, 0.]).unwrap().unwrap() - m.value([1e-5, 0.]).unwrap().unwrap()).abs()
                < 1e-6
        );
    }
    #[test]
    fn narrow_overlap_gets_reference_samples_without_control_points() {
        let m = FineMosaic {
            dem:None,
            patches: vec![FinePatch {
                work: std::cell::RefCell::new(Work::default()),
                world: [0., 0.], core: [0., 0., 100., 100.], bound: 0.,
                model: Fast::new(vec![[-1000.,-1000.,7.], [1000.,-1000.,7.],
                    [-1000.,1000.,7.], [1000.,1000.,7.], [50.,50.,7.]]).unwrap(),
            }],
            domains: vec![[110., 20., 110.2, 21.]],
            parent: [100., 0., 120., 100.],
        };
        let mut samples = CurveSamples { upward_ratio: 1., q: Vec::new(), target: Vec::new(),
            known: HashMap::new(), world: [0.,0.] };
        m.seed_samples(&mut samples).unwrap();
        assert_eq!(samples.q.len(), 9);
        assert!(samples.q.iter().all(|&q| inside(q, m.domains[0])));
        assert!(samples.target.iter().all(|z| (z-7.).abs()<1e-8));
    }
    #[test]
    fn quantized_support_can_enclose_the_evaluation_boundary() {
        let input = Arc::new(Inputs { cache: PathBuf::new(), states: HashMap::new() });
        let job = Job { root: PathBuf::new(), z: 13, x: 0, y: 0, error: 40.,
            side: 100., world: [0.,0.], folder: PathBuf::new(),
            reference: Reference::new(input, [0.,0.], 300.),
            started: Instant::now(), last: Instant::now() };
        let m = FineMosaic {
            dem:None,
            patches: vec![FinePatch {
                work: std::cell::RefCell::new(Work::default()), world: [0.,0.],
                core: [0.,0.,100.,100.], bound: 0.,
                model: Fast::new(vec![[-1000.,-1000.,7.], [1000.,-1000.,7.],
                    [-1000.,1000.,7.], [1000.,1000.,7.], [50.,50.,7.]]).unwrap(),
            }],
            domains: vec![[0.02,0.,100.,100.]], parent: [0.02,0.,100.,100.],
        };
        let p = job.hierarchy_control(&m, [0.02,50.], &[]).unwrap();
        assert_eq!(p, [0.,50.,7.]);
        assert!(m.value([p[0],p[1]]).unwrap().is_none());
        let next = job.hierarchy_control(&m, [0.02,50.], &[p]).unwrap();
        assert_ne!(p, next);
        assert_eq!(next[2], 7.);
    }
    #[test]
    fn adaptive_audit_collects_more_than_sixteen_bad_regions() {
        let source=FineMosaic {
            dem:None,
            patches:vec![FinePatch { work:std::cell::RefCell::new(Work::default()),world:[0.;2],core:[0.,0.,100.,100.],bound:0.,
                model:Fast::new(vec![[-1000.,-1000.,7.],[1000.,-1000.,7.],[-1000.,1000.,7.],[1000.,1000.,7.],[50.,50.,7.]]).unwrap() }],
            domains:(0..25).map(|i|{let x=(i%5) as f64*20.;let y=(i/5) as f64*20.;[x,y,x+5.,y+5.]}).collect(),
            parent:[0.,0.,100.,100.]
        };
        let coarse=Fast::new(vec![[-1000.,-1000.,0.],[1000.,-1000.,0.],[-1000.,1000.,0.],[1000.,1000.,0.],[50.,50.,0.]]).unwrap();
        let mut samples=CurveSamples { upward_ratio:1.,q:Vec::new(),target:Vec::new(),known:HashMap::new(),world:[0.;2] };
        assert!(samples.adaptive(&source,&coarse,1.).unwrap());
        assert_eq!(samples.q.len(),25*9);
        for r in &source.domains { assert!(samples.q.iter().any(|&q| inside(q,*r))); }
    }
    #[test]
    fn hierarchical_checkpoint_above_twenty_thousand_points_roundtrips() {
        let root=std::env::temp_dir().join(format!("yamamuki-hier-large-checkpoint-{}-{}",std::process::id(),now()));
        let input=Arc::new(Inputs {cache:PathBuf::new(),states:HashMap::new()});
        let job=Job::new(&root,9,451,200,input).unwrap();
        let points=(0..21004).map(|i|[(i%200) as f64,(i/200) as f64,(i%123) as f64]).collect::<Vec<_>>();
        let mask=vec![true;points.len()];job.save(0,&points,&mask).unwrap();
        let (stage,restored,active)=job.restore().unwrap().unwrap();
        assert_eq!(stage,0);assert_eq!(restored,points);assert_eq!(active,mask);
        drop(job);fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn hierarchy_limit_scales_with_finer_data_and_is_bounded() {
        assert_eq!(hierarchy_point_limit(1000),20004);
        assert_eq!(hierarchy_point_limit(44862),44866);
        assert_eq!(hierarchy_point_limit(92398),92402);
        assert_eq!(hierarchy_point_limit(999999),131076);
    }
    #[test]
    fn local_budget_target_uses_dem_without_changing_fine_support_heights() {
        let input=Arc::new(Inputs {cache:PathBuf::new(),states:HashMap::new()});
        let mut dem=Reference::new(input,[0.;2],100.);
        let q=vec![[STEP*0.5,STEP*0.5]];let mut cells=vec![Vec::new();256];cells[0]=vec![0];
        let mut raw=vec![f64::NAN;65536];for i in [0,1,256,257] {raw[i]=3.;}dem.height_samples.insert((0,0),(Some(Arc::new(raw)),0));
        dem.samples.insert((0,0),(Arc::new(Block {cell_bounds:std::sync::OnceLock::new(),q:q.clone(),z:vec![3.],cells}),0));
        let source=FineMosaic {dem:Some(std::cell::RefCell::new(dem)),patches:vec![FinePatch {
            work:std::cell::RefCell::new(Work::default()),world:[0.;2],core:[0.,0.,100.,100.],bound:8.,
            model:Fast::new(vec![[-1000.,-1000.,5.],[1000.,-1000.,5.],[-1000.,1000.,5.],[1000.,1000.,5.],[50.,50.,5.]]).unwrap()}],
            domains:vec![[0.,0.,100.,100.]],parent:[0.,0.,100.,100.]};
        let mut samples=CurveSamples {upward_ratio:0.25,q:Vec::new(),target:Vec::new(),known:HashMap::new(),world:[0.;2]};
        samples.add(&source,q[0]).unwrap();assert_eq!(samples.target,vec![3.]);
        assert!((source.support_value(q[0]).unwrap().unwrap()-5.).abs()<1e-9);
        assert!((samples.residuals(&source.patches[0].model).unwrap()[0]-8.).abs()<1e-9);
        let mid=[STEP,STEP];samples.add(&source,mid).unwrap();
        assert!((samples.target[1]-5.).abs()<1e-9);
        assert!(samples.residuals(&source.patches[0].model).unwrap()[1]<1e-9);
    }
    #[test]
    fn full_fit_option_changes_initial_target_but_not_final_limits() {
        let root=std::env::temp_dir().join(format!("yamamuki-full-fit-{}-{}",std::process::id(),now()));
        let levels=&[(13,25.)];prepare_hierarchy_with_options(&root,&[13,7252,3234],None,levels,0.25,true,true).unwrap();
        let input=Arc::new(Inputs {cache:PathBuf::new(),states:HashMap::new()});
        let job=Job::new(&root,13,7252,3234,input).unwrap();
        assert_eq!(job.initial_fit_error().unwrap(),25.);assert_eq!(job.error,25.);assert_eq!(job.reference.upward_ratio,0.25);
        let cfg:Value=serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
        assert_eq!(cfg["levels"][0]["fine_error_m"],25.);assert!(cfg["fineFitFullBudget"].as_bool().unwrap());
        drop(job);fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn dependencies_are_one_level_finer() {
        let domain = [2e7, 1e7, 2.1e7, 1.1e7];
        for (z, x, y) in hier_children(8, 128, 64, &[domain]) {
            assert_eq!(z, 9);
            assert!(hier_intersection(hier_bounds(z, x, y), domain).is_some());
        }
    }
}
