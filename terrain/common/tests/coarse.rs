use yamamuki_terrain::{fixture, sqlite::Database};
#[test]
fn coarse_levels_validate_and_select() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("full.sqlite3");
    let mut c = yamamuki_data_sqlite::rusqlite::Connection::open(&path)?;
    yamamuki_data_sqlite::initialize(&c)?;
    fixture::create_with_coarse(&mut c)?;
    drop(c);
    let mut db = Database::open(&path)?;
    assert_eq!(db.validate_all()?, 10);
    assert_eq!(db.level_errors[&4], 1250.);
    assert_eq!(db.level_errors[&5], 625.);
    let out = dir.path().join("z9.sqlite3");
    let mut c = yamamuki_data_sqlite::rusqlite::Connection::open(&out)?;
    yamamuki_data_sqlite::initialize(&c)?;
    assert_eq!(
        yamamuki_terrain::select::copy_levels(&mut c, &path, &[4, 5, 6, 7, 8, 9])?,
        6
    );
    drop(c);
    assert_eq!(Database::open(&out)?.validate_all()?, 6);
    Ok(())
}

#[test]
fn national_generation_and_release_exclude_coarse_extension() {
    let settings: serde_json::Value =
        serde_json::from_str(include_str!("../../gsi_dem/settings.json")).unwrap();
    let generated = settings["levels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|level| level["z"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(generated, (6..=13).collect::<Vec<_>>());
    let release: serde_json::Value =
        serde_json::from_str(include_str!("../../gsi_dem/release.json")).unwrap();
    for package in release["packages"].as_object().unwrap().values() {
        for entry in std::iter::once(package)
            .chain(package["splitPackages"].as_array().into_iter().flatten())
        {
            let levels = entry["levels"]["terrain"]
                .as_array()
                .unwrap()
                .iter()
                .map(|level| level.as_u64().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(levels.first(), Some(&6));
            assert!(levels.iter().all(|z| generated.contains(z)));
        }
    }
}
