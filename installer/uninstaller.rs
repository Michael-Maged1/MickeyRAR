#![windows_subsystem = "windows"]

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;
const MOVEFILE_DELAY_UNTIL_REBOOT: u32 = 0x00000004;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x00000080;

extern "system" {
    fn MoveFileExW(
        lpExistingFileName: *const u16,
        lpNewFileName: *const u16,
        dwFlags: u32,
    ) -> i32;
    fn SetFileAttributesW(
        lpFileName: *const u16,
        dwFileAttributes: u32,
    ) -> i32;
}

fn delete_key(key: &str) {
    let mut command = Command::new("reg.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    let _ = command.args(["DELETE", key, "/f"]).status();
}

fn delete_reg_value(key: &str, value_name: &str) {
    let mut command = Command::new("reg.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    let _ = command.args(["DELETE", key, "/v", value_name, "/f"]).status();
}

fn is_elevated() -> bool {
    let mut command = Command::new("net.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    if let Ok(output) = command.args(["session"]).output() {
        if output.status.success() {
            return true;
        }
    }
    let mut command2 = Command::new("fltmc.exe");
    command2.creation_flags(CREATE_NO_WINDOW);
    command2.output().map(|output| output.status.success()).unwrap_or(false)
}

fn relaunch_as_admin(current_exe: &Path) {
    let escaped = current_exe.display().to_string().replace('\'', "''");
    let command = format!("Start-Process -FilePath '{}' -Verb RunAs -Wait", escaped);
    let mut powershell = Command::new("powershell.exe");
    powershell.creation_flags(CREATE_NO_WINDOW);
    let _ = powershell
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &command])
        .status();
}

fn query_reg_value_string(key: &str, value_name: &str) -> Option<String> {
    let mut cmd = Command::new("reg.exe");
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.args(["QUERY", key, "/v", value_name]);
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && parts[0].eq_ignore_ascii_case(value_name) {
            let val = parts[2..].join(" ");
            return Some(val);
        }
    }
    None
}

fn expand_env_string(input: &str) -> String {
    let mut result = input.to_string();
    for (k, v) in env::vars() {
        let pattern = format!("%{k}%");
        if result.contains(&pattern) {
            result = result.replace(&pattern, &v);
        }
    }
    result
}

fn get_default_install_dir(current_exe: &Path) -> PathBuf {
    if let Some(parent) = current_exe.parent() {
        let p_str = parent.to_string_lossy().to_lowercase();
        if !p_str.contains("temp") && !p_str.contains("appdata\\local\\temp") {
            return parent.to_path_buf();
        }
    }

    if let Some(loc) = query_reg_value_string(
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\MickeyRAR",
        "InstallLocation",
    ) {
        let p = PathBuf::from(loc.trim().trim_matches('"'));
        if p.exists() {
            return p;
        }
    }

    let pf = env::var("ProgramW6432")
        .or_else(|_| env::var("ProgramFiles"))
        .unwrap_or_else(|_| r"C:\Program Files".into());
    PathBuf::from(pf).join("Mickey RAR")
}

fn terminate_processes() {
    for img in &["mickey-rar.exe", "7z.exe"] {
        let mut cmd = Command::new("taskkill.exe");
        cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.args(["/F", "/IM", img, "/T"]).status();
    }
    thread::sleep(Duration::from_millis(600));
}

