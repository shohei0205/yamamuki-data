//! GPUプログラムの変換・コンパイル時間を個別に記録する。
pub fn timed<T>(label: &str, create: impl FnOnce() -> T) -> T {
    let started = std::time::Instant::now();
    let value = create();
    let ms = started.elapsed().as_secs_f64() * 1000.;
    eprintln!("GPUプログラム {label}: {ms:.2}ms");
    if let Some(path) = std::env::var_os("YAMAMUKI_GPU_PROFILE") {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let line = format!(
                "{}\n",
                serde_json::json!({"label":label,"ms":ms,"pid":std::process::id()})
            );
            let _ = file.write_all(line.as_bytes());
        }
    }
    value
}
