use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::Cell, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{RDW_INTERNALPAINT, RedrawWindow, ValidateRect},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{WM_EXITSIZEMOVE, WM_MOVING, WM_NCDESTROY, WM_PAINT, WM_SIZING},
    },
};

pub fn install(window: &impl HasWindowHandle) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    install_hwnd(handle.hwnd.get() as HWND);
}

fn install_hwnd(hwnd: HWND) {
    let moving = Box::into_raw(Box::new(Cell::new(false)));

    // ウィンドウを作成したスレッドから登録し、状態は WM_NCDESTROY まで保持する。
    unsafe {
        if SetWindowSubclass(hwnd, Some(window_proc), 0, moving as usize) == 0 {
            drop(Box::from_raw(moving));
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    let moving = data as *const Cell<bool>;

    // Win32 がウィンドウのスレッド上で呼び出す。data は登録時に確保した状態を指す。
    unsafe {
        match message {
            WM_MOVING => (*moving).set(true),
            WM_SIZING => (*moving).set(false),
            WM_PAINT if (*moving).get() => {
                // 移動中はコンポジタが直前の画面を表示する。
                ValidateRect(hwnd, ptr::null());
                return 0;
            }
            WM_EXITSIZEMOVE if (*moving).replace(false) => {
                let result = DefSubclassProc(hwnd, message, wparam, lparam);
                RedrawWindow(hwnd, ptr::null(), ptr::null_mut(), RDW_INTERNALPAINT);
                return result;
            }
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(window_proc), id);
                drop(Box::from_raw(moving.cast_mut()));
            }
            _ => {}
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::{
        Win32::{
            Foundation::RECT,
            UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, SendMessageW, WS_POPUP},
        },
        w,
    };

    #[test]
    fn moving_blocks_paint_and_releasing_or_resizing_allows_it() {
        unsafe extern "system" fn count_paint(
            hwnd: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
            _: usize,
            data: usize,
        ) -> LRESULT {
            // テスト用のカウンターはウィンドウを破棄するまで有効。
            unsafe {
                if message == WM_PAINT {
                    let count = &*(data as *const Cell<u32>);
                    count.set(count.get() + 1);
                    ValidateRect(hwnd, ptr::null());
                    return 0;
                }
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
        }

        let count = Cell::new(0_u32);
        // 同じスレッドでテスト用ウィンドウを作成し、同期メッセージで移動とサイズ変更を再現する。
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                w!("STATIC"),
                ptr::null(),
                WS_POPUP,
                0,
                0,
                320,
                240,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            assert!(!hwnd.is_null());
            let observing =
                SetWindowSubclass(hwnd, Some(count_paint), 1, &count as *const _ as usize);
            assert_ne!(observing, 0);
            install_hwnd(hwnd);

            let mut rect = RECT {
                left: 0,
                top: 0,
                right: 320,
                bottom: 240,
            };
            let bounds = &mut rect as *mut RECT as LPARAM;
            SendMessageW(hwnd, WM_MOVING, 0, bounds);
            SendMessageW(hwnd, WM_PAINT, 0, 0);
            let while_moving = count.get();
            SendMessageW(hwnd, WM_EXITSIZEMOVE, 0, 0);
            SendMessageW(hwnd, WM_PAINT, 0, 0);
            let after_move = count.get();
            SendMessageW(hwnd, WM_MOVING, 0, bounds);
            SendMessageW(hwnd, WM_SIZING, 2, bounds);
            SendMessageW(hwnd, WM_PAINT, 0, 0);
            let while_resizing = count.get();
            DestroyWindow(hwnd);

            assert_eq!(while_moving, 0);
            assert_eq!(after_move, 1);
            assert_eq!(while_resizing, 2);
        }
    }
}
