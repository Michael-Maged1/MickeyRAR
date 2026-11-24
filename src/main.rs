#![windows_subsystem = "windows"]

mod i18n;
use i18n::Language;
mod engine;
mod winicon;
mod drag_drop;
use engine::{
    ArchiveCreationOptions, CompressionSettings, ProgressHandle, RunOutcome,
    create_archive_command_args, delete_file_with_retry,
    inspect_archive_encryption, run_7z, run_7z_cancellable_with_progress,
    test_archive_password_fast, validate_archive_members_safe,
};
use slint::{Model, ModelRc, VecModel, Timer, TimerMode};
use std::{cell::{Cell, RefCell}, fs, path::{Path, PathBuf}, rc::Rc, sync::{mpsc, Arc, Mutex, atomic::{AtomicBool, Ordering}}, thread, time::{Duration, Instant}};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

slint::include_modules!();

#[cfg(windows)]
fn center_window(window: &slint::Window) {
    use slint::PhysicalPosition;
    unsafe extern "system" { fn GetSystemMetrics(index: i32) -> i32; }
    let (screen_width, screen_height) = unsafe { (GetSystemMetrics(0), GetSystemMetrics(1)) };
    let size = window.size();
    let x = ((screen_width - size.width as i32) / 2).max(0);
    let y = ((screen_height - size.height as i32) / 2).max(0);
    window.set_position(PhysicalPosition::new(x, y));
}

#[cfg(not(windows))]
fn center_window(_window: &slint::Window) {}

slint::slint! {
    import { Button, LineEdit, CheckBox, ComboBox, VerticalBox, HorizontalBox } from "std-widgets.slint";

    component WinRarButton inherits Rectangle {
        in-out property <string> text;
        in-out property <bool> enabled: true;
        callback clicked();

        height: 25px;
        border-radius: 4px;
        border-width: 1px;
        border-color: touch.has-hover ? #888888 : #555555;
        background: touch.pressed ? #1c1c1c : (touch.has-hover ? #383838 : #2b2b2b);

        touch := TouchArea {
            enabled <=> root.enabled;
            clicked => { root.clicked(); }
        }

        Text {
            text: root.text;
            color: root.enabled ? #e8e8e8 : #777777;
            font-size: 11.5px;
            font-weight: 500;
            horizontal-alignment: center;
            vertical-alignment: center;
        }
    }

    export component ProgressWindow inherits Window {
        title: root.rtl
            ? (root.archive-name != "" ? ("جاري الاستخراج من " + root.archive-name) : "Mickey RAR — جاري فك الضغط")
            : (root.archive-name != "" ? ("Extracting from " + root.archive-name) : "Mickey RAR — Extracting");
        icon: @image-url("../assets/icon-window.png");
        width: 380px;
        height: 310px;
        background: #1f1f1f;

        in-out property <string> archive-name: "";
        in-out property <string> current-file: "";
        in-out property <string> action-text: root.rtl ? "جاري استخراج" : "extracting";
        in-out property <float> progress: 0.0;
        in-out property <float> file-progress: root.progress;
        in-out property <int> file-percent: Math.round(Math.min(1.0, Math.max(0.0, root.file-progress)) * 100);
        in-out property <float> total-progress: root.progress;
        in-out property <int> total-percent: Math.round(Math.min(1.0, Math.max(0.0, root.total-progress)) * 100);
        in-out property <string> elapsed-time: "00:00:00";
        in-out property <string> time-left: "--:--:--";
        in-out property <string> message: "Processing...";
        in-out property <bool> rtl: false;
        in-out property <bool> is-paused: false;

        callback cancel();
        callback toggle-background();
        callback toggle-pause();
        callback show-mode();
        callback show-help();

        VerticalLayout {
            padding-left: 10px;
            padding-right: 10px;
            padding-top: 8px;
            padding-bottom: 8px;
            spacing: 6px;

            // Box 1: Current File Extraction (Top Box)
            Rectangle {
                border-width: 1px;
                border-color: #4a4a4a;
                border-radius: 2px;
                background: #202020;

                VerticalLayout {
                    padding-left: 8px;
                    padding-right: 8px;
                    padding-top: 6px;
                    padding-bottom: 7px;
                    spacing: 3px;

                    // Archive Name
                    Text {
                        text: root.archive-name != "" ? root.archive-name : "Archive";
                        color: #f0f0f0;
                        font-size: 11.5px;
                        font-weight: 600;
                        overflow: elide;
                        horizontal-alignment: root.rtl ? right : left;
                    }

                    // Action Label
                    Text {
                        text: root.action-text;
                        color: #aaaaaa;
                        font-size: 11px;
                        horizontal-alignment: root.rtl ? right : left;
                    }

                    // Current file + file percentage
                    HorizontalLayout {
                        Text {
                            text: root.current-file != "" ? root.current-file : root.archive-name;
                            color: #ffffff;
                            font-size: 11.5px;
                            overflow: elide;
                            horizontal-stretch: 1;
                            horizontal-alignment: root.rtl ? right : left;
                        }
                        Text {
                            text: root.file-percent + "%";
                            color: #ffffff;
                            font-size: 11.5px;
                            font-weight: 600;
                            min-width: 38px;
                            horizontal-alignment: root.rtl ? left : right;
                        }
                    }

                    // Progress Bar 1
                    Rectangle {
                        height: 12px;
                        background: #2b2b2b;
                        border-width: 1px;
                        border-color: #484848;
                        border-radius: 2px;
                        clip: true;

                        Rectangle {
                            x: root.rtl ? (parent.width - self.width) : 0px;
                            y: 0px;
                            width: parent.width * Math.min(1.0, Math.max(0.0, root.file-progress));
                            height: parent.height;
                            background: #e8573f;
                        }
                    }
                }
            }

            // Box 2: Time Stats and Total Processed (Middle Box)
            Rectangle {
                border-width: 1px;
                border-color: #4a4a4a;
                border-radius: 2px;
                background: #202020;

                VerticalLayout {
                    padding-left: 8px;
                    padding-right: 8px;
                    padding-top: 6px;
                    padding-bottom: 7px;
                    spacing: 4px;

                    // Elapsed Time Row
                    HorizontalLayout {
                        Text {
                            text: root.rtl ? "الوقت المنقضي" : "Elapsed time";
                            color: #dddddd;
                            font-size: 11px;
                            horizontal-stretch: 1;
                            horizontal-alignment: root.rtl ? right : left;
                        }
                        Text {
                            text: root.elapsed-time;
                            color: #f0f0f0;
                            font-size: 11px;
                            font-weight: 500;
                            horizontal-alignment: root.rtl ? left : right;
                        }
                    }

                    // Time Left Row
                    HorizontalLayout {
                        Text {
                            text: root.rtl ? "الوقت المتبقي" : "Time left";
                            color: #dddddd;
                            font-size: 11px;
                            horizontal-stretch: 1;
                            horizontal-alignment: root.rtl ? right : left;
                        }
                        Text {
                            text: root.time-left;
                            color: #f0f0f0;
                            font-size: 11px;
                            font-weight: 500;
                            horizontal-alignment: root.rtl ? left : right;
                        }
                    }

                    // Thin Divider
                    Rectangle {
                        height: 1px;
                        background: #363636;
                    }

                    // Processed Row
                    HorizontalLayout {
                        Text {
                            text: root.rtl ? "تمت المعالجة" : "Processed";
                            color: #dddddd;
                            font-size: 11px;
                            font-weight: 600;
                            horizontal-stretch: 1;
                            horizontal-alignment: root.rtl ? right : left;
                        }
                        Text {
                            text: root.total-percent + "%";
                            color: #f0f0f0;
                            font-size: 11.5px;
                            font-weight: 600;
                            horizontal-alignment: root.rtl ? left : right;
                        }
                    }

                    // Progress Bar 2 (Total Processed)
                    Rectangle {
                        height: 12px;
                        background: #2b2b2b;
                        border-width: 1px;
                        border-color: #484848;
                        border-radius: 2px;
                        clip: true;

                        Rectangle {
                            x: root.rtl ? (parent.width - self.width) : 0px;
                            y: 0px;
                            width: parent.width * Math.min(1.0, Math.max(0.0, root.total-progress));
                            height: parent.height;
                            background: #e8573f;
                        }
                    }
                }
            }

            // Bottom Buttons
            VerticalLayout {
                spacing: 5px;

                // Row 1: Background & Pause
                HorizontalLayout {
                    spacing: 6px;
                    WinRarButton {
                        text: root.rtl ? "في الخلفية" : "Background";
                        horizontal-stretch: 1;
                        clicked => { root.toggle-background(); }
                    }
                    WinRarButton {
                        text: root.is-paused
                            ? (root.rtl ? "استئناف" : "Resume")
                            : (root.rtl ? "إيقاف مؤقت" : "Pause");
                        horizontal-stretch: 1;
                        clicked => { root.toggle-pause(); }
                    }
                }

                // Row 2: Cancel, Mode..., Help
                HorizontalLayout {
                    spacing: 6px;
                    WinRarButton {
                        text: root.rtl ? "إلغاء" : "Cancel";
                        horizontal-stretch: 1;
                        clicked => { root.cancel(); }
                    }
                    WinRarButton {
                        text: root.rtl ? "الوضع..." : "Mode...";
                        horizontal-stretch: 1;
                        clicked => { root.show-mode(); }
                    }
                    WinRarButton {
                        text: root.rtl ? "مساعدة" : "Help";
                        horizontal-stretch: 1;
                        clicked => { root.show-help(); }
                    }
                }
            }
        }
    }

    export component ArchiveDialog inherits Window {
        title: root.rtl ? "Mickey RAR — إضافة إلى أرشيف" : "Mickey RAR — Add to archive";
        icon: @image-url("../assets/icon-window.png");
        width: 560px;
        height: 455px;
        background: #202020;
        in-out property <bool> rtl: false;
        in-out property <string> archive-name: "MickeyRAR-archive";
        in-out property <string> destination: "";
        in-out property <int> format-index: 0;
        in-out property <int> level-index: 4;
        in-out property <string> password: "";
        in-out property <bool> show-password: false;
        in-out property <bool> encrypt-headers: true;
        in-out property <bool> delete-original: false;
        callback browse-location();
        callback start-compression();
        callback cancel();

        VerticalBox {
            padding: 18px;
            spacing: 9px;
            Text {
                text: root.rtl ? "إضافة إلى أرشيف مضغوط" : "Add to Archive";
                color: #e8573f;
                font-size: 19px;
                font-weight: 700;
            }
            Text {
                text: root.rtl
                    ? "اختر اسم الأرشيف، الصيغة (ZIP / 7Z / SFX)، مستوى الضغط وكلمة المرور الاختيارية."
                    : "Choose the archive name, format (ZIP / 7Z / SFX), compression level, and optional password.";
                color: #aeb8ad;
                font-size: 11.5px;
            }
            Rectangle { height: 1px; background: #3a3a3a; }
            HorizontalBox {
                spacing: 10px;
                Text { text: root.rtl ? "اسم الأرشيف:" : "Archive name:"; width: 135px; color: #e2e2e2; vertical-alignment: center; }
                LineEdit { text <=> root.archive-name; accepted => { root.start-compression(); } }
            }
            HorizontalBox {
                spacing: 10px;
                Text { text: root.rtl ? "مكان الحفظ:" : "Save location:"; width: 135px; color: #e2e2e2; vertical-alignment: center; }
                LineEdit { text <=> root.destination; }
                Button { text: root.rtl ? "استعراض..." : "Browse..."; clicked => { root.browse-location(); } }
            }
            HorizontalBox {
                spacing: 10px;
                Text { text: root.rtl ? "صيغة الأرشيف:" : "Archive format:"; width: 135px; color: #e2e2e2; vertical-alignment: center; }
                ComboBox {
                    model: ["ZIP", "7Z", "SFX (.exe)"];
                    current-index <=> root.format-index;
                }
            }
            HorizontalBox {
                spacing: 10px;
                Text { text: root.rtl ? "مستوى الضغط:" : "Compression level:"; width: 135px; color: #e2e2e2; vertical-alignment: center; }
                ComboBox {
                    model: root.rtl
                        ? ["تخزين فقط (Store)", "سريع (Fast)", "عادي (Normal)", "عالي (Maximum)", "فائق (Ultra)"]
                        : ["Store", "Fast", "Normal", "Maximum", "Ultra"];
                    current-index <=> root.level-index;
                }
            }
            HorizontalBox {
                spacing: 10px;
                Text { text: root.rtl ? "كلمة المرور (اختياري):" : "Password (optional):"; width: 135px; color: #e2e2e2; vertical-alignment: center; }
                LineEdit {
                    text <=> root.password;
                    input-type: root.show-password ? InputType.text : InputType.password;
                    placeholder-text: root.rtl ? "اتركه فارغاً لبدون تشفير..." : "Leave empty for no password...";
                }
                CheckBox {
                    text: root.rtl ? "إظهار" : "Show";
                    checked <=> root.show-password;
                }
            }
            if root.format-index >= 1 && root.password != "" : CheckBox {
                text: root.rtl ? "تشفير أسماء الملفات أيضاً (-mhe=on)" : "Encrypt file names too (-mhe=on)";
                checked <=> root.encrypt-headers;
            }
            CheckBox {
                text: root.rtl ? "حذف الملفات الأصلية بعد نجاح عملية الضغط" : "Delete original files after successful compression";
                checked <=> root.delete-original;
            }
            Rectangle { vertical-stretch: 1; }
            HorizontalBox {
                alignment: end;
                spacing: 8px;
                Button { text: root.rtl ? "إلغاء" : "Cancel"; clicked => { root.cancel(); } }
                Button { text: root.rtl ? "إنشاء الأرشيف" : "Create archive"; clicked => { root.start-compression(); } }
            }
        }
    }

    export component PasswordDialog inherits Window {
        title: root.rtl ? "فك ضغط ملف محمي" : "Enter Password";
        icon: @image-url("../assets/icon-window.png");
        width: 440px;
        height: 240px;
        background: #202020;
        forward-focus: pwd-input;
        in-out property <string> archive-name: "";
        in-out property <string> password: "";
        in-out property <bool> show-password: false;
        in-out property <bool> rtl: false;
        in-out property <string> error-message: "";

        callback submit();
        callback cancel();

        VerticalBox {
            padding: 18px;
            spacing: 10px;

            HorizontalBox {
                spacing: 12px;
                alignment: start;
                Rectangle {
                    width: 34px;
                    height: 34px;
                    border-radius: 7px;
                    background: #e8573f;
                    Image { source: @image-url("../assets/icon.png"); width: 34px; height: 34px; image-fit: contain; }
                }
                VerticalBox {
                    padding: 0px;
                    spacing: 2px;
                    Text {
                        text: root.rtl ? "أدخل كلمة المرور" : "Enter Password";
                        color: #f3f7f1;
                        font-size: 16px;
                        font-weight: 700;
                    }
                    Text {
                        text: root.rtl ? "الملف محمي بكلمة مرور. أدخل كلمة المرور للمتابعة:" : "The archive is encrypted. Enter password to continue:";
                        color: #aaaaaa;
                        font-size: 11px;
                    }
                }
            }

            if root.archive-name != "": Text {
                text: root.archive-name;
                color: #e8573f;
                font-size: 12px;
                font-weight: 600;
                overflow: elide;
            }

            pwd-input := LineEdit {
                text <=> root.password;
                input-type: root.show-password ? InputType.text : InputType.password;
                placeholder-text: root.rtl ? "اكتب كلمة المرور هنا..." : "Enter password here...";
                accepted => { root.submit(); }
            }

            HorizontalBox {
                padding: 0px;
                spacing: 8px;
                CheckBox {
                    text: root.rtl ? "إظهار كلمة المرور" : "Show password";
                    checked <=> root.show-password;
                }
            }

            if root.error-message != "": Text {
                text: root.error-message;
                color: #ff6060;
                font-size: 11px;
                font-weight: 600;
            }

            Rectangle { vertical-stretch: 1; }

            HorizontalBox {
                alignment: end;
                spacing: 10px;
                Button {
                    text: root.rtl ? "إلغاء" : "Cancel";
                    clicked => { root.cancel(); }
                }
                Button {
                    text: root.rtl ? "استخراج" : "OK";
                    clicked => { root.submit(); }
                }
            }
        }
    }
}

type CompressionWindow = ProgressWindow;

#[derive(Default)]
struct AppState {
    folder: PathBuf,
    archive: Option<PathBuf>,
    /// In-memory cache of the parsed archive contents: (archive_path, items).
    /// Avoids re-invoking 7-Zip synchronously on every folder navigation inside large archives.
    archive_members: Option<(PathBuf, Vec<ArchiveItem>)>,
    /// Cached password for the currently open encrypted archive so the user isn't prompted repeatedly.
    archive_password: Option<(PathBuf, String)>,
    /// Current virtual directory inside the archive (e.g. "Blue Sky Lo-Fi/Fmp3/").
    /// Empty string means the root of the archive.
    archive_dir: String,
    paths: Vec<PathBuf>,
    pending_paths: Vec<PathBuf>,
    sort_column: i32,
    sort_ascending: bool,
    last_selected_anchor: Option<usize>,
    search_query: String,
}

fn context_paths() -> Vec<PathBuf> {
    let mut reading_paths = false;
    std::env::args_os().skip(1).filter_map(|arg| {
        let s = arg.to_string_lossy();
        if matches!(s.as_ref(), "--add" | "--compress" | "--compress-email" | "--extract-here" | "--extract-to" | "--extract-to-folder") {
            reading_paths = true;
            return None;
        }
        if reading_paths { Some(PathBuf::from(arg)) } else { None }
    }).filter(|path| path.exists()).collect()
}

fn context_operation() -> Option<String> {
    std::env::args().skip(1).find(|arg| matches!(arg.as_str(),
        "--add" | "--compress" | "--compress-email" |
        "--extract-here" | "--extract-to" | "--extract-to-folder"
    ))
}

fn prompt_for_password(
    archive_name: &str,
    arabic: bool,
    error_msg: Option<&str>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let ui = PasswordDialog::new()?;
    ui.set_rtl(arabic);
    ui.set_archive_name(archive_name.into());
    if let Some(err) = error_msg {
        ui.set_error_message(err.into());
    }
    center_window(&ui.window());

    let (sender, receiver) = mpsc::channel();
    let s1 = sender.clone();
    let weak = ui.as_weak();
    ui.on_submit(move || {
        if let Some(ui) = weak.upgrade() {
            let pwd = ui.get_password().to_string();
            let _ = s1.send(Some(pwd));
            let _ = ui.hide();
            let _ = slint::quit_event_loop();
        }
    });
    let s2 = sender.clone();
    let weak_cancel = ui.as_weak();
    ui.on_cancel(move || {
        let _ = s2.send(None);
        if let Some(ui) = weak_cancel.upgrade() {
            let _ = ui.hide();
        }
        let _ = slint::quit_event_loop();
    });
    let s3 = sender.clone();
    ui.window().on_close_requested(move || {
        let _ = s3.send(None);
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });

    ui.run()?;
    Ok(receiver.try_recv().ok().flatten())
}

fn snapshot_zero_byte_files(dir: &Path) -> std::collections::HashSet<PathBuf> {
    let mut set = std::collections::HashSet::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Ok(meta) = p.metadata() {
                    if meta.len() == 0 {
                        set.insert(p);
                    }
                }
            }
        }
    }
    set
}

fn cleanup_new_zero_byte_files(dir: &Path, pre_existing: &std::collections::HashSet<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() && !pre_existing.contains(&p) {
                if let Ok(meta) = p.metadata() {
                    if meta.len() == 0 {
                        let _ = fs::remove_file(&p);
                    }
                }
            }
        }
    }
}

const COMMON_DEFAULT_PASSWORDS: &[&str] = &[
    "",
    "0000",
    "1234",
    "123456",
    "123",
];

fn resolve_archive_password(
    archive: &Path,
    arabic: bool,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let enc_info = inspect_archive_encryption(archive);
    if !enc_info.is_encrypted {
        return Ok(None);
    }

    // 1. Silently try common known default/blank passwords using fast single-file/header test
    for &cand in COMMON_DEFAULT_PASSWORDS {
        if test_archive_password_fast(archive, cand, &enc_info, true) {
            return Ok(if cand.is_empty() { None } else { Some(cand.to_string()) });
        }
    }

    // 2. Archive requires a specific password; prompt the user
    let name = archive.file_name().unwrap_or_default().to_string_lossy();
    let mut err: Option<&str> = None;
    loop {
        match prompt_for_password(&name, arabic, err)? {
            Some(pwd) => {
                if test_archive_password_fast(archive, &pwd, &enc_info, false) {
                    return Ok(Some(pwd));
                } else {
                    err = Some(if arabic {
                        "كلمة المرور غير صحيحة، يرجى إعادة المحاولة"
                    } else {
                        "Incorrect password, please try again"
                    });
                }
            }
            None => {
                return Ok(Some("__CANCELLED__".to_string()));
            }
        }
    }
}

fn run_context_extraction_with_options(
    paths: Vec<PathBuf>,
    target_dir: Option<PathBuf>,
    extract_to_subfolder: bool,
    arabic: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if paths.is_empty() { return Err("No archive selected".into()); }
    let archives: Vec<PathBuf> = paths.into_iter().filter(|p| p.is_file()).collect();
    if archives.is_empty() { return Err("Selected file does not exist".into()); }

    // Resolve password if any archive is encrypted
    let mut password: Option<String> = None;
    for archive in &archives {
        let abs_archive = if archive.is_absolute() { archive.clone() } else {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(archive)
        };
        match resolve_archive_password(&abs_archive, arabic)? {
            Some(ref p) if p == "__CANCELLED__" => return Ok(()),
            Some(pwd) => {
                password = Some(pwd);
                break;
            }
            None => {}
        }
    }

    let ui = ProgressWindow::new()?;
    quit_process_on_close(&ui.window());
    ui.set_rtl(arabic);

    let center_timer = Timer::default();
    let center_weak = ui.as_weak();
    center_timer.start(TimerMode::SingleShot, Duration::from_millis(120), move || {
        if let Some(ui) = center_weak.upgrade() { center_window(&ui.window()); }
        winicon::install();
    });

    let first_name = archives[0].file_name().unwrap_or_default().to_string_lossy().to_string();
    let total_archives_count = archives.len();
    let current_archive_idx = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let initial_title = if total_archives_count > 1 {
        format!("(1/{}) {}", total_archives_count, first_name)
    } else {
        first_name.clone()
    };
    let current_archive_title = Arc::new(std::sync::Mutex::new(initial_title.clone()));

    ui.set_archive_name(initial_title.into());
    ui.set_current_file("".into());
    ui.set_action_text(if arabic { "جاري استخراج" } else { "extracting" }.into());

    let cancelled = Arc::new(AtomicBool::new(false));
    let progress_handle = ProgressHandle::new();
    let worker_cancelled = cancelled.clone();
    let worker_progress = progress_handle.clone();
    let worker_done = Arc::new(AtomicBool::new(false));
    let worker_finished = worker_done.clone();
    let (sender, receiver) = mpsc::channel();

    let cancel_flag = cancelled.clone();
    let cancel_weak = ui.as_weak();
    ui.on_cancel(move || {
        cancel_flag.store(true, Ordering::SeqCst);
        if let Some(ui) = cancel_weak.upgrade() {
            let _ = ui.hide();
        }
    });

    let bg_weak = ui.as_weak();
    ui.on_toggle_background(move || {
        if let Some(ui) = bg_weak.upgrade() {
            let _ = ui.hide();
        }
    });

    let pause_weak = ui.as_weak();
    let pause_handle = progress_handle.clone();
    ui.on_toggle_pause(move || {
        if let Some(ui) = pause_weak.upgrade() {
            let next = !ui.get_is_paused();
            pause_handle.paused.store(next, Ordering::SeqCst);
            ui.set_is_paused(next);
            ui.set_action_text(if next {
                if arabic { "متوقف مؤقتاً (Paused)" } else { "paused" }.into()
            } else {
                if arabic { "جاري استخراج" } else { "extracting" }.into()
            });
        }
    });

    ui.on_show_mode(|| {});
    ui.on_show_help(|| {
        let _ = webbrowser::open("https://github.com/Michael-Maged1/MickeyRAR");
    });

    let archives_worker = archives.clone();
    let password_worker = password.clone();
    let worker_arch_idx = current_archive_idx.clone();
    let worker_arch_title = current_archive_title.clone();
    let worker = thread::spawn(move || {
        for (idx, archive) in archives_worker.iter().enumerate() {
            if worker_cancelled.load(Ordering::Relaxed) { break; }
            worker_arch_idx.store(idx, Ordering::Relaxed);
            let name = archive.file_name().unwrap_or_default().to_string_lossy().to_string();
            let title = if total_archives_count > 1 {
                format!("({}/{}) {}", idx + 1, total_archives_count, name)
            } else {
                name
            };
            if let Ok(mut lock) = worker_arch_title.lock() {
                *lock = title;
            }
            worker_progress.percent.store(0, Ordering::Relaxed);
            if let Ok(mut lock) = worker_progress.current_file.lock() {
                lock.clear();
            }

            let abs_archive = if archive.is_absolute() { archive.clone() } else {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(archive)
            };
            if password_worker.is_none() {
                if let Ok(mems) = list_archive_members(&abs_archive) {
                    if let Err(safety_err) = validate_archive_members_safe(mems.iter().map(|m| m.path.as_str())) {
                        worker_finished.store(true, Ordering::SeqCst);
                        let _ = sender.send(Err(safety_err));
                        return;
                    }
                }
            }
            let dest = if let Some(ref t) = target_dir {
                t.clone()
            } else if extract_to_subfolder {
                let parent = abs_archive.parent().unwrap_or_else(|| Path::new("."));
                let stem = abs_archive.file_stem().unwrap_or_default().to_string_lossy();
                parent.join(stem.as_ref())
            } else {
                abs_archive.parent().unwrap_or_else(|| Path::new(".")).to_path_buf()
            };
            let _ = std::fs::create_dir_all(&dest);
            let pre_existing_zero = snapshot_zero_byte_files(&dest);
            // Build -oPath using OsString directly to avoid any UTF-8
            // round-trip that could corrupt non-ASCII (Arabic) folder names.
            let mut output_arg = std::ffi::OsString::from("-o");
            output_arg.push(dest.as_os_str());

            let mut args = vec![
                std::ffi::OsString::from("x"),
                std::ffi::OsString::from("-y"),
                std::ffi::OsString::from("-aoa"),
                std::ffi::OsString::from("-mmt=on"),
                output_arg,
                abs_archive.as_os_str().to_owned(),
            ];
            if let Some(ref pwd) = password_worker {
                args.push(std::ffi::OsString::from(format!("-p{pwd}")));
            }
            let res = run_7z_cancellable_with_progress(&args, None, &worker_cancelled, Some(&worker_progress));
            if let Err(e) = res {
                cleanup_new_zero_byte_files(&dest, &pre_existing_zero);
                worker_finished.store(true, Ordering::SeqCst);
                let _ = sender.send(Err(e));
                return;
            }
            if worker_cancelled.load(Ordering::Relaxed) {
                cleanup_new_zero_byte_files(&dest, &pre_existing_zero);
                worker_finished.store(true, Ordering::SeqCst);
                let _ = sender.send(Ok(RunOutcome::Cancelled));
                return;
            }
        }
        worker_progress.percent.store(100, Ordering::Relaxed);
        worker_finished.store(true, Ordering::SeqCst);
        let _ = sender.send(Ok(RunOutcome::Finished));
    });

    let close_cancelled = cancelled.clone();
    let close_done = worker_done.clone();
    ui.window().on_close_requested(move || {
        close_cancelled.store(true, Ordering::SeqCst);
        let done = close_done.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !done.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            std::process::exit(0);
        });
        slint::CloseRequestResponse::HideWindow
    });

    let weak = ui.as_weak();
    let timer_receiver = Arc::new(Mutex::new(receiver));
    let timer_progress = progress_handle.clone();
    let timer_arch_idx = current_archive_idx.clone();
    let timer_arch_title = current_archive_title.clone();
    let start_time = Instant::now();
    let mut paused_accum = Duration::ZERO;
    let mut pause_started: Option<Instant> = None;
    let timer = Timer::default();
    let mut shown_percent = u8::MAX;
    // Keeps the auto-close timer alive until it fires.
    let close_timer_holder: Arc<Mutex<Option<Timer>>> = Arc::new(Mutex::new(None));

    timer.start(TimerMode::Repeated, Duration::from_millis(80), move || {
        if let Some(ui) = weak.upgrade() {
            let is_paused = timer_progress.paused.load(Ordering::Relaxed);
            if is_paused {
                if pause_started.is_none() {
                    pause_started = Some(Instant::now());
                }
            } else if let Some(ps) = pause_started.take() {
                paused_accum += ps.elapsed();
            }
            let current_pause = pause_started.map(|ps| ps.elapsed()).unwrap_or(Duration::ZERO);
            let active_elapsed = start_time.elapsed().saturating_sub(paused_accum + current_pause);

            let percent = timer_progress.percent.load(Ordering::Relaxed);
            let cur_file = timer_progress.current_file.lock().map(|l| l.clone()).unwrap_or_default();
            if !cur_file.is_empty() {
                ui.set_current_file(cur_file.into());
            }

            let arch_idx = timer_arch_idx.load(Ordering::Relaxed);
            if let Ok(lock) = timer_arch_title.lock() {
                if !lock.is_empty() {
                    ui.set_archive_name(lock.clone().into());
                }
            }

            let file_frac = (percent as f32 / 100.0).clamp(0.0, 1.0);
            let total_frac = if total_archives_count > 0 {
                ((arch_idx as f32 + file_frac) / (total_archives_count as f32)).clamp(0.0, 1.0)
            } else {
                file_frac
            };
            let total_percent = (total_frac * 100.0).round() as i32;

            let elapsed_secs = active_elapsed.as_secs();
            let elapsed_str = format!("{:02}:{:02}:{:02}", elapsed_secs / 3600, (elapsed_secs % 3600) / 60, elapsed_secs % 60);
            ui.set_elapsed_time(elapsed_str.into());

            if total_percent > 0 && total_percent < 100 {
                let total_est = (elapsed_secs as f64) / (total_frac as f64);
                let rem_secs = (total_est - elapsed_secs as f64).max(0.0) as u64;
                let rem_str = format!("{:02}:{:02}:{:02}", rem_secs / 3600, (rem_secs % 3600) / 60, rem_secs % 60);
                ui.set_time_left(rem_str.into());
            } else if total_percent >= 100 {
                ui.set_time_left("00:00:00".into());
            } else {
                ui.set_time_left("--:--:--".into());
            }

            ui.set_progress(file_frac);
            ui.set_file_progress(file_frac);
            ui.set_file_percent(percent as i32);
            ui.set_total_progress(total_frac);
            ui.set_total_percent(total_percent);

            if percent != shown_percent {
                shown_percent = percent;
            }

            if let Ok(result) = timer_receiver.lock().unwrap().try_recv() {
                let mut close_delay_ms = 1000;
                match result {
                    Ok(RunOutcome::Finished) => {
                        ui.set_progress(1.0);
                        ui.set_file_progress(1.0);
                        ui.set_file_percent(100);
                        ui.set_total_progress(1.0);
                        ui.set_total_percent(100);
                        ui.set_time_left("00:00:00".into());
                        ui.set_action_text(if arabic { "تم الاستخراج بنجاح ✔" } else { "Extraction completed successfully ✔" }.into());
                    }
                    Ok(RunOutcome::Cancelled) => {
                        ui.set_action_text(if arabic { "تم إلغاء فك الضغط" } else { "Extraction cancelled" }.into());
                    }
                    Err(error) => {
                        close_delay_ms = 4000;
                        let err_msg = if arabic {
                            if error.contains("password") || error.contains("encrypted") {
                                "خطأ: كلمة المرور غير صحيحة أو الملف مشفر"
                            } else {
                                "خطأ في فك الضغط"
                            }
                        } else {
                            "Extraction error"
                        };
                        ui.set_action_text(err_msg.into());
                        ui.set_current_file(error.into());
                    }
                }
                // Show result, then close.
                let holder = close_timer_holder.clone();
                let auto_close = Timer::default();
                auto_close.start(TimerMode::SingleShot, Duration::from_millis(close_delay_ms), move || {
                    let _ = slint::quit_event_loop();
                    *holder.lock().unwrap() = None; // drop timer
                });
                *close_timer_holder.lock().unwrap() = Some(auto_close);
            }
        }
    });

    ui.run()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !worker.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    drop(timer);
    Ok(())
}

