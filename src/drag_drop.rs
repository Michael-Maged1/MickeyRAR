//! Windows OLE drag and drop support.
//! Enables dragging files or folders out of Mickey RAR directly into
//! Windows Explorer, the Desktop, or any other application.
//!
//! For items inside archives, we use a custom COM `IDropSource` so that the
//! drag gesture starts immediately with lightweight placeholders, and actual
//! 7-Zip extraction ONLY runs in `QueryContinueDrag` after the user releases
//! the mouse button over a valid drop target (`last_effect != DROPEFFECT_NONE`).

#[cfg(windows)]
pub mod windows {
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct GUID {
        pub data1: u32,
        pub data2: u16,
        pub data3: u16,
        pub data4: [u8; 8],
    }

    // 00000000-0000-0000-c000-000000000046
    const IID_IUNKNOWN: GUID = GUID {
        data1: 0x00000000,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

    // 00000121-0000-0000-c000-000000000046
    const IID_IDROPSOURCE: GUID = GUID {
        data1: 0x00000121,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

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

    const S_OK: i32 = 0;
    const S_FALSE: i32 = 1;
    const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
    const DRAGDROP_S_DROP: i32 = 0x0004_0100u32 as i32;
    const DRAGDROP_S_CANCEL: i32 = 0x0004_0101u32 as i32;
    const DRAGDROP_S_USEDEFAULTCURSORS: i32 = 0x0004_0102u32 as i32;

    const MK_LBUTTON: u32 = 0x0001;
    const MK_RBUTTON: u32 = 0x0002;

    const DROPEFFECT_NONE: u32 = 0;
    const DROPEFFECT_COPY: u32 = 1;
    const DROPEFFECT_MOVE: u32 = 2;

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

    type ReleaseFn = unsafe extern "system" fn(this: *mut c_void) -> u32;
    type BindToHandlerFn = unsafe extern "system" fn(
        this: *mut c_void,
        pbc: *mut c_void,
        rbhid: *const GUID,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> i32;

    #[repr(C)]
    struct IDropSourceVtbl {
        query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        query_continue_drag: unsafe extern "system" fn(*mut c_void, i32, u32) -> i32,
        give_feedback: unsafe extern "system" fn(*mut c_void, u32) -> i32,
    }

    #[repr(C)]
    struct CustomDropSource {
        vtbl: *const IDropSourceVtbl,
        ref_count: AtomicU32,
        last_effect: Cell<u32>,
        drop_invoked: Cell<bool>,
        on_drop: RefCell<Option<Box<dyn FnOnce() -> bool>>>,
    }

    static DROP_SOURCE_VTBL: IDropSourceVtbl = IDropSourceVtbl {
        query_interface: drop_source_query_interface,
        add_ref: drop_source_add_ref,
        release: drop_source_release,
        query_continue_drag: drop_source_query_continue_drag,
        give_feedback: drop_source_give_feedback,
    };

    unsafe extern "system" fn drop_source_query_interface(
        this: *mut c_void,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> i32 {
        if ppv.is_null() || riid.is_null() {
            return E_NOINTERFACE;
        }
        let iid = *riid;
        if iid == IID_IUNKNOWN || iid == IID_IDROPSOURCE {
            *ppv = this;
            drop_source_add_ref(this);
            S_OK
        } else {
            *ppv = std::ptr::null_mut();
            E_NOINTERFACE
        }
    }

    unsafe extern "system" fn drop_source_add_ref(this: *mut c_void) -> u32 {
        let obj = &*(this as *const CustomDropSource);
        obj.ref_count.fetch_add(1, Ordering::Relaxed) + 1
    }

    unsafe extern "system" fn drop_source_release(this: *mut c_void) -> u32 {
        let obj = &*(this as *const CustomDropSource);
        let prev = obj.ref_count.fetch_sub(1, Ordering::AcqRel);
        if prev == 1 {
            let _ = Box::from_raw(this as *mut CustomDropSource);
            0
        } else {
            prev - 1
        }
    }

    unsafe extern "system" fn drop_source_query_continue_drag(
        this: *mut c_void,
        escape_pressed: i32,
        key_state: u32,
    ) -> i32 {
        if escape_pressed != 0 || (key_state & MK_RBUTTON) != 0 {
            return DRAGDROP_S_CANCEL;
        }
        if (key_state & MK_LBUTTON) == 0 {
            let obj = &*(this as *const CustomDropSource);
            // Only extract & drop if the cursor is over a valid drop target.
            if obj.last_effect.get() == DROPEFFECT_NONE {
                return DRAGDROP_S_CANCEL;
            }
            if !obj.drop_invoked.get() {
                obj.drop_invoked.set(true);
                if let Some(callback) = obj.on_drop.borrow_mut().take() {
                    if !callback() {
                        return DRAGDROP_S_CANCEL;
                    }
                }
            }
            return DRAGDROP_S_DROP;
        }
        S_OK
    }

    unsafe extern "system" fn drop_source_give_feedback(
        this: *mut c_void,
        effect: u32,
    ) -> i32 {
        let obj = &*(this as *const CustomDropSource);
        obj.last_effect.set(effect);
        DRAGDROP_S_USEDEFAULTCURSORS
    }

    fn create_drop_source(on_drop: Option<Box<dyn FnOnce() -> bool>>) -> *mut c_void {
        let boxed = Box::new(CustomDropSource {
            vtbl: &DROP_SOURCE_VTBL,
            ref_count: AtomicU32::new(1),
            last_effect: Cell::new(DROPEFFECT_NONE),
            drop_invoked: Cell::new(false),
            on_drop: RefCell::new(on_drop),
        });
        Box::into_raw(boxed) as *mut c_void
    }

    fn to_wide_path(path: &Path) -> Vec<u16> {
        let clean = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let s = clean.to_string_lossy();
        let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s).replace('/', "\\");
        std::ffi::OsStr::new(&stripped)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Initiates a native Windows OLE Drag and Drop operation for regular disk files.
    pub fn start_drag_drop_files(paths: &[&Path]) -> Result<u32, String> {
        start_drag_drop_inner(paths, DROPEFFECT_COPY | DROPEFFECT_MOVE, None)
    }

    /// Initiates a native Windows OLE Drag and Drop operation for archive entries.
    /// `placeholder_paths` are lightweight placeholders created in a temp session dir.
    /// `on_drop` is called ONLY when the user releases the mouse over a valid drop
    /// target, right before Explorer copies the files — so no extraction happens
    /// during the drag gesture itself.
    pub fn start_drag_drop_deferred(
        placeholder_paths: &[&Path],
        on_drop: Box<dyn FnOnce() -> bool>,
    ) -> Result<u32, String> {
        start_drag_drop_inner(placeholder_paths, DROPEFFECT_COPY, Some(on_drop))
    }

    fn start_drag_drop_inner(
        paths: &[&Path],
        allowed_effects: u32,
        on_drop: Option<Box<dyn FnOnce() -> bool>>,
    ) -> Result<u32, String> {
        if paths.is_empty() {
            return Err("No files to drag".into());
        }

        unsafe {
            let ole_hr = OleInitialize(std::ptr::null_mut());
            let should_uninit = ole_hr == S_OK || ole_hr == S_FALSE;

            let cleanup_ole = |uninit: bool| {
                if uninit {
                    OleUninitialize();
                }
            };

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
                    cleanup_ole(should_uninit);
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
                    cleanup_ole(should_uninit);
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
                    cleanup_ole(should_uninit);
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
                    cleanup_ole(should_uninit);
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
                    cleanup_ole(should_uninit);
                    return Err(format!("BindToHandler for array failed: 0x{:08x}", hr2));
                }
            }

            let drop_source = if on_drop.is_some() {
                create_drop_source(on_drop)
            } else {
                std::ptr::null_mut()
            };

            let mut effect: u32 = 0;
            let _hr = SHDoDragDrop(
                0,
                data_obj,
                drop_source,
                allowed_effects,
                &mut effect,
            );

            if !drop_source.is_null() {
                drop_source_release(drop_source);
            }

            let vtbl = *(data_obj as *mut *mut *mut c_void);
            let release: ReleaseFn = std::mem::transmute(*vtbl.add(2));
            release(data_obj);

            cleanup_ole(should_uninit);
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
    pub fn start_drag_drop_deferred(
        _placeholder_paths: &[&Path],
        _on_drop: Box<dyn FnOnce() -> bool>,
    ) -> Result<u32, String> {
        Err("Drag and drop is only supported on Windows".into())
    }
}
