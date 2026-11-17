#![windows_subsystem = "windows"]

use std::{
    env,
    fs,
    path::PathBuf,
    process::Command,
};

const PROGRAM_BYTES: &[u8] = include_bytes!("../target/release/mickey-rar.exe");
const SEVEN_ZIP_BYTES: &[u8] = include_bytes!("runtime/7z.exe");

fn registry_value(key: &str, name: &str, value: &str) {
    let _ = Command::new("reg.exe")
        .args(["ADD", key, "/v", name, "/t", "REG_SZ", "/d", value, "/f"])
        .status();
}

fn register_context_menu(executable: &PathBuf) {
    let command_store = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\CommandStore\shell";
    let executable = executable.display().to_string();
    let commands = [
        ("MickeyRAR.Add", "Add to archive...", "--add"),
        ("MickeyRAR.Compress", "Compress with Mickey RAR", "--compress"),
        ("MickeyRAR.Email", "Compress and email...", "--compress-email"),
    ];

    for (verb, label, flag) in commands {
        let verb_key = format!(r"{command_store}\{verb}");
        let command_key = format!(r"{verb_key}\command");
        registry_value(&verb_key, "", label);
        registry_value(&command_key, "", &format!("\"{executable}\" {flag} \"%1\""));
    }

    for target in [
        r"HKCU\Software\Classes\*\shell\MickeyRAR",
        r"HKCU\Software\Classes\Directory\shell\MickeyRAR",
    ] {
        registry_value(target, "MUIVerb", "Mickey RAR");
        registry_value(target, "SubCommands", "MickeyRAR.Add;MickeyRAR.Compress;MickeyRAR.Email");
        registry_value(target, "Icon", &executable);
    }
}

fn main() {
    let Some(local_app_data) = env::var_os("LOCALAPPDATA") else {
        return;
    };

    let install_dir = PathBuf::from(local_app_data).join("Programs").join("Mickey RAR");
    let executable = install_dir.join("mickey-rar.exe");
    let seven_zip = install_dir.join("7z.exe");

    if fs::create_dir_all(&install_dir).is_err() {
        return;
    }
    if fs::write(&executable, PROGRAM_BYTES).is_err() {
        return;
    }
    if fs::write(&seven_zip, SEVEN_ZIP_BYTES).is_err() {
        return;
    }
    register_context_menu(&executable);

    let _ = Command::new(&executable).spawn();
}
