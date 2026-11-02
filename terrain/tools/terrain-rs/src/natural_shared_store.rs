//! 全国点集合をSQLiteへ保持し、外接円の全域照合で局所接続を証明する。
//! 固定余白の打切りを認めず、支持集合が予算を超えた場合は失敗する。
use crate::natural_regional_surface::Fast;
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub struct Store {
    pub db: Connection,
    pub limit: usize,
}
pub struct Patch {
    pub model: Fast,
    pub ids: Vec<i64>,
    pub epoch: i64,
    pub origin: [f64; 2],
}
impl Patch {
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
        Ok(Self { db, limit })
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
        Ok(changed)
    }
    pub fn set_active(&self, z: i64, id: i64, enabled: bool) -> Result<()> {
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
        Ok(())
    }
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
    fn query(&self, z: i64, b: [f64; 4], out: &mut BTreeMap<i64, [f64; 3]>) -> Result<()> {
        let mut s=self.db.prepare("SELECT p.id,p.x,p.y,p.h FROM point_index i CROSS JOIN points p ON p.id=i.id CROSS JOIN active a ON a.id=p.id AND a.z=? WHERE i.x1>=? AND i.y1>=? AND i.x0<=? AND i.y0<=?")?;
        let mut rows = s.query(params![z, b[0] * 16., b[1] * 16., b[2] * 16., b[3] * 16.])?;
        while let Some(r) = rows.next()? {
            let id = r.get(0)?;
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
        ids: &BTreeSet<i64>,
        faces: &BTreeSet<usize>,
        out: &mut BTreeMap<i64, [f64; 3]>,
    ) -> Result<()> {
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
                let id = r.get(0)?;
                if ids.contains(&id) || out.contains_key(&id) {
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
    pub fn patch(&self, z: i64, b: [f64; 4]) -> Result<Patch> {
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
                points.insert(i, p);
            }
        }
        let padding = ((b[2] - b[0]).max(b[3] - b[1]) * 0.05).max(20.);
        self.query(
            z,
            [
                b[0] - padding,
                b[1] - padding,
                b[2] + padding,
                b[3] + padding,
            ],
            &mut points,
        )?;
        let mut iteration = 0u64;
        loop {
            iteration += 1;
            if iteration % 64 == 0 {
                eprintln!("支持閉包 z{z} 反復{iteration} 読込{}点", points.len());
            }
            let ids = points.keys().copied().collect::<Vec<_>>();
            let m = Fast::certified(
                points
                    .values()
                    .map(|p| [p[0] - origin[0], p[1] - origin[1], p[2]])
                    .collect(),
            )?;
            // 問合せの自然近傍に加えて傾き計算用の隣接面まで証明する。
            let support = m.base.support_vertices(local_b);
            let mut faces = BTreeSet::new();
            for i in support {
                faces.extend(m.base.incident[i].iter().copied());
            }
            let mut extra = BTreeMap::new();
            self.missing(
                z,
                &m,
                origin,
                &ids.iter().copied().collect(),
                &faces,
                &mut extra,
            )?;
            if extra.is_empty() {
                ensure!(
                    epoch == self.epoch(z)?,
                    "支持証明中に点集合が変更されました"
                );
                return Ok(Patch {
                    model: m,
                    ids,
                    epoch,
                    origin,
                });
            }
            points.extend(extra);
        }
    }
    pub fn persist_patch(&self, z: i64, key: &str, p: &Patch) -> Result<()> {
        ensure!(self.epoch(z)? == p.epoch, "古い接続ページは保存できません");
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
        self.db.execute(
            "INSERT OR REPLACE INTO pages VALUES(?,?,?,?)",
            params![z, key, p.epoch, blob],
        )?;
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
    fn robust_circle_orientation_and_cocircular() {
        let a = [0., 0., 0.];
        let b = [2., 0., 0.];
        let c = [0., 2., 0.];
        assert!(circle_contains(a, b, c, [1., 1., 0.]));
        assert!(circle_contains(a, c, b, [1., 1., 0.]));
        assert!(circle_contains(a, b, c, [2., 2., 0.]));
        assert!(!circle_contains(a, b, c, [3., 3., 0.]));
    }
}
