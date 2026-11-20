//! 元DEMからの格子生成と、レベルごとの形式を指定した配布SQLiteの構築。
use anyhow::{Context, Result, ensure};
use rayon::prelude::*;
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
};
use yamamuki_terrain::{
    heightmap::{
        Heightmap, WEBP_ENCODING as WEBP, WEBP_MINMAX_ENCODING as WEBP_MINMAX, is_encoding,
    },
    natural_codec,
    sqlite::{Database, ENCODING as CONTROLS},
};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub dem14: Option<PathBuf>,
    pub control_database: Option<PathBuf>,
    pub data_version: String,
    pub levels: Vec<Level>,
    #[serde(default)]
    pub window: Option<[u32; 4]>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Level {
    pub z: u32,
    pub encoding: String,
    #[serde(default = "cells")]
    pub cells_per_tile: u32,
    #[serde(default = "step")]
    pub height_step_metres: f64,
    #[serde(default = "webp_quality")]
    pub webp_quality: u32,
    pub error_down_metres: Option<f64>,
    pub error_up_metres: Option<f64>,
}
fn webp_quality() -> u32 {
    90
}
fn cells() -> u32 {
    256
}
fn step() -> f64 {
    0.125
}
struct Dem {
    root: PathBuf,
    cache: HashMap<(u32, u32), Option<Vec<f64>>>,
    order: VecDeque<(u32, u32)>,
    hashes: BTreeMap<(u32, u32), Option<String>>,
}
impl Dem {
    fn tile(&mut self, x: u32, y: u32) -> Result<Option<&Vec<f64>>> {
        if !self.cache.contains_key(&(x, y)) {
            let path = self.root.join(x.to_string()).join(format!("{y}.png"));
            let (data, signature) = match fs::read(&path) {
                Ok(b) => (
                    Some(
                        crate::natural_inventory::decode(&b)
                            .with_context(|| format!("DEM {}", path.display()))?,
                    ),
                    Some(crate::natural_inventory::hash(&b)),
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, None),
                Err(e) => return Err(e.into()),
            };
            if let Some(previous) = self.hashes.get(&(x, y)) {
                ensure!(previous == &signature, "生成中に元DEMが変更されました");
            }
            self.hashes.insert((x, y), signature);
            self.cache.insert((x, y), data);
            self.order.push_back((x, y));
            while self.order.len() > 128 {
                if let Some(k) = self.order.pop_front() {
                    self.cache.remove(&k);
                }
            }
        }
        Ok(self.cache.get(&(x, y)).and_then(Option::as_ref))
    }
    fn pixel(&mut self, x: i64, y: i64) -> Result<Option<f64>> {
        let size = 16384i64 * 256;
        if y < 0 || y >= size {
            return Ok(None);
        }
        let x = x.rem_euclid(size);
        Ok(self
            .tile((x / 256) as u32, (y / 256) as u32)?
            .and_then(|v| {
                let h = v[((y % 256) * 256 + x % 256) as usize];
                h.is_finite().then_some(h)
            }))
    }
    fn sample(&mut self, x: f64, y: f64) -> Result<Option<f64>> {
        let x = x - 0.5;
        let y = y - 0.5;
        let ix = x.floor() as i64;
        let iy = y.floor() as i64;
        let fx = x - ix as f64;
        let fy = y - iy as f64;
        let mut sum = 0.;
        for (dx, dy, w) in [
            (0, 0, (1. - fx) * (1. - fy)),
            (1, 0, fx * (1. - fy)),
            (0, 1, (1. - fx) * fy),
            (1, 1, fx * fy),
        ] {
            if w > 1e-12 {
                let Some(v) = self.pixel(ix + dx, iy + dy)? else {
                    return Ok(None);
                };
                sum += v * w;
            }
        }
        Ok(Some(sum))
    }
}
struct GridTile {
    x: u32,
    y: u32,
    payload: Vec<u8>,
    down: f64,
    up: f64,
    valid: u64,
    missing: u64,
    hashes: BTreeMap<(u32, u32), Option<String>>,
}
fn grid_tile(d: &mut Dem, l: &Level, x: u32, y: u32, source: &[(u32, u32)]) -> Result<GridTile> {
    let mut down = 0f64;
    let mut up = 0f64;
    let mut valid = 0u64;
    let mut missing = 0u64;
    let size = (1u32 << (14 - l.z)) as f64 * 256.;
    let mut h = Vec::with_capacity((l.cells_per_tile as usize + 3).pow(2));
    for row in -1..=l.cells_per_tile as i64 + 1 {
        for col in -1..=l.cells_per_tile as i64 + 1 {
            h.push(d.sample(
                x as f64 * size + col as f64 * size / l.cells_per_tile as f64,
                y as f64 * size + row as f64 * size / l.cells_per_tile as f64,
            )?);
        }
    }
    let (map, payload) = if l.encoding == WEBP_MINMAX {
        yamamuki_terrain::heightmap_webp::encode_minmax(l.cells_per_tile, &h, l.webp_quality)?
    } else if l.encoding == WEBP {
        yamamuki_terrain::heightmap_webp::encode(
            l.cells_per_tile,
            l.height_step_metres,
            &h,
            l.webp_quality,
        )?
    } else {
        let map = Heightmap::quantize(l.cells_per_tile, l.height_step_metres, &h)?;
        let payload = map.encode()?;
        (map, payload)
    };
    for &(sx, sy) in source {
        let raw = d.tile(sx, sy)?.context("列挙済みのDEMが消えました")?;
        for (i, &truth) in raw.iter().enumerate() {
            if !truth.is_finite() {
                continue;
            }
            let u = (sx as f64 * 256. + (i % 256) as f64 + 0.5 - x as f64 * size) / size;
            let v = (sy as f64 * 256. + (i / 256) as f64 + 0.5 - y as f64 * size) / size;
            if let Some(value) = map.sample(u, v) {
                down = down.max(truth - value);
                up = up.max(value - truth);
                valid += 1;
            } else {
                missing += 1;
            }
        }
    }
    Ok(GridTile {
        x,
        y,
        payload,
        down,
        up,
        valid,
        missing,
        hashes: std::mem::take(&mut d.hashes),
    })
}

