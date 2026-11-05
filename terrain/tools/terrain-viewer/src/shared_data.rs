use crate::{
    natural_codec, natural_local_gradient, natural_regional_gpu, natural_regional_surface::Fast,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
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
}
impl Packet {
    pub fn bytes(&self) -> usize {
        self.sections.iter().map(Vec::len).sum::<usize>()
            + self.vertices.len() * 16
            + self.triangles.len() * 4
    }
}
pub struct Database {
    db: Connection,
    pub catalog: HashMap<Key, usize>,
    pub generation: String,
    points: HashMap<Key, Arc<Vec<[i32; 3]>>>,
    packets: HashMap<Key, Arc<Packet>>,
    order: VecDeque<Key>,
}
impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        db.busy_timeout(std::time::Duration::from_secs(2))?;
        let complete: String = db
            .query_row("SELECT value FROM metadata WHERE key='complete'", [], |r| {
                r.get(0)
            })
            .context("全国共有曲面のSQLiteを指定してください")?;
        ensure!(
            complete == "true",
            "生成・検証が完了したSQLiteを指定してください"
        );
        let generation: String = db.query_row(
            "SELECT value FROM metadata WHERE key='generation_id'",
            [],
            |r| r.get(0),
        )?;
        let catalog = {
            let mut stmt = db.prepare("SELECT z,x,y,points FROM tiles")?;
            stmt.query_map([], |r| Ok(((r.get(0)?, r.get(1)?, r.get(2)?), r.get(3)?)))?
                .collect::<rusqlite::Result<HashMap<_, _>>>()?
        };
        ensure!(catalog.keys().any(|k| k.0 == 6), "z6タイルがありません");
        Ok(Self {
            db,
            catalog,
            generation,
            points: HashMap::new(),
            packets: HashMap::new(),
            order: VecDeque::new(),
        })
    }
    fn restore(
        &mut self,
        key: Key,
        stack: &mut HashSet<Key>,
        deps: &mut HashSet<Key>,
    ) -> Result<Arc<Vec<[i32; 3]>>> {
        if let Some(p) = self.points.get(&key) {
            return Ok(p.clone());
        }
        ensure!(
            stack.len() < 3 && stack.insert(key),
            "親タイル参照が循環、または深さ超過"
        );
        deps.insert(key);
        ensure!(deps.len() <= 16, "タイル依存数が上限超過");
        let (ox, oy, blob, hash): (i32, i32, Vec<u8>, String) = self.db.query_row(
            "SELECT origin_x,origin_y,payload,xyz_sha256 FROM tiles WHERE z=? AND x=? AND y=?",
            key,
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        let parents = {
            let mut stmt=self.db.prepare("SELECT parent_z,parent_x,parent_y FROM parents WHERE z=? AND x=? AND y=? ORDER BY slot")?;
            stmt.query_map(key, |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, i32>(1)?,
                    r.get::<_, i32>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        ensure!(parents.len() <= 4, "直接親タイルが上限超過");
        let mut parent_points = Vec::new();
        for parent in parents {
            parent_points.push(self.restore(parent, stack, deps)?);
        }
        let parent_values = parent_points
            .iter()
            .map(|v| v.as_ref().clone())
            .collect::<Vec<_>>();
        let points = natural_codec::decode_global(&blob, [ox, oy], &parent_values)?;
        let mut digest = Sha256::new();
        for point in &points {
            for value in point {
                digest.update(value.to_le_bytes());
            }
        }
        ensure!(
            digest
                .finalize()
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect::<String>()
                == hash,
            "タイルXYZの検査値が不一致 {:?}",
            key
        );
        stack.remove(&key);
        let points = Arc::new(points);
        self.points.insert(key, points.clone());
        Ok(points)
    }
    pub fn packet(&mut self, key: Key) -> Result<Arc<Packet>> {
        if let Some(packet) = self.packets.get(&key) {
            return Ok(packet.clone());
        }
        let integers = self.restore(key, &mut HashSet::new(), &mut HashSet::new())?;
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
        let gradients = natural_local_gradient::local_gradients(&model);
        let (bytes, info) = natural_regional_gpu::packet(
            &model,
            &gradients,
            [0., 0., side, side],
            serde_json::json!({}),
        )?;
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
        let compressed_bytes = self.db.query_row(
            "SELECT length(payload) FROM tiles WHERE z=? AND x=? AND y=?",
            key,
            |r| r.get(0),
        )?;
        let mut density = vec![0u32; 1024];
        for p in &model.base.points {
            if p[0] >= 0. && p[1] >= 0. && p[0] <= side && p[1] <= side {
                let x = (p[0] / side * 32.).floor().clamp(0., 31.) as usize;
                let y = (p[1] / side * 32.).floor().clamp(0., 31.) as usize;
                density[y * 32 + x] += 1;
            }
        }
        let packet = Arc::new(Packet {
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
        self.order.push_back(key);
        self.packets.insert(key, packet.clone());
        self.trim();
        Ok(packet)
    }
    pub fn cached_keys(&self) -> HashSet<Key> {
        self.packets.keys().copied().collect()
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
        if self.points.values().map(|p| p.len() * 12).sum::<usize>() > 256 * 1024 * 1024 {
            self.points.retain(|k, _| k.0 == 6);
        }
    }
    pub fn check(path: &Path) -> Result<()> {
        let mut db = Self::open(path)?;
        let mut report = Vec::new();
        for z in 6..=13 {
            let key = db
                .catalog
                .keys()
                .filter(|k| k.0 == z)
                .min()
                .copied()
                .context("レベルなし")?;
            let packet = db.packet(key)?;
            report.push(serde_json::json!({"key":key,"controls":packet.controls,"gpu_bytes":packet.bytes()}));
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
        Ok(())
    }
}
