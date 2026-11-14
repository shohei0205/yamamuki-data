//! Windowsの資源計測を含め、PNGから完成SQLiteまでCLIで確認する。
#[cfg(windows)]
#[test]
#[ignore = "出力ドライブに100GiB以上の空き容量が必要"]
fn png_to_verified_distribution() {
    use std::{
        fs,
        path::PathBuf,
        process::Command,
        time::{Duration, Instant},
    };
    let parent = std::env::var_os("TERRAIN_SMOKE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = tempfile::tempdir_in(parent).unwrap();
    let dem = dir.path().join("dem/14/14505");
    fs::create_dir_all(&dem).unwrap();
    let file = fs::File::create(dem.join("6468.png")).unwrap();
    let mut encoder = png::Encoder::new(file, 256, 256);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&[0u8, 39, 16].repeat(256 * 256))
        .unwrap();
    let engine = env!("CARGO_BIN_EXE_terrain-gsi-dem");
    let prepared = dir.path().join("run");
    let status = Command::new(engine)
        .args(["prepare", "--dem14"])
        .arg(dir.path().join("dem/14"))
        .arg("--output")
        .arg(&prepared)
        .status()
        .unwrap();
    assert!(status.success());
    let mut child = Command::new(engine)
        .args(["run", "--prepared"])
        .arg(&prepared)
        .args(["--window", "14505", "6468", "14506", "6469"])
        .spawn()
        .unwrap();
    let begin = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "{:?}",
                fs::read_to_string(prepared.join("progress.json"))
            );
            break;
        }
        if begin.elapsed() > Duration::from_secs(180) {
            child.kill().unwrap();
            let _ = child.wait();
            panic!(
                "180秒で完了しません: {:?}",
                fs::read_to_string(prepared.join("progress.json"))
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let conn = rusqlite::Connection::open(prepared.join("terrain-shared.sqlite3")).unwrap();
    let complete: String = conn
        .query_row("SELECT value FROM metadata WHERE key='complete'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(complete, "true");
    let report: String = conn
        .query_row(
            "SELECT value FROM metadata WHERE key='verification'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(report["saved_dem"]["all_valid_pixels"], true);
    assert_eq!(report["integer_roundtrip"]["integer_roundtrip"], true);
}
