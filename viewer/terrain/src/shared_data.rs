use crate::{natural_local_gradient, natural_regional_gpu, natural_regional_surface::Fast};
use anyhow::{Context, Result, ensure};

use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::Arc,
};

pub const WORLD: f64 = 40075016.68557849;
pub type Key = (u32, i32, i32);
pub const ERRORS: [f64; 8] = [500., 400., 300., 250., 200., 125., 60., 25.];
pub fn bounds((z, x, y): Key) -> [f64; 4] {
    let side = WORLD / 2f64.powi(z as i32);
    [
        x as f64 * side,
        y as f64 * side,
        (x + 1) as f64 * side,
        (y + 1) as f64 * side,
    ]
}
pub struct Packet {
    pub key: Key,
    pub origin: [f64; 2],
    pub side: f64,
    pub sections: [Vec<u8>; 4],
    pub index_bounds: [f32; 4],
    pub controls: usize,
    pub compressed_bytes: usize,
    pub density: Vec<u32>,
    pub vertices: Vec<[f32; 4]>,
    pub triangles: Vec<u32>,
    pub timings_ms: [f64; 4],
}
impl Packet {
    /// 地形OFFの景色表示に使う、標高0mの一枚の平面。
    pub fn flat() -> Result<Self> {
        Self::constant_plane((0, 0, 0), 0.)
    }
    pub(crate) fn constant_plane(key: Key, height: f64) -> Result<Self> {
        let area = bounds(key);
        let side = area[2] - area[0];
        let model = Fast::certified(vec![
            [0., 0., height],
            [side, 0., height],
            [0., side, height],
            [side, side, height],
        ])?;
        let gradients = natural_local_gradient::local_gradients(&model);
        let (bytes, info) = natural_regional_gpu::packet(
            &model,
            &gradients,
            [0., 0., side, side],
            serde_json::json!({}),
        )?;
        let mut at = 4 + u32::from_le_bytes(bytes[..4].try_into()?) as usize;
        let mut sections = std::array::from_fn(|_| Vec::new());
        for (section, name) in
            sections
                .iter_mut()
                .zip(["pointBytes", "cellBytes", "nodeBytes", "referenceBytes"])
        {
            let length = info[name].as_u64().context("平面パケット長なし")? as usize;
            *section = bytes[at..at + length].to_vec();
            at += length;
        }
        Ok(Self {
            key,
            origin: [area[0], area[1]],
            side,
            sections,
            index_bounds: std::array::from_fn(|j| {
                info["indexBounds"][j].as_f64().unwrap() as f32 / 1000.
            }),
            controls: 4,
            compressed_bytes: 0,
            density: vec![0; 1024],
            vertices: model
                .base
                .points
                .iter()
                .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32, 0.])
                .collect(),
            triangles: model
                .base
                .topology
                .triangles
                .iter()
                .map(|i| *i as u32)
                .collect(),
            timings_ms: [0.; 4],
        })
    }
    pub fn elevation_at(&self, world: [f64; 2], amount: f64) -> Result<f64> {
        let model = Fast::certified(
            self.vertices
                .iter()
                .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
                .collect(),
        )?;
        let q = [world[0] - self.origin[0], world[1] - self.origin[1]];
        let gradients = natural_local_gradient::local_gradients(&model);
        let basis = model.basis(q, &mut crate::natural_regional_surface::Work::default())?;
        let mut h0 = 0.;
        let mut tangent = 0.;
        let mut den = 0.;
        let mut alpha = 0.;
        let mut beta = 0.;
        for (i, w) in basis {
            let p = model.base.points[i];
            let d = [p[0] - q[0], p[1] - q[1]];
            let r2 = d[0] * d[0] + d[1] * d[1];
            if r2 < 1e-16 {
                return Ok(p[2]);
            }
            let r = r2.sqrt();
            let inv = w / r;
            h0 += w * p[2];
            den += inv;
            tangent += inv * (p[2] - gradients[i][0] * d[0] - gradients[i][1] * d[1]);
            alpha += w * r;
            beta += w * r2;
        }
        ensure!(den > 0., "選択座標の標高を補間できません");
        alpha /= den;
        let h1 = (alpha * h0 + beta * tangent / den) / (alpha + beta);
        Ok(h0 + (h1 - h0) * amount.clamp(0., 1.))
    }
    pub fn bytes(&self) -> usize {
        self.sections.iter().map(Vec::len).sum::<usize>()
            + self.vertices.len() * 16
            + self.triangles.len() * 4
    }
}
pub struct Database {
    db: yamamuki_terrain::sqlite::Database,
    pub catalog: HashMap<Key, usize>,
    pub level_errors: [f64; 8],
    pub coarse_level_errors: [f64; 2],
    pub generation: String,
    pub credits: String,
    packets: HashMap<Key, Arc<Packet>>,
    order: VecDeque<Key>,
}
impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let started = std::time::Instant::now();
        let mut db = yamamuki_terrain::sqlite::Database::open_metadata(path)?;
        crate::catalog_cache::load(&mut db, path)?;
        let catalog = db
            .catalog
            .iter()
            .map(|(&(z, x, y), &n)| ((z, x as i32, y as i32), n))
            .collect();
        let mut level_errors = ERRORS;
        let mut coarse_level_errors = [1250., 625.];
        for (&z, &error) in &db.level_errors {
            if (6..=13).contains(&z) {
                level_errors[(z - 6) as usize] = error;
            } else if (4..=5).contains(&z) {
                coarse_level_errors[(z - 4) as usize] = error;
            }
        }
        let generation = db.generation.clone();
        let credits = db.credits.clone();
        eprintln!(
            "地形SQLite索引: {}タイル / {:.2}ms",
            db.catalog.len(),
            started.elapsed().as_secs_f64() * 1000.
        );
        Ok(Self {
            db,
            catalog,
            level_errors,
            coarse_level_errors,
            generation,
            credits,
            packets: HashMap::new(),
            order: VecDeque::new(),
        })
    }
    pub fn packet(&mut self, key: Key) -> Result<Arc<Packet>> {
        if let Some(packet) = self.packets.get(&key) {
            return Ok(packet.clone());
        }
        let started = std::time::Instant::now();
        let integers = self.db.points((key.0, key.1 as u32, key.2 as u32))?;
        let decoded = started.elapsed();
        let compressed_bytes = self.db.tile_size((key.0, key.1 as u32, key.2 as u32))?;
        let packet = prepare_packet(key, integers, compressed_bytes, started, decoded)?;
        self.order.push_back(key);
        self.packets.insert(key, packet.clone());
        self.trim();
        Ok(packet)
    }
    /// 復号と親キャッシュは同じ接続で共有し、CPU補助情報だけを並列に作る。
    pub fn packet_batch(
        &mut self,
        keys: &[Key],
        emit: &(impl Fn(Arc<Packet>) + Sync),
    ) -> Result<()> {
        let mut jobs = Vec::new();
        for &key in keys {
            if let Some(packet) = self.packets.get(&key) {
                emit(packet.clone());
                continue;
            }
            let started = std::time::Instant::now();
            let points = self.db.points((key.0, key.1 as u32, key.2 as u32))?;
            let decoded = started.elapsed();
            let bytes = self.db.tile_size((key.0, key.1 as u32, key.2 as u32))?;
            jobs.push((key, points, bytes, started, decoded));
        }
        let ready = std::thread::scope(|scope| {
            let handles = jobs
                .into_iter()
                .map(|(key, points, bytes, started, decoded)| {
                    scope.spawn(move || {
                        let packet = prepare_packet(key, points, bytes, started, decoded)?;
                        emit(packet.clone());
                        Ok::<_, anyhow::Error>((key, packet))
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .map_err(|_| anyhow::anyhow!("タイル準備スレッドが停止しました"))?
                })
                .collect::<Result<Vec<_>>>()
        })?;
        for (key, packet) in ready {
            self.order.push_back(key);
            self.packets.insert(key, packet);
        }
        self.trim();
        Ok(())
    }
    fn trim(&mut self) {
        let mut bytes = self.packets.values().map(|p| p.bytes()).sum::<usize>();
        let attempts = self.order.len();
        for _ in 0..attempts {
            if bytes <= 256 * 1024 * 1024 {
                break;
            }
            let Some(key) = self.order.pop_front() else {
                break;
            };
            if key.0 == 6 {
                self.order.push_back(key);
                continue;
            }
            if let Some(packet) = self.packets.remove(&key) {
                bytes -= packet.bytes();
            }
        }
    }
    pub fn check(path: &Path) -> Result<()> {
        let mut db = Self::open(path)?;
        let mut report = Vec::new();
        let levels = db
            .catalog
            .keys()
            .map(|k| k.0)
            .collect::<std::collections::BTreeSet<_>>();
        for z in levels {
            let key = db
                .catalog
                .keys()
                .filter(|k| k.0 == z)
                .min()
                .copied()
                .context("レベルなし")?;
            let packet = db.packet(key)?;
            report.push(serde_json::json!({"key":key,"controls":packet.controls,"gpu_bytes":packet.bytes(),"timings_ms":packet.timings_ms}));
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
        Ok(())
    }
}

fn prepare_packet(
    key: Key,
    integers: Arc<Vec<[i32; 3]>>,
    compressed_bytes: usize,
    _started: std::time::Instant,
    decoded: std::time::Duration,
) -> Result<Arc<Packet>> {
    let started = std::time::Instant::now() - decoded;
    let b = bounds(key);
    let side = b[2] - b[0];
    let points = integers
        .iter()
        .map(|p| {
            [
                p[0] as f64 / 16. - b[0],
                p[1] as f64 / 16. - b[1],
                p[2] as f64 / 8.,
            ]
        })
        .collect();
    let model = Fast::certified(points)?;
    let modeled = started.elapsed();
    let gradients = natural_local_gradient::local_gradients(&model);
    let graded = started.elapsed();
    let (bytes, info) = natural_regional_gpu::packet(
        &model,
        &gradients,
        [0., 0., side, side],
        serde_json::json!({}),
    )?;
    let packed = started.elapsed();
    ensure!(
        info["maxCandidates"].as_u64().unwrap_or(257) <= 256,
        "GPU候補点が上限超過"
    );
    ensure!(
        info["maxCellVertices"].as_u64().unwrap_or(129) <= 128,
        "VoronoiセルがGPU上限超過"
    );
    let header = u32::from_le_bytes(bytes[..4].try_into()?) as usize;
    let mut at = 4 + header;
    let mut sections = std::array::from_fn(|_| Vec::new());
    for (section, name) in
        sections
            .iter_mut()
            .zip(["pointBytes", "cellBytes", "nodeBytes", "referenceBytes"])
    {
        let length = info[name].as_u64().context("GPUパケット長なし")? as usize;
        *section = bytes
            .get(at..at + length)
            .context("GPUパケット長が不正")?
            .to_vec();
        at += length;
    }

    let mut density = vec![0u32; 1024];
    for p in &model.base.points {
        if p[0] >= 0. && p[1] >= 0. && p[0] <= side && p[1] <= side {
            let x = (p[0] / side * 32.).floor().clamp(0., 31.) as usize;
            let y = (p[1] / side * 32.).floor().clamp(0., 31.) as usize;
            density[y * 32 + x] += 1;
        }
    }
    let packet = Arc::new(Packet {
        timings_ms: [
            decoded,
            modeled - decoded,
            graded - modeled,
            packed - graded,
        ]
        .map(|t| t.as_secs_f64() * 1000.),
        density,
        vertices: model
            .base
            .points
            .iter()
            .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32, 0.])
            .collect(),
        triangles: model
            .base
            .topology
            .triangles
            .iter()
            .map(|&i| i as u32)
            .collect(),
        key,
        origin: [b[0], b[1]],
        side,
        sections,
        index_bounds: std::array::from_fn(|j| {
            info["indexBounds"][j].as_f64().unwrap() as f32 / 1000.
        }),
        controls: integers.len(),
        compressed_bytes,
    });
    Ok(packet)
}

