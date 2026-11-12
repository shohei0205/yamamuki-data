//! DEMからタイル化した陰影の画質と容量を比較する。
use anyhow::{Context, Result, ensure};
use clap::Parser;
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::json;
use std::{collections::BTreeSet, fs, path::PathBuf};

const TILE: usize = 256;
const WORLD: f64 = 40_075_016.685_578_49;
const PAD: usize = 6;
const SOURCE: usize = 2 * TILE + 2 * PAD;
const FLAT: u8 = 136; // 8ビットと4ビットの両方で正確に表せる中立値。

#[derive(Parser)]
struct Args {
    #[arg(long)]
    dem: PathBuf,
    #[arg(long)]
    output: PathBuf,
    /// 全国生成で対象範囲を取得する地表SQLite。
    #[arg(long)]
    national_landcover: Option<PathBuf>,
    /// 全国陰影PNGのビット深度。
    #[arg(long, default_value_t = 8, requires = "national_landcover")]
    national_bits: u32,
    /// 明暗が急に変化する画素だけ4ビット相当に丸め、8ビットPNGに格納する。
    #[arg(long, requires = "national_landcover")]
    adaptive: bool,
    /// 明暗の変化に応じて8〜3ビット相当に丸める。
    #[arg(long, requires = "national_landcover", conflicts_with = "adaptive")]
    adaptive_graded: bool,
}

#[derive(Parser)]
struct MergeArgs {
    #[arg(long)]
    merge_hillshade: PathBuf,
    #[arg(long)]
    landcover: PathBuf,
    #[arg(long)]
    output: PathBuf,
    /// 森林統合版の砂礫地レイヤーを配布用コピーから除く。
    #[arg(long)]
    omit_bare: bool,
}

