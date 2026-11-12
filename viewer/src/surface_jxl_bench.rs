//! 実データを使うWebPと独立JPEG XLの復号速度比較。
use super::*;
use std::{collections::BTreeMap, hint::black_box, time::Instant};

struct Input {
    layer: i64,
    z: u32,
    x: u32,
    y: u32,
    side: u32,
    webp: Vec<u8>,
    jxl: Vec<u8>,
}
fn rank(layer: i64, z: u32, x: u32, y: u32) -> u64 {
    let mut v = (x as u64) << 32 | y as u64;
    v ^= (z as u64) << 48 ^ (layer as u64).wrapping_mul(0x9e3779b97f4a7c15);
    v = (v ^ (v >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    v = (v ^ (v >> 27)).wrapping_mul(0x94d049bb133111eb);
    v ^ (v >> 31)
}
#[test]
#[ignore = "全国WebPと独立JPEG XLの復号速度を同じタイルで測る"]
fn compares_webp_and_independent_jxl() -> Result<()> {
    let webp_path = std::env::var_os("YAMAMUKI_WEBP_SQLITE").context("YAMAMUKI_WEBP_SQLITE")?;
    let jxl_path =
        std::env::var_os("YAMAMUKI_JXL_INDEPENDENT").context("YAMAMUKI_JXL_INDEPENDENT")?;
    let webp = open(Path::new(&webp_path))?;
    let jxl = open(Path::new(&jxl_path))?;
    let mut buckets = BTreeMap::<(i64, u32), Vec<(u32, u32, u32)>>::new();
    let mut stmt=webp.prepare("SELECT t.layer_key,t.z,t.x,t.y,l.tile_pixels FROM surface_tiles t JOIN surface_levels l USING(dataset_key,layer_key,z) WHERE t.dataset_key=1 AND t.encoding=2")?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, u32>(1)?,
            r.get::<_, u32>(2)?,
            r.get::<_, u32>(3)?,
            r.get::<_, u32>(4)?,
        ))
    })? {
        let (l, z, x, y, side) = row?;
        buckets.entry((l, z)).or_default().push((x, y, side));
    }
    let mut inputs = Vec::new();
    for ((layer, z), mut coords) in buckets {
        coords.sort_by_key(|&(x, y, _)| rank(layer, z, x, y));
        for (x, y, side) in coords.into_iter().take(32) {
            let a:Vec<u8>=webp.query_row("SELECT payload FROM surface_tiles WHERE dataset_key=1 AND layer_key=? AND z=? AND x=? AND y=?",params![layer,z,x,y],|r|r.get(0))?;
            let (b,actual_side,count):(Vec<u8>,u32,u32)=jxl.query_row("SELECT g.payload,g.pixel_size,g.source_tiles FROM surface_tiles t JOIN jxl_groups g USING(group_id) WHERE t.dataset_key=1 AND t.layer_key=? AND t.z=? AND t.x=? AND t.y=?",params![layer,z,x,y],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            ensure!(
                actual_side == side && count == 1,
                "独立画像でないか、画素数が一致しません"
            );
            // タイマー外で両実装を暖機し、すべての画像が正しい寸法で出ることも確かめる。
            black_box(db::decode_tile(2, &a, side)?);
            black_box(decode_rs(&b, side)?);
            inputs.push(Input {
                layer,
                z,
                x,
                y,
                side,
                webp: a,
                jxl: b,
            });
        }
    }
    let repeats = 5;
    let mut elapsed = BTreeMap::<(i64, u32), [[f64; 2]; 3]>::new();
    let mut repeat_totals = Vec::new();
    for repeat in 0..repeats {
        let mut totals = [[0f64; 2]; 3];
        for input in &inputs {
            for phase in 0..3 {
                // 先に測る形式を反転し、常に同じ形式が先になる偏りを避ける。
                for codec in if repeat % 2 == 0 { [0, 1] } else { [1, 0] } {
                    let start = Instant::now();
                    match (phase, codec) {
                        (0, 0) => {
                            black_box(db::decode_tile(2, &input.webp, input.side)?);
                        }
                        (0, 1) => {
                            black_box(decode_rs(&input.jxl, input.side)?);
                        }
                        (1, 0) => {
                            black_box(db::decode_tile(2, &input.webp, input.side)?.into_rgba8());
                        }
                        (1, 1) => {
                            black_box(
                                image::DynamicImage::ImageLuma8(decode_rs(&input.jxl, input.side)?)
                                    .into_rgba8(),
                            );
                        }
                        (2, 0) => {
                            black_box(
                                db::surface_tile(&webp, 1, input.layer, input.z, input.x, input.y)?
                                    .context("WebPなし")?
                                    .into_rgba8(),
                            );
                        }
                        (2, 1) => {
                            black_box(
                                Cache::default()
                                    .tile(&jxl, 1, input.layer, input.z, input.x, input.y)?
                                    .context("JPEG XLなし")?,
                            );
                        }
                        _ => unreachable!(),
                    }
                    let ms = start.elapsed().as_secs_f64() * 1000.;
                    elapsed.entry((input.layer, input.z)).or_default()[phase][codec] += ms;
                    totals[phase][codec] += ms;
                }
            }
        }
        repeat_totals.push(totals);
    }
    let rows:Vec<_>=elapsed.into_iter().map(|((layer,z),mut times)| {
        let subset:Vec<_>=inputs.iter().filter(|i|i.layer==layer&&i.z==z).collect();
        for phase in &mut times {for t in phase {*t/=repeats as f64;}}
        serde_json::json!({"layer":layer,"z":z,"tiles":subset.len(),"webp_bytes":subset.iter().map(|i|i.webp.len()).sum::<usize>(),"jxl_bytes":subset.iter().map(|i|i.jxl.len()).sum::<usize>(),"milliseconds_webp_jxl":times})
    }).collect();
    let report = serde_json::json!({"tiles":inputs.len(),"repeats":repeats,"phases":["decoder_native_output","decode_and_rgba","sqlite_to_rgba_empty_cache"],"codec_order":["WebP image-webp","JPEG XL jxl-rs 0.8.0"],"rows":rows,"repeat_total_ms":repeat_totals,"webp":webp_path.to_string_lossy(),"jxl":jxl_path.to_string_lossy()});
    if let Some(path) = std::env::var_os("YAMAMUKI_CODEC_BENCH_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