fn delete_shortcuts() {
    let mut dirs_to_check: Vec<PathBuf> = Vec::new();

    // 1. Current user desktop via User Shell Folders (handles OneDrive desktop redirection)
    if let Some(path) = query_reg_value_string(
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders",
        "Desktop",
    ) {
        let expanded = expand_env_string(&path);
        dirs_to_check.push(PathBuf::from(expanded));
    }

    // 2. Common Desktop (Public)
    if let Some(path) = query_reg_value_string(
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders",
        "Common Desktop",
    ) {
        let expanded = expand_env_string(&path);
        dirs_to_check.push(PathBuf::from(expanded));
    }

    // 3. User profile desktop & OneDrive desktop paths
    if let Ok(userprofile) = env::var("USERPROFILE") {
        dirs_to_check.push(PathBuf::from(&userprofile).join("Desktop"));
        if let Ok(onedrive) = env::var("OneDrive") {
            dirs_to_check.push(PathBuf::from(&onedrive).join("Desktop"));
        }
        if let Ok(onedrive_c) = env::var("OneDriveConsumer") {
            dirs_to_check.push(PathBuf::from(&onedrive_c).join("Desktop"));
        }
        if let Ok(onedrive_b) = env::var("OneDriveCommercial") {
            dirs_to_check.push(PathBuf::from(&onedrive_b).join("Desktop"));
        }
    }
    if let Ok(public) = env::var("PUBLIC") {
        dirs_to_check.push(PathBuf::from(public).join("Desktop"));
    }
    dirs_to_check.push(PathBuf::from(r"C:\Users\Public\Desktop"));

    // 4. Start Menu Programs
    if let Some(path) = query_reg_value_string(
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders",
        "Programs",
    ) {
        let expanded = expand_env_string(&path);
        dirs_to_check.push(PathBuf::from(expanded));
    }
    if let Some(path) = query_reg_value_string(
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders",
        "Common Programs",
    ) {
        let expanded = expand_env_string(&path);
        dirs_to_check.push(PathBuf::from(expanded));
    }
    if let Ok(appdata) = env::var("APPDATA") {
        dirs_to_check.push(PathBuf::from(&appdata).join(r"Microsoft\Windows\Start Menu\Programs"));
        dirs_to_check.push(PathBuf::from(&appdata).join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar"));
    }
    if let Ok(programdata) = env::var("PROGRAMDATA") {
        dirs_to_check.push(PathBuf::from(&programdata).join(r"Microsoft\Windows\Start Menu\Programs"));
    }

    for dir in dirs_to_check {
        if !dir.exists() {
            continue;
        }

        for name in &["Mickey RAR.lnk", "MickeyRAR.lnk", "mickey-rar.lnk"] {
            let lnk = dir.join(name);
            if lnk.exists() {
                let _ = fs::remove_file(&lnk);
            }
        }

        let subfolder = dir.join("Mickey RAR");
        if subfolder.is_dir() {
            let _ = fs::remove_dir_all(&subfolder);
        }
        let subfolder2 = dir.join("MickeyRAR");
        if subfolder2.is_dir() {
            let _ = fs::remove_dir_all(&subfolder2);
        }

        // Scan all .lnk in the directory for Mickey references
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let is_lnk = path
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.eq_ignore_ascii_case("lnk"))
                        .unwrap_or(false);
                    if is_lnk {
                        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                        if file_name.to_lowercase().contains("mickey") {
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
            }
        }
    }
}

fn delete_registry_entries() {
    let keys_to_delete = [
        // Shell Context Menu HKLM & HKCU
        r"HKLM\Software\Classes\*\shell\MickeyRAR",
        r"HKLM\Software\Classes\Directory\shell\MickeyRAR",
        r"HKLM\Software\Classes\Directory\Background\shell\MickeyRAR",
        r"HKLM\Software\Classes\Drive\shell\MickeyRAR",
        r"HKCU\Software\Classes\*\shell\MickeyRAR",
        r"HKCU\Software\Classes\Directory\shell\MickeyRAR",
        r"HKCU\Software\Classes\Directory\Background\shell\MickeyRAR",
        r"HKCU\Software\Classes\Drive\shell\MickeyRAR",

        // CommandStore subcommands HKLM & HKCU
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Open",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractHere",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractToFolder",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractTo",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Add",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Compress",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Email",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Open",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractHere",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractToFolder",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractTo",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Add",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Compress",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Email",

        // WOW6432Node equivalents
        r"HKLM\Software\WOW6432Node\Classes\*\shell\MickeyRAR",
        r"HKLM\Software\WOW6432Node\Classes\Directory\shell\MickeyRAR",
        r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Open",
        r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractHere",
        r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractToFolder",
        r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractTo",
        r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Add",
        r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Compress",
        r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Email",

        // ProgID
        r"HKLM\Software\Classes\MickeyRAR.Archive",
        r"HKCU\Software\Classes\MickeyRAR.Archive",
        r"HKLM\Software\WOW6432Node\Classes\MickeyRAR.Archive",

        // Uninstall keys
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\MickeyRAR",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\MickeyRAR",
        r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\MickeyRAR",

        // App keys
        r"HKCU\Software\Mickey RAR",
        r"HKLM\Software\Mickey RAR",
        r"HKCU\Software\MickeyRAR",
        r"HKLM\Software\MickeyRAR",
        r"HKLM\Software\WOW6432Node\Mickey RAR",
        r"HKLM\Software\WOW6432Node\MickeyRAR",
    ];

    for key in keys_to_delete {
        delete_key(key);
    }

    // Clean OpenWithProgids for all supported archive formats
    for ext in &[".zip", ".rar", ".7z", ".tar", ".gz", ".bz2", ".xz", ".iso", ".tgz", ".tbz2", ".cab", ".wim"] {
        delete_reg_value(&format!(r"HKLM\Software\Classes\{ext}\OpenWithProgids"), "MickeyRAR.Archive");
        delete_reg_value(&format!(r"HKCU\Software\Classes\{ext}\OpenWithProgids"), "MickeyRAR.Archive");
        delete_reg_value(&format!(r"HKLM\Software\WOW6432Node\Classes\{ext}\OpenWithProgids"), "MickeyRAR.Archive");
    }
}

