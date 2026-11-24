//! 7-Zip command-line integration. Each argument is passed separately, never via a shell.
use std::{ffi::OsString, io::Read, path::{Path, PathBuf}, process::{Command, Stdio}, sync::{atomic::{AtomicBool, AtomicU8, Ordering}, Arc}, thread, time::Duration};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;
#[cfg(windows)]
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x00004000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressionSettings {
    /// Compression level from 0 (store) to 9 (maximum).
    pub level: u8,
    pub solid: bool,
    /// None => let 7-Zip choose the number of threads.
    pub threads: Option<u16>,
}
impl Default for CompressionSettings {
    fn default() -> Self { Self { level: 9, solid: true, threads: None } }
}
impl CompressionSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.level > 9 { return Err("Compression level must be from 0 to 9".into()); }
        if matches!(self.threads, Some(0) | Some(257..)) { return Err("Thread count must be from 1 to 256".into()); }
        Ok(())
    }
    #[allow(dead_code)]
    pub fn ultra_arguments(&self) -> Result<Vec<OsString>, String> {
        self.validate()?;
        let mut args = vec!["-t7z".into(), format!("-mx={}", self.level).into(), "-m0=LZMA2".into(),
            (if self.solid { "-ms=on" } else { "-ms=off" }).into()];
        if let Some(threads) = self.threads { args.push(format!("-mmt={threads}").into()); }
        Ok(args)
    }

    pub fn zip_arguments(&self) -> Result<Vec<OsString>, String> {
        self.validate()?;
        let mut args = vec![OsString::from("-tzip"), format!("-mx={}", self.level).into(), OsString::from("-mfb=258")];
        if let Some(threads) = self.threads { args.push(format!("-mmt={threads}").into()); }
        Ok(args)
    }
}

/// Return the archive arguments, keeping source paths as individual arguments.
/// Change cwd to a common parent so paths don't become absolute entries in archives.
#[allow(dead_code)]
pub fn create_arguments(output: &Path, inputs: &[PathBuf], settings: &CompressionSettings)
    -> Result<(PathBuf, Vec<OsString>), String> {
    create_arguments_for_format(output, inputs, settings, "zip")
}

#[allow(dead_code)]
pub fn create_arguments_for_format(output: &Path, inputs: &[PathBuf], settings: &CompressionSettings, format: &str)
    -> Result<(PathBuf, Vec<OsString>), String> {
    if inputs.is_empty() { return Err("Choose at least one file".into()); }
    if !output.is_absolute() { return Err("Choose an absolute output path".into()); }
    let parent = inputs[0].parent().ok_or("Cannot determine file parent")?.to_path_buf();
    if !parent.is_absolute() { return Err("Input paths must be absolute".into()); }
    if !inputs.iter().all(|p| p.parent() == Some(parent.as_path())) {
        return Err("Select files from the same folder for this release".into());
    }
    for input in inputs {
        if !input.exists() { return Err(format!("Input not found: {}", input.display())); }
        if input == output { return Err("The output archive cannot overwrite an input".into()); }
    }
    let mut args = vec![OsString::from("a")];
    match format.to_ascii_lowercase().as_str() {
        "zip" => args.extend(settings.zip_arguments()?),
        "7z" => args.extend(settings.ultra_arguments()?),
        _ => return Err("Only ZIP and 7Z formats are supported".into()),
    }
    args.push(output.as_os_str().to_owned());
    for input in inputs {
        let name = input.file_name().ok_or("Invalid input name")?;
        if name.to_string_lossy().starts_with('-') { return Err("Names beginning with '-' are not yet supported".into()); }
        args.push(name.to_owned());
    }
    Ok((parent, args))
}

#[derive(Clone, Debug, Default)]
pub struct ArchiveCreationOptions {
    pub format: String,
    pub level: u8,
    pub solid: bool,
    pub threads: Option<u16>,
    pub password: Option<String>,
    pub encrypt_headers: bool,
}