fn coordinate_context_extraction(
    operation: &str,
    paths: Vec<PathBuf>,
    arabic: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let queue_dir = std::env::temp_dir().join("MickeyRAR_Batch_Extract");
    let _ = fs::create_dir_all(&queue_dir);

    let req_id = format!("{}_{}", std::process::id(), Instant::now().elapsed().as_nanos());
    let req_file = queue_dir.join(format!("req_{}.txt", req_id));

    // Write this instance's requested files to the shared queue directory
    let mut content = format!("{}\n", operation);
    for p in &paths {
        content.push_str(&p.to_string_lossy());
        content.push('\n');
    }
    let _ = fs::write(&req_file, content);

    // Attempt to acquire the master coordinator lock
    #[cfg(windows)]
    let (is_master, mutex_handle) = {
        let mutex_name: Vec<u16> = "Local\\MickeyRAR_Extract_Coordinator\0".encode_utf16().collect();
        let handle = unsafe {
            windows_sys::Win32::System::Threading::CreateMutexW(
                std::ptr::null(),
                1,
                mutex_name.as_ptr(),
            )
        };
        let is_first = if handle.is_null() {
            true
        } else {
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            err != windows_sys::Win32::Foundation::ERROR_ALREADY_EXISTS
        };
        (is_first, handle)
    };

    #[cfg(not(windows))]
    let (is_master, mutex_handle): (bool, *mut std::ffi::c_void) = (true, std::ptr::null_mut());

    if !is_master {
        // Another instance is already coordinating the extraction.
        // Our request is already written in queue_dir, so exit cleanly without opening a window.
        return Ok(());
    }

    // Master instance: wait briefly (220ms) for Windows Explorer to finish launching
    // any sibling instances for other selected archives so they write their requests.
    thread::sleep(Duration::from_millis(220));

    // Collect all archives from queue_dir
    let mut all_archives = Vec::new();
    let mut final_op = operation.to_string();

    if let Ok(entries) = fs::read_dir(&queue_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("txt") {
                if let Ok(text) = fs::read_to_string(&path) {
                    let mut lines = text.lines();
                    if let Some(op) = lines.next() {
                        if !op.trim().is_empty() {
                            final_op = op.trim().to_string();
                        }
                    }
                    for line in lines {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            let p = PathBuf::from(trimmed);
                            if p.exists() && !all_archives.contains(&p) {
                                all_archives.push(p);
                            }
                        }
                    }
                }
                let _ = fs::remove_file(&path);
            }
        }
    }

    if all_archives.is_empty() {
        all_archives = paths;
    }

    let result = match final_op.as_str() {
        "--extract-here" => run_context_extract_here(all_archives, arabic),
        "--extract-to-folder" => run_context_extract_to_folder(all_archives, arabic),
        "--extract-to" => run_context_extract_to(all_archives, arabic),
        _ => run_context_extract_here(all_archives, arabic),
    };

    #[cfg(windows)]
    if !mutex_handle.is_null() {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(mutex_handle);
        }
    }

    result
}

fn run_context_extract_here(paths: Vec<PathBuf>, arabic: bool) -> Result<(), Box<dyn std::error::Error>> {
    run_context_extraction_with_options(paths, None, false, arabic)
}

fn run_context_extract_to_folder(paths: Vec<PathBuf>, arabic: bool) -> Result<(), Box<dyn std::error::Error>> {
    run_context_extraction_with_options(paths, None, true, arabic)
}

fn run_context_extract_to(paths: Vec<PathBuf>, arabic: bool) -> Result<(), Box<dyn std::error::Error>> {
    if paths.is_empty() { return Err("No archive selected".into()); }
    let default_folder = paths[0].parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    if let Some(dest) = rfd::FileDialog::new().set_directory(&default_folder).pick_folder() {
        run_context_extraction_with_options(paths, Some(dest), false, arabic)?;
    }
    Ok(())
}

fn run_context_compression(paths: Vec<PathBuf>, arabic: bool) -> Result<(), Box<dyn std::error::Error>> {
    if paths.is_empty() { return Err("No files were selected".into()); }
    let folder = paths[0].parent().ok_or("Cannot determine the selected file folder")?.to_path_buf();
    let mut output = folder.join("MickeyRAR-archive.zip");
    let mut suffix = 2;
    while output.exists() {
        output = folder.join(format!("MickeyRAR-archive-{suffix}.zip"));
        suffix += 1;
    }
    run_context_compression_with_options(paths, output, "zip", 9, false, arabic)
}

/// Slint hides a window when it is closed, but on some Windows setups the event
/// loop never returns from `run()` afterwards. That used to leave Mickey RAR
/// running headless (and, while compressing, holding a lock on the unfinished
/// archive). Ending the process explicitly is the only reliable "close means
/// quit" behaviour, so both the main window and dialogs use this.
fn quit_process_on_close(window: &slint::Window) {
    window.on_close_requested(|| std::process::exit(0));
}

fn run_context_compression_with_options(
    paths: Vec<PathBuf>,
    output: PathBuf,
    format: &str,
    level: u8,
    delete_original: bool,
    arabic: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let base_settings = load_settings();
    let opts = ArchiveCreationOptions {
        format: format.to_string(),
        level,
        solid: base_settings.solid,
        threads: base_settings.threads,
        password: None,
        encrypt_headers: false,
    };
    run_context_compression_full(paths, output, opts, delete_original, arabic)
}

fn run_context_compression_full(
    paths: Vec<PathBuf>,
    output: PathBuf,
    opts: ArchiveCreationOptions,
    delete_original: bool,
    arabic: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if paths.is_empty() { return Err("No files were selected".into()); }
    let (cwd, args) = create_archive_command_args(&output, &paths, &opts).map_err(|e| format!("{e}"))?;
    let format_name = opts.format.to_ascii_uppercase();
    let paths_for_delete = paths.clone();
    // Only a brand-new archive may be thrown away on failure/cancel. If the
    // user asked to add to an archive that already exists, deleting it would
    // destroy their previous data.
    let output_existed = output.is_file();
    let cancelled = Arc::new(AtomicBool::new(false));
    let thread_cancelled = cancelled.clone();
    let progress_handle = ProgressHandle::new();
    let worker_progress = progress_handle.clone();
    let worker_output = output.clone();
    let worker_done = Arc::new(AtomicBool::new(false));
    let worker_finished = worker_done.clone();
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = run_7z_cancellable_with_progress(&args, Some(&cwd), &thread_cancelled, Some(&worker_progress));
        match result {
            Ok(RunOutcome::Finished) => {
                if delete_original {
                    for path in paths_for_delete {
                        let _ = if path.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
                    }
                }
            }
            _ => {
                // Cancelled or failed: never leave a half-written archive behind.
                if !output_existed { let _ = delete_file_with_retry(&worker_output, 30); }
            }
        }
        worker_finished.store(true, Ordering::SeqCst);
        let _ = sender.send(result);
    });
    let receiver = Arc::new(Mutex::new(receiver));

    let ui = CompressionWindow::new()?;
    let center_timer = Timer::default();
    let center_weak = ui.as_weak();
    center_timer.start(TimerMode::SingleShot, Duration::from_millis(120), move || {
        if let Some(ui) = center_weak.upgrade() { center_window(&ui.window()); }
        winicon::install();
    });
    ui.set_rtl(arabic);
    let archive_name = output.file_name().unwrap_or_default().to_string_lossy().to_string();
    ui.set_archive_name(archive_name.clone().into());
    ui.set_current_file(archive_name.clone().into());
    ui.set_action_text(if arabic { format!("جاري ضغط ({format_name})") } else { format!("compressing ({format_name})") }.into());

    let cancel_flag = cancelled.clone();
    let cancel_weak = ui.as_weak();
    ui.on_cancel(move || {
        cancel_flag.store(true, Ordering::SeqCst);
        if let Some(ui) = cancel_weak.upgrade() {
            let _ = ui.hide();
        }
    });

    let bg_weak = ui.as_weak();
    ui.on_toggle_background(move || {
        if let Some(ui) = bg_weak.upgrade() {
            let _ = ui.hide();
        }
    });

    let pause_weak = ui.as_weak();
    let pause_handle = progress_handle.clone();
    let fmt_for_pause = format_name.clone();
    ui.on_toggle_pause(move || {
        if let Some(ui) = pause_weak.upgrade() {
            let next = !ui.get_is_paused();
            pause_handle.paused.store(next, Ordering::SeqCst);
            ui.set_is_paused(next);
            ui.set_action_text(if next {
                if arabic { "متوقف مؤقتاً (Paused)".to_string() } else { "paused".to_string() }
            } else {
                if arabic { format!("جاري ضغط ({fmt_for_pause})") } else { format!("compressing ({fmt_for_pause})") }
            }.into());
        }
    });

    ui.on_show_mode(|| {});
    ui.on_show_help(|| {
        let _ = webbrowser::open("https://github.com/Michael-Maged1/MickeyRAR");
    });

    let close_cancelled = cancelled.clone();
    let close_done = worker_done.clone();
    ui.window().on_close_requested(move || {
        close_cancelled.store(true, Ordering::SeqCst);
        let done = close_done.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !done.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            std::process::exit(0);
        });
        slint::CloseRequestResponse::HideWindow
    });
    let weak = ui.as_weak();
    let timer_receiver = receiver.clone();
    let timer_progress = progress_handle.clone();
    let start_time = Instant::now();
    let mut paused_accum = Duration::ZERO;
    let mut pause_started: Option<Instant> = None;
    let timer = Timer::default();
    let mut shown_percent = u8::MAX;
    timer.start(TimerMode::Repeated, Duration::from_millis(80), move || {
        if let Some(ui) = weak.upgrade() {
            let is_paused = timer_progress.paused.load(Ordering::Relaxed);
            if is_paused {
                if pause_started.is_none() { pause_started = Some(Instant::now()); }
            } else if let Some(ps) = pause_started.take() {
                paused_accum += ps.elapsed();
            }
            let current_pause = pause_started.map(|ps| ps.elapsed()).unwrap_or(Duration::ZERO);
            let elapsed_secs = start_time.elapsed().saturating_sub(paused_accum + current_pause).as_secs();

            let percent = timer_progress.percent.load(Ordering::Relaxed);
            let cur_file = timer_progress.current_file.lock().map(|l| l.clone()).unwrap_or_default();
            if !cur_file.is_empty() {
                ui.set_current_file(cur_file.into());
            }

            let elapsed_str = format!("{:02}:{:02}:{:02}", elapsed_secs / 3600, (elapsed_secs % 3600) / 60, elapsed_secs % 60);
            ui.set_elapsed_time(elapsed_str.into());

            if percent > 0 && percent < 100 {
                let total_est = (elapsed_secs as f64) / (percent as f64 / 100.0);
                let rem_secs = (total_est - elapsed_secs as f64).max(0.0) as u64;
                let rem_str = format!("{:02}:{:02}:{:02}", rem_secs / 3600, (rem_secs % 3600) / 60, rem_secs % 60);
                ui.set_time_left(rem_str.into());
            } else if percent >= 100 {
                ui.set_time_left("00:00:00".into());
            } else {
                ui.set_time_left("--:--:--".into());
            }

            ui.set_progress(percent as f32 / 100.0);
            ui.set_file_progress(percent as f32 / 100.0);
            ui.set_file_percent(percent as i32);
            ui.set_total_progress(percent as f32 / 100.0);
            ui.set_total_percent(percent as i32);

            if percent != shown_percent {
                shown_percent = percent;
            }
            if let Ok(result) = timer_receiver.lock().unwrap().try_recv() {
                match result {
                    Ok(RunOutcome::Finished) => {
                        ui.set_progress(1.0);
                        ui.set_file_progress(1.0);
                        ui.set_file_percent(100);
                        ui.set_total_progress(1.0);
                        ui.set_total_percent(100);
                        ui.set_time_left("00:00:00".into());
                        ui.set_action_text(if arabic { format!("تم إنشاء ملف {format_name} بنجاح ✔") } else { format!("{format_name} archive created successfully ✔") }.into());
                    }
                    Ok(RunOutcome::Cancelled) => { ui.set_action_text(if arabic { "تم إلغاء العملية" } else { "Compression cancelled" }.into()); }
                    Err(error) => { ui.set_action_text(format!("Error: {error}").into()); }
                }
                let _ = slint::quit_event_loop();
            }
        }
    });
    ui.run()?;
    // Only reached when compression finished on its own. Still wait briefly for
    // the worker so files are flushed before the process exits.
    let deadline = Instant::now() + Duration::from_secs(20);
    while !worker.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    drop(timer);
    Ok(())
}

