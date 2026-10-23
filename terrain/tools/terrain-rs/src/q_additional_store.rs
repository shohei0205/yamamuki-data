//! 基本メッシュは読み取り専用にし、追加工程の成果と再開情報を別DBへ保存する。
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use std::{fs, path::Path, time::{Duration, UNIX_EPOCH}};

// 全国追加の計算側から、結果と再開位置を同じトランザクションで確定する。
#[allow(dead_code)]
pub fn save_tile(output: &Path, key: [i64;3], vertices: usize, triangles: usize,
    mesh: &[u8], added_vertex_ids: &[u8], checkpoint: &[u8]) -> Result<()> {
    let db=Connection::open_with_flags(output,OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_secs(30))?;
    let tx=db.unchecked_transaction()?;
    let config:String=tx.query_row("SELECT value FROM metadata WHERE key='additional_config'",[],|r|r.get(0))?;
    ensure!(serde_json::from_str::<serde_json::Value>(&config)?["format"]=="YQ-additional-v1","追加工程用ではないSQLiteです");
    let state:i64=tx.query_row("SELECT state FROM jobs WHERE q=?1 AND x=?2 AND y=?3",key,|r|r.get(0))?;
    ensure!(state==0,"未処理ではないタイルへは上書きしません");
    tx.execute("INSERT INTO additional VALUES(?1,?2,?3,?4,?5,?6,?7)",rusqlite::params![key[0],key[1],key[2],vertices,triangles,mesh,added_vertex_ids])?;
    tx.execute("INSERT OR REPLACE INTO checkpoints VALUES('global-pool',?1)",[checkpoint])?;
    tx.execute("UPDATE jobs SET state=1,failure=NULL WHERE q=?1 AND x=?2 AND y=?3",key)?;
    tx.commit()?;Ok(())
}

