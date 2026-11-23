//! 平滑化済み作業DEMの閲覧。全画素はGPUの格子として使い、粗表示だけ別キャッシュへ縮小する。
use crate::shared_data::{Key, Packet, bounds};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::PathBuf,
    sync::Arc,
};
use yamamuki_terrain::dem_cache;
pub const TITLE: &str = "平滑化済み標高（作業用DEM）";
#[derive(Clone)]
pub struct Grid {
    pub cells: u32,
    pub values: Vec<f32>,
}
impl Grid {
    pub fn width(&self) -> usize {
        self.cells as usize + 2
    }
    /// 格子は画素中心。タイル端は両側の画素で補間し、欠損を0mへ変換しない。
    pub fn sample(&self, u: f64, v: f64) -> Option<f64> {
        let x = (u * self.cells as f64 + 0.5).clamp(0., self.width() as f64 - 1.000001);
        let y = (v * self.cells as f64 + 0.5).clamp(0., self.width() as f64 - 1.000001);
        let (ix, iy) = (x.floor() as usize, y.floor() as usize);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let mut h = 0.;
        for (dx, dy, w) in [
            (0, 0, (1. - fx) * (1. - fy)),
            (1, 0, fx * (1. - fy)),
            (0, 1, (1. - fx) * fy),
            (1, 1, fx * fy),
        ] {
            if w <= 1e-12 {
                continue;
            }
            let v = self.values[(iy + dy) * self.width() + ix + dx];
            if !v.is_finite() {
                return None;
            }
            h += v as f64 * w;
        }
        Some(h)
    }
}
pub struct Source {
    pub raw: dem_cache::Source,
    pub catalog: HashMap<Key, usize>,
    overviews: Option<Connection>,
    cache_path: PathBuf,
    tiles: HashMap<Key, Arc<Vec<f32>>>,
    order: VecDeque<Key>,
}
fn overview_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_CACHE_HOME"))
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                PathBuf::from(h).join(if cfg!(target_os = "macos") {
                    "Library/Caches"
                } else {
                    ".cache"
                })
            })
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("yamamuki-viewer/dem-overviews-v1")
}
fn store(c: &Connection, key: Key, values: &[f32]) -> Result<()> {
    let raw = values
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    let bytes = zstd::bulk::compress(&raw, 3)?;
    c.execute(
        "INSERT OR REPLACE INTO tiles VALUES(?1,?2,?3,?4)",
        params![key.0, key.1, key.2, bytes],
    )?;
    Ok(())
}
fn read(c: &Connection, key: Key) -> Result<Option<Vec<f32>>> {
    let b = c
        .query_row(
            "SELECT payload FROM tiles WHERE z=?1 AND x=?2 AND y=?3",
            params![key.0, key.1, key.2],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .optional()?;
    b.map(|b| {
        let raw = zstd::bulk::decompress(&b, 128 * 128 * 4)?;
        ensure!(
            raw.len() == 128 * 128 * 4,
            "閲覧用DEMキャッシュが破損しています"
        );
        Ok(raw
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect())
    })
    .transpose()
}
impl Source {
    pub fn from_raw(raw: dem_cache::Source) -> Self {
        let mut catalog = HashMap::new();
        for &(x, y) in raw.header.tiles.keys() {
            for z in 6..=13 {
                catalog.insert(
                    (z, (x >> (14 - z)) as i32, (y >> (14 - z)) as i32),
                    if z == 13 { 512 * 512 } else { 128 * 128 },
                );
            }
        }
        let cache_path = overview_root().join(format!("{}.sqlite3", raw.header.stamp));
        Self {
            raw,
            catalog,
            overviews: None,
            cache_path,
            tiles: HashMap::new(),
            order: VecDeque::new(),
        }
    }
    pub fn prepare(&mut self, progress: &impl Fn(usize, usize) -> bool) -> Result<()> {
        if self.overviews.is_some() {
            return Ok(());
        }
        std::fs::create_dir_all(self.cache_path.parent().unwrap())?;
        let c = Connection::open(&self.cache_path)?;
        c.busy_timeout(std::time::Duration::from_secs(30))?;
        c.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS tiles(z INTEGER,x INTEGER,y INTEGER,payload BLOB NOT NULL,PRIMARY KEY(z,x,y)) WITHOUT ROWID;")?;
        let complete = c
            .query_row("SELECT value FROM meta WHERE key='complete'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .as_deref()
            == Some("true");
        if !complete {
            let mut groups = BTreeMap::<(i32, i32), Vec<(u32, u32)>>::new();
            for &(x, y) in self.raw.header.tiles.keys() {
                groups
                    .entry(((x >> 2) as i32, (y >> 2) as i32))
                    .or_default()
                    .push((x, y));
            }
            let total = self.catalog.keys().filter(|k| k.0 <= 12).count();
            let mut done = 0;
            for (&(x, y), tiles) in &groups {
                let key = (12, x, y);
                if read(&c, key)?.is_none() {
                    let mut values = vec![f32::NAN; 128 * 128];
                    for &(sx, sy) in tiles {
                        let a = self.raw.tile(sx, sy)?.context("原本タイルなし")?;
                        for yy in 0..32 {
                            for xx in 0..32 {
                                let mut sum = 0f64;
                                let mut n = 0;
                                for dy in 0..8 {
                                    for dx in 0..8 {
                                        let h = a[(yy * 8 + dy) * 256 + xx * 8 + dx];
                                        if h.is_finite() {
                                            sum += h as f64;
                                            n += 1;
                                        }
                                    }
                                }
                                if n > 0 {
                                    let gx = (sx % 4) as usize * 32 + xx;
                                    let gy = (sy % 4) as usize * 32 + yy;
                                    values[gy * 128 + gx] = (sum / n as f64) as f32;
                                }
                            }
                        }
                    }
                    store(&c, key, &values)?;
                }
                done += 1;
                if done % 16 == 0 {
                    ensure!(progress(done, total), "DEM閲覧の準備を中断しました");
                }
            }
            for z in (6..12).rev() {
                let mut keys = self
                    .catalog
                    .keys()
                    .filter(|k| k.0 == z)
                    .copied()
                    .collect::<Vec<_>>();
                keys.sort_unstable();
                for key in keys {
                    if read(&c, key)?.is_none() {
                        let mut values = vec![f32::NAN; 128 * 128];
                        for dy in 0..2 {
                            for dx in 0..2 {
                                if let Some(child) =
                                    read(&c, (z + 1, key.1 * 2 + dx, key.2 * 2 + dy))?
                                {
                                    for y in 0..64 {
                                        for x in 0..64 {
                                            let mut sum = 0f64;
                                            let mut n = 0;
                                            for yy in 0..2 {
                                                for xx in 0..2 {
                                                    let h = child[(y * 2 + yy) * 128 + x * 2 + xx];
                                                    if h.is_finite() {
                                                        sum += h as f64;
                                                        n += 1;
                                                    }
                                                }
                                            }
                                            if n > 0 {
                                                values[(dy as usize * 64 + y) * 128
                                                    + dx as usize * 64
                                                    + x] = (sum / n as f64) as f32;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        store(&c, key, &values)?;
                    }
                    done += 1;
                    if done % 16 == 0 {
                        ensure!(progress(done, total), "DEM閲覧の準備を中断しました");
                    }
                }
            }
            c.execute("INSERT OR REPLACE INTO meta VALUES('complete','true')", [])?;
            c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        }
        self.overviews = Some(c);
        Ok(())
    }
    fn core(&mut self, key: Key) -> Result<Option<Arc<Vec<f32>>>> {
        if let Some(v) = self.tiles.get(&key) {
            return Ok(Some(v.clone()));
        }
        let values = if key.0 == 14 {
            self.raw.tile(key.1 as u32, key.2 as u32)?
        } else {
            read(
                self.overviews.as_ref().context("縮小キャッシュ未準備")?,
                key,
            )?
        };
        let Some(v) = values else {
            return Ok(None);
        };
        let v = Arc::new(v);
        self.tiles.insert(key, v.clone());
        self.order.push_back(key);
        while self.tiles.len() > 64 {
            if let Some(k) = self.order.pop_front() {
                self.tiles.remove(&k);
            }
        }
        Ok(Some(v))
    }
    pub fn packet(&mut self, key: Key) -> Result<Arc<Packet>> {
        ensure!(self.catalog.contains_key(&key), "未収録のDEMです");
        self.prepare(&|_, _| true)?;
        let n = if key.0 == 13 { 512 } else { 128 };
        let source_zoom = if key.0 == 13 { 14 } else { key.0 };
        let source_side = if key.0 == 13 { 256 } else { 128 };
        let source_x = if key.0 == 13 { key.1 * 2 } else { key.1 };
        let source_y = if key.0 == 13 { key.2 * 2 } else { key.2 };
        let mut values = vec![f32::NAN; (n + 2) * (n + 2)];
        let mut loaded = HashMap::new();
        for y in -1i32..=n as i32 {
            for x in -1i32..=n as i32 {
                let sx = source_x + x.div_euclid(source_side);
                let sy = source_y + y.div_euclid(source_side);
                let sk = (source_zoom, sx, sy);
                if let std::collections::hash_map::Entry::Vacant(e) = loaded.entry(sk) {
                    e.insert(if sx < 0 || sy < 0 {
                        None
                    } else {
                        self.core(sk)?
                    });
                }
                if let Some(a) = &loaded[&sk] {
                    values[((y + 1) as usize) * (n + 2) + (x + 1) as usize] =
                        a[(y.rem_euclid(source_side) * source_side + x.rem_euclid(source_side))
                            as usize];
                }
            }
        }
        let b = bounds(key);
        let grid = Arc::new(Grid {
            cells: n as u32,
            values,
        });
        Ok(Arc::new(Packet {
            dem_grid: Some(grid),
            key,
            origin: [b[0], b[1]],
            side: b[2] - b[0],
            sections: std::array::from_fn(|_| Vec::new()),
            index_bounds: [0., 1., n as f32, 1.],
            controls: 0,
            compressed_bytes: 0,
            density: vec![(n as u32 / 32).max(1).pow(2); 1024],
            vertices: Vec::new(),
            triangles: Vec::new(),
            timings_ms: [0.; 4],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Result<(PathBuf, Source)> {
        let path = std::env::temp_dir().join(format!(
            "dem-preview-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let db = Connection::open(&path)?;
        db.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE tiles(x INTEGER,y INTEGER,sha256 TEXT,payload BLOB,PRIMARY KEY(x,y));")?;
        let identity = serde_json::json!({"format":"smoothed-dem-cache-v1","mode":"ridge-preserving-gaussian-v2","pixelEncoding":"zstd-5bytes-int32-centimetres-uint8-protected","missingCentimetres":i32::MIN});
        db.execute(
            "INSERT INTO meta VALUES('identity',?1)",
            [identity.to_string()],
        )?;
        db.execute("INSERT INTO meta VALUES('complete','true')", [])?;
        use sha2::{Digest, Sha256};
        for tx in 100..102 {
            let mut raw = Vec::new();
            for y in 0..256 {
                for x in 0..256 {
                    let cm = 100000 + (tx - 100) * 25600 + x * 100 + y * 200;
                    raw.extend((cm as i32).to_le_bytes());
                    raw.push(0);
                }
            }
            let bytes = zstd::bulk::compress(&raw, 3)?;
            db.execute(
                "INSERT INTO tiles VALUES(?1,100,?2,?3)",
                params![tx, format!("{:x}", Sha256::digest(&bytes)), bytes],
            )?;
        }
        drop(db);
        let source = Source::from_raw(dem_cache::Source::try_open(&path)?.unwrap());
        Ok((path, source))
    }
    #[test]
    fn raw_pixels_and_cross_tile_edges_are_preserved() -> Result<()> {
        let (path, mut source) = fixture()?;
        source.prepare(&|_, _| true)?;
        let packet = source.packet((13, 50, 50))?;
        let grid = packet.dem_grid.as_ref().unwrap();
        assert_eq!(grid.cells, 512);
        for (x, y) in [(0, 0), (100, 100), (255, 20), (256, 20), (511, 255)] {
            let h = grid
                .sample((x as f64 + 0.5) / 512., (y as f64 + 0.5) / 512.)
                .unwrap();
            assert!((h - (1000. + x as f64 + y as f64 * 2.)).abs() < 0.001);
        }
        assert!((grid.sample(0.5, 20.5 / 512.).unwrap() - 1295.5).abs() < 0.001);
        assert!(grid.sample(0.5, 0.75).is_none());
        let cache = source.cache_path.clone();
        drop(source);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(cache);
        Ok(())
    }
    #[test]
    fn source_is_read_only_and_rejects_incomplete_or_corrupt_tiles() -> Result<()> {
        let (path, source) = fixture()?;
        assert!(
            source
                .raw
                .connection
                .execute("UPDATE meta SET value='false' WHERE key='complete'", [])
                .is_err()
        );
        let writer = Connection::open(&path)?;
        writer.execute("UPDATE meta SET value='false' WHERE key='complete'", [])?;
        assert!(dem_cache::Source::try_open(&path).is_err());
        writer.execute("UPDATE meta SET value='true' WHERE key='complete'", [])?;
        writer.execute("UPDATE tiles SET payload=zeroblob(10) WHERE x=100", [])?;
        assert!(source.raw.tile(100, 100).is_err());
        drop(writer);
        drop(source);
        let _ = std::fs::remove_file(path);
        Ok(())
    }
    #[test]
    fn shader_is_valid_and_missing_values_do_not_become_sea_level() {
        let module = naga::front::wgsl::parse_str(include_str!("dem_grid_gpu.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let mut grid = Grid {
            cells: 2,
            values: vec![100.; 16],
        };
        grid.values[5] = f32::NAN;
        assert!(grid.sample(0.25, 0.25).is_none());
        assert_eq!(grid.sample(0.75, 0.75), Some(100.));
    }
    #[test]
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの標高格子検査。ローカルで明示実行する"
    )]
    fn gpu_grid_matches_source_heights_and_missing_mask() -> Result<()> {
        use wgpu::util::DeviceExt;
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .context("GPUアダプターなし")?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("DEM格子検査"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
            },
            None,
        ))?;
        let mut grid = Grid {
            cells: 2,
            values: (0..16)
                .map(|k| 100. + (k % 4) as f32 + 2. * (k / 4) as f32)
                .collect(),
        };
        grid.values[5] = f32::NAN;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("DEM格子検査"),
            source: wgpu::ShaderSource::Wgsl(include_str!("dem_grid_gpu.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("DEM格子検査"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&grid.values),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let n = 8u32;
        let bytes = u64::from((n + 1) * (n + 1)) * 32;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let result = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut params = Vec::new();
        for v in [0f32, 0., 1., 1., 0., 1., 2., 1.] {
            params.extend(v.to_le_bytes());
        }
        for v in [n, 0, 0, 0] {
            params.extend(v.to_le_bytes());
        }
        for v in [1000f32, 1., 1000., 0.] {
            params.extend(v.to_le_bytes());
        }
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &params,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(2, 2, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &result, 0, bytes);
        queue.submit([encoder.finish()]);
        let slice = result.slice(..);
        let (send, recv) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| {
            let _ = send.send(v);
        });
        let _ = device.poll(wgpu::Maintain::Wait);
        recv.recv()??;
        let data = slice.get_mapped_range();
        let records = bytemuck::cast_slice::<u8, f32>(&data);
        for y in 0..=n {
            for x in 0..=n {
                let record = &records[((y * (n + 1) + x) * 8) as usize..];
                match grid.sample(x as f64 / n as f64, y as f64 / n as f64) {
                    Some(h) => {
                        assert_eq!(record[3], 1.);
                        assert!((record[0] as f64 - h).abs() < 0.001);
                    }
                    None => assert_eq!(record[3], 0.),
                }
            }
        }
        drop(data);
        result.unmap();
        Ok(())
    }
}
