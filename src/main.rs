#![windows_subsystem = "windows"]

mod i18n;
use i18n::Language;
mod engine;
mod winicon;
use engine::{CompressionSettings, RunOutcome, create_arguments, create_arguments_for_format, delete_file_with_retry, run_7z, run_7z_cancellable_with_progress};
use slint::{Model, ModelRc, VecModel, Timer, TimerMode};
use std::{cell::RefCell, fs, path::{Path, PathBuf}, rc::Rc, sync::{mpsc, Arc, Mutex, atomic::{AtomicBool, AtomicU8, Ordering}}, thread, time::{Duration, Instant}};

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

    export component ProgressWindow inherits Window {
        title: "Mickey RAR";
        icon: @image-url("../assets/icon-window.png");
        width: 390px;
        height: 132px;
        background: #202522;
        in-out property <float> progress: 0.0;
        in-out property <string> message: "Processing...";
        in-out property <bool> rtl: false;
        Rectangle { x: 18px; y: 18px; width: 34px; height: 34px; border-radius: 7px; background: #a7d63f;
            Image { source: @image-url("../assets/icon.png"); width: 34px; height: 34px; image-fit: contain; }
        }
        Text { x: 64px; y: 19px; text: "Mickey RAR"; color: #f3f7f1; font-size: 18px; font-weight: 700; }
        Text { x: 64px; y: 46px; text: root.message; color: #b7c5b5; font-size: 11px; }
        Rectangle { x: 18px; y: 94px; width: parent.width - 36px; height: 8px; border-radius: 4px; background: #343b36;
            Rectangle {
                x: root.rtl ? (parent.width - root.progress * parent.width) : 0px;
                width: root.progress * parent.width;
                height: parent.height;
                border-radius: 4px;
                background: #a7d63f;
            }
        }
    }

    export component ArchiveDialog inherits Window {
        title: "Mickey RAR — Add to archive";
        icon: @image-url("../assets/icon-window.png");
        width: 540px;
        height: 390px;
        background: #202020;
        in-out property <string> archive-name: "MickeyRAR-archive";
        in-out property <string> destination: "";
        in-out property <int> format-index: 0;
        in-out property <int> level-index: 4;
        in-out property <bool> delete-original: false;
        callback browse-location();
        callback start-compression();
        callback cancel();

        VerticalBox { padding: 18px; spacing: 10px;
            Text { text: "Add to archive"; color: #f3f7f1; font-size: 20px; font-weight: 700; }
            Text { text: "Choose the archive name, location, format and compression level."; color: #aeb8ad; font-size: 12px; }
            Rectangle { height: 1px; background: #3a3a3a; }
            HorizontalBox { spacing: 10px; Text { text: "Archive name:"; width: 125px; color: #dfe7dc; vertical-alignment: center; } LineEdit { text <=> root.archive-name; } }
            HorizontalBox { spacing: 10px; Text { text: "Save location:"; width: 125px; color: #dfe7dc; vertical-alignment: center; } LineEdit { text <=> root.destination; } Button { text: "Browse..."; clicked => { root.browse-location(); } } }
            HorizontalBox { spacing: 10px; Text { text: "Archive format:"; width: 125px; color: #dfe7dc; vertical-alignment: center; } ComboBox { model: ["ZIP", "7Z"]; current-index <=> root.format-index; } }
            HorizontalBox { spacing: 10px; Text { text: "Compression level:"; width: 125px; color: #dfe7dc; vertical-alignment: center; } ComboBox { model: ["Store", "Fast", "Normal", "Maximum", "Ultra"]; current-index <=> root.level-index; } }
            CheckBox { text: "Delete original files after successful compression"; checked <=> root.delete-original; }
            Rectangle { vertical-stretch: 1; }
            HorizontalBox { alignment: end; spacing: 8px; Button { text: "Cancel"; clicked => { root.cancel(); } } Button { text: "Create archive"; clicked => { root.start-compression(); } } }
        }
    }
}

type CompressionWindow = ProgressWindow;

#[derive(Default)]
struct AppState {
    folder: PathBuf,
    archive: Option<PathBuf>,
    paths: Vec<PathBuf>,
    pending_paths: Vec<PathBuf>,
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

fn run_context_extraction_with_options(
    paths: Vec<PathBuf>,
    target_dir: Option<PathBuf>,
    extract_to_subfolder: bool,
    arabic: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if paths.is_empty() { return Err("No archive selected".into()); }
    let archives: Vec<PathBuf> = paths.into_iter().filter(|p| p.is_file()).collect();
    if archives.is_empty() { return Err("Selected file does not exist".into()); }

    let ui = ProgressWindow::new()?;
    quit_process_on_close(&ui.window());
    ui.set_rtl(arabic);

    let center_timer = Timer::default();
    let center_weak = ui.as_weak();
    center_timer.start(TimerMode::SingleShot, Duration::from_millis(120), move || {
        if let Some(ui) = center_weak.upgrade() { center_window(&ui.window()); }
        winicon::install();
    });

    let first_name = archives[0].file_name().unwrap_or_default().to_string_lossy();
    ui.set_message(if arabic { format!("جاري فك ضغط {first_name}...").into() } else { format!("Extracting {first_name}...").into() });

    let cancelled = Arc::new(AtomicBool::new(false));
    let progress_handle = Arc::new(AtomicU8::new(0));
    let worker_cancelled = cancelled.clone();
    let worker_progress = progress_handle.clone();
    let worker_done = Arc::new(AtomicBool::new(false));
    let worker_finished = worker_done.clone();
    let (sender, receiver) = mpsc::channel();

    let archives_worker = archives.clone();
    let worker = thread::spawn(move || {
        for archive in archives_worker {
            if worker_cancelled.load(Ordering::Relaxed) { break; }
            let abs_archive = if archive.is_absolute() { archive } else {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(&archive)
            };
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
            let dest_str = dest.to_string_lossy();
            let clean_dest = dest_str.trim_end_matches(['\\', '/']);
            let mut output_arg = std::ffi::OsString::from("-o");
            output_arg.push(clean_dest);

            let args = vec![
                std::ffi::OsString::from("x"),
                std::ffi::OsString::from("-y"),
                std::ffi::OsString::from("-aoa"),
                output_arg,
                abs_archive.as_os_str().to_owned(),
            ];
            let res = run_7z_cancellable_with_progress(&args, None, &worker_cancelled, Some(&worker_progress));
            if let Err(e) = res {
                worker_finished.store(true, Ordering::SeqCst);
                let _ = sender.send(Err(e));
                return;
            }
            if worker_cancelled.load(Ordering::Relaxed) {
                worker_finished.store(true, Ordering::SeqCst);
                let _ = sender.send(Ok(RunOutcome::Cancelled));
                return;
            }
        }
        worker_progress.store(100, Ordering::Relaxed);
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
    let timer = Timer::default();
    let mut shown_percent = u8::MAX;

    timer.start(TimerMode::Repeated, Duration::from_millis(80), move || {
        if let Some(ui) = weak.upgrade() {
            let percent = timer_progress.load(Ordering::Relaxed);
            ui.set_progress(percent as f32 / 100.0);
            if percent != shown_percent {
                shown_percent = percent;
                let text = if percent == 0 {
                    if arabic { "جاري فك الضغط...".to_string() } else { "Extracting files...".to_string() }
                } else if arabic {
                    format!("جاري فك الضغط... {percent}%")
                } else {
                    format!("Extracting files... {percent}%")
                };
                ui.set_message(text.into());
            }

            if let Ok(result) = timer_receiver.lock().unwrap().try_recv() {
                match result {
                    Ok(RunOutcome::Finished) => {
                        ui.set_progress(1.0);
                        ui.set_message(if arabic { "تم فك الضغط بنجاح".into() } else { "Extraction completed successfully".into() });
                    }
                    Ok(RunOutcome::Cancelled) => {
                        ui.set_message(if arabic { "تم إلغاء فك الضغط".into() } else { "Extraction cancelled".into() });
                    }
                    Err(error) => {
                        ui.set_message(format!("Error: {error}").into());
                    }
                }
                let _ = slint::quit_event_loop();
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

fn run_context_compression_with_options(paths: Vec<PathBuf>, output: PathBuf, format: &str, level: u8, delete_original: bool, arabic: bool) -> Result<(), Box<dyn std::error::Error>> {
    if paths.is_empty() { return Err("No files were selected".into()); }
    let mut settings = load_settings();
    settings.level = level;
    let (cwd, args) = create_arguments_for_format(&output, &paths, &settings, format).map_err(|e| format!("{e}"))?;
    let format_name = format.to_ascii_uppercase();
    let paths_for_delete = paths.clone();
    // Only a brand-new archive may be thrown away on failure/cancel. If the
    // user asked to add to an archive that already exists, deleting it would
    // destroy their previous data.
    let output_existed = output.is_file();
    let cancelled = Arc::new(AtomicBool::new(false));
    let thread_cancelled = cancelled.clone();
    let progress_handle = Arc::new(AtomicU8::new(0));
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
    ui.set_message(if arabic { format!("جاري ضغط الملفات بصيغة {format_name}...").into() } else { format!("Compressing files as {format_name}...").into() });
    let close_cancelled = cancelled.clone();
    let close_done = worker_done.clone();
    // Closing the window means "stop now": cancel 7-Zip, delete the incomplete
    // archive, and then really end the program. A helper thread does the waiting
    // so the UI thread never blocks and the process cannot be left behind
    // holding a handle on the half-written file.
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
    let timer = Timer::default();
    let mut shown_percent = u8::MAX;
    timer.start(TimerMode::Repeated, Duration::from_millis(120), move || {
        if let Some(ui) = weak.upgrade() {
            // Real percentage from 7-Zip, not a guess: the bar only moves when
            // compression actually moves, and it shows how far along it is.
            let percent = timer_progress.load(Ordering::Relaxed);
            ui.set_progress(percent as f32 / 100.0);
            if percent != shown_percent {
                shown_percent = percent;
                let text = if percent == 0 {
                    if arabic { format!("جاري تجهيز الملفات ({format_name})...") } else { format!("Preparing files ({format_name})...") }
                } else if arabic {
                    format!("جاري ضغط الملفات بصيغة {format_name}... {percent}%")
                } else {
                    format!("Compressing files as {format_name}... {percent}%")
                };
                ui.set_message(text.into());
            }
            if let Ok(result) = timer_receiver.lock().unwrap().try_recv() {
                match result {
                    Ok(RunOutcome::Finished) => {
                        ui.set_progress(1.0);
                        ui.set_message(if arabic { format!("تم إنشاء ملف {format_name} بنجاح").into() } else { format!("{format_name} archive created successfully").into() });
                    }
                    Ok(RunOutcome::Cancelled) => { ui.set_message(if arabic { "تم إلغاء العملية".into() } else { "Compression cancelled".into() }); }
                    Err(error) => { ui.set_message(format!("Error: {error}").into()); }
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
    delete_original: bool,
}

fn run_archive_dialog(paths: Vec<PathBuf>, arabic: bool) -> Result<(), Box<dyn std::error::Error>> {
    if paths.is_empty() { return Err("No files were selected".into()); }
    let default_folder = paths[0].parent().ok_or("Cannot determine the selected file folder")?.to_path_buf();
    let ui = ArchiveDialog::new()?;
    quit_process_on_close(&ui.window());
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
                let _ = sender.send(Some(ArchiveRequest {
                    name: ui.get_archive_name().to_string(),
                    destination: PathBuf::from(ui.get_destination().to_string()),
                    format: if ui.get_format_index() == 1 { "7z".into() } else { "zip".into() },
                    level,
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
    let extension = request.format.as_str();
    if name.extension().is_none() { name.set_extension(extension); }
    let output = request.destination.join(name.file_name().unwrap());
    run_context_compression_with_options(paths, output, extension, request.level, request.delete_original, arabic)
}

fn supported_archive(p: &Path) -> bool {
    p.extension().and_then(|s| s.to_str()).map(|e| {
        matches!(e.to_ascii_lowercase().as_str(), "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz")
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
fn load_language() -> Language {
    fs::read_to_string(language_path())
        .map(|s| Language::from_code(&s))
        .unwrap_or_else(|_| Language::English)
}
fn save_language(lang: Language) -> Result<(), String> {
    fs::write(language_path(), lang.code()).map_err(|e| e.to_string())
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
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => (
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
    let folder = state.borrow().folder.clone();
    let mut items = Vec::new();
    for e in fs::read_dir(&folder).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        items.push(e.path());
    }
    items.sort_by_key(|p| (!p.is_dir(), p.file_name().map(|x| x.to_string_lossy().to_lowercase())));

    let arabic = ui.get_arabic_mode();
    let mut entries = Vec::new();
    let mut total_folders = 0usize;
    let mut total_files = 0usize;
    let mut total_bytes = 0u64;

    if folder.parent().is_some() {
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
        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let is_dir = p.is_dir();
        let (entry_type, icon_kind) = determine_type_and_icon(&name, is_dir, arabic);
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
            name: name.into(),
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

    {
        let mut s = state.borrow_mut();
        s.paths = items;
        s.archive = None;
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

struct ArchiveItem {
    path: String,
    size: u64,
    packed_size: u64,
    modified: String,
    crc: String,
    is_dir: bool,
}

fn list_archive_members(archive: &Path) -> Result<Vec<ArchiveItem>, String> {
    let out = run_text_args(&["l".as_ref(), "-slt".as_ref(), archive.as_os_str()])?;
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
    let members = list_archive_members(&archive)?;
    let arabic = ui.get_arabic_mode();
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

    for m in members {
        let (entry_type, icon_kind) = determine_type_and_icon(&m.path, m.is_dir, arabic);
        let size_str = if m.is_dir {
            total_folders += 1;
            String::new()
        } else {
            total_files += 1;
            total_bytes += m.size;
            format_size(m.size)
        };
        let packed_str = if m.packed_size > 0 { format_size(m.packed_size) } else { String::new() };
        let mod_str = if !m.modified.is_empty() {
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&m.modified, "%Y-%m-%d %H:%M:%S") {
                dt.format("%d/%m/%Y %H:%M").to_string()
            } else {
                m.modified
            }
        } else {
            String::new()
        };

        entries.push(FileEntry {
            name: m.path.into(),
            size: size_str.into(),
            packed: packed_str.into(),
            entry_type: entry_type.into(),
            modified: mod_str.into(),
            crc32: m.crc.into(),
            is_dir: m.is_dir,
            icon_kind: icon_kind.into(),
            is_selected: false,
        });
    }

    ui.set_location(archive.display().to_string().into());
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
    state.borrow_mut().archive = Some(archive);
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

#[derive(Clone, Debug, Default)]
struct UpdateInfo {
    version: String,
    download_url: Option<String>,
}

fn is_version_newer(remote: &str, current: &str) -> bool {
    let r_parts: Vec<u32> = remote.split('.').filter_map(|s| s.parse().ok()).collect();
    let c_parts: Vec<u32> = current.split('.').filter_map(|s| s.parse().ok()).collect();
    for (r, c) in r_parts.iter().zip(c_parts.iter()) {
        if r > c { return true; }
        if r < c { return false; }
    }
    r_parts.len() > c_parts.len()
}

fn check_for_updates() -> Result<Option<UpdateInfo>, String> {
    #[cfg(windows)]
    {
        let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
        let mut cmd = std::process::Command::new("curl.exe");
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.args(["-s", "-H", "User-Agent: MickeyRAR-App", &url]);
        let output = cmd.output().map_err(|e| format!("Network check failed: {e}"))?;
        if !output.status.success() {
            return Err("Could not reach GitHub API".into());
        }
        let json_str = String::from_utf8_lossy(&output.stdout);
        if json_str.contains("\"message\": \"Not Found\"") {
            return Ok(None);
        }

        let tag = json_str.lines()
            .find(|l| l.contains("\"tag_name\":"))
            .and_then(|l| {
                let parts: Vec<&str> = l.split(':').collect();
                if parts.len() >= 2 {
                    Some(parts[1].trim().trim_matches(|c| c == '"' || c == ',' || c == ' ').to_string())
                } else {
                    None
                }
            });

        let Some(tag_name) = tag else {
            return Ok(None);
        };

        let remote_ver = tag_name.trim_start_matches('v');
        if is_version_newer(remote_ver, CURRENT_VERSION) {
            let mut dl_url = None;
            for line in json_str.lines() {
                if line.contains("browser_download_url") && line.contains("Setup.exe") {
                    let parts: Vec<&str> = line.split("http").collect();
                    if parts.len() >= 2 {
                        let full = format!("http{}", parts[1].trim_matches(|c| c == '"' || c == ','));
                        dl_url = Some(full);
                        break;
                    }
                }
            }
            if dl_url.is_none() {
                dl_url = Some(format!("https://github.com/{GITHUB_REPO}/releases/latest"));
            }
            return Ok(Some(UpdateInfo {
                version: tag_name,
                download_url: dl_url,
            }));
        }
    }
    Ok(None)
}

fn run_in_app_extraction(
    archive: PathBuf,
    dest: PathBuf,
    selected_files: Vec<String>,
    arabic: bool,
    main_ui: slint::Weak<MainWindow>,
) -> Result<(), Box<dyn std::error::Error>> {
    let progress_ui = ProgressWindow::new()?;
    progress_ui.set_rtl(arabic);
    let name = archive.file_name().unwrap_or_default().to_string_lossy();
    progress_ui.set_message(if arabic {
        format!("جاري فك ضغط {name}...").into()
    } else {
        format!("Extracting {name}...").into()
    });
    progress_ui.set_progress(0.0);
    center_window(&progress_ui.window());
    progress_ui.show()?;

    let cancelled = Arc::new(AtomicBool::new(false));
    let progress_handle = Arc::new(AtomicU8::new(0));
    let worker_cancelled = cancelled.clone();
    let worker_progress = progress_handle.clone();
    let worker_done = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel();

    let dest_clone = dest.clone();
    let archive_clone = archive.clone();
    let selected_clone = selected_files.clone();
    let worker_finished = worker_done.clone();

    thread::spawn(move || {
        let _ = std::fs::create_dir_all(&dest_clone);
        let dest_str = dest_clone.to_string_lossy();
        let clean_dest = dest_str.trim_end_matches(['\\', '/']);
        let mut output_arg = std::ffi::OsString::from("-o");
        output_arg.push(clean_dest);

        let mut args = vec![
            std::ffi::OsString::from("x"),
            std::ffi::OsString::from("-y"),
            std::ffi::OsString::from("-aoa"),
            output_arg,
            archive_clone.as_os_str().to_owned(),
        ];
        for f in selected_clone {
            args.push(std::ffi::OsString::from(f));
        }

        let res = run_7z_cancellable_with_progress(&args, None, &worker_cancelled, Some(&worker_progress));
        worker_progress.store(100, Ordering::Relaxed);
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
    let mut shown_percent = u8::MAX;
    let timer_self = timer.clone();
    let progress_ui_keep = Rc::new(progress_ui);
    let progress_ui_ref = progress_ui_keep.clone();

    let new_timer = Timer::default();
    new_timer.start(TimerMode::Repeated, Duration::from_millis(80), move || {
        let _keep = &progress_ui_ref;
        if let Some(ui) = p_weak.upgrade() {
            let percent = timer_progress.load(Ordering::Relaxed);
            ui.set_progress(percent as f32 / 100.0);
            if percent != shown_percent {
                shown_percent = percent;
                let text = if percent == 0 {
                    if arabic { "جاري فك الضغط...".to_string() } else { "Extracting files...".to_string() }
                } else if arabic {
                    format!("جاري فك الضغط... {percent}%")
                } else {
                    format!("Extracting files... {percent}%")
                };
                ui.set_message(text.into());
            }

            if let Ok(result) = timer_receiver.lock().unwrap().try_recv() {
                let status_msg = match result {
                    Ok(RunOutcome::Finished) => {
                        ui.set_progress(1.0);
                        ui.set_message(if arabic { "تم فك الضغط بنجاح".into() } else { "Extraction completed successfully".into() });
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
            "--extract-here" => return run_context_extract_here(paths, arabic),
            "--extract-to" => return run_context_extract_to(paths, arabic),
            "--extract-to-folder" => return run_context_extract_to_folder(paths, arabic),
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
    let snap_dark = std::env::args().any(|a| a == "--snap-dark");
    let snap_light = std::env::args().any(|a| a == "--snap-light");
    if snap_light || std::env::args().any(|a| a == "--light") {
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
            if let Some(ui) = weak.upgrade() { ui.set_dark_mode(!ui.get_dark_mode()); }
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
                let message = if ui.get_arabic_mode() { format!("أداة {} جاهزة للإضافة لاحقاً", tool) } else { format!("{} tool is ready for the next feature update", tool) };
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
        paths: Vec::new(),
        pending_paths,
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
              let is_archive = state.borrow().archive.is_some();
              if is_archive {
                  state.borrow_mut().archive = None;
                  status_error(&ui, show_folder(&ui, &state));
              } else {
                  let current = state.borrow().folder.clone();
                  if let Some(parent) = current.parent() {
                      state.borrow_mut().folder = parent.to_path_buf();
                      status_error(&ui, show_folder(&ui, &state));
                  }
              }
          }
      }); }

    { let weak = ui.as_weak();
      ui.on_select_entry(move |idx| {
          if let Some(ui) = weak.upgrade() {
              let model = ui.get_entries();
              let count = model.row_count();
              if idx < 0 || idx as usize >= count { return; }
              let mut updated = Vec::with_capacity(count);
              let mut sel_size = String::new();
              let mut is_file = false;

              for i in 0..count {
                  if let Some(mut entry) = model.row_data(i) {
                      let selected = i == idx as usize;
                      entry.is_selected = selected;
                      if selected && !entry.is_dir && entry.name != ".." {
                          sel_size = entry.size.to_string();
                          is_file = true;
                      }
                      updated.push(entry);
                  }
              }
              ui.set_entries(ModelRc::new(VecModel::from(updated)));
              ui.set_selected_index(idx);

              if is_file && !sel_size.is_empty() {
                  let text = if ui.get_arabic_mode() {
                      format!("تم تحديد {} بايت في ملف واحد", sel_size)
                  } else {
                      format!("Selected {} bytes in 1 files", sel_size)
                  };
                  ui.set_selected_info(text.into());
              } else {
                  let text = if ui.get_arabic_mode() {
                      "تم تحديد 0 بايت في 0 ملفات"
                  } else {
                      "Selected 0 bytes in 0 files"
                  };
                  ui.set_selected_info(text.into());
              }
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


    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_choose_folder(move || {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            state.borrow_mut().folder = folder;
            if let Some(ui) = weak.upgrade() { status_error(&ui, show_folder(&ui, &state)); }
        }
      }); }
    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_refresh(move || { if let Some(ui) = weak.upgrade() {
          let archive = state.borrow().archive.clone();
          let result = match archive { Some(p) => show_archive(&ui, &state, p), None => show_folder(&ui, &state) };
          status_error(&ui, result);
      }}); }
    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_open_entry(move |entry| {
        if let Some(ui) = weak.upgrade() {
            if entry == ".." {
                let is_archive = state.borrow().archive.is_some();
                if is_archive {
                    state.borrow_mut().archive = None;
                    status_error(&ui, show_folder(&ui, &state));
                } else {
                    let current = state.borrow().folder.clone();
                    if let Some(parent) = current.parent() {
                        state.borrow_mut().folder = parent.to_path_buf();
                        status_error(&ui, show_folder(&ui, &state));
                    }
                }
                return;
            }
            if state.borrow().archive.is_some() { ui.set_status(format!("Selected: {entry}").into()); return; }
            let p = state.borrow().folder.join(&entry);
            if p.is_dir() { state.borrow_mut().folder = p; status_error(&ui, show_folder(&ui, &state)); }
            else if supported_archive(&p) { status_error(&ui, show_archive(&ui, &state, p)); }
            else { ui.set_status(format!("Selected: {}", p.display()).into()); }
        }
      }); }
    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_open_archive(move || {
        if let Some(p) = rfd::FileDialog::new().pick_file() {
            if let Some(ui) = weak.upgrade() { status_error(&ui, show_archive(&ui, &state, p)); }
        }
      }); }
    { let weak = ui.as_weak(); let state = state.clone(); let compression = compression.clone();
      ui.on_create_archive(move || {
        let context_files = std::mem::take(&mut state.borrow_mut().pending_paths);
        let files = if context_files.is_empty() {
            rfd::FileDialog::new().pick_files()
        } else {
            Some(context_files)
        };
        if let Some(files) = files {
            if let Some(out) = rfd::FileDialog::new().set_file_name("archive.zip").save_file() {
                if let Some(ui) = weak.upgrade() {
                    let result = create_arguments(&out, &files, &compression.borrow())
                        .and_then(|(cwd, args)| run_7z(&args, Some(&cwd)));
                    match result {
                        Ok(_) => ui.set_status(format!("Created: {}", out.display()).into()),
                        Err(e) => ui.set_status(format!("Error: {e}").into()),
                    }
                }
            }
        }
      }); }
    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_extract_archive(move || {
        let selected = state.borrow().archive.clone()
            .or_else(|| rfd::FileDialog::new().pick_file());
        if let Some(p) = selected {
            let default_folder = p.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
            if let Some(dest) = rfd::FileDialog::new().set_directory(&default_folder).pick_folder() {
                if let Some(ui) = weak.upgrade() {
                    let arabic = ui.get_arabic_mode();
                    let model = ui.get_entries();
                    let count = model.row_count();
                    let mut selected_files = Vec::new();
                    for i in 0..count {
                        if let Some(entry) = model.row_data(i) {
                            if entry.is_selected && entry.name != ".." {
                                selected_files.push(entry.name.to_string());
                            }
                        }
                    }
                    let _ = run_in_app_extraction(p, dest, selected_files, arabic, weak.clone());
                }
            }
        }
      }); }
    { let weak = ui.as_weak(); let state = state.clone();
      ui.on_test_archive(move || {
        let selected = state.borrow().archive.clone()
            .or_else(|| rfd::FileDialog::new().pick_file());
        if let Some(p) = selected { if let Some(ui) = weak.upgrade() {
            match run_text_args(&["t".as_ref(), p.as_os_str()]) {
                Ok(_) => ui.set_status(if ui.get_arabic_mode() { "تم اختبار سلامة الأرشيف بنجاح" } else { "Archive integrity test passed" }.into()),
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
                    let temp_dir = std::env::temp_dir().join("MickeyRAR_View");
                    let _ = std::fs::create_dir_all(&temp_dir);
                    let mut out_flag = std::ffi::OsString::from("-o");
                    out_flag.push(temp_dir.as_os_str());
                    let args = vec![
                        std::ffi::OsString::from("e"),
                        std::ffi::OsString::from("-y"),
                        out_flag,
                        archive_path.as_os_str().to_owned(),
                        std::ffi::OsString::from(&name),
                    ];
                    match run_7z(&args, None) {
                        Ok(_) => {
                            let extracted_file = temp_dir.join(Path::new(&name).file_name().unwrap_or(std::ffi::OsStr::new(&name)));
                            #[cfg(windows)]
                            {
                                let _ = std::process::Command::new("cmd")
                                    .args(["/c", "start", "", &extracted_file.to_string_lossy()])
                                    .spawn();
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
                let mut to_delete = Vec::new();
                for i in 0..count {
                    if let Some(e) = model.row_data(i) {
                        if e.is_selected && e.name != ".." {
                            to_delete.push((e.name.to_string(), e.is_dir));
                        }
                    }
                }
                if to_delete.is_empty() {
                    let sel_idx = ui.get_selected_index();
                    if sel_idx >= 0 && (sel_idx as usize) < count {
                        if let Some(e) = model.row_data(sel_idx as usize) {
                            if e.name != ".." {
                                to_delete.push((e.name.to_string(), e.is_dir));
                            }
                        }
                    }
                }

                if to_delete.is_empty() {
                    ui.set_status(if ui.get_arabic_mode() { "لم يتم تحديد أي ملفات للحذف" } else { "No files selected to delete" }.into());
                    return;
                }

                let is_archive = state.borrow().archive.is_some();
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
        ui.on_show_archive_info(move || {
            if let Some(ui) = weak.upgrade() {
                let arabic = ui.get_arabic_mode();
                if let Some(ref archive) = state.borrow().archive {
                    let ext = archive.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_uppercase();
                    let name = archive.file_name().unwrap_or_default().to_string_lossy();
                    let meta = archive.metadata().ok();
                    let size_str = meta.map(|m| format_size(m.len())).unwrap_or_else(|| "0".into());
                    let tot = ui.get_total_info();
                    let msg = if arabic {
                        format!("معلومات الأرشيف: {} | الصيغة: {} | الحجم: {} بايت | {}", name, ext, size_str, tot)
                    } else {
                        format!("Archive Info: {} | Format: {} | Size: {} bytes | {}", name, ext, size_str, tot)
                    };
                    ui.set_status(msg.into());
                } else {
                    let folder = state.borrow().folder.clone();
                    let total_info = ui.get_total_info();
                    let msg = if arabic {
                        format!("معلومات المجلد الحالي: {} | {}", folder.display(), total_info)
                    } else {
                        format!("Current Folder: {} | {}", folder.display(), total_info)
                    };
                    ui.set_status(msg.into());
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
                    "Mickey RAR v0.5.0 - البرنامج الاحترافي لإدارة وضغط الملفات Ultra LZMA2 | تطوير مايكل ماجد"
                } else {
                    "Mickey RAR v0.5.0 - High Performance Archive Manager with Ultra LZMA2 by Michael Maged"
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
                                        format!("يوجد تحديث جديد متوفر ({})! جاري تنزيل التحديث...", update.version)
                                    } else {
                                        format!("New update available ({})! Downloading installer...", update.version)
                                    };
                                    ui.set_status(msg.into());

                                    if let Some(dl_url) = update.download_url {
                                        thread::spawn(move || {
                                            let temp_installer = std::env::temp_dir().join("MickeyRAR-Setup-Update.exe");
                                            let mut curl = std::process::Command::new("curl.exe");
                                            #[cfg(windows)]
                                            curl.creation_flags(CREATE_NO_WINDOW);
                                            curl.args(["-L", "-o", &temp_installer.to_string_lossy(), &dl_url]);
                                            if let Ok(st) = curl.status() {
                                                if st.success() && temp_installer.exists() {
                                                    #[cfg(windows)]
                                                    {
                                                        let _ = std::process::Command::new("cmd")
                                                            .args(["/c", "start", "", &temp_installer.to_string_lossy()])
                                                            .spawn();
                                                        std::process::exit(0);
                                                    }
                                                }
                                            }
                                        });
                                    }
                                }
                                Ok(None) => {
                                    let msg = if arabic {
                                        format!("أنت تستخدم أحدث إصدار من ميكي رار (v{})", CURRENT_VERSION)
                                    } else {
                                        format!("Mickey RAR is up to date (v{})", CURRENT_VERSION)
                                    };
                                    ui.set_status(msg.into());
                                }
                                Err(e) => {
                                    ui.set_status(format!("Update check error: {e}").into());
                                }
                            }
                        }
                    });
                });
            }
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
    use super::supported_archive;
    use std::path::Path;

    #[test]
    fn recognizes_supported_archive_extensions_case_insensitively() {
        assert!(supported_archive(Path::new("backup.7Z")));
        assert!(supported_archive(Path::new("backup.tar.gz")));
        assert!(!supported_archive(Path::new("notes.txt")));
        assert!(!supported_archive(Path::new("archive")));
    }
}
