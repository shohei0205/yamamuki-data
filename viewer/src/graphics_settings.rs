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
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Haze {
    pub enabled: bool,
    pub strength: f64,
}
impl Default for Haze {
    fn default() -> Self {
        Self {
            enabled: true,
            strength: 1.,
        }
    }
}
pub fn load_haze() -> Haze {
    path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| serde_json::from_value(v.get("haze")?.clone()).ok())
        .unwrap_or_default()
}
pub fn save_haze(haze: Haze) -> anyhow::Result<()> {
    save_all(load(), haze)
}
pub fn save(vsync: bool) -> anyhow::Result<()> {
    save_all(vsync, load_haze())
}
fn save_all(vsync: bool, haze: Haze) -> anyhow::Result<()> {
    let path = path().ok_or_else(|| anyhow::anyhow!("設定の保存先がありません"))?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&serde_json::json!({"vsync": vsync, "haze": haze}))?,
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn haze_defaults_and_disabled_strength_survive_roundtrip() {
        let old: Haze = serde_json::from_str("{}").unwrap();
        assert!(old.enabled && old.strength == 1.);
        let saved = Haze {
            enabled: false,
            strength: 0.3,
        };
        let restored: Haze = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(!restored.enabled && restored.strength == 0.3);
    }
}