pub(super) fn merge() -> Result<()> {
    let args = MergeArgs::parse();
    let landcover = Connection::open_with_flags(&args.landcover, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let path_string = |path: &std::path::Path| {
        path.to_str()
            .map(str::to_owned)
            .context("SQLiteのパスがUTF-8ではありません")
    };
    let manifest: String = landcover.query_row(
        "SELECT value FROM metadata WHERE key='manifest'",
        [],
        |row| row.get(0),
    )?;
    let mut metadata: serde_json::Value = serde_json::from_str(&manifest)?;
    ensure!(
        metadata["layer"] == "combined" && metadata["tilePixels"] == TILE,
        "統合型の全国地表SQLiteが必要です"
    );
    if args.omit_bare {
        ensure!(
            matches!(
                metadata["landEncoding"].as_str(),
                Some("forest-bare-unified-gray4" | "forest-bare-unified-gray2")
            ),
            "砂礫地の省略には森林統合版の地表SQLiteが必要です"
        );
        metadata["bareOmitted"] = json!(true);
        if let Some(layers) = metadata["layers"].as_object_mut() {
            layers.remove("bare");
        }
        if let Some(levels) = metadata["levels"].as_array_mut() {
            for level in levels {
                let total = level["payloadBytes"]
                    .as_u64()
                    .context("地表レベルの容量がありません")?;
                let bare = level["barePayloadBytes"]
                    .as_u64()
                    .context("砂礫地レベルの容量がありません")?;
                level["payloadBytes"] = json!(
                    total
                        .checked_sub(bare)
                        .context("地表レベルの容量が不正です")?
                );
                level["barePayloadBytes"] = json!(0);
            }
        }
    }
    fs::create_dir(&args.output).context("新しい出力フォルダーを指定してください")?;
    let temp = args.output.join("landcover-with-hillshade.sqlite3.tmp");
    let path = args.output.join("landcover-with-hillshade.sqlite3");
    landcover.execute("VACUUM INTO ?1", [path_string(&temp)?])?;
    let mut output = Connection::open(&temp)?;
    output.execute(
        "ATTACH DATABASE ?1 AS shade",
        [format!(
            "file:{}?mode=ro",
            path_string(&args.merge_hillshade.canonicalize()?)?
                .trim_start_matches(r"\\?\")
                .replace('\\', "/")
                .replace('%', "%25")
                .replace('#', "%23")
                .replace('?', "%3F")
        )],
    )?;
    let tx = output.transaction()?;
    if args.omit_bare {
        tx.execute("DROP TABLE main.bare_tiles", [])?;
        tx.execute(
            "UPDATE main.metadata SET value=?1 WHERE key='manifest'",
            [metadata.to_string()],
        )?;
    }
    let metadata: String = tx.query_row(
        "SELECT value FROM shade.metadata WHERE key='manifest'",
        [],
        |row| row.get(0),
    )?;
    let spec: serde_json::Value = serde_json::from_str(&metadata)?;
    ensure!(
        spec["type"] == "hillshade"
            && spec["version"] == 1
            && spec["scheme"] == "xyz"
            && spec["tilePixels"] == TILE
            && matches!(
                (spec["encoding"].as_str(), spec["bitDepth"].as_u64()),
                (Some("png-gray4"), Some(4)) | (Some("png-gray8"), Some(8))
            )
            && spec["neutral"] == FLAT,
        "全国陰影SQLite v1 が必要です"
    );
    let outside: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM shade.hillshade_tiles h
         WHERE NOT EXISTS(SELECT 1 FROM main.tiles l WHERE (l.z,l.x,l.y)=(h.z,h.x,h.y)))",
        [],
        |row| row.get(0),
    )?;
    ensure!(!outside, "地表SQLiteの範囲外に陰影タイルがあります");
    tx.execute_batch("CREATE TABLE hillshade_tiles(z INTEGER,x INTEGER,y INTEGER,payload BLOB NOT NULL,PRIMARY KEY(z,x,y)) WITHOUT ROWID;
        INSERT INTO main.hillshade_tiles SELECT * FROM shade.hillshade_tiles;")?;
    tx.execute(
        "INSERT INTO main.metadata VALUES ('hillshade',?1)",
        [metadata],
    )?;
    tx.commit()?;
    if args.omit_bare {
        output.execute_batch("VACUUM main")?;
    }
    let check: String = output.query_row("PRAGMA main.integrity_check", [], |row| row.get(0))?;
    ensure!(
        check == "ok",
        "地表SQLiteの整合性検査に失敗しました: {check}"
    );
    drop(output);
    fs::rename(temp, &path)?;
    println!("Complete: {}", path.display());
    Ok(())
}

#[derive(Parser)]
struct PackArgs {
    #[arg(long)]
    pack_ground_512: PathBuf,
    #[arg(long)]
    output: PathBuf,
    /// Keep the packed z10 footprint, but store 384 samples per side.
    #[arg(long)]
    l2_384: bool,
}

// Each four source samples cover three destination samples, with exact area weights.
fn resize_512_to_384(source: &[u8], bits: usize) -> Vec<u8> {
    const WEIGHTS: [[u16; 4]; 3] = [[3, 1, 0, 0], [0, 2, 2, 0], [0, 0, 1, 3]];
    let mask = ((1u16 << bits) - 1) as u8;
    let read = |x: usize, y: usize| {
        let bit = (y * 512 + x) * bits;
        (source[bit / 8] >> (8 - bits - bit % 8)) & mask
    };
    let mut output = vec![0; 384 * 384 * bits / 8];
    for y in 0..384 {
        let wy = WEIGHTS[y % 3];
        for x in 0..384 {
            let wx = WEIGHTS[x % 3];
            let mut sum = 0;
            for (dy, &v) in wy.iter().enumerate().filter(|&(_, &w)| w != 0) {
                for (dx, &u) in wx.iter().enumerate().filter(|&(_, &w)| w != 0) {
                    sum += u * v * u16::from(read(x / 3 * 4 + dx, y / 3 * 4 + dy));
                }
            }
            let value = ((sum + 8) / 16) as u8;
            let bit = (y * 384 + x) * bits;
            output[bit / 8] |= value << (8 - bits - bit % 8);
        }
    }
    output
}

// 4ビットの森林被覆率を面積平均し、親タイルの対応する1/4に格納する。
fn replace_forest_quadrant(
    parent: &mut [u8],
    child: &[u8],
    parent_side: usize,
    child_side: usize,
    dx: usize,
    dy: usize,
) {
    let quadrant_side = parent_side / 2;
    let parent_stride = parent_side / 2;
    // 各出力画素が重なる入力画素は2個。入力画素の幅を units で表す。
    let (step, units) = if parent_side == child_side {
        (2, 1)
    } else {
        (3, 2)
    };
    debug_assert_eq!(child_side * units, quadrant_side * step);
    let span = |i: usize| {
        let position = i * step;
        let first = units - position % units;
        (position / units, first, step - first)
    };
    let read = |x: usize, y: usize| {
        let i = y * child_side + x;
        usize::from((child[i / 2] >> (4 * (1 - i % 2))) & 15)
    };
    let denominator = step * step;
    for y in 0..quadrant_side {
        let (sy, top, bottom) = span(y);
        let start = (dy * quadrant_side + y) * parent_stride + dx * (quadrant_side / 2);
        let average = |x| {
            let (sx, left, right) = span(x);
            let sum = top * (left * read(sx, sy) + right * read(sx + 1, sy))
                + bottom * (left * read(sx, sy + 1) + right * read(sx + 1, sy + 1));
            ((sum + denominator / 2) / denominator) as u8
        };
        for (x, byte) in parent[start..start + quadrant_side / 2]
            .iter_mut()
            .enumerate()
        {
            *byte = average(2 * x) << 4 | average(2 * x + 1);
        }
    }
}

#[test]
fn forest_parent_averages_child_coverage_in_the_matching_quadrant() {
    let child = [0xf0, 0x12, 0x00, 0x01, 0xff, 0x00, 0xff, 0x00];
    let mut parent = [0xaa; 8];
    replace_forest_quadrant(&mut parent, &child, 4, 4, 1, 1);
    assert_eq!(parent, [0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0x41, 0xaa, 0xf0]);
}

#[test]
fn forest_parent_uses_exact_weights_when_child_is_384_and_parent_is_512() {
    let child_side = 384;
    let mut child = vec![0; child_side * child_side / 2];
    child[0] = 0xf0;
    child[1] = 0xf0;
    child[child_side / 2 + 1] = 0xf0;
    let mut parent = vec![0xaa; 512 * 512 / 2];
    replace_forest_quadrant(&mut parent, &child, 512, child_side, 0, 1);
    let start = 256 * 256;
    assert_eq!(parent[start], 0x7a);
    assert_eq!(parent[start + 256], 0x03);
    assert_eq!(parent[start + 128], 0xaa);
    assert_eq!(parent[start - 256], 0xaa);
}

fn packed_side(z: u32, compact: bool) -> u32 {
    if z >= 11 && compact { 384 } else { 512 }
}

pub(super) fn pack_ground() -> Result<()> {
    let args = PackArgs::parse();
    let source =
        Connection::open_with_flags(&args.pack_ground_512, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let read_metadata = |key: &str| -> Result<serde_json::Value> {
        let value: String =
            source.query_row("SELECT value FROM metadata WHERE key=?1", [key], |r| {
                r.get(0)
            })?;
        Ok(serde_json::from_str(&value)?)
    };
    let mut land = read_metadata("manifest")?;
    let mut shade = read_metadata("hillshade")?;
    ensure!(
        land["bareOmitted"] == true && land["tilePixels"] == TILE && shade["bitDepth"] == 8,
        "砂礫地を省いた256画素・8ビット陰影の地表SQLiteが必要です"
    );
    fs::create_dir(&args.output).context("新しい出力フォルダーを指定してください")?;
    let temp = args.output.join("landcover-with-hillshade.sqlite3.tmp");
    let path = args.output.join("landcover-with-hillshade.sqlite3");
    let mut db = Connection::open(&temp)?;
    db.pragma_update(None, "page_size", 512)?;
    let tx = db.transaction()?;
    tx.execute_batch(
        "CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;",
    )?;
    let keys = source
        .prepare("SELECT DISTINCT z,x/2,y/2 FROM tiles ORDER BY 1 DESC,2,3")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<Key>>>()?;
    let finest_zoom = keys.first().map_or(0, |&(z, _, _)| z);
    let mut stats = std::collections::BTreeMap::new();
    for (table, bits, neutral) in [
        ("tiles", 4, 0),
        ("water_tiles", 4, 0),
        ("hillshade_tiles", 8, FLAT),
    ] {
        tx.execute_batch(&format!("CREATE TABLE {table}(z INTEGER,x INTEGER,y INTEGER,payload BLOB NOT NULL,PRIMARY KEY(z,x,y)) WITHOUT ROWID;"))?;
        let mut read = source.prepare(&format!(
            "SELECT payload FROM {table} WHERE z=?1 AND x=?2 AND y=?3"
        ))?;
        let mut write = tx.prepare(&format!("INSERT INTO {table} VALUES (?,?,?,?)"))?;
        let mut read_packed_forest =
            tx.prepare("SELECT payload FROM tiles WHERE z=?1 AND x=?2 AND y=?3")?;
        for &(z, x, y) in &keys {
            let stride = TILE * bits / 8;
            let mut image = vec![neutral; 4 * TILE * stride];
            for dy in 0..2 {
                for dx in 0..2 {
                    let child = (z, 2 * x + dx, 2 * y + dy);
                    let blob: Option<Vec<u8>> = read.query_row(child, |r| r.get(0)).optional()?;
                    let mut pixels = vec![neutral; TILE * stride];
                    if let Some(blob) = blob {
                        let mut reader =
                            png::Decoder::new(std::io::Cursor::new(blob)).read_info()?;
                        let info = reader.info();
                        ensure!(
                            info.width == TILE as u32
                                && info.height == TILE as u32
                                && info.color_type == png::ColorType::Grayscale
                                && info.bit_depth as usize == bits,
                            "Unexpected PNG {table} {child:?}"
                        );
                        reader.next_frame(&mut pixels)?;
                    }
                    for (row, line) in pixels.chunks_exact(stride).enumerate() {
                        let start = (2 * (dy as usize * TILE + row) + dx as usize) * stride;
                        image[start..start + stride].copy_from_slice(line);
                    }
                }
            }
            if table == "hillshade_tiles"
                && land["completeCoarseTiles"] != true
                && image.iter().all(|&p| p == neutral)
            {
                continue;
            }
            let side = packed_side(z, args.l2_384);
            if side == 384 {
                image = resize_512_to_384(&image, bits);
            }
            // 森林は最も細かいレベルから順に縮小し、直上が欠ける区画は元画像で埋める。
            if table == "tiles" && z < finest_zoom {
                let child_side = packed_side(z + 1, args.l2_384) as usize;
                let mut child = vec![0; child_side * child_side / 2];
                for dy in 0..2 {
                    for dx in 0..2 {
                        let blob: Option<Vec<u8>> = read_packed_forest
                            .query_row(params![z + 1, 2 * x + dx as u32, 2 * y + dy as u32], |r| {
                                r.get(0)
                            })
                            .optional()?;
                        if let Some(blob) = blob {
                            let mut reader =
                                png::Decoder::new(std::io::Cursor::new(blob)).read_info()?;
                            reader.next_frame(&mut child)?;
                            replace_forest_quadrant(
                                &mut image,
                                &child,
                                side as usize,
                                child_side,
                                dx,
                                dy,
                            );
                        }
                    }
                }
            }
            let mut png = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut png, side, side);
                encoder.set_color(png::ColorType::Grayscale);
                encoder.set_depth(match bits {
                    4 => png::BitDepth::Four,
                    _ => png::BitDepth::Eight,
                });
                encoder.set_compression(png::Compression::High);
                let mut writer = encoder.write_header()?;
                writer.write_image_data(&image)?;
                writer.finish()?;
            }
            let [count, bytes] = stats.entry((table, z)).or_insert([0usize; 2]);
            *count += 1;
            *bytes += png.len();
            write.execute(params![z, x, y, png])?;
        }
        println!("Packed {table}");
    }
    if let Some(layers) = land["layers"].as_object_mut() {
        layers.remove("landMissing");
    }
    let stat = |table, z| stats.get(&(table, z)).copied().unwrap_or([0; 2]);
    for spec in [&mut land, &mut shade] {
        spec["tilePixels"] = json!(512);
        spec["tileKeyZoomOffset"] = json!(-1);
        if args.l2_384 {
            spec["l2TilePixels"] = json!(384);
        }
    }
    for (key, table) in [("levels", "tiles"), ("waterLevels", "water_tiles")] {
        for level in land[key]
            .as_array_mut()
            .context("Missing landcover levels")?
        {
            let z = level["z"].as_u64().context("Missing zoom")? as u32;
            let [count, bytes] = stat(table, z);
            level["tiles"] = json!(count);
            if args.l2_384 && z >= 11 {
                level["tilePixels"] = json!(384);
                level["projectedMetresPerPixel"] = json!(WORLD / f64::from(1u32 << (z - 1)) / 384.);
            }
            if table == "tiles" {
                level
                    .as_object_mut()
                    .context("Invalid land level")?
                    .remove("landMissingPayloadBytes");
                level["forestPayloadBytes"] = json!(bytes);
                level["payloadBytes"] = json!(bytes);
            } else {
                level["payloadBytes"] = json!(bytes);
            }
        }
    }
    for level in shade["levels"]
        .as_array_mut()
        .context("Missing shade levels")?
    {
        let z = level["z"].as_u64().context("Missing zoom")? as u32;
        let [count, bytes] = stat("hillshade_tiles", z);
        level
            .as_object_mut()
            .context("Invalid shade level")?
            .remove("noDem");
        level["target"] = json!(stat("tiles", z)[0]);
        level["stored"] = json!(count);
        if args.l2_384 && z >= 11 {
            level["tilePixels"] = json!(384);
        }
        level["pngBytes"] = json!(bytes);
    }
    for (key, value) in [("manifest", land), ("hillshade", shade)] {
        tx.execute(
            "INSERT INTO metadata VALUES (?,?)",
            params![key, value.to_string()],
        )?;
    }
    tx.commit()?;
    let check: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(
        check == "ok",
        "地表SQLiteの整合性検査に失敗しました: {check}"
    );
    drop(db);
    fs::rename(temp, &path)?;
    println!("Complete: {}", path.display());
    Ok(())
}

