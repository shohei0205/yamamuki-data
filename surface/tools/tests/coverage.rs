use serde_json::json;
use yamamuki_data_sqlite::{self as db, rusqlite::Connection};

// GDAL を使わない梱包処理を、共通の SQLite テストと一緒に確かめる。
#[path = "../../esa_worldcover/src/pack.rs"]
mod pack;

#[test]
fn pack_keeps_regions_and_language_and_recomputes_geometry() {
    let folder = tempfile::tempdir().unwrap();
    let input = folder.path().join("intermediate.sqlite3");
    let source = Connection::open(&input).unwrap();
    source
        .execute_batch(
            "CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT);
        CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,payload BLOB);
        CREATE TABLE water_tiles(z INTEGER,x INTEGER,y INTEGER,payload BLOB);
        CREATE TABLE hillshade_tiles(z INTEGER,x INTEGER,y INTEGER,payload BLOB);",
        )
        .unwrap();
    let mut manifest = json!({
        "version":1,"crs":"EPSG:3857","landEncoding":"forest-bare-unified-gray4",
        "waterEncoding":"gray4-coverage","bareOmitted":true,"maskWebp":true,
        "sourceNoDataClassification":{"landEvidence":"positive-elevation"},
        "attribution":"ESA", "palette":[0,1,2,3,4],"tileKeyZoomOffset":0,"tilePixels":2,
        "levels":[{"z":5}],"waterLevels":[{"z":5}]
    });
    let shade = json!({"encoding":"webp-or-png-gray8","version":1,"scheme":"xyz",
        "tileKeyZoomOffset":0,"tilePixels":2,"levels":[{"z":5}]});
    source
        .execute(
            "INSERT INTO metadata VALUES('hillshade',?1)",
            [shade.to_string()],
        )
        .unwrap();
    source
        .execute(
            "INSERT INTO metadata VALUES('manifest',?1)",
            [manifest.to_string()],
        )
        .unwrap();
    let mut target = Connection::open_in_memory().unwrap();
    db::initialize(&target).unwrap();
    pack::import(&mut target, &input, "test").unwrap();
    let metadata = &db::datasets(&target).unwrap()[0].metadata;
    assert_eq!(metadata["coverage"], json!({"regions":["JP"]}));
    assert!(metadata.get("language").is_none());

    let geometry =
        json!({"type":"Polygon","coordinates":[[[130,30],[140,30],[140,40],[130,40],[130,30]]]});
    manifest["coverage"] =
        json!({"regions":["JP","KR"],"geometry":geometry,"finest":{"geometry":geometry}});
    manifest["language"] = json!("ja");
    source
        .execute(
            "UPDATE metadata SET value=?1 WHERE key='manifest'",
            [manifest.to_string()],
        )
        .unwrap();
    source
        .execute(
            "INSERT INTO tiles VALUES(5,0,0,?1)",
            [b"\x89PNG\r\n\x1a\n".as_slice()],
        )
        .unwrap();
    let mut explicit = Connection::open_in_memory().unwrap();
    db::initialize(&explicit).unwrap();
    pack::import(&mut explicit, &input, "test").unwrap();
    db::validate(&explicit).unwrap();
    let metadata = &db::datasets(&explicit).unwrap()[0].metadata;
    assert_eq!(metadata["coverage"]["regions"], json!(["JP", "KR"]));
    assert_eq!(
        metadata["coverage"]["geometry"],
        metadata["coverage"]["finest"]["geometry"]
    );
    assert_ne!(metadata["coverage"]["geometry"], geometry);
    assert_eq!(metadata["coverage"]["geometry"]["type"], "Polygon");
    assert_eq!(metadata["language"], "ja");

    let mut selected = Connection::open_in_memory().unwrap();
    db::initialize(&selected).unwrap();
    db::copy_surface_layers(&mut selected, &explicit, &["forest".into()]).unwrap();
    let metadata = &db::datasets(&selected).unwrap()[0].metadata;
    assert_eq!(metadata["coverage"]["regions"], json!(["JP", "KR"]));
    assert_eq!(
        metadata["coverage"]["geometry"],
        metadata["coverage"]["finest"]["geometry"]
    );
    assert_ne!(metadata["coverage"]["geometry"], geometry);
    assert_eq!(metadata["coverage"]["geometry"]["type"], "Polygon");
    assert_eq!(metadata["language"], "ja");

    manifest["coverage"]["geometry"] = json!(null);
    source
        .execute(
            "UPDATE metadata SET value=?1 WHERE key='manifest'",
            [manifest.to_string()],
        )
        .unwrap();
    let mut rejected = Connection::open_in_memory().unwrap();
    db::initialize(&rejected).unwrap();
    assert!(pack::import(&mut rejected, &input, "test").is_err());
    assert!(db::datasets(&rejected).unwrap().is_empty());
}