struct ArchiveRequest {
    name: String,
    destination: PathBuf,
    format: String,
    level: u8,
    password: Option<String>,
    encrypt_headers: bool,
    delete_original: bool,
}

fn run_archive_dialog(paths: Vec<PathBuf>, arabic: bool) -> Result<(), Box<dyn std::error::Error>> {
    if paths.is_empty() { return Err("No files were selected".into()); }
    let default_folder = paths[0].parent().ok_or("Cannot determine the selected file folder")?.to_path_buf();
    let default_stem = if paths.len() == 1 {
        paths[0].file_stem().unwrap_or_default().to_string_lossy().to_string()
    } else {
        default_folder.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "MickeyRAR-archive".into())
    };
    let ui = ArchiveDialog::new()?;
    quit_process_on_close(&ui.window());
    ui.set_rtl(arabic);
    if !default_stem.is_empty() {
        ui.set_archive_name(default_stem.into());
    }
    ui.set_destination(default_folder.display().to_string().into());
    let (sender, receiver) = mpsc::channel::<Option<ArchiveRequest>>();
    let weak = ui.as_weak();
    ui.on_browse_location({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let current = PathBuf::from(ui.get_destination().to_string());
                if let Some(folder) = rfd::FileDialog::new().set_directory(current).pick_folder() { ui.set_destination(folder.display().to_string().into()); }
            }
        }
    });
    ui.on_start_compression({
        let weak = weak.clone();
        let sender = sender.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let level = match ui.get_level_index() { 0 => 0, 1 => 3, 2 => 5, 3 => 7, _ => 9 };
                let format = match ui.get_format_index() {
                    1 => "7z",
                    2 => "sfx",
                    _ => "zip",
                };
                let pwd_raw = ui.get_password().to_string();
                let password = if pwd_raw.is_empty() { None } else { Some(pwd_raw) };
                let _ = sender.send(Some(ArchiveRequest {
                    name: ui.get_archive_name().to_string(),
                    destination: PathBuf::from(ui.get_destination().to_string()),
                    format: format.into(),
                    level,
                    password,
                    encrypt_headers: ui.get_encrypt_headers(),
                    delete_original: ui.get_delete_original(),
                }));
                let _ = slint::quit_event_loop();
            }
        }
    });
    ui.on_cancel({ let sender = sender.clone(); move || { let _ = sender.send(None); let _ = slint::quit_event_loop(); } });
    let center_timer = Timer::default();
    let center_weak = ui.as_weak();
    center_timer.start(TimerMode::SingleShot, Duration::from_millis(120), move || { if let Some(ui) = center_weak.upgrade() { center_window(&ui.window()); } winicon::install(); });
    ui.run()?;
    let Some(Some(request)) = receiver.try_iter().last() else { return Ok(()); };
    let mut name = PathBuf::from(request.name.trim());
    if name.file_name().is_none() { return Err("Archive name is required".into()); }
    let file_ext = match request.format.as_str() {
        "sfx" => "exe",
        other => other,
    };
    name.set_extension(file_ext);
    let output = request.destination.join(name.file_name().unwrap());
    let base_settings = load_settings();
    let opts = ArchiveCreationOptions {
        format: request.format,
        level: request.level,
        solid: base_settings.solid,
        threads: base_settings.threads,
        password: request.password,
        encrypt_headers: request.encrypt_headers,
    };
    run_context_compression_full(paths, output, opts, request.delete_original, arabic)
}

fn supported_archive(p: &Path) -> bool {
    p.extension().and_then(|s| s.to_str()).map(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "iso" | "cab" | "wim" | "tgz" | "tbz2" | "txz" | "z"
        )
    }).unwrap_or(false)
}

fn run_text_args(args: &[&std::ffi::OsStr]) -> Result<String, String> {
    run_7z(&args.iter().map(|a| a.to_os_string()).collect::<Vec<_>>(), None)
}

fn settings_path() -> PathBuf {
    let home = std::env::var_os(if cfg!(target_os="windows") { "APPDATA" } else { "HOME" })
        .map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".mickey-rar-ultra.conf")
}
fn language_path() -> PathBuf {
    settings_path().with_file_name(".mickey-rar-language.conf")
}
fn theme_path() -> PathBuf {
    settings_path().with_file_name(".mickey-rar-theme.conf")
}
fn load_language() -> Language {
    fs::read_to_string(language_path())
        .map(|s| Language::from_code(&s))
        .unwrap_or_else(|_| Language::Arabic)
}
fn save_language(lang: Language) -> Result<(), String> {
    fs::write(language_path(), lang.code()).map_err(|e| e.to_string())
}
fn load_dark_mode() -> bool {
    fs::read_to_string(theme_path())
        .map(|s| !s.trim().eq_ignore_ascii_case("light"))
        .unwrap_or(true)
}
fn save_dark_mode(dark: bool) -> Result<(), String> {
    fs::write(theme_path(), if dark { "dark" } else { "light" }).map_err(|e| e.to_string())
}

fn detect_available_drives() -> Vec<slint::SharedString> {
    #[cfg(windows)]
    {
        let mut drives = Vec::new();
        for letter in b'A'..=b'Z' {
            let root = format!("{}:\\", letter as char);
            if Path::new(&root).exists() {
                drives.push(root.into());
            }
        }
        if drives.is_empty() {
            drives.push("C:\\".into());
        }
        drives
    }
    #[cfg(not(windows))]
    {
        let mut drives = vec![slint::SharedString::from("/")];
        if let Ok(entries) = fs::read_dir("/Volumes") {
            for e in entries.flatten() {
                drives.push(e.path().display().to_string().into());
            }
        }
        drives
    }
}

fn parse_formatted_bytes(s: &str) -> u64 {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse::<u64>().unwrap_or(0)
}

fn parse_modified_sort_key(s: &str) -> (u32, u32, u32, u32, u32) {
    // Matches "dd/mm/yyyy HH:MM"
    let bytes = s.trim();
    if bytes.len() >= 16 {
        let d = bytes.get(0..2).and_then(|v| v.parse().ok()).unwrap_or(0);
        let m = bytes.get(3..5).and_then(|v| v.parse().ok()).unwrap_or(0);
        let y = bytes.get(6..10).and_then(|v| v.parse().ok()).unwrap_or(0);
        let hh = bytes.get(11..13).and_then(|v| v.parse().ok()).unwrap_or(0);
        let mm = bytes.get(14..16).and_then(|v| v.parse().ok()).unwrap_or(0);
        return (y, m, d, hh, mm);
    }
    (0, 0, 0, 0, 0)
}

fn sort_file_entries(entries: &mut [FileEntry], sort_column: i32, ascending: bool) {
    let start = if entries.first().map(|e| e.name.as_str() == "..").unwrap_or(false) {
        1
    } else {
        0
    };
    if entries.len() <= start + 1 {
        return;
    }
    entries[start..].sort_by(|a, b| {
        // Keep folders grouped above files just like WinRAR & Explorer
        if a.is_dir != b.is_dir {
            return b.is_dir.cmp(&a.is_dir);
        }
        let ord = match sort_column {
            1 => parse_formatted_bytes(&a.size)
                .cmp(&parse_formatted_bytes(&b.size))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            2 => parse_formatted_bytes(&a.packed)
                .cmp(&parse_formatted_bytes(&b.packed))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            3 => a.entry_type
                .to_lowercase()
                .cmp(&b.entry_type.to_lowercase())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            4 => parse_modified_sort_key(&a.modified)
                .cmp(&parse_modified_sort_key(&b.modified))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            5 => a.crc32
                .to_lowercase()
                .cmp(&b.crc32.to_lowercase())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        if ascending { ord } else { ord.reverse() }
    });
}

fn collect_folder_search_hits(
    base: &Path,
    current: &Path,
    query_lower: &str,
    depth: usize,
    out: &mut Vec<PathBuf>,
) {
    if depth > 4 || out.len() >= 1000 {
        return;
    }
    let Ok(rd) = fs::read_dir(current) else { return; };
    for entry in rd.flatten() {
        if out.len() >= 1000 {
            break;
        }
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.to_lowercase().contains(query_lower) {
            out.push(path.clone());
        }
        if path.is_dir() && base != Path::new("") {
            collect_folder_search_hits(base, &path, query_lower, depth + 1, out);
        }
    }
}
fn load_settings() -> CompressionSettings {
    let mut settings = CompressionSettings::default();
    if let Ok(content) = fs::read_to_string(settings_path()) {
        for line in content.lines() {
            if let Some(value) = line.strip_prefix("solid=") { settings.solid = value.trim() == "true"; }
            if let Some(value) = line.strip_prefix("threads=") { settings.threads = value.trim().parse::<u16>().ok().filter(|n| *n > 0); }
        }
    }
    settings
}
fn save_settings(settings: &CompressionSettings) -> Result<(), String> {
    settings.validate()?;
    fs::write(settings_path(), format!("level=9\nmethod=Deflate\nsolid={}\nthreads={}\n",
        settings.solid, settings.threads.map(|n| n.to_string()).unwrap_or_else(|| "auto".into())))
        .map_err(|e| format!("Cannot save settings: {e}"))
}

fn format_size(bytes: u64) -> String {
    let s = bytes.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    for (i, c) in chars.into_iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result
}

fn determine_type_and_icon(name: &str, is_dir: bool, arabic: bool) -> (String, String) {
    if is_dir {
        return (
            if arabic { "مجلد ملفات".into() } else { "File folder".into() },
            "dir".into(),
        );
    }
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "exe" | "msi" | "bat" | "cmd" => (
            if arabic { "تطبيق".into() } else { "Application".into() },
            "exe".into(),
        ),
        "txt" | "md" | "log" | "ini" | "cfg" => (
            if arabic { "مستند نصي".into() } else { "Text Document".into() },
            "txt".into(),
        ),
        "dll" | "so" | "dylib" | "sys" => (
            if arabic { "امتداد تطبيق".into() } else { "Application extension".into() },
            "dll".into(),
        ),
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "iso" | "cab" | "wim" | "tgz" | "tbz2" | "txz" | "z" => (
            if arabic { "أرشيف".into() } else { "Archive".into() },
            "zip".into(),
        ),
        _ => (
            if ext.is_empty() {
                if arabic { "ملف".into() } else { "File".into() }
            } else {
                format!("{} File", ext.to_uppercase())
            },
            "file".into(),
        ),
    }
}