type Key = (u32, u32, u32);

fn source_grid(db: &Connection, (z, x, y): Key) -> Result<Vec<f32>> {
    let start_x = x as usize * 2 * TILE - PAD;
    let start_y = y as usize * 2 * TILE - PAD;
    let end_x = start_x + SOURCE;
    let end_y = start_y + SOURCE;
    let mut grid = vec![f32::NAN; SOURCE * SOURCE];
    let mut query = db.prepare_cached(
        "SELECT x,y,data FROM tiles WHERE z=?1 AND x BETWEEN ?2 AND ?3 AND y BETWEEN ?4 AND ?5",
    )?;
    let mut rows = query.query((
        z + 1,
        start_x / TILE,
        (end_x - 1) / TILE,
        start_y / TILE,
        (end_y - 1) / TILE,
    ))?;
    while let Some(row) = rows.next()? {
        let tx: usize = row.get(0)?;
        let ty: usize = row.get(1)?;
        let data = crate::codec::decode_grid(row.get_ref(2)?.as_blob()?)?;
        ensure!(
            data.rows == TILE && data.columns == TILE,
            "標高タイルの寸法が不正です"
        );
        let left = start_x.max(tx * TILE);
        let right = end_x.min((tx + 1) * TILE);
        let width = right - left;
        for y in start_y.max(ty * TILE)..end_y.min((ty + 1) * TILE) {
            let src = (y - ty * TILE) * TILE + left - tx * TILE;
            let dst = (y - start_y) * SOURCE + left - start_x;
            grid[dst..dst + width].copy_from_slice(&data.values[src..src + width]);
        }
    }
    Ok(grid)
}