pub fn candidate_7z_executables() -> Vec<PathBuf> {
    let mut executables = Vec::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            executables.push(dir.join(if cfg!(target_os = "windows") { "7z.exe" } else { "7zz" }));
        }
    }
    if cfg!(target_os = "windows") {
        executables.extend([
            r"installer\runtime\7z.exe",
            r"C:\Program Files\Mickey RAR\7z.exe",
            r"C:\Program Files\7-Zip\7z.exe",
            r"C:\Program Files (x86)\7-Zip\7z.exe",
            "7z.exe", "7zz.exe", "7z", "7zz",
        ].into_iter().map(PathBuf::from));
    } else {
        executables.extend(["7zz", "7z"].into_iter().map(PathBuf::from));
    }
    executables
}

pub fn find_sfx_module() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            candidates.push(dir.join("7z.sfx"));
            candidates.push(dir.join("7zCon.sfx"));
        }
    }
    if cfg!(target_os = "windows") {
        candidates.extend([
            PathBuf::from(r"installer\runtime\7z.sfx"),
            PathBuf::from(r"C:\Program Files\Mickey RAR\7z.sfx"),
            PathBuf::from(r"C:\Program Files\7-Zip\7z.sfx"),
            PathBuf::from(r"C:\Program Files (x86)\7-Zip\7z.sfx"),
        ]);
    }
    candidates.into_iter().find(|p| p.is_file())
}

pub fn create_archive_command_args(
    output: &Path,
    inputs: &[PathBuf],
    opts: &ArchiveCreationOptions,
) -> Result<(PathBuf, Vec<OsString>), String> {
    if inputs.is_empty() { return Err("Choose at least one file or folder".into()); }
    if !output.is_absolute() { return Err("Choose an absolute output path".into()); }
    let settings = CompressionSettings {
        level: opts.level,
        solid: opts.solid,
        threads: opts.threads,
    };
    settings.validate()?;

    let first_parent = inputs[0].parent().ok_or("Cannot determine file parent")?.to_path_buf();
    let same_parent = first_parent.is_absolute()
        && inputs.iter().all(|p| p.parent() == Some(first_parent.as_path()));

    for input in inputs {
        if !input.exists() { return Err(format!("Input not found: {}", input.display())); }
        if input == output { return Err("The output archive cannot overwrite an input".into()); }
    }

    let mut args = vec![OsString::from("a"), OsString::from("-y")];
    let fmt = opts.format.to_ascii_lowercase();
    match fmt.as_str() {
        "zip" => {
            args.extend(settings.zip_arguments()?);
            if let Some(ref pwd) = opts.password {
                if !pwd.is_empty() {
                    args.push(OsString::from(format!("-p{pwd}")));
                    args.push(OsString::from("-mem=AES256"));
                }
            }
        }
        "7z" => {
            args.extend(settings.ultra_arguments()?);
            if let Some(ref pwd) = opts.password {
                if !pwd.is_empty() {
                    args.push(OsString::from(format!("-p{pwd}")));
                    if opts.encrypt_headers {
                        args.push(OsString::from("-mhe=on"));
                    }
                }
            }
        }
        "sfx" => {
            let sfx_mod = find_sfx_module()
                .ok_or_else(|| "7z.sfx module was not found for creating SFX archive".to_string())?;
            let mut sfx_arg = OsString::from("-sfx");
            sfx_arg.push(sfx_mod.as_os_str());
            args.push(sfx_arg);
            args.extend(settings.ultra_arguments()?);
            if let Some(ref pwd) = opts.password {
                if !pwd.is_empty() {
                    args.push(OsString::from(format!("-p{pwd}")));
                    if opts.encrypt_headers {
                        args.push(OsString::from("-mhe=on"));
                    }
                }
            }
        }
        _ => return Err(format!("Unsupported creation format: {}", opts.format)),
    }

    args.push(output.as_os_str().to_owned());
    args.push(OsString::from("--"));
    if same_parent {
        for input in inputs {
            let name = input.file_name().ok_or("Invalid input name")?;
            args.push(name.to_owned());
        }
        Ok((first_parent, args))
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        for input in inputs {
            args.push(input.as_os_str().to_owned());
        }
        Ok((cwd, args))
    }
}