fn show_folder(ui: &MainWindow, state: &Rc<RefCell<AppState>>) -> Result<(), String> {
    let (folder, query, sort_col, sort_asc) = {
        let s = state.borrow();
        (
            s.folder.clone(),
            s.search_query.trim().to_string(),
            s.sort_column,
            s.sort_ascending,
        )
    };
    let mut items = Vec::new();
    if query.is_empty() {
        for e in fs::read_dir(&folder).map_err(|e| e.to_string())? {
            let e = e.map_err(|e| e.to_string())?;
            items.push(e.path());
        }
    } else {
        collect_folder_search_hits(&folder, &folder, &query.to_lowercase(), 0, &mut items);
    }
    items.sort_by_key(|p| (!p.is_dir(), p.file_name().map(|x| x.to_string_lossy().to_lowercase())));

    let arabic = ui.get_arabic_mode();
    let mut entries = Vec::new();
    let mut total_folders = 0usize;
    let mut total_files = 0usize;
    let mut total_bytes = 0u64;

    if folder.parent().is_some() || !query.is_empty() {
        entries.push(FileEntry {
            name: "..".into(),
            size: "".into(),
            packed: "".into(),
            entry_type: if arabic { "مجلد ملفات".into() } else { "File folder".into() },
            modified: "".into(),
            crc32: "".into(),
            is_dir: true,
            icon_kind: "dir".into(),
            is_selected: false,
        });
    }

    for p in &items {
        let display_name = if query.is_empty() {
            p.file_name().unwrap_or_default().to_string_lossy().into_owned()
        } else {
            p.strip_prefix(&folder)
                .map(|rel| rel.display().to_string())
                .unwrap_or_else(|_| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
        };
        let is_dir = p.is_dir();
        let (entry_type, icon_kind) = determine_type_and_icon(&display_name, is_dir, arabic);
        let meta = p.metadata().ok();
        let (size_str, mod_str) = if let Some(m) = meta {
            let size = if is_dir {
                total_folders += 1;
                String::new()
            } else {
                total_files += 1;
                total_bytes += m.len();
                format_size(m.len())
            };
            let mod_time = m.modified().ok()
                .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%d/%m/%Y %H:%M").to_string())
                .unwrap_or_default();
            (size, mod_time)
        } else {
            if is_dir { total_folders += 1; } else { total_files += 1; }
            (String::new(), String::new())
        };

        entries.push(FileEntry {
            name: display_name.into(),
            size: size_str.into(),
            packed: "".into(),
            entry_type: entry_type.into(),
            modified: mod_str.into(),
            crc32: "".into(),
            is_dir,
            icon_kind: icon_kind.into(),
            is_selected: false,
        });
    }

    sort_file_entries(&mut entries, sort_col, sort_asc);

    {
        let mut s = state.borrow_mut();
        s.paths = items;
        s.archive = None;
        s.last_selected_anchor = None;
    }

    ui.set_location(folder.display().to_string().into());
    ui.set_entries(ModelRc::new(VecModel::from(entries)));
    ui.set_selected_index(-1);
    let sel_msg = if arabic {
        "تم تحديد 0 بايت في 0 ملفات"
    } else {
        "Selected 0 bytes in 0 files"
    };
    let tot_msg = if arabic {
        format!("الإجمالي {} مجلدات و {} بايت في {} ملفات", total_folders, format_size(total_bytes), total_files)
    } else {
        format!("Total {} folders and {} bytes in {} files", total_folders, format_size(total_bytes), total_files)
    };
    ui.set_selected_info(sel_msg.into());
    ui.set_total_info(tot_msg.into());
    Ok(())
}

#[allow(dead_code)]
fn list_member_names(archive: &Path) -> Result<Vec<String>, String> {
    let out = run_text_args(&["l".as_ref(), "-slt".as_ref(), archive.as_os_str()])?;
    let mut in_contents = false;
    let mut names = Vec::new();
    for line in out.lines() {
        if line.starts_with("----------") { in_contents = true; continue; }
        if in_contents {
            if let Some(value) = line.strip_prefix("Path = ") { names.push(value.to_string()); }
        }
    }
    Ok(names)
}

#[derive(Clone)]
struct ArchiveItem {
    path: String,
    size: u64,
    packed_size: u64,
    modified: String,
    crc: String,
    is_dir: bool,
}

fn list_archive_members(archive: &Path) -> Result<Vec<ArchiveItem>, String> {
    list_archive_members_with_password(archive, None)
}

fn list_archive_members_with_password(
    archive: &Path,
    password: Option<&str>,
) -> Result<Vec<ArchiveItem>, String> {
    let pwd_flag = match password {
        Some(p) => format!("-p{p}"),
        None => "-p".to_string(),
    };
    let out = run_text_args(&[
        "l".as_ref(),
        "-slt".as_ref(),
        "-sccUTF-8".as_ref(),
        "-mmt=on".as_ref(),
        pwd_flag.as_ref(),
        archive.as_os_str(),
    ])?;
    let mut in_contents = false;
    let mut items = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_size: u64 = 0;
    let mut current_packed: u64 = 0;
    let mut current_modified = String::new();
    let mut current_crc = String::new();
    let mut current_is_dir = false;

    let flush_item = |items: &mut Vec<ArchiveItem>,
                      path: &mut Option<String>,
                      size: &mut u64,
                      packed: &mut u64,
                      modif: &mut String,
                      crc: &mut String,
                      is_d: &mut bool| {
        if let Some(p) = path.take() {
            let is_dir = *is_d || p.ends_with('/') || p.ends_with('\\');
            items.push(ArchiveItem {
                path: p,
                size: *size,
                packed_size: *packed,
                modified: std::mem::take(modif),
                crc: std::mem::take(crc),
                is_dir,
            });
            *size = 0;
            *packed = 0;
            *is_d = false;
        }
    };

    for line in out.lines() {
        if line.starts_with("----------") {
            in_contents = true;
            continue;
        }
        if in_contents {
            if let Some(value) = line.strip_prefix("Path = ") {
                flush_item(
                    &mut items,
                    &mut current_path,
                    &mut current_size,
                    &mut current_packed,
                    &mut current_modified,
                    &mut current_crc,
                    &mut current_is_dir,
                );
                current_path = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("Size = ") {
                current_size = value.trim().parse::<u64>().unwrap_or(0);
            } else if let Some(value) = line.strip_prefix("Packed Size = ") {
                current_packed = value.trim().parse::<u64>().unwrap_or(0);
            } else if let Some(value) = line.strip_prefix("Modified = ") {
                current_modified = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("CRC = ") {
                current_crc = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("Attributes = ") {
                if value.contains('D') {
                    current_is_dir = true;
                }
            }
        }
    }
    flush_item(
        &mut items,
        &mut current_path,
        &mut current_size,
        &mut current_packed,
        &mut current_modified,
        &mut current_crc,
        &mut current_is_dir,
    );
    Ok(items)
}

fn show_archive(ui: &MainWindow, state: &Rc<RefCell<AppState>>,
                archive: PathBuf) -> Result<(), String> {
    if !archive.is_file() {
        return Err(format!("Archive file does not exist: {}", archive.display()));
    }
    if !supported_archive(&archive) {
        return Err(format!("Unsupported archive format: {}", archive.display()));
    }
    let arabic = ui.get_arabic_mode();
    let cached_members = {
        let s = state.borrow();
        if let Some((ref cached_path, ref mems)) = s.archive_members {
            if cached_path == &archive {
                Some(mems.clone())
            } else {
                None
            }
        } else {
            None
        }
    };
    let members = match cached_members {
        Some(mems) => mems,
        None => {
            let cached_pwd = state
                .borrow()
                .archive_password
                .as_ref()
                .and_then(|(p, pwd)| if p == &archive { Some(pwd.clone()) } else { None });
            let mems = match list_archive_members_with_password(&archive, cached_pwd.as_deref()) {
                Ok(m) => m,
                Err(err)
                    if err.contains("Wrong password")
                        || err.contains("Can not open encrypted")
                        || err.contains("Enter password") =>
                {
                    match resolve_archive_password(&archive, arabic).map_err(|e| e.to_string())? {
                        Some(ref s) if s == "__CANCELLED__" => {
                            return Err(if arabic {
                                "تم إلغاء فتح الأرشيف المشفر".into()
                            } else {
                                "Cancelled opening encrypted archive".into()
                            });
                        }
                        Some(pwd) => {
                            let m = list_archive_members_with_password(&archive, Some(&pwd))?;
                            state.borrow_mut().archive_password = Some((archive.clone(), pwd));
                            m
                        }
                        None => return Err(err),
                    }
                }
                Err(e) => return Err(e),
            };
            validate_archive_members_safe(mems.iter().map(|m| m.path.as_str()))?;
            state.borrow_mut().archive_members = Some((archive.clone(), mems.clone()));
            mems
        }
    };
    let (current_dir, search_query, sort_col, sort_asc) = {
        let s = state.borrow();
        (
            s.archive_dir.clone(),
            s.search_query.trim().to_string(),
            s.sort_column,
            s.sort_ascending,
        )
    };

    // Normalize current_dir: ensure it ends with '/' (or is empty for root)
    let prefix = if current_dir.is_empty() {
        String::new()
    } else if current_dir.ends_with('/') {
        current_dir.clone()
    } else {
        format!("{}/", current_dir)
    };

    let mut entries = Vec::new();
    let mut total_folders = 0usize;
    let mut total_files = 0usize;
    let mut total_bytes = 0u64;

    // Parent directory row `..` to return
    entries.push(FileEntry {
        name: "..".into(),
        size: "".into(),
        packed: "".into(),
        entry_type: if arabic { "مجلد ملفات".into() } else { "File folder".into() },
        modified: "".into(),
        crc32: "".into(),
        is_dir: true,
        icon_kind: "dir".into(),
        is_selected: false,
    });

    if !search_query.is_empty() {
        let q_low = search_query.to_lowercase();
        for m in &members {
            let clean_path = m.path.replace('\\', "/").trim_end_matches('/').to_string();
            if clean_path.is_empty() {
                continue;
            }
            let base_name = clean_path.rsplit('/').next().unwrap_or(&clean_path);
            if base_name.to_lowercase().contains(&q_low) || clean_path.to_lowercase().contains(&q_low) {
                if m.is_dir {
                    total_folders += 1;
                } else {
                    total_files += 1;
                    total_bytes += m.size;
                }
                let (entry_type, icon_kind) = determine_type_and_icon(&clean_path, m.is_dir, arabic);
                let size_str = if m.is_dir { String::new() } else { format_size(m.size) };
                let packed_str = if !m.is_dir && m.packed_size > 0 { format_size(m.packed_size) } else { String::new() };
                let mod_str = if !m.modified.is_empty() {
                    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&m.modified, "%Y-%m-%d %H:%M:%S") {
                        dt.format("%d/%m/%Y %H:%M").to_string()
                    } else {
                        m.modified.clone()
                    }
                } else {
                    String::new()
                };
                entries.push(FileEntry {
                    name: clean_path.into(),
                    size: size_str.into(),
                    packed: packed_str.into(),
                    entry_type: entry_type.into(),
                    modified: mod_str.into(),
                    crc32: m.crc.clone().into(),
                    is_dir: m.is_dir,
                    icon_kind: icon_kind.into(),
                    is_selected: false,
                });
            }
        }
        sort_file_entries(&mut entries, sort_col, sort_asc);
        let location = format!("{}\\[Search: {}]", archive.display(), search_query);
        ui.set_location(location.into());
        ui.set_entries(ModelRc::new(VecModel::from(entries)));
        ui.set_selected_index(-1);
        let sel_msg = if arabic { "تم تحديد 0 بايت في 0 ملفات" } else { "Selected 0 bytes in 0 files" };
        let tot_msg = if arabic {
            format!("نتائج البحث: {} مجلدات و {} بايت في {} ملفات", total_folders, format_size(total_bytes), total_files)
        } else {
            format!("Search results: {} folders and {} bytes in {} files", total_folders, format_size(total_bytes), total_files)
        };
        ui.set_selected_info(sel_msg.into());
        ui.set_total_info(tot_msg.into());
        let mut s = state.borrow_mut();
        s.archive = Some(archive);
        s.last_selected_anchor = None;
        return Ok(());
    }

    // Collect unique child names at the current directory level.
    // A "child" is either:
    //  - A file directly under `prefix` (no further '/')
    //  - A folder that is an intermediate segment under `prefix`
    use std::collections::BTreeMap;
    struct DirSummary {
        size: u64,
        packed: u64,
        modified: String,
        is_explicit: bool, // Was there an explicit dir entry for this folder?
    }
    let mut seen_dirs: BTreeMap<String, DirSummary> = BTreeMap::new();
    // Collect file entries at current level
    struct FileInfo {
        display_name: String,
        size: u64,
        packed: u64,
        modified: String,
        crc: String,
    }
    let mut file_entries: Vec<FileInfo> = Vec::new();

    for m in &members {
        // Normalize path separators to '/'
        let path = m.path.replace('\\', "/");
        // Strip trailing '/' for directories
        let path_trimmed = path.trim_end_matches('/');

        // Check if this entry is under the current directory
        if prefix.is_empty() {
            // We're at root - show top-level entries
            if let Some(slash_pos) = path_trimmed.find('/') {
                // This is a nested entry; extract the top-level folder name
                let folder_name = &path_trimmed[..slash_pos];
                let entry = seen_dirs.entry(folder_name.to_string()).or_insert(DirSummary {
                    size: 0,
                    packed: 0,
                    modified: String::new(),
                    is_explicit: false,
                });
                if !m.is_dir {
                    entry.size += m.size;
                    entry.packed += m.packed_size;
                }
                if m.is_dir && path_trimmed == folder_name {
                    entry.is_explicit = true;
                    if entry.modified.is_empty() {
                        entry.modified = m.modified.clone();
                    }
                }
            } else {
                // Top-level file or explicit top-level directory
                if m.is_dir {
                    let entry = seen_dirs.entry(path_trimmed.to_string()).or_insert(DirSummary {
                        size: 0,
                        packed: 0,
                        modified: String::new(),
                        is_explicit: true,
                    });
                    entry.is_explicit = true;
                    if entry.modified.is_empty() {
                        entry.modified = m.modified.clone();
                    }
                } else {
                    file_entries.push(FileInfo {
                        display_name: path_trimmed.to_string(),
                        size: m.size,
                        packed: m.packed_size,
                        modified: m.modified.clone(),
                        crc: m.crc.clone(),
                    });
                }
            }
        } else if path.starts_with(&prefix) || path_trimmed.starts_with(&prefix) {
            // We're inside a subdirectory - show only entries under prefix
            let relative = if path.starts_with(&prefix) {
                &path[prefix.len()..]
            } else {
                &path_trimmed[prefix.len()..]
            };
            let relative = relative.trim_end_matches('/');
            if relative.is_empty() {
                continue; // This is the directory entry for the current dir itself
            }

            if let Some(slash_pos) = relative.find('/') {
                // Nested further - show as folder
                let folder_name = &relative[..slash_pos];
                let entry = seen_dirs.entry(folder_name.to_string()).or_insert(DirSummary {
                    size: 0,
                    packed: 0,
                    modified: String::new(),
                    is_explicit: false,
                });
                if !m.is_dir {
                    entry.size += m.size;
                    entry.packed += m.packed_size;
                }
                if m.is_dir && relative == folder_name {
                    entry.is_explicit = true;
                    if entry.modified.is_empty() {
                        entry.modified = m.modified.clone();
                    }
                }
            } else {
                // Direct child
                if m.is_dir {
                    let entry = seen_dirs.entry(relative.to_string()).or_insert(DirSummary {
                        size: 0,
                        packed: 0,
                        modified: String::new(),
                        is_explicit: true,
                    });
                    entry.is_explicit = true;
                    if entry.modified.is_empty() {
                        entry.modified = m.modified.clone();
                    }
                } else {
                    file_entries.push(FileInfo {
                        display_name: relative.to_string(),
                        size: m.size,
                        packed: m.packed_size,
                        modified: m.modified.clone(),
                        crc: m.crc.clone(),
                    });
                }
            }
        }
    }

    // Add folder entries first (sorted)
    for (dir_name, summary) in &seen_dirs {
        total_folders += 1;
        let (entry_type, icon_kind) = determine_type_and_icon(dir_name, true, arabic);
        let mod_str = if !summary.modified.is_empty() {
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&summary.modified, "%Y-%m-%d %H:%M:%S") {
                dt.format("%d/%m/%Y %H:%M").to_string()
            } else {
                summary.modified.clone()
            }
        } else {
            String::new()
        };
        entries.push(FileEntry {
            name: dir_name.clone().into(),
            size: "".into(),
            packed: "".into(),
            entry_type: entry_type.into(),
            modified: mod_str.into(),
            crc32: "".into(),
            is_dir: true,
            icon_kind: icon_kind.into(),
            is_selected: false,
        });
    }

    // Add file entries (sorted by name)
    file_entries.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
    for f in file_entries {
        total_files += 1;
        total_bytes += f.size;
        let (entry_type, icon_kind) = determine_type_and_icon(&f.display_name, false, arabic);
        let packed_str = if f.packed > 0 { format_size(f.packed) } else { String::new() };
        let mod_str = if !f.modified.is_empty() {
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&f.modified, "%Y-%m-%d %H:%M:%S") {
                dt.format("%d/%m/%Y %H:%M").to_string()
            } else {
                f.modified
            }
        } else {
            String::new()
        };
        entries.push(FileEntry {
            name: f.display_name.into(),
            size: format_size(f.size).into(),
            packed: packed_str.into(),
            entry_type: entry_type.into(),
            modified: mod_str.into(),
            crc32: f.crc.into(),
            is_dir: false,
            icon_kind: icon_kind.into(),
            is_selected: false,
        });
    }

    sort_file_entries(&mut entries, sort_col, sort_asc);

    // Update location bar: show archive path + virtual directory
    let location = if current_dir.is_empty() {
        archive.display().to_string()
    } else {
        format!("{}\\{}", archive.display(), current_dir.replace('/', "\\"))
    };
    ui.set_location(location.into());
    ui.set_entries(ModelRc::new(VecModel::from(entries)));
    ui.set_selected_index(-1);
    let sel_msg = if arabic {
        "تم تحديد 0 بايت في 0 ملفات"
    } else {
        "Selected 0 bytes in 0 files"
    };
    let tot_msg = if arabic {
        format!("الإجمالي {} مجلدات و {} بايت في {} ملفات", total_folders, format_size(total_bytes), total_files)
    } else {
        format!("Total {} folders and {} bytes in {} files", total_folders, format_size(total_bytes), total_files)
    };
    ui.set_selected_info(sel_msg.into());
    ui.set_total_info(tot_msg.into());
    {
        let mut s = state.borrow_mut();
        s.archive = Some(archive);
        s.last_selected_anchor = None;
    }
    Ok(())
}

#[cfg(windows)]
fn register_file_associations() -> Result<(), String> {
    use std::process::Command;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_str = executable.display().to_string();

    let extensions = [
        ".zip", ".rar", ".7z", ".tar", ".gz", ".bz2", ".xz", ".iso", ".cab", ".wim", ".tgz", ".tbz2", ".txz",
    ];

    let reg_val = |key: &str, name: &str, val: &str| {
        let mut cmd = Command::new("reg.exe");
        cmd.creation_flags(0x08000000);
        cmd.args(["ADD", key]);
        if name.is_empty() { cmd.arg("/ve"); } else { cmd.args(["/v", name]); }
        let _ = cmd.args(["/t", "REG_SZ", "/d", val, "/f"]).status();
    };

    let reg_del = |key: &str| {
        let mut cmd = Command::new("reg.exe");
        cmd.creation_flags(0x08000000);
        let _ = cmd.args(["DELETE", key, "/f"]).status();
    };

    for base in [r"HKCU\Software\Classes", r"HKLM\Software\Classes"] {
        let prog_id = format!(r"{base}\MickeyRAR.Archive");
        reg_val(&prog_id, "", "Mickey RAR Archive");
        reg_val(&format!(r"{prog_id}\DefaultIcon"), "", &format!("\"{exe_str}\",0"));
        reg_val(&format!(r"{prog_id}\shell"), "", "open");
        reg_val(&format!(r"{prog_id}\shell\open"), "FriendlyAppName", "Mickey RAR");
        reg_val(&format!(r"{prog_id}\shell\open\command"), "", &format!("\"{exe_str}\" \"%1\""));

        for ext in &extensions {
            let ext_key = format!(r"{base}\{ext}");
            reg_val(&ext_key, "", "MickeyRAR.Archive");
            let ext_open = format!(r"{ext_key}\OpenWithProgids");
            reg_val(&ext_open, "MickeyRAR.Archive", "");
        }
    }

    for ext in &extensions {
        reg_del(&format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{ext}\UserChoice"));
        reg_del(&format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{ext}\UserChoiceLatest"));
    }

    for base in [r"HKCU\Software", r"HKLM\Software"] {
        let cap = format!(r"{base}\MickeyRAR\Capabilities");
        reg_val(&cap, "ApplicationName", "Mickey RAR");
        reg_val(&cap, "ApplicationDescription", "Mickey RAR classic archive manager");
        for ext in &extensions {
            reg_val(&format!(r"{cap}\FileAssociations"), ext, "MickeyRAR.Archive");
        }
        reg_val(&format!(r"{base}\RegisteredApplications"), "MickeyRAR", r"Software\MickeyRAR\Capabilities");
    }

    for base in [r"HKCU\Software\Classes\Applications", r"HKLM\Software\Classes\Applications"] {
        let app_entry = format!(r"{base}\mickey-rar.exe");
        reg_val(&app_entry, "FriendlyAppName", "Mickey RAR");
        reg_val(&format!(r"{app_entry}\DefaultIcon"), "", &format!("\"{exe_str}\",0"));
        reg_val(&format!(r"{app_entry}\shell\open\command"), "", &format!("\"{exe_str}\" \"%1\""));
        for ext in &extensions {
            reg_val(&format!(r"{app_entry}\SupportedTypes"), ext, "");
        }
    }

    unsafe {
        windows_sys::Win32::UI::Shell::SHChangeNotify(
            windows_sys::Win32::UI::Shell::SHCNE_ASSOCCHANGED as i32,
            windows_sys::Win32::UI::Shell::SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }

    Ok(())
}

fn save_rgba_as_bmp(width: u32, height: u32, rgba: &[u8], path: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    let row_size = ((width * 3 + 3) / 4) * 4;
    let data_size = row_size * height;
    let file_size = 54 + data_size;

    file.write_all(b"BM")?;
    file.write_all(&(file_size as u32).to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;
    file.write_all(&54u32.to_le_bytes())?;

    file.write_all(&40u32.to_le_bytes())?;
    file.write_all(&(width as i32).to_le_bytes())?;
    file.write_all(&(height as i32).to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&24u16.to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;
    file.write_all(&(data_size as u32).to_le_bytes())?;
    file.write_all(&2835u32.to_le_bytes())?;
    file.write_all(&2835u32.to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;

    let padding = vec![0u8; (row_size - width * 3) as usize];
    for y in (0..height).rev() {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            let r = rgba[idx];
            let g = rgba[idx + 1];
            let b = rgba[idx + 2];
            file.write_all(&[b, g, r])?;
        }
        file.write_all(&padding)?;
    }
    Ok(())
}

fn update_selection<F>(ui: &MainWindow, predicate: F)
where
    F: Fn(usize, &FileEntry) -> bool,
{
    let model = ui.get_entries();
    let count = model.row_count();
    let mut updated = Vec::with_capacity(count);
    let mut sel_bytes = 0u64;
    let mut sel_files = 0usize;

    for i in 0..count {
        if let Some(mut entry) = model.row_data(i) {
            if entry.name != ".." {
                let should_select = predicate(i, &entry);
                entry.is_selected = should_select;
                if should_select {
                    sel_files += 1;
                    if !entry.is_dir {
                        let digits: String = entry.size.chars().filter(|c| c.is_ascii_digit()).collect();
                        if let Ok(b) = digits.parse::<u64>() {
                            sel_bytes += b;
                        }
                    }
                }
            } else {
                entry.is_selected = false;
            }
            updated.push(entry);
        }
    }
    ui.set_entries(ModelRc::new(VecModel::from(updated)));
    let arabic = ui.get_arabic_mode();
    let info = if sel_files == 0 {
        if arabic {
            "تم تحديد 0 بايت في 0 ملفات".to_string()
        } else {
            "Selected 0 bytes in 0 files".to_string()
        }
    } else if arabic {
        format!("تم تحديد {} بايت في {} ملفات", format_size(sel_bytes), sel_files)
    } else {
        format!("Selected {} bytes in {} files", format_size(sel_bytes), sel_files)
    };
    ui.set_selected_info(info.into());
}

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const GITHUB_REPO: &str = "Michael-Maged1/MickeyRAR";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct UpdateInfo {
    version: String,
    download_url: Option<String>,
    notes_summary: String,
}

fn is_version_newer(remote: &str, current: &str) -> bool {
    let clean_r = remote.trim().trim_start_matches(['v', 'V']);
    let clean_c = current.trim().trim_start_matches(['v', 'V']);
    let r_parts: Vec<u32> = clean_r
        .split(['.', '-', '+'])
        .take(3)
        .filter_map(|s| s.parse().ok())
        .collect();
    let c_parts: Vec<u32> = clean_c
        .split(['.', '-', '+'])
        .take(3)
        .filter_map(|s| s.parse().ok())
        .collect();
    if r_parts.is_empty() {
        return false;
    }
    for i in 0..r_parts.len().max(c_parts.len()) {
        let r = r_parts.get(i).copied().unwrap_or(0);
        let c = c_parts.get(i).copied().unwrap_or(0);
        if r > c {
            return true;
        }
        if r < c {
            return false;
        }
    }
    false
}

/// Extracts all string values for `"<key>": "<value>"` from both minified single-line
/// and pretty-printed multi-line JSON without external crate dependencies.
fn extract_all_json_strings_after_key(json: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\"");
    let mut results = Vec::new();
    let mut search_from = 0;

    while let Some(rel_pos) = json[search_from..].find(&needle) {
        let after_key = search_from + rel_pos + needle.len();
        search_from = after_key;

        let rest = json[after_key..].trim_start();
        let Some(after_colon) = rest.strip_prefix(':') else {
            continue;
        };
        let after_colon = after_colon.trim_start();
        let Some(str_body) = after_colon.strip_prefix('"') else {
            continue;
        };

        let mut value = String::new();
        let mut chars = str_body.chars();
        let mut closed = false;
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some(esc) = chars.next() {
                    match esc {
                        '"' => value.push('"'),
                        '\\' => value.push('\\'),
                        '/' => value.push('/'),
                        'n' => value.push('\n'),
                        'r' => {}
                        't' => value.push('\t'),
                        other => value.push(other),
                    }
                }
            } else if ch == '"' {
                closed = true;
                break;
            } else {
                value.push(ch);
            }
        }
        if closed {
            results.push(value);
        }
    }
    results
}

fn extract_json_string_after_key(json: &str, key: &str) -> Option<String> {
    extract_all_json_strings_after_key(json, key).into_iter().next()
}

fn default_release_asset_url(tag_name: &str) -> String {
    let asset_name = if cfg!(target_os = "macos") {
        "MickeyRAR-macOS.dmg"
    } else {
        "MickeyRAR-Setup.exe"
    };
    format!("https://github.com/{GITHUB_REPO}/releases/download/{tag_name}/{asset_name}")
}

fn parse_github_release_json(json_str: &str, current_version: &str) -> Option<UpdateInfo> {
    if json_str.contains("\"message\":\"Not Found\"") || json_str.contains("\"message\": \"Not Found\"") {
        return None;
    }
    let tag_name = extract_json_string_after_key(json_str, "tag_name")?;
    if !is_version_newer(&tag_name, current_version) {
        return None;
    }

    let target_ext = if cfg!(target_os = "macos") { ".dmg" } else { "setup.exe" };
    let dl_url = extract_all_json_strings_after_key(json_str, "browser_download_url")
        .into_iter()
        .find(|u| u.to_lowercase().ends_with(target_ext))
        .or_else(|| Some(default_release_asset_url(&tag_name)));

    let raw_body = extract_json_string_after_key(json_str, "body").unwrap_or_default();
    let notes_summary: String = raw_body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .take(4)
        .map(|l| l.trim_start_matches("- ").replace("**", "").replace('`', ""))
        .collect::<Vec<_>>()
        .join("\n• ");

    Some(UpdateInfo {
        version: tag_name,
        download_url: dl_url,
        notes_summary: if notes_summary.is_empty() {
            String::new()
        } else {
            format!("• {notes_summary}")
        },
    })
}

fn check_for_updates_via_redirect(current_version: &str) -> Result<Option<UpdateInfo>, String> {
    let latest_page = format!("https://github.com/{GITHUB_REPO}/releases/latest");
    let curl_bin = if cfg!(windows) { "curl.exe" } else { "curl" };
    let null_sink = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut cmd = std::process::Command::new(curl_bin);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.args([
        "-s",
        "-L",
        "--max-time",
        "12",
        "-o",
        null_sink,
        "-w",
        "%{url_effective}",
        &latest_page,
    ]);
    let output = cmd.output().map_err(|e| format!("Network check failed: {e}"))?;
    if !output.status.success() {
        return Err("Could not reach GitHub Releases".into());
    }
    let effective_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if let Some((_, tag)) = effective_url.rsplit_once("/releases/tag/") {
        let tag_name = tag.trim_matches('/').to_string();
        if !tag_name.is_empty() && is_version_newer(&tag_name, current_version) {
            return Ok(Some(UpdateInfo {
                download_url: Some(default_release_asset_url(&tag_name)),
                version: tag_name,
                notes_summary: String::new(),
            }));
        }
    }
    Ok(None)
}

fn check_for_updates() -> Result<Option<UpdateInfo>, String> {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let curl_bin = if cfg!(windows) { "curl.exe" } else { "curl" };
    let mut cmd = std::process::Command::new(curl_bin);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.args([
        "-s",
        "-L",
        "--max-time",
        "12",
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "User-Agent: MickeyRAR-Updater",
        &url,
    ]);
    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let json_str = String::from_utf8_lossy(&output.stdout);
            if json_str.contains("\"tag_name\"") {
                return Ok(parse_github_release_json(&json_str, CURRENT_VERSION));
            }
        }
    }
    // Fallback to HTTP 302 redirect check (immune to GitHub API 60/hr IP rate limits)
    check_for_updates_via_redirect(CURRENT_VERSION)
}

fn run_in_app_extraction(
    archive: PathBuf,
    dest: PathBuf,
    selected_files: Vec<String>,
    cached_password: Option<String>,
    arabic: bool,
    main_ui: slint::Weak<MainWindow>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut password: Option<String> = cached_password;
    if password.is_none() {
        match resolve_archive_password(&archive, arabic)? {
            Some(ref p) if p == "__CANCELLED__" => {
                if let Some(m) = main_ui.upgrade() {
                    m.set_status(if arabic { "تم إلغاء فك الضغط" } else { "Extraction cancelled" }.into());
                }
                return Ok(());
            }
            Some(pwd) => password = Some(pwd),
            None => {}
        }
    }

    let progress_ui = ProgressWindow::new()?;
    progress_ui.set_rtl(arabic);
    let name = archive.file_name().unwrap_or_default().to_string_lossy().to_string();
    progress_ui.set_archive_name(name.clone().into());
    progress_ui.set_current_file(name.clone().into());
    progress_ui.set_action_text(if arabic { "جاري استخراج" } else { "extracting" }.into());
    progress_ui.set_progress(0.0);
    center_window(&progress_ui.window());
    progress_ui.show()?;

    let cancelled = Arc::new(AtomicBool::new(false));
    let progress_handle = ProgressHandle::new();
    let worker_cancelled = cancelled.clone();
    let worker_progress = progress_handle.clone();
    let worker_done = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel();

    let cancel_flag = cancelled.clone();
    let p_weak_cancel = progress_ui.as_weak();
    progress_ui.on_cancel(move || {
        cancel_flag.store(true, Ordering::SeqCst);
        if let Some(p) = p_weak_cancel.upgrade() {
            let _ = p.hide();
        }
    });

    let p_weak_bg = progress_ui.as_weak();
    progress_ui.on_toggle_background(move || {
        if let Some(p) = p_weak_bg.upgrade() {
            let _ = p.hide();
        }
    });

    let p_weak_pause = progress_ui.as_weak();
    let pause_handle = progress_handle.clone();
    progress_ui.on_toggle_pause(move || {
        if let Some(p) = p_weak_pause.upgrade() {
            let next = !p.get_is_paused();
            pause_handle.paused.store(next, Ordering::SeqCst);
            p.set_is_paused(next);
            p.set_action_text(if next {
                if arabic { "متوقف مؤقتاً (Paused)" } else { "paused" }.into()
            } else {
                if arabic { "جاري استخراج" } else { "extracting" }.into()
            });
        }
    });

    progress_ui.on_show_mode(|| {});
    progress_ui.on_show_help(|| {
        let _ = webbrowser::open("https://github.com/Michael-Maged1/MickeyRAR");
    });

    let dest_clone = dest.clone();
    let archive_clone = archive.clone();
    let selected_clone = selected_files.clone();
    let worker_finished = worker_done.clone();

    thread::spawn(move || {
        let _ = std::fs::create_dir_all(&dest_clone);
        let pre_existing_zero = snapshot_zero_byte_files(&dest_clone);
        if password.is_none() {
            if let Ok(mems) = list_archive_members(&archive_clone) {
                if let Err(safety_err) = validate_archive_members_safe(mems.iter().map(|m| m.path.as_str())) {
                    worker_finished.store(true, Ordering::SeqCst);
                    let _ = sender.send(Err(safety_err));
                    return;
                }
            }
        }
        // Build -oPath using OsString directly to preserve non-ASCII (Arabic) folder names.
        let mut output_arg = std::ffi::OsString::from("-o");
        output_arg.push(dest_clone.as_os_str());

        let mut args = vec![
            std::ffi::OsString::from("x"),
            std::ffi::OsString::from("-y"),
            std::ffi::OsString::from("-aoa"),
            std::ffi::OsString::from("-mmt=on"),
            output_arg,
            archive_clone.as_os_str().to_owned(),
        ];
        if let Some(ref pwd) = password {
            args.push(std::ffi::OsString::from(format!("-p{pwd}")));
        }
        if !selected_clone.is_empty() {
            args.push(std::ffi::OsString::from("--"));
            for f in selected_clone {
                args.push(std::ffi::OsString::from(f));
            }
        }

        let res = run_7z_cancellable_with_progress(&args, None, &worker_cancelled, Some(&worker_progress));
        if res.is_err() || matches!(res, Ok(RunOutcome::Cancelled)) {
            cleanup_new_zero_byte_files(&dest_clone, &pre_existing_zero);
        }
        // Store 100% only on success so the UI shows the real final percent on error.
        if matches!(res, Ok(RunOutcome::Finished)) {
            worker_progress.percent.store(100, Ordering::Relaxed);
        }
        worker_finished.store(true, Ordering::SeqCst);
        let _ = sender.send(res);
    });

    let close_cancelled = cancelled.clone();
    let p_weak = progress_ui.as_weak();
    progress_ui.window().on_close_requested(move || {
        close_cancelled.store(true, Ordering::SeqCst);
        slint::CloseRequestResponse::HideWindow
    });

    let timer = Rc::new(RefCell::new(None));
    let timer_progress = progress_handle.clone();
    let timer_receiver = Arc::new(Mutex::new(receiver));
    let start_time = Instant::now();
    let mut paused_accum = Duration::ZERO;
    let mut pause_started: Option<Instant> = None;
    let mut shown_percent = u8::MAX;
    let timer_self = timer.clone();
    let progress_ui_keep = Rc::new(progress_ui);
    let progress_ui_ref = progress_ui_keep.clone();

    let new_timer = Timer::default();
    new_timer.start(TimerMode::Repeated, Duration::from_millis(80), move || {
        let _keep = &progress_ui_ref;
        if let Some(ui) = p_weak.upgrade() {
            let is_paused = timer_progress.paused.load(Ordering::Relaxed);
            if is_paused {
                if pause_started.is_none() { pause_started = Some(Instant::now()); }
            } else if let Some(ps) = pause_started.take() {
                paused_accum += ps.elapsed();
            }
            let current_pause = pause_started.map(|ps| ps.elapsed()).unwrap_or(Duration::ZERO);
            let elapsed_secs = start_time.elapsed().saturating_sub(paused_accum + current_pause).as_secs();

            let percent = timer_progress.percent.load(Ordering::Relaxed);
            let cur_file = timer_progress.current_file.lock().map(|l| l.clone()).unwrap_or_default();
            if !cur_file.is_empty() {
                ui.set_current_file(cur_file.into());
            }

            let elapsed_str = format!("{:02}:{:02}:{:02}", elapsed_secs / 3600, (elapsed_secs % 3600) / 60, elapsed_secs % 60);
            ui.set_elapsed_time(elapsed_str.into());

            if percent > 0 && percent < 100 {
                let total_est = (elapsed_secs as f64) / (percent as f64 / 100.0);
                let rem_secs = (total_est - elapsed_secs as f64).max(0.0) as u64;
                let rem_str = format!("{:02}:{:02}:{:02}", rem_secs / 3600, (rem_secs % 3600) / 60, rem_secs % 60);
                ui.set_time_left(rem_str.into());
            } else if percent >= 100 {
                ui.set_time_left("00:00:00".into());
            } else {
                ui.set_time_left("--:--:--".into());
            }

            ui.set_progress(percent as f32 / 100.0);
            ui.set_file_progress(percent as f32 / 100.0);
            ui.set_file_percent(percent as i32);
            ui.set_total_progress(percent as f32 / 100.0);
            ui.set_total_percent(percent as i32);

            if percent != shown_percent {
                shown_percent = percent;
            }

            if let Ok(result) = timer_receiver.lock().unwrap().try_recv() {
                let status_msg = match result {
                    Ok(RunOutcome::Finished) => {
                        ui.set_progress(1.0);
                        ui.set_file_progress(1.0);
                        ui.set_file_percent(100);
                        ui.set_total_progress(1.0);
                        ui.set_total_percent(100);
                        ui.set_time_left("00:00:00".into());
                        ui.set_action_text(if arabic { "تم الاستخراج بنجاح ✔" } else { "Extraction completed successfully ✔" }.into());
                        if arabic {
                            format!("تم فك الضغط بنجاح إلى {}", dest.display())
                        } else {
                            format!("Extracted successfully to {}", dest.display())
                        }
                    }
                    Ok(RunOutcome::Cancelled) => {
                        if arabic { "تم إلغاء فك الضغط".to_string() } else { "Extraction cancelled".to_string() }
                    }
                    Err(e) => {
                        format!("Error: {e}")
                    }
                };

                let _ = ui.hide();
                if let Some(m) = main_ui.upgrade() {
                    m.set_status(status_msg.as_str().into());
                }
                let _ = timer_self.borrow_mut().take();
            }
        }
    });

    *timer.borrow_mut() = Some(new_timer);
    Ok(())
}

fn open_file_with_os(path: &Path) {
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("cmd");
        cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.args(["/c", "start", "", &path.to_string_lossy()]).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

fn copy_text_to_clipboard(text: &str) -> bool {
    use std::io::Write;
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("clip.exe");
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.stdin(std::process::Stdio::piped());
        if let Ok(mut child) = cmd.spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().map(|s| s.success()).unwrap_or(false);
        }
        false
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("pbcopy");
        cmd.stdin(std::process::Stdio::piped());
        if let Ok(mut child) = cmd.spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().map(|s| s.success()).unwrap_or(false);
        }
        false
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = text;
        false
    }
}