fn heights(grid: &[f32]) -> Vec<f32> {
    // 欠損は重みに含めず、実標高のある中心だけを平滑化する。
    const WEIGHTS: [f32; 5] = [1., 4., 6., 4., 1.];
    let horizontal: Vec<[f32; 2]> = grid
        .par_iter()
        .enumerate()
        .map(|(i, _)| {
            let x = i % SOURCE;
            if !(2..SOURCE - 2).contains(&x) {
                return [0., 0.];
            }
            grid[i - 2..=i + 2]
                .iter()
                .zip(WEIGHTS)
                .fold([0., 0.], |[sum, weight], (&h, w)| {
                    if h.is_finite() {
                        [sum + h * w, weight + w]
                    } else {
                        [sum, weight]
                    }
                })
        })
        .collect();
    grid.par_iter()
        .enumerate()
        .map(|(i, h)| {
            let y = i / SOURCE;
            if !h.is_finite() || !(2..SOURCE - 2).contains(&y) {
                return f32::NAN;
            }
            let [sum, weights] =
                WEIGHTS
                    .into_iter()
                    .enumerate()
                    .fold([0., 0.], |[sum, weight], (dy, w)| {
                        let [a, b] = horizontal[i - 2 * SOURCE + dy * SOURCE];
                        [sum + a * w, weight + b * w]
                    });
            sum / weights
        })
        .collect()
}