pub fn run_7z(args: &[OsString], cwd: Option<&Path>) -> Result<String, String> {
    let executables = candidate_7z_executables();
    for executable in executables {
        let mut cmd = Command::new(&executable);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
        cmd.args(args);
        if let Some(dir) = cwd { cmd.current_dir(dir); }
        match cmd.output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                if !output.status.success() { return Err(format!("7-Zip failed ({}): {} {}", output.status, stdout, stderr)); }
                return Ok(stdout);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("Cannot start 7-Zip: {e}")),
        }
    }
    Err("7-Zip executable (7zz or 7z) was not found in PATH".into())
}

/// How a cancellable 7-Zip run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// 7-Zip exited successfully and the archive is complete.
    Finished,
    /// The caller asked to cancel; 7-Zip was killed and the output is incomplete.
    Cancelled,
}

/// Progress percentage (0-100), current file, and pause state for 7-Zip runs.
#[derive(Clone, Default)]
pub struct ProgressHandle {
    pub percent: Arc<AtomicU8>,
    pub current_file: Arc<std::sync::Mutex<String>>,
    pub paused: Arc<AtomicBool>,
}

impl ProgressHandle {
    pub fn new() -> Self {
        Self {
            percent: Arc::new(AtomicU8::new(0)),
            current_file: Arc::new(std::sync::Mutex::new(String::new())),
            paused: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[cfg(windows)]
fn set_process_paused(child: &std::process::Child, pause: bool) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtSuspendProcess(process_handle: HANDLE) -> i32;
        fn NtResumeProcess(process_handle: HANDLE) -> i32;
    }
    unsafe {
        let handle = child.as_raw_handle() as HANDLE;
        if !handle.is_null() {
            if pause {
                let _ = NtSuspendProcess(handle);
            } else {
                let _ = NtResumeProcess(handle);
            }
        }
    }
}

#[cfg(not(windows))]
fn set_process_paused(child: &std::process::Child, pause: bool) {
    let sig = if pause { "-STOP" } else { "-CONT" };
    let _ = Command::new("kill")
        .args([sig, &child.id().to_string()])
        .status();
}

/// Ask 7-Zip to print its own progress percentage (`-bsp1`) and store it in
/// `progress`, so the UI can show the real position instead of guessing. Large
/// archives spend a long time between 90% and 100%, which looked like a frozen
/// window before.
pub fn run_7z_cancellable_with_progress(
    args: &[OsString],
    cwd: Option<&Path>,
    cancelled: &AtomicBool,
    progress: Option<&ProgressHandle>,
) -> Result<RunOutcome, String> {
    let executables = candidate_7z_executables();
    for executable in executables {
        let mut command = Command::new(&executable);
        command.stdin(Stdio::null());
        let err_output = Arc::new(std::sync::Mutex::new(String::new()));
        let err_output_clone = err_output.clone();
        if progress.is_some() && !args.is_empty() {
            // Keep the operation switch first, then ask 7-Zip for progress data in UTF-8 with multi-threading.
            command.arg(&args[0])
                .arg("-bsp1")
                .arg("-mmt=on")
                .arg("-sccUTF-8")
                .args(&args[1..]);
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
        } else {
            command.arg("-mmt=on").arg("-sccUTF-8").args(args).stdout(Stdio::null()).stderr(Stdio::piped());
        }
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
        if let Some(dir) = cwd { command.current_dir(dir); }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Cannot start 7-Zip: {error}")),
        };
        // If this process is killed (Task Manager, crash, log off) the job object
        // takes 7-Zip down with it, so it can never keep the archive file locked.
        #[cfg(windows)]
        let _kill_on_close = KillOnCloseJob::attach(&child);
        let _err_reader = child.stderr.take().map(|mut stream| {
            thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = stream.read_to_end(&mut buf);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Ok(mut lock) = err_output_clone.lock() {
                    *lock = text;
                }
            })
        });
        // 7-Zip writes progress into a pipe: drain it in a helper thread, otherwise
        // the pipe buffer fills up and 7-Zip blocks (a real freeze).
        let reader = child.stdout.take().map(|stream| {
            let sink = progress.cloned();
            thread::spawn(move || read_progress_percentages(stream, sink))
        });
        let mut currently_paused = false;
        loop {
            if cancelled.load(Ordering::Relaxed) {
                if currently_paused {
                    set_process_paused(&child, false);
                }
                // Kill first, then wait for the process to really be gone: only
                // after that does Windows release the half-written archive.
                let _ = child.kill();
                let _ = child.wait();
                join_reader(reader);
                return Ok(RunOutcome::Cancelled);
            }
            let want_paused = progress
                .map(|p| p.paused.load(Ordering::Relaxed))
                .unwrap_or(false);
            if want_paused != currently_paused {
                set_process_paused(&child, want_paused);
                currently_paused = want_paused;
            }
            if currently_paused {
                thread::sleep(Duration::from_millis(80));
                continue;
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    join_reader(reader);
                    if status.success() {
                        return Ok(RunOutcome::Finished);
                    } else {
                        let err_text = err_output.lock().map(|l| l.clone()).unwrap_or_default();
                        let clean_err = err_text.trim();
                        if clean_err.contains("Wrong password")
                            || clean_err.contains("Break signaled")
                            || clean_err.contains("Can not open encrypted archive")
                        {
                            return Err("Wrong password or password required".into());
                        }
                        if !clean_err.is_empty() {
                            let first_err = clean_err.lines().find(|l| l.contains("ERROR:") || l.contains("Error")).unwrap_or(clean_err);
                            return Err(first_err.to_string());
                        }
                        return Err(format!("7-Zip failed ({status})"));
                    }
                }
                Ok(None) => thread::sleep(Duration::from_millis(80)),
                Err(error) => { join_reader(reader); return Err(format!("Could not read 7-Zip status: {error}")); }
            }
        }
    }
    Err("7-Zip executable (7zz or 7z) was not found in PATH".into())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArchiveEncryptionInfo {
    pub is_encrypted: bool,
    pub header_encrypted: bool,
    pub smallest_encrypted_member: Option<(String, u64)>,
}