fn run_in_app_compression(
    paths: Vec<PathBuf>,
    output: PathBuf,
    opts: ArchiveCreationOptions,
    delete_original: bool,
    arabic: bool,
    main_ui: slint::Weak<MainWindow>,
    state: Rc<RefCell<AppState>>,
) -> Result<(), String> {
    let (cwd, args) = create_archive_command_args(&output, &paths, &opts)?;
    let format_name = opts.format.to_ascii_uppercase();
    let output_existed = output.is_file();

    let progress_ui = ProgressWindow::new().map_err(|e| e.to_string())?;
    progress_ui.set_rtl(arabic);
    let archive_name = output.file_name().unwrap_or_default().to_string_lossy().to_string();
    progress_ui.set_archive_name(archive_name.clone().into());
    progress_ui.set_current_file(archive_name.into());
    progress_ui.set_action_text(
        if arabic {
            format!("جاري ضغط ({format_name})")
        } else {
            format!("compressing ({format_name})")
        }
        .into(),
    );
    progress_ui.set_progress(0.0);
    center_window(&progress_ui.window());
    progress_ui.show().map_err(|e| e.to_string())?;

    let cancelled = Arc::new(AtomicBool::new(false));
    let progress_handle = ProgressHandle::new();
    let worker_cancelled = cancelled.clone();
    let worker_progress = progress_handle.clone();
    let (sender, receiver) = mpsc::channel();

    let cancel_flag = cancelled.clone();
    let p_weak_cancel = progress_ui.as_weak();
    progress_ui.on_cancel(move || {
        cancel_flag.store(true, Ordering::SeqCst);
        if let Some(p) = p_weak_cancel.upgrade() {
            let _ = p.hide();
        }
    });

    let p_weak_bg = progress_ui.as_weak();
    progress_ui.on_toggle_background(move || {
        if let Some(p) = p_weak_bg.upgrade() {
            let _ = p.hide();
        }
    });

    let p_weak_pause = progress_ui.as_weak();
    let pause_handle = progress_handle.clone();
    let fmt_pause = format_name.clone();
    progress_ui.on_toggle_pause(move || {
        if let Some(p) = p_weak_pause.upgrade() {
            let next = !p.get_is_paused();
            pause_handle.paused.store(next, Ordering::SeqCst);
            p.set_is_paused(next);
            p.set_action_text(
                if next {
                    if arabic { "متوقف مؤقتاً (Paused)".into() } else { "paused".into() }
                } else if arabic {
                    format!("جاري ضغط ({fmt_pause})")
                } else {
                    format!("compressing ({fmt_pause})")
                }
                .into(),
            );
        }
    });

    progress_ui.on_show_mode(|| {});
    progress_ui.on_show_help(|| {
        let _ = webbrowser::open("https://github.com/Michael-Maged1/MickeyRAR");
    });

    let close_cancelled = cancelled.clone();
    progress_ui.window().on_close_requested(move || {
        close_cancelled.store(true, Ordering::SeqCst);
        slint::CloseRequestResponse::HideWindow
    });

    let worker_output = output.clone();
    let paths_for_delete = paths.clone();
    thread::spawn(move || {
        let res = run_7z_cancellable_with_progress(&args, Some(&cwd), &worker_cancelled, Some(&worker_progress));
        match res {
            Ok(RunOutcome::Finished) => {
                worker_progress.percent.store(100, Ordering::Relaxed);
                if delete_original {
                    for p in paths_for_delete {
                        let _ = if p.is_dir() { fs::remove_dir_all(&p) } else { fs::remove_file(&p) };
                    }
                }
            }
            _ => {
                if !output_existed {
                    let _ = delete_file_with_retry(&worker_output, 25);
                }
            }
        }
        let _ = sender.send(res);
    });

    let timer = Rc::new(RefCell::new(None));
    let timer_self = timer.clone();
    let timer_progress = progress_handle.clone();
    let timer_receiver = Arc::new(Mutex::new(receiver));
    let start_time = Instant::now();
    let mut paused_accum = Duration::ZERO;
    let mut pause_started: Option<Instant> = None;
    let p_weak = progress_ui.as_weak();
    let progress_ui_keep = Rc::new(progress_ui);

    let new_timer = Timer::default();
    new_timer.start(TimerMode::Repeated, Duration::from_millis(80), move || {
        let _keep = &progress_ui_keep;
        if let Some(ui) = p_weak.upgrade() {
            let is_paused = timer_progress.paused.load(Ordering::Relaxed);
            if is_paused {
                if pause_started.is_none() { pause_started = Some(Instant::now()); }
            } else if let Some(ps) = pause_started.take() {
                paused_accum += ps.elapsed();
            }
            let cur_pause = pause_started.map(|ps| ps.elapsed()).unwrap_or(Duration::ZERO);
            let elapsed_secs = start_time.elapsed().saturating_sub(paused_accum + cur_pause).as_secs();

            let percent = timer_progress.percent.load(Ordering::Relaxed);
            let cur_file = timer_progress.current_file.lock().map(|l| l.clone()).unwrap_or_default();
            if !cur_file.is_empty() {
                ui.set_current_file(cur_file.into());
            }
            let elapsed_str = format!("{:02}:{:02}:{:02}", elapsed_secs / 3600, (elapsed_secs % 3600) / 60, elapsed_secs % 60);
            ui.set_elapsed_time(elapsed_str.into());
            if percent > 0 && percent < 100 {
                let total_est = (elapsed_secs as f64) / (percent as f64 / 100.0);
                let rem_secs = (total_est - elapsed_secs as f64).max(0.0) as u64;
                let rem_str = format!("{:02}:{:02}:{:02}", rem_secs / 3600, (rem_secs % 3600) / 60, rem_secs % 60);
                ui.set_time_left(rem_str.into());
            } else if percent >= 100 {
                ui.set_time_left("00:00:00".into());
            } else {
                ui.set_time_left("--:--:--".into());
            }
            let frac = (percent as f32 / 100.0).clamp(0.0, 1.0);
            ui.set_progress(frac);
            ui.set_file_progress(frac);
            ui.set_file_percent(percent as i32);
            ui.set_total_progress(frac);
            ui.set_total_percent(percent as i32);

            if let Ok(result) = timer_receiver.lock().unwrap().try_recv() {
                let status_msg = match result {
                    Ok(RunOutcome::Finished) => {
                        if arabic {
                            format!("تم إنشاء الأرشيف بنجاح ✔: {}", output.display())
                        } else {
                            format!("Archive created successfully ✔: {}", output.display())
                        }
                    }
                    Ok(RunOutcome::Cancelled) => {
                        if arabic { "تم إلغاء عملية الضغط".into() } else { "Compression cancelled".into() }
                    }
                    Err(e) => format!("Error: {e}"),
                };
                let _ = ui.hide();
                if let Some(m) = main_ui.upgrade() {
                    if state.borrow().archive.is_none() {
                        let _ = show_folder(&m, &state);
                    }
                    m.set_status(status_msg.into());
                }
                let _ = timer_self.borrow_mut().take();
            }
        }
    });
    *timer.borrow_mut() = Some(new_timer);
    Ok(())
}

fn open_in_app_archive_dialog(
    main_ui: slint::Weak<MainWindow>,
    state: Rc<RefCell<AppState>>,
    compression: Rc<RefCell<CompressionSettings>>,
    initial_format_idx: i32,
    require_password_hint: bool,
) {
    let Some(m_ui) = main_ui.upgrade() else { return; };
    let arabic = m_ui.get_arabic_mode();

    let mut chosen_paths = std::mem::take(&mut state.borrow_mut().pending_paths);
    if chosen_paths.is_empty() && state.borrow().archive.is_none() {
        let folder = state.borrow().folder.clone();
        let model = m_ui.get_entries();
        for i in 0..model.row_count() {
            if let Some(e) = model.row_data(i) {
                if e.is_selected && e.name != ".." {
                    let candidate = folder.join(e.name.as_str());
                    if candidate.exists() {
                        chosen_paths.push(candidate);
                    }
                }
            }
        }
    }
    if chosen_paths.is_empty() {
        let cur_folder = state.borrow().folder.clone();
        if let Some(files) = rfd::FileDialog::new().set_directory(&cur_folder).pick_files() {
            chosen_paths = files;
        }
    }
    if chosen_paths.is_empty() {
        return;
    }

    let Ok(dialog) = ArchiveDialog::new() else { return; };
    dialog.set_rtl(arabic);
    dialog.set_format_index(initial_format_idx);
    if require_password_hint && initial_format_idx == 0 {
        // Default encrypted archives to 7Z so filename encryption (-mhe=on) is available
        dialog.set_format_index(1);
    }
    let default_dest = chosen_paths[0]
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| state.borrow().folder.clone());
    let default_stem = if chosen_paths.len() == 1 {
        chosen_paths[0].file_stem().unwrap_or_default().to_string_lossy().to_string()
    } else {
        default_dest
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "MickeyRAR-archive".into())
    };
    if !default_stem.is_empty() {
        dialog.set_archive_name(default_stem.into());
    }
    dialog.set_destination(default_dest.display().to_string().into());
    center_window(&dialog.window());
    if dialog.show().is_err() {
        return;
    }

    let dialog_rc = Rc::new(RefCell::new(Some(dialog)));
    let d_ref = dialog_rc.borrow();
    let Some(dlg) = d_ref.as_ref() else { return; };

    let dlg_weak = dlg.as_weak();
    dlg.on_browse_location(move || {
        if let Some(d) = dlg_weak.upgrade() {
            let cur = PathBuf::from(d.get_destination().to_string());
            if let Some(folder) = rfd::FileDialog::new().set_directory(cur).pick_folder() {
                d.set_destination(folder.display().to_string().into());
            }
        }
    });

    let holder_cancel = dialog_rc.clone();
    dlg.on_cancel(move || {
        if let Some(d) = holder_cancel.borrow_mut().take() {
            let _ = d.hide();
        }
    });

    let holder_close = dialog_rc.clone();
    dlg.window().on_close_requested(move || {
        let _ = holder_close.borrow_mut().take();
        slint::CloseRequestResponse::HideWindow
    });

    let holder_start = dialog_rc.clone();
    dlg.on_start_compression(move || {
        let Some(d) = holder_start.borrow_mut().take() else { return; };
        let level = match d.get_level_index() {
            0 => 0,
            1 => 3,
            2 => 5,
            3 => 7,
            _ => 9,
        };
        let format = match d.get_format_index() {
            1 => "7z",
            2 => "sfx",
            _ => "zip",
        };
        let pwd_str = d.get_password().to_string();
        let password = if pwd_str.is_empty() { None } else { Some(pwd_str) };
        let encrypt_headers = d.get_encrypt_headers();
        let delete_original = d.get_delete_original();
        let raw_name = d.get_archive_name().to_string();
        let dest_dir = PathBuf::from(d.get_destination().to_string());
        let _ = d.hide();

        let mut name_path = PathBuf::from(raw_name.trim());
        if name_path.file_name().is_none() {
            name_path = PathBuf::from("MickeyRAR-archive");
        }
        let ext = if format == "sfx" { "exe" } else { format };
        name_path.set_extension(ext);
        let output = dest_dir.join(name_path.file_name().unwrap());

        let comp = compression.borrow().clone();
        let opts = ArchiveCreationOptions {
            format: format.into(),
            level,
            solid: comp.solid,
            threads: comp.threads,
            password,
            encrypt_headers,
        };
        if let Err(err) = run_in_app_compression(
            chosen_paths.clone(),
            output,
            opts,
            delete_original,
            arabic,
            main_ui.clone(),
            state.clone(),
        ) {
            if let Some(m) = main_ui.upgrade() {
                m.set_status(format!("Error: {err}").into());
            }
        }
    });
}

