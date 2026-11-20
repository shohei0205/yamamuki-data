//! 完成済みの細レベルを維持し、z6の正本からz5・z4だけを追加生成する。
use super::*;
fn source_identity(source: &Path, baseline: &Path) -> Result<(Value, Vec<u32>, String)> {
    let job: Value = serde_json::from_slice(&fs::read(source.join("job.json"))?)?;
    ensure!(
        job["inputs_verified"] == true,
        "元DEMの確認済み記録が必要です"
    );
    let c = Connection::open_with_flags(
        source.join("work.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let raw: String = c.query_row("SELECT value FROM meta WHERE key='identity'", [], |r| {
        r.get(0)
    })?;
    let identity: Value = serde_json::from_str(&raw)?;
    let window: Vec<u32> = serde_json::from_value(identity["window"].clone())?;
    ensure!(
        window.is_empty() || window.len() == 4,
        "元の生成範囲が不正です"
    );
    let (state, details): (String, String) = c.query_row(
        "SELECT state,details FROM stages WHERE name='certify-6'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let details: Value = serde_json::from_str(&details)?;
    let epoch: i64 = c.query_row("SELECT epoch FROM epochs WHERE z=6", [], |r| r.get(0))?;
    ensure!(
        state == "done" && details["epoch"] == epoch,
        "z6の認証済み正本が必要です"
    );
    let base = yamamuki_terrain::sqlite::Database::open(baseline)?;
    ensure!(
        base.level_formats.len() == 8
            && base
                .level_formats
                .iter()
                .all(|(z, v)| (6..=13).contains(z) && v == yamamuki_terrain::sqlite::ENCODING),
        "基準SQLiteには制御点z6〜z13が必要です"
    );
    let raw: String = base.connection.query_row(
        "SELECT metadata_json FROM datasets WHERE dataset_key=?1",
        [base.dataset],
        |r| r.get(0),
    )?;
    let metadata: Value = serde_json::from_str(&raw)?;
    ensure!(
        metadata["generation"]["id"] == job["generation_id"],
        "作業正本と基準SQLiteの世代が一致しません"
    );
    Ok((job, window, natural_inventory::hash_file(baseline)?))
}
fn check_pilots(pilots: &[PathBuf], engine: &str) -> Result<()> {
    ensure!(
        pilots.len() == 3,
        "全国実行には同じ生成器で完了した三地点の粗レベル試験が必要です"
    );
    let mut windows = BTreeSet::new();
    for p in pilots {
        let v: Value = serde_json::from_slice(&fs::read(p.join("coarse-verification.json"))?)?;
        ensure!(
            v["complete"] == true
                && v["engine_sha256"] == engine
                && v["levels"] == json!([[5, 625.], [4, 1250.]]),
            "粗レベルの事前検証条件が一致しません"
        );
        let w: Vec<u32> = serde_json::from_value(v["window"].clone())?;
        ensure!(
            w.len() == 4 && windows.insert(w),
            "事前試験は異なる三地点が必要です"
        );
    }
    Ok(())
}
fn readonly_uri(path: &Path) -> String {
    let text = path.to_string_lossy();
    let plain = text.strip_prefix("\\\\?\\").unwrap_or(&text);
    let normalized = plain
        .replace('\\', "/")
        .replace('%', "%25")
        .replace(' ', "%20")
        .replace('#', "%23")
        .replace('?', "%3F");
    format!("file:{normalized}?mode=ro")
}
fn seed(source: &Path, s: &Store, baseline: &Path) -> Result<()> {
    if state(s, "seed-source-6")?.is_some() {
        return Ok(());
    }
    let path = source.join("work.sqlite3").canonicalize()?;
    // 元の作業DBは読取専用で接続し、必要なz6の点と番兵だけを新しいストアへ複製する。
    let uri = readonly_uri(&path);
    if s.db
        .query_row("SELECT COUNT(*) FROM points", [], |r| r.get::<_, i64>(0))?
        == 0
    {
        s.db.execute("ATTACH DATABASE ?1 AS seed", [uri])?;
        let tx = s.db.unchecked_transaction()?;
        tx.execute_batch("INSERT INTO points SELECT p.* FROM seed.points p WHERE p.id IN (SELECT id FROM seed.active WHERE z=6 UNION SELECT id FROM seed.points WHERE guard=1); INSERT INTO point_index SELECT i.* FROM points p CROSS JOIN seed.point_index i WHERE i.id=p.id; INSERT INTO active SELECT z,id FROM seed.active WHERE z=6; INSERT INTO epochs SELECT z,epoch FROM seed.epochs WHERE z=6;")?;
        tx.commit()?;
        s.db.execute_batch("DETACH DATABASE seed")?;
    }
    let mut base = yamamuki_terrain::sqlite::Database::open(baseline)?;
    let stored = {
        let mut q=s.db.prepare("SELECT p.x,p.y,p.h FROM points p WHERE p.guard=1 OR EXISTS(SELECT 1 FROM active a WHERE a.z=6 AND a.id=p.id)")?;
        q.query_map([], |r| {
            Ok([
                r.get::<_, i32>(0)?,
                r.get::<_, i32>(1)?,
                r.get::<_, i32>(2)?,
            ])
        })?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?
    };
    let keys = base
        .catalog
        .keys()
        .filter(|k| k.0 == 6)
        .copied()
        .collect::<Vec<_>>();
    for key in keys {
        ensure!(
            base.points(key)?.iter().all(|p| stored.contains(p)),
            "基準SQLiteのz6と作業正本のXYZが一致しません"
        );
    }
    s.db.execute("INSERT INTO stages VALUES('seed-source-6','done','{}')", [])?;
    Ok(())
}
fn append_fine(root: &Path, baseline: &Path, version: &str, verification: &Value) -> Result<()> {
    let target = root.join("gsi-terrain.sqlite3");
    if target.exists() {
        return Ok(());
    }
    let tmp = root.join("gsi-terrain-building.sqlite3");
    ensure!(!tmp.exists(), "未確定の格納ファイルがあります");
    let mut c = Connection::open(&tmp)?;
    yamamuki_data_sqlite::initialize(&c)?;
    yamamuki_terrain::import::import_shared(&mut c, &root.join("terrain-shared.sqlite3"), version)?;
    c.execute("ATTACH DATABASE ?1 AS fine", [readonly_uri(baseline)])?;
    let fine_id: i64 = c.query_row(
        "SELECT dataset_key FROM fine.datasets WHERE kind='terrain'",
        [],
        |r| r.get(0),
    )?;
    let id: i64 = c.query_row(
        "SELECT dataset_key FROM datasets WHERE kind='terrain'",
        [],
        |r| r.get(0),
    )?;
    let raw: String = c.query_row(
        "SELECT metadata_json FROM fine.datasets WHERE dataset_key=?1",
        [fine_id],
        |r| r.get(0),
    )?;
    let mut metadata: Value = serde_json::from_str(&raw)?;
    metadata["generation"]["coarseExtension"] = verification.clone();
    let tx = c.transaction()?;
    tx.execute("INSERT INTO terrain_levels SELECT ?1,level_key,z,settings_json FROM fine.terrain_levels WHERE dataset_key=?2",params![id,fine_id])?;
    tx.execute("INSERT INTO terrain_tiles SELECT ?1,level_key,x,y,encoding,payload FROM fine.terrain_tiles WHERE dataset_key=?2",params![id,fine_id])?;
    tx.execute(
        "UPDATE datasets SET metadata_json=?2 WHERE dataset_key=?1",
        params![id, metadata.to_string()],
    )?;
    tx.commit()?;
    let mismatch:i64=c.query_row("SELECT COUNT(*) FROM fine.terrain_tiles f LEFT JOIN terrain_tiles t ON t.dataset_key=?1 AND t.level_key=f.level_key AND t.x=f.x AND t.y=f.y WHERE f.dataset_key=?2 AND (t.payload IS NULL OR t.payload<>f.payload OR t.encoding<>f.encoding)",params![id,fine_id],|r|r.get(0))?;
    ensure!(mismatch == 0, "既存z6〜z13の圧縮本文が変化しました");
    c.execute_batch("DETACH DATABASE fine")?;
    drop(c);
    let mut check = yamamuki_terrain::sqlite::Database::open(&tmp)?;
    check.validate_all()?;
    drop(check);
    fs::rename(tmp, target)?;
    Ok(())
}
pub fn execute(
    source: &Path,
    baseline: &Path,
    root: &Path,
    version: &str,
    pilots: &[PathBuf],
    limit: usize,
) -> Result<()> {
    let source = source.canonicalize()?;
    let baseline = baseline.canonicalize()?;
    let (original, window, base_sha) = source_identity(&source, &baseline)?;
    let engine = natural_inventory::hash_file(&std::env::current_exe()?)?;
    if window.is_empty() {
        check_pilots(pilots, &engine)?;
    }
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(configured_threads(workers))
        .build_global();
    fs::create_dir_all(root)?;
    let lock = Connection::open(root.join("engine-lock.sqlite3"))?;
    lock.busy_timeout(std::time::Duration::ZERO)?;
    lock.execute_batch("CREATE TABLE IF NOT EXISTS lock(id INTEGER);BEGIN EXCLUSIVE")?;
    let levels=COARSE_LEVELS.iter().rev().map(|(z,e)|json!({"z":z,"error_down_m":e,"error_up_m":e,"anchor":*z==4,"parent_z":if *z==4{Value::Null}else{json!(4)}})).collect::<Vec<_>>();
    let identity = json!({"source_generation":original["generation_id"],"baseline_sha256":base_sha,"source_inputs_sha256":natural_inventory::hash_file(&source.join("inputs.sqlite3"))?,"engine_sha256":engine,"levels":levels,"window":window});
    let identity_path = root.join("coarse-identity.json");
    if identity_path.exists() {
        ensure!(
            serde_json::from_slice::<Value>(&fs::read(&identity_path)?)? == identity,
            "再開時の元データ・生成器・条件が一致しません"
        );
    } else {
        fs::write(&identity_path, serde_json::to_vec_pretty(&identity)?)?;
    }
    let job = json!({"generation_id":natural_inventory::hash(identity.to_string().as_bytes()),"settings":{"levels":levels,"profile":"coarse-extension-z4-z5-v1"}});
    fs::write(root.join("job.json"), serde_json::to_vec_pretty(&job)?)?;
    let inputs_db = Connection::open_with_flags(
        source.join("inputs.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let inputs = {
        let mut q = inputs_db.prepare(
            "SELECT x,y,sha256 FROM inputs WHERE state='cached' AND verified=1 ORDER BY y,x",
        )?;
        q.query_map([], |r| {
            Ok(Input {
                x: r.get(0)?,
                y: r.get(1)?,
                sha: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    }
    .into_iter()
    .filter(|i| {
        window.is_empty()
            || (i.x >= window[0] as i32
                && i.x < window[2] as i32
                && i.y >= window[1] as i32
                && i.y < window[3] as i32)
    })
    .collect::<Vec<_>>();
    ensure!(!inputs.is_empty(), "監査対象の元DEMがありません");
    let s = Store::open(&root.join("work.sqlite3"), limit)?;
    seed(&source, &s, &baseline)?;
    let mut dem = Dem {
        root: original["identity"]["cache"]
            .as_str()
            .context("DEMパスなし")?
            .into(),
        cache: HashMap::new(),
        order: VecDeque::new(),
    };
    if !root.join("terrain-shared.sqlite3").exists() {
        let mut fine = 6;
        for &(z, error) in &COARSE_LEVELS {
            timed(root, &format!("prune-{z}"), || {
                prune(&s, z, fine, error, &inputs, &mut dem, root)
            })?;
            timed(root, &format!("certify-{z}"), || {
                certify(&s, z, &inputs, &mut dem, root)
            })?;
            let missing:i64=s.db.query_row("SELECT COUNT(*) FROM active a WHERE a.z=?1 AND NOT EXISTS(SELECT 1 FROM active b WHERE b.z=?2 AND b.id=a.id)",params![z,fine],|r|r.get(0))?;
            ensure!(missing == 0, "粗レベルの点が細レベルに含まれません");
            fine = z;
        }
        pack_levels(&s, &job, &inputs, root, &window, &COARSE_LEVELS)?;
        let seams = verify_seams(root)?;
        let audit = audit_saved_levels(root, &inputs, &mut dem, &COARSE_LEVELS)?;
        let integers = verify_distribution(root)?;
        let report = json!({"complete":true,"engine_sha256":engine,"source_generation":original["generation_id"],"baseline_sha256":base_sha,"levels":COARSE_LEVELS,"window":window,"saved_dem":audit,"same_quality_seams":seams,"integer_roundtrip":integers});
        finalize(root, &window, &report)?;
        fs::write(
            root.join("coarse-verification.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
    }
    let report: Value = serde_json::from_slice(&fs::read(root.join("coarse-verification.json"))?)?;
    append_fine(root, &baseline, version, &report)?;
    progress(
        root,
        json!({"stage":"complete","complete":true,"levels":"z4-z13","coarse_extension":report}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extended_windows_path_becomes_sqlite_readonly_uri() {
        assert_eq!(
            readonly_uri(Path::new(r"\\?\D:\data #\work.sqlite3")),
            "file:D:/data%20%23/work.sqlite3?mode=ro"
        );
    }
}
