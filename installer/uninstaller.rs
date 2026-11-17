#![windows_subsystem = "windows"]

use std::{env, path::PathBuf, process::Command};
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;

fn delete_key(key: &str) {
    let mut command = Command::new("reg.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    let _ = command.args(["DELETE", key, "/f"]).status();
}

fn is_elevated() -> bool {
    let mut command = Command::new("fltmc");
    command.creation_flags(CREATE_NO_WINDOW);
    command.output().map(|output| output.status.success()).unwrap_or(false)
}

fn main() {
    let Ok(current) = env::current_exe() else { return; };
    if !is_elevated() {
        let escaped = current.display().to_string().replace('\'', "''");
        let command = format!("Start-Process -FilePath '{}' -Verb RunAs -Wait", escaped);
        let mut powershell = Command::new("powershell.exe");
        powershell.creation_flags(CREATE_NO_WINDOW);
        let _ = powershell.args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &command]).status();
        return;
    }

    for key in [
        r"HKLM\Software\Classes\*\shell\MickeyRAR",
        r"HKLM\Software\Classes\Directory\shell\MickeyRAR",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Add",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Compress",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Email",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractHere",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractToFolder",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractTo",
        r"HKCU\Software\Classes\*\shell\MickeyRAR",
        r"HKCU\Software\Classes\Directory\shell\MickeyRAR",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Add",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Compress",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.Email",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractHere",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractToFolder",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell\MickeyRAR.ExtractTo",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\MickeyRAR",
    ] { delete_key(key); }

    let Some(install_dir) = current.parent().map(PathBuf::from) else { return; };
    let mut command = Command::new("cmd.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    let command_line = format!("/c timeout /t 1 /nobreak >nul & rmdir /s /q \"{}\"", install_dir.display());
    let _ = command.arg(command_line).spawn();
}
