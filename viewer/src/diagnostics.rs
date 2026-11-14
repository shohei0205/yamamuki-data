//! GUIアプリで標準エラーが見えない場合も、異常終了の原因を保存する。
use std::{fs::OpenOptions, io::Write, path::PathBuf, time::SystemTime};
pub fn path() -> PathBuf {
    eframe::storage_dir(crate::APP_ID)
        .unwrap_or_else(std::env::temp_dir)
        .join("logs")
        .join("viewer.log")
}
pub fn record(message: &str) {
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let _ = writeln!(
            file,
            "[{} pid={}] {message}",
            now.as_secs(),
            std::process::id()
        );
    }
}
pub fn install() {
    record(&format!("起動: {:?}", std::env::current_exe()));
    let previous = std::panic::take_hook();
    let main_thread = std::thread::current().id();
    std::panic::set_hook(Box::new(move |info| {
        record(&format!(
            "{info}\n{}",
            std::backtrace::Backtrace::force_capture()
        ));
        previous(info);
        if std::thread::current().id() == main_thread {
            let _ = rfd::MessageDialog::new()
                .set_title("データビューワのエラー")
                .set_description(format!(
                    "エラーが発生しました。詳細を以下に保存しました。\n{}\n\n{info}",
                    path().display()
                ))
                .show();
        }
    }));
}