#[cfg(test)]
mod benchmarks {
    use super::*;
    #[test]
    #[ignore = "実データのレベル切替に伴う標高差を計測する"]
    fn measure_level_height_bias() -> Result<()> {
        let path = std::env::var("YAMAMUKI_TERRAIN_BENCH_DB")?;
        let mut db = Database::open(Path::new(&path))?;
        for (name, lon, lat) in [
            ("fuji", 138.7274, 35.3606),
            ("yari", 137.647, 36.342),
            ("tsukuba", 140.1069, 36.2254),
        ] {
            if std::env::var("YAMAMUKI_TERRAIN_BENCH_REGION")
                .ok()
                .is_some_and(|v| v != name)
            {
                continue;
            }
            let center = crate::selection::lonlat(lon, lat);
            let mut models = HashMap::new();
            for z in 6..13u32 {
                let mut differences = Vec::new();
                for y in -4..=4 {
                    for x in -4..=4 {
                        let q = [
                            center[0] + f64::from(x) * 2000.,
                            center[1] + f64::from(y) * 2000.,
                        ];
                        let mut h = [0.; 2];
                        let mut valid = true;
                        for (j, level) in [z, z + 1].into_iter().enumerate() {
                            let side = WORLD / 2f64.powi(level as i32);
                            let key = (level, (q[0] / side) as i32, (q[1] / side) as i32);
                            if !db.catalog.contains_key(&key) {
                                valid = false;
                                break;
                            }
                            if !models.contains_key(&key) {
                                let b = bounds(key);
                                let points = db.db.points((level, key.1 as u32, key.2 as u32))?;
                                models.insert(
                                    key,
                                    Fast::certified(
                                        points
                                            .iter()
                                            .map(|p| {
                                                [
                                                    p[0] as f64 / 16. - b[0],
                                                    p[1] as f64 / 16. - b[1],
                                                    p[2] as f64 / 8.,
                                                ]
                                            })
                                            .collect(),
                                    )?,
                                );
                            }
                            let b = bounds(key);
                            h[j] = models[&key].evaluate(
                                [q[0] - b[0], q[1] - b[1]],
                                &mut crate::natural_regional_surface::Work::default(),
                            )?;
                        }
                        if valid {
                            differences.push(h[1] - h[0]);
                        }
                    }
                }
                if differences.is_empty() {
                    continue;
                }
                eprintln!(
                    "{name} z{z}->z{} count={} mean_fine_minus_coarse={:.3}m rising={} falling={} max_rise={:.3}m max_fall={:.3}m",
                    z + 1,
                    differences.len(),
                    differences.iter().sum::<f64>() / differences.len() as f64,
                    differences.iter().filter(|v| **v > 0.).count(),
                    differences.iter().filter(|v| **v < 0.).count(),
                    differences.iter().copied().fold(0., f64::max),
                    differences.iter().copied().fold(0., f64::min)
                );
            }
        }
        Ok(())
    }
    #[test]
    #[ignore = "全国SQLiteの索引とGPU初期化の時間を実測する"]
    fn benchmark_open_national_database() -> Result<()> {
        let path = std::env::var("YAMAMUKI_TERRAIN_BENCH_DB")?;
        for i in 0..2 {
            let started = std::time::Instant::now();
            let db = Database::open(Path::new(&path))?;
            eprintln!(
                "SQLite open {i}: {}タイル / {:.2}ms",
                db.catalog.len(),
                started.elapsed().as_secs_f64() * 1000.
            );
        }
        Ok(())
    }
    #[test]
    #[ignore = "実データの並列準備を手元で計測する"]
    fn benchmark_parallel_packet_preparation() -> Result<()> {
        let path = std::env::var("YAMAMUKI_TERRAIN_BENCH_DB")?;
        let mut serial = Database::open(Path::new(&path))?;
        let mut keys = serial
            .catalog
            .keys()
            .filter(|k| k.0 == 6)
            .copied()
            .collect::<Vec<_>>();
        keys.sort();
        let start = std::time::Instant::now();
        let baseline = keys
            .iter()
            .map(|&k| serial.packet(k))
            .collect::<Result<Vec<_>>>()?;
        let serial_ms = start.elapsed().as_secs_f64() * 1000.;
        let mut parallel = Database::open(Path::new(&path))?;
        let start = std::time::Instant::now();
        for batch in keys.chunks(4) {
            parallel.packet_batch(batch, &|_| {})?;
        }
        let parallel_ms = start.elapsed().as_secs_f64() * 1000.;
        for (key, expected) in keys.iter().zip(baseline) {
            let actual = parallel.packet(*key)?;
            assert_eq!(expected.sections, actual.sections);
            assert_eq!(expected.vertices, actual.vertices);
            assert_eq!(expected.triangles, actual.triangles);
        }
        eprintln!(
            "{}タイル: 直列 {:.2} ms / 並列 {:.2} ms（補助情報・頂点・三角形は同一）",
            keys.len(),
            serial_ms,
            parallel_ms
        );
        Ok(())
    }
    #[test]
    #[ignore = "実データのタイル準備を手元で計測する"]
    fn benchmark_packet_preparation() -> Result<()> {
        let path =
            std::env::var("YAMAMUKI_TERRAIN_BENCH_DB").context("DBのパスを指定してください")?;
        let now = std::time::Instant::now();
        let mut db = Database::open(Path::new(&path))?;
        eprintln!("DB初期化 {:.1} ms", now.elapsed().as_secs_f64() * 1000.);
        let xy = crate::selection::lonlat(138.7274, 35.3606);
        for z in [6, 10, 13] {
            let side = WORLD / 2f64.powi(z);
            let key = (z as u32, (xy[0] / side) as i32, (xy[1] / side) as i32);
            if !db.catalog.contains_key(&key) {
                continue;
            }
            let p = db.packet(key)?;
            eprintln!(
                "{key:?} {}点: 復号 {:.2} / 位相 {:.2} / 勾配 {:.2} / 補助情報 {:.2} ms",
                p.controls, p.timings_ms[0], p.timings_ms[1], p.timings_ms[2], p.timings_ms[3]
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod height_tests {
    use super::*;
    #[test]
    fn no_terrain_plane_has_zero_height_and_gpu_data() {
        let packet = Packet::flat().unwrap();
        assert!(packet.sections.iter().all(|s| !s.is_empty()));
        assert_eq!(packet.triangles.len(), 6);
        for p in [[WORLD * 0.8, WORLD * 0.4], [WORLD * 0.1, WORLD * 0.7]] {
            assert!(packet.elevation_at(p, 0.25).unwrap().abs() < 1e-6);
        }
    }
    fn plane(flat: bool) -> Packet {
        Packet {
            key: (13, 0, 0),
            origin: [10000., 20000.],
            side: 1000.,
            sections: std::array::from_fn(|_| Vec::new()),
            index_bounds: [0.; 4],
            controls: 5,
            compressed_bytes: 0,
            density: Vec::new(),
            triangles: Vec::new(),
            timings_ms: [0.; 4],
            vertices: [
                [0f32, 0.],
                [1000., 0.],
                [0., 1000.],
                [1000., 1000.],
                [500., 500.],
            ]
            .map(|[x, y]| {
                [
                    x,
                    y,
                    if flat {
                        150.
                    } else {
                        100. + 0.5 * x + 0.25 * y
                    },
                    0.,
                ]
            })
            .to_vec(),
        }
    }
    #[test]
    fn coordinate_height_uses_natural_surface_and_control_values() {
        let plane = plane(false);
        assert!(
            (plane.elevation_at([10345., 20642.], 0.).unwrap() - (100. + 0.5 * 345. + 0.25 * 642.))
                .abs()
                < 0.001
        );
        for amount in [0., 0.25, 1.] {
            assert!((plane.elevation_at([10500., 20500.], amount).unwrap() - 475.).abs() < 0.001);
            assert!(
                (super::height_tests::plane(true)
                    .elevation_at([10345., 20642.], amount)
                    .unwrap()
                    - 150.)
                    .abs()
                    < 0.001
            );
        }
    }
}
