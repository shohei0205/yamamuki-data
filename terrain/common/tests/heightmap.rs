use yamamuki_data_sqlite as container;
use yamamuki_terrain::{fixture, sqlite::Database};
#[test]
fn mixed_read_validate_split_and_mismatch() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("mixed.sqlite3");
    let mut db = container::rusqlite::Connection::open(&path)?;
    container::initialize(&db)?;
    fixture::create_mixed(&mut db)?;
    drop(db);
    let mut source = Database::open(&path)?;
    assert_eq!(source.validate_all()?, 8);
    let key = *source.catalog.keys().find(|k| k.0 == 13).unwrap();
    let grid = source.heightmap(key)?.unwrap();
    assert_eq!(grid.sample(0., 0.), Some(115.));
    assert!(source.points(key).is_err());
    let split = dir.path().join("split.sqlite3");
    let mut out = container::rusqlite::Connection::open(&split)?;
    container::initialize(&out)?;
    assert_eq!(
        yamamuki_terrain::select::copy_levels(&mut out, &path, &[13])?,
        1
    );
    drop(out);
    assert_eq!(Database::open(&split)?.validate_all()?, 1);
    let edit = container::rusqlite::Connection::open(&path)?;
    edit.execute("UPDATE terrain_levels SET settings_json=json_set(settings_json,'$.cellsPerTile',16) WHERE level_key=13",[])?;
    assert!(Database::open(&path)?.validate_all().is_err());
    edit.execute("UPDATE terrain_levels SET settings_json=json_set(settings_json,'$.cellsPerTile',8) WHERE level_key=13",[])?;
    edit.execute(
        "UPDATE terrain_tiles SET encoding='natural-controls-v1' WHERE level_key=13",
        [],
    )?;
    assert!(Database::open(&path).is_err());
    Ok(())
}

#[test]
fn webp_read_validate_split_and_quality_mismatch() -> anyhow::Result<()> {
    webp_split_test(false)
}
#[test]
fn minmax_read_validate_split_and_quality_mismatch() -> anyhow::Result<()> {
    webp_split_test(true)
}
fn webp_split_test(minmax: bool) -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("webp.sqlite3");
    let mut db = container::rusqlite::Connection::open(&path)?;
    container::initialize(&db)?;
    fixture::create_mixed(&mut db)?;
    let blob: Vec<u8> = db.query_row(
        "SELECT payload FROM terrain_tiles WHERE level_key=13",
        [],
        |r| r.get(0),
    )?;
    let original = yamamuki_terrain::heightmap::Heightmap::decode(&blob)?;
    let heights = (0..original.width())
        .flat_map(|y| (0..original.width()).map(move |x| (x, y)))
        .map(|(x, y)| original.height(x, y))
        .collect::<Vec<_>>();
    let (expected, blob) = if minmax {
        yamamuki_terrain::heightmap_webp::encode_minmax(original.cells, &heights, 90)?
    } else {
        yamamuki_terrain::heightmap_webp::encode(original.cells, 0.125, &heights, 90)?
    };
    db.execute("UPDATE terrain_levels SET settings_json=json_set(json_remove(settings_json,'$.heightStepMetres'),'$.encoding','heightmap-u8-webp-v1','$.minimumHeightStepMetres',0.125,'$.heightScale','per-tile','$.webpQuality',90) WHERE level_key=13",[])?;
    db.execute(
        "UPDATE terrain_tiles SET encoding='heightmap-u8-webp-v1',payload=?1 WHERE level_key=13",
        [&blob],
    )?;
    if minmax {
        db.execute("UPDATE terrain_levels SET settings_json=json_set(json_remove(settings_json,'$.minimumHeightStepMetres'),'$.encoding','heightmap-u8-webp-minmax-v1','$.heightScale','min-max') WHERE level_key=13",[])?;
        db.execute(
            "UPDATE terrain_tiles SET encoding='heightmap-u8-webp-minmax-v1' WHERE level_key=13",
            [],
        )?;
    }
    drop(db);
    let mut source = Database::open(&path)?;
    assert_eq!(source.validate_all()?, 8);
    let key = *source.catalog.keys().find(|k| k.0 == 13).unwrap();
    assert_eq!(source.heightmap(key)?.unwrap().values, expected.values);
    assert_eq!(
        source.heightmap(key)?.unwrap().offset_metres,
        expected.offset_metres
    );
    let split = dir.path().join("split.sqlite3");
    let mut output = container::rusqlite::Connection::open(&split)?;
    container::initialize(&output)?;
    assert_eq!(
        yamamuki_terrain::select::copy_levels(&mut output, &path, &[13])?,
        1
    );
    drop(output);
    let mut restored = Database::open(&split)?;
    assert_eq!(restored.validate_all()?, 1);
    assert_eq!(restored.heightmap(key)?.unwrap().values, expected.values);
    assert_eq!(
        restored.heightmap(key)?.unwrap().offset_metres,
        expected.offset_metres
    );
    assert_eq!(restored.heightmap(key)?.unwrap().step, expected.step);
    let edit = container::rusqlite::Connection::open(&path)?;
    edit.execute("UPDATE terrain_levels SET settings_json=json_set(settings_json,'$.webpQuality',80) WHERE level_key=13",[])?;
    assert!(Database::open(&path)?.validate_all().is_err());
    Ok(())
}