fn delete_appdata() {
    if let Ok(appdata) = env::var("APPDATA") {
        let p = PathBuf::from(appdata).join("Mickey RAR");
        let _ = fs::remove_dir_all(&p);
    }
    if let Ok(localappdata) = env::var("LOCALAPPDATA") {
        let p = PathBuf::from(&localappdata).join("Mickey RAR");
        let _ = fs::remove_dir_all(&p);
        let p2 = PathBuf::from(&localappdata).join(r"Programs\Mickey RAR");
        let _ = fs::remove_dir_all(&p2);
    }
    let temp_drag = env::temp_dir().join("MickeyRAR_Drag");
    let _ = fs::remove_dir_all(&temp_drag);
}

fn schedule_reboot_delete(path: &Path) {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        MoveFileExW(wide.as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT);
    }
}

fn clear_read_only_recursive(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                clear_read_only_recursive(&p);
            } else {
                let wide: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
                unsafe {
                    SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_NORMAL);
                }
                let _ = fs::remove_file(&p);
            }
        }
    }
}

fn schedule_reboot_delete_recursive(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                schedule_reboot_delete_recursive(&p);
            } else {
                schedule_reboot_delete(&p);
            }
        }
    }
    schedule_reboot_delete(dir);
}

fn delete_directory_robust(dir: &Path) {
    if !dir.exists() {
        return;
    }

    // Try up to 15 times with 200ms delay between attempts
    for _ in 0..15 {
        clear_read_only_recursive(dir);
        let _ = fs::remove_dir_all(dir);
        if !dir.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }

    // If still locked, schedule reboot deletion for all remaining items and the folder
    schedule_reboot_delete_recursive(dir);
}

fn schedule_self_delete(helper_exe: &Path) {
    schedule_reboot_delete(helper_exe);

    let escaped = helper_exe.display().to_string().replace('"', "\\\"");
    let cmd_line = format!("ping 127.0.0.1 -n 3 >nul & del /f /q \"{escaped}\"");
    let mut cmd = Command::new("cmd.exe");
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.args(["/c", &cmd_line]);
    let _ = cmd.spawn();
}

fn notify_shell() {
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

fn main() {
    let args: Vec<String> = env::args().collect();
    let is_helper = args.iter().any(|a| a == "--helper");

    let Ok(current) = env::current_exe() else { return; };

    let target_dir: PathBuf = if let Some(pos) = args.iter().position(|a| a == "--target") {
        if pos + 1 < args.len() {
            PathBuf::from(&args[pos + 1])
        } else {
            get_default_install_dir(&current)
        }
    } else {
        get_default_install_dir(&current)
    };

    if !is_helper {
        if !is_elevated() {
            relaunch_as_admin(&current);
            return;
        }

        let temp_exe = env::temp_dir().join("MickeyRAR_Uninstall_Helper.exe");
        if fs::copy(&current, &temp_exe).is_ok() {
            let mut cmd = Command::new(&temp_exe);
            cmd.creation_flags(CREATE_NO_WINDOW);
            cmd.arg("--helper");
            cmd.arg("--target");
            cmd.arg(&target_dir);
            if cmd.spawn().is_ok() {
                std::process::exit(0);
            }
        }
    }

    // Helper process:
    // 1. Give the original uninstall.exe process time to exit
    thread::sleep(Duration::from_millis(500));

    // 2. Terminate any running processes
    terminate_processes();

    // 3. Delete shortcuts everywhere (Desktop, Start Menu, Quick Launch, OneDrive)
    delete_shortcuts();

    // 4. Delete registry entries
    delete_registry_entries();

    // 5. Delete AppData / LocalAppData / Temp
    delete_appdata();

    // 6. Delete installation directory with retry loop and reboot fallback
    delete_directory_robust(&target_dir);

    // Also check if C:\Program Files\Mickey RAR exists even if target_dir was different
    let default_pf = PathBuf::from(r"C:\Program Files\Mickey RAR");
    if default_pf != target_dir && default_pf.exists() {
        delete_directory_robust(&default_pf);
    }

    // 7. Notify Windows Shell to refresh desktop, start menu, and file icons immediately
    notify_shell();

    // 8. Schedule deletion of our temporary helper executable
    if is_helper {
        schedule_self_delete(&current);
    }
}
