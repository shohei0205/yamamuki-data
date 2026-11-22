use yamamuki_data_sqlite as common;
use yamamuki_terrain::{
    fixture,
    sqlite::{Database, Tile},
};
fn database() -> (tempfile::NamedTempFile, Database) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut db = common::rusqlite::Connection::open(file.path()).unwrap();
    common::initialize(&db).unwrap();
    fixture::create(&mut db).unwrap();
    drop(db);
    let db = Database::open(file.path()).unwrap();
    (file, db)
}
#[test]
fn all_levels_roundtrip_and_merge() {
    let (_file, mut db) = database();
    assert_eq!(db.validate_all().unwrap(), 8);
    for (z, error) in yamamuki_terrain::sqlite::LEVELS {
        assert_eq!(db.level_errors[&z], error);
    }
    let merged = tempfile::NamedTempFile::new().unwrap();
    let mut dest = common::rusqlite::Connection::open(merged.path()).unwrap();
    common::initialize(&dest).unwrap();
    common::merge_from(&mut dest, &db.connection).unwrap();
    drop(dest);
    assert_eq!(
        Database::open(merged.path())
            .unwrap()
            .validate_all()
            .unwrap(),
        8
    );
}
#[test]
fn truncated_envelope_is_rejected() {
    let (_file, db) = database();
    let key = *db.catalog.keys().next().unwrap();
    let bytes = db.tile(key).unwrap().encode().unwrap();
    for length in 0..bytes.len() {
        assert!(Tile::decode(&bytes[..length]).is_err());
    }
    let mut bytes = bytes;
    bytes.push(0);
    assert!(Tile::decode(&bytes).is_err());
}
#[test]
fn tampered_hash_is_rejected() {
    let (file, mut db) = database();
    let key = *db.catalog.keys().next().unwrap();
    let mut tile = db.tile(key).unwrap();
    tile.hash[0] ^= 1;
    common::rusqlite::Connection::open(file.path())
        .unwrap()
        .execute(
            "UPDATE terrain_tiles SET payload=?1 WHERE level_key=?2",
            common::rusqlite::params![tile.encode().unwrap(), key.0],
        )
        .unwrap();
    assert!(db.points(key).is_err());
}
#[test]
fn reads_control_levels_without_an_encoding_setting() {
    let (file, db) = database();
    drop(db);
    common::rusqlite::Connection::open(file.path())
        .unwrap()
        .execute(
            "UPDATE terrain_levels SET settings_json=json_remove(settings_json,'$.encoding')",
            [],
        )
        .unwrap();
    assert_eq!(
        Database::open(file.path()).unwrap().validate_all().unwrap(),
        8
    );
}

#[test]
fn a_declared_zoom_with_no_tiles_is_rejected() {
    let (file, db) = database();
    drop(db);
    let conn = common::rusqlite::Connection::open(file.path()).unwrap();
    conn.execute("DELETE FROM terrain_tiles WHERE level_key=13", [])
        .unwrap();
    assert!(
        common::terrain_zooms(&conn, 1)
            .unwrap_err()
            .to_string()
            .contains("z13")
    );
    assert!(common::validate(&conn).is_err());
    assert!(Database::open(file.path()).is_err());
    assert!(Database::open_metadata(file.path()).is_err());
}

#[test]
fn geometry_validation_rejects_degenerate_points_with_a_valid_hash() {
    use yamamuki_terrain::{natural_codec, sqlite::point_hash};

    for (points, expected) in [
        (vec![[0, 0, 0]], "3点以上"),
        (vec![[0, 0, 0], [2, 0, 0], [0, 2, 0], [0, 0, 9]], "XYが重複"),
        (vec![[0, 0, 0], [2, 2, 0], [4, 4, 9]], "一直線"),
    ] {
        let (file, db) = database();
        let key = *db.catalog.keys().find(|key| key.0 == 6).unwrap();
        drop(db);
        let tile = Tile {
            origin: [0, 0],
            parents: Vec::new(),
            count: points.len(),
            hash: point_hash(&points),
            controls: natural_codec::independent(&points).unwrap(),
        };
        let conn = common::rusqlite::Connection::open(file.path()).unwrap();
        conn.execute(
            "UPDATE terrain_tiles SET payload=?1 WHERE level_key=6",
            [tile.encode().unwrap()],
        )
        .unwrap();
        drop(conn);

        let mut reader = Database::open(file.path()).unwrap();
        assert_eq!(reader.points(key).unwrap().as_ref(), &points);
        let error = reader.validate_all().unwrap_err();
        assert!(format!("{error:#}").contains(expected), "{error:#}");
    }
}

