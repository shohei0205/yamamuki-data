use geo::{Covers, LineString, MultiPolygon, Point, Polygon};
use serde_json::{Value, json};
use yamamuki_data_sqlite::{
    self as db,
    rusqlite::{Connection, params},
};

fn container() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    db::initialize(&conn).unwrap();
    conn.execute_batch(r#"INSERT INTO datasets VALUES(1,'sample','surface','地表','1',1,'{"coverage":{"regions":["JP"]},"language":"ja"}');
        INSERT INTO surface_layers VALUES(1,1,'forest','mask',0,'[]','{}');"#).unwrap();
    conn
}

#[test]
fn reading_metadata_keeps_coordinate_precision() {
    let conn = container();
    conn.execute("UPDATE datasets SET metadata_json=?1", [r#"{"coverage":{"geometry":{"type":"Polygon","coordinates":[[[130,27.059125784374054],[131,27.059125784374054],[131,28],[130,28],[130,27.059125784374054]]]}}}"#]).unwrap();
    let latitude = metadata(&conn)["coverage"]["geometry"]["coordinates"][0][0][1]
        .as_f64()
        .unwrap();
    assert_eq!(latitude.to_bits(), 27.059125784374054_f64.to_bits());
}

fn level(conn: &Connection, layer: i64, z: u32, pixels: u32, tiles: &[(u32, u32)]) {
    conn.execute(
        "INSERT INTO surface_levels VALUES(1,?1,?2,?3,?4)",
        params![layer, z, format!("z{z}"), pixels],
    )
    .unwrap();
    for &(x, y) in tiles {
        conn.execute(
            "INSERT INTO surface_tiles VALUES(1,?1,?2,?3,?4,1,x'01')",
            params![layer, z, x, y],
        )
        .unwrap();
    }
}

fn metadata(conn: &Connection) -> Value {
    db::datasets(conn).unwrap().remove(0).metadata
}

fn geometry(value: &Value) -> MultiPolygon<f64> {
    let polygons: Vec<&Value> = if value["type"] == "Polygon" {
        vec![&value["coordinates"]]
    } else {
        value["coordinates"].as_array().unwrap().iter().collect()
    };
    MultiPolygon(
        polygons
            .iter()
            .map(|p| {
                let mut rings: Vec<_> = p
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| {
                        LineString::from(
                            r.as_array()
                                .unwrap()
                                .iter()
                                .map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap()))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect();
                Polygon::new(rings.remove(0), rings)
            })
            .collect(),
    )
}

fn tile_center(z: u32, x: u32, y: u32) -> Point<f64> {
    let side = f64::from(1u32 << z);
    Point::new(
        (f64::from(x) + 0.5) / side * 360. - 180.,
        (std::f64::consts::PI * (1. - 2. * (f64::from(y) + 0.5) / side))
            .sinh()
            .atan()
            .to_degrees(),
    )
}

#[test]
fn split_files_recompute_whole_and_finest_from_their_own_levels() {
    let source = container();
    level(&source, 1, 2, 256, &[(1, 1), (3, 3)]);
    level(&source, 1, 4, 256, &[(4, 4), (6, 6)]);
    db::recompute_surface_coverage(&source, 1).unwrap();
    let original = metadata(&source);
    let whole = geometry(&original["coverage"]["geometry"]);
    let finest = geometry(&original["coverage"]["finest"]["geometry"]);
    assert!(whole.covers(&tile_center(2, 3, 3)));
    assert!(!finest.covers(&tile_center(2, 3, 3)));
    assert!(finest.covers(&tile_center(4, 4, 4)));
    for z in [2, 4] {
        let mut output = Connection::open_in_memory().unwrap();
        db::initialize(&output).unwrap();
        db::copy_surface_selection(
            &mut output,
            &source,
            &[("forest".into(), Some(vec![z]))].into(),
        )
        .unwrap();
        let selected = metadata(&output);
        assert_eq!(
            selected["coverage"]["geometry"],
            selected["coverage"]["finest"]["geometry"]
        );
        if z == 2 {
            assert_ne!(
                selected["coverage"]["finest"]["geometry"],
                original["coverage"]["finest"]["geometry"]
            );
        } else {
            assert_eq!(
                selected["coverage"]["finest"]["geometry"],
                original["coverage"]["finest"]["geometry"]
            );
        }
        if z == 2 {
            assert_eq!(
                selected["coverage"]["geometry"],
                original["coverage"]["geometry"]
            );
        } else {
            assert_ne!(
                selected["coverage"]["geometry"],
                original["coverage"]["geometry"]
            );
        }
        assert_eq!(selected["language"], "ja");
        assert_eq!(selected["coverage"]["regions"], json!(["JP"]));
        db::validate(&output).unwrap();
    }
    assert_eq!(metadata(&source), original);
}

#[test]
fn finest_uses_pixel_density_and_whole_keeps_all_layers() {
    let conn = container();
    conn.execute(
        "INSERT INTO surface_layers VALUES(1,2,'water','mask',1,'[]','{}')",
        [],
    )
    .unwrap();
    level(&conn, 1, 3, 256, &[(1, 1)]);
    level(&conn, 2, 2, 1024, &[(3, 3)]);
    db::recompute_surface_coverage(&conn, 1).unwrap();
    let info = metadata(&conn);
    assert!(geometry(&info["coverage"]["geometry"]).covers(&tile_center(3, 1, 1)));
    let fine = geometry(&info["coverage"]["finest"]["geometry"]);
    assert!(fine.covers(&tile_center(2, 3, 3)));
    assert!(!fine.covers(&tile_center(3, 1, 1)));
}

#[test]
fn joins_tiles_without_filling_holes_or_gaps() {
    let conn = container();
    let mut tiles = vec![(0, 0)];
    tiles.extend(
        (3..6).flat_map(|x| (3..6).filter_map(move |y| ((x, y) != (4, 4)).then_some((x, y)))),
    );
    level(&conn, 1, 3, 256, &tiles);
    db::recompute_surface_coverage(&conn, 1).unwrap();
    let value = metadata(&conn)["coverage"]["geometry"].clone();
    assert_eq!(value["type"], "MultiPolygon");
    let geo = geometry(&value);
    assert_eq!(geo.0.len(), 2);
    assert_eq!(geo.0.iter().map(|p| p.interiors().len()).sum::<usize>(), 1);
    assert!(!geo.covers(&tile_center(3, 4, 4)));
    assert!(!geo.covers(&tile_center(3, 1, 1)));
    assert!(geo.covers(&tile_center(3, 3, 3)));
    db::validate(&conn).unwrap();
}

#[test]
fn empty_levels_do_not_claim_coverage_and_world_tile_remains_valid() {
    let conn = container();
    level(&conn, 1, 5, 256, &[]);
    db::recompute_surface_coverage(&conn, 1).unwrap();
    assert_eq!(metadata(&conn)["coverage"], json!({"regions":["JP"]}));
    level(&conn, 1, 0, 256, &[(0, 0)]);
    db::recompute_surface_coverage(&conn, 1).unwrap();
    db::validate(&conn).unwrap();
    assert!(geometry(&metadata(&conn)["coverage"]["geometry"]).covers(&Point::new(139., 35.)));
}