fn derivative(a: f32, b: f32, center: f32, step: f32) -> f32 {
    match (a.is_finite(), b.is_finite()) {
        (true, true) => (b - a) / (2. * step),
        (true, false) => (center - a) / step,
        (false, true) => (b - center) / step,
        (false, false) => 0.,
    }
}

fn light_response(east: f32, south: f32, multi: bool) -> u8 {
    let norm = (1. + east * east + south * south).sqrt();
    let direction = |a: f32| {
        let (sin, cos) = a.to_radians().sin_cos();
        ((1. - east * sin + south * cos) / (std::f32::consts::SQRT_2 * norm)).max(0.)
    };
    let light = if multi {
        let aspect = (-east).atan2(south);
        let (sum, weight) =
            [225_f32, 270., 315., 360.]
                .into_iter()
                .fold((0., 0.), |(sum, weight), a| {
                    let w = (aspect - a.to_radians()).sin().powi(2);
                    (sum + w * direction(a), weight + w)
                });
        sum / weight
    } else {
        direction(315.)
    };
    (f32::from(FLAT) + 145. * (light - std::f32::consts::FRAC_1_SQRT_2))
        .round()
        .clamp(0., 255.) as u8
}

fn shade<const N: usize>(
    grid: &[f32],
    (z, _, y): Key,
    multi: [bool; N],
    halo: usize,
) -> Vec<[u8; N]> {
    let h = heights(grid);
    let projected_step = WORLD / f64::from(1u32 << (z + 1)) / TILE as f64;
    let side = TILE + 2 * halo;
    let pad = PAD - 2 * halo;
    (0..side * side)
        .into_par_iter()
        .map(|i| {
            let row = i / side;
            let col = i % side;
            let p = (pad + 2 * row) * SOURCE + pad + 2 * col;
            let world_y = (y as f64 * TILE as f64 + row as f64 - halo as f64 + 0.5)
                / (f64::from(1u32 << z) * TILE as f64);
            let lat = (std::f64::consts::PI * (1. - 2. * world_y)).sinh().atan();
            let step = (projected_step * lat.cos()) as f32;
            let mut east = 0.;
            let mut south = 0.;
            let mut count = 0;
            for p in [p, p + 1, p + SOURCE, p + SOURCE + 1] {
                let center = h[p];
                if center.is_finite() {
                    east += derivative(h[p - 1], h[p + 1], center, step);
                    south += derivative(h[p - SOURCE], h[p + SOURCE], center, step);
                    count += 1;
                }
            }
            if count == 0 {
                return [FLAT; N];
            }
            let east = east / count as f32;
            let south = south / count as f32;
            multi.map(|multi| light_response(east, south, multi))
        })
        .collect()
}

