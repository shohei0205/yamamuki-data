//! 全国点集合をSQLiteへ保持し、外接円の全域照合で局所接続を証明する。
//! 固定余白の打切りを認めず、支持集合が予算を超えた場合は失敗する。
#[path = "natural_shared_certificates.rs"]
mod certificates;
use crate::natural_regional_surface::Fast;
use anyhow::{Context, Result, ensure};
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

#[derive(Clone, PartialEq)]
struct IndexedPoint {
    id: i64,
    point: [f64; 3],
}
impl rstar::RTreeObject for IndexedPoint {
    type Envelope = rstar::AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        rstar::AABB::from_point([self.point[0], self.point[1]])
    }
}
impl rstar::PointDistance for IndexedPoint {
    fn distance_2(&self, p: &[f64; 2]) -> f64 {
        (self.point[0] - p[0]).powi(2) + (self.point[1] - p[1]).powi(2)
    }
}
#[derive(Clone)]
struct PointCache {
    z: i64,
    epoch: i64,
    tree: Arc<rstar::RTree<IndexedPoint>>,
}
#[derive(Clone)]
pub struct ReadSnapshot {
    path: PathBuf,
    limit: usize,
    z: i64,
    epoch: i64,
    cache: Option<PointCache>,
    certificates: Arc<certificates::Certificates>,
}
impl ReadSnapshot {
    pub fn open(&self) -> Result<Store> {
        let db = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.execute_batch("BEGIN")?;
        let store = Store {
            db,
            limit: self.limit,
            point_cache: RefCell::new(self.cache.clone()),
            certificates: self.certificates.clone(),
        };
        ensure!(
            store.epoch(self.z)? == self.epoch,
            "読み取り専用スナップショットの世代が変更されました"
        );
        Ok(store)
    }
}
pub struct Store {
    pub db: Connection,
    pub limit: usize,
    point_cache: RefCell<Option<PointCache>>,
    certificates: Arc<certificates::Certificates>,
}
pub struct Patch {
    pub id_index: OnceLock<HashMap<i64, usize>>,
    pub model: Fast,
    pub ids: Vec<i64>,
    pub epoch: i64,
    pub origin: [f64; 2],
    pub hint_face: usize,
    pub point_limit: usize,
    pub certified_bounds: [f64; 4],
}
impl Patch {
    pub fn restore_id(&mut self, k: usize, id: i64) {
        self.ids[k] = id;
        if let Some(index) = self.id_index.get_mut() {
            index.insert(id, k);
        }
    }
    pub fn index_of(&self, id: i64) -> Option<usize> {
        self.id_index
            .get_or_init(|| {
                self.ids
                    .iter()
                    .enumerate()
                    .filter_map(|(k, &v)| (v != 0).then_some((v, k)))
                    .collect()
            })
            .get(&id)
            .copied()
            .filter(|&k| self.ids[k] == id)
    }
    pub fn evaluate(
        &self,
        q: [f64; 2],
        w: &mut crate::natural_regional_surface::Work,
    ) -> Result<f64> {
        self.model
            .evaluate([q[0] - self.origin[0], q[1] - self.origin[1]], w)
    }
    pub fn point(&self, i: usize) -> [f64; 3] {
        let p = self.model.base.points[i];
        [p[0] + self.origin[0], p[1] + self.origin[1], p[2]]
    }
    pub fn local_rect(&self, b: [f64; 4]) -> [f64; 4] {
        [
            b[0] - self.origin[0],
            b[1] - self.origin[1],
            b[2] - self.origin[0],
            b[3] - self.origin[1],
        ]
    }
    pub fn deletion_box(&self, i: usize, margin: f64) -> [f64; 4] {
        let b = self.model.deletion_box(i, margin);
        [
            b[0] + self.origin[0],
            b[1] + self.origin[1],
            b[2] + self.origin[0],
            b[3] + self.origin[1],
        ]
    }
}
fn coord(p: [f64; 3]) -> robust::Coord<f64> {
    robust::Coord { x: p[0], y: p[1] }
}
fn circle_contains(a: [f64; 3], b: [f64; 3], c: [f64; 3], p: [f64; 3]) -> bool {
    let orientation = robust::orient2d(coord(a), coord(b), coord(c));
    let det = robust::incircle(coord(a), coord(b), coord(c), coord(p));
    det == 0. || det.is_sign_positive() == orientation.is_sign_positive()
}
fn bounds_circle(c: [f64; 3]) -> [f64; 4] {
    let r = c[2].sqrt();
    let padding = r * 1e-12 + 0.125;
    [
        c[0] - r - padding,
        c[1] - r - padding,
        c[0] + r + padding,
        c[1] + r + padding,
    ]
}
impl Store {
    pub fn enable_deletion_certificates(&self, z: i64) {
        self.certificates.activate(z);
    }
    pub fn certificate_stats(&self) -> serde_json::Value {
        self.certificates.stats()
    }
    pub fn open(path: &Path, limit: usize) -> Result<Self> {
        ensure!(
            (16..=262144).contains(&limit),
            "局所点数予算は16～262144の範囲です"
        );
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(30))?;
        db.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=FULL;PRAGMA cache_size=-65536;PRAGMA wal_autocheckpoint=4096;
        CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS points(id INTEGER PRIMARY KEY,x INTEGER NOT NULL,y INTEGER NOT NULL,h INTEGER NOT NULL,guard INTEGER NOT NULL DEFAULT 0,UNIQUE(x,y));
        CREATE INDEX IF NOT EXISTS point_guards ON points(guard,id);
        CREATE VIRTUAL TABLE IF NOT EXISTS point_index USING rtree(id,x0,x1,y0,y1);
        CREATE TABLE IF NOT EXISTS active(z INTEGER,id INTEGER,PRIMARY KEY(z,id)) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS epochs(z INTEGER PRIMARY KEY,epoch INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS pages(z INTEGER,key TEXT,epoch INTEGER,payload BLOB,PRIMARY KEY(z,key)) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS audit(z INTEGER,x INTEGER,y INTEGER,epoch INTEGER,valid INTEGER,up REAL,down REAL,PRIMARY KEY(z,x,y)) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS stages(name TEXT PRIMARY KEY,state TEXT,details TEXT) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS angle_banned(z INTEGER,id INTEGER,PRIMARY KEY(z,id)) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS removed(z INTEGER,id INTEGER,PRIMARY KEY(z,id)) WITHOUT ROWID;")?;
        Ok(Self {
            db,
            limit,
            point_cache: RefCell::new(None),
            certificates: Arc::new(certificates::Certificates::new()),
        })
    }
    /// 全国照合用のXY索引。DBを正とし、世代が違えば必ずSQLへ戻す。
    /// 3200万点を超える場合はディスク索引を利用し、無制限には確保しない。
    pub fn cache_level(&self, z: i64) -> Result<()> {
        if self
            .point_cache
            .borrow()
            .as_ref()
            .is_some_and(|c| c.z == z && self.epoch(z).ok() == Some(c.epoch))
        {
            return Ok(());
        }
        // 次の品質の索引を作る前に旧索引を解放し、二つ分のピーク確保を避ける。
        *self.point_cache.borrow_mut() = None;
        let count: i64 = self
            .db
            .query_row("SELECT count(*) FROM active WHERE z=?", [z], |r| r.get(0))?;
        if count > 32_000_000 {
            *self.point_cache.borrow_mut() = None;
            return Ok(());
        }
        let mut stmt = self.db.prepare(
            "SELECT p.id,p.x,p.y,p.h FROM active a JOIN points p ON p.id=a.id WHERE a.z=?",
        )?;
        let points = stmt
            .query_map([z], |r| {
                Ok(IndexedPoint {
                    id: r.get(0)?,
                    point: [
                        r.get::<_, i32>(1)? as f64 / 16.,
                        r.get::<_, i32>(2)? as f64 / 16.,
                        r.get::<_, i32>(3)? as f64 / 8.,
                    ],
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        *self.point_cache.borrow_mut() = Some(PointCache {
            z,
            epoch: self.epoch(z)?,
            tree: Arc::new(rstar::RTree::bulk_load(points)),
        });
        Ok(())
    }
    /// SAVEPOINTの試行削除・ロールバックでも点索引を同じ世代へ追従させる。
    pub fn refresh_cached_point(&self, z: i64, id: i64) -> Result<()> {
        self.certificates.invalidate();
        let epoch = self.epoch(z)?;
        let mut cache = self.point_cache.borrow_mut();
        let Some(c) = cache.as_mut() else {
            return Ok(());
        };
        if c.z != z {
            return Ok(());
        }
        if (c.epoch - epoch).abs() > 1 {
            *cache = None;
            return Ok(());
        }
        let point = self.point(id)?;
        let entry = IndexedPoint { id, point };
        let active = self
            .db
            .query_row(
                "SELECT 1 FROM active WHERE z=? AND id=?",
                params![z, id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Arc::make_mut(&mut c.tree).remove(&entry);
        if active {
            Arc::make_mut(&mut c.tree).insert(entry);
        }
        c.epoch = epoch;
        Ok(())
    }
    pub fn cached_points(&self, z: i64) -> Option<usize> {
        self.point_cache
            .borrow()
            .as_ref()
            .filter(|c| c.z == z)
            .map(|c| c.tree.size())
    }
    pub fn rebuild_cache(&self, z: i64) -> Result<()> {
        self.check_stop()?;
        // 古い索引を先に解放し、再構築中に全国索引を二重保持しない。
        *self.point_cache.borrow_mut() = None;
        self.cache_level(z)
    }
    pub fn epoch(&self, z: i64) -> Result<i64> {
        Ok(self
            .db
            .query_row("SELECT epoch FROM epochs WHERE z=?", [z], |r| r.get(0))
            .optional()?
            .unwrap_or(0))
    }
    pub fn bump(&self, z: i64) -> Result<()> {
        self.db.execute(
            "INSERT INTO epochs VALUES(?,1) ON CONFLICT(z) DO UPDATE SET epoch=epoch+1",
            [z],
        )?;
        Ok(())
    }
    pub fn insert(&self, z: i64, p: [i32; 3], guard: bool) -> Result<bool> {
        self.certificates.invalidate();
        let previous_epoch = self.epoch(z)?;
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO points(x,y,h,guard) VALUES(?,?,?,?)",
            params![p[0], p[1], p[2], guard],
        )?;
        let (id, h): (i64, i32) = tx.query_row(
            "SELECT id,h FROM points WHERE x=? AND y=?",
            params![p[0], p[1]],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(h == p[2], "同じXYへ異なる標高を登録しようとしました");
        tx.execute(
            "INSERT OR IGNORE INTO point_index VALUES(?,?,?,?,?)",
            params![id, p[0], p[0], p[1], p[1]],
        )?;
        let changed = tx.execute("INSERT OR IGNORE INTO active VALUES(?,?)", params![z, id])? > 0;
        if changed {
            tx.execute(
                "INSERT INTO epochs VALUES(?,1) ON CONFLICT(z) DO UPDATE SET epoch=epoch+1",
                [z],
            )?;
        }
        tx.commit()?;
        if changed {
            if self
                .point_cache
                .borrow()
                .as_ref()
                .is_some_and(|c| c.tree.size() >= 32_000_000)
            {
                *self.point_cache.borrow_mut() = None;
            }
            let mut cache = self.point_cache.borrow_mut();
            if let Some(c) = cache.as_mut() {
                if c.z == z && c.epoch == previous_epoch {
                    Arc::make_mut(&mut c.tree).insert(IndexedPoint {
                        id,
                        point: [p[0] as f64 / 16., p[1] as f64 / 16., p[2] as f64 / 8.],
                    });
                    c.epoch = previous_epoch + 1;
                }
            }
        }
        Ok(changed)
    }
    /// 影響円が独立した複数の削除を一括で確定する。点そのものは保存して残す。
    pub fn delete_batch(&self, z: i64, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let epoch = self.epoch(z)?;
        let entries = ids
            .iter()
            .map(|&id| {
                Ok(IndexedPoint {
                    id,
                    point: self.point(id)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let tx = self.db.unchecked_transaction()?;
        {
            let mut delete = tx.prepare("DELETE FROM active WHERE z=? AND id=?")?;
            let mut removed = tx.prepare("INSERT OR IGNORE INTO removed VALUES(?,?)")?;
            for &id in ids {
                ensure!(
                    delete.execute(params![z, id])? == 1,
                    "削除候補が既に変更されています"
                );
                removed.execute(params![z, id])?;
            }
        }
        tx.execute(
            "UPDATE epochs SET epoch=epoch+? WHERE z=?",
            params![ids.len(), z],
        )?;
        tx.commit()?;
        if let Some(c) = self
            .point_cache
            .borrow_mut()
            .as_mut()
            .filter(|c| c.z == z && c.epoch == epoch)
        {
            for p in entries {
                Arc::make_mut(&mut c.tree).remove(&p);
            }
            c.epoch += ids.len() as i64;
        }
        Ok(())
    }
    /// 一つの確定した適合提案を原子的に保存する。FULL同期は提案ごとに一度。
    pub fn insert_batch(&self, z: i64, points: &[[i32; 3]]) -> Result<usize> {
        self.certificates.invalidate();
        if points.is_empty() {
            return Ok(0);
        }
        let epoch = self.epoch(z)?;
        let tx = self.db.unchecked_transaction()?;
        let mut changed = Vec::new();
        {
            let mut put =
                tx.prepare("INSERT OR IGNORE INTO points(x,y,h,guard) VALUES(?,?,?,0)")?;
            let mut find = tx.prepare("SELECT id,h FROM points WHERE x=? AND y=?")?;
            let mut spatial = tx.prepare("INSERT OR IGNORE INTO point_index VALUES(?,?,?,?,?)")?;
            let mut active = tx.prepare("INSERT OR IGNORE INTO active VALUES(?,?)")?;
            for p in points {
                put.execute(params![p[0], p[1], p[2]])?;
                let (id, h): (i64, i32) =
                    find.query_row(params![p[0], p[1]], |r| Ok((r.get(0)?, r.get(1)?)))?;
                ensure!(h == p[2], "同じXYへ異なる標高の提案が衝突しました");
                spatial.execute(params![id, p[0], p[0], p[1], p[1]])?;
                if active.execute(params![z, id])? > 0 {
                    changed.push(IndexedPoint {
                        id,
                        point: [p[0] as f64 / 16., p[1] as f64 / 16., p[2] as f64 / 8.],
                    });
                }
            }
        }
        if !changed.is_empty() {
            tx.execute(
                "INSERT INTO epochs VALUES(?1,?2) ON CONFLICT(z) DO UPDATE SET epoch=epoch+?2",
                params![z, changed.len()],
            )?;
        }
        tx.commit()?;
        let n = changed.len();
        let mut cache = self.point_cache.borrow_mut();
        if cache
            .as_ref()
            .is_some_and(|c| c.tree.size() + n > 32_000_000)
        {
            *cache = None;
        }
        if let Some(c) = cache.as_mut().filter(|c| c.z == z && c.epoch == epoch) {
            for p in changed {
                Arc::make_mut(&mut c.tree).insert(p);
            }
            c.epoch += n as i64;
        }
        Ok(n)
    }
    /// 証明済み領域内への挿入は既知のDelaunay空円の内部で完結する。
    /// 既存点の削除や領域外挿入、別の更新があれば証明を再取得する。
    pub fn insert_cached(&self, z: i64, q: [i32; 3], patch: &mut Patch) -> Result<bool> {
        let xy = [q[0] as f64 / 16., q[1] as f64 / 16.];
        let b = patch.certified_bounds;
        let reusable = self.epoch(z)? == patch.epoch
            && xy[0] >= b[0]
            && xy[0] <= b[2]
            && xy[1] >= b[1]
            && xy[1] <= b[3];
        let changed = self.insert(z, q, false)?;
        if !reusable {
            *patch = self.patch(z, b)?;
        } else if changed {
            let id: i64 = self.db.query_row(
                "SELECT id FROM points WHERE x=? AND y=?",
                params![q[0], q[1]],
                |r| r.get(0),
            )?;
            ensure!(
                patch.ids.len() < self.limit,
                "挿入後の局所点数予算を超過しました"
            );
            patch.model.insert_local_from(
                [
                    xy[0] - patch.origin[0],
                    xy[1] - patch.origin[1],
                    q[2] as f64 / 8.,
                ],
                patch.hint_face,
            )?;
            patch.hint_face = *patch
                .model
                .base
                .incident
                .last()
                .and_then(|v| v.first())
                .ok_or_else(|| anyhow::anyhow!("挿入面がありません"))?;
            patch.id_index.take();
            patch.ids.push(id);
            patch.epoch = self.epoch(z)?;
        }
        Ok(changed)
    }
    /// 領域内の頂点削除は既知の星形領域だけを再三角化する。
    /// 呼出元は採用時のみ世代を確定し、棄却時は旧Patchを保持する。
    #[cfg(test)]
    pub fn removed_patch(&self, z: i64, id: i64, patch: &Patch) -> Result<Patch> {
        ensure!(
            self.epoch(z)? == patch.epoch,
            "削除試行の接続が古くなっています"
        );
        let k = patch
            .ids
            .iter()
            .position(|&v| v == id)
            .ok_or_else(|| anyhow::anyhow!("削除点が支持集合にありません"))?;
        let q = patch.point(k);
        let b = patch.certified_bounds;
        ensure!(
            q[0] >= b[0] && q[1] >= b[1] && q[0] <= b[2] && q[1] <= b[3],
            "証明領域外の点は削除できません"
        );
        Ok(Patch {
            id_index: OnceLock::new(),
            model: Fast::certified(
                patch
                    .model
                    .base
                    .points
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &p)| (i != k).then_some(p))
                    .collect(),
            )?,
            ids: patch.ids.iter().copied().filter(|&v| v != id).collect(),
            epoch: patch.epoch + 1,
            origin: patch.origin,
            hint_face: 0,
            point_limit: self.limit,
            certified_bounds: b,
        })
    }
    pub fn set_active(&self, z: i64, id: i64, enabled: bool) -> Result<()> {
        self.certificates.invalidate();
        let tx = self.db.unchecked_transaction()?;
        let n = if enabled {
            tx.execute("INSERT OR IGNORE INTO active VALUES(?,?)", params![z, id])?
        } else {
            tx.execute("DELETE FROM active WHERE z=? AND id=?", params![z, id])?
        };
        if n > 0 {
            tx.execute(
                "INSERT INTO epochs VALUES(?,1) ON CONFLICT(z) DO UPDATE SET epoch=epoch+1",
                [z],
            )?;
        }
        tx.commit()?;
        self.refresh_cached_point(z, id)?;
        Ok(())
    }
    #[cfg(test)]
    pub fn copy_level(&self, fine: i64, coarse: i64) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        let count: i64 = tx.query_row("SELECT count(*) FROM active WHERE z=?", [coarse], |r| {
            r.get(0)
        })?;
        ensure!(count == 0, "既存の粗品質を上書きしません");
        tx.execute(
            "INSERT INTO active SELECT ?,id FROM active WHERE z=?",
            params![coarse, fine],
        )?;
        tx.execute(
            "INSERT INTO epochs VALUES(?,1) ON CONFLICT(z) DO UPDATE SET epoch=epoch+1",
            [coarse],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn point(&self, id: i64) -> Result<[f64; 3]> {
        Ok(self
            .db
            .query_row("SELECT x,y,h FROM points WHERE id=?", [id], |r| {
                Ok([
                    r.get::<_, i32>(0)? as f64 / 16.,
                    r.get::<_, i32>(1)? as f64 / 16.,
                    r.get::<_, i32>(2)? as f64 / 8.,
                ])
            })?)
    }
    fn query(
        &self,
        z: i64,
        b: [f64; 4],
        omit: Option<i64>,
        out: &mut BTreeMap<i64, [f64; 3]>,
    ) -> Result<()> {
        let cache = self.point_cache.borrow();
        if let Some(c) = cache
            .as_ref()
            .filter(|c| c.z == z && self.epoch(z).ok() == Some(c.epoch))
        {
            let envelope = rstar::AABB::from_corners([b[0], b[1]], [b[2], b[3]]);
            for p in c.tree.locate_in_envelope_intersecting(&envelope) {
                if Some(p.id) == omit {
                    continue;
                }
                out.insert(p.id, p.point);
                ensure!(
                    out.len() <= self.limit,
                    "局所支持集合が点数予算{}を超過。精度を落とさず停止します",
                    self.limit
                );
            }
            return Ok(());
        }
        let mut s=self.db.prepare("SELECT p.id,p.x,p.y,p.h FROM point_index i CROSS JOIN points p ON p.id=i.id CROSS JOIN active a ON a.id=p.id AND a.z=? WHERE i.x1>=? AND i.y1>=? AND i.x0<=? AND i.y0<=?")?;
        let mut rows = s.query(params![z, b[0] * 16., b[1] * 16., b[2] * 16., b[3] * 16.])?;
        while let Some(r) = rows.next()? {
            let id = r.get(0)?;
            if Some(id) == omit {
                continue;
            }
            out.insert(
                id,
                [
                    r.get::<_, i32>(1)? as f64 / 16.,
                    r.get::<_, i32>(2)? as f64 / 16.,
                    r.get::<_, i32>(3)? as f64 / 8.,
                ],
            );
            ensure!(
                out.len() <= self.limit,
                "局所支持集合が点数予算{}を超過。精度を落とさず停止します",
                self.limit
            );
        }
        Ok(())
    }
    /// 外接円内の未読点を逐次探索。全域点配列を作らない。
    fn missing(
        &self,
        z: i64,
        m: &Fast,
        origin: [f64; 2],
        ids: &HashSet<i64>,
        point_ids: &[i64],
        faces: &BTreeSet<usize>,
        omit: Option<i64>,
        query_scale2: f64,
        certified: &mut HashSet<[usize; 3]>,
        out: &mut BTreeMap<i64, [f64; 3]>,
    ) -> Result<()> {
        let mut visited = 0usize;
        let support_batch = std::env::var("TERRAIN_SUPPORT_BATCH")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(512)
            .clamp(64, 4096);
        let legacy_support = std::env::var_os("TERRAIN_LEGACY_SUPPORT_PROOF").is_some();
        let rect_support = std::env::var_os("TERRAIN_RECT_SUPPORT_SCAN").is_some();
        let probe_limit = std::env::var("TERRAIN_SUPPORT_PROBE_LIMIT")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(16);
        let cache = self.point_cache.borrow();
        if let Some(c) = cache
            .as_ref()
            .filter(|c| c.z == z && self.epoch(z).ok() == Some(c.epoch))
        {
            for (face_index, &face) in faces.iter().enumerate() {
                if face_index % 128 == 0 {
                    self.check_stop()?;
                }
                let mut circle = m.base.circles[face];
                circle[0] += origin[0];
                circle[1] += origin[1];
                let b = bounds_circle(circle);
                let tri = &m.base.topology.triangles[3 * face..3 * face + 3];
                let [a, bp, cp] = [
                    m.base.points[tri[0]],
                    m.base.points[tri[1]],
                    m.base.points[tri[2]],
                ];
                let mut key = [tri[0], tri[1], tri[2]];
                key.sort_unstable();
                if certified.contains(&key) {
                    continue;
                }
                // Sterbenz条件で全候補の原点差分が厳密な場合だけ、地域間の平行移動で証明を共有。
                let share = origin[0] > 0.
                    && origin[1] > 0.
                    && b[0] >= origin[0] * 0.5
                    && b[2] <= origin[0] * 2.
                    && b[1] >= origin[1] * 0.5
                    && b[3] <= origin[1] * 2.;
                let mut global_key = [point_ids[tri[0]], point_ids[tri[1]], point_ids[tri[2]]];
                global_key.sort_unstable();
                if share && self.certificates.contains(z, global_key) {
                    certified.insert(key);
                    continue;
                }
                let bounds = rstar::AABB::from_corners([b[0], b[1]], [b[2], b[3]]);
                // 巨大な仮外接円では、任意の512点を取り込む前に最も強い反例を探す。
                // 近傍探索は反例の選択だけに使う。反例がなければ必ず従来の全域照会で証明する。
                let nearest = circle[2] > query_scale2 && !legacy_support;
                if nearest {
                    let mut found = false;
                    for p in c
                        .tree
                        .nearest_neighbor_iter(&[circle[0], circle[1]])
                        .take(probe_limit)
                    {
                        // 候補選択のみ。打切り後は従来の完全照会へ戻る。
                        let d2 =
                            (p.point[0] - circle[0]).powi(2) + (p.point[1] - circle[1]).powi(2);
                        if d2 > circle[2] * (1. + 1e-10) + 1e-6 {
                            break;
                        }
                        if Some(p.id) == omit || ids.contains(&p.id) {
                            continue;
                        }
                        if circle_contains(
                            a,
                            bp,
                            cp,
                            [p.point[0] - origin[0], p.point[1] - origin[1], p.point[2]],
                        ) {
                            out.entry(p.id).or_insert(p.point);
                            found = true;
                            break;
                        }
                    }
                    if found {
                        ensure!(
                            ids.len() + out.len() <= self.limit,
                            "外接円の支持閉包が点数予算を超過しました"
                        );
                        if out.len() >= support_batch {
                            return Ok(());
                        }
                        continue;
                    }
                }
                // 点集合の世代が同じ間、同じ3頂点の空円証明は挿入後も有効。
                // 未挿入のout内候補を除外して得た走査は証明として保存しない。
                let mut has_missing = false;
                let mut closed_except_vertices = true;
                // 取得した反例はBTreeMapのID順で取り込む。
                // 外接円の外接矩形の四隅まで全国走査しない。
                // 既存の矩形余裕より大きい半径余裕を付け、最後は従来どおり堅牢なincircleで判定。
                let radius = circle[2].sqrt();
                let padded_radius = radius + 4. * (radius * 1e-12 + 0.125);
                let candidates: Box<dyn Iterator<Item = &IndexedPoint> + '_> = if rect_support {
                    Box::new(c.tree.locate_in_envelope_intersecting(&bounds))
                } else {
                    Box::new(
                        c.tree
                            .locate_within_distance(
                                [circle[0], circle[1]],
                                padded_radius * padded_radius,
                            )
                            .filter(|p| {
                                p.point[0] >= b[0]
                                    && p.point[0] <= b[2]
                                    && p.point[1] >= b[1]
                                    && p.point[1] <= b[3]
                            }),
                    )
                };
                for p in candidates {
                    visited += 1;
                    if visited % 65536 == 0 {
                        self.check_stop()?;
                    }
                    if Some(p.id) == omit {
                        continue;
                    }
                    if ids.contains(&p.id) {
                        // 共円の4点目等を省略しない。頂点以外の既知点があれば共有証明にはしない。
                        if !global_key.contains(&p.id) {
                            closed_except_vertices = false;
                        }
                        continue;
                    }
                    if out.contains_key(&p.id) {
                        has_missing = true;
                        continue;
                    }
                    if circle_contains(
                        a,
                        bp,
                        cp,
                        [p.point[0] - origin[0], p.point[1] - origin[1], p.point[2]],
                    ) {
                        has_missing = true;
                        out.insert(p.id, p.point);
                        ensure!(
                            ids.len() + out.len() <= self.limit,
                            "外接円の支持閉包が点数予算を超過しました"
                        );
                        if out.len() >= support_batch {
                            return Ok(());
                        }
                        if nearest {
                            break;
                        }
                    }
                }
                if !has_missing {
                    certified.insert(key);
                    if share && omit.is_none() && closed_except_vertices {
                        self.certificates.insert(z, global_key);
                    }
                }
                ensure!(
                    ids.len() + out.len() <= self.limit,
                    "外接円の支持閉包が点数予算を超過しました"
                );
                if out.len() >= support_batch {
                    break;
                }
            }
            return Ok(());
        }
        let mut stmt=self.db.prepare("SELECT p.id,p.x,p.y,p.h FROM point_index i CROSS JOIN points p ON p.id=i.id CROSS JOIN active a ON a.id=p.id AND a.z=? WHERE i.x1>=? AND i.y1>=? AND i.x0<=? AND i.y0<=?")?;
        for &face in faces {
            let mut circle = m.base.circles[face];
            circle[0] += origin[0];
            circle[1] += origin[1];
            let b = bounds_circle(circle);
            let tri = &m.base.topology.triangles[3 * face..3 * face + 3];
            let [a, bp, c] = [
                m.base.points[tri[0]],
                m.base.points[tri[1]],
                m.base.points[tri[2]],
            ];
            let mut rows =
                stmt.query(params![z, b[0] * 16., b[1] * 16., b[2] * 16., b[3] * 16.])?;
            while let Some(r) = rows.next()? {
                visited += 1;
                if visited % 65536 == 0 {
                    self.check_stop()?;
                }
                let id = r.get(0)?;
                if Some(id) == omit || ids.contains(&id) || out.contains_key(&id) {
                    continue;
                }
                let p = [
                    r.get::<_, i32>(1)? as f64 / 16.,
                    r.get::<_, i32>(2)? as f64 / 16.,
                    r.get::<_, i32>(3)? as f64 / 8.,
                ];
                if circle_contains(a, bp, c, [p[0] - origin[0], p[1] - origin[1], p[2]]) {
                    out.insert(id, p);
                    ensure!(
                        ids.len() + out.len() <= self.limit,
                        "外接円の支持閉包が点数予算を超過しました"
                    );
                    // 未確定の巨大な外接円へ含まれる全国点を一括で読まない。
                    // 少数の反例を取り込み、再三角化してから残りの空円を再検証する。
                    if out.len() >= 64 {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }
    /// 同じ世代の読み取り専用接続から領域の証明を並列化する。
    /// 全域の空間インデックスはArcで共有し、SQLへの変更は全ワーカー終了後に行う。
    pub fn read_snapshot(&self, z: i64) -> Result<Option<ReadSnapshot>> {
        let path = self.db.path().unwrap_or("");
        if path.is_empty() || path == ":memory:" {
            return Ok(None);
        }
        let epoch = self.epoch(z)?;
        Ok(Some(ReadSnapshot {
            path: PathBuf::from(path),
            limit: self.limit,
            z,
            epoch,
            certificates: self.certificates.clone(),
            cache: self
                .point_cache
                .borrow()
                .as_ref()
                .filter(|c| c.z == z && c.epoch == epoch)
                .cloned(),
        }))
    }
    /// 密集地ではキャッシュの幾何範囲だけを縮小し、支持閉包の点数予算を確保する。
    /// 省略した領域は通常の全域再証明へ戻すため、品質条件は変わらない。
    pub fn cache_region(&self, z: i64, q: [f64; 2], mut b: [f64; 4]) -> Result<[f64; 4]> {
        let cache = self.point_cache.borrow();
        let Some(c) = cache
            .as_ref()
            .filter(|c| c.z == z && self.epoch(z).ok() == Some(c.epoch))
        else {
            return Ok(b);
        };
        let budget = (self.limit / 4).max(4);
        for _ in 0..32 {
            let padding = ((b[2] - b[0]).max(b[3] - b[1])
                * std::env::var("TERRAIN_SUPPORT_PADDING")
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v >= 0. && *v <= 0.5)
                    .unwrap_or(0.05))
            .max(20.);
            let envelope = rstar::AABB::from_corners(
                [b[0] - padding, b[1] - padding],
                [b[2] + padding, b[3] + padding],
            );
            if c.tree
                .locate_in_envelope_intersecting(&envelope)
                .take(budget + 1)
                .count()
                <= budget
            {
                return Ok(b);
            }
            let dx = (b[2] - b[0]) * 0.25;
            let dy = (b[3] - b[1]) * 0.25;
            b = [q[0] - dx, q[1] - dy, q[0] + dx, q[1] + dy];
        }
        anyhow::bail!("点数予算に適合するキャッシュ範囲を確定できません")
    }
    pub fn patches(&self, z: i64, bounds: &[[f64; 4]]) -> Result<Vec<Patch>> {
        self.patches_omitting(z, &bounds.iter().map(|&b| (b, None)).collect::<Vec<_>>())
    }
    pub fn patches_omitting(
        &self,
        z: i64,
        requests: &[([f64; 4], Option<i64>)],
    ) -> Result<Vec<Patch>> {
        let path = self.db.path().unwrap_or("").to_owned();
        if requests.len() < 2 || path.is_empty() || path == ":memory:" {
            return requests
                .iter()
                .map(|&(b, omit)| self.patch_omitting(z, b, omit))
                .collect();
        }
        let epoch = self.epoch(z)?;
        let snapshot = self
            .point_cache
            .borrow()
            .as_ref()
            .filter(|c| c.z == z && c.epoch == epoch)
            .cloned();
        let limit = self.limit;
        let certificates = self.certificates.clone();
        let result = requests
            .par_iter()
            .map(|&(b, omit)| -> Result<Patch> {
                let db = Connection::open_with_flags(
                    &path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )?;
                db.execute_batch("BEGIN")?;
                let local = Store {
                    db,
                    limit,
                    point_cache: RefCell::new(snapshot.clone()),
                    certificates: certificates.clone(),
                };
                ensure!(
                    local.epoch(z)? == epoch,
                    "並列支持証明の世代が変更されました"
                );
                local.patch_omitting(z, b, omit)
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            self.epoch(z)? == epoch,
            "並列支持証明中に点集合が変更されました"
        );
        Ok(result)
    }
    fn check_stop(&self) -> Result<()> {
        if let Some(path) = self.db.path().filter(|p| !p.is_empty() && *p != ":memory:") {
            ensure!(
                !Path::new(path)
                    .parent()
                    .unwrap()
                    .join("stop-requested")
                    .exists(),
                "停止要求を受けました。支持証明は正本を変更していません"
            );
        }
        Ok(())
    }
    pub fn patch(&self, z: i64, b: [f64; 4]) -> Result<Patch> {
        self.patch_omitting(z, b, None)
    }
    pub fn patch_omitting(&self, z: i64, b: [f64; 4], omit: Option<i64>) -> Result<Patch> {
        let origin = [b[0], b[1]];
        let local_b = [0., 0., b[2] - b[0], b[3] - b[1]];
        let epoch = self.epoch(z)?;
        let mut points = BTreeMap::new();
        {
            let mut s=self.db.prepare("SELECT p.id,p.x,p.y,p.h FROM points p INDEXED BY point_guards CROSS JOIN active a ON a.id=p.id WHERE a.z=? AND p.guard=1")?;
            for r in s.query_map([z], |r| {
                Ok((
                    r.get(0)?,
                    [
                        r.get::<_, i32>(1)? as f64 / 16.,
                        r.get::<_, i32>(2)? as f64 / 16.,
                        r.get::<_, i32>(3)? as f64 / 8.,
                    ],
                ))
            })? {
                let (i, p) = r?;
                if Some(i) != omit {
                    points.insert(i, p);
                }
            }
        }
        let padding = ((b[2] - b[0]).max(b[3] - b[1])
            * std::env::var("TERRAIN_SUPPORT_PADDING")
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0. && *v <= 0.5)
                .unwrap_or(0.05))
        .max(20.);
        self.query(
            z,
            [
                b[0] - padding,
                b[1] - padding,
                b[2] + padding,
                b[3] + padding,
            ],
            omit,
            &mut points,
        )?;
        // 原点番兵へつながる巨大な仮外接円を、近い実点で先に狭める。
        if let Some(c) = self
            .point_cache
            .borrow()
            .as_ref()
            .filter(|c| c.z == z && c.epoch == epoch)
        {
            let center = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
            for p in c
                .tree
                .nearest_neighbor_iter(&center)
                .take((self.limit / 2).min(64))
            {
                if Some(p.id) != omit {
                    points.insert(p.id, p.point);
                }
            }
            ensure!(
                points.len() <= self.limit,
                "初期支持点が局所点数予算を超過しました"
            );
        }
        let mut ids = points.keys().copied().collect::<Vec<_>>();
        let mut known = ids.iter().copied().collect::<HashSet<_>>();
        let mut m = Fast::certified(
            points
                .values()
                .map(|p| [p[0] - origin[0], p[1] - origin[1], p[2]])
                .collect(),
        )?;
        let mut insertion_hint = 0usize;
        let mut certified = HashSet::new();
        let reuse_proofs = std::env::var_os("TERRAIN_REPEAT_SUPPORT_PROOF").is_none();
        let mut iteration = 0u64;
        loop {
            iteration += 1;
            if let Some(path) = self.db.path().filter(|p| !p.is_empty() && *p != ":memory:") {
                ensure!(
                    !Path::new(path)
                        .parent()
                        .unwrap()
                        .join("stop-requested")
                        .exists(),
                    "停止要求を受けました。支持証明は正本を変更していません"
                );
            }
            if iteration % 64 == 0 {
                eprintln!("支持閉包 z{z} 反復{iteration} 読込{}点", ids.len());
            }

            // 問合せの自然近傍に加えて傾き計算用の隣接面まで証明する。
            let mut support = BTreeSet::new();
            for f in m.base.faces_for_rect_walk_from(local_b, insertion_hint)? {
                for &v in &m.base.topology.triangles[f * 3..f * 3 + 3] {
                    support.insert(v);
                    support.extend(m.base.neighbors[v].iter().copied());
                }
            }
            let mut faces = BTreeSet::new();
            for i in support {
                faces.extend(m.base.incident[i].iter().copied());
            }
            let mut extra = BTreeMap::new();
            if !reuse_proofs {
                certified.clear();
            }
            self.missing(
                z,
                &m,
                origin,
                &known,
                &ids,
                &faces,
                omit,
                local_b[2].powi(2) + local_b[3].powi(2),
                &mut certified,
                &mut extra,
            )?;
            if extra.is_empty() {
                ensure!(
                    epoch == self.epoch(z)?,
                    "支持証明中に点集合が変更されました"
                );
                let center = [
                    (local_b[0] + local_b[2]) * 0.5,
                    (local_b[1] + local_b[3]) * 0.5,
                ];
                let hint_face = m.base.locate(center, 0)?.0;
                return Ok(Patch {
                    id_index: OnceLock::new(),
                    model: m,
                    hint_face,
                    point_limit: self.limit,
                    ids,
                    epoch,
                    origin,
                    certified_bounds: b,
                });
            }
            // 全域空円の反例だけを既存Delaunayへ局所挿入する。
            // 反例を得るたびに既知の全点を再三角化しない。
            for (id, q) in extra {
                let q = [q[0] - origin[0], q[1] - origin[1], q[2]];
                let before = m.base.points.len();
                match m.insert_local_from(q, insertion_hint) {
                    Ok(hint) => insertion_hint = hint,
                    Err(_) => {
                        let mut all = m.base.points.clone();
                        if all.len() == before {
                            all.push(q);
                        }
                        m = Fast::certified(all)
                            .with_context(|| format!("支持閉包の再構築 z={z} id={id}"))?;
                        insertion_hint = 0;
                    }
                }
                known.insert(id);
                ids.push(id);
            }
        }
    }
    fn encode_patch(p: &Patch) -> Result<Vec<u8>> {
        let triangles = p
            .model
            .base
            .topology
            .triangles
            .chunks_exact(3)
            .map(|t| [p.ids[t[0]], p.ids[t[1]], p.ids[t[2]]])
            .collect::<Vec<_>>();
        let raw = serde_json::to_vec(&serde_json::json!({"points":p.ids,"triangles":triangles}))?;
        use sha2::Digest;
        let mut blob = b"NTP1".to_vec();
        blob.extend((raw.len() as u64).to_le_bytes());
        blob.extend(sha2::Sha256::digest(&raw));
        blob.extend(zstd::stream::encode_all(raw.as_slice(), 3)?);
        Ok(blob)
    }
    pub fn persist_patch(&self, z: i64, key: &str, p: &Patch) -> Result<()> {
        ensure!(self.epoch(z)? == p.epoch, "古い接続ページは保存できません");
        self.db.execute(
            "INSERT OR REPLACE INTO pages VALUES(?,?,?,?)",
            params![z, key, p.epoch, Self::encode_patch(p)?],
        )?;
        Ok(())
    }
    pub fn persist_patches(&self, z: i64, keys: &[String], patches: &[Patch]) -> Result<()> {
        ensure!(keys.len() == patches.len(), "接続ページ数が異なります");
        let epoch = self.epoch(z)?;
        ensure!(
            patches.iter().all(|p| p.epoch == epoch),
            "古い接続ページは保存できません"
        );
        let blobs = patches
            .par_iter()
            .map(Self::encode_patch)
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            self.epoch(z)? == epoch,
            "接続ページ圧縮中に点集合が変わりました"
        );
        let tx = self.db.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached("INSERT OR REPLACE INTO pages VALUES(?,?,?,?)")?;
            for (key, blob) in keys.iter().zip(blobs) {
                stmt.execute(params![z, key, epoch, blob])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::natural_regional_surface::Work;
    #[test]
    fn disk_patch_matches_whole_and_resume() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "natural-store-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let mut all = Vec::new();
        {
            let s = Store::open(&path, 4096)?;
            for p in [
                [-200, -200, 0],
                [200, -200, 0],
                [200, 200, 0],
                [-200, 200, 0],
            ] {
                s.insert(13, p, true)?;
                all.push([p[0] as f64 / 16., p[1] as f64 / 16., 0.]);
            }
            for y in 0..9 {
                for x in 0..9 {
                    let p = [x * 16, y * 16, (x * x + y * 2) % 17];
                    s.insert(13, p, false)?;
                    all.push([x as f64, y as f64, p[2] as f64 / 8.]);
                }
            }
            let whole = Fast::new(all)?;
            let patch = s.patch(13, [2., 2., 6., 6.])?;
            let mut w = Work::default();
            let mut w2 = Work::default();
            for y in 0..13 {
                for x in 0..13 {
                    let q = [2. + x as f64 / 3., 2. + y as f64 / 3.];
                    assert!(
                        (whole.evaluate(q, &mut w)? - patch.evaluate(q, &mut w2)?).abs() < 1e-9
                    );
                }
            }
            s.persist_patch(13, "test", &patch)?;
            s.copy_level(13, 12)?;
        }
        {
            let s = Store::open(&path, 4096)?;
            assert!(s.epoch(12)? > 0);
            let patch = s.patch(12, [2., 2., 6., 6.])?;
            assert!(!patch.ids.is_empty());
        }
        std::fs::remove_file(path)?;
        Ok(())
    }
    #[test]
    fn cached_insert_matches_global_recertification() -> Result<()> {
        let s = Store::open(Path::new(":memory:"), 4096)?;
        for p in [
            [-16000, -16000, 0],
            [16000, -16000, 0],
            [16000, 16000, 0],
            [-16000, 16000, 0],
        ] {
            s.insert(13, p, true)?;
        }
        // 領域外にも点を置き、支持閉包が全点を読み込まない条件で比較する。
        for y in 0..49 {
            for x in 0..49 {
                s.insert(13, [x * 160, y * 160, ((x * 7 + y * 11) % 31) * 8], false)?;
            }
        }
        let b = [90., 90., 150., 150.];
        s.cache_level(13)?;
        let mut cached = s.patch(13, b)?;
        assert!(cached.ids.len() < 2405);
        for q in [
            [1441, 1441, 88],
            [1697, 1901, 120],
            [2200, 2327, 64],
            [2399, 2399, 200],
        ] {
            s.insert_cached(13, q, &mut cached)?;
            let index = s.point_cache.replace(None);
            let full = s.patch(13, b)?;
            *s.point_cache.borrow_mut() = index;
            let mut a = Work::default();
            let mut c = Work::default();
            for y in 0..21 {
                for x in 0..21 {
                    let p = [90. + x as f64 * 3., 90. + y as f64 * 3.];
                    assert!((cached.evaluate(p, &mut a)? - full.evaluate(p, &mut c)?).abs() < 1e-8);
                }
            }
        }
        for _ in 0..3 {
            let id = cached
                .ids
                .iter()
                .copied()
                .find(|&id| {
                    let q = s.point(id).unwrap();
                    q[0] > 95. && q[0] < 145. && q[1] > 95. && q[1] < 145.
                })
                .unwrap();
            let after = s.removed_patch(13, id, &cached)?;
            s.db.execute_batch("SAVEPOINT trial")?;
            s.db.execute("DELETE FROM active WHERE z=13 AND id=?", [id])?;
            s.bump(13)?;
            s.refresh_cached_point(13, id)?;
            let index = s.point_cache.replace(None);
            let full = s.patch(13, b)?;
            *s.point_cache.borrow_mut() = index;
            let mut a = Work::default();
            let mut c = Work::default();
            for y in 0..21 {
                for x in 0..21 {
                    let p = [90. + x as f64 * 3., 90. + y as f64 * 3.];
                    assert!((after.evaluate(p, &mut a)? - full.evaluate(p, &mut c)?).abs() < 1e-8);
                }
            }
            s.db.execute_batch("ROLLBACK TO trial;RELEASE trial")?;
            s.refresh_cached_point(13, id)?;
            assert_eq!(s.point_cache.borrow().as_ref().unwrap().epoch, cached.epoch);
            s.set_active(13, id, false)?;
            cached = after;
        }
        // キャッシュの外で変更してから挿入すると再証明する。
        s.insert(13, [1000, 1001, 30], false)?;
        s.insert_cached(13, [1801, 1803, 80], &mut cached)?;
        assert_eq!(cached.epoch, s.epoch(13)?);
        Ok(())
    }
    #[test]
    fn parallel_certified_regions_match_serial_after_mutation() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "shared-parallel-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        {
            let s = Store::open(&path, 4096)?;
            for p in [
                [-16000, -16000, 0],
                [16000, -16000, 0],
                [16000, 16000, 0],
                [-16000, 16000, 0],
            ] {
                s.insert(13, p, true)?;
            }
            for y in 0..25 {
                for x in 0..25 {
                    s.insert(13, [x * 160, y * 160, ((x * 7 + y * 11) % 31) * 8], false)?;
                }
            }
            let bounds = [
                [20., 20., 60., 60.],
                [70., 70., 110., 110.],
                [140., 140., 190., 190.],
            ];
            for pass in 0..2 {
                s.cache_level(13)?;
                let parallel = s.patches(13, &bounds)?;
                for (p, b) in parallel.iter().zip(bounds) {
                    let serial = s.patch(13, b)?;
                    assert_eq!(p.ids, serial.ids);
                    let mut a = Work::default();
                    let mut c = Work::default();
                    for y in 0..9 {
                        for x in 0..9 {
                            let q = [
                                b[0] + (b[2] - b[0]) * x as f64 / 8.,
                                b[1] + (b[3] - b[1]) * y as f64 / 8.,
                            ];
                            assert!(
                                (p.evaluate(q, &mut a)? - serial.evaluate(q, &mut c)?).abs() < 1e-8
                            );
                        }
                    }
                }
                if pass == 0 {
                    s.insert(13, [1441, 1443, 152], false)?;
                }
            }
        }
        std::fs::remove_file(path)?;
        Ok(())
    }
    #[test]
    fn virtual_deletion_matches_transactional_global_proof() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "shared-omit-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        {
            let s = Store::open(&path, 4096)?;
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
                    s.insert(13, [x * 160, y * 160, ((x * 7 + y * 11) % 31) * 8], false)?;
                }
            }
            s.cache_level(13)?;
            let ids = [[800, 800], [1600, 1600], [2400, 2400]]
                .into_iter()
                .map(|q| {
                    s.db.query_row(
                        "SELECT id FROM points WHERE x=? AND y=?",
                        params![q[0], q[1]],
                        |r| r.get::<_, i64>(0),
                    )
                })
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let b = [20., 20., 190., 190.];
            let requests = ids.iter().map(|&id| (b, Some(id))).collect::<Vec<_>>();
            let proofs = s.patches_omitting(13, &requests)?;
            let epoch = s.epoch(13)?;
            for (&id, p) in ids.iter().zip(proofs) {
                assert!(!p.ids.contains(&id));
                assert_eq!(s.epoch(13)?, epoch);
                assert!(
                    s.db.query_row("SELECT 1 FROM active WHERE z=13 AND id=?", [id], |_| Ok(()))
                        .is_ok()
                );
                s.db.execute_batch("SAVEPOINT reference_delete")?;
                s.db.execute("DELETE FROM active WHERE z=13 AND id=?", [id])?;
                s.bump(13)?;
                s.refresh_cached_point(13, id)?;
                let expected = s.patch(13, b)?;
                let mut a = Work::default();
                let mut c = Work::default();
                for y in 0..17 {
                    for x in 0..17 {
                        let q = [
                            b[0] + (b[2] - b[0]) * x as f64 / 16.,
                            b[1] + (b[3] - b[1]) * y as f64 / 16.,
                        ];
                        assert!(
                            (p.evaluate(q, &mut a)? - expected.evaluate(q, &mut c)?).abs() < 1e-8
                        );
                    }
                }
                s.db.execute_batch("ROLLBACK TO reference_delete;RELEASE reference_delete")?;
                s.refresh_cached_point(13, id)?;
            }
        }
        std::fs::remove_file(path)?;
        Ok(())
    }
    #[test]
    fn dense_cache_region_shrinks_without_changing_surface() -> Result<()> {
        let s = Store::open(Path::new(":memory:"), 128)?;
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
                s.insert(13, [x * 160, y * 160, ((x * 7 + y * 11) % 31) * 8], false)?;
            }
        }
        s.cache_level(13)?;
        let b = s.cache_region(13, [100., 100.], [0., 0., 200., 200.])?;
        assert!(b[0] <= 100. && b[2] >= 100. && b[1] <= 100. && b[3] >= 100.);
        assert!(b[2] - b[0] < 200.);
        let p = s.patch(13, b)?;
        assert!(p.ids.len() <= 128);
        let all =
            s.db.prepare("SELECT x,y,h FROM points ORDER BY id")?
                .query_map([], |r| {
                    Ok([
                        r.get::<_, i32>(0)? as f64 / 16.,
                        r.get::<_, i32>(1)? as f64 / 16.,
                        r.get::<_, i32>(2)? as f64 / 8.,
                    ])
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        let full = Fast::certified(all)?;
        let mut a = Work::default();
        let mut c = Work::default();
        for y in 0..9 {
            for x in 0..9 {
                let q = [
                    b[0] + (b[2] - b[0]) * x as f64 / 8.,
                    b[1] + (b[3] - b[1]) * y as f64 / 8.,
                ];
                assert!((p.evaluate(q, &mut a)? - full.evaluate(q, &mut c)?).abs() < 1e-8);
            }
        }
        Ok(())
    }
    #[test]
    fn remote_support_closure_matches_whole_triangulation() -> Result<()> {
        let s = Store::open(Path::new(":memory:"), 4096)?;
        let world = 641200267;
        for p in [[0, 0, 0], [world, 0, 0], [world, world, 0], [0, world, 0]] {
            s.insert(13, p, true)?;
        }
        let mut samples = Vec::new();
        for y in 0..32 {
            for x in 0..32 {
                samples.push([
                    560000000 + x * 320,
                    224000000 + y * 320,
                    ((x * 13 + y * 17) % 31) * 8,
                ]);
            }
        }
        s.insert_batch(13, &samples)?;
        s.cache_level(13)?;
        let b = [34999970., 14000010., 35000060., 14000590.];
        let p = s.patch(13, b)?;
        let all =
            s.db.prepare("SELECT x,y,h FROM points ORDER BY id")?
                .query_map([], |r| {
                    Ok([
                        r.get::<_, i32>(0)? as f64 / 16. - b[0],
                        r.get::<_, i32>(1)? as f64 / 16. - b[1],
                        r.get::<_, i32>(2)? as f64 / 8.,
                    ])
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        let whole = Fast::certified(all)?;
        let mut a = Work::default();
        let mut c = Work::default();
        for y in 0..9 {
            for x in 0..9 {
                let q = [
                    b[0] + (b[2] - b[0]) * x as f64 / 8.,
                    b[1] + (b[3] - b[1]) * y as f64 / 8.,
                ];
                assert!(
                    (p.evaluate(q, &mut a)?
                        - whole.evaluate([q[0] - b[0], q[1] - b[1]], &mut c)?)
                    .abs()
                        < 1e-8
                );
            }
        }
        Ok(())
    }
    #[test]
    fn rebalanced_index_keeps_epoch_points_and_surface() -> Result<()> {
        let s = Store::open(Path::new(":memory:"), 4096)?;
        for p in [
            [-16000, -16000, 0],
            [16000, -16000, 0],
            [16000, 16000, 0],
            [-16000, 16000, 0],
        ] {
            s.insert(13, p, true)?;
        }
        s.cache_level(13)?;
        for y in 0..21 {
            for x in 0..21 {
                s.insert(13, [x * 160, y * 160, ((x * 7 + y * 11) % 31) * 8], false)?;
            }
        }
        let b = [50., 50., 150., 150.];
        let before = s.patch(13, b)?;
        let epoch = s.epoch(13)?;
        let count = s.cached_points(13);
        s.rebuild_cache(13)?;
        assert_eq!(s.epoch(13)?, epoch);
        assert_eq!(s.cached_points(13), count);
        let after = s.patch(13, b)?;
        let mut a = Work::default();
        let mut c = Work::default();
        for y in 0..17 {
            for x in 0..17 {
                let q = [50. + x as f64 * 6.25, 50. + y as f64 * 6.25];
                assert!((before.evaluate(q, &mut a)? - after.evaluate(q, &mut c)?).abs() < 1e-8);
            }
        }
        Ok(())
    }
    #[test]
    fn radial_pruning_keeps_robust_circle_candidates() -> Result<()> {
        let origin = [35000000., 14000000.];
        let mut global = vec![
            [0., 0., 0.],
            [40075016., 0., 0.],
            [0., 40075016., 0.],
            [40075016., 40075016., 0.],
        ];
        for y in 0..24 {
            for x in 0..24 {
                global.push([
                    origin[0] + x as f64 * 19. + (y % 3) as f64 * 0.0625,
                    origin[1] + y as f64 * 17.,
                    (x + y) as f64,
                ]);
            }
        }
        // 量子化されたほぼ一直線の帯と、極めて遠い番兵の組合せ。
        for x in 1..64 {
            global.push([
                origin[0] - x as f64 * 71.,
                origin[1] - 100. + (x % 3) as f64 * 0.0625,
                0.,
            ]);
        }
        let model = Fast::certified(
            global
                .iter()
                .map(|p| [p[0] - origin[0], p[1] - origin[1], p[2]])
                .collect(),
        )?;
        let tree = rstar::RTree::bulk_load(
            global
                .iter()
                .enumerate()
                .map(|(i, &point)| IndexedPoint {
                    id: i as i64,
                    point,
                })
                .collect(),
        );
        for size in [0.25, 17., 113., 800.] {
            for p in global.iter().skip(4).step_by(31) {
                let q = [p[0] - origin[0], p[1] - origin[1]];
                let rect = [q[0] - size, q[1] - size, q[0] + size, q[1] + size];
                let expected = model
                    .base
                    .faces_for_rect(rect)
                    .into_iter()
                    .collect::<BTreeSet<_>>();
                let actual = model
                    .base
                    .faces_for_rect_walk_from(rect, model.base.circles.len() / 2)?
                    .into_iter()
                    .collect::<BTreeSet<_>>();
                assert_eq!(actual, expected, "rect={rect:?}");
                let expected = model
                    .base
                    .faces_for_point(q)?
                    .into_iter()
                    .collect::<BTreeSet<_>>();
                let actual = model
                    .base
                    .faces_for_point_from(q, model.base.circles.len() / 2)?
                    .into_iter()
                    .collect::<BTreeSet<_>>();
                assert_eq!(actual, expected);
            }
        }
        for (face, t) in model.base.topology.triangles.chunks_exact(3).enumerate() {
            let mut circle = model.base.circles[face];
            circle[0] += origin[0];
            circle[1] += origin[1];
            let b = bounds_circle(circle);
            let bounds = rstar::AABB::from_corners([b[0], b[1]], [b[2], b[3]]);
            let inside = |p: &&IndexedPoint| {
                circle_contains(
                    model.base.points[t[0]],
                    model.base.points[t[1]],
                    model.base.points[t[2]],
                    [p.point[0] - origin[0], p.point[1] - origin[1], p.point[2]],
                )
            };
            let expected = tree
                .locate_in_envelope_intersecting(&bounds)
                .filter(inside)
                .map(|p| p.id)
                .collect::<BTreeSet<_>>();
            let r = circle[2].sqrt();
            let padded = r + 4. * (r * 1e-12 + 0.125);
            let actual = tree
                .locate_within_distance([circle[0], circle[1]], padded * padded)
                .filter(|p| {
                    p.point[0] >= b[0]
                        && p.point[0] <= b[2]
                        && p.point[1] >= b[1]
                        && p.point[1] <= b[3]
                })
                .filter(inside)
                .map(|p| p.id)
                .collect::<BTreeSet<_>>();
            assert_eq!(actual, expected, "face={face} circle={circle:?}");
        }
        Ok(())
    }
    #[test]
    fn robust_circle_orientation_and_cocircular() {
        let a = [0., 0., 0.];
        let b = [2., 0., 0.];
        let c = [0., 2., 0.];
        assert!(circle_contains(a, b, c, [1., 1., 0.]));
        assert!(circle_contains(a, c, b, [1., 1., 0.]));
        assert!(circle_contains(a, b, c, [2., 2., 0.]));
        assert!(!circle_contains(a, b, c, [3., 3., 0.]));
    }
    #[test]
    fn deletion_circle_certificates_reuse_and_invalidate() -> Result<()> {
        let s = Store::open(Path::new(":memory:"), 8192)?;
        let o = [35000000i32 * 16, 14000000i32 * 16];
        for (x, y) in [(-2000, -2000), (2000, -2000), (2000, 2000), (-2000, 2000)] {
            s.insert(13, [o[0] + x * 16, o[1] + y * 16, 0], true)?;
        }
        for y in 0..20 {
            for x in 0..20 {
                s.insert(
                    13,
                    [
                        o[0] + x * 160 + (y * 7 + x * 3) % 11,
                        o[1] + y * 160 + (x * 5 + y * 3) % 13,
                        (x * x + y * 7) % 23,
                    ],
                    false,
                )?;
            }
        }
        s.cache_level(13)?;
        s.enable_deletion_certificates(13);
        let b = [35000040., 14000040., 35000140., 14000140.];
        let first = s.patch(13, b)?;
        let second = s.patch(13, b)?;
        assert!(s.certificate_stats()["hits"].as_u64().unwrap() > 0);
        let mut a = Work::default();
        let mut c = Work::default();
        for y in 0..9 {
            for x in 0..9 {
                let q = [b[0] + x as f64 * 12., b[1] + y as f64 * 12.];
                assert!((first.evaluate(q, &mut a)? - second.evaluate(q, &mut c)?).abs() < 1e-8);
            }
        }
        s.insert(13, [o[0] + 1001, o[1] + 1003, 17], false)?;
        assert_eq!(s.certificate_stats()["z"].as_i64(), Some(-1));
        assert_eq!(s.certificate_stats()["entries"].as_u64(), Some(0));
        Ok(())
    }
}
