//! 同じ詳細標高から「法線の縮小」と「照明を計算してから縮小」を全国で比較する。
use crate::{codec, write_json};
use anyhow::{Result, ensure};
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};
const SOURCE: u32 = 14;
const OVERVIEW: u32 = 6;
const DETAIL: u32 = 7;
const NORMAL_SIDE: usize = 256 >> (SOURCE - DETAIL);
const SHADE_SIDE: usize = 256 >> (SOURCE - OVERVIEW);
const NORMAL_BLOCK: usize = 1 << (SOURCE - DETAIL);
const SHADE_BLOCK: usize = 1 << (SOURCE - OVERVIEW);
fn read(db: &Connection, z: u32, x: u32, y: u32) -> Result<Option<Vec<f32>>> {
    let b: Option<Vec<u8>> = db
        .query_row(
            "SELECT data FROM tiles WHERE z=?1 AND x=?2 AND y=?3",
            (z, x, y),
            |r| r.get(0),
        )
        .optional()?;
    b.map(|b| {
        let g = codec::decode_grid(&b)?;
        ensure!(
            g.rows == 256 && g.columns == 256,
            "256 四方の標高が必要です"
        );
        Ok(g.values)
    })
    .transpose()
}
fn unit(v: [f64; 3]) -> [f64; 3] {
    let len = v.iter().map(|v| v * v).sum::<f64>().sqrt();
    if len < 1e-12 {
        [0., 0., 1.]
    } else {
        v.map(|v| v / len)
    }
}
fn light_direction(azimuth: f64, altitude: f64) -> [f64; 3] {
    let a = azimuth.to_radians();
    let h = altitude.to_radians();
    [a.sin() * h.cos(), a.cos() * h.cos(), h.sin()]
}
fn illuminate(n: [f64; 3], light: [f64; 3]) -> f64 {
    n.iter().zip(light).map(|(n, l)| n * l).sum::<f64>().max(0.)
}
fn solar_direction(angles: [f64; 2], latitude: f64, longitude: f64) -> [f64; 3] {
    let d = angles[0];
    let h = angles[1] + longitude;
    [
        -d.cos() * h.sin(),
        d.sin() * latitude.cos() - d.cos() * h.cos() * latitude.sin(),
        d.sin() * latitude.sin() + d.cos() * h.cos() * latitude.cos(),
    ]
}
fn sun_illuminate(n: [f64; 3], light: [f64; 3], dated: bool) -> f64 {
    if dated && light[2] <= 0. {
        0.
    } else {
        illuminate(n, light)
    }
}
fn pack(n: [f64; 3]) -> [u8; 3] {
    unit(n).map(|v| ((v + 1.) * 127.5).round().clamp(0., 255.) as u8)
}
struct Part {
    x: u32,
    y: u32,
    normal: Vec<[f64; 3]>,
    counts: Vec<u32>,
    detail_shade: Vec<f64>,
    shade: Vec<f64>,
    valid: Vec<u32>,
}
fn process(
    source: &Path,
    x: u32,
    y: u32,
    light: [f64; 3],
    solar: Option<[f64; 2]>,
) -> Result<Part> {
    let db = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let center = read(&db, SOURCE, x, y)?.ok_or_else(|| anyhow::anyhow!("元標高がありません"))?;
    let mut neighbors = HashMap::new();
    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
        if let Some(v) = read(
            &db,
            SOURCE,
            x.checked_add_signed(dx).unwrap(),
            y.checked_add_signed(dy).unwrap(),
        )? {
            neighbors.insert((dx, dy), v);
        }
    }
    let sample = |px: i32, py: i32, h: f64| {
        let dx = px.div_euclid(256);
        let dy = py.div_euclid(256);
        let i = py.rem_euclid(256) as usize * 256 + px.rem_euclid(256) as usize;
        let v = if dx == 0 && dy == 0 {
            center[i]
        } else {
            neighbors.get(&(dx, dy)).map_or(f32::NAN, |v| v[i])
        };
        if v.is_finite() { v as f64 } else { h }
    };
    let mut part = Part {
        x,
        y,
        normal: vec![[0.; 3]; NORMAL_SIDE * NORMAL_SIDE],
        counts: vec![0; NORMAL_SIDE * NORMAL_SIDE],
        detail_shade: vec![0.; NORMAL_SIDE * NORMAL_SIDE],
        shade: vec![0.; SHADE_SIDE * SHADE_SIDE],
        valid: vec![0; SHADE_SIDE * SHADE_SIDE],
    };
    // 同じ列・行で共通の三角関数は先に計算する。
    let solar_columns = solar.map(|angles| {
        (0..256)
            .map(|px| {
                let lon = ((x as f64 + (px as f64 + 0.5) / 256.) / (1u32 << SOURCE) as f64 * 360.
                    - 180.)
                    .to_radians();
                let h = angles[1] + lon;
                [angles[0].cos() * h.sin(), angles[0].cos() * h.cos()]
            })
            .collect::<Vec<_>>()
    });
    let decl_sin = solar.map_or(0., |a| a[0].sin());
    for py in 0..256 {
        let latitude = (std::f64::consts::PI
            * (1. - 2. * (y as f64 + (py as f64 + 0.5) / 256.) / (1u32 << SOURCE) as f64))
            .sinh()
            .atan();
        let meters = 2. * std::f64::consts::PI * 6378137. * latitude.cos()
            / ((1u32 << SOURCE) as f64 * 256.);
        let (lat_sin, lat_cos) = latitude.sin_cos();
        for px in 0..256 {
            let h = center[py * 256 + px] as f64;
            if !h.is_finite() {
                continue;
            }
            let dx = (sample(px as i32 + 1, py as i32, h) - sample(px as i32 - 1, py as i32, h))
                / (2. * meters);
            let north = (sample(px as i32, py as i32 - 1, h) - sample(px as i32, py as i32 + 1, h))
                / (2. * meters);
            let n = unit([-dx, -north, 1.]);
            let local_light = solar_columns.as_ref().map_or(light, |cols| {
                let c = cols[px];
                [
                    -c[0],
                    decl_sin * lat_cos - c[1] * lat_sin,
                    decl_sin * lat_sin + c[1] * lat_cos,
                ]
            });
            let i = (py / NORMAL_BLOCK) * NORMAL_SIDE + px / NORMAL_BLOCK;
            for k in 0..3 {
                part.normal[i][k] += n[k];
            }
            part.counts[i] += 1;
            part.detail_shade[i] += sun_illuminate(n, local_light, solar.is_some());
            let i = (py / SHADE_BLOCK) * SHADE_SIDE + px / SHADE_BLOCK;
            part.shade[i] += sun_illuminate(n, local_light, solar.is_some());
            part.valid[i] += 1;
        }
    }
    Ok(part)
}
fn png(path: &Path, w: usize, h: usize, channels: usize, data: &[u8]) -> Result<()> {
    let mut encoder =
        png::Encoder::new(BufWriter::new(fs::File::create(path)?), w as u32, h as u32);
    encoder.set_color(if channels == 3 {
        png::ColorType::Rgb
    } else {
        png::ColorType::Grayscale
    });
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    encoder.write_header()?.write_image_data(data)?;
    Ok(())
}
pub fn run(source: &Path, output: &Path, azimuth: f64, altitude: f64) -> Result<()> {
    run_solar(source, output, azimuth, altitude, None, None)
}
pub fn run_solar(
    source: &Path,
    output: &Path,
    azimuth: f64,
    altitude: f64,
    solar: Option<[f64; 2]>,
    datetime: Option<&str>,
) -> Result<()> {
    if let Some(a) = solar {
        ensure!(
            a.iter().all(|v| v.is_finite())
                && a[0].abs() <= std::f64::consts::FRAC_PI_2
                && a[1].abs() <= 2. * std::f64::consts::PI,
            "太陽計算の係数が不正です"
        );
    }
    ensure!(
        azimuth.is_finite()
            && (0.0..=360.0).contains(&azimuth)
            && altitude.is_finite()
            && (0.0..=90.0).contains(&altitude),
        "太陽の方角または高度が範囲外です"
    );
    let light = light_direction(azimuth, altitude);
    fs::create_dir_all(output)?;
    let start = Instant::now();
    let db = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let coords: Vec<(u32, u32)> = db
        .prepare("SELECT x,y FROM tiles WHERE z=?1 ORDER BY y,x")?
        .query_map([SOURCE], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    ensure!(!coords.is_empty(), "全国の元標高がありません");
    let west = coords
        .iter()
        .map(|p| p.0 >> (SOURCE - OVERVIEW))
        .min()
        .unwrap();
    let east = coords
        .iter()
        .map(|p| p.0 >> (SOURCE - OVERVIEW))
        .max()
        .unwrap()
        + 1;
    let north = coords
        .iter()
        .map(|p| p.1 >> (SOURCE - OVERVIEW))
        .min()
        .unwrap();
    let south = coords
        .iter()
        .map(|p| p.1 >> (SOURCE - OVERVIEW))
        .max()
        .unwrap()
        + 1;
    let w = (east - west) as usize * 256;
    let h = (south - north) as usize * 256;
    let dw = w * 2;
    let dh = h * 2;
    let mut normals = vec![[0f64; 3]; dw * dh];
    let mut counts = vec![0u32; dw * dh];
    let mut detail_shades = vec![0f64; dw * dh];
    let mut shades = vec![0f64; w * h];
    let mut valid = vec![0u32; w * h];
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build()?;
    for (index, batch) in coords.chunks(64).enumerate() {
        let parts: Vec<Result<Part>> = pool.install(|| {
            batch
                .par_iter()
                .map(|&(x, y)| process(source, x, y, light, solar))
                .collect()
        });
        for part in parts {
            let p = part?;
            let x = p.x as usize * NORMAL_SIDE - west as usize * 512;
            let y = p.y as usize * NORMAL_SIDE - north as usize * 512;
            for py in 0..NORMAL_SIDE {
                for px in 0..NORMAL_SIDE {
                    let i = (y + py) * dw + x + px;
                    normals[i] = p.normal[py * NORMAL_SIDE + px];
                    counts[i] = p.counts[py * NORMAL_SIDE + px];
                    detail_shades[i] = p.detail_shade[py * NORMAL_SIDE + px];
                }
            }
            for py in 0..SHADE_SIDE {
                for px in 0..SHADE_SIDE {
                    let i = (y / 2 + py) * w + x / 2 + px;
                    shades[i] = p.shade[py * SHADE_SIDE + px];
                    valid[i] = p.valid[py * SHADE_SIDE + px];
                }
            }
        }
        write_json(
            &output.join("progress.json"),
            &json!({"state":"running","completed":((index+1)*64).min(coords.len()),"total":coords.len(),"sourceLevel":SOURCE,"seconds":start.elapsed().as_secs_f64()}),
        )?;
        println!(
            "全国陰影比較: {} / {}",
            ((index + 1) * 64).min(coords.len()),
            coords.len()
        );
    }
    // 正規化や 8 bit 化の前の合計から、全表示レベルを直接生成する。
    let mut illumination_levels = Vec::new();
    for level in 4..=7 {
        let factor = 1usize << (7 - level);
        let lw = dw / factor;
        let lh = dh / factor;
        let mut normal_pixels = vec![0u8; lw * lh * 3];
        let mut shade_pixels = vec![0u8; lw * lh];
        for py in 0..lh {
            for px in 0..lw {
                let mut sum = [0.; 3];
                let mut shade_sum = 0.;
                let mut samples = 0u64;
                for dy in 0..factor {
                    for dx in 0..factor {
                        let i = (py * factor + dy) * dw + px * factor + dx;
                        for k in 0..3 {
                            sum[k] += normals[i][k];
                        }
                        shade_sum += detail_shades[i];
                        samples += counts[i] as u64;
                    }
                }
                let i = py * lw + px;
                normal_pixels[i * 3..i * 3 + 3].copy_from_slice(&pack(sum));
                if samples > 0 {
                    shade_pixels[i] = (shade_sum / samples as f64 * 255.).round() as u8;
                }
            }
        }
        let normal_name = format!("normal-L{level}.png");
        let shade_name = format!("shade-L{level}.png");
        png(&output.join(&normal_name), lw, lh, 3, &normal_pixels)?;
        png(&output.join(&shade_name), lw, lh, 1, &shade_pixels)?;
        illumination_levels.push(json!({"level":level,"width":lw,"height":lh,"normalFile":normal_name,"shadeFile":shade_name,"normalBytes":fs::metadata(output.join(&normal_name))?.len(),"shadeBytes":fs::metadata(output.join(&shade_name))?.len()}));
    }
    write_json(
        &output.join("illumination-levels.json"),
        &json!({"levels":illumination_levels,"lightAzimuth":azimuth,"lightAltitude":altitude,"solarDateTime":datetime,"solarAngles":solar,"sourceLevel":SOURCE}),
    )?;
    let mut detail = vec![0u8; dw * dh * 3];
    for i in 0..normals.len() {
        detail[i * 3..i * 3 + 3].copy_from_slice(&pack(normals[i]));
    }
    png(&output.join("normal-detail.png"), dw, dh, 3, &detail)?;
    drop(detail);
    let mut low = vec![0u8; w * h * 3];
    let mut shade = vec![0u8; w * h];
    let mut mask = vec![0u8; w * h];
    let mut difference = 0.;
    let mut count = 0u64;
    for py in 0..h {
        for px in 0..w {
            let i = py * w + px;
            let mut sum = [0.; 3];
            for dy in 0..2 {
                for dx in 0..2 {
                    let n = normals[(py * 2 + dy) * dw + px * 2 + dx];
                    for k in 0..3 {
                        sum[k] += n[k];
                    }
                }
            }
            low[i * 3..i * 3 + 3].copy_from_slice(&pack(sum));
            if valid[i] > 0 {
                let baked = shades[i] / valid[i] as f64;
                shade[i] = (baked * 255.).round() as u8;
                mask[i] = ((valid[i] as f64 / (SHADE_BLOCK * SHADE_BLOCK) as f64) * 255.)
                    .round()
                    .max(1.) as u8;
                let local_light = solar.map_or(light, |angles| {
                    let latitude = (std::f64::consts::PI
                        * (1. - 2. * (north as f64 + (py as f64 + 0.5) / 256.) / 64.))
                        .sinh()
                        .atan();
                    let longitude =
                        ((west as f64 + (px as f64 + 0.5) / 256.) / 64. * 360. - 180.).to_radians();
                    solar_direction(angles, latitude, longitude)
                });
                difference +=
                    (sun_illuminate(unit(sum), local_light, solar.is_some()) - baked).abs();
                count += 1;
            }
        }
    }
    png(&output.join("normal-equal.png"), w, h, 3, &low)?;
    png(&output.join("shade.png"), w, h, 1, &shade)?;
    png(&output.join("coverage.png"), w, h, 1, &mask)?;
    let gw = w / 4;
    let gh = h / 4;
    let mut tiles = HashMap::new();
    let mut heights = Vec::with_capacity(gw * gh * 4);
    for py in 0..gh {
        for px in 0..gw {
            let gx = west as usize * 64 + px;
            let gy = north as usize * 64 + py;
            let key = ((gx / 256) as u32, (gy / 256) as u32);
            if !tiles.contains_key(&key) {
                tiles.insert(key, read(&db, 4, key.0, key.1)?);
            }
            let v = tiles[&key]
                .as_ref()
                .map_or(0., |t| t[(gy % 256) * 256 + gx % 256]);
            heights.extend(if v.is_finite() { v } else { 0. }.to_le_bytes());
        }
    }
    fs::write(output.join("height.f32"), heights)?;
    let files: serde_json::Map<String, serde_json::Value> = [
        "normal-detail.png",
        "normal-equal.png",
        "shade.png",
        "coverage.png",
        "height.f32",
    ]
    .into_iter()
    .map(|name| {
        Ok((
            name.to_string(),
            json!(fs::metadata(output.join(name))?.len()),
        ))
    })
    .collect::<Result<_>>()?;
    write_json(
        &output.join("manifest.json"),
        &json!({"landBounds":[(coords.iter().map(|p|p.0).min().unwrap() as f64/(1u32<<(SOURCE-OVERVIEW)) as f64-west as f64)/(east-west) as f64,(coords.iter().map(|p|p.1).min().unwrap() as f64/(1u32<<(SOURCE-OVERVIEW)) as f64-north as f64)/(south-north) as f64,((coords.iter().map(|p|p.0).max().unwrap()+1) as f64/(1u32<<(SOURCE-OVERVIEW)) as f64-west as f64)/(east-west) as f64,((coords.iter().map(|p|p.1).max().unwrap()+1) as f64/(1u32<<(SOURCE-OVERVIEW)) as f64-north as f64)/(south-north) as f64],"sourceLevel":SOURCE,"sourceTiles":coords.len(),"source":"国土地理院 DEM10B から作成済みの最詳細 L14 標高（5 m 単位）全タイル。","geometryLevel":4,"overviewLevel":OVERVIEW,"normalLevel":DETAIL,"west":west,"north":north,"width":w,"height":h,"detailWidth":dw,"detailHeight":dh,"geometryWidth":gw,"geometryHeight":gh,"worldWidth":(east-west) as f64*2.*std::f64::consts::PI*6378137./64.,"worldHeight":(south-north) as f64*2.*std::f64::consts::PI*6378137./64.,"lightAzimuth":azimuth,"lightAltitude":altitude,"solarDateTime":datetime,"solarAngles":solar,"files":files,"meanDiffuseDifference":difference/count.max(1) as f64,"seconds":start.elapsed().as_secs_f64(),"normalMethod":"細かな法線を面積平均して正規化。詳細版は L7、同解像度版は L6 相当。","shadeMethod":"同じ細かな法線に固定光源を適用し、その diffuse を面積平均して L6 相当へ縮小。","coverage":"同じ L14 の有効標高の面積率。両モードで共用。"}),
    )?;
    write_json(
        &output.join("progress.json"),
        &json!({"state":"complete","completed":coords.len(),"total":coords.len(),"seconds":start.elapsed().as_secs_f64()}),
    )?;
    Ok(())
}
/// 全国比較と同じ地理範囲の L4〜L7 を、元 DB の各レベルから取り出す。
pub fn geometry(source: &Path, output: &Path) -> Result<()> {
    let meta: serde_json::Value = serde_json::from_slice(&fs::read(output.join("manifest.json"))?)?;
    let west = meta["west"].as_u64().unwrap() as usize;
    let north = meta["north"].as_u64().unwrap() as usize;
    let width = meta["width"].as_u64().unwrap() as usize;
    let height = meta["height"].as_u64().unwrap() as usize;
    let db = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut levels = Vec::new();
    for level in 4..=7 {
        let (start_x, start_y, w, h) = if level <= 6 {
            let f = 1usize << (6 - level);
            (west * 256 / f, north * 256 / f, width / f, height / f)
        } else {
            (west * 512, north * 512, width * 2, height * 2)
        };
        let mut values = vec![0f32; w * h];
        for ty in start_y / 256..(start_y + h).div_ceil(256) {
            for tx in start_x / 256..(start_x + w).div_ceil(256) {
                if let Some(tile) = read(&db, level, tx as u32, ty as u32)? {
                    for sy in 0..256 {
                        let gy = ty * 256 + sy;
                        if gy < start_y || gy >= start_y + h {
                            continue;
                        }
                        for sx in 0..256 {
                            let gx = tx * 256 + sx;
                            if gx < start_x || gx >= start_x + w {
                                continue;
                            }
                            let v = tile[sy * 256 + sx];
                            values[(gy - start_y) * w + gx - start_x] =
                                if v.is_finite() { v } else { 0. };
                        }
                    }
                }
            }
        }
        let file = format!("height-L{level}.f32.gz");
        let path = output.join(&file);
        let mut writer = flate2::write::GzEncoder::new(
            BufWriter::new(fs::File::create(&path)?),
            flate2::Compression::new(6),
        );
        let raw: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        writer.write_all(&raw)?;
        writer.finish()?.flush()?;
        levels.push(json!({"level":level,"width":w,"height":h,"bytes":raw.len(),"transferBytes":fs::metadata(&path)?.len(),"url":format!("/national-geometry/height-L{level}.f32")}));
    }
    write_json(
        &output.join("geometry-levels.json"),
        &json!({"levels":levels,"source":"保存済み全国 DEM の各レベル。欠損は 0 m、土地被覆は共通の詳細マスク。","drawStep":4}),
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn diffuse(n: [f64; 3]) -> f64 {
        illuminate(n, light_direction(315., 45.))
    }
    #[test]
    fn sun_cardinal_directions() {
        let east = light_direction(90., 0.);
        assert!((east[0] - 1.).abs() < 1e-12);
        assert!(east[1].abs() < 1e-12);
        assert_eq!(illuminate([1., 0., 0.], east), 1.);
        assert_eq!(illuminate([-1., 0., 0.], east), 0.);
        let overhead = light_direction(0., 90.);
        assert!((overhead[2] - 1.).abs() < 1e-12);
    }

    #[test]
    fn shading_before_average_keeps_ridges() {
        let a = unit([1., 0., 1.]);
        let b = unit([-1., 0., 1.]);
        let average = unit([a[0] + b[0], 0., a[2] + b[2]]);
        assert!(diffuse(average) > (diffuse(a) + diffuse(b)) / 2. + 0.1);
    }
    #[test]
    fn flat_surface_matches_both_methods() {
        let n = [0., 0., 1.];
        assert!((diffuse(n) - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        assert_eq!(pack(n), [128, 128, 255]);
    }
}

#[cfg(test)]
mod solar_tests {
    use super::*;
    #[test]
    fn equatorial_noon_and_night() {
        let noon = solar_direction([0., 0.], 0., 0.);
        assert!((noon[2] - 1.).abs() < 1e-12);
        let night = solar_direction([0., std::f64::consts::PI], 0., 0.);
        assert_eq!(sun_illuminate([0., 0., -1.], night, true), 0.);
    }
    #[test]
    fn east_west_dawn() {
        let east = solar_direction([0., -std::f64::consts::FRAC_PI_2], 0., 0.1);
        let west = solar_direction([0., -std::f64::consts::FRAC_PI_2], 0., -0.1);
        assert!(east[2] > 0. && west[2] < 0.);
    }
}
