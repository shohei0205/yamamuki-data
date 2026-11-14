//! Windowsでプロセス全体のメモリーと出力先の空き容量を実測する。
use anyhow::{Result, ensure};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
static PEAK_WS: AtomicUsize = AtomicUsize::new(0);
static PEAK_PRIVATE: AtomicUsize = AtomicUsize::new(0);
static FREE_DISK: AtomicU64 = AtomicU64::new(0);
pub fn report() -> serde_json::Value {
    serde_json::json!({"peak_working_set_bytes":PEAK_WS.load(Ordering::Relaxed),"peak_private_bytes":PEAK_PRIVATE.load(Ordering::Relaxed),"last_free_disk_bytes":FREE_DISK.load(Ordering::Relaxed),"scope":"single-Rust-engine / no child processes"})
}
fn validate(ws: usize, private: usize, free: u64, min_free: u64) -> Result<()> {
    ensure!(
        ws <= 8 * 1024usize.pow(3) && private <= 10 * 1024usize.pow(3),
        "メモリー予算を超過: 常駐{ws} / private{private}"
    );
    ensure!(free >= min_free, "ディスク空き容量が不足: {free}バイト");
    Ok(())
}
#[cfg(windows)]
pub fn check(path: &Path, min_free: u64) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak_ws: usize,
        ws: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn GetDiskFreeSpaceExW(
            path: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    let mut c: Counters = unsafe { std::mem::zeroed() };
    c.cb = std::mem::size_of::<Counters>() as u32;
    ensure!(
        unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) } != 0,
        "メモリー計測に失敗"
    );
    ensure!(
        c.ws <= 8 * 1024usize.pow(3) && c.private <= 10 * 1024usize.pow(3),
        "メモリー予算を超過: 常駐{} / private{}。保存済み状態から再開できます",
        c.ws,
        c.private
    );
    let name = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let (mut available, mut total, mut free) = (0, 0, 0);
    ensure!(
        unsafe { GetDiskFreeSpaceExW(name.as_ptr(), &mut available, &mut total, &mut free) } != 0,
        "ディスク空き計測に失敗"
    );
    PEAK_WS.fetch_max(c.peak_ws, Ordering::Relaxed);
    PEAK_PRIVATE.fetch_max(c.private, Ordering::Relaxed);
    FREE_DISK.store(available, Ordering::Relaxed);
    validate(c.ws, c.private, available, min_free)
}
#[cfg(not(windows))]
pub fn check(_path: &Path, _min_free: u64) -> Result<()> {
    anyhow::bail!("この実行入口の資源監視はWindows版のみです。未計測で全国実行しません")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn over_budget_is_rejected() {
        assert!(validate(8 * 1024usize.pow(3) + 1, 0, 100, 0).is_err());
        assert!(validate(0, 10 * 1024usize.pow(3) + 1, 100, 0).is_err());
        assert!(validate(0, 0, 99, 100).is_err());
        assert!(validate(100, 200, 100, 100).is_ok());
    }
}