pub fn prepare(base: &Path, output: &Path) -> Result<()> {
    let base_path = base.canonicalize()?;
    let parent = output.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let target = parent.canonicalize()?.join(output.file_name().context("出力名が必要です")?);
    ensure!(base_path != target && (!output.exists() || base_path != output.canonicalize()?),
        "基本メッシュと追加出力は別ファイルにしてください");
    let source = Connection::open_with_flags(&base_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.execute_batch("BEGIN")?;
    let remaining: u64 = source.query_row("SELECT count(*) FROM jobs WHERE state NOT IN (1,2)", [], |r|r.get(0))?;
    ensure!(remaining == 0, "基本メッシュ生成が未完了です。残り{remaining}タイル");
    let count: u64 = source.query_row("SELECT count(*) FROM basic", [], |r|r.get(0))?;
    let saved: u64 = source.query_row("SELECT count(*) FROM jobs WHERE state=1", [], |r|r.get(0))?;
    ensure!(count > 0 && saved == count, "基本メッシュの保存件数が不正です");
    let manifest: String = source.query_row("SELECT value FROM metadata WHERE key='manifest'", [], |r|r.get(0))?;
    let vertices: u64 = source.query_row("SELECT sum(vertices) FROM basic", [], |r|r.get(0))?;
    let metadata = fs::metadata(&base_path)?;
    let identity = json!({"path":base_path.to_string_lossy(), "bytes":metadata.len(),
        "modifiedNs":metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos() as u64,
        "manifest":serde_json::from_str::<serde_json::Value>(&manifest)?, "tiles":count, "vertices":vertices});
    let config = json!({"format":"YQ-additional-v1", "base":identity,
        "additionalFraction":0.2, "scope":"national-global-pool", "pruningEnabled":false});
    // 基本DBなど、用途の違う既存SQLiteへはスキーマも追加しない。
    let db = Connection::open(output)?;
    db.busy_timeout(Duration::from_secs(30))?;
    let tx = db.unchecked_transaction()?;
    let tables: u64 = tx.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |r|r.get(0))?;
    if tables > 0 {
        let old: String = tx.query_row("SELECT value FROM metadata WHERE key='additional_config'", [], |r|r.get(0))
            .context("追加工程用ではない既存SQLiteへは書き込みません")?;
        ensure!(serde_json::from_str::<serde_json::Value>(&old)? == config,
            "基本メッシュまたは追加条件が異なります。別の出力名を指定してください");
        return Ok(());
    }
    tx.execute_batch("CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT);
        CREATE TABLE jobs(q INTEGER,x INTEGER,y INTEGER,state INTEGER NOT NULL DEFAULT 0,failure TEXT,PRIMARY KEY(q,x,y));
        CREATE TABLE additional(q INTEGER,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER,
            mesh BLOB NOT NULL,added_vertex_ids BLOB NOT NULL,PRIMARY KEY(q,x,y));
        CREATE TABLE checkpoints(name TEXT PRIMARY KEY,payload BLOB NOT NULL);
        CREATE TABLE timings(phase TEXT PRIMARY KEY,elapsed_seconds REAL NOT NULL,processed INTEGER NOT NULL);")?;
    tx.execute("INSERT INTO metadata VALUES('additional_config',?1)", [config.to_string()])?;
    tx.execute("INSERT INTO metadata VALUES('stage','additional-not-started')", [])?;
    tx.execute("INSERT INTO metadata VALUES('complete','false')", [])?;
    tx.execute("INSERT INTO metadata VALUES('distributable','false')", [])?;
    let mut query = source.prepare("SELECT q,x,y FROM basic ORDER BY q,y,x")?;
    let mut rows = query.query([])?;
    let mut insert = tx.prepare("INSERT INTO jobs(q,x,y) VALUES(?1,?2,?3)")?;
    while let Some(row) = rows.next()? {
        insert.execute([row.get::<_,i64>(0)?,row.get(1)?,row.get(2)?])?;
    }
    drop(insert);
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize,Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn fixture() -> (std::path::PathBuf,std::path::PathBuf) {
        let dir=std::env::temp_dir().join(format!("q-additional-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        fs::create_dir_all(&dir).unwrap();let base=dir.join("basic.sqlite3");let out=dir.join("extra.sqlite3");
        let db=Connection::open(&base).unwrap();db.execute_batch("CREATE TABLE metadata(key TEXT,value TEXT);INSERT INTO metadata VALUES('manifest','{}');CREATE TABLE jobs(q,x,y,state);INSERT INTO jobs VALUES(4,1,2,1);CREATE TABLE basic(q,x,y,vertices);INSERT INTO basic VALUES(4,1,2,128);").unwrap();
        (base,out)
    }
    #[test] fn separate_output_preserves_base_and_resume_progress() {
        let (base,out)=fixture();let before=fs::read(&base).unwrap();prepare(&base,&out).unwrap();
        let db=Connection::open(&out).unwrap();db.execute("UPDATE jobs SET state=1",[]).unwrap();drop(db);
        prepare(&base,&out).unwrap();assert_eq!(fs::read(&base).unwrap(),before);
        let db=Connection::open(&out).unwrap();assert_eq!(db.query_row("SELECT state FROM jobs",[],|r|r.get::<_,u64>(0)).unwrap(),1);
        assert_eq!(db.query_row("SELECT count(*) FROM additional",[],|r|r.get::<_,u64>(0)).unwrap(),0);
    }
    #[test] fn same_file_is_rejected_without_changes() {
        let (base,_)=fixture();let before=fs::read(&base).unwrap();assert!(prepare(&base,&base).is_err());assert_eq!(fs::read(&base).unwrap(),before);
    }
    #[test] fn unfinished_base_and_unrelated_output_are_rejected() {
        let (base,out)=fixture();let db=Connection::open(&base).unwrap();db.execute("UPDATE jobs SET state=0",[]).unwrap();drop(db);
        assert!(prepare(&base,&out).is_err());assert!(!out.exists());
        let db=Connection::open(&base).unwrap();db.execute("UPDATE jobs SET state=1",[]).unwrap();drop(db);
        let other=Connection::open(&out).unwrap();other.execute_batch("CREATE TABLE keep(value);INSERT INTO keep VALUES(123)").unwrap();drop(other);
        assert!(prepare(&base,&out).is_err());let db=Connection::open(&out).unwrap();assert_eq!(db.query_row("SELECT value FROM keep",[],|r|r.get::<_,u64>(0)).unwrap(),123);
    }
    #[test] fn result_and_checkpoint_are_separate_from_base() {
        let (base,out)=fixture();prepare(&base,&out).unwrap();let before=fs::read(&base).unwrap();
        save_tile(&out,[4,1,2],130,250,b"packed-mesh",b"added-ids",b"pool-state").unwrap();
        assert!(save_tile(&out,[4,1,2],131,252,b"different",b"ids",b"state").is_err());
        assert!(save_tile(&base,[4,1,2],130,250,b"mesh",b"ids",b"state").is_err());
        assert_eq!(fs::read(&base).unwrap(),before);
        let db=Connection::open(&out).unwrap();assert_eq!(db.query_row("SELECT payload FROM checkpoints",[],|r|r.get::<_,Vec<u8>>(0)).unwrap(),b"pool-state");
        assert_eq!(db.query_row("SELECT vertices FROM additional",[],|r|r.get::<_,usize>(0)).unwrap(),130);
    }
    #[test] fn changed_base_rejects_resume() {
        let (base,out)=fixture();prepare(&base,&out).unwrap();let db=Connection::open(&base).unwrap();db.execute("UPDATE basic SET vertices=129",[]).unwrap();drop(db);assert!(prepare(&base,&out).is_err());
    }
}
