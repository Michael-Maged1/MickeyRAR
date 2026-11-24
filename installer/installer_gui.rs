#![windows_subsystem = "windows"]

use slint::{slint, ComponentHandle, Timer, TimerMode};
use std::{cell::RefCell, env, fs, path::PathBuf, process::Command, rc::Rc, time::Duration};
use std::os::windows::process::CommandExt;

#[path = "../src/winicon.rs"]
mod winicon;

const CREATE_NO_WINDOW: u32 = 0x08000000;

fn center_window(window: &slint::Window) {
    use slint::PhysicalPosition;
    unsafe extern "system" { fn GetSystemMetrics(index: i32) -> i32; }
    let (screen_width, screen_height) = unsafe { (GetSystemMetrics(0), GetSystemMetrics(1)) };
    let size = window.size();
    let x = ((screen_width - size.width as i32) / 2).max(0);
    let y = ((screen_height - size.height as i32) / 2).max(0);
    window.set_position(PhysicalPosition::new(x, y));
}

slint! {
    export component InstallerWindow inherits Window {
        title: "Mickey RAR";
        icon: @image-url("../assets/icon-window.png");
        width: 420px;
        height: 150px;
        background: #17191b;
        in-out property <float> progress: 0.0;

        Rectangle {
            x: 18px;
            y: 18px;
            width: 38px;
            height: 38px;
            border-radius: 9px;
            background: #e8573f;
            Image { source: @image-url("../assets/icon.png"); width: 38px; height: 38px; image-fit: contain; }
        }
        Text { x: 68px; y: 20px; text: "Mickey RAR"; color: #f4f7ef; font-size: 20px; font-weight: 700; }
        Text { x: 68px; y: 51px; text: "Installing Mickey RAR..."; color: #a8b1aa; font-size: 12px; }
        Rectangle {
            x: 18px;
            y: 98px;
            width: parent.width - 36px;
            height: 8px;
            border-radius: 4px;
            background: #303638;
            Rectangle {
                x: parent.width - root.progress * parent.width;
                width: root.progress * parent.width;
                height: parent.height;
                border-radius: 4px;
                background: #e8573f;
            }
        }
    }
}

const PROGRAM_BYTES: &[u8] = include_bytes!("../target/release/mickey-rar.exe");
const SEVEN_ZIP_BYTES: &[u8] = include_bytes!("runtime/7z.exe");
const SEVEN_ZIP_DLL_BYTES: &[u8] = include_bytes!("runtime/7z.dll");
const SEVEN_ZIP_SFX_BYTES: &[u8] = include_bytes!("runtime/7z.sfx");
const UNINSTALLER_BYTES: &[u8] = include_bytes!("../target/release/mickey-rar-uninstaller.exe");
const ICON_BYTES: &[u8] = include_bytes!("../assets/icon.ico");

fn registry_value(key: &str, name: &str, value: &str) {
    let mut command = Command::new("reg.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    command.args(["ADD", key]);
    if name.is_empty() { command.arg("/ve"); } else { command.args(["/v", name]); }
    let _ = command.args(["/t", "REG_SZ", "/d", value, "/f"]).status();
}

fn registry_delete(key: &str) {
    let mut command = Command::new("reg.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    let _ = command.args(["DELETE", key, "/f"]).status();
}

fn is_elevated() -> bool {
    let mut command = Command::new("fltmc");
    command.creation_flags(CREATE_NO_WINDOW);
    command.output().map(|output| output.status.success()).unwrap_or(false)
}

fn relaunch_as_admin() -> Result<(), Box<dyn std::error::Error>> {
    let current = env::current_exe()?;
    let escaped = current.display().to_string().replace('\'', "''");
    let command = format!("Start-Process -FilePath '{}' -Verb RunAs", escaped);
    let mut powershell = Command::new("powershell.exe");
    powershell.creation_flags(CREATE_NO_WINDOW);
    let _ = powershell.args(["-NoProfile", "-Command", &command])
        .status()?;
    Ok(())
}

fn register_context_menu(executable: &PathBuf) {
    for old_key in [
        r"HKCU\Software\Classes\*\shell\MickeyRAR",
        r"HKCU\Software\Classes\Directory\shell\MickeyRAR",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Open",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Add",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Compress",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Email",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractHere",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractToFolder",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractTo",
    ] { registry_delete(old_key); }
    let command_store = r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell";
    let executable = executable.display().to_string();
    let commands = [
        ("MickeyRAR.Open", "Open with Mickey RAR (فتح باستخدام Mickey RAR)", ""),
        ("MickeyRAR.ExtractHere", "Extract Here (فك الضغط هنا)", "--extract-here"),
        ("MickeyRAR.ExtractToFolder", "Extract to subfolder (استخراج إلى مجلد)", "--extract-to-folder"),
        ("MickeyRAR.ExtractTo", "Extract files... (استخراج إلى...)", "--extract-to"),
        ("MickeyRAR.Add", "Add to archive with Mickey RAR...", "--add"),
        ("MickeyRAR.Compress", "Compress to ZIP with Mickey RAR", "--compress"),
        ("MickeyRAR.Email", "Compress to ZIP and email...", "--compress-email"),
    ];
    for (verb, label, flag) in commands {
        let verb_key = format!(r"{command_store}\{verb}");
        let command_key = format!(r"{verb_key}\command");
        registry_value(&verb_key, "", label);
        registry_value(&verb_key, "Icon", &format!("\"{executable}\",0"));
        let cmd = if flag.is_empty() {
            format!("\"{executable}\" \"%1\"")
        } else {
            format!("\"{executable}\" {flag} \"%1\"")
        };
        registry_value(&command_key, "", &cmd);
    }
    for target in [
        r"HKLM\Software\Classes\*\shell\MickeyRAR",
        r"HKLM\Software\Classes\Directory\shell\MickeyRAR",
        r"HKCU\Software\Classes\*\shell\MickeyRAR",
        r"HKCU\Software\Classes\Directory\shell\MickeyRAR",
    ] {
        registry_value(target, "MUIVerb", "Mickey RAR");
        registry_value(target, "SubCommands", "MickeyRAR.Open;MickeyRAR.ExtractHere;MickeyRAR.ExtractToFolder;MickeyRAR.ExtractTo;MickeyRAR.Add;MickeyRAR.Compress;MickeyRAR.Email");
        registry_value(target, "Icon", &format!("\"{executable}\",0"));
    }

    let extensions = [
        ".zip", ".rar", ".7z", ".tar", ".gz", ".bz2", ".xz", ".iso", ".cab", ".wim", ".tgz", ".tbz2", ".txz",
    ];

    for base in [r"HKLM\Software\Classes", r"HKCU\Software\Classes"] {
        let prog_id = format!(r"{base}\MickeyRAR.Archive");
        registry_value(&prog_id, "", "Mickey RAR Archive");
        registry_value(&format!(r"{prog_id}\DefaultIcon"), "", &format!("\"{executable}\",0"));
        registry_value(&format!(r"{prog_id}\shell"), "", "open");
        registry_value(&format!(r"{prog_id}\shell\open"), "FriendlyAppName", "Mickey RAR");
        registry_value(&format!(r"{prog_id}\shell\open\command"), "", &format!("\"{executable}\" \"%1\""));

        for ext in &extensions {
            let ext_key = format!(r"{base}\{ext}");
            registry_value(&ext_key, "", "MickeyRAR.Archive");
            let ext_open = format!(r"{ext_key}\OpenWithProgids");
            registry_value(&ext_open, "MickeyRAR.Archive", "");
        }
    }

    // Clear broken/orphaned UserChoice in FileExts so Windows falls back to our ProgID
    for ext in &extensions {
        registry_delete(&format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{ext}\UserChoice"));
        registry_delete(&format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{ext}\UserChoiceLatest"));
    }

    // Register Capabilities for Windows 10/11 Default Apps
    for base in [r"HKLM\Software", r"HKCU\Software"] {
        let cap = format!(r"{base}\MickeyRAR\Capabilities");
        registry_value(&cap, "ApplicationName", "Mickey RAR");
        registry_value(&cap, "ApplicationDescription", "Mickey RAR archive manager");
        for ext in &extensions {
            registry_value(&format!(r"{cap}\FileAssociations"), ext, "MickeyRAR.Archive");
        }
        registry_value(&format!(r"{base}\RegisteredApplications"), "MickeyRAR", r"Software\MickeyRAR\Capabilities");
    }

    // Register Applications\mickey-rar.exe
    for base in [r"HKLM\Software\Classes\Applications", r"HKCU\Software\Classes\Applications"] {
        let app_entry = format!(r"{base}\mickey-rar.exe");
        registry_value(&app_entry, "FriendlyAppName", "Mickey RAR");
        registry_value(&format!(r"{app_entry}\DefaultIcon"), "", &format!("\"{executable}\",0"));
        registry_value(&format!(r"{app_entry}\shell\open\command"), "", &format!("\"{executable}\" \"%1\""));
        for ext in &extensions {
            registry_value(&format!(r"{app_entry}\SupportedTypes"), ext, "");
        }
    }

    // App Paths
    let app_paths = r"HKLM\Software\Microsoft\Windows\CurrentVersion\App Paths\mickey-rar.exe";
    registry_value(app_paths, "", &executable);
    if let Some(parent) = std::path::Path::new(&executable).parent() {
        registry_value(app_paths, "Path", &parent.display().to_string());
    }

    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::UI::Shell::SHChangeNotify(
            windows_sys::Win32::UI::Shell::SHCNE_ASSOCCHANGED as i32,
            windows_sys::Win32::UI::Shell::SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

fn register_uninstall(install_dir: &PathBuf, executable: &PathBuf, uninstaller: &PathBuf) {
    let key = r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\MickeyRAR";
    let values = [
        ("DisplayName", "Mickey RAR".to_string()),
        ("DisplayVersion", env!("CARGO_PKG_VERSION").to_string()),
        ("Publisher", "Michael Maged".to_string()),
        ("InstallLocation", install_dir.display().to_string()),
        ("DisplayIcon", executable.display().to_string()),
        ("UninstallString", format!("\"{}\"", uninstaller.display())),
        ("QuietUninstallString", format!("\"{}\"", uninstaller.display())),
        ("WindowsInstaller", "0".to_string()),
        ("NoModify", "1".to_string()),
        ("NoRepair", "1".to_string()),
    ];
    for (name, value) in values { registry_value(key, name, &value); }
}

fn create_desktop_shortcut(executable: &PathBuf) {
    let target = executable.display().to_string().replace('\'', "''");
    let command = format!(
        "$desktop=[Environment]::GetFolderPath('Desktop'); $shell=New-Object -ComObject WScript.Shell; $link=$shell.CreateShortcut((Join-Path $desktop 'Mickey RAR.lnk')); $link.TargetPath='{}'; $link.WorkingDirectory='{}'; $link.IconLocation='{},0'; $link.Description='Mickey RAR archive manager'; $link.Save()",
        target,
        executable.parent().unwrap_or(executable).display().to_string().replace('\'', "''"),
        target
    );
    let mut powershell = Command::new("powershell.exe");
    powershell.creation_flags(CREATE_NO_WINDOW);
    let _ = powershell.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &command]).status();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !is_elevated() {
        relaunch_as_admin()?;
        return Ok(());
    }
    let ui = InstallerWindow::new()?;
    let center_timer = Timer::default();
    let center_weak = ui.as_weak();
    center_timer.start(TimerMode::SingleShot, Duration::from_millis(120), move || {
        if let Some(ui) = center_weak.upgrade() { center_window(&ui.window()); }
        winicon::install();
    });
    let progress = Rc::new(RefCell::new(0.0_f32));
    let progress_state = progress.clone();
    let ui_weak = ui.as_weak();
    let timer = Timer::default();

    timer.start(TimerMode::Repeated, Duration::from_millis(120), move || {
        let Some(ui) = ui_weak.upgrade() else { return; };
        let mut value = progress_state.borrow_mut();
        *value = (*value + 0.12).min(1.0);
        ui.set_progress(*value);
        if *value < 1.0 { return; }

        let program_files = env::var_os("ProgramW6432")
            .or_else(|| env::var_os("ProgramFiles"))
            .unwrap_or_else(|| "C:\\Program Files".into());
        let install_dir = PathBuf::from(program_files).join("Mickey RAR");
        let executable = install_dir.join("mickey-rar.exe");
        let seven_zip = install_dir.join("7z.exe");
        let seven_zip_dll = install_dir.join("7z.dll");
        let seven_zip_sfx = install_dir.join("7z.sfx");
        let uninstaller = install_dir.join("uninstall.exe");
        let icon_file = install_dir.join("icon.ico");

        // Close any running Mickey RAR instance so Windows releases file locks during upgrade
        let mut kill_cmd = Command::new("taskkill.exe");
        kill_cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = kill_cmd.args(["/F", "/IM", "mickey-rar.exe"]).status();

        if fs::create_dir_all(&install_dir).is_err() {
            std::process::exit(1);
        }
        let mut wrote_exe = false;
        for _ in 0..15 {
            if fs::write(&executable, PROGRAM_BYTES).is_ok() {
                wrote_exe = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        if !wrote_exe
            || fs::write(&seven_zip, SEVEN_ZIP_BYTES).is_err()
            || fs::write(&seven_zip_dll, SEVEN_ZIP_DLL_BYTES).is_err()
            || fs::write(&seven_zip_sfx, SEVEN_ZIP_SFX_BYTES).is_err()
            || fs::write(&uninstaller, UNINSTALLER_BYTES).is_err()
            || fs::write(&icon_file, ICON_BYTES).is_err()
        {
            std::process::exit(1);
        }
        register_context_menu(&executable);
        register_uninstall(&install_dir, &executable, &uninstaller);
        create_desktop_shortcut(&executable);

        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::UI::Shell::SHChangeNotify(
                windows_sys::Win32::UI::Shell::SHCNE_ASSOCCHANGED as i32,
                windows_sys::Win32::UI::Shell::SHCNF_IDLIST,
                std::ptr::null(),
                std::ptr::null(),
            );
        }

        let _ = Command::new(&executable).spawn();
        std::process::exit(0);
    });

    ui.run()?;
    Ok(())
}
