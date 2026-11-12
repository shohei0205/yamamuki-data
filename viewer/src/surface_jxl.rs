//! 段階式JPEG XLの比較用SQLiteを読み、グループの復号結果を共有する。
use anyhow::{Context, Result, ensure};
#[cfg(test)]
use image::imageops::FilterType;
use image::{
    GrayImage, RgbaImage,
    imageops::{self},
};
use serde_json::Value;
#[cfg(test)]
use std::io::Cursor;
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};
use yamamuki_data_sqlite::{
    self as db,
    rusqlite::{Connection, OpenFlags, OptionalExtension, params},
};
const APPLICATION_ID: i64 = 1498237528;
const CACHE_BYTES: usize = 64 * 1024 * 1024;

pub fn is_jxl(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row("PRAGMA application_id", [], |r| r.get::<_, i64>(0))? == APPLICATION_ID)
}
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("SQLiteを開けません: {}", path.display()))?;
    if !is_jxl(&conn)? {
        return db::open(path);
    }
    ensure!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? == 1,
        "未対応のJPEG XL SQLite形式です"
    );
    let config: String = conn.query_row(
        "SELECT value FROM experiment_info WHERE key='configuration'",
        [],
        |r| r.get(0),
    )?;
    let config: Value = serde_json::from_str(&config)?;
    ensure!(
        (config["format"] == "surface-jxl-progressive-experiment-v1"
            || config["format"] == "surface-jxl-independent-experiment-v1")
            && config["mode"] == "VarDCT"
            && config["distance"] == 4,
        "未対応のJPEG XL生成設定です"
    );
    Ok(conn)
}
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct Key {
    group: i64,
    downsample: u32,
    actual: u32,
    prefix: usize,
}
#[derive(Debug)]
struct TileInfo {
    key: Key,
    side: u32,
    left: u32,
    top: u32,
    pixels: u32,
    length: usize,
}
impl TileInfo {
    fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.side, 384 | 512 | 1536)
                && matches!(self.key.downsample, 1 | 2 | 4)
                && matches!(self.key.actual, 1 | 2 | 4)
                && self.key.actual <= self.key.downsample,
            "JPEG XLの解像度情報が不正です"
        );
        ensure!(
            self.key.prefix > 0
                && self.key.prefix <= self.length
                && self.length <= 32 * 1024 * 1024,
            "JPEG XLの入力範囲が不正です"
        );
        let side = self.side / self.key.downsample;
        ensure!(
            matches!(self.pixels, 384 | 512)
                && self
                    .left
                    .checked_add(self.pixels)
                    .is_some_and(|x| x <= side)
                && self.top.checked_add(self.pixels).is_some_and(|y| y <= side),
            "JPEG XLの切り出し範囲が不正です"
        );
        Ok(())
    }
}
type Loaded = std::result::Result<Arc<GrayImage>, String>;
type Slot = Arc<OnceLock<Loaded>>;
#[derive(Default)]
struct Inner {
    images: HashMap<Key, (Slot, usize)>,
    order: VecDeque<Key>,
    bytes: usize,
}
#[derive(Default)]
pub struct Cache {
    inner: Mutex<Inner>,
}
impl Cache {
    fn get(&self, key: Key, load: impl FnOnce() -> Result<GrayImage>) -> Result<Arc<GrayImage>> {
        let slot = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("JPEG XLキャッシュのロックが壊れました"))?;
            inner.order.retain(|k| *k != key);
            inner.order.push_back(key);
            inner
                .images
                .entry(key)
                .or_insert_with(|| (Arc::new(OnceLock::new()), 0))
                .0
                .clone()
        };
        // グループごとのOnceLockで重複した復号を待つ。異なるグループは並列に復号する。
        let loaded = slot
            .get_or_init(|| load().map(Arc::new).map_err(|e| format!("{e:#}")))
            .clone()
            .map_err(anyhow::Error::msg)?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("JPEG XLキャッシュのロックが壊れました"))?;
        let charge = if let Some((_, bytes)) = inner.images.get_mut(&key) {
            if *bytes == 0 {
                *bytes = loaded.as_raw().len();
                *bytes
            } else {
                0
            }
        } else {
            0
        };
        inner.bytes += charge;
        while inner.bytes > CACHE_BYTES || inner.images.len() > 256 {
            let Some(old) = inner.order.pop_front() else {
                break;
            };
            if old == key {
                inner.order.push_back(old);
                break;
            }
            if let Some((_, bytes)) = inner.images.remove(&old) {
                inner.bytes = inner.bytes.saturating_sub(bytes);
            }
        }
        Ok(loaded)
    }
    pub fn tile(
        &self,
        conn: &Connection,
        dataset: i64,
        layer: i64,
        z: u32,
        x: u32,
        y: u32,
    ) -> Result<Option<Arc<RgbaImage>>> {
        let info=conn.query_row("SELECT t.group_id,t.downsample,t.actual_ratio,t.prefix_bytes,g.pixel_size,t.left_px,t.top_px,t.tile_pixels,length(g.payload) FROM surface_tiles t JOIN jxl_groups g USING(group_id) WHERE t.dataset_key=? AND t.layer_key=? AND t.z=? AND t.x=? AND t.y=? AND g.dataset_key=t.dataset_key AND g.layer_key=t.layer_key",params![dataset,layer,z,x,y],|r|Ok(TileInfo{key:Key{group:r.get(0)?,downsample:r.get(1)?,actual:r.get(2)?,prefix:r.get(3)?},side:r.get(4)?,left:r.get(5)?,top:r.get(6)?,pixels:r.get(7)?,length:r.get(8)?})).optional()?;
        let Some(info) = info else { return Ok(None) };
        info.validate()?;
        let image = self.get(info.key, || {
            let payload: Vec<u8> = conn.query_row(
                "SELECT substr(payload,1,?) FROM jxl_groups WHERE group_id=?",
                params![info.key.prefix, info.key.group],
                |r| r.get(0),
            )?;
            ensure!(
                payload.len() == info.key.prefix,
                "JPEG XLの画像が途中で切れています"
            );
            decode_fast(
                &payload,
                info.side,
                info.key.downsample,
                info.key.prefix == info.length,
            )
        })?;
        let crop = imageops::crop_imm(
            image.as_ref(),
            info.left,
            info.top,
            info.pixels,
            info.pixels,
        )
        .to_image();
        Ok(Some(Arc::new(
            image::DynamicImage::ImageLuma8(crop).into_rgba8(),
        )))
    }
}
#[cfg(test)]
fn decode(bytes: &[u8], side: u32) -> Result<GrayImage> {
    let mut image = jxl_oxide::JxlImage::builder()
        .alloc_tracker(jxl_oxide::AllocTracker::with_limit(128 * 1024 * 1024))
        .read(Cursor::new(bytes))
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("JPEG XLのヘッダーを読み込めません")?;
    ensure!(
        image.width() == side && image.height() == side,
        "JPEG XLの寸法がSQLiteと一致しません"
    );
    let render = if image.num_loaded_keyframes() > 0 {
        image.render_frame(0)
    } else {
        image.render_loading_frame()
    }
    .map_err(|e| anyhow::anyhow!("{e}"))
    .context("JPEG XLの画像を復号できません")?;
    let mut stream = render.stream_no_alpha();
    ensure!(
        stream.width() == side && stream.height() == side && stream.channels() == 1,
        "JPEG XLは指定寸法のグレースケール画像が必要です"
    );
    let mut pixels = vec![0u8; side as usize * side as usize];
    let written = stream.write_to_buffer(&mut pixels);
    ensure!(written == pixels.len(), "JPEG XLの復号結果が不足しています");
    GrayImage::from_raw(side, side, pixels).context("JPEG XLの画像を構成できません")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_references() {
        let mut i = TileInfo {
            key: Key {
                group: 1,
                downsample: 2,
                actual: 2,
                prefix: 20,
            },
            side: 1536,
            left: 384,
            top: 384,
            pixels: 384,
            length: 30,
        };
        assert!(i.validate().is_ok());
        i.left = 385;
        assert!(i.validate().is_err());
        i.left = 384;
        i.key.prefix = 31;
        assert!(i.validate().is_err());
        i.key.prefix = 20;
        i.key.actual = 8;
        assert!(i.validate().is_err());
    }
    #[test]
    fn shares_group_results_and_keeps_stages_separate() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = Arc::new(Cache::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let key = Key {
            group: 1,
            downsample: 2,
            actual: 2,
            prefix: 10,
        };
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let c = cache.clone();
                let n = calls.clone();
                scope.spawn(move || {
                    c.get(key, || {
                        n.fetch_add(1, Ordering::SeqCst);
                        Ok(GrayImage::new(768, 768))
                    })
                    .unwrap();
                });
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache
            .get(
                Key {
                    downsample: 1,
                    ..key
                },
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(GrayImage::new(1536, 1536))
                },
            )
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn cache_evicts_old_groups_within_the_memory_budget() {
        let cache = Cache::default();
        for group in 0..34 {
            cache
                .get(
                    Key {
                        group,
                        downsample: 1,
                        actual: 1,
                        prefix: 10,
                    },
                    || Ok(GrayImage::new(1536, 1536)),
                )
                .unwrap();
        }
        let inner = cache.inner.lock().unwrap();
        assert!(inner.bytes <= CACHE_BYTES);
        assert!(!inner.images.keys().any(|k| k.group == 0));
        assert!(inner.images.keys().any(|k| k.group == 33));
        assert_eq!(
            inner.bytes,
            inner
                .images
                .values()
                .map(|(_, bytes)| *bytes)
                .sum::<usize>()
        );
    }
    #[test]
    fn opens_independent_jxl_configuration() -> Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        let conn = Connection::open(file.path())?;
        conn.execute_batch("PRAGMA application_id=1498237528;PRAGMA user_version=1;CREATE TABLE experiment_info(key TEXT PRIMARY KEY,value TEXT)")?;
        conn.execute("INSERT INTO experiment_info VALUES('configuration',?)",[serde_json::json!({"format":"surface-jxl-independent-experiment-v1","mode":"VarDCT","distance":4}).to_string()])?;
        drop(conn);
        assert!(is_jxl(&open(file.path())?)?);
        Ok(())
    }
    #[test]
    fn open_rejects_other_experiment_versions() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = Connection::open(file.path()).unwrap();
        conn.execute_batch("PRAGMA application_id=1498237528;PRAGMA user_version=2;")
            .unwrap();
        drop(conn);
        assert!(open(file.path()).is_err());
    }
    #[test]
    #[ignore = "全国JPEG XLの復号・原寸展開・縮小の時間を測定する"]
    fn profiles_progressive_decode_steps() -> Result<()> {
        let path = std::env::var_os("YAMAMUKI_JXL_SQLITE").context("YAMAMUKI_JXL_SQLITE")?;
        let conn = open(Path::new(&path))?;
        let mut times = [0f64; 5];
        let mut count = 0u32;
        for (x, y) in [(906, 404), (909, 403), (906, 405), (903, 400), (910, 401)] {
            for layer in 1..=3 {
                for z in 10..=12 {
                    let n = 1u32 << (z - 10);
                    let start = std::time::Instant::now();
                    let (bytes,side,downsample,_actual):(Vec<u8>,u32,u32,u32)=conn.query_row("SELECT substr(g.payload,1,t.prefix_bytes),g.pixel_size,t.downsample,t.actual_ratio FROM surface_tiles t JOIN jxl_groups g USING(group_id) WHERE t.dataset_key=1 AND t.layer_key=? AND t.z=? AND t.x=? AND t.y=?",params![layer,z,x*n,y*n],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
                    times[0] += start.elapsed().as_secs_f64();
                    let start = std::time::Instant::now();
                    let mut image = jxl_oxide::JxlImage::builder()
                        .alloc_tracker(jxl_oxide::AllocTracker::with_limit(128 * 1024 * 1024))
                        .read(Cursor::new(&bytes))
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    times[1] += start.elapsed().as_secs_f64();
                    let start = std::time::Instant::now();
                    let render = if image.num_loaded_keyframes() > 0 {
                        image.render_frame(0)
                    } else {
                        image.render_loading_frame()
                    }
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                    times[2] += start.elapsed().as_secs_f64();
                    let start = std::time::Instant::now();
                    let mut pixels = vec![0u8; side as usize * side as usize];
                    let mut stream = render.stream_no_alpha();
                    ensure!(
                        stream.write_to_buffer(&mut pixels) == pixels.len(),
                        "出力不足"
                    );
                    let raster = GrayImage::from_raw(side, side, pixels).unwrap();
                    times[3] += start.elapsed().as_secs_f64();
                    let start = std::time::Instant::now();
                    std::hint::black_box(imageops::resize(
                        &raster,
                        side / downsample,
                        side / downsample,
                        FilterType::Triangle,
                    ));
                    times[4] += start.elapsed().as_secs_f64();
                    count += 1;
                }
            }
        }
        println!(
            "{}",
            serde_json::json!({"group_stages":count,"sqlite_ms":times[0]*1000.,"parse_ms":times[1]*1000.,"render_ms":times[2]*1000.,"full_size_output_ms":times[3]*1000.,"resize_ms":times[4]*1000.,"total_ms":times.iter().sum::<f64>()*1000.})
        );
        Ok(())
    }
    #[test]
    #[ignore = "JPEG XL全国SQLiteと公式復号画像を指定する実データ検査"]
    fn reads_national_groups() {
        let path = std::env::var_os("YAMAMUKI_JXL_SQLITE").expect("YAMAMUKI_JXL_SQLITE");
        let conn = open(Path::new(&path)).unwrap();
        let cache = Cache::default();
        let fallback:Option<(i64,i64,u32,u32,u32)>=conn.query_row("SELECT dataset_key,layer_key,z,x,y FROM surface_tiles WHERE downsample>1 AND actual_ratio=1 LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().unwrap();
        if let Some((d, l, z, x, y)) = fallback {
            assert_eq!(
                cache
                    .tile(&conn, d, l, z, x, y)
                    .unwrap()
                    .unwrap()
                    .dimensions(),
                (384, 384)
            );
        }
        let preview = std::env::var_os("YAMAMUKI_JXL_PREVIEW").expect("YAMAMUKI_JXL_PREVIEW");
        for (name, x, y) in [
            ("fuji", 906, 404),
            ("yari", 903, 400),
            ("tsukuba", 910, 401),
        ] {
            for (id, layer) in [(1, "forest"), (2, "water"), (3, "hillshade")] {
                for z in 10..=12 {
                    let n = 1 << (z - 10);
                    let dx = if z == 10 { 0 } else { n - 1 };
                    let image = cache
                        .tile(&conn, 1, id, z, x * n + dx, y * n + dx)
                        .unwrap()
                        .unwrap();
                    let reference =
                        image::open(Path::new(&preview).join(format!("{name}-{layer}-z{z}.png")))
                            .unwrap()
                            .to_luma8();
                    let decoded =
                        image::DynamicImage::ImageRgba8(image.as_ref().clone()).into_luma8();
                    let rmse = (decoded
                        .as_raw()
                        .iter()
                        .zip(reference.as_raw())
                        .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
                        .sum::<f64>()
                        / decoded.as_raw().len() as f64)
                        .sqrt();
                    assert!(rmse < 1.0, "{name} {layer} z{z}: RMSE {rmse}");
                }
            }
        }
    }
}

fn decode_rs(bytes: &[u8], side: u32) -> Result<GrayImage> {
    use jxl::api::{
        Event, JxlColorType, JxlDataFormat, JxlDecoder, JxlDecoderOptions, JxlOutputBuffer,
        JxlPixelFormat,
    };
    let mut options = JxlDecoderOptions::default();
    options.sample_limit = Some(16 * 1024 * 1024);
    let mut decoder = JxlDecoder::new(options);
    let mut input = bytes;
    let mut pixels = Vec::new();
    loop {
        let event = if pixels.is_empty() {
            decoder.process(&mut input, None, None)?
        } else {
            let mut buffers = [JxlOutputBuffer::new(
                &mut pixels,
                side as usize,
                side as usize,
            )];
            decoder.process(&mut input, Some(&mut buffers), None)?
        };
        match event {
            Event::BasicInfo => {
                ensure!(
                    decoder.basic_info().unwrap().size == (side as usize, side as usize),
                    "寸法不一致"
                );
                decoder.set_pixel_format(JxlPixelFormat {
                    color_type: JxlColorType::Grayscale,
                    color_data_format: Some(JxlDataFormat::U8 { bit_depth: 8 }),
                    extra_channel_format: vec![],
                })?;
                pixels.resize((side * side) as usize, 0);
            }
            Event::NeedMoreInput { .. } => {
                ensure!(
                    input.is_empty() && !pixels.is_empty(),
                    "段階データの読み込みが進みません"
                );
                let mut buffers = [JxlOutputBuffer::new(
                    &mut pixels,
                    side as usize,
                    side as usize,
                )];
                ensure!(
                    decoder.flush_pixels(&mut buffers, None)?,
                    "段階データを出力できません"
                );
                break;
            }
            Event::FrameComplete { .. } | Event::Complete => break,
            Event::FrameHeader => {}
        }
    }
    GrayImage::from_raw(side, side, pixels).context("出力不足")
}

#[cfg(test)]
mod decoder_comparison {
    use super::*;
    #[test]
    #[ignore = "全国SQLiteでデコーダーの時間と画質を比較する"]
    fn compares_decoders() -> Result<()> {
        let path = std::env::var_os("YAMAMUKI_JXL_SQLITE").context("YAMAMUKI_JXL_SQLITE")?;
        let conn = open(Path::new(&path))?;
        let mut times = [[0f64; 3]; 3];
        let mut direct_errors = [0f64; 3];
        let mut errors = [0f64; 3];
        let mut counts = [0usize; 3];
        for (x, y) in [(906, 404), (909, 403), (906, 405), (903, 400), (910, 401)] {
            for layer in 1..=3 {
                for z in 10..=12 {
                    let n = 1u32 << (z - 10);
                    let (bytes,side,ratio,actual):(Vec<u8>,u32,u32,u32)=conn.query_row("SELECT substr(g.payload,1,t.prefix_bytes),g.pixel_size,t.downsample,t.actual_ratio FROM surface_tiles t JOIN jxl_groups g USING(group_id) WHERE t.dataset_key=1 AND t.layer_key=? AND t.z=? AND t.x=? AND t.y=?",params![layer,z,x*n,y*n],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
                    let index = (z - 10) as usize;
                    let t = std::time::Instant::now();
                    let a = decode(&bytes, side)?;
                    let a = imageops::resize(&a, side / ratio, side / ratio, FilterType::Triangle);
                    times[index][0] += t.elapsed().as_secs_f64();
                    let t = std::time::Instant::now();
                    let b = decode_rs(&bytes, side)?;
                    let b = imageops::resize(&b, side / ratio, side / ratio, FilterType::Triangle);
                    times[index][1] += t.elapsed().as_secs_f64();
                    let t = std::time::Instant::now();
                    let direct = decode_fast(&bytes, side, ratio, actual == 1)?;
                    times[index][2] += t.elapsed().as_secs_f64();
                    let mse = a
                        .as_raw()
                        .iter()
                        .zip(b.as_raw())
                        .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
                        .sum::<f64>()
                        / a.as_raw().len() as f64;
                    errors[index] = errors[index].max(mse.sqrt());
                    let mse = b
                        .as_raw()
                        .iter()
                        .zip(direct.as_raw())
                        .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
                        .sum::<f64>()
                        / b.as_raw().len() as f64;
                    direct_errors[index] = direct_errors[index].max(mse.sqrt());
                    ensure!(
                        mse.sqrt() < 1.,
                        "直接縮小が従来の縮小と一致しません: {}",
                        mse.sqrt()
                    );
                    counts[index] += 1;
                }
            }
        }
        println!(
            "{}",
            serde_json::json!({"seconds_oxide_rs_direct":times,"max_rmse":errors,"direct_max_rmse":direct_errors,"count":counts})
        );
        Ok(())
    }
}

// 原寸のu8画像を作らず、行ごとにTriangle縮小して低解像度の結果だけ保持する。
#[derive(Debug)]
struct ReducedOutput {
    axis: Vec<Vec<(usize, f32)>>,
    size: usize,
    values: Mutex<ReductionState>,
}
#[derive(Debug)]
struct ReductionState {
    sum: Vec<f32>,
    row: Vec<f32>,
}
impl ReducedOutput {
    fn new(side: u32, ratio: u32) -> Self {
        let n = (side / ratio) as usize;
        let mut axis = vec![Vec::new(); side as usize];
        for target in 0..n {
            let center = (target as f32 + 0.5) * ratio as f32;
            let start = (center - ratio as f32).floor().max(0.) as usize;
            let end = (center + ratio as f32).ceil().min(side as f32) as usize;
            let weights: Vec<_> = (start..end)
                .map(|i| {
                    (
                        i,
                        (1. - ((i as f32 + 0.5 - center) / ratio as f32).abs()).max(0.),
                    )
                })
                .collect();
            let total: f32 = weights.iter().map(|(_, w)| w).sum();
            for (i, w) in weights {
                if w > 0. {
                    axis[i].push((target, w / total));
                }
            }
        }
        Self {
            axis,
            size: n,
            values: Mutex::new(ReductionState {
                sum: vec![0.; n * n],
                row: vec![0.; n],
            }),
        }
    }
    fn image(self) -> Result<GrayImage> {
        let pixels = self
            .values
            .into_inner()
            .map_err(|_| anyhow::anyhow!("縮小出力が破損しました"))?
            .sum
            .into_iter()
            .map(|v| v.round().clamp(0., 255.) as u8)
            .collect();
        GrayImage::from_raw(self.size as u32, self.size as u32, pixels)
            .context("縮小出力の寸法が不正です")
    }
}
impl jxl::image::JxlRowSink for ReducedOutput {
    fn write_row(&self, x: usize, y: usize, pixels: &[u8]) {
        let mut values = self.values.lock().unwrap();
        let ReductionState { sum, row } = &mut *values;
        row.fill(0.);
        for (i, &value) in pixels.iter().enumerate() {
            for &(ox, wx) in &self.axis[x + i] {
                row[ox] += f32::from(value) * wx;
            }
        }
        for &(oy, wy) in &self.axis[y] {
            for (dst, src) in sum[oy * self.size..(oy + 1) * self.size]
                .iter_mut()
                .zip(row.iter())
            {
                *dst += src * wy;
            }
        }
    }
}
fn decode_fast(bytes: &[u8], side: u32, ratio: u32, complete: bool) -> Result<GrayImage> {
    ensure!(
        matches!(ratio, 1 | 2 | 4) && matches!(side, 384 | 512 | 1536),
        "JPEG XLの出力寸法が不正です"
    );
    if ratio == 1 {
        return decode_rs(bytes, side);
    }
    use jxl::api::{
        Event, JxlColorType, JxlDataFormat, JxlDecoder, JxlDecoderOptions, JxlOutputBuffer,
        JxlPixelFormat,
    };
    let mut options = JxlDecoderOptions::default();
    options.sample_limit = Some(16 * 1024 * 1024);
    options.adjust_orientation = false;
    let mut decoder = JxlDecoder::new(options);
    let output = ReducedOutput::new(side, ratio);
    let mut ready = false;
    let mut input = bytes;
    loop {
        let mut buffers = [JxlOutputBuffer::new_gray_sink(
            &output,
            side as usize,
            side as usize,
        )];
        let event = decoder.process(
            &mut input,
            if complete && ready {
                Some(&mut buffers)
            } else {
                None
            },
            None,
        )?;
        match event {
            Event::BasicInfo => {
                ready = true;
                ensure!(
                    decoder.basic_info().unwrap().size == (side as usize, side as usize),
                    "JPEG XLの寸法がSQLiteと一致しません"
                );
                decoder.set_pixel_format(JxlPixelFormat {
                    color_type: JxlColorType::Grayscale,
                    color_data_format: Some(JxlDataFormat::U8 { bit_depth: 8 }),
                    extra_channel_format: vec![],
                })?;
            }
            Event::FrameHeader => {}
            Event::FrameComplete { .. } | Event::Complete => {
                // 単色画像は粗い段階の入力でもフレーム全体が完了する。
                if !complete {
                    return decode_fast(bytes, side, ratio, true);
                }
                break;
            }
            Event::NeedMoreInput { .. } => {
                ensure!(input.is_empty(), "JPEG XLの読み込みが進みません");
                ensure!(
                    !complete && decoder.flush_pixels(&mut buffers, None)?,
                    "JPEG XLの段階データを出力できません"
                );
                break;
            }
        }
    }
    output.image()
}

#[cfg(test)]
mod reduced_output_tests {
    use super::*;
    use jxl::image::JxlRowSink;
    #[test]
    fn triangle_matches_whole_image_in_shuffled_chunks() -> Result<()> {
        for ratio in [1, 2, 4] {
            let side = 32;
            let source = GrayImage::from_fn(side, side, |x, y| {
                image::Luma([((x * 17 + y * 31 + x * y) % 256) as u8])
            });
            let output = ReducedOutput::new(side, ratio);
            // グループ・行の到着順に依存せず、端でも同じ重みになることを確認する。
            for x in [16, 0] {
                for y in (0..side).rev() {
                    let start = (y * side + x) as usize;
                    output.write_row(x as usize, y as usize, &source.as_raw()[start..start + 16]);
                }
            }
            let actual = output.image()?;
            let expected =
                imageops::resize(&source, side / ratio, side / ratio, FilterType::Triangle);
            for (&a, &b) in actual.as_raw().iter().zip(expected.as_raw()) {
                assert!(a.abs_diff(b) <= 1);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod nationwide_reduced_checks {
    use super::*;
    #[test]
    #[ignore = "全国SQLiteから段階と原寸代替を分散抽出して検査する"]
    fn reduced_output_across_national_groups() -> Result<()> {
        let path = std::env::var_os("YAMAMUKI_JXL_SQLITE").context("YAMAMUKI_JXL_SQLITE")?;
        let conn = open(Path::new(&path))?;
        let mut stmt=conn.prepare("SELECT substr(g.payload,1,t.prefix_bytes),g.pixel_size,t.downsample,t.actual_ratio FROM surface_tiles t JOIN jxl_groups g USING(group_id) WHERE t.downsample>1 AND t.group_id%73=0 GROUP BY t.group_id,t.downsample ORDER BY t.group_id,t.downsample LIMIT 120")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, u32>(3)?,
            ))
        })?;
        let mut count = 0;
        let mut max_error = 0u8;
        for row in rows {
            let (bytes, side, ratio, actual) = row?;
            let expected = decode_rs(&bytes, side)?;
            let expected =
                imageops::resize(&expected, side / ratio, side / ratio, FilterType::Triangle);
            let actual = decode_fast(&bytes, side, ratio, actual == 1)?;
            assert_eq!(actual.dimensions(), expected.dimensions());
            for (&a, &b) in actual.as_raw().iter().zip(expected.as_raw()) {
                max_error = max_error.max(a.abs_diff(b));
            }
            count += 1;
        }
        ensure!(
            count >= 60 && max_error <= 1,
            "縮小結果が一致しません: {count}段階, 最大差{max_error}"
        );
        println!("分散抽出: {count}段階, 最大画素差{max_error}/255");
        Ok(())
    }
}

#[cfg(test)]
mod independent_decode_comparison {
    use super::*;
    #[test]
    #[ignore = "全国の階層版と独立版のコールドタイル読込を比較する"]
    fn compares_independent_tile_reads() -> Result<()> {
        let hierarchy = std::env::var_os("YAMAMUKI_JXL_SQLITE").context("YAMAMUKI_JXL_SQLITE")?;
        let independent =
            std::env::var_os("YAMAMUKI_JXL_INDEPENDENT").context("YAMAMUKI_JXL_INDEPENDENT")?;
        let conns = [open(Path::new(&hierarchy))?, open(Path::new(&independent))?];
        let mut seconds = [[0f64; 2]; 3];
        for _ in 0..3 {
            for (x, y) in [(906, 404), (909, 403), (906, 405), (903, 400), (910, 401)] {
                for layer in 1..=3 {
                    for z in 10..=12 {
                        let n = 1u32 << (z - 10);
                        for (i, conn) in conns.iter().enumerate() {
                            // 各タイルでキャッシュを空にし、初回の待ち時間を測る。
                            let cache = Cache::default();
                            let start = std::time::Instant::now();
                            let image = cache
                                .tile(conn, 1, layer, z, x * n, y * n)?
                                .context("比較タイルがありません")?;
                            ensure!(image.dimensions() == (384, 384), "出力寸法が違います");
                            seconds[(z - 10) as usize][i] += start.elapsed().as_secs_f64();
                            std::hint::black_box(image);
                        }
                    }
                }
            }
        }
        for row in &mut seconds {
            for value in row {
                *value /= 3.;
            }
        }
        println!(
            "{}",
            serde_json::json!({"seconds_hierarchy_independent":seconds,"tiles_per_level":15,"repetitions":3,"cache":"empty-per-tile"})
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "surface_jxl_bench.rs"]
mod webp_comparison;
