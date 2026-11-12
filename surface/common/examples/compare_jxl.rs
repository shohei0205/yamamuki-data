//! 配布済み地表のWebPと、独立／階層JPEG XLを比較するWindows用実験。
#[cfg(not(windows))]
fn main() {
    eprintln!("この比較実験はWindows版libjxl DLLを使用します");
}
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    experiment::run()
}
#[cfg(windows)]
pub mod experiment {
    use anyhow::{Context, Result, ensure};
    use image::{
        GrayImage,
        imageops::{self, FilterType},
    };
    use rusqlite::{Connection, params};
    use serde_json::{Value, json};
    use std::{
        alloc::{Layout, alloc, dealloc},
        ffi::{CString, c_void},
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
        time::Instant,
    };

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryExW(path: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const i8) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }
    #[repr(C)]
    struct Pixel {
        channels: u32,
        data_type: i32,
        endian: i32,
        align: usize,
    }
    #[repr(C)]
    struct Manager {
        opaque: *mut c_void,
        alloc: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void,
        free: unsafe extern "C" fn(*mut c_void, *mut c_void),
    }
    #[derive(Default)]
    struct Memory {
        live: AtomicUsize,
        peak: AtomicUsize,
    }
    // 先頭16バイトに確保長を保存し、16バイト境界の画素領域とピーク使用量を管理する。
    unsafe extern "C" fn allocate(ctx: *mut c_void, size: usize) -> *mut c_void {
        let Some(total) = size.checked_add(16) else {
            return std::ptr::null_mut();
        };
        let Ok(layout) = Layout::from_size_align(total, 16) else {
            return std::ptr::null_mut();
        };
        unsafe {
            let p = alloc(layout);
            if p.is_null() {
                return p.cast();
            }
            p.cast::<usize>().write(total);
            let m = &*ctx.cast::<Memory>();
            let live = m.live.fetch_add(total, Ordering::Relaxed) + total;
            m.peak.fetch_max(live, Ordering::Relaxed);
            p.add(16).cast()
        }
    }
    unsafe extern "C" fn release(ctx: *mut c_void, p: *mut c_void) {
        if p.is_null() {
            return;
        }
        unsafe {
            let base = p.cast::<u8>().sub(16);
            let total = base.cast::<usize>().read();
            (&*ctx.cast::<Memory>())
                .live
                .fetch_sub(total, Ordering::Relaxed);
            dealloc(base, Layout::from_size_align(total, 16).unwrap());
        }
    }
    macro_rules! api {
    ($( $field:ident : $name:literal => $ty:ty ),* $(,)?) => {
        pub struct Api { module:*mut c_void, $( $field:$ty, )* }
        impl Api {
            pub fn load(path:&Path)->Result<Self> {
                use std::os::windows::ffi::OsStrExt;
                let wide:Vec<u16>=path.as_os_str().encode_wide().chain(Some(0)).collect();
                unsafe {
                    let module=LoadLibraryExW(wide.as_ptr(),std::ptr::null_mut(),8);
                    ensure!(!module.is_null(),"DLL読込失敗: {}",std::io::Error::last_os_error());
                    $( let addr=GetProcAddress(module,CString::new($name)?.as_ptr());
                       ensure!(!addr.is_null(),"関数がありません: {}",$name);
                       let $field=std::mem::transmute::<*mut c_void,$ty>(addr); )*
                    Ok(Self {module,$($field,)*})
                }
            }
        }
        impl Drop for Api {fn drop(&mut self){unsafe{FreeLibrary(self.module);}}}
    }
}
    api! {
        create:"JxlDecoderCreate" => unsafe extern "C" fn(*const Manager)->*mut c_void,
        destroy:"JxlDecoderDestroy" => unsafe extern "C" fn(*mut c_void),
        subscribe:"JxlDecoderSubscribeEvents" => unsafe extern "C" fn(*mut c_void,i32)->i32,
        detail:"JxlDecoderSetProgressiveDetail" => unsafe extern "C" fn(*mut c_void,i32)->i32,
        close:"JxlDecoderCloseInput" => unsafe extern "C" fn(*mut c_void),
        input:"JxlDecoderSetInput" => unsafe extern "C" fn(*mut c_void,*const u8,usize)->i32,
        process:"JxlDecoderProcessInput" => unsafe extern "C" fn(*mut c_void)->i32,
        size:"JxlDecoderImageOutBufferSize" => unsafe extern "C" fn(*mut c_void,*const Pixel,*mut usize)->i32,
        output:"JxlDecoderSetImageOutBuffer" => unsafe extern "C" fn(*mut c_void,*const Pixel,*mut c_void,usize)->i32,
        ratio:"JxlDecoderGetIntendedDownsamplingRatio" => unsafe extern "C" fn(*mut c_void)->usize,
        flush:"JxlDecoderFlushImage" => unsafe extern "C" fn(*mut c_void)->i32,
        remaining:"JxlDecoderReleaseInput" => unsafe extern "C" fn(*mut c_void)->usize,
    }
    struct Decoder<'a> {
        api: &'a Api,
        ptr: *mut c_void,
    }
    impl Drop for Decoder<'_> {
        fn drop(&mut self) {
            unsafe { (self.api.destroy)(self.ptr) }
        }
    }
    pub struct Decoded {
        pub image: GrayImage,
        pub bytes: usize,
        pub ratio: usize,
        pub peak: usize,
    }
    impl Api {
        pub fn decode(&self, bytes: &[u8], side: u32, target: usize) -> Result<Decoded> {
            let mut mem = Memory::default();
            let manager = Manager {
                opaque: (&mut mem as *mut Memory).cast(),
                alloc: allocate,
                free: release,
            };
            let dec = Decoder {
                api: self,
                ptr: unsafe { (self.create)(&manager) },
            };
            ensure!(!dec.ptr.is_null(), "復号器の作成失敗");
            let fmt = Pixel {
                channels: 1,
                data_type: 2,
                endian: 0,
                align: 0,
            };
            let mut pixels = Vec::new();
            unsafe {
                ensure!(
                    (self.subscribe)(dec.ptr, if target > 1 { 0x1000 | 0x8000 } else { 0x1000 })
                        == 0,
                    "イベント登録失敗"
                );
                ensure!((self.detail)(dec.ptr, 3) == 0, "精細度設定失敗");
                ensure!(
                    (self.input)(dec.ptr, bytes.as_ptr(), bytes.len()) == 0,
                    "入力失敗"
                );
                if target == 1 {
                    (self.close)(dec.ptr);
                }
                loop {
                    match (self.process)(dec.ptr) {
                        5 => {
                            let mut size = 0;
                            ensure!((self.size)(dec.ptr, &fmt, &mut size) == 0, "サイズ取得失敗");
                            ensure!(size == side as usize * side as usize, "画像寸法不一致");
                            pixels.resize(size, 0);
                            ensure!(
                                (self.output)(dec.ptr, &fmt, pixels.as_mut_ptr().cast(), size) == 0,
                                "出力設定失敗"
                            );
                        }
                        event @ (0x1000 | 0x8000) => {
                            let ratio = if event == 0x1000 {
                                1
                            } else {
                                (self.ratio)(dec.ptr)
                            };
                            if event == 0x1000 || (target > 1 && ratio <= target) {
                                if event == 0x8000 {
                                    ensure!((self.flush)(dec.ptr) == 0, "部分復号失敗");
                                }
                                let consumed = if event == 0x1000 {
                                    bytes.len()
                                } else {
                                    bytes.len() - (self.remaining)(dec.ptr)
                                };
                                let peak = mem.peak.load(Ordering::Relaxed) + pixels.len();
                                drop(dec);
                                ensure!(
                                    mem.live.load(Ordering::Relaxed) == 0,
                                    "復号メモリの解放漏れ"
                                );
                                return Ok(Decoded {
                                    image: GrayImage::from_raw(side, side, pixels)
                                        .context("画像出力失敗")?,
                                    bytes: consumed,
                                    ratio,
                                    peak,
                                });
                            }
                        }
                        2 if target == 0 => {
                            ensure!(
                                !pixels.is_empty() && (self.flush)(dec.ptr) == 0,
                                "この入力長では途中画像を復号できません"
                            );
                            let consumed = bytes.len() - (self.remaining)(dec.ptr);
                            let peak = mem.peak.load(Ordering::Relaxed) + pixels.len();
                            drop(dec);
                            ensure!(
                                mem.live.load(Ordering::Relaxed) == 0,
                                "部分復号メモリの解放漏れ"
                            );
                            return Ok(Decoded {
                                image: GrayImage::from_raw(side, side, pixels)
                                    .context("部分画像の出力失敗")?,
                                bytes: consumed,
                                ratio: 0,
                                peak,
                            });
                        }
                        status => anyhow::bail!("JPEG XL復号の予期しない状態: {status}"),
                    }
                }
            }
        }
    }
    pub fn metrics(a: &GrayImage, b: &GrayImage) -> Value {
        assert_eq!(a.dimensions(), b.dimensions());
        let mut max = 0u32;
        let mut sq = 0f64;
        let mut abs = 0f64;
        let mut half_flips = 0usize;
        let mut outside_visible = 0usize;
        let mut coverage_delta = 0f64;
        for (a, b) in a.as_raw().iter().zip(b.as_raw()) {
            let d = (*a as i32 - *b as i32).unsigned_abs();
            max = max.max(d);
            sq += (d * d) as f64;
            abs += d as f64;
            half_flips += usize::from((*a >= 128) != (*b >= 128));
            outside_visible += usize::from(*a == 0 && *b > 17);
            coverage_delta += *b as f64 - *a as f64;
        }
        let n = a.as_raw().len() as f64;
        let rmse = (sq / n).sqrt();
        json!({"mae":abs/n,"half_threshold_flip_percent":100.0*half_flips as f64/n,"outside_zero_over_17_percent":100.0*outside_visible as f64/n,"coverage_delta_percent":100.0*coverage_delta/(255.0*n),"rmse":rmse,"max":max,"psnr_db":if rmse==0.0{None}else{Some(20.0*(255.0/rmse).log10())}})
    }
    fn tile(db: &Connection, layer: i64, z: i32, x: i32, y: i32) -> Result<(Vec<u8>, GrayImage)> {
        let bytes:Vec<u8>=db.query_row("SELECT payload FROM surface_tiles WHERE dataset_key=1 AND layer_key=? AND z=? AND x=? AND y=?",params![layer,z,x,y],|r|r.get(0))?;
        let image = image::load_from_memory(&bytes)?.to_luma8();
        ensure!(
            image.dimensions() == (384, 384),
            "384画素のタイルが必要です"
        );
        Ok((bytes, image))
    }
    fn encode(
        cjxl: &Path,
        input: &Path,
        output: &Path,
        distance: &str,
        progressive: bool,
    ) -> Result<f64> {
        ensure!(!output.exists(), "既存の圧縮結果には上書きしません");
        let mut cmd = Command::new(cjxl);
        cmd.arg(input)
            .arg(output)
            .args(["-d", distance, "-e", "7", "--num_threads=1"]);
        if progressive {
            cmd.arg("-p");
        }
        let start = Instant::now();
        let result = cmd.output()?;
        ensure!(
            result.status.success(),
            "圧縮失敗: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(start.elapsed().as_secs_f64() * 1000.0)
    }
    fn average(a: &GrayImage, side: u32) -> GrayImage {
        imageops::resize(a, side, side, FilterType::Triangle)
    }
    fn timed(api: &Api, data: &[u8], side: u32, ratio: usize) -> Result<(Decoded, f64)> {
        let result = api.decode(data, side, ratio)?;
        let start = Instant::now();
        for _ in 0..5 {
            std::hint::black_box(api.decode(data, side, ratio)?);
        }
        Ok((result, start.elapsed().as_secs_f64() * 1000.0 / 5.0))
    }
    #[test]
    fn memory_manager_releases_partial_decoder_allocations() {
        let mut mem = Memory::default();
        let ctx = (&mut mem as *mut Memory).cast();
        unsafe {
            let a = allocate(ctx, 32);
            let b = allocate(ctx, 64);
            assert!(!a.is_null() && !b.is_null());
            assert_eq!(a as usize % 16, 0);
            assert_eq!(mem.live.load(Ordering::Relaxed), 128);
            release(ctx, a);
            assert_eq!(mem.live.load(Ordering::Relaxed), 80);
            release(ctx, b);
            release(ctx, std::ptr::null_mut());
            assert_eq!(mem.live.load(Ordering::Relaxed), 0);
            assert_eq!(mem.peak.load(Ordering::Relaxed), 128);
        }
    }
    #[test]
    fn grayscale_error_uses_pixel_values() {
        let a = GrayImage::from_raw(2, 1, vec![0, 255]).unwrap();
        let b = GrayImage::from_raw(2, 1, vec![3, 251]).unwrap();
        let m = metrics(&a, &b);
        assert_eq!(m["max"], 4);
        assert_eq!(m["mae"], 3.5);
        assert!((m["rmse"].as_f64().unwrap() - (12.5f64).sqrt()).abs() < 1e-9);
        assert_eq!(metrics(&a, &a)["max"], 0);
        let edge = metrics(
            &GrayImage::from_raw(3, 1, vec![0, 127, 255]).unwrap(),
            &GrayImage::from_raw(3, 1, vec![18, 128, 255]).unwrap(),
        );
        assert!((edge["half_threshold_flip_percent"].as_f64().unwrap() - 100.0 / 3.0).abs() < 1e-9);
        assert!(
            (edge["outside_zero_over_17_percent"].as_f64().unwrap() - 100.0 / 3.0).abs() < 1e-9
        );
    }
    // 可逆比較で保存した原画を使い、レイヤーの非可逆モードを追加比較する。
    pub fn sweep() -> Result<()> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        ensure!(
            (3..=4).contains(&args.len()),
            "引数: compare_jxl結果 libjxlのbin 新しい出力先 [hillshade]"
        );
        let source = fs::canonicalize(&args[0])?;
        let bin = fs::canonicalize(&args[1])?;
        let out = PathBuf::from(&args[2]);
        fs::create_dir(&out)?;
        let api = Api::load(&bin.join("jxl.dll"))?;
        let original: Value = serde_json::from_slice(&fs::read(source.join("results.json"))?)?;
        let shade_only = args.get(3).is_some_and(|x| x == "hillshade");
        ensure!(
            args.len() == 3 || shade_only,
            "追加の対象レイヤーはhillshadeを指定してください"
        );
        let controls: Vec<_> = original["results"]
            .as_array()
            .context("比較元が不正です")?
            .iter()
            .filter(|r| {
                r["distance"] == "0"
                    && if shade_only {
                        r["layer"] == "hillshade"
                    } else {
                        r["layer"] == "forest" || r["layer"] == "water"
                    }
            })
            .collect();
        ensure!(
            controls.len() == if shade_only { 5 } else { 10 },
            "5地点分の比較元が必要です"
        );
        let mut results = Vec::new();
        for row in controls {
            let region = row["region"].as_str().unwrap();
            let layer = row["layer"].as_str().unwrap();
            let name = format!("{region}-{layer}");
            let src = source.join(&name);
            let dir = out.join(&name);
            fs::create_dir(&dir)?;
            for z in 10..=12 {
                fs::copy(
                    src.join(format!("reference-z{z}.png")),
                    dir.join(format!("reference-z{z}.png")),
                )?;
            }
            let mut current_fine_error = Value::Null;
            for z in 10..=12 {
                let n = 1 << (z - 10);
                let mut current = GrayImage::new(384 * n, 384 * n);
                for dy in 0..n {
                    for dx in 0..n {
                        let img =
                            image::open(src.join(format!("current-{z}-{dx}-{dy}.png")))?.to_luma8();
                        imageops::replace(&mut current, &img, (dx * 384) as i64, (dy * 384) as i64);
                    }
                }
                if z == 12 {
                    let reference = image::open(src.join("reference-z12.png"))?.to_luma8();
                    current_fine_error = metrics(&reference, &current);
                }
                current.save(dir.join(format!("current-z{z}.png")))?;
            }
            for mode in ["0", "1"] {
                for distance in ["0.5", "1", "2", "4", "8"] {
                    let path = dir.join(format!("m{mode}-d{distance}.jxl"));
                    let start = Instant::now();
                    let status = Command::new(bin.join("cjxl.exe"))
                        .arg(src.join("mosaic.png"))
                        .arg(&path)
                        .args([
                            "-d",
                            distance,
                            "-e",
                            "7",
                            "--num_threads=1",
                            "-p",
                            &format!("--modular={mode}"),
                        ])
                        .output()?;
                    ensure!(
                        status.status.success(),
                        "非可逆画像圧縮失敗: {}",
                        String::from_utf8_lossy(&status.stderr)
                    );
                    let encode_ms = start.elapsed().as_secs_f64() * 1000.0;
                    let bytes = fs::read(&path)?;
                    let mut stages = Vec::new();
                    for (z, ratio) in [(10, 4), (11, 2), (12, 1)] {
                        let (decoded, ms) = timed(&api, &bytes, 1536, ratio)?;
                        let side = 384 * (1 << (z - 10));
                        let reference =
                            image::open(src.join(format!("reference-z{z}.png")))?.to_luma8();
                        let display = average(&decoded.image, side);
                        display.save(dir.join(format!("m{mode}-d{distance}-z{z}.png")))?;
                        let prefix = api.decode(&bytes[..decoded.bytes], 1536, ratio)?;
                        ensure!(
                            prefix.image == decoded.image,
                            "途中入力の一致検証に失敗しました"
                        );
                        stages.push(json!({"z":z,"requested_ratio":ratio,"actual_ratio":decoded.ratio,"prefix_bytes":decoded.bytes,"decode_ms":ms,"decoder_peak_plus_output_bytes":decoded.peak,"error":metrics(&reference,&display)}));
                    }
                    let value = json!({"region":region,"layer":layer,"mode":if mode=="0"{"VarDCT"}else{"Modular"},"distance":distance,"current_bytes":row["current_bytes"],"current_bytes_and_ms_by_z":row["current_bytes_and_ms_by_z"],"current_z12_error":current_fine_error,"grouped_bytes":bytes.len(),"encode_ms":encode_ms,"encoder_log":String::from_utf8_lossy(&status.stderr),"stages":stages});
                    eprintln!("{region} {layer} m{mode} d{distance}: {}", bytes.len());
                    results.push(value);
                    fs::write(
                        out.join("results.json"),
                        serde_json::to_vec_pretty(
                            &json!({"source":source,"decoder_threads":1,"repetitions":5,"results":results}),
                        )?,
                    )?;
                }
            }
        }
        Ok(())
    }
    pub fn run() -> Result<()> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        if args.first().is_some_and(|x| x == "--probe") {
            ensure!(
                args.len() == 4,
                "--probe 入力JXL libjxlのbin 新しい出力ディレクトリ"
            );
            let bin = fs::canonicalize(&args[2])?;
            let api = Api::load(&bin.join("jxl.dll"))?;
            let data = fs::read(&args[1])?;
            let out = PathBuf::from(&args[3]);
            fs::create_dir(&out)?;
            let full = api.decode(&data, 1536, 1)?;
            let reference = average(&full.image, 384);
            let mut results = Vec::new();
            for fraction in 1..16 {
                let n = data.len() * fraction / 16;
                let row = match api.decode(&data[..n], 1536, 0) {
                    Ok(decoded) => {
                        let img = average(&decoded.image, 384);
                        img.save(out.join(format!("prefix-{fraction}-of-16.png")))?;
                        json!({"fraction":fraction,"prefix_bytes":n,"flush":true,"error_vs_full_downsample":metrics(&reference,&img)})
                    }
                    Err(error) => {
                        json!({"fraction":fraction,"prefix_bytes":n,"flush":false,"error":error.to_string()})
                    }
                };
                eprintln!("{row}");
                results.push(row);
            }
            fs::write(
                out.join("results.json"),
                serde_json::to_vec_pretty(&results)?,
            )?;
            return Ok(());
        }
        ensure!(
            (3..=4).contains(&args.len()),
            "引数: 入力SQLite libjxlのbin 出力ディレクトリ [圧縮前のpacked SQLite]"
        );
        let db = Connection::open_with_flags(&args[0], rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let bin = fs::canonicalize(&args[1])?;
        let out = PathBuf::from(&args[2]);
        fs::create_dir(&out).context("比較結果には新しい出力ディレクトリを指定してください")?;
        let raw = args
            .get(3)
            .map(|p| Connection::open_with_flags(p, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY))
            .transpose()?;
        let api = Api::load(&bin.join("jxl.dll"))?;
        let cjxl = bin.join("cjxl.exe");
        let version = Command::new(&cjxl).arg("--version").output()?;
        let layers: Vec<(i64, String)> = db
            .prepare(
                "SELECT layer_key,id FROM surface_layers WHERE dataset_key=1 ORDER BY layer_key",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        ensure!(
            layers.len() == 3
                && ["forest", "water", "hillshade"]
                    .iter()
                    .all(|id| layers.iter().any(|(_, name)| name == id)),
            "ESAの森林・水域・陰影の3レイヤーが必要です"
        );
        let mut results = Vec::new();
        // 完全な21タイルがあるz10区画を各地点の近傍から選び、選択座標を結果にも残す。
        for (name, lat, lon) in [
            ("fuji", 35.36f64, 138.73f64),
            ("tokyo", 35.68, 139.76),
            ("izu", 34.98, 138.85),
            ("yari", 36.34, 137.65),
            ("tsukuba", 36.225, 140.107),
        ] {
            let fx = (lon + 180.0) / 360.0 * 1024.0;
            let fy = (1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0 * 1024.0;
            let (x,y):(i32,i32)=db.query_row("SELECT p.x,p.y FROM surface_tiles p WHERE p.dataset_key=1 AND p.layer_key=? AND p.z=10 AND (SELECT count(*) FROM surface_tiles c WHERE c.dataset_key=p.dataset_key AND c.layer_key=p.layer_key AND c.z=11 AND c.x BETWEEN p.x*2 AND p.x*2+1 AND c.y BETWEEN p.y*2 AND p.y*2+1)=4 AND (SELECT count(*) FROM surface_tiles c WHERE c.dataset_key=p.dataset_key AND c.layer_key=p.layer_key AND c.z=12 AND c.x BETWEEN p.x*4 AND p.x*4+3 AND c.y BETWEEN p.y*4 AND p.y*4+3)=16 ORDER BY (p.x+0.5-?)*(p.x+0.5-?)+(p.y+0.5-?)*(p.y+0.5-?) LIMIT 1",params![layers[0].0,fx,fx,fy,fy],|r|Ok((r.get(0)?,r.get(1)?)))?;
            for (key, layer) in &layers {
                eprintln!("{name} {layer} z10/{x}/{y}");
                let dir = out.join(format!("{name}-{layer}"));
                fs::create_dir_all(&dir)?;
                let mut mosaic = GrayImage::new(1536, 1536);
                let mut originals = Vec::new();
                let mut baseline_errors = Vec::new();
                let mut base_bytes = 0usize;
                let mut web_ms = 0f64;
                let mut current_by_z = [[0f64; 2]; 3];
                for z in 10..=12 {
                    let n = 1 << (z - 10);
                    for dy in 0..n {
                        for dx in 0..n {
                            let (bytes, current) = tile(&db, *key, z, x * n + dx, y * n + dy)?;
                            // 陰影だけは元のPNGを使い、既存WebPの再圧縮による誤差を避ける。
                            let img = if layer == "hillshade" {
                                if let Some(raw) = &raw {
                                    let b:Vec<u8>=raw.query_row("SELECT payload FROM hillshade_tiles WHERE z=? AND x=? AND y=?",params![z+1,x*n+dx,y*n+dy],|r|r.get(0))?;
                                    image::load_from_memory(&b)?.to_luma8()
                                } else {
                                    current.clone()
                                }
                            } else {
                                current.clone()
                            };
                            ensure!(
                                img.dimensions() == (384, 384),
                                "比較元の陰影PNGの寸法が一致しません"
                            );
                            baseline_errors.push(
                                json!({"z":z,"dx":dx,"dy":dy,"error":metrics(&img,&current)}),
                            );
                            current.save(dir.join(format!("current-{z}-{dx}-{dy}.png")))?;
                            base_bytes += bytes.len();
                            let t = Instant::now();
                            for _ in 0..5 {
                                std::hint::black_box(image::load_from_memory(&bytes)?.to_luma8());
                            }
                            let ms = t.elapsed().as_secs_f64() * 1000.0 / 5.0;
                            web_ms += ms;
                            current_by_z[(z - 10) as usize][0] += bytes.len() as f64;
                            current_by_z[(z - 10) as usize][1] += ms;
                            if z == 12 {
                                imageops::replace(
                                    &mut mosaic,
                                    &img,
                                    (dx * 384) as i64,
                                    (dy * 384) as i64,
                                );
                            }
                            let path = dir.join(format!("{z}-{dx}-{dy}.png"));
                            img.save(&path)?;
                            originals.push((z, dx, dy, path, img));
                        }
                    }
                }
                mosaic.save(dir.join("mosaic.png"))?;
                for distance in if layer == "hillshade" {
                    vec!["0", "0.5", "1"]
                } else {
                    vec!["0", "0.25", "0.5", "1", "2"]
                } {
                    let mut independent_bytes = 0usize;
                    let mut independent_ms = 0f64;
                    let mut independent_by_z = [[0f64; 2]; 3];
                    let mut independent_peak = 0usize;
                    let mut enc_ms = 0f64;
                    let mut independent_error = Vec::new();
                    for (z, dx, dy, path, original) in &originals {
                        let encoded = dir.join(format!("{z}-{dx}-{dy}-d{distance}.jxl"));
                        enc_ms += encode(&cjxl, path, &encoded, distance, false)?;
                        let data = fs::read(encoded)?;
                        independent_bytes += data.len();
                        let (decoded, ms) = timed(&api, &data, 384, 1)?;
                        independent_ms += ms;
                        independent_by_z[(*z - 10) as usize][0] += data.len() as f64;
                        independent_by_z[(*z - 10) as usize][1] += ms;
                        decoded
                            .image
                            .save(dir.join(format!("independent-d{distance}-{z}-{dx}-{dy}.png")))?;
                        independent_peak = independent_peak.max(decoded.peak);
                        let error = metrics(original, &decoded.image);
                        if distance == "0" {
                            ensure!(error["max"] == 0, "可逆検証失敗");
                        }
                        independent_error.push(json!({"z":z,"dx":dx,"dy":dy,"error":error}));
                    }
                    let encoded = dir.join(format!("progressive-d{distance}.jxl"));
                    enc_ms += encode(&cjxl, &dir.join("mosaic.png"), &encoded, distance, true)?;
                    let data = fs::read(&encoded)?;
                    let plain = dir.join(format!("grouped-plain-d{distance}.jxl"));
                    enc_ms += encode(&cjxl, &dir.join("mosaic.png"), &plain, distance, false)?;
                    let plain_bytes = fs::metadata(&plain)?.len();
                    let mut stages = Vec::new();
                    for (z, ratio) in [(10, 4), (11, 2), (12, 1)] {
                        let (decoded, ms) = timed(&api, &data, 1536, ratio)?;
                        let n = 1 << (z - 10);
                        let side = 384 * n;
                        let mut reference = GrayImage::new(side, side);
                        for (zz, dx, dy, _, img) in &originals {
                            if *zz == z {
                                imageops::replace(
                                    &mut reference,
                                    img,
                                    (*dx * 384) as i64,
                                    (*dy * 384) as i64,
                                );
                            }
                        }
                        let display = average(&decoded.image, side);
                        display.save(dir.join(format!("progressive-d{distance}-z{z}.png")))?;
                        reference.save(dir.join(format!("reference-z{z}.png")))?;
                        let error = metrics(&reference, &display);
                        if z == 12 && distance == "0" {
                            ensure!(error["max"] == 0, "一括可逆検証失敗");
                        }
                        // 復号イベントで得た区切りまでの入力でも、同じ段階を再現できるかを検証する。
                        let prefix = api.decode(&data[..decoded.bytes], 1536, ratio)?;
                        ensure!(prefix.image == decoded.image, "入力区切りの検証失敗");
                        stages.push(json!({"z":z,"requested_ratio":ratio,"actual_ratio":decoded.ratio,"prefix_bytes":decoded.bytes,"decode_ms":ms,"decoder_peak_plus_output_bytes":decoded.peak,"output_buffer_bytes":1536*1536,"error_vs_current_level":error}));
                    }
                    let row = json!({"region":name,"layer":layer,"z10_x":x,"z10_y":y,"distance":distance,"current_bytes":base_bytes,"current_bytes_and_ms_by_z":current_by_z,"independent_bytes_and_ms_by_z":independent_by_z,"current_decode_ms_21_tiles":web_ms,"independent_jxl_bytes":independent_bytes,"independent_decode_ms_21_tiles":independent_ms,"independent_decoder_peak_plus_output_bytes":independent_peak,"independent_errors":independent_error,"current_errors":baseline_errors,"grouped_nonprogressive_bytes":plain_bytes,"grouped_jxl_bytes":data.len(),"encoding_ms":enc_ms,"stages":stages});
                    eprintln!(
                        "  d{distance}: WebP/PNG {base_bytes}, 独立 {independent_bytes}, 階層 {}",
                        data.len()
                    );
                    results.push(row);
                    fs::write(
                        out.join("results.json"),
                        serde_json::to_vec_pretty(
                            &json!({"input":fs::canonicalize(&args[0])?,"raw_shade":args.get(3).map(fs::canonicalize).transpose()?,"encoder":format!("{}{}",String::from_utf8_lossy(&version.stdout),String::from_utf8_lossy(&version.stderr)),"decoder_threads":1,"repetitions":5,"reference":"陰影の第4引数がある場合は圧縮前PNG、その他は配布済みWebP/PNGの復号画素。時間はファイルI/O・プロセス起動・GPU転送を含まない。","results":results}),
                        )?,
                    )?;
                }
            }
        }
        Ok(())
    }
}