// 明暗が急に折れる場所を選び、一定勾配の斜面は8ビットのまま残す。
// Strongest threshold first. Values are in the 8-bit shade signal.
const GRADED_THRESHOLDS: [(u8, i16, u8); 5] = [
    (52, 26, 3),
    (36, 18, 4),
    (24, 12, 5),
    (18, 9, 6),
    (12, 6, 7),
];

fn graded_depth(range: u8, curvature: i16) -> u8 {
    GRADED_THRESHOLDS
        .into_iter()
        .find(|&(min_range, min_curvature, _)| range >= min_range && curvature >= min_curvature)
        .map_or(8, |(_, _, depth)| depth)
}

fn round_shade(pixel: u8, depth: u8) -> u8 {
    let step = 1i16 << (8 - depth);
    let offset = i16::from(FLAT) % step;
    let bin = ((i16::from(pixel) - offset + step / 2).div_euclid(step)).clamp(0, 255 / step);
    (offset + bin * step) as u8
}

#[test]
fn graded_shade_preserves_neutral_and_selects_each_depth() {
    assert_eq!(graded_depth(72, 0), 8);
    for (range, curvature, bits) in GRADED_THRESHOLDS {
        assert_eq!(graded_depth(range, curvature), bits);
    }
    for bits in 3..=8 {
        assert_eq!(round_shade(FLAT, bits), FLAT);
        let distinct: BTreeSet<_> = (0..=255).map(|pixel| round_shade(pixel, bits)).collect();
        assert_eq!(distinct.len(), 1 << bits);
    }
}

