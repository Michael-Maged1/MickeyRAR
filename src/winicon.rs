//! Full-size window icons.
//!
//! Slint's `icon` window property only sets the *small* icon on Windows, so the
//! Alt+Tab switcher and some shell previews still fall back to the generic
//! application icon. This loads the icon compiled into our own executable and
//! applies it as both the small and the large icon of every visible window.

#[cfg(windows)]
pub fn install() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::Shell::ExtractIconExW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
        SendMessageW, ICON_BIG, ICON_SMALL, WM_SETICON,
    };

    struct Collector {
        pid: u32,
        windows: Vec<HWND>,
    }

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
        let collector = &mut *(lparam as *mut Collector);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == collector.pid && IsWindowVisible(hwnd) != 0 && GetWindowTextLengthW(hwnd) > 0 {
            collector.windows.push(hwnd);
        }
        1
    }

    let Ok(exe) = std::env::current_exe() else { return };
    let exe_wide: Vec<u16> = exe.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut large: *mut std::ffi::c_void = std::ptr::null_mut();
    let mut small: *mut std::ffi::c_void = std::ptr::null_mut();
    let found = unsafe { ExtractIconExW(exe_wide.as_ptr(), 0, &mut large, &mut small, 1) };
    if found == 0 || large.is_null() { return; }

    let mut collector = Collector { pid: std::process::id(), windows: Vec::new() };
    unsafe { EnumWindows(Some(collect), &mut collector as *mut Collector as LPARAM) };
    for hwnd in collector.windows {
        unsafe {
            // Deliberately not destroyed: Windows keeps using these handles for
            // the window caption, the taskbar button and Alt+Tab.
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, large as isize);
            if !small.is_null() {
                SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small as isize);
            }
        }
    }
}

#[cfg(not(windows))]
pub fn install() {}
