use std::process::Command;
use yamamuki_data_sqlite::{self as db, rusqlite::Connection};

#[test]
fn reidentify_preserves_input_and_compressed_tiles() {
    let folder = tempfile::tempdir().unwrap();
    let input = folder.path().join("input.sqlite3");
    let output = folder.path().join("output.sqlite3");
    let source = Connection::open(&input).unwrap();
    db::initialize(&source).unwrap();
    source
        .execute_batch(
            r#"
        INSERT INTO datasets VALUES(1,'sample','surface','地表','0.1.0',1,'{}');
        INSERT INTO surface_layers VALUES(1,1,'forest','mask',0,'[]','{}');
        INSERT INTO surface_levels VALUES(1,1,5,'z5',512);
        INSERT INTO surface_tiles VALUES(1,1,5,1,2,3,X'0102030405');
    "#,
        )
        .unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_surface-tool"))
        .args([
            "set-data-version",
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "20261110T032217Z",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let result = db::open(&output).unwrap();
    let before = db::datasets(&source).unwrap().remove(0);
    let after = db::datasets(&result).unwrap().remove(0);
    assert_eq!(before.data_version, "0.1.0");
    assert_eq!(after.data_version, "20261110T032217Z");
    assert_eq!(
        (
            before.id,
            before.title,
            before.metadata,
            before.format_version
        ),
        (after.id, after.title, after.metadata, after.format_version)
    );
    let payload: Vec<u8> = result
        .query_row("SELECT payload FROM surface_tiles", [], |r| r.get(0))
        .unwrap();
    assert_eq!(payload, [1, 2, 3, 4, 5]);
    let status = Command::new(env!("CARGO_BIN_EXE_surface-tool"))
        .args([
            "set-data-version",
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "20261111T000000Z",
        ])
        .status()
        .unwrap();
    assert!(!status.success());
    assert_eq!(
        db::datasets(&db::open(&output).unwrap()).unwrap()[0].data_version,
        "20261110T032217Z"
    );
}