fn parse_smallest_encrypted_from_slt(slt_output: &str, require_encrypted_flag: bool) -> (bool, Option<(String, u64)>) {
    let mut in_contents = false;
    let mut cur_path: Option<String> = None;
    let mut cur_size: u64 = 0;
    let mut cur_is_dir = false;
    let mut cur_enc = false;
    let mut any_encrypted = false;
    let mut smallest: Option<(String, u64)> = None;

    let flush = |path: &mut Option<String>,
                 size: u64,
                 is_dir: bool,
                 enc: bool,
                 any_enc_out: &mut bool,
                 smallest_out: &mut Option<(String, u64)>| {
        if let Some(p) = path.take() {
            if enc {
                *any_enc_out = true;
            }
            let eligible = !is_dir && !p.ends_with('/') && !p.ends_with('\\') && (enc || !require_encrypted_flag);
            if eligible {
                match smallest_out {
                    None => *smallest_out = Some((p, size)),
                    Some((_, best_sz)) => {
                        let better = if *best_sz == 0 && size > 0 {
                            true
                        } else if size == 0 && *best_sz > 0 {
                            false
                        } else {
                            size < *best_sz
                        };
                        if better {
                            *smallest_out = Some((p, size));
                        }
                    }
                }
            }
        }
    };

    for line in slt_output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("----------") {
            in_contents = true;
            continue;
        }
        if !in_contents {
            if trimmed == "Encrypted = +" {
                any_encrypted = true;
            }
            continue;
        }
        if let Some(val) = line.strip_prefix("Path = ") {
            flush(&mut cur_path, cur_size, cur_is_dir, cur_enc, &mut any_encrypted, &mut smallest);
            cur_path = Some(val.trim().to_string());
            cur_size = 0;
            cur_is_dir = false;
            cur_enc = false;
        } else if let Some(val) = line.strip_prefix("Size = ") {
            cur_size = val.trim().parse::<u64>().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("Attributes = ") {
            if val.contains('D') {
                cur_is_dir = true;
            }
        } else if trimmed == "Encrypted = +" {
            cur_enc = true;
        } else if trimmed == "Folder = +" {
            cur_is_dir = true;
        }
    }
    flush(&mut cur_path, cur_size, cur_is_dir, cur_enc, &mut any_encrypted, &mut smallest);
    (any_encrypted, smallest)
}

