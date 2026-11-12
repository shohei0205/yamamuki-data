//! 森林・水域または陰影の非可逆段階式JPEG XLをVarDCTとModularで比較する。
#[cfg(windows)]
#[path = "compare_jxl.rs"]
#[allow(dead_code)]
mod comparison;
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    comparison::experiment::sweep()
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows版libjxl DLLを使用する比較実験です");
}