#[test]
fn unknown_encoding_is_rejected() {
    let (file, _db) = database();
    common::rusqlite::Connection::open(file.path())
        .unwrap()
        .execute("UPDATE terrain_tiles SET encoding='unknown'", [])
        .unwrap();
    assert!(Database::open(file.path()).is_err());
}
#[test]
fn missing_parent_is_rejected() {
    let (file, mut db) = database();
    let mut keys = db.catalog.keys().copied().collect::<Vec<_>>();
    keys.sort();
    let key = keys[3];
    let mut tile = db.tile(key).unwrap();
    tile.parents = vec![(1, 0, 0)];
    tile.controls[..4].copy_from_slice(b"NDC1");
    tile.controls[8..12].copy_from_slice(&1u32.to_le_bytes());
    common::rusqlite::Connection::open(file.path())
        .unwrap()
        .execute(
            "UPDATE terrain_tiles SET payload=?1 WHERE level_key=?2",
            common::rusqlite::params![tile.encode().unwrap(), key.0],
        )
        .unwrap();
    assert!(db.points(key).is_err());
}

#[test]
fn cached_parent_does_not_hide_dependency_depth() {
    let (file, mut db) = database();
    let mut keys = db.catalog.keys().copied().collect::<Vec<_>>();
    keys.sort();
    let points = db.points(keys[0]).unwrap();
    let conn = common::rusqlite::Connection::open(file.path()).unwrap();
    for i in 1..=3 {
        let mut tile = db.tile(keys[i]).unwrap();
        let local = points
            .iter()
            .map(|p| [p[0] - tile.origin[0], p[1] - tile.origin[1], p[2]])
            .collect::<Vec<_>>();
        let refs = (0..points.len()).map(|n| Some((0, n))).collect::<Vec<_>>();
        tile.parents = vec![keys[i - 1]];
        tile.hash = yamamuki_terrain::sqlite::point_hash(&points);
        tile.controls =
            yamamuki_terrain::natural_codec::delta(&local, &refs, &[points.as_ref().clone()])
                .unwrap();
        conn.execute(
            "UPDATE terrain_tiles SET payload=?1 WHERE level_key=?2",
            common::rusqlite::params![tile.encode().unwrap(), keys[i].0],
        )
        .unwrap();
        if i <= 2 {
            db.points(keys[i]).unwrap();
        }
    }
    assert!(db.points(keys[3]).is_err());
}

