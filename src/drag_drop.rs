//! Windows OLE drag and drop support.
//! Enables dragging files or folders out of Mickey RAR directly into
//! Windows Explorer, the Desktop, or any other application.

#[cfg(windows)]
pub mod windows {
    use std::path::Path;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct GUID {
        pub data1: u32,
        pub data2: u16,
        pub data3: u16,
        pub data4: [u8; 8],
    }

    // b8c0bd9f-ed24-455c-83e6-d5390c4fe8c4
    const BHID_DATA_OBJECT: GUID = GUID {
        data1: 0xb8c0bd9f,
        data2: 0xed24,
        data3: 0x455c,
        data4: [0x83, 0xe6, 0xd5, 0x39, 0x0c, 0x4f, 0xe8, 0xc4],
    };

    // 0000010e-0000-0000-c000-000000000046
    const IID_IDATA_OBJECT: GUID = GUID {
        data1: 0x0000010e,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

    // 43826d1e-e718-42ee-bc55-a1e261c37bfe
    const IID_ISHELL_ITEM: GUID = GUID {
        data1: 0x43826d1e,
        data2: 0xe718,
        data3: 0x42ee,
        data4: [0xbc, 0x55, 0xa1, 0xe2, 0x61, 0xc3, 0x7b, 0xfe],
    };

    #[link(name = "shell32")]
    extern "system" {
        fn SHCreateItemFromParsingName(
            pszpath: *const u16,
            pbc: *mut c_void,
            riid: *const GUID,
            ppv: *mut *mut c_void,
        ) -> i32;

        fn SHParseDisplayName(
            pszname: *const u16,
            pbc: *mut c_void,
            ppidl: *mut *mut c_void,
            sfgaoin: u32,
            psfgaoout: *mut u32,
        ) -> i32;

        fn SHCreateShellItemArrayFromIDLists(
            celt: u32,
            rgpidl: *const *mut c_void,
            ppsiitemarray: *mut *mut c_void,
        ) -> i32;

        fn SHDoDragDrop(
            hwnd: isize,
            pdtobj: *mut c_void,
            pdsrc: *mut c_void,
            dweffect: u32,
            pdweffect: *mut u32,
        ) -> i32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn OleInitialize(pvreserved: *mut c_void) -> i32;
        fn OleUninitialize();
        fn CoTaskMemFree(pv: *mut c_void);
    }

    const DROPEFFECT_COPY: u32 = 1;
    const DROPEFFECT_MOVE: u32 = 2;

    type ReleaseFn = unsafe extern "system" fn(this: *mut c_void) -> u32;
    type BindToHandlerFn = unsafe extern "system" fn(
        this: *mut c_void,
        pbc: *mut c_void,
        rbhid: *const GUID,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> i32;

    fn to_wide_path(path: &Path) -> Vec<u16> {
        let clean = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let s = clean.to_string_lossy();
        let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s).replace('/', "\\");
        std::ffi::OsStr::new(&stripped)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Initiates a native Windows OLE Drag and Drop operation with the given disk file paths.
    /// This allows dragging files out of Mickey RAR into Explorer, Desktop, or other apps.
    pub fn start_drag_drop_files(paths: &[&Path]) -> Result<u32, String> {
        if paths.is_empty() {
            return Err("No files to drag".into());
        }

        unsafe {
            let _ = OleInitialize(std::ptr::null_mut());

            let mut data_obj: *mut c_void = std::ptr::null_mut();

            if paths.len() == 1 {
                let path_wide = to_wide_path(paths[0]);
                let mut shell_item: *mut c_void = std::ptr::null_mut();
                let hr = SHCreateItemFromParsingName(
                    path_wide.as_ptr(),
                    std::ptr::null_mut(),
                    &IID_ISHELL_ITEM,
                    &mut shell_item,
                );
                if hr != 0 || shell_item.is_null() {
                    OleUninitialize();
                    return Err(format!("SHCreateItemFromParsingName failed: 0x{:08x}", hr));
                }

                let vtbl = *(shell_item as *mut *mut *mut c_void);
                let bind_to_handler: BindToHandlerFn = std::mem::transmute(*vtbl.add(3));
                let release: ReleaseFn = std::mem::transmute(*vtbl.add(2));

                let hr2 = bind_to_handler(
                    shell_item,
                    std::ptr::null_mut(),
                    &BHID_DATA_OBJECT,
                    &IID_IDATA_OBJECT,
                    &mut data_obj,
                );
                release(shell_item);

                if hr2 != 0 || data_obj.is_null() {
                    OleUninitialize();
                    return Err(format!("BindToHandler failed: 0x{:08x}", hr2));
                }
            } else {
                let mut pidls: Vec<*mut c_void> = Vec::with_capacity(paths.len());
                for p in paths {
                    let path_wide = to_wide_path(p);
                    let mut pidl: *mut c_void = std::ptr::null_mut();
                    let hr = SHParseDisplayName(
                        path_wide.as_ptr(),
                        std::ptr::null_mut(),
                        &mut pidl,
                        0,
                        std::ptr::null_mut(),
                    );
                    if hr == 0 && !pidl.is_null() {
                        pidls.push(pidl);
                    }
                }

                if pidls.is_empty() {
                    OleUninitialize();
                    return Err("Failed to resolve file paths to PIDLs".into());
                }

                let mut shell_array: *mut c_void = std::ptr::null_mut();
                let hr = SHCreateShellItemArrayFromIDLists(
                    pidls.len() as u32,
                    pidls.as_ptr(),
                    &mut shell_array,
                );
                for pidl in pidls {
                    CoTaskMemFree(pidl);
                }

                if hr != 0 || shell_array.is_null() {
                    OleUninitialize();
                    return Err(format!("SHCreateShellItemArrayFromIDLists failed: 0x{:08x}", hr));
                }

                let vtbl = *(shell_array as *mut *mut *mut c_void);
                let bind_to_handler: BindToHandlerFn = std::mem::transmute(*vtbl.add(3));
                let release: ReleaseFn = std::mem::transmute(*vtbl.add(2));

                let hr2 = bind_to_handler(
                    shell_array,
                    std::ptr::null_mut(),
                    &BHID_DATA_OBJECT,
                    &IID_IDATA_OBJECT,
                    &mut data_obj,
                );
                release(shell_array);

                if hr2 != 0 || data_obj.is_null() {
                    OleUninitialize();
                    return Err(format!("BindToHandler for array failed: 0x{:08x}", hr2));
                }
            }

            let mut effect: u32 = 0;
            let _hr = SHDoDragDrop(
                0,
                data_obj,
                std::ptr::null_mut(),
                DROPEFFECT_COPY | DROPEFFECT_MOVE,
                &mut effect,
            );

            let vtbl = *(data_obj as *mut *mut *mut c_void);
            let release: ReleaseFn = std::mem::transmute(*vtbl.add(2));
            release(data_obj);

            OleUninitialize();
            Ok(effect)
        }
    }
}

#[cfg(not(windows))]
pub mod windows {
    use std::path::Path;
    pub fn start_drag_drop_files(_paths: &[&Path]) -> Result<u32, String> {
        Err("Drag and drop is only supported on Windows".into())
    }
}


