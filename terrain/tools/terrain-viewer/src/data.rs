use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags};
use std::{collections::HashMap, path::Path, sync::Arc};

#[path = "../../terrain-rs/src/parent_vertex_reference.rs"]
#[allow(dead_code)]
mod parent_vertex_reference;
#[path = "../../terrain-rs/src/triangle_frontier32.rs"]
#[allow(dead_code)]
mod triangle_frontier32;

pub const WORLD: f64 = 40075016.68557849;
pub const UNIT: f64 = WORLD / 268435456.0;
pub type Key = (u32, i32, i32);
pub type Point = [f64; 3];
pub struct Tile {
    pub key: Key,
    pub points: Arc<Vec<Point>>,
    pub faces: Vec<[usize; 3]>,
}
pub struct Database {
    db: Connection,
    pub catalog: HashMap<Key, usize>,
    cache: HashMap<Key, Arc<Vec<Point>>>,
    pub roots: Vec<Arc<Tile>>,
}
fn u32at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
pub fn bounds((z, x, y): Key) -> [f64; 4] {
    let s = 16384.0 * 2f64.powi(14 - z as i32);
    [
        x as f64 * s,
        y as f64 * s,
        (x + 1) as f64 * s,
        (y + 1) as f64 * s,
    ]
}
impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let catalog = {
            let mut stmt = db.prepare("SELECT z,tile_x,tile_y,triangle_count FROM tiles")?;
            stmt.query_map([], |r| Ok(((r.get(0)?, r.get(1)?, r.get(2)?), r.get(3)?)))?
                .collect::<rusqlite::Result<HashMap<_, _>>>()?
        };
        ensure!(!catalog.is_empty(), "地形タイルがありません");
        let mut result = Self {
            db,
            catalog,
            cache: HashMap::new(),
            roots: Vec::new(),
        };
        let mut keys: Vec<_> = result
            .catalog
            .keys()
            .filter(|k| k.0 == 6)
            .copied()
            .collect();
        keys.sort();
        for key in keys {
            let tile = result.tile(key)?;
            result.roots.push(Arc::new(tile));
        }
        Ok(result)
    }
    pub fn points(&mut self, key: Key) -> Result<Arc<Vec<Point>>> {
        if let Some(v) = self.cache.get(&key) {
            return Ok(v.clone());
        }
        let bytes: Vec<u8> = self
            .db
            .query_row(
                "SELECT vertices FROM tiles WHERE z=? AND tile_x=? AND tile_y=?",
                key,
                |r| r.get(0),
            )
            .context("頂点の取得")?;
        ensure!(
            bytes.len() >= 32 && &bytes[..4] == b"YMV4" && bytes[4..6] == [4, 0],
            "YMV4ヘッダー不正"
        );
        let (n, d, r, a, p) = (
            u32at(&bytes, 8) as usize,
            u32at(&bytes, 12) as usize,
            u32at(&bytes, 16) as usize,
            u32at(&bytes, 20) as usize,
            u32at(&bytes, 24) as usize,
        );
        let mode = u16::from_le_bytes(bytes[6..8].try_into()?);
        ensure!(
            mode <= 1
                && d <= 256
                && n == r + a
                && u32at(&bytes, 28) == 0
                && bytes.len() == 32 + 12 * d + p + 6 * a
                && (6..=14).contains(&key.0),
            "YMV4長さ不正"
        );
        let mut parents = Vec::new();
        for i in 0..d {
            let at = 32 + 12 * i;
            let z = u32at(&bytes, at);
            ensure!(z + 1 == key.0, "親レベル不正");
            let x = i32::from_le_bytes(bytes[at + 4..at + 8].try_into()?);
            let y = i32::from_le_bytes(bytes[at + 8..at + 12].try_into()?);
            parents.push(self.points((z, x, y))?);
        }
        let at = 32 + 12 * d;
        let counts = parents.iter().map(|v| v.len() as u32).collect::<Vec<_>>();
        let refs =
            parent_vertex_reference::decode_stream(mode as u8, &bytes[at..at + p], r, &counts)?;
        let mut out = Vec::with_capacity(n);
        for v in refs {
            let (slot, index) = parent_vertex_reference::decode(v.to_le_bytes(), &counts)?;
            out.push(parents[slot][index as usize]);
        }
        let k = 2f64.powi(14 - key.0 as i32);
        for i in 0..a {
            let at = at + p + 6 * i;
            let x = i16::from_le_bytes(bytes[at..at + 2].try_into()?) as f64;
            let y = i16::from_le_bytes(bytes[at + 2..at + 4].try_into()?) as f64;
            let h = u16::from_le_bytes(bytes[at + 4..at + 6].try_into()?) as i32 - 24000;
            out.push([
                (key.1 as f64 * 16384.0 + x) * k,
                (key.2 as f64 * 16384.0 + y) * k,
                h as f64 * if h < 0 { 0.5 } else { 0.25 },
            ]);
        }
        let out = Arc::new(out);
        self.cache.insert(key, out.clone());
        Ok(out)
    }
    pub fn tile(&mut self, key: Key) -> Result<Tile> {
        let points = self.points(key)?;
        let bytes: Vec<u8> = self.db.query_row(
            "SELECT triangles FROM tiles WHERE z=? AND tile_x=? AND tile_y=?",
            key,
            |r| r.get(0),
        )?;
        ensure!(
            bytes.len() >= 24 && &bytes[..4] == b"YMT3" && bytes[4..8] == [3, 0, 0, 0],
            "YMT3ヘッダー不正"
        );
        let (all, body, len, halo) = (
            u32at(&bytes, 8) as usize,
            u32at(&bytes, 12) as usize,
            u32at(&bytes, 16) as usize,
            u32at(&bytes, 20) as usize,
        );
        ensure!(
            body <= all && bytes.len() == 24 + len + halo,
            "YMT3長さ不正"
        );
        let faces = triangle_frontier32::decode(&bytes[24..24 + len], body, points.len())?;
        Ok(Tile { key, points, faces })
    }
    pub fn trim(&mut self) {
        // z6は常駐。その他は復元済み頂点256MiBを超えたら整理する。
        if self.cache.values().map(|v| v.len() * 24).sum::<usize>() > 256 * 1024 * 1024 {
            self.cache
                .retain(|k, v| k.0 == 6 || Arc::strong_count(v) > 1);
        }
    }
}