/// Inspect archive encryption without decompressing large payloads.
pub fn inspect_archive_encryption(archive: &Path) -> ArchiveEncryptionInfo {
    for executable in candidate_7z_executables() {
        let mut cmd = Command::new(&executable);
        cmd.stdin(Stdio::null());
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
        cmd.args(["l", "-slt", "-sccUTF-8", "-p"]).arg(archive);
        if let Ok(output) = cmd.output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Wrong password")
                || stderr.contains("Can not open encrypted")
                || stdout.contains("Can not open encrypted")
                || stdout.contains("Enter password")
            {
                return ArchiveEncryptionInfo {
                    is_encrypted: true,
                    header_encrypted: true,
                    smallest_encrypted_member: None,
                };
            }
            let (any_enc, smallest) = parse_smallest_encrypted_from_slt(&stdout, true);
            return ArchiveEncryptionInfo {
                is_encrypted: any_enc,
                header_encrypted: false,
                smallest_encrypted_member: smallest,
            };
        }
    }
    ArchiveEncryptionInfo::default()
}

/// Check if an archive contains encrypted files or has an encrypted header.
#[allow(dead_code)]
pub fn is_archive_encrypted(archive: &Path) -> bool {
    inspect_archive_encryption(archive).is_encrypted
}

/// Fast password verification: tests header decryption or ONLY the single smallest
/// encrypted member (if <= 25 MB), avoiding a full 10 GB decompression before extraction!
pub fn test_archive_password_fast(
    archive: &Path,
    password: &str,
    info: &ArchiveEncryptionInfo,
    is_silent_guess: bool,
) -> bool {
    const MAX_FAST_TEST_BYTES: u64 = 25 * 1024 * 1024;
    for executable in candidate_7z_executables() {
        let pwd_arg = format!("-p{password}");
        if info.header_encrypted {
            let mut list_cmd = Command::new(&executable);
            list_cmd.stdin(Stdio::null());
            #[cfg(windows)]
            list_cmd.creation_flags(CREATE_NO_WINDOW);
            list_cmd.args(["l", "-slt", "-sccUTF-8", "-y", &pwd_arg]).arg(archive);
            let Ok(list_out) = list_cmd.output() else { continue; };
            if !list_out.status.success() {
                return false;
            }
            let stdout = String::from_utf8_lossy(&list_out.stdout);
            let (_, smallest) = parse_smallest_encrypted_from_slt(&stdout, false);
            if let Some((member, sz)) = smallest {
                if sz <= MAX_FAST_TEST_BYTES {
                    let mut t_cmd = Command::new(&executable);
                    t_cmd.stdin(Stdio::null());
                    #[cfg(windows)]
                    t_cmd.creation_flags(CREATE_NO_WINDOW);
                    t_cmd.args(["t", "-y", "-sccUTF-8", &pwd_arg]).arg(archive).arg("--").arg(&member);
                    if let Ok(t_out) = t_cmd.output() {
                        return t_out.status.success();
                    }
                }
            }
            return true;
        } else if let Some((ref member, sz)) = info.smallest_encrypted_member {
            if sz > MAX_FAST_TEST_BYTES {
                // Never stall the UI brute-forcing default passwords on a giant single-file archive.
                return !is_silent_guess;
            }
            let mut t_cmd = Command::new(&executable);
            t_cmd.stdin(Stdio::null());
            #[cfg(windows)]
            t_cmd.creation_flags(CREATE_NO_WINDOW);
            t_cmd.args(["t", "-y", "-sccUTF-8", &pwd_arg]).arg(archive).arg("--").arg(member);
            if let Ok(t_out) = t_cmd.output() {
                return t_out.status.success();
            }
        } else {
            let mut cmd = Command::new(&executable);
            cmd.stdin(Stdio::null());
            #[cfg(windows)]
            cmd.creation_flags(CREATE_NO_WINDOW);
            cmd.args(["t", "-y", &pwd_arg]).arg(archive);
            if let Ok(output) = cmd.output() {
                return output.status.success();
            }
        }
    }
    false
}