fn adaptive_pixel([a, b, c, d, center, e, f, g, h]: [u8; 9], graded: bool) -> u8 {
    let values = [a, b, c, d, center, e, f, g, h];
    let range = values.iter().max().unwrap() - values.iter().min().unwrap();
    let curvature = [(a, h), (b, g), (c, f), (d, e)]
        .into_iter()
        .map(|(before, after)| (i16::from(before) + i16::from(after) - 2 * i16::from(center)).abs())
        .max()
        .unwrap();
    if graded {
        round_shade(center, graded_depth(range, curvature))
    } else if range >= 24 && curvature >= 12 {
        gray4(center) * 17
    } else {
        center
    }
}

fn adaptive_shade(padded: &[[u8; 1]], graded: bool) -> Vec<u8> {
    let side = TILE + 2;
    (0..TILE * TILE)
        .map(|i| {
            let start = i / TILE * side + i % TILE;
            adaptive_pixel(
                std::array::from_fn(|j| padded[start + j / 3 * side + j % 3][0]),
                graded,
            )
        })
        .collect()
}

#[test]
fn adaptive_shade_keeps_smooth_ramps_and_quantizes_a_sharp_ridge() {
    assert_eq!(
        adaptive_pixel([108, 122, 136, 114, 128, 142, 120, 134, 148], false),
        128
    );
    assert_eq!(
        adaptive_pixel([96, 96, 96, 96, 128, 96, 96, 96, 96], false),
        136
    );
    assert_eq!(adaptive_pixel([FLAT; 9], false), FLAT);
}

fn gray4(pixel: u8) -> u8 {
    ((u16::from(pixel) + 8) / 17) as u8
}

fn encode_png(pixels: &[u8], side: usize, bits: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, side as u32, side as u32);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(if bits == 4 {
        png::BitDepth::Four
    } else {
        png::BitDepth::Eight
    });
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header()?;
    if bits == 4 {
        let packed: Vec<_> = pixels
            .chunks_exact(2)
            .map(|p| gray4(p[0]) << 4 | gray4(p[1]))
            .collect();
        writer.write_image_data(&packed)?;
    } else {
        writer.write_image_data(pixels)?;
    }
    writer.finish()?;
    Ok(out)
}

