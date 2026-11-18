//! 画面提示の設定はウィンドウ作成前に読み込む。
use std::path::PathBuf;
fn path() -> Option<PathBuf> {
    eframe::storage_dir(crate::APP_ID).map(|dir| dir.join("graphics.json"))
}
pub fn load() -> bool {
    path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.get("vsync")?.as_bool())
        .unwrap_or(false)
}
pub fn save(vsync: bool) -> anyhow::Result<()> {
    let path = path().ok_or_else(|| anyhow::anyhow!("設定の保存先がありません"))?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&serde_json::json!({"vsync": vsync}))?,
    )?;
    Ok(())
}
pub fn present_mode(vsync: bool) -> eframe::wgpu::PresentMode {
    if vsync {
        eframe::wgpu::PresentMode::AutoVsync
    } else {
        eframe::wgpu::PresentMode::AutoNoVsync
    }
}