/// Test whether the given password successfully decrypts the archive.
#[allow(dead_code)]
pub fn test_archive_password(archive: &Path, password: &str) -> bool {
    let info = inspect_archive_encryption(archive);
    test_archive_password_fast(archive, password, &info, false)
}

fn join_reader(reader: Option<thread::JoinHandle<()>>) {
    // Detach reader handle so deadlocked pipe reads can never block child exit
    drop(reader);
}

/// Parse `NN%` tokens and current filenames from 7-Zip's progress stream.
fn read_progress_percentages(mut stream: impl Read, progress: Option<ProgressHandle>) {
    let mut buffer = [0u8; 4096];
    let mut digits = String::new();
    let mut highest: u8 = 0;
    let mut line_bytes: Vec<u8> = Vec::new();
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        for &byte in &buffer[..read] {
            if byte == b'\r' || byte == b'\n' {
                if !line_bytes.is_empty() {
                    let line_str = String::from_utf8_lossy(&line_bytes);
                    let trimmed = line_str.trim();
                    if !trimmed.is_empty() {
                        let fname_opt = if let Some(dash) = trimmed.find(" - ") {
                            Some(trimmed[dash + 3..].trim())
                        } else if let Some(plus) = trimmed.find(" + ") {
                            Some(trimmed[plus + 3..].trim())
                        } else if let Some(rest) = trimmed.strip_prefix("- ") {
                            Some(rest.trim())
                        } else if let Some(rest) = trimmed.strip_prefix("+ ") {
                            Some(rest.trim())
                        } else if let Some(rest) = trimmed.strip_prefix("Extracting") {
                            Some(rest.trim())
                        } else {
                            None
                        };

                        if let Some(fname) = fname_opt {
                            let clean = fname.trim_end_matches(|c| c == '/' || c == '\\').trim();
                            if !clean.is_empty()
                                && !clean.starts_with("Everything is Ok")
                                && !clean.starts_with("Scanning")
                                && !clean.starts_with("Creating archive")
                                && !clean.starts_with("Extracting archive")
                                && !clean.starts_with("Folders:")
                                && !clean.starts_with("Files:")
                                && !clean.starts_with("Size:")
                                && !clean.starts_with("Compressed:")
                            {
                                if let Some(handle) = &progress {
                                    if let Ok(mut lock) = handle.current_file.lock() {
                                        if *lock != clean {
                                            *lock = clean.to_string();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    line_bytes.clear();
                }
            } else {
                line_bytes.push(byte);
            }

            if byte.is_ascii_digit() {
                if digits.len() == 3 { digits.remove(0); }
                digits.push(byte as char);
            } else {
                if byte == b'%' {
                    if let Ok(value) = digits.parse::<u8>() {
                        if value <= 100 && value >= highest {
                            highest = value;
                            if let Some(handle) = &progress {
                                handle.percent.store(value, Ordering::Relaxed);
                            }
                        }
                    }
                }
                digits.clear();
            }
        }
    }
}

/// Delete a file, retrying briefly because antivirus, the indexer or the shell
/// can hold a handle on a just-written archive for a fraction of a second.
/// Returns true when the file is gone (including when it never existed).
pub fn delete_file_with_retry(path: &Path, attempts: u32) -> bool {
    let attempts = attempts.max(1);
    for attempt in 0..attempts {
        match std::fs::remove_file(path) {
            Ok(()) => return true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) if attempt + 1 < attempts => thread::sleep(Duration::from_millis(100)),
            Err(_) => return false,
        }
    }
    true
}

/// Windows job object created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
/// Closing the last handle to the job (which happens when this process exits,
/// however it exits) terminates every process assigned to it.
#[cfg(windows)]
struct KillOnCloseJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl KillOnCloseJob {
    fn attach(child: &std::process::Child) -> Option<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        unsafe {
            let job: HANDLE = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            // Failing to assign is not fatal: cancellation still kills the child.
            if configured == 0 || AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) == 0 {
                CloseHandle(job);
                return None;
            }
            Some(Self(job))
        }
    }
}

#[cfg(windows)]
impl Drop for KillOnCloseJob {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn ultra_defaults() {
        assert_eq!(CompressionSettings::default().ultra_arguments().unwrap(),
            vec![OsString::from("-t7z"), OsString::from("-mx=9"), OsString::from("-m0=LZMA2"), OsString::from("-ms=on")]);
    }
    #[test] fn threads_and_non_solid() {
        let s = CompressionSettings { level: 9, solid: false, threads: Some(4) };
        assert!(s.ultra_arguments().unwrap().contains(&OsString::from("-mmt=4")));
        assert!(s.ultra_arguments().unwrap().contains(&OsString::from("-ms=off")));
    }
    #[test] fn rejects_excessive_threads() {
        let s = CompressionSettings { threads: Some(257), ..Default::default() };
        assert!(s.validate().is_err());
    }
    #[test] fn rejects_no_inputs() {
        assert!(create_arguments(Path::new("backup.7z"), &[], &CompressionSettings::default()).is_err());
    }
    #[test] fn archive_arguments_are_distinct_and_ultra() {
        let dir = std::env::temp_dir().join(format!("mickey-rar-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a file.txt");
        std::fs::write(&file, b"hello world").unwrap();
        let inputs = vec![file];
        let output = dir.join("out.7z");
        let (_, args) = create_arguments(&output, &inputs, &CompressionSettings::default()).unwrap();
        assert_eq!(args[0], OsString::from("a"));
        assert!(args.contains(&OsString::from("-mx=9")));
        assert!(args.contains(&OsString::from("a file.txt")));
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test] fn refuses_missing_input() {
        let dir = std::env::temp_dir().join(format!("mickey-missing-{}", std::process::id()));
        let result = create_arguments(&dir.join("backup.7z"), &[dir.join("not-present.txt")], &CompressionSettings::default());
        assert!(result.is_err());
    }
    #[test] fn refuses_same_input_and_output() {
        let dir = std::env::temp_dir().join(format!("mickey-same-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("file.7z");
        std::fs::write(&file, b"data").unwrap();
        assert!(create_arguments(&file, &[file.clone()], &CompressionSettings::default()).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test] fn rejects_unsupported_level() {
        let s = CompressionSettings { level: 10, ..Default::default() };
        assert!(s.ultra_arguments().is_err());
    }
    #[test] fn deletes_partial_file_and_tolerates_missing_file() {
        let dir = std::env::temp_dir().join(format!("mickey-delete-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let partial = dir.join("partial.zip");
        std::fs::write(&partial, b"half written").unwrap();
        assert!(delete_file_with_retry(&partial, 3));
        assert!(!partial.exists());
        // Already gone counts as success, so cancelling twice is harmless.
        assert!(delete_file_with_retry(&partial, 3));
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test] fn parses_7zip_progress_percentages() {
        let feed = b"Scanning\r\n  0%\r\n  7% + file.bin\r\n 60% + file.bin\r\n100% + file.bin\r\n";
        let handle = ProgressHandle::new();
        read_progress_percentages(&feed[..], Some(handle.clone()));
        assert_eq!(handle.percent.load(Ordering::Relaxed), 100);
        assert_eq!(*handle.current_file.lock().unwrap(), "file.bin");
        // Missing handle must be harmless.
        read_progress_percentages(&feed[..], None);
        // Values that cannot be a percentage (or go backwards) are ignored.
        let handle = ProgressHandle::new();
        read_progress_percentages(&b" 80% then 40% then 999%"[..], Some(handle.clone()));
        assert_eq!(handle.percent.load(Ordering::Relaxed), 80);
    }
}

/// Reject suspicious archive member names *before* asking the external extractor to write files.
/// Defense in depth only: archive metadata/link handling also needs OS-level integration tests.
pub fn member_path_looks_safe(name: &str) -> bool {
    use std::path::Component;
    let text = name.replace('\\', "/");
    if text.starts_with('/') || text.starts_with("//") || text.contains('\0') { return false; }
    if text.len() >= 2 && text.as_bytes()[1] == b':' { return false; }
    let mut depth = 0usize;
    for component in Path::new(&text).components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {},
            Component::ParentDir => { if depth == 0 { return false; } depth -= 1; }
            _ => return false,
        }
    }
    true
}