fn run_national(
    dem: &Connection,
    landcover: &std::path::Path,
    output: &std::path::Path,
    bits: u32,
    adaptive: bool,
    graded: bool,
) -> Result<()> {
    let adaptive = adaptive || graded;
    ensure!(!adaptive || bits == 8, "適応量子化には8ビットPNGが必要です");
    ensure!(
        matches!(bits, 4 | 8),
        "全国陰影のビット深度は4または8を指定してください"
    );
    let coverage = Connection::open_with_flags(landcover, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let keys = coverage
        .prepare("SELECT z,x,y FROM tiles WHERE z BETWEEN 6 AND 13 ORDER BY z,x,y")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<Vec<Key>>>()?;
    let zooms: BTreeSet<_> = keys.iter().map(|&(z, _, _)| z).collect();
    ensure!(
        (10..=13).any(|max| zooms == (6..=max).collect()),
        "地表SQLiteの処理用レベルz6〜z13が連続していません"
    );

    fs::create_dir(output).context("新しい出力フォルダーを指定してください")?;
    let temp = output.join("hillshade-national.sqlite3.tmp");
    let path = output.join("hillshade-national.sqlite3");
    let mut db = Connection::open(&temp)?;
    db.pragma_update(None, "page_size", 512)?;
    let tx = db.transaction()?;
    tx.execute_batch("CREATE TABLE hillshade_tiles(z INTEGER,x INTEGER,y INTEGER,payload BLOB NOT NULL,PRIMARY KEY(z,x,y)) WITHOUT ROWID;
        CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;")?;

    let mut levels = std::collections::BTreeMap::new();
    {
        let mut insert = tx.prepare("INSERT INTO hillshade_tiles VALUES (?,?,?,?)")?;
        for (i, &key) in keys.iter().enumerate() {
            let (z, x, y) = key;
            let counts = levels.entry(z).or_insert([0usize; 4]);
            counts[0] += 1;
            let grid = source_grid(dem, key).with_context(|| format!("z{z}/{x}/{y} の標高"))?;
            let has_dem = (PAD..PAD + 2 * TILE).any(|row| {
                grid[row * SOURCE + PAD..row * SOURCE + PAD + 2 * TILE]
                    .iter()
                    .any(|h| h.is_finite())
            });
            if has_dem {
                let samples = shade(&grid, key, [false], usize::from(adaptive));
                let pixels: Vec<_> = if adaptive {
                    adaptive_shade(&samples, graded)
                } else {
                    samples.into_iter().map(|[p]| p).collect()
                };
                let has_shade = if bits == 4 {
                    pixels.iter().any(|&p| gray4(p) != gray4(FLAT))
                } else {
                    pixels.iter().any(|&p| p != FLAT)
                };
                if has_shade {
                    let png = encode_png(&pixels, TILE, bits)?;
                    counts[1] += 1;
                    counts[3] += png.len();
                    insert.execute(params![z, x, y, png])?;
                }
            } else {
                counts[2] += 1;
            }
            if (i + 1) % 25 == 0 || i + 1 == keys.len() {
                println!("{}/{} z{z}/{x}/{y}", i + 1, keys.len());
            }
        }
    }

    let stats: Vec<_> = levels.into_iter().map(|(z, [target, stored, no_dem, bytes])| {
        json!({"z":z,"target":target,"stored":stored,"noDem":no_dem,"pngBytes":bytes})
    }).collect();
    let report = json!({"version":1,"type":"hillshade","crs":"EPSG:3857","scheme":"xyz",
        "tilePixels":TILE,"bitDepth":bits,"encoding":format!("png-gray{bits}"),"neutral":FLAT,
        "missingTile":"neutral","method":"single","azimuths":[315],
        "quantization":if graded { "adaptive-graded-8-to-3" } else if adaptive { "adaptive-range24-curvature12" } else { "uniform" },
        "gradedThresholds":graded.then_some(GRADED_THRESHOLDS),
        "lightAltitude":45,"source":"国土地理院 DEM10Bを加工した標高SQLite","levels":stats});
    tx.execute(
        "INSERT INTO metadata VALUES ('manifest',?)",
        [report.to_string()],
    )?;
    tx.commit()?;
    let check: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    ensure!(
        check == "ok",
        "陰影SQLiteの整合性検査に失敗しました: {check}"
    );
    drop(db);
    fs::rename(&temp, &path)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    println!("Complete: {}", path.display());
    Ok(())
}

pub(super) fn run() -> Result<()> {
    let args = Args::parse();
    let dem = Connection::open_with_flags(&args.dem, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let landcover = args
        .national_landcover
        .context("全国地表SQLiteを指定してください")?;
    run_national(
        &dem,
        &landcover,
        &args.output,
        args.national_bits,
        args.adaptive,
        args.adaptive_graded,
    )
}
#[test]
fn flat_light_is_neutral_and_aspect_changes_the_shade() {
    for multi in [false, true] {
        assert_eq!(light_response(0., 0., multi), FLAT);
        assert_ne!(
            light_response(0.5, 0., multi),
            light_response(-0.5, 0., multi)
        );
    }
    assert_eq!(derivative(f32::NAN, 50., 50., 2.), 0.);
}