fn resolve(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}
fn intersects(z: u32, x: u32, y: u32, window: Option<[u32; 4]>) -> bool {
    window.is_none_or(|w| {
        let k = 1u32 << (14 - z);
        x * k < w[2] && (x + 1) * k > w[0] && y * k < w[3] && (y + 1) * k > w[1]
    })
}
pub fn build(config: &Path, output: &Path) -> Result<()> {
    let cfg: Config = serde_json::from_slice(&fs::read(config)?)?;
    let base = config.parent().unwrap_or(Path::new("."));
    ensure!(
        !cfg.levels.is_empty()
            && !cfg.data_version.is_empty()
            && cfg
                .data_version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
        "レベルまたは配布識別子が不正です"
    );
    let mut seen = BTreeSet::new();
    for l in &cfg.levels {
        ensure!(
            ((6..=13).contains(&l.z) || ((4..=5).contains(&l.z) && l.encoding == CONTROLS))
                && seen.insert(l.z)
                && (l.encoding == CONTROLS || is_encoding(&l.encoding)),
            "ズーム・形式が未対応または重複しています"
        );
        if is_encoding(&l.encoding) {
            ensure!(l.webp_quality <= 100, "WebP品質は0〜100が必要です");
            ensure!(
                (8..=256).contains(&l.cells_per_tile) && l.cells_per_tile.is_power_of_two(),
                "格子は8〜256セルの2の累乗が必要です"
            );
            Heightmap::quantize(
                l.cells_per_tile,
                l.height_step_metres,
                &vec![Some(0.); (l.cells_per_tile as usize + 3).pow(2)],
            )?;
        }
        for e in [l.error_down_metres, l.error_up_metres]
            .into_iter()
            .flatten()
        {
            ensure!(e.is_finite() && e > 0., "許容誤差が不正です");
        }
    }
    if let Some(w) = cfg.window {
        ensure!(
            w[0] < w[2] && w[1] < w[3] && w[2] <= 16384 && w[3] <= 16384,
            "z14の生成範囲が不正です"
        );
    }
    let mut controls = if cfg.levels.iter().any(|l| l.encoding == CONTROLS) {
        Some(Database::open(&resolve(
            base,
            cfg.control_database
                .as_deref()
                .context("制御点レベルにはcontrolDatabaseが必要です")?,
        ))?)
    } else {
        None
    };
    let mut dem = if cfg.levels.iter().any(|l| is_encoding(&l.encoding)) {
        let root = resolve(
            base,
            cfg.dem14
                .as_deref()
                .context("格子レベルにはdem14が必要です")?,
        );
        ensure!(
            root.file_name().is_some_and(|s| s == "14") && root.is_dir(),
            "DEMの14フォルダーがありません"
        );
        Some(Dem {
            root,
            cache: HashMap::new(),
            order: VecDeque::new(),
            hashes: BTreeMap::new(),
        })
    } else {
        None
    };
    let mut inputs = BTreeSet::new();
    if let Some(d) = &dem {
        for x in fs::read_dir(&d.root)? {
            let x = x?;
            if !x.file_type()?.is_dir() {
                continue;
            }
            let Ok(ix) = x.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            if cfg.window.is_some_and(|w| ix < w[0] || ix >= w[2]) {
                continue;
            }
            for y in fs::read_dir(x.path())? {
                let p = y?.path();
                if p.extension().is_none_or(|s| s != "png") {
                    continue;
                }
                let iy = p
                    .file_stem()
                    .context("DEM名なし")?
                    .to_string_lossy()
                    .parse::<u32>()?;
                ensure!(ix < 16384 && iy < 16384, "DEM座標が範囲外です");
                if cfg
                    .window
                    .is_none_or(|w| ix >= w[0] && ix < w[2] && iy >= w[1] && iy < w[3])
                {
                    inputs.insert((ix, iy));
                }
            }
        }
        ensure!(!inputs.is_empty(), "範囲内の元DEMがありません");
    }
    ensure!(!output.exists(), "出力先は新規ファイルを指定してください");
    let parent = output.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = output.with_extension(format!("{}-building.sqlite3", std::process::id()));
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    let result = (|| -> Result<()> {
        let mut db = Connection::open(&tmp)?;
        yamamuki_data_sqlite::initialize(&db)?;
        let tx = db.transaction()?;
        let mut metadata = if let Some(c) = &controls {
            let raw: String = c.connection.query_row(
                "SELECT metadata_json FROM datasets WHERE dataset_key=?1",
                [c.dataset],
                |r| r.get(0),
            )?;
            serde_json::from_str(&raw)?
        } else {
            json!({"authors":[{"name":"The yamamuki Authors","role":"配布データの編集・加工","url":"https://github.com/shohei0205/yamamuki-data/blob/main/AUTHORS.md"}],"sources":[{"name":"国土地理院の標高タイル（DEM10B）","source":"https://maps.gsi.go.jp/development/demtile.html","license":"公共データ利用規約（第1.0版） https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html","attribution":"地理院タイル（標高タイル）を加工して作成","changes":"標高格子と可逆圧縮の地形タイルへ加工"}]})
        };
        metadata["generation"] = json!({"levelFormats":cfg.levels.iter().map(|l|json!({"z":l.z,"encoding":l.encoding})).collect::<Vec<_>>(),"scope":if cfg.window.is_some(){"window-test"}else{"input-coverage"},"sourceWindowZ14":cfg.window,"heightmapSource":"GSI DEM10B z14"});
        if dem.is_none() {
            metadata["generation"]
                .as_object_mut()
                .unwrap()
                .remove("heightmapSource");
        }
        tx.execute("INSERT INTO datasets(id,kind,title,data_version,format_version,metadata_json) VALUES('gsi-terrain-japan','terrain','地形（国土地理院標高）',?1,1,?2)",params![cfg.data_version,metadata.to_string()])?;
        let dataset = tx.last_insert_rowid();
        let natural = cfg
            .levels
            .iter()
            .filter(|l| l.encoding == CONTROLS)
            .map(|l| l.z)
            .collect::<BTreeSet<_>>();
        let mut total = 0;
        for l in &cfg.levels {
            let mut settings = if l.encoding == CONTROLS {
                controls
                    .as_ref()
                    .unwrap()
                    .level_settings
                    .get(&l.z)
                    .context("制御点元に指定ズームがありません")?
                    .clone()
            } else {
                json!({"cellsPerTile":l.cells_per_tile,"halo":1,"heightStepMetres":l.height_step_metres,"interpolation":"bilinear","anchor":true,"maxParentDepth":0,"maxDependencies":1})
            };
            if l.encoding == CONTROLS {
                ensure!(
                    l.error_down_metres
                        .is_none_or(|e| settings["errorDownMetres"]
                            .as_f64()
                            .is_some_and(|v| v <= e))
                        && l.error_up_metres.is_none_or(|e| settings["errorUpMetres"]
                            .as_f64()
                            .is_some_and(|v| v <= e)),
                    "制御点元の品質が指定値を満たしていません"
                );
            }
            if l.encoding == WEBP {
                settings.as_object_mut().unwrap().remove("heightStepMetres");
                settings["minimumHeightStepMetres"] = json!(l.height_step_metres);
                settings["heightScale"] = json!("per-tile");
                settings["webpQuality"] = json!(l.webp_quality);
            }
            if l.encoding == WEBP_MINMAX {
                settings.as_object_mut().unwrap().remove("heightStepMetres");
                settings["heightScale"] = json!("min-max");
                settings["webpQuality"] = json!(l.webp_quality);
            }
            settings["encoding"] = json!(l.encoding);
            tx.execute(
                "INSERT INTO terrain_levels VALUES(?1,?2,?2,?3)",
                params![dataset, l.z, settings.to_string()],
            )?;
            let mut count = 0;
            if l.encoding == CONTROLS {
                let c = controls.as_mut().unwrap();
                ensure!(
                    c.level_formats[&l.z] == CONTROLS,
                    "controlDatabaseの指定レベルは制御点形式が必要です"
                );
                let mut keys = c
                    .catalog
                    .keys()
                    .copied()
                    .filter(|k| k.0 == l.z && intersects(k.0, k.1, k.2, cfg.window))
                    .collect::<Vec<_>>();
                keys.sort_unstable();
                for key in keys {
                    let mut tile = c.tile(key)?;
                    if tile
                        .parents
                        .iter()
                        .any(|p| !natural.contains(&p.0) || !intersects(p.0, p.1, p.2, cfg.window))
                    {
                        let points = c.points(key)?;
                        let local = points
                            .iter()
                            .map(|p| {
                                Ok([
                                    p[0].checked_sub(tile.origin[0])
                                        .context("局所Xの範囲超過")?,
                                    p[1].checked_sub(tile.origin[1])
                                        .context("局所Yの範囲超過")?,
                                    p[2],
                                ])
                            })
                            .collect::<Result<Vec<_>>>()?;
                        tile.controls = natural_codec::independent(&local)?;
                        tile.parents.clear();
                    }
                    tx.execute(
                        "INSERT INTO terrain_tiles VALUES(?1,?2,?3,?4,?5,?6)",
                        params![dataset, l.z, key.1, key.2, CONTROLS, tile.encode()?],
                    )?;
                    count += 1;
                }
            } else {
                let d = dem.as_mut().unwrap();
                let mut groups = BTreeMap::<(u32, u32), Vec<(u32, u32)>>::new();
                for &(x, y) in &inputs {
                    groups
                        .entry((x >> (14 - l.z), y >> (14 - l.z)))
                        .or_default()
                        .push((x, y));
                }
                let mut down = 0f64;
                let mut up = 0f64;
                let mut valid = 0u64;
                let mut missing = 0u64;
                let groups = groups.into_iter().collect::<Vec<_>>();
                // 復号キャッシュと生成結果のメモリを制限し、SQLiteへの書き込み順も固定する。
                for batch in groups.chunks(32) {
                    let results = batch
                        .par_iter()
                        .map_init(
                            || Dem {
                                root: d.root.clone(),
                                cache: HashMap::new(),
                                order: VecDeque::new(),
                                hashes: BTreeMap::new(),
                            },
                            |worker, ((x, y), source)| grid_tile(worker, l, *x, *y, source),
                        )
                        .collect::<Result<Vec<_>>>()?;
                    for tile in results {
                        for (key, hash) in tile.hashes {
                            if let Some(previous) = d.hashes.insert(key, hash.clone()) {
                                ensure!(previous == hash, "生成中に元DEMが変更されました");
                            }
                        }
                        down = down.max(tile.down);
                        up = up.max(tile.up);
                        valid += tile.valid;
                        missing += tile.missing;
                        tx.execute(
                            "INSERT INTO terrain_tiles VALUES(?1,?2,?3,?4,?5,?6)",
                            params![dataset, l.z, tile.x, tile.y, l.encoding, tile.payload],
                        )?;
                        count += 1;
                    }
                    eprintln!("z{} 格子生成 {}/{}タイル", l.z, count, groups.len());
                }
                ensure!(
                    l.error_down_metres.is_none_or(|e| down <= e)
                        && l.error_up_metres.is_none_or(|e| up <= e),
                    "z{}のハイトマップが指定の許容誤差を超えました（下{}m・上{}m）",
                    l.z,
                    down,
                    up
                );
                settings["errorDownMetres"] = json!(down.max(if l.encoding == WEBP_MINMAX {
                    1e-6
                } else {
                    l.height_step_metres
                }));
                settings["errorUpMetres"] = json!(up.max(if l.encoding == WEBP_MINMAX {
                    1e-6
                } else {
                    l.height_step_metres
                }));
                settings["coverageAudit"] = json!({"representedDemPixels":valid,"unrepresentedDemPixels":missing,"errorDomain":"represented-valid-dem-pixels"});
            }
            ensure!(count > 0, "z{}の収録タイルがありません", l.z);
            tx.execute(
                "UPDATE terrain_levels SET settings_json=?3 WHERE dataset_key=?1 AND level_key=?2",
                params![dataset, l.z, settings.to_string()],
            )?;
            total += count;
            eprintln!("z{} {}: {}タイル", l.z, l.encoding, count);
        }
        if let Some(d) = &dem {
            let mut digest = Sha256::new();
            for ((x, y), hash) in &d.hashes {
                digest.update(x.to_le_bytes());
                digest.update(y.to_le_bytes());
                match hash {
                    Some(v) => {
                        digest.update([1]);
                        digest.update(v.as_bytes());
                    }
                    None => digest.update([0]),
                }
            }
            metadata["generation"]["demInputsSha256"] = json!(format!("{:x}", digest.finalize()));
            metadata["generation"]["demInputTiles"] = json!(d.hashes.len());
        }
        if let Some(c) = &controls {
            metadata["generation"]["controlDataVersion"] = json!(c.generation);
        }
        tx.execute(
            "UPDATE datasets SET metadata_json=?2 WHERE dataset_key=?1",
            params![dataset, metadata.to_string()],
        )?;
        tx.commit()?;
        drop(db);
        let mut check = Database::open(&tmp)?;
        ensure!(check.validate_all()? == total, "格納タイル数が不一致です");
        drop(check);
        ensure!(!output.exists(), "出力先が既に存在します");
        fs::rename(&tmp, output)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use yamamuki_terrain::heightmap::ENCODING as GRID;
    #[test]
    fn parallel_grid_tiles_match_serial_audit() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("14");
        let mut groups = BTreeMap::<(u32, u32), Vec<(u32, u32)>>::new();
        for x in 14500u32..14504 {
            fs::create_dir_all(root.join(x.to_string()))?;
            for y in 6460u32..6464 {
                let mut raw = Vec::new();
                for i in 0..256u32 * 256 {
                    let height = 10000
                        + (x - 14500) * 600
                        + (y - 6460) * 400
                        + (i % 256) * 3
                        + (i / 256) * 5;
                    let packed = height.to_be_bytes();
                    raw.extend_from_slice(if i % 997 == 0 {
                        &[128, 0, 0]
                    } else {
                        &packed[1..]
                    });
                }
                let mut png = Vec::new();
                let mut encoder = png::Encoder::new(&mut png, 256, 256);
                encoder.set_color(png::ColorType::Rgb);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header()?.write_image_data(&raw)?;
                fs::write(root.join(x.to_string()).join(format!("{y}.png")), png)?;
                groups.entry((x >> 1, y >> 1)).or_default().push((x, y));
            }
        }
        let level: Level = serde_json::from_value(json!({"z":13,"encoding":GRID}))?;
        let fresh = || Dem {
            root: root.clone(),
            cache: HashMap::new(),
            order: VecDeque::new(),
            hashes: BTreeMap::new(),
        };
        let groups = groups.into_iter().collect::<Vec<_>>();
        let mut serial = fresh();
        let expected = groups
            .iter()
            .map(|((x, y), source)| grid_tile(&mut serial, &level, *x, *y, source))
            .collect::<Result<Vec<_>>>()?;
        let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build()?;
        let actual = pool.install(|| {
            groups
                .par_iter()
                .map_init(fresh, |d, ((x, y), source)| {
                    grid_tile(d, &level, *x, *y, source)
                })
                .collect::<Result<Vec<_>>>()
        })?;
        let hashes = |v: Vec<GridTile>| {
            let mut h = BTreeMap::new();
            for (i, tile) in v.into_iter().enumerate() {
                let old = &expected[i];
                assert_eq!(
                    (tile.x, tile.y, tile.down, tile.up, tile.valid, tile.missing),
                    (old.x, old.y, old.down, old.up, old.valid, old.missing)
                );
                assert_eq!(tile.payload, old.payload);
                for (key, hash) in tile.hashes {
                    h.insert(key, hash);
                }
            }
            h
        };
        let actual_hashes = hashes(actual);
        let expected_hashes = expected
            .iter()
            .flat_map(|t| t.hashes.clone())
            .collect::<BTreeMap<_, _>>();
        assert_eq!(actual_hashes, expected_hashes);
        Ok(())
    }

    #[test]
    fn builds_from_dem_and_mixes_levels() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("dem/14");
        fs::create_dir_all(root.join("14505"))?;
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, 256, 256);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header()?
                .write_image_data(&[0u8, 3, 232].repeat(256 * 256))?;
        }
        fs::write(root.join("14505/6465.png"), png)?;
        let controls = dir.path().join("controls.sqlite3");
        let mut c = Connection::open(&controls)?;
        yamamuki_data_sqlite::initialize(&c)?;
        yamamuki_terrain::fixture::create(&mut c)?;
        drop(c);
        let cfg = dir.path().join("config.json");
        fs::write(
            &cfg,
            serde_json::to_vec(
                &json!({"dem14":"dem/14","controlDatabase":"controls.sqlite3","dataVersion":"20261119T000000Z","levels":[{"z":12,"encoding":CONTROLS},{"z":13,"encoding":GRID,"cellsPerTile":8}]}),
            )?,
        )?;
        let output = dir.path().join("output.sqlite3");
        build(&cfg, &output)?;
        let mut db = Database::open(&output)?;
        assert_eq!(db.validate_all()?, 2);
        assert_eq!(
            yamamuki_data_sqlite::datasets(&db.connection)?[0].format_version,
            1
        );
        let k = *db.catalog.keys().find(|k| k.0 == 13).unwrap();
        assert_eq!(db.heightmap(k)?.unwrap().sample(0.75, 0.75), Some(10.));
        assert_eq!(db.level_formats[&12], CONTROLS);
        assert!(build(&cfg, &output).is_err());
        fs::write(
            &cfg,
            serde_json::to_vec(
                &json!({"dem14":"dem/14","dataVersion":"20261119T000000Z","levels":[{"z":13,"encoding":GRID,"cellsPerTile":8}]}),
            )?,
        )?;
        let pure = dir.path().join("pure.sqlite3");
        build(&cfg, &pure)?;
        assert_eq!(Database::open(&pure)?.validate_all()?, 1);
        fs::write(
            &cfg,
            serde_json::to_vec(
                &json!({"dem14":"dem/14","dataVersion":"20261119T000000Z","levels":[{"z":13,"encoding":WEBP,"cellsPerTile":8,"webpQuality":90}]}),
            )?,
        )?;
        let webp = dir.path().join("webp.sqlite3");
        build(&cfg, &webp)?;
        let mut decoded = Database::open(&webp)?;
        assert_eq!(decoded.validate_all()?, 1);
        assert_eq!(decoded.level_formats[&13], WEBP);
        assert!(decoded.heightmap(k)?.is_some());
        fs::write(
            &cfg,
            serde_json::to_vec(
                &json!({"dem14":"dem/14","dataVersion":"20261119T000000Z","levels":[{"z":13,"encoding":WEBP_MINMAX,"cellsPerTile":8,"webpQuality":90}]}),
            )?,
        )?;
        let minmax = dir.path().join("minmax.sqlite3");
        build(&cfg, &minmax)?;
        let mut decoded = Database::open(&minmax)?;
        assert_eq!(decoded.validate_all()?, 1);
        assert_eq!(decoded.level_formats[&13], WEBP_MINMAX);
        let map = decoded.heightmap(k)?.unwrap();
        assert_eq!(map.step, 0.);
        assert_eq!(map.sample(0.75, 0.75), Some(10.));

        let mut original = Database::open(&controls)?;
        let child = *original.catalog.keys().find(|k| k.0 == 13).unwrap();
        let parent = *original.catalog.keys().find(|k| k.0 == 12).unwrap();
        let expected = original.points(child)?;
        let parent_points = original.points(parent)?;
        let mut tile = original.tile(child)?;
        let local = expected
            .iter()
            .map(|p| [p[0] - tile.origin[0], p[1] - tile.origin[1], p[2]])
            .collect::<Vec<_>>();
        tile.controls = natural_codec::delta(
            &local,
            &vec![None; local.len()],
            &[(*parent_points).clone()],
        )?;
        tile.parents = vec![parent];
        let original_payload = tile.encode()?;
        drop(original);
        let edit = Connection::open(&controls)?;
        edit.execute(
            "UPDATE terrain_tiles SET payload=?1 WHERE level_key=13",
            [&original_payload],
        )?;
        drop(edit);
        fs::write(
            &cfg,
            serde_json::to_vec(
                &json!({"dem14":"dem/14","controlDatabase":"controls.sqlite3","dataVersion":"20261119T000000Z","levels":[{"z":12,"encoding":GRID,"cellsPerTile":8},{"z":13,"encoding":CONTROLS}]}),
            )?,
        )?;
        let switched = dir.path().join("switched.sqlite3");
        build(&cfg, &switched)?;
        let mut restored = Database::open(&switched)?;
        assert!(restored.tile(child)?.parents.is_empty());
        assert_eq!(*restored.points(child)?, *expected);
        drop(restored);
        let edit = Connection::open(&switched)?;
        edit.execute(
            "UPDATE terrain_tiles SET payload=?1 WHERE level_key=13",
            [original_payload],
        )?;
        drop(edit);
        assert!(Database::open(&switched)?.validate_all().is_err());
        Ok(())
    }
}