pub fn validate_archive_members_safe<'a, I>(paths: I) -> Result<(), String>
where
    I: IntoIterator<Item = &'a str>,
{
    for p in paths {
        if !member_path_looks_safe(p) {
            return Err(format!("Blocked unsafe path inside archive (Zip-Slip protection): {p}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod path_safety_tests {
    use super::{
        create_archive_command_args, member_path_looks_safe, parse_smallest_encrypted_from_slt,
        validate_archive_members_safe, ArchiveCreationOptions,
    };
    use std::ffi::OsString;
    #[test] fn allows_nested_paths() { assert!(member_path_looks_safe("folder/file.txt")); }
    #[test] fn rejects_parent_escape() { assert!(!member_path_looks_safe("../evil.txt")); }
    #[test] fn rejects_absolute_paths() { assert!(!member_path_looks_safe("/etc/passwd")); }
    #[test] fn rejects_windows_paths() { assert!(!member_path_looks_safe("C:\\Windows\\win.ini")); }
    #[test] fn rejects_windows_backslash_traversal() { assert!(!member_path_looks_safe("..\\evil.txt")); }
    #[test] fn batch_validation_catches_zip_slip() {
        assert!(validate_archive_members_safe(["ok/a.txt", "ok/b.txt"]).is_ok());
        assert!(validate_archive_members_safe(["ok/a.txt", "../../evil.dll"]).is_err());
    }
    #[test] fn slt_parser_picks_smallest_non_empty_encrypted_file() {
        let slt = "\
----------
Path = big.bin
Size = 9999999999
Encrypted = +

Path = empty.txt
Size = 0
Encrypted = +

Path = tiny.ini
Size = 42
Encrypted = +
";
        let (enc, smallest) = parse_smallest_encrypted_from_slt(slt, true);
        assert!(enc);
        assert_eq!(smallest, Some(("tiny.ini".to_string(), 42)));
    }
    #[test] fn create_archive_command_supports_password_and_dash_filenames() {
        let dir = std::env::temp_dir().join(format!("mickey-opts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dash_file = dir.join("-weird.txt");
        std::fs::write(&dash_file, b"abc").unwrap();
        let out = dir.join("sec.7z");
        let opts = ArchiveCreationOptions {
            format: "7z".into(),
            level: 9,
            solid: true,
            threads: None,
            password: Some("secret123".into()),
            encrypt_headers: true,
        };
        let (_, args) = create_archive_command_args(&out, &[dash_file], &opts).unwrap();
        assert!(args.contains(&OsString::from("-psecret123")));
        assert!(args.contains(&OsString::from("-mhe=on")));
        assert!(args.contains(&OsString::from("--")));
        assert!(args.contains(&OsString::from("-weird.txt")));
        let _ = std::fs::remove_dir_all(dir);
    }
}