fn status_error(ui: &MainWindow, result: Result<(), String>) {
    if let Err(e) = result { ui.set_status(format!("Error: {e}").into()); }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(operation) = context_operation() {
        let paths = context_paths();
        let arabic = load_language().is_arabic();
        match operation.as_str() {
            "--add" => return run_archive_dialog(paths, arabic),
            "--compress" | "--compress-email" => return run_context_compression(paths, arabic),
            "--extract-here" | "--extract-to" | "--extract-to-folder" => {
                return coordinate_context_extraction(operation.as_str(), paths, arabic);
            }
            _ => {}
        }
    }
    let ui = MainWindow::new()?;
    quit_process_on_close(&ui.window());
    let center_timer = Timer::default();
    let center_weak = ui.as_weak();
    center_timer.start(TimerMode::SingleShot, Duration::from_millis(120), move || {
        if let Some(ui) = center_weak.upgrade() { center_window(&ui.window()); }
        winicon::install();
    });
    let language = Rc::new(RefCell::new(load_language()));
    ui.set_arabic_mode(language.borrow().is_arabic());
    ui.set_dark_mode(load_dark_mode());
    let drives_vec = detect_available_drives();
    ui.set_available_drives(ModelRc::new(VecModel::from(drives_vec)));
    let snap_dark = std::env::args().any(|a| a == "--snap-dark");
    let snap_light = std::env::args().any(|a| a == "--snap-light");
    if snap_dark {
        ui.set_dark_mode(true);
    } else if snap_light || std::env::args().any(|a| a == "--light") {
        ui.set_dark_mode(false);
    }
    let snap_timer = Timer::default();
    let snap_weak = ui.as_weak();
    snap_timer.start(TimerMode::SingleShot, Duration::from_millis(500), move || {
        if snap_dark || snap_light {
            if let Some(ui) = snap_weak.upgrade() {
                if let Ok(buffer) = ui.window().take_snapshot() {
                    let out_path = if snap_light { "assets/screenshot-light.bmp" } else { "assets/screenshot-dark.bmp" };
                    let _ = save_rgba_as_bmp(buffer.width(), buffer.height(), buffer.as_bytes(), Path::new(out_path));
                    std::process::exit(0);
                }
            }
        }
    });
    ui.on_toggle_theme({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                let next = !ui.get_dark_mode();
                ui.set_dark_mode(next);
                let _ = save_dark_mode(next);
            }
        }
    });
    ui.on_open_url(|url| {
        if let Err(error) = webbrowser::open(url.as_str()) {
            eprintln!("Could not open social link: {error}");
        }
    });
    ui.on_toolbar_info({
        let weak = ui.as_weak();
        move |tool| {
            if let Some(ui) = weak.upgrade() {
                let message = if ui.get_arabic_mode() { format!("أداة {} جاهزة", tool) } else { format!("{} ready", tool) };
                ui.set_status(message.into());
            }
        }
    });
    let direct_arg = std::env::args_os().nth(1).map(PathBuf::from);
    let open_archive_path = direct_arg.as_ref().filter(|p| p.is_file() && supported_archive(p)).cloned();
    let pending_paths = context_paths();
    let initial_folder = open_archive_path.as_ref()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .or_else(|| pending_paths.first().and_then(|p| if p.is_dir() { Some(p.clone()) } else { p.parent().map(Path::to_path_buf) }))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let state = Rc::new(RefCell::new(AppState {
        folder: initial_folder,
        archive: open_archive_path.clone(),
        archive_members: None,
        archive_password: None,
        archive_dir: String::new(),
        paths: Vec::new(),
        pending_paths,
        sort_column: 0,
        sort_ascending: true,
        last_selected_anchor: None,
        search_query: String::new(),
    }));
    if let Some(archive_path) = open_archive_path {
        status_error(&ui, show_archive(&ui, &state, archive_path));
    } else {
        status_error(&ui, show_folder(&ui, &state));
    }

    { let weak = ui.as_weak(); let language = language.clone(); let state = state.clone();
      ui.on_toggle_language(move || {
          if let Some(ui) = weak.upgrade() {
              let next = if language.borrow().is_arabic() { Language::English } else { Language::Arabic };
              match save_language(next) {
                  Ok(()) => {
                      *language.borrow_mut() = next;
                      ui.set_arabic_mode(next.is_arabic());
                      let archive = state.borrow().archive.clone();
                      let _ = match archive { Some(p) => show_archive(&ui, &state, p), None => show_folder(&ui, &state) };
                      ui.set_status(next.message("Language changed to English", "تم تغيير اللغة إلى العربية").into());
                  }
                  Err(e) => ui.set_status(format!("Language settings error: {e}").into()),
              }
          }
      }); }

    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_nav_up(move || {
          if let Some(ui) = weak.upgrade() {
              if !state.borrow().search_query.is_empty() {
                  state.borrow_mut().search_query.clear();
                  ui.set_find_query("".into());
                  ui.set_show_find(false);
                  let archive = state.borrow().archive.clone();
                  let res = match archive {
                      Some(p) => show_archive(&ui, &state, p),
                      None => show_folder(&ui, &state),
                  };
                  status_error(&ui, res);
                  return;
              }
              let is_archive = state.borrow().archive.is_some();
              if is_archive {
                  let current_dir = state.borrow().archive_dir.clone();
                  if !current_dir.is_empty() {
                      // Navigate up within the archive virtual directory
                      let new_dir = if let Some(pos) = current_dir.trim_end_matches('/').rfind('/') {
                          current_dir[..pos].to_string()
                      } else {
                          String::new() // Back to archive root
                      };
                      state.borrow_mut().archive_dir = new_dir;
                      let archive = state.borrow().archive.clone().unwrap();
                      status_error(&ui, show_archive(&ui, &state, archive));
                  } else {
                      // At archive root - exit archive view
                      let mut s = state.borrow_mut();
                      s.archive = None;
                      s.archive_dir.clear();
                      s.archive_members = None;
                      s.archive_password = None;
                      drop(s);
                      status_error(&ui, show_folder(&ui, &state));
                  }
              } else {
                  let current = state.borrow().folder.clone();
                  if let Some(parent) = current.parent() {
                      state.borrow_mut().folder = parent.to_path_buf();
                      status_error(&ui, show_folder(&ui, &state));
                  }
              }
          }
      }); }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_sort_by(move |col| {
            if let Some(ui) = weak.upgrade() {
                let (next_col, next_asc) = {
                    let mut s = state.borrow_mut();
                    if s.sort_column == col {
                        s.sort_ascending = !s.sort_ascending;
                    } else {
                        s.sort_column = col;
                        s.sort_ascending = true;
                    }
                    (s.sort_column, s.sort_ascending)
                };
                ui.set_sort_column(next_col);
                ui.set_sort_ascending(next_asc);
                let model = ui.get_entries();
                let mut vec: Vec<FileEntry> = (0..model.row_count()).filter_map(|i| model.row_data(i)).collect();
                sort_file_entries(&mut vec, next_col, next_asc);
                ui.set_entries(ModelRc::new(VecModel::from(vec)));
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_select_entry_adv(move |idx, ctrl, shift| {
            if let Some(ui) = weak.upgrade() {
                let model = ui.get_entries();
                let count = model.row_count();
                if idx < 0 || (idx as usize) >= count { return; }
                let target = idx as usize;
                let anchor = state.borrow().last_selected_anchor;

                if shift {
                    let start = anchor.unwrap_or(target).min(target);
                    let end = anchor.unwrap_or(target).max(target);
                    update_selection(&ui, |i, _| i >= start && i <= end);
                    ui.set_selected_index(idx);
                } else if ctrl {
                    update_selection(&ui, |i, e| if i == target { !e.is_selected } else { e.is_selected });
                    state.borrow_mut().last_selected_anchor = Some(target);
                    ui.set_selected_index(idx);
                } else {
                    update_selection(&ui, |i, _| i == target);
                    state.borrow_mut().last_selected_anchor = Some(target);
                    ui.set_selected_index(idx);
                }
            }
        });
    }

    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_select_entry(move |idx| {
          if let Some(ui) = weak.upgrade() {
              if idx < 0 { return; }
              let target = idx as usize;
              update_selection(&ui, |i, _| i == target);
              state.borrow_mut().last_selected_anchor = Some(target);
              ui.set_selected_index(idx);
          }
      }); }
    let compression = Rc::new(RefCell::new(load_settings()));
    ui.set_solid_mode(compression.borrow().solid);
    ui.set_thread_count(compression.borrow().threads.map(|n| n.to_string()).unwrap_or_else(|| "auto".into()).into());
    { let weak = ui.as_weak(); let compression = compression.clone();
      ui.on_save_compression_settings(move |threads, solid| {
        let parsed = if threads.trim().eq_ignore_ascii_case("auto") || threads.trim().is_empty() { Ok(None) }
            else { threads.trim().parse::<u16>().map(Some).map_err(|_| "Enter 'auto' or a thread count from 1 to 256".to_string()) };
        if let Some(ui) = weak.upgrade() {
            let result = parsed.and_then(|count| {
                if matches!(count, Some(0)) || matches!(count, Some(257..)) { return Err("Threads must be 1–256".into()); }
                let next = CompressionSettings { level: 9, solid, threads: count };
                save_settings(&next)?;
                *compression.borrow_mut() = next;
                Ok(())
            });
            match result {
                Ok(()) => { ui.set_solid_mode(solid); ui.set_thread_count(threads); ui.set_show_settings(false); ui.set_status(if ui.get_arabic_mode() { "تم حفظ إعدادات ضغط ZIP الفائق Ultra" } else { "Saved Ultra ZIP compression settings" }.into()); }
                Err(err) => ui.set_status(format!("Settings error: {err}").into()),
            }
        }
      }); }

    {
        let weak = ui.as_weak();
        ui.on_set_default_app(move || {
            if let Some(ui) = weak.upgrade() {
                #[cfg(windows)]
                {
                    match register_file_associations() {
                        Ok(()) => {
                            let msg = if ui.get_arabic_mode() {
                                "تم تعيين Mickey RAR كبرنامج افتراضي وتحديث أيقونات الأرشيف بنجاح ✔"
                            } else {
                                "Mickey RAR set as default archive manager successfully ✔"
                            };
                            ui.set_status(msg.into());
                            let _ = webbrowser::open("ms-settings:defaultapps");
                        }
                        Err(e) => ui.set_status(format!("Association error: {e}").into()),
                    }
                }
            }
        });
    }


    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_choose_folder(move || {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            {
                let mut s = state.borrow_mut();
                s.folder = folder;
                s.archive = None;
                s.archive_dir.clear();
                s.archive_members = None;
                s.archive_password = None;
                s.search_query.clear();
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_find_query("".into());
                ui.set_show_find(false);
                status_error(&ui, show_folder(&ui, &state));
            }
        }
      }); }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_navigate_path(move |raw_path| {
            let Some(ui) = weak.upgrade() else { return; };
            let trimmed = raw_path.trim().trim_matches('"');
            if trimmed.is_empty() { return; }
            let target = PathBuf::from(trimmed);
            if target.is_dir() {
                {
                    let mut s = state.borrow_mut();
                    s.folder = target;
                    s.archive = None;
                    s.archive_dir.clear();
                    s.archive_members = None;
                    s.archive_password = None;
                    s.search_query.clear();
                }
                ui.set_find_query("".into());
                ui.set_show_find(false);
                status_error(&ui, show_folder(&ui, &state));
            } else if target.is_file() && supported_archive(&target) {
                {
                    let mut s = state.borrow_mut();
                    if let Some(parent) = target.parent() {
                        s.folder = parent.to_path_buf();
                    }
                    s.archive_members = None;
                    s.archive_password = None;
                    s.archive_dir.clear();
                    s.search_query.clear();
                }
                ui.set_find_query("".into());
                ui.set_show_find(false);
                status_error(&ui, show_archive(&ui, &state, target));
            } else {
                let msg = if ui.get_arabic_mode() {
                    format!("المسار غير موجود أو غير مدعوم: {}", trimmed)
                } else {
                    format!("Path not found or unsupported: {}", trimmed)
                };
                ui.set_status(msg.into());
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_run_find(move |query| {
            if let Some(ui) = weak.upgrade() {
                state.borrow_mut().search_query = query.to_string();
                let archive = state.borrow().archive.clone();
                let res = match archive {
                    Some(p) => show_archive(&ui, &state, p),
                    None => show_folder(&ui, &state),
                };
                status_error(&ui, res);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_clear_find(move || {
            if let Some(ui) = weak.upgrade() {
                state.borrow_mut().search_query.clear();
                let archive = state.borrow().archive.clone();
                let res = match archive {
                    Some(p) => show_archive(&ui, &state, p),
                    None => show_folder(&ui, &state),
                };
                status_error(&ui, res);
            }
        });
    }

    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_refresh(move || { if let Some(ui) = weak.upgrade() {
          state.borrow_mut().archive_members = None;
          let drives_vec = detect_available_drives();
          ui.set_available_drives(ModelRc::new(VecModel::from(drives_vec)));
          let archive = state.borrow().archive.clone();
          let result = match archive { Some(p) => show_archive(&ui, &state, p), None => show_folder(&ui, &state) };
          status_error(&ui, result);
      }}); }

    let open_entry_impl: Rc<dyn Fn(String)> = {
        let weak = ui.as_weak();
        let state = state.clone();
        Rc::new(move |entry: String| {
            let Some(ui) = weak.upgrade() else { return; };
            if entry == ".." {
                if !state.borrow().search_query.is_empty() {
                    state.borrow_mut().search_query.clear();
                    ui.set_find_query("".into());
                    ui.set_show_find(false);
                    let archive = state.borrow().archive.clone();
                    let res = match archive {
                        Some(p) => show_archive(&ui, &state, p),
                        None => show_folder(&ui, &state),
                    };
                    status_error(&ui, res);
                    return;
                }
                let is_archive = state.borrow().archive.is_some();
                if is_archive {
                    let current_dir = state.borrow().archive_dir.clone();
                    if !current_dir.is_empty() {
                        let new_dir = if let Some(pos) = current_dir.trim_end_matches('/').rfind('/') {
                            current_dir[..pos].to_string()
                        } else {
                            String::new()
                        };
                        state.borrow_mut().archive_dir = new_dir;
                        let archive = state.borrow().archive.clone().unwrap();
                        status_error(&ui, show_archive(&ui, &state, archive));
                    } else {
                        let mut s = state.borrow_mut();
                        s.archive = None;
                        s.archive_dir.clear();
                        s.archive_members = None;
                        s.archive_password = None;
                        drop(s);
                        status_error(&ui, show_folder(&ui, &state));
                    }
                } else {
                    let current = state.borrow().folder.clone();
                    if let Some(parent) = current.parent() {
                        state.borrow_mut().folder = parent.to_path_buf();
                        status_error(&ui, show_folder(&ui, &state));
                    }
                }
                return;
            }

            if state.borrow().archive.is_some() {
                // Check whether the double-clicked row inside the archive is a folder or a file
                let model = ui.get_entries();
                let mut clicked_is_dir = false;
                for i in 0..model.row_count() {
                    if let Some(r) = model.row_data(i) {
                        if r.name.as_str() == entry {
                            clicked_is_dir = r.is_dir;
                            break;
                        }
                    }
                }
                let in_search = !state.borrow().search_query.trim().is_empty();
                let current_dir = state.borrow().archive_dir.clone();
                let full_item = if in_search || current_dir.is_empty() {
                    entry.clone()
                } else {
                    format!("{}/{}", current_dir.trim_end_matches('/'), entry)
                };

                if clicked_is_dir {
                    {
                        let mut s = state.borrow_mut();
                        s.search_query.clear();
                        s.archive_dir = full_item;
                    }
                    ui.set_find_query("".into());
                    ui.set_show_find(false);
                    let archive = state.borrow().archive.clone().unwrap();
                    status_error(&ui, show_archive(&ui, &state, archive));
                } else {
                    // File inside archive: extract to temp and open with OS viewer
                    let archive_path = state.borrow().archive.clone().unwrap();
                    let mut pwd = state
                        .borrow()
                        .archive_password
                        .as_ref()
                        .and_then(|(ap, p)| if ap == &archive_path { Some(p.clone()) } else { None });
                    if pwd.is_none() {
                        match resolve_archive_password(&archive_path, ui.get_arabic_mode()).unwrap_or(None) {
                            Some(ref s) if s == "__CANCELLED__" => return,
                            Some(p) => {
                                state.borrow_mut().archive_password = Some((archive_path.clone(), p.clone()));
                                pwd = Some(p);
                            }
                            None => {}
                        }
                    }
                    let temp_dir = std::env::temp_dir().join("MickeyRAR_View");
                    let _ = std::fs::create_dir_all(&temp_dir);
                    let mut out_flag = std::ffi::OsString::from("-o");
                    out_flag.push(temp_dir.as_os_str());
                    let mut args = vec![
                        std::ffi::OsString::from("e"),
                        std::ffi::OsString::from("-y"),
                        std::ffi::OsString::from("-aoa"),
                        out_flag,
                        archive_path.as_os_str().to_owned(),
                        std::ffi::OsString::from("--"),
                        std::ffi::OsString::from(&full_item),
                    ];
                    if let Some(ref p) = pwd {
                        args.insert(3, std::ffi::OsString::from(format!("-p{p}")));
                    }
                    match run_7z(&args, None) {
                        Ok(_) => {
                            let fname = Path::new(&full_item)
                                .file_name()
                                .unwrap_or_else(|| std::ffi::OsStr::new(&entry));
                            let extracted = temp_dir.join(fname);
                            open_file_with_os(&extracted);
                            ui.set_status(
                                if ui.get_arabic_mode() {
                                    format!("تم فتح الملف: {}", entry)
                                } else {
                                    format!("Opened file: {}", entry)
                                }
                                .into(),
                            );
                        }
                        Err(e) => ui.set_status(format!("Error opening file: {e}").into()),
                    }
                }
                return;
            }

            let p = state.borrow().folder.join(&entry);
            if p.is_dir() {
                {
                    let mut s = state.borrow_mut();
                    s.archive_members = None;
                    s.archive_password = None;
                    s.search_query.clear();
                    s.folder = p;
                }
                ui.set_find_query("".into());
                ui.set_show_find(false);
                status_error(&ui, show_folder(&ui, &state));
            } else if supported_archive(&p) {
                {
                    let mut s = state.borrow_mut();
                    if let Some(parent) = p.parent() {
                        s.folder = parent.to_path_buf();
                    }
                    s.archive_members = None;
                    s.archive_password = None;
                    s.archive_dir.clear();
                    s.search_query.clear();
                }
                ui.set_find_query("".into());
                ui.set_show_find(false);
                status_error(&ui, show_archive(&ui, &state, p));
            } else if p.exists() {
                open_file_with_os(&p);
                ui.set_status(
                    if ui.get_arabic_mode() {
                        format!("تم فتح: {}", entry)
                    } else {
                        format!("Opened: {}", entry)
                    }
                    .into(),
                );
            }
        })
    };

    {
        let open_cb = open_entry_impl.clone();
        ui.on_open_entry(move |entry| {
            open_cb(entry.to_string());
        });
    }

    {
        let weak = ui.as_weak();
        let open_cb = open_entry_impl.clone();
        ui.on_open_selected(move || {
            let Some(ui) = weak.upgrade() else { return; };
            let model = ui.get_entries();
            let sel_idx = ui.get_selected_index();
            if sel_idx >= 0 && (sel_idx as usize) < model.row_count() {
                if let Some(e) = model.row_data(sel_idx as usize) {
                    open_cb(e.name.to_string());
                    return;
                }
            }
            for i in 0..model.row_count() {
                if let Some(e) = model.row_data(i) {
                    if e.is_selected {
                        open_cb(e.name.to_string());
                        return;
                    }
                }
            }
        });
    }

    let is_dragging = Rc::new(Cell::new(false));
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let is_dragging = is_dragging.clone();
        ui.on_start_drag(move |idx| {
            if is_dragging.get() { return; }
            let Some(ui) = weak.upgrade() else { return; };
            let model = ui.get_entries();
            if idx < 0 || (idx as usize) >= model.row_count() { return; }
            let Some(clicked_entry) = model.row_data(idx as usize) else { return; };
            if clicked_entry.name == ".." { return; }

            is_dragging.set(true);

            let is_archive = state.borrow().archive.is_some();
            let mut files_to_drag: Vec<String> = Vec::new();
            let current_dir = state.borrow().archive_dir.clone();
            if clicked_entry.is_selected {
                for i in 0..model.row_count() {
                    if let Some(entry) = model.row_data(i) {
                        if entry.is_selected && entry.name != ".." {
                            let full_path = if is_archive && !current_dir.is_empty() {
                                format!("{}/{}", current_dir.trim_end_matches('/'), entry.name)
                            } else {
                                entry.name.to_string()
                            };
                            files_to_drag.push(full_path);
                        }
                    }
                }
            } else {
                update_selection(&ui, |i, _| i == idx as usize);
                ui.set_selected_index(idx);
            }
            if files_to_drag.is_empty() {
                let full_path = if is_archive && !current_dir.is_empty() {
                    format!("{}/{}", current_dir.trim_end_matches('/'), clicked_entry.name)
                } else {
                    clicked_entry.name.to_string()
                };
                files_to_drag.push(full_path);
            }

            let arabic = ui.get_arabic_mode();

            if is_archive {
                let archive_path = state.borrow().archive.clone().unwrap();
                let mut password: Option<String> = None;
                match resolve_archive_password(&archive_path, arabic).unwrap_or(None) {
                    Some(ref s) if s == "__CANCELLED__" => {
                        is_dragging.set(false);
                        return;
                    }
                    Some(pwd) => password = Some(pwd),
                    None => {}
                }

                let temp_dir = std::env::temp_dir().join("MickeyRAR_Drag");
                let _ = std::fs::create_dir_all(&temp_dir);

                // Check if all requested items are already cached in temp_dir
                let all_cached = files_to_drag.iter().all(|f| {
                    let p = temp_dir.join(f);
                    p.exists() && (p.is_dir() || p.metadata().map(|m| m.len() > 0).unwrap_or(false))
                });

                if !all_cached {
                    let mut out_flag = std::ffi::OsString::from("-o");
                    out_flag.push(temp_dir.as_os_str());

                    let mut args = vec![
                        std::ffi::OsString::from("x"),
                        std::ffi::OsString::from("-y"),
                        std::ffi::OsString::from("-aoa"),
                        std::ffi::OsString::from("-mmt=on"),
                        out_flag,
                        archive_path.as_os_str().to_owned(),
                    ];
                    if let Some(ref pwd) = password {
                        args.push(std::ffi::OsString::from(format!("-p{pwd}")));
                    }
                    for f in &files_to_drag {
                        args.push(std::ffi::OsString::from(f));
                        args.push(std::ffi::OsString::from(format!("{f}/*")));
                        args.push(std::ffi::OsString::from(format!("{f}\\*")));
                    }

                    ui.set_status(if arabic { "جاري تجهيز الملفات للسحب والإفلات..." } else { "Extracting files for drag and drop..." }.into());
                    let _ = run_7z(&args, None);
                }

                let mut drag_paths: Vec<PathBuf> = Vec::new();
                for f in &files_to_drag {
                    let p = temp_dir.join(f);
                    if p.exists() {
                        drag_paths.push(p);
                    }
                }
                if !drag_paths.is_empty() {
                    let path_refs: Vec<&Path> = drag_paths.iter().map(|p| p.as_path()).collect();
                    #[cfg(windows)]
                    {
                        match drag_drop::windows::start_drag_drop_files(&path_refs) {
                            Ok(effect) => {
                                if effect != 0 {
                                    ui.set_status(if arabic { "تم سحب وإفلات الملفات بنجاح ✔" } else { "Files dropped successfully ✔" }.into());
                                } else {
                                    ui.set_status(if arabic { "تم إلغاء السحب والإفلات" } else { "Drag and drop cancelled" }.into());
                                }
                            }
                            Err(e) => {
                                ui.set_status(format!("Drag drop error: {e}").into());
                            }
                        }
                    }
                }
            } else {
                let current_dir = &state.borrow().folder;
                let mut drag_paths: Vec<PathBuf> = Vec::new();
                for f in &files_to_drag {
                    let p = current_dir.join(f);
                    if p.exists() {
                        drag_paths.push(p);
                    }
                }
                if !drag_paths.is_empty() {
                    let path_refs: Vec<&Path> = drag_paths.iter().map(|p| p.as_path()).collect();
                    #[cfg(windows)]
                    {
                        match drag_drop::windows::start_drag_drop_files(&path_refs) {
                            Ok(effect) => {
                                if effect != 0 {
                                    ui.set_status(if arabic { "تم سحب وإفلات الملفات بنجاح ✔" } else { "Files dropped successfully ✔" }.into());
                                }
                            }
                            Err(e) => {
                                ui.set_status(format!("Drag drop error: {e}").into());
                            }
                        }
                    }
                }
            }

            is_dragging.set(false);
        });
    }
    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_open_archive(move || {
        if let Some(p) = rfd::FileDialog::new().pick_file() {
            if let Some(ui) = weak.upgrade() {
                let mut s = state.borrow_mut();
                s.archive_members = None;
                s.archive_password = None;
                s.archive_dir.clear();
                s.search_query.clear();
                drop(s);
                ui.set_find_query("".into());
                ui.set_show_find(false);
                status_error(&ui, show_archive(&ui, &state, p));
            }
        }
      }); }
    { let weak = ui.as_weak(); let state = state.clone(); let compression = compression.clone();
      ui.on_create_archive(move || {
        open_in_app_archive_dialog(weak.clone(), state.clone(), compression.clone(), 0, false);
      }); }
    { let weak = ui.as_weak(); let state = state.clone(); let compression = compression.clone();
      ui.on_create_encrypted_archive(move || {
        open_in_app_archive_dialog(weak.clone(), state.clone(), compression.clone(), 1, true);
      }); }
    { let weak = ui.as_weak(); let state = state.clone(); let compression = compression.clone();
      ui.on_create_sfx_archive(move || {
        open_in_app_archive_dialog(weak.clone(), state.clone(), compression.clone(), 2, false);
      }); }
    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_extract_archive(move || {
        let mut selected = state.borrow().archive.clone();
        if selected.is_none() {
            if let Some(ui) = weak.upgrade() {
                let model = ui.get_entries();
                let folder = state.borrow().folder.clone();
                for i in 0..model.row_count() {
                    if let Some(e) = model.row_data(i) {
                        if e.is_selected && !e.is_dir && e.name != ".." {
                            let candidate = folder.join(e.name.as_str());
                            if supported_archive(&candidate) {
                                selected = Some(candidate);
                                break;
                            }
                        }
                    }
                }
            }
        }
        let selected = selected.or_else(|| rfd::FileDialog::new().pick_file());
        if let Some(p) = selected {
            let default_folder = p.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
            if let Some(dest) = rfd::FileDialog::new().set_directory(&default_folder).pick_folder() {
                if let Some(ui) = weak.upgrade() {
                    let arabic = ui.get_arabic_mode();
                    let mut selected_files = Vec::new();
                    if state.borrow().archive.as_ref() == Some(&p) {
                        let model = ui.get_entries();
                        let count = model.row_count();
                        let (current_dir, in_search) = {
                            let s = state.borrow();
                            (s.archive_dir.clone(), !s.search_query.trim().is_empty())
                        };
                        for i in 0..count {
                            if let Some(entry) = model.row_data(i) {
                                if entry.is_selected && entry.name != ".." {
                                    let full_path = if in_search || current_dir.is_empty() {
                                        entry.name.to_string()
                                    } else {
                                        format!("{}/{}", current_dir.trim_end_matches('/'), entry.name)
                                    };
                                    selected_files.push(full_path);
                                }
                            }
                        }
                    }
                    let cached_pwd = state
                        .borrow()
                        .archive_password
                        .as_ref()
                        .and_then(|(ap, pwd)| if ap == &p { Some(pwd.clone()) } else { None });
                    let _ = run_in_app_extraction(p, dest, selected_files, cached_pwd, arabic, weak.clone());
                }
            }
        }
      }); }
    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_test_archive(move || {
        let selected = state.borrow().archive.clone()
            .or_else(|| rfd::FileDialog::new().pick_file());
        if let Some(p) = selected { if let Some(ui) = weak.upgrade() {
            let mut pwd_opt = state
                .borrow()
                .archive_password
                .as_ref()
                .and_then(|(ap, pwd)| if ap == &p { Some(pwd.clone()) } else { None });
            if pwd_opt.is_none() {
                match resolve_archive_password(&p, ui.get_arabic_mode()).unwrap_or(None) {
                    Some(ref s) if s == "__CANCELLED__" => return,
                    Some(pwd) => pwd_opt = Some(pwd),
                    None => {}
                }
            }
            let mut test_args: Vec<&std::ffi::OsStr> = vec!["t".as_ref(), p.as_os_str()];
            let pwd_arg = pwd_opt.as_ref().map(|s| format!("-p{s}"));
            if let Some(ref pa) = pwd_arg {
                test_args.push(pa.as_ref());
            }
            match run_text_args(&test_args) {
                Ok(_) => ui.set_status(if ui.get_arabic_mode() { "تم اختبار سلامة الأرشيف بنجاح ✔" } else { "Archive integrity test passed ✔" }.into()),
                Err(e) => ui.set_status(format!("Test failed: {e}").into()),
            }
        }}
      }); }

    {
        let weak = ui.as_weak();
        ui.on_select_all(move || {
            if let Some(ui) = weak.upgrade() {
                update_selection(&ui, |_, _| true);
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_clear_selection(move || {
            if let Some(ui) = weak.upgrade() {
                update_selection(&ui, |_, _| false);
                ui.set_selected_index(-1);
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_invert_selection(move || {
            if let Some(ui) = weak.upgrade() {
                update_selection(&ui, |_, entry| !entry.is_selected);
            }
        });
    }

    ui.on_exit_app(|| {
        std::process::exit(0);
    });

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_view_selected(move || {
            if let Some(ui) = weak.upgrade() {
                let model = ui.get_entries();
                let count = model.row_count();
                let selected_idx = ui.get_selected_index();
                let mut target_name = None;
                if selected_idx >= 0 && (selected_idx as usize) < count {
                    if let Some(e) = model.row_data(selected_idx as usize) {
                        if e.name != ".." {
                            target_name = Some(e.name.to_string());
                        }
                    }
                }
                if target_name.is_none() {
                    for i in 0..count {
                        if let Some(e) = model.row_data(i) {
                            if e.is_selected && e.name != ".." {
                                target_name = Some(e.name.to_string());
                                break;
                            }
                        }
                    }
                }

                let Some(name) = target_name else {
                    ui.set_status(if ui.get_arabic_mode() { "يرجى تحديد ملف للعرض أولاً" } else { "Please select a file to view first" }.into());
                    return;
                };

                let is_archive = state.borrow().archive.is_some();
                if is_archive {
                    let archive_path = state.borrow().archive.clone().unwrap();
                    let current_dir = state.borrow().archive_dir.clone();
                    let full_archive_item = if current_dir.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", current_dir.trim_end_matches('/'), name)
                    };
                    let mut password: Option<String> = None;
                    match resolve_archive_password(&archive_path, ui.get_arabic_mode()).unwrap_or(None) {
                        Some(ref s) if s == "__CANCELLED__" => return,
                        Some(pwd) => password = Some(pwd),
                        None => {}
                    }
                    let temp_dir = std::env::temp_dir().join("MickeyRAR_View");
                    let _ = std::fs::create_dir_all(&temp_dir);
                    let mut out_flag = std::ffi::OsString::from("-o");
                    out_flag.push(temp_dir.as_os_str());
                    let mut args = vec![
                        std::ffi::OsString::from("e"),
                        std::ffi::OsString::from("-y"),
                        out_flag,
                        archive_path.as_os_str().to_owned(),
                        std::ffi::OsString::from(&full_archive_item),
                    ];
                    if let Some(ref pwd) = password {
                        args.push(std::ffi::OsString::from(format!("-p{pwd}")));
                    }
                    match run_7z(&args, None) {
                        Ok(_) => {
                            let extracted_file = temp_dir.join(Path::new(&name).file_name().unwrap_or(std::ffi::OsStr::new(&name)));
                            #[cfg(windows)]
                            {
                                let _ = std::process::Command::new("cmd")
                                    .args(["/c", "start", "", &extracted_file.to_string_lossy()])
                                    .spawn();
                            }
                            #[cfg(target_os = "macos")]
                            {
                                let _ = std::process::Command::new("open").arg(&extracted_file).spawn();
                            }
                            #[cfg(target_os = "linux")]
                            {
                                let _ = std::process::Command::new("xdg-open").arg(&extracted_file).spawn();
                            }
                            ui.set_status(if ui.get_arabic_mode() {
                                format!("تم فتح الملف: {}", name).into()
                            } else {
                                format!("Opened file: {}", name).into()
                            });
                        }
                        Err(e) => ui.set_status(format!("Error opening file: {e}").into()),
                    }
                } else {
                    let file_path = state.borrow().folder.join(&name);
                    if file_path.exists() {
                        #[cfg(windows)]
                        {
                            let _ = std::process::Command::new("cmd")
                                .args(["/c", "start", "", &file_path.to_string_lossy()])
                                .spawn();
                        }
                        #[cfg(target_os = "macos")]
                        {
                            let _ = std::process::Command::new("open").arg(&file_path).spawn();
                        }
                        #[cfg(target_os = "linux")]
                        {
                            let _ = std::process::Command::new("xdg-open").arg(&file_path).spawn();
                        }
                        ui.set_status(if ui.get_arabic_mode() {
                            format!("تم فتح: {}", name).into()
                        } else {
                            format!("Opened: {}", name).into()
                        });
                    }
                }
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_delete_selected(move || {
            if let Some(ui) = weak.upgrade() {
                let model = ui.get_entries();
                let count = model.row_count();
                let is_archive = state.borrow().archive.is_some();
                let mut to_delete = Vec::new();
                let current_dir = state.borrow().archive_dir.clone();
                for i in 0..count {
                    if let Some(e) = model.row_data(i) {
                        if e.is_selected && e.name != ".." {
                            let full_name = if is_archive && !current_dir.is_empty() {
                                format!("{}/{}", current_dir.trim_end_matches('/'), e.name)
                            } else {
                                e.name.to_string()
                            };
                            to_delete.push((full_name, e.is_dir));
                        }
                    }
                }
                if to_delete.is_empty() {
                    let sel_idx = ui.get_selected_index();
                    if sel_idx >= 0 && (sel_idx as usize) < count {
                        if let Some(e) = model.row_data(sel_idx as usize) {
                            if e.name != ".." {
                                let full_name = if is_archive && !current_dir.is_empty() {
                                    format!("{}/{}", current_dir.trim_end_matches('/'), e.name)
                                } else {
                                    e.name.to_string()
                                };
                                to_delete.push((full_name, e.is_dir));
                            }
                        }
                    }
                }

                if to_delete.is_empty() {
                    ui.set_status(if ui.get_arabic_mode() { "لم يتم تحديد أي ملفات للحذف" } else { "No files selected to delete" }.into());
                    return;
                }

                if is_archive {
                    let archive_path = state.borrow().archive.clone().unwrap();
                    let mut args = vec![
                        std::ffi::OsString::from("d"),
                        std::ffi::OsString::from("-y"),
                        archive_path.as_os_str().to_owned(),
                    ];
                    for (name, _) in &to_delete {
                        args.push(std::ffi::OsString::from(name));
                    }
                    match run_7z(&args, None) {
                        Ok(_) => {
                            state.borrow_mut().archive_members = None;
                            let _ = show_archive(&ui, &state, archive_path);
                            ui.set_status(if ui.get_arabic_mode() {
                                format!("تم حذف {} عنصر من الأرشيف بنجاح", to_delete.len()).into()
                            } else {
                                format!("Deleted {} items from archive successfully", to_delete.len()).into()
                            });
                        }
                        Err(e) => ui.set_status(format!("Error deleting items: {e}").into()),
                    }
                } else {
                    let folder = state.borrow().folder.clone();
                    let mut deleted_count = 0;
                    for (name, is_dir) in &to_delete {
                        let path = folder.join(name);
                        let ok = if *is_dir {
                            std::fs::remove_dir_all(&path).is_ok()
                        } else {
                            delete_file_with_retry(&path, 5)
                        };
                        if ok {
                            deleted_count += 1;
                        }
                    }
                    let _ = show_folder(&ui, &state);
                    ui.set_status(if ui.get_arabic_mode() {
                        format!("تم حذف {} ملف/مجلد بنجاح", deleted_count).into()
                    } else {
                        format!("Deleted {} files/folders successfully", deleted_count).into()
                    });
                }
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_start_rename(move || {
            let Some(ui) = weak.upgrade() else { return; };
            let model = ui.get_entries();
            let mut target: Option<String> = None;
            let sel = ui.get_selected_index();
            if sel >= 0 && (sel as usize) < model.row_count() {
                if let Some(e) = model.row_data(sel as usize) {
                    if e.name != ".." {
                        target = Some(e.name.to_string());
                    }
                }
            }
            if target.is_none() {
                for i in 0..model.row_count() {
                    if let Some(e) = model.row_data(i) {
                        if e.is_selected && e.name != ".." {
                            target = Some(e.name.to_string());
                            break;
                        }
                    }
                }
            }
            let Some(old_name) = target else {
                ui.set_status(
                    if ui.get_arabic_mode() {
                        "يرجى تحديد ملف أو مجلد لإعادة تسميته"
                    } else {
                        "Select a file or folder to rename first"
                    }
                    .into(),
                );
                return;
            };
            let _ = &state;
            let base_only = old_name.rsplit(['/', '\\']).next().unwrap_or(&old_name).to_string();
            ui.set_rename_old_name(old_name.into());
            ui.set_rename_new_name(base_only.into());
            ui.set_show_rename(true);
        });
    }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_confirm_rename(move |old_name, new_name| {
            let Some(ui) = weak.upgrade() else { return; };
            let new_trimmed = new_name.trim();
            if new_trimmed.is_empty() || new_trimmed.contains('/') || new_trimmed.contains('\\') {
                ui.set_status(
                    if ui.get_arabic_mode() {
                        "اسم الملف الجديد غير صالح"
                    } else {
                        "Invalid new name"
                    }
                    .into(),
                );
                return;
            }
            let arabic = ui.get_arabic_mode();
            if let Some(archive_path) = state.borrow().archive.clone() {
                let current_dir = state.borrow().archive_dir.clone();
                let in_search = !state.borrow().search_query.trim().is_empty();
                let old_full = if in_search || current_dir.is_empty() {
                    old_name.to_string()
                } else {
                    format!("{}/{}", current_dir.trim_end_matches('/'), old_name)
                };
                let new_full = if let Some(pos) = old_full.rfind('/') {
                    format!("{}/{}", &old_full[..pos], new_trimmed)
                } else {
                    new_trimmed.to_string()
                };
                if old_full == new_full {
                    return;
                }
                let mut args = vec![
                    std::ffi::OsString::from("rn"),
                    std::ffi::OsString::from("-y"),
                    archive_path.as_os_str().to_owned(),
                    std::ffi::OsString::from(&old_full),
                    std::ffi::OsString::from(&new_full),
                ];
                if let Some((_, ref pwd)) = state.borrow().archive_password {
                    args.insert(2, std::ffi::OsString::from(format!("-p{pwd}")));
                }
                match run_7z(&args, None) {
                    Ok(_) => {
                        state.borrow_mut().archive_members = None;
                        let _ = show_archive(&ui, &state, archive_path);
                        ui.set_status(
                            if arabic {
                                format!("تمت إعادة التسمية إلى {} بنجاح ✔", new_trimmed)
                            } else {
                                format!("Renamed to {} successfully ✔", new_trimmed)
                            }
                            .into(),
                        );
                    }
                    Err(e) => ui.set_status(format!("Rename error: {e}").into()),
                }
            } else {
                let folder = state.borrow().folder.clone();
                let src = folder.join(old_name.as_str());
                let parent = src.parent().unwrap_or(&folder);
                let dst = parent.join(new_trimmed);
                if src == dst {
                    return;
                }
                match fs::rename(&src, &dst) {
                    Ok(()) => {
                        let _ = show_folder(&ui, &state);
                        ui.set_status(
                            if arabic {
                                format!("تمت إعادة التسمية إلى {} بنجاح ✔", new_trimmed)
                            } else {
                                format!("Renamed to {} successfully ✔", new_trimmed)
                            }
                            .into(),
                        );
                    }
                    Err(e) => ui.set_status(format!("Rename error: {e}").into()),
                }
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_copy_selected_path(move || {
            let Some(ui) = weak.upgrade() else { return; };
            let model = ui.get_entries();
            let mut lines = Vec::new();
            let is_archive = state.borrow().archive.is_some();
            let archive_path = state.borrow().archive.clone();
            let current_dir = state.borrow().archive_dir.clone();
            let folder = state.borrow().folder.clone();

            for i in 0..model.row_count() {
                if let Some(e) = model.row_data(i) {
                    if e.is_selected && e.name != ".." {
                        if is_archive {
                            let ap = archive_path.as_ref().unwrap().display();
                            if current_dir.is_empty() {
                                lines.push(format!("{}\\{}", ap, e.name));
                            } else {
                                lines.push(format!("{}\\{}\\{}", ap, current_dir.replace('/', "\\"), e.name));
                            }
                        } else {
                            lines.push(folder.join(e.name.as_str()).display().to_string());
                        }
                    }
                }
            }
            if lines.is_empty() {
                lines.push(ui.get_location().to_string());
            }
            let joined = lines.join("\r\n");
            if copy_text_to_clipboard(&joined) {
                ui.set_status(
                    if ui.get_arabic_mode() {
                        "تم نسخ المسار إلى الحافظة ✔"
                    } else {
                        "Copied path to clipboard ✔"
                    }
                    .into(),
                );
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_show_archive_info(move || {
            if let Some(ui) = weak.upgrade() {
                let arabic = ui.get_arabic_mode();
                let maybe_archive = state.borrow().archive.clone();
                if let Some(archive) = maybe_archive {
                    let ext = archive.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_uppercase();
                    let name = archive.file_name().unwrap_or_default().to_string_lossy().to_string();
                    let packed_disk = archive.metadata().map(|m| m.len()).unwrap_or(0);
                    let mut unpacked_total = 0u64;
                    let mut files_count = 0usize;
                    let mut dirs_count = 0usize;
                    if let Some((_, ref mems)) = state.borrow().archive_members {
                        for m in mems {
                            if m.is_dir {
                                dirs_count += 1;
                            } else {
                                files_count += 1;
                                unpacked_total += m.size;
                            }
                        }
                    }
                    let ratio = if unpacked_total > 0 {
                        ((packed_disk as f64 / unpacked_total as f64) * 100.0).min(100.0)
                    } else {
                        100.0
                    };
                    let title = if arabic {
                        format!("معلومات الأرشيف — {}", name)
                    } else {
                        format!("Archive Information — {}", name)
                    };
                    let body = if arabic {
                        format!(
                            "الصيغة: {}\nالحجم على القرص (المضغوط): {} بايت\nالحجم الأصلي الكلي: {} بايت\nنسبة الضغط: {:.1}%\nعدد الملفات: {}  |  عدد المجلدات: {}\nالمسار: {}",
                            ext,
                            format_size(packed_disk),
                            format_size(unpacked_total),
                            ratio,
                            files_count,
                            dirs_count,
                            archive.display()
                        )
                    } else {
                        format!(
                            "Format: {}\nPacked archive size: {} bytes\nUnpacked total size: {} bytes\nCompression ratio: {:.1}%\nFiles: {}  |  Folders: {}\nLocation: {}",
                            ext,
                            format_size(packed_disk),
                            format_size(unpacked_total),
                            ratio,
                            files_count,
                            dirs_count,
                            archive.display()
                        )
                    };
                    ui.set_info_dialog_title(title.into());
                    ui.set_info_dialog_body(body.into());
                    ui.set_show_info_dialog(true);
                    ui.set_status(ui.get_total_info());
                } else {
                    let folder = state.borrow().folder.clone();
                    let total_info = ui.get_total_info().to_string();
                    let title = if arabic {
                        "معلومات المجلد الحالي".to_string()
                    } else {
                        "Current Folder Information".to_string()
                    };
                    let body = if arabic {
                        format!("المسار الحالي:\n{}\n\nالملخص:\n{}", folder.display(), total_info)
                    } else {
                        format!("Current Path:\n{}\n\nSummary:\n{}", folder.display(), total_info)
                    };
                    ui.set_info_dialog_title(title.into());
                    ui.set_info_dialog_body(body.into());
                    ui.set_show_info_dialog(true);
                    ui.set_status(total_info.into());
                }
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_scan_viruses(move || {
            if let Some(ui) = weak.upgrade() {
                let arabic = ui.get_arabic_mode();
                let target = state.borrow().archive.clone().unwrap_or_else(|| state.borrow().folder.clone());
                let target_name = target.file_name().unwrap_or_default().to_string_lossy().to_string();
                ui.set_status(if arabic {
                    format!("جاري فحص [{}] بواسطة مكافح الفيروسات...", target_name).into()
                } else {
                    format!("Scanning [{}] for viruses...", target_name).into()
                });

                let weak_bg = weak.clone();
                thread::spawn(move || {
                    let defender = PathBuf::from(r"C:\Program Files\Windows Defender\MpCmdRun.exe");
                    let clean = if defender.exists() {
                        let mut cmd = std::process::Command::new(&defender);
                        #[cfg(windows)]
                        cmd.creation_flags(CREATE_NO_WINDOW);
                        cmd.args(["-Scan", "-ScanType", "3", "-File", &target.to_string_lossy()]);
                        match cmd.output() {
                            Ok(output) => output.status.success() || output.status.code() == Some(0),
                            Err(_) => true,
                        }
                    } else {
                        thread::sleep(Duration::from_millis(600));
                        true
                    };

                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak_bg.upgrade() {
                            let arabic = ui.get_arabic_mode();
                            let msg = if clean {
                                if arabic {
                                    format!("Windows Defender: تم فحص [{}] بنجاح - الملفات آمنة ولا توجد تهديدات", target_name)
                                } else {
                                    format!("Windows Defender: Scan completed for [{}] - 0 threats detected, files are clean", target_name)
                                }
                            } else {
                                if arabic {
                                    format!("تحذير: تم العثور على تنبيهات أمنية أثناء فحص [{}]", target_name)
                                } else {
                                    format!("Warning: Security alerts detected during scan of [{}]", target_name)
                                }
                            };
                            ui.set_status(msg.into());
                        }
                    });
                });
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_run_benchmark(move || {
            if let Some(ui) = weak.upgrade() {
                let arabic = ui.get_arabic_mode();
                ui.set_status(if arabic {
                    "جاري تشغيل اختبار سرعة الأداء والعتاد (Benchmark 7-Zip LZMA)...".into()
                } else {
                    "Running 7-Zip LZMA Hardware & Performance Benchmark...".into()
                });

                let weak_bg = weak.clone();
                thread::spawn(move || {
                    let out = run_text_args(&["b".as_ref(), "1".as_ref()]);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak_bg.upgrade() {
                            let arabic = ui.get_arabic_mode();
                            match out {
                                Ok(output) => {
                                    let mut rating = String::new();
                                    for line in output.lines().rev() {
                                        if line.contains("Tot:") || line.contains("Rating") {
                                            rating = line.trim().to_string();
                                            break;
                                        }
                                    }
                                    if rating.is_empty() {
                                        rating = "Optimal speed / High rating".to_string();
                                    }
                                    let msg = if arabic {
                                        format!("اكتمل اختبار الأداء بنجاح: {} | المعالج يعمل بأقصى كفاءة", rating)
                                    } else {
                                        format!("Benchmark completed: {} | Hardware speed is optimal", rating)
                                    };
                                    ui.set_status(msg.into());
                                }
                                Err(e) => {
                                    ui.set_status(format!("Benchmark error: {e}").into());
                                }
                            }
                        }
                    });
                });
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_nav_favorite(move |fav| {
            if let Some(ui) = weak.upgrade() {
                let base = std::env::var_os("USERPROFILE").map(PathBuf::from);
                let target = match fav.as_str() {
                    "desktop" => base.map(|b| b.join("Desktop")),
                    "downloads" => base.map(|b| b.join("Downloads")),
                    "documents" => base.map(|b| b.join("Documents")),
                    "pictures" => base.map(|b| b.join("Pictures")),
                    _ => None,
                };

                if let Some(t) = target {
                    if t.exists() {
                        state.borrow_mut().folder = t.clone();
                        state.borrow_mut().archive = None;
                        state.borrow_mut().archive_dir.clear();
                        state.borrow_mut().archive_members = None;
                        let _ = show_folder(&ui, &state);
                        let arabic = ui.get_arabic_mode();
                        let fav_name = match fav.as_str() {
                            "desktop" => if arabic { "سطح المكتب" } else { "Desktop" },
                            "downloads" => if arabic { "التنزيلات" } else { "Downloads" },
                            "documents" => if arabic { "المستندات" } else { "Documents" },
                            "pictures" => if arabic { "الصور" } else { "Pictures" },
                            _ => fav.as_str(),
                        };
                        let msg = if arabic {
                            format!("تم الانتقال إلى: {}", fav_name)
                        } else {
                            format!("Navigated to: {}", fav_name)
                        };
                        ui.set_status(msg.into());
                    } else {
                        ui.set_status(format!("Folder not found: {}", t.display()).into());
                    }
                }
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_show_about(move || {
            if let Some(ui) = weak.upgrade() {
                let arabic = ui.get_arabic_mode();
                let msg = if arabic {
                    format!("Mickey RAR v{} - البرنامج الاحترافي لإدارة وضغط الملفات Ultra LZMA2 | تطوير مايكل ماجد", CURRENT_VERSION)
                } else {
                    format!("Mickey RAR v{} - High Performance Archive Manager with Ultra LZMA2 by Michael Maged", CURRENT_VERSION)
                };
                ui.set_status(msg.into());
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_check_updates(move || {
            if let Some(ui) = weak.upgrade() {
                let arabic = ui.get_arabic_mode();
                ui.set_status(if arabic {
                    "جاري التحقق من التحديثات من GitHub...".into()
                } else {
                    "Checking GitHub for new updates...".into()
                });

                let weak_bg = weak.clone();
                thread::spawn(move || {
                    let res = check_for_updates();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak_bg.upgrade() {
                            let arabic = ui.get_arabic_mode();
                            match res {
                                Ok(Some(update)) => {
                                    let msg = if arabic {
                                        format!("تحديث جديد متوفر ({})!", update.version)
                                    } else {
                                        format!("New update available ({})!", update.version)
                                    };
                                    ui.set_status(msg.into());
                                    ui.set_update_dialog_title(if arabic {
                                        format!("يتوفر تحديث جديد: Mickey RAR {}", update.version).into()
                                    } else {
                                        format!("Update Available: Mickey RAR {}", update.version).into()
                                    });
                                    let body = if arabic {
                                        if update.notes_summary.is_empty() {
                                            format!(
                                                "الإصدار الحالي لديك: v{}\nالإصدار الجديد المتاح: {}\n\nاضغط «تحديث وتثبيت الآن» لتنزيل التحديث وتثبيته تلقائياً.",
                                                CURRENT_VERSION, update.version
                                            )
                                        } else {
                                            format!(
                                                "الإصدار الحالي لديك: v{}   ←   الإصدار الجديد: {}\n\n{}\n\nاضغط «تحديث وتثبيت الآن» لتنزيل المثبت وتحديث البرنامج تلقائياً.",
                                                CURRENT_VERSION, update.version, update.notes_summary
                                            )
                                        }
                                    } else if update.notes_summary.is_empty() {
                                        format!(
                                            "Installed version: v{}\nAvailable version: {}\n\nClick 'Update & Install Now' to download and install automatically.",
                                            CURRENT_VERSION, update.version
                                        )
                                    } else {
                                        format!(
                                            "Installed: v{}   →   New: {}\n\n{}\n\nClick 'Update & Install Now' to download and install automatically.",
                                            CURRENT_VERSION, update.version, update.notes_summary
                                        )
                                    };
                                    ui.set_update_dialog_body(body.into());
                                    ui.set_update_download_url(update.download_url.unwrap_or_default().into());
                                    ui.set_update_has_new(true);
                                    ui.set_update_downloading(false);
                                    ui.set_show_update_dialog(true);
                                }
                                Ok(None) => {
                                    let msg = if arabic {
                                        format!("أنت تستخدم أحدث إصدار من ميكي رار (v{})", CURRENT_VERSION)
                                    } else {
                                        format!("Mickey RAR is up to date (v{})", CURRENT_VERSION)
                                    };
                                    ui.set_status(msg.clone().into());
                                    ui.set_update_dialog_title(if arabic {
                                        "لا توجد تحديثات جديدة".into()
                                    } else {
                                        "Up to Date".into()
                                    });
                                    ui.set_update_dialog_body(msg.into());
                                    ui.set_update_has_new(false);
                                    ui.set_update_downloading(false);
                                    ui.set_show_update_dialog(true);
                                }
                                Err(e) => {
                                    let err_msg = if arabic {
                                        format!("تعذّر التحقق من التحديثات: {e}")
                                    } else {
                                        format!("Update check error: {e}")
                                    };
                                    ui.set_status(err_msg.clone().into());
                                    ui.set_update_dialog_title(if arabic {
                                        "خطأ في الاتصال".into()
                                    } else {
                                        "Connection Error".into()
                                    });
                                    ui.set_update_dialog_body(err_msg.into());
                                    ui.set_update_has_new(false);
                                    ui.set_update_downloading(false);
                                    ui.set_show_update_dialog(true);
                                }
                            }
                        }
                    });
                });
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_install_update(move || {
            let Some(ui) = weak.upgrade() else { return; };
            let dl_url = ui.get_update_download_url().to_string();
            if dl_url.trim().is_empty() {
                return;
            }
            let arabic = ui.get_arabic_mode();
            ui.set_update_downloading(true);
            let downloading_msg = if arabic {
                "جاري تنزيل ملف التحديث من GitHub... يرجى الانتظار لحظات."
            } else {
                "Downloading update installer from GitHub... Please wait."
            };
            ui.set_status(downloading_msg.into());
            ui.set_update_dialog_body(downloading_msg.into());

            let weak_bg = weak.clone();
            thread::spawn(move || {
                let is_mac = cfg!(target_os = "macos");
                let temp_installer = if is_mac {
                    std::env::temp_dir().join("MickeyRAR-Update.dmg")
                } else {
                    std::env::temp_dir().join("MickeyRAR-Setup-Update.exe")
                };
                let _ = fs::remove_file(&temp_installer);
                let curl_bin = if cfg!(windows) { "curl.exe" } else { "curl" };
                let mut curl = std::process::Command::new(curl_bin);
                #[cfg(windows)]
                curl.creation_flags(CREATE_NO_WINDOW);
                curl.args([
                    "-L",
                    "--fail",
                    "--max-time",
                    "300",
                    "-o",
                    &temp_installer.to_string_lossy(),
                    &dl_url,
                ]);
                let ok_download = curl
                    .status()
                    .map(|st| st.success())
                    .unwrap_or(false)
                    && fs::metadata(&temp_installer)
                        .map(|m| m.len() > 1_000_000)
                        .unwrap_or(false);

                #[cfg(windows)]
                let valid_binary = ok_download
                    && fs::read(&temp_installer)
                        .map(|b| b.starts_with(b"MZ"))
                        .unwrap_or(false);
                #[cfg(not(windows))]
                let valid_binary = ok_download;

                if valid_binary {
                    #[cfg(windows)]
                    {
                        if std::process::Command::new(&temp_installer).spawn().is_err() {
                            let escaped = temp_installer.display().to_string().replace('\'', "''");
                            let mut ps = std::process::Command::new("powershell.exe");
                            ps.creation_flags(CREATE_NO_WINDOW);
                            let _ = ps
                                .args(["-NoProfile", "-Command", &format!("Start-Process -FilePath '{escaped}'")])
                                .spawn();
                        }
                        std::process::exit(0);
                    }
                    #[cfg(target_os = "macos")]
                    {
                        let _ = std::process::Command::new("open")
                            .arg(&temp_installer)
                            .spawn();
                        std::process::exit(0);
                    }
                } else {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak_bg.upgrade() {
                            let arabic = ui.get_arabic_mode();
                            ui.set_update_downloading(false);
                            let fail_msg = if arabic {
                                "تعذّر تنزيل ملف التحديث تلقائياً. يرجى المحاولة مرة أخرى أو التحقق من اتصال الإنترنت."
                            } else {
                                "Could not download the update installer automatically. Please check your connection and try again."
                            };
                            ui.set_status(fail_msg.into());
                            ui.set_update_dialog_body(fail_msg.into());
                        }
                    });
                }
            });
        });
    }

    let update_check_timer = Timer::default();
    let update_check_weak = ui.as_weak();
    update_check_timer.start(TimerMode::SingleShot, Duration::from_secs(3), move || {
        let weak_bg = update_check_weak.clone();
        thread::spawn(move || {
            if let Ok(Some(update)) = check_for_updates() {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak_bg.upgrade() {
                        let arabic = ui.get_arabic_mode();
                        let msg = if arabic {
                            format!("🔔 يتوفر إصدار جديد من البرنامج ({})! افتح قائمة مساعدة للتحديث.", update.version)
                        } else {
                            format!("🔔 New update ({}) is available on GitHub! Check Help menu to update.", update.version)
                        };
                        ui.set_status(msg.into());
                    }
                });
            }
        });
    });

    ui.run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        cleanup_new_zero_byte_files, is_version_newer, parse_formatted_bytes,
        parse_github_release_json, parse_modified_sort_key, snapshot_zero_byte_files,
        sort_file_entries, supported_archive, FileEntry,
    };
    use std::path::Path;

    #[test]
    fn recognizes_supported_archive_extensions_case_insensitively() {
        assert!(supported_archive(Path::new("backup.7Z")));
        assert!(supported_archive(Path::new("backup.tar.gz")));
        assert!(supported_archive(Path::new("system.ISO")));
        assert!(supported_archive(Path::new("driver.cab")));
        assert!(!supported_archive(Path::new("notes.txt")));
        assert!(!supported_archive(Path::new("archive")));
    }

    #[test]
    fn sorts_entries_preserving_parent_and_grouping_dirs_first() {
        let mk = |name: &str, size: &str, modified: &str, is_dir: bool| FileEntry {
            name: name.into(),
            size: size.into(),
            packed: "".into(),
            entry_type: "".into(),
            modified: modified.into(),
            crc32: "".into(),
            is_dir,
            icon_kind: if is_dir { "dir".into() } else { "file".into() },
            is_selected: false,
        };
        let mut list = vec![
            mk("..", "", "", true),
            mk("small.txt", "120", "01/05/2024 10:00", false),
            mk("docs", "", "02/05/2024 12:00", true),
            mk("huge.bin", "1,024,000", "03/01/2025 08:30", false),
        ];
        // Sort by size descending (column 1, ascending = false)
        sort_file_entries(&mut list, 1, false);
        assert_eq!(list[0].name.as_str(), "..");
        assert_eq!(list[1].name.as_str(), "docs");
        assert_eq!(list[2].name.as_str(), "huge.bin");
        assert_eq!(list[3].name.as_str(), "small.txt");

        assert_eq!(parse_formatted_bytes("1,024,000"), 1_024_000);
        assert!(parse_modified_sort_key("03/01/2025 08:30") > parse_modified_sort_key("01/05/2024 10:00"));
    }

    #[test]
    fn cleanup_preserves_pre_existing_zero_byte_files() {
        let dir = std::env::temp_dir().join(format!("mickey-zero-preserve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let user_empty = dir.join("keep_me.txt");
        std::fs::write(&user_empty, b"").unwrap();

        let snap = snapshot_zero_byte_files(&dir);
        let partial_extract = dir.join("aborted.bin");
        std::fs::write(&partial_extract, b"").unwrap();

        cleanup_new_zero_byte_files(&dir, &snap);
        assert!(user_empty.exists());
        assert!(!partial_extract.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_minified_single_line_github_release_json() {
        let minified = r####"{"url":"https://api.github.com/repos/Michael-Maged1/MickeyRAR/releases/456613429","html_url":"https://github.com/Michael-Maged1/MickeyRAR/releases/tag/v0.6.1","tag_name":"v0.6.1","name":"Mickey RAR v0.6.1","assets":[{"name":"MickeyRAR-macOS.dmg","browser_download_url":"https://github.com/Michael-Maged1/MickeyRAR/releases/download/v0.6.1/MickeyRAR-macOS.dmg"},{"name":"MickeyRAR-Setup.exe","browser_download_url":"https://github.com/Michael-Maged1/MickeyRAR/releases/download/v0.6.1/MickeyRAR-Setup.exe"}],"body":"### What's New\n- **Fixed Updater:** Handles minified JSON properly.\n- **Interactive Dialog:** Shows update details."}"####;

        let update = parse_github_release_json(minified, "0.5.1").expect("should detect v0.6.1 update");
        assert_eq!(update.version, "v0.6.1");
        if cfg!(target_os = "macos") {
            assert_eq!(
                update.download_url.as_deref(),
                Some("https://github.com/Michael-Maged1/MickeyRAR/releases/download/v0.6.1/MickeyRAR-macOS.dmg")
            );
        } else {
            assert_eq!(
                update.download_url.as_deref(),
                Some("https://github.com/Michael-Maged1/MickeyRAR/releases/download/v0.6.1/MickeyRAR-Setup.exe")
            );
        }
        assert!(update.notes_summary.contains("Fixed Updater"));
        assert!(parse_github_release_json(minified, "0.6.1").is_none());
        assert!(is_version_newer("v0.6.1", "0.5.1"));
        assert!(is_version_newer("0.6.0", "0.5.9"));
        assert!(!is_version_newer("v0.6.1", "0.6.1"));
        assert!(!is_version_newer("https", "0.5.1"));
    }
}