#[test]
fn imports_completed_work_without_reencoding_and_normalizes_identity() {
    let (_fixture, db) = database();
    let legacy = tempfile::NamedTempFile::new().unwrap();
    let source = common::rusqlite::Connection::open(legacy.path()).unwrap();
    source.execute_batch("CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,origin_x INTEGER,origin_y INTEGER,points INTEGER,xyz_sha256 TEXT,payload BLOB);CREATE TABLE parents(z INTEGER,x INTEGER,y INTEGER,slot INTEGER,parent_z INTEGER,parent_x INTEGER,parent_y INTEGER);INSERT INTO metadata VALUES('complete','true');INSERT INTO metadata VALUES('national_complete','true');").unwrap();
    source
        .execute(
            "INSERT INTO metadata VALUES('generation_id',?1)",
            [serde_json::json!("NS1-fixture").to_string()],
        )
        .unwrap();
    let levels = yamamuki_terrain::sqlite::LEVELS
        .map(|(z, error)| serde_json::json!({"z":z,"error_down_m":error,"error_up_m":error}));
    source
        .execute(
            "INSERT INTO metadata VALUES('levels',?1)",
            [serde_json::to_string(&levels).unwrap()],
        )
        .unwrap();
    for (&key, _) in &db.catalog {
        let tile = db.tile(key).unwrap();
        let hash = tile
            .hash
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>();
        source
            .execute(
                "INSERT INTO tiles VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                common::rusqlite::params![
                    key.0,
                    key.1,
                    key.2,
                    tile.origin[0],
                    tile.origin[1],
                    tile.count,
                    hash,
                    tile.controls
                ],
            )
            .unwrap();
    }
    let output = tempfile::NamedTempFile::new().unwrap();
    let mut conn = common::rusqlite::Connection::open(output.path()).unwrap();
    common::initialize(&conn).unwrap();
    assert_eq!(
        yamamuki_terrain::import::import_shared(&mut conn, legacy.path(), "20261113T000000Z")
            .unwrap(),
        8
    );
    let info = common::datasets(&conn).unwrap();
    assert_eq!(info[0].metadata["generation"]["id"], "NS1-fixture");
    assert_eq!(info[0].metadata["generation"]["scope"], "national");
    assert_eq!(info[0].metadata["generation"]["errorUpRatio"], 1.);
    for (z, error) in yamamuki_terrain::sqlite::LEVELS {
        let raw: String = conn
            .query_row(
                "SELECT settings_json FROM terrain_levels WHERE z=?1",
                [z],
                |r| r.get(0),
            )
            .unwrap();
        let setting: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(setting["errorDownMetres"], error);
        assert_eq!(setting["errorUpMetres"], error);
    }
    // 以前の非対称生成物の取り込みでも、実際の品質表をそのまま維持する。
    let old_levels = yamamuki_terrain::sqlite::LEVELS.map(
        |(z, error)| serde_json::json!({"z":z,"error_down_m":error/0.625,"error_up_m":error/2.5}),
    );
    source
        .execute(
            "UPDATE metadata SET value=?1 WHERE key='levels'",
            [serde_json::to_string(&old_levels).unwrap()],
        )
        .unwrap();
    let old_output = tempfile::NamedTempFile::new().unwrap();
    let mut old_conn = common::rusqlite::Connection::open(old_output.path()).unwrap();
    common::initialize(&old_conn).unwrap();
    yamamuki_terrain::import::import_shared(&mut old_conn, legacy.path(), "20261113T000000Z")
        .unwrap();
    assert_eq!(
        common::datasets(&old_conn).unwrap()[0].metadata["generation"]["errorUpRatio"],
        0.25
    );
    let raw: String = old_conn
        .query_row(
            "SELECT settings_json FROM terrain_levels WHERE z=13",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let setting: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(setting["errorDownMetres"], 25.);
    assert_eq!(setting["errorUpMetres"], 6.25);

    source
        .execute(
            "UPDATE metadata SET value='false' WHERE key='national_complete'",
            [],
        )
        .unwrap();
    source
        .execute(
            "INSERT INTO metadata VALUES('source_window_z14','[14421,6379,14491,6449]')",
            [],
        )
        .unwrap();
    let regional_output = tempfile::NamedTempFile::new().unwrap();
    let mut regional_conn = common::rusqlite::Connection::open(regional_output.path()).unwrap();
    common::initialize(&regional_conn).unwrap();
    yamamuki_terrain::import::import_shared(&mut regional_conn, legacy.path(), "20261116T000000Z")
        .unwrap();
    let sets = common::datasets(&regional_conn).unwrap();
    assert_eq!(sets[0].title, "地域の地形（国土地理院標高）");
    assert_eq!(
        sets[0].metadata["generation"]["sourceWindowZ14"],
        serde_json::json!([14421, 6379, 14491, 6449])
    );
    let restored = Database::open(output.path()).unwrap();
    for &key in db.catalog.keys() {
        assert_eq!(
            db.tile(key).unwrap().controls,
            restored.tile(key).unwrap().controls
        );
    }
}

#[test]
fn selects_zoom_prefix_without_reencoding() {
    let (file, source) = database();
    let output = tempfile::NamedTempFile::new().unwrap();
    let mut dest = common::rusqlite::Connection::open(output.path()).unwrap();
    common::initialize(&dest).unwrap();
    assert_eq!(
        yamamuki_terrain::select::copy_levels(&mut dest, file.path(), &[6, 7, 8, 9]).unwrap(),
        4
    );
    let mut selected = Database::open(output.path()).unwrap();
    assert_eq!(selected.validate_all().unwrap(), 4);
    for &key in selected.catalog.keys() {
        assert_eq!(
            source.tile(key).unwrap().encode().unwrap(),
            selected.tile(key).unwrap().encode().unwrap()
        );
    }
    assert!(!selected.catalog.keys().any(|key| key.0 > 9));
}
#[test]
fn selection_rejects_missing_parent_and_duplicate_zoom() {
    let (file, source) = database();
    let mut keys = source.catalog.keys().copied().collect::<Vec<_>>();
    keys.sort();
    let key = keys[3];
    let mut tile = source.tile(key).unwrap();
    tile.controls[..4].copy_from_slice(b"NDC1");
    tile.controls[8..12].copy_from_slice(&1u32.to_le_bytes());
    tile.parents = vec![keys[2]];
    common::rusqlite::Connection::open(file.path())
        .unwrap()
        .execute(
            "UPDATE terrain_tiles SET payload=?1 WHERE level_key=?2",
            common::rusqlite::params![tile.encode().unwrap(), key.0],
        )
        .unwrap();
    for levels in [vec![9], vec![6, 6]] {
        let out = tempfile::NamedTempFile::new().unwrap();
        let mut conn = common::rusqlite::Connection::open(out.path()).unwrap();
        common::initialize(&conn).unwrap();
        assert!(yamamuki_terrain::select::copy_levels(&mut conn, file.path(), &levels).is_err());
    }
}
