use crate::natural_codec;
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

pub const WORLD: f64 = 40075016.68557849;
pub const ENCODING: &str = "natural-controls-v1";
pub const LEVELS: [(u32, f64); 8] = [
    (6, 500.),
    (7, 400.),
    (8, 300.),
    (9, 250.),
    (10, 200.),
    (11, 125.),
    (12, 60.),
    (13, 25.),
];
pub type Key = (u32, u32, u32);

#[derive(Clone, Debug)]
pub struct Tile {
    pub origin: [i32; 2],
    pub count: usize,
    pub parents: Vec<Key>,
    pub hash: [u8; 32],
    pub controls: Vec<u8>,
}
impl Tile {
    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.count > 0
                && self.count <= 262144
                && self.parents.len() <= 4
                && self.controls.len() <= 16777216,
            "地形タイルの上限超過"
        );
        let mut bytes = b"NTS1".to_vec();
        for v in self.origin {
            bytes.extend(v.to_le_bytes());
        }
        bytes.extend((self.count as u32).to_le_bytes());
        bytes.extend((self.parents.len() as u32).to_le_bytes());
        bytes.extend(self.hash);
        for (z, x, y) in &self.parents {
            for v in [z, x, y] {
                bytes.extend(v.to_le_bytes());
            }
        }
        bytes.extend((self.controls.len() as u32).to_le_bytes());
        bytes.extend(&self.controls);
        Self::decode(&bytes)?;
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() >= 56 && bytes.len() <= 16777320 && &bytes[..4] == b"NTS1",
            "地形タイルの形式・長さが不正です"
        );
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let origin = [
            i32::from_le_bytes(bytes[4..8].try_into()?),
            i32::from_le_bytes(bytes[8..12].try_into()?),
        ];
        let count = u32_at(12) as usize;
        let parent_count = u32_at(16) as usize;
        ensure!(
            count > 0
                && count <= 262144
                && parent_count <= 4
                && bytes.len() >= 56 + parent_count * 12,
            "地形タイルの点数・親数が不正です"
        );
        let mut parents = Vec::new();
        for i in 0..parent_count {
            let at = 52 + i * 12;
            let key = (u32_at(at), u32_at(at + 4), u32_at(at + 8));
            ensure!(
                key.0 <= 30 && key.1 < (1u32 << key.0) && key.2 < (1u32 << key.0),
                "親タイル座標が不正です"
            );
            parents.push(key);
        }
        let at = 52 + parent_count * 12;
        let length = u32_at(at) as usize;
        ensure!(
            length <= 16777216 && at + 4 + length == bytes.len() && length >= 24,
            "制御点BLOBの長さが不正です"
        );
        let controls = bytes[at + 4..].to_vec();
        ensure!(
            &controls[..4] == b"NQC1" || &controls[..4] == b"NDC1",
            "制御点形式が未対応です"
        );
        ensure!(
            u32::from_le_bytes(controls[4..8].try_into()?) as usize == count
                && u32::from_le_bytes(controls[8..12].try_into()?) as usize == parent_count,
            "制御点ヘッダーと外側ヘッダーが不一致です"
        );
        if &controls[..4] == b"NQC1" {
            ensure!(parents.is_empty(), "独立タイルに親参照があります");
        }
        Ok(Self {
            origin,
            count,
            parents,
            hash: bytes[20..52].try_into()?,
            controls,
        })
    }
}

pub fn point_hash(points: &[[i32; 3]]) -> [u8; 32] {
    let mut hash = Sha256::new();
    for p in points {
        for v in p {
            hash.update(v.to_le_bytes());
        }
    }
    hash.finalize().into()
}

