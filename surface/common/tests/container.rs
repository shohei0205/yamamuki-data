use std::io::Cursor;
use yamamuki_data_sqlite::{
    self as db,
    rusqlite::{Connection, params},
};

fn container() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db::initialize(&db).unwrap();
    db
}

#[test]
fn combines_three_dataset_types_without_changing_payloads() {
    let surface = container();
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    surface
        .execute_batch(
            "INSERT INTO datasets VALUES(1,'color','surface','地図','2026',1,'{}');
         INSERT INTO surface_layers VALUES(1,1,'rgb','rgb',0,'[]','{}');
         INSERT INTO surface_layers VALUES(1,2,'shade','hillshade',1,'[]','{}');
         INSERT INTO surface_levels VALUES(1,1,8,'L2',2);
         INSERT INTO surface_levels VALUES(1,2,6,'L2',4);",
        )
        .unwrap();
    surface
        .execute(
            "INSERT INTO surface_tiles VALUES(1,1,8,1,1,1,?1)",
            [png.get_ref()],
        )
        .unwrap();

    let terrain = container();
    terrain
        .execute_batch(
            "INSERT INTO datasets VALUES(1,'heights','terrain','地形','2026',1,'{}');
         INSERT INTO terrain_levels VALUES(1,1,6,'{}');
         INSERT INTO terrain_tiles VALUES(1,1,1,1,'test',x'010203');",
        )
        .unwrap();

    let points = container();
    let metadata = r#"{"pointCount":2,"sources":[{"name":"地点の出典","attribution":"出典表示"}],"authors":[{"name":"作成者"}],"tags":[{"name":"山"}]}"#;
    let point = r#"{"id":"z","nameReading":["やま"],"tags":["山"]}"#;
    points
        .execute(
            "INSERT INTO datasets VALUES(1,'peaks','points','山頂','2026',6,?1)",
            [metadata],
        )
        .unwrap();
    points
        .execute("INSERT INTO point_records VALUES(1,0,'z',?1)", [point])
        .unwrap();
    points
        .execute(
            "INSERT INTO point_records VALUES(1,1,'a',?1)",
            [r#"{"id":"a"}"#],
        )
        .unwrap();

    let mut merged = container();
    for source in [&surface, &terrain, &points] {
        db::merge_from(&mut merged, source).unwrap();
    }
    db::validate(&merged).unwrap();
    let datasets = db::datasets(&merged).unwrap();
    assert_eq!(datasets.len(), 3);
    assert_eq!(
        datasets[2].metadata,
        serde_json::from_str::<serde_json::Value>(metadata).unwrap()
    );
    assert_ne!(datasets[1].key, datasets[2].key);
    assert_eq!(
        db::surface_layers(&merged, datasets[0].key).unwrap().len(),
        2
    );
    assert_eq!(
        db::surface_levels(&merged, datasets[0].key, 1).unwrap()[0].z,
        8
    );
    assert_eq!(
        db::surface_levels(&merged, datasets[0].key, 2).unwrap()[0].z,
        6
    );
    assert_eq!(db::validate_surface_images(&merged).unwrap(), 1);
    assert!(
        db::surface_tile(&merged, datasets[0].key, 1, 8, 1, 1)
            .unwrap()
            .is_some()
    );
    let raster: Vec<u8> = merged
        .query_row("SELECT payload FROM surface_tiles", [], |row| row.get(0))
        .unwrap();
    assert_eq!(&raster, png.get_ref());
    let mesh: Vec<u8> = merged
        .query_row(
            "SELECT payload FROM terrain_tiles WHERE dataset_key=?1",
            [datasets[1].key],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mesh, [1, 2, 3]);
    let mut read = merged
        .prepare(
            "SELECT point_id,payload_json FROM point_records WHERE dataset_key=?1 ORDER BY ordinal",
        )
        .unwrap();
    let rows: Vec<(String, String)> = read
        .query_map(params![datasets[2].key], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(read);
    assert_eq!(rows[0], ("z".into(), point.into()));
    assert_eq!(rows[1].0, "a");
    assert!(db::merge_from(&mut merged, &points).is_err());
    assert_eq!(db::datasets(&merged).unwrap().len(), 3);
}

#[test]
fn recomposes_surface_with_different_zoom_sets_per_layer() {
    let source = container();
    source
        .execute_batch(
            "INSERT INTO datasets VALUES(1,'test','surface','地表','1',1,'{}');
         INSERT INTO surface_layers VALUES
             (1,1,'forest','mask',0,'[]','{}'),
             (1,2,'water','mask',1,'[]','{}'),
             (1,3,'hillshade','hillshade',2,'[]','{}');
         INSERT INTO surface_levels VALUES
             (1,1,5,'z5',2),(1,1,7,'z7',2),(1,1,8,'z8',2),
             (1,2,5,'z5',2),(1,2,9,'z9',2),(1,3,5,'z5',2);
         INSERT INTO surface_tiles VALUES
             (1,1,5,0,0,1,x'01'),(1,1,7,0,0,1,x'02'),(1,1,8,0,0,1,x'03'),
             (1,2,5,0,0,1,x'04'),(1,2,9,0,0,1,x'05'),(1,3,5,0,0,1,x'06');",
        )
        .unwrap();

    let mut target = container();
    let selection = [
        ("forest".to_owned(), Some(vec![8, 5])),
        ("water".to_owned(), Some(vec![9])),
    ]
    .into();
    db::copy_surface_selection(&mut target, &source, &selection).unwrap();
    let mut query = target.prepare(
        "SELECT l.id,t.z,t.payload FROM surface_tiles t JOIN surface_layers l USING(dataset_key,layer_key)
         ORDER BY t.layer_key,t.z",
    ).unwrap();
    let rows: Vec<(String, u32, Vec<u8>)> = query
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        [
            ("forest".into(), 5, vec![1]),
            ("forest".into(), 8, vec![3]),
            ("water".into(), 9, vec![5])
        ]
    );
    assert_eq!(db::surface_layers(&target, 1).unwrap().len(), 2);
    assert_eq!(db::surface_levels(&target, 1, 1).unwrap().len(), 2);
    assert_eq!(db::surface_levels(&target, 1, 2).unwrap().len(), 1);
    db::validate(&target).unwrap();

    for invalid in [vec![], vec![5, 5], vec![9]] {
        let mut rejected = container();
        let selection = [("forest".to_owned(), Some(invalid))].into();
        assert!(db::copy_surface_selection(&mut rejected, &source, &selection).is_err());
        assert!(db::datasets(&rejected).unwrap().is_empty());
    }
}
