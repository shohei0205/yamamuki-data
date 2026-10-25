#[allow(dead_code)]
#[path = "../src/data.rs"]
mod data;
#[allow(dead_code)]
#[path = "../src/selection.rs"]
mod selection;
use selection::{Camera, lonlat};
fn main() -> anyhow::Result<()> {
    let path = std::env::args_os().nth(1).expect("SQLite path");
    let db = data::Database::open(std::path::Path::new(&path))?;
    let mut drops = 0;
    let mut subdivision_drops = 0;
    let mut cases = 0;
    for (name, lon, lat, h) in [
        ("fuji", 138.727, 35.36, 1800.),
        ("yari", 137.647, 36.342, 2200.),
        ("tsukuba", 140.106, 36.225, 400.),
    ] {
        for pitch in [0.3, 0.85, 1.56] {
            for yaw in [0., 1.57, 3.14, 4.71] {
                let mut c = Camera::default();
                c.center = lonlat(lon, lat);
                c.height = h;
                c.pitch = pitch;
                c.yaw = yaw;
                c.viewport = [1400., 800.];
                let mut old_z = 0;
                let mut old_div = 1;
                let mut old_d = 0.;
                let mut distance = 40000.;
                while distance >= 400. {
                    c.distance = distance;
                    let keys = selection::select(&db.catalog, &c);
                    let faces: usize = keys.iter().map(|k| db.catalog[k]).sum();
                    assert!(faces <= 350000 && keys.len() <= 192);
                    let z = keys
                        .iter()
                        .filter(|k| {
                            let b = data::bounds(**k);
                            let x = c.center[0] / data::UNIT;
                            let y = c.center[1] / data::UNIT;
                            x >= b[0] && x < b[2] && y >= b[1] && y < b[3]
                        })
                        .map(|k| k.0)
                        .max()
                        .unwrap_or(0);
                    let mut div = 1;
                    while div < 4 && faces * div * div * 4 <= 2000000 {
                        div *= 2;
                    }
                    if z > 0 && old_z > z {
                        println!(
                            "LEVEL {name} pitch={pitch:.2} yaw={yaw:.2} d={old_d:.1}->{distance:.1} z={old_z}->{z} faces={faces}"
                        );
                        drops += 1;
                    }
                    if div < old_div {
                        subdivision_drops += 1;
                    }
                    old_z = z;
                    old_div = div;
                    old_d = distance;
                    distance *= 0.85;
                    cases += 1;
                }
            }
        }
    }
    println!(
        "samples={cases} center_level_drops={drops} estimated_curve_division_drops={subdivision_drops}"
    );
    anyhow::ensure!(drops == 0, "ズームインで中心のLODが下がっています");
    Ok(())
}
