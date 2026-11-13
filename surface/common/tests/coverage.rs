use serde_json::{Value, json};
use yamamuki_data_sqlite::{self as db, rusqlite::Connection};

#[test]
fn shares_geometry_and_language_cases_with_python() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../schemas/testdata/coverage-metadata.json"
    ))
    .unwrap();
    for case in cases {
        let result =
            db::validate_coverage_metadata(&case["metadata"], case["surface"].as_bool().unwrap());
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}: {result:?}",
            case["name"]
        );
    }
}

#[test]
fn sqlite_validates_new_metadata_and_keeps_old_datasets() {
    let conn = Connection::open_in_memory().unwrap();
    db::initialize(&conn).unwrap();
    conn.execute(
        "INSERT INTO datasets VALUES(1,'sample','surface','地表','1',1,'{}')",
        [],
    )
    .unwrap();
    db::validate(&conn).unwrap();
    for metadata in [
        json!({"language": "ja_JP"}),
        json!({"coverage": {"geometry": null}}),
    ] {
        conn.execute(
            "UPDATE datasets SET metadata_json=?1",
            [metadata.to_string()],
        )
        .unwrap();
        assert!(db::validate(&conn).is_err());
    }
}