pub struct Database {
    pub connection: Connection,
    pub dataset: i64,
    pub generation: String,
    pub credits: String,
    levels: HashMap<u32, i64>,
    pub catalog: HashMap<Key, usize>,
    cache: HashMap<Key, Arc<Vec<[i32; 3]>>>,
}
impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_internal(path, true)
    }
    /// 閲覧用索引のキャッシュを利用する呼出し元向け。XYZの復号・検査は通常と同じ。
    pub fn open_metadata(path: &Path) -> Result<Self> {
        Self::open_internal(path, false)
    }
    fn open_internal(path: &Path, read_index: bool) -> Result<Self> {
        let connection = yamamuki_data_sqlite::open(path)?;
        let sets = yamamuki_data_sqlite::datasets(&connection)?;
        let sets = sets
            .iter()
            .filter(|d| d.kind == "terrain" && d.format_version == 1)
            .collect::<Vec<_>>();
        ensure!(
            sets.len() == 1,
            "対応する地形データセットを1件収録したSQLiteを指定してください"
        );
        let credits = sets[0].metadata["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|source| source["attribution"].as_str())
            .collect::<Vec<_>>()
            .join(" ／ ");
        let dataset = sets[0].key;
        let generation = sets[0].data_version.clone();
        let mut levels = HashMap::new();
        {
            let mut query = connection
                .prepare("SELECT z,level_key FROM terrain_levels WHERE dataset_key=?1")?;
            let rows = query.query_map([dataset], |r| {
                Ok((r.get::<_, u32>(0)?, r.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (z, key) = row?;
                ensure!(
                    z <= 30 && levels.insert(z, key).is_none(),
                    "地形のズームが不正または重複しています"
                );
            }
        }
        let catalog = if read_index {
            Self::read_index(&connection, dataset)?
        } else {
            HashMap::new()
        };
        Ok(Self {
            connection,
            dataset,
            generation,
            credits,
            levels,
            catalog,
            cache: HashMap::new(),
        })
    }
    fn read_index(connection: &Connection, dataset: i64) -> Result<HashMap<Key, usize>> {
        let mut catalog = HashMap::new();
        {
            let mut stmt=connection.prepare("SELECT l.z,t.x,t.y,t.encoding,substr(t.payload,1,20) FROM terrain_tiles t JOIN terrain_levels l USING(dataset_key,level_key) WHERE dataset_key=?1")?;
            let mut rows = stmt.query([dataset])?;
            while let Some(row) = rows.next()? {
                let key = (row.get(0)?, row.get(1)?, row.get(2)?);
                ensure!(
                    row.get::<_, String>(3)? == ENCODING,
                    "未対応の地形圧縮形式です"
                );
                let header: Vec<u8> = row.get(4)?;
                ensure!(
                    header.len() == 20 && &header[..4] == b"NTS1",
                    "地形タイルヘッダーが不正です"
                );
                ensure!(
                    key.0 <= 30 && key.1 < (1u32 << key.0) && key.2 < (1u32 << key.0),
                    "地形タイル座標が範囲外です"
                );
                let count = u32::from_le_bytes(header[12..16].try_into()?) as usize;
                ensure!(count > 0 && count <= 262144, "制御点数が上限超過です");
                ensure!(
                    catalog.insert(key, count).is_none(),
                    "地形ズームが重複しています"
                );
            }
        }
        ensure!(!catalog.is_empty(), "地形タイルがありません");
        Ok(catalog)
    }
    pub fn load_catalog(&mut self) -> Result<()> {
        self.catalog = Self::read_index(&self.connection, self.dataset)?;
        Ok(())
    }
    pub fn use_catalog(&mut self, catalog: HashMap<Key, usize>) -> Result<()> {
        ensure!(!catalog.is_empty(), "地形タイルがありません");
        for (&(z, x, y), &points) in &catalog {
            ensure!(
                z <= 30
                    && self.levels.contains_key(&z)
                    && x < (1u32 << z)
                    && y < (1u32 << z)
                    && points > 0
                    && points <= 262144,
                "地形索引の値が不正です"
            );
        }
        self.catalog = catalog;
        Ok(())
    }
    pub fn tile_size(&self, key: Key) -> Result<usize> {
        Ok(self.connection.prepare_cached("SELECT length(payload) FROM terrain_tiles WHERE dataset_key=?1 AND level_key=?2 AND x=?3 AND y=?4")?
            .query_row(params![self.dataset,self.levels.get(&key.0).context("ズームがありません")?,key.1,key.2],|r|r.get(0))?)
    }
    pub fn tile(&self, key: Key) -> Result<Tile> {
        let (encoding,bytes):(String,Vec<u8>)=self.connection.query_row("SELECT encoding,payload FROM terrain_tiles WHERE dataset_key=?1 AND level_key=?2 AND x=?3 AND y=?4",params![self.dataset,self.levels.get(&key.0).context("親または指定ズームがありません")?,key.1,key.2],|r|Ok((r.get(0)?,r.get(1)?)))?;
        ensure!(encoding == ENCODING, "未対応の地形圧縮形式です");
        Tile::decode(&bytes)
    }
    pub fn points(&mut self, key: Key) -> Result<Arc<Vec<[i32; 3]>>> {
        if let Some(points) = self.cache.get(&key) {
            return Ok(points.clone());
        }
        self.check_dependencies(key, &mut HashSet::new(), 0)?;
        let result = self.restore(key, &mut HashSet::new(), 0)?;
        if self.cache.values().map(|p| p.len() * 12).sum::<usize>() > 256 * 1024 * 1024 {
            self.cache.retain(|k, _| k.0 == 6);
        }
        Ok(result)
    }
    fn check_dependencies(&self, key: Key, deps: &mut HashSet<Key>, depth: usize) -> Result<()> {
        ensure!(depth <= 2, "地形の差分依存が深すぎます");
        deps.insert(key);
        ensure!(deps.len() <= 16, "地形の依存数が多すぎます");
        let tile = self.tile(key)?;
        for parent in tile.parents {
            ensure!(parent.0 < key.0, "親は粗いズームでなければなりません");
            self.check_dependencies(parent, deps, depth + 1)?;
        }
        Ok(())
    }
    fn restore(
        &mut self,
        key: Key,
        deps: &mut HashSet<Key>,
        depth: usize,
    ) -> Result<Arc<Vec<[i32; 3]>>> {
        if let Some(points) = self.cache.get(&key) {
            return Ok(points.clone());
        }
        ensure!(depth < 3, "地形の差分依存が深すぎます");
        deps.insert(key);
        ensure!(deps.len() <= 16, "地形の依存数が多すぎます");
        let tile = self.tile(key)?;
        let mut parents = Vec::new();
        for parent in tile.parents {
            ensure!(parent.0 < key.0, "親タイルは粗いズームでなければなりません");
            parents.push(self.restore(parent, deps, depth + 1)?.as_ref().clone());
        }
        let points = natural_codec::decode_global(&tile.controls, tile.origin, &parents)?;
        ensure!(
            points.len() == tile.count && point_hash(&points) == tile.hash,
            "地形XYZの点数またはSHA-256が一致しません"
        );
        let points = Arc::new(points);
        self.cache.insert(key, points.clone());
        Ok(points)
    }
    pub fn validate_all(&mut self) -> Result<usize> {
        let mut keys = self.catalog.keys().copied().collect::<Vec<_>>();
        keys.sort_unstable();
        for (i, key) in keys.iter().enumerate() {
            self.points(*key)
                .with_context(|| format!("地形タイル {:?}", key))?;
            if (i + 1) % 1024 == 0 {
                eprintln!("XYZ照合 {}/{}", i + 1, keys.len());
            }
        }
        Ok(keys.len())
    }
}
