//! 7-Zip command-line integration. Each argument is passed separately, never via a shell.
use std::{ffi::OsString, io::Read, path::{Path, PathBuf}, process::{Command, Stdio}, sync::{atomic::{AtomicBool, AtomicU8, Ordering}, Arc}, thread, time::Duration};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

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
pub fn create_arguments(output: &Path, inputs: &[PathBuf], settings: &CompressionSettings)
    -> Result<(PathBuf, Vec<OsString>), String> {
    create_arguments_for_format(output, inputs, settings, "zip")
}

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

pub fn run_7z(args: &[OsString], cwd: Option<&Path>) -> Result<String, String> {
    let mut executables = Vec::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            executables.push(dir.join(if cfg!(target_os="windows") { "7z.exe" } else { "7zz" }));
        }
    }
    if cfg!(target_os="windows") {
        executables.extend([
            r"C:\Program Files\7-Zip\7z.exe",
            r"C:\Program Files (x86)\7-Zip\7z.exe",
            r"C:\Program Files\Mickey RAR\7z.exe",
            "7z.exe", "7zz.exe", "7z", "7zz"
        ].into_iter().map(PathBuf::from));
    } else {
        executables.extend(["7zz", "7z"].into_iter().map(PathBuf::from));
    }
    for executable in executables {
        let mut cmd = Command::new(&executable);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
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

/// Progress percentage (0-100) as reported by 7-Zip itself.
pub type ProgressHandle = Arc<AtomicU8>;

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
    let mut executables = Vec::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            executables.push(dir.join(if cfg!(target_os="windows") { "7z.exe" } else { "7zz" }));
        }
    }
    if cfg!(target_os = "windows") {
        executables.extend([
            r"C:\Program Files\7-Zip\7z.exe",
            r"C:\Program Files (x86)\7-Zip\7z.exe",
            r"C:\Program Files\Mickey RAR\7z.exe",
            "7z.exe", "7zz.exe", "7z", "7zz"
        ].into_iter().map(PathBuf::from));
    } else {
        executables.extend(["7zz", "7z"].into_iter().map(PathBuf::from));
    }
    for executable in executables {
        let mut command = Command::new(&executable);
        if progress.is_some() && !args.is_empty() {
            // Keep the operation switch first, then ask 7-Zip for progress data.
            command.arg(&args[0]).arg("-bsp1").args(&args[1..]);
            command.stdout(Stdio::piped()).stderr(Stdio::null());
        } else {
            command.args(args).stdout(Stdio::null()).stderr(Stdio::null());
        }
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
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
        // 7-Zip writes progress into a pipe: drain it in a helper thread, otherwise
        // the pipe buffer fills up and 7-Zip blocks (a real freeze).
        let reader = child.stdout.take().map(|stream| {
            let sink = progress.cloned();
            thread::spawn(move || read_progress_percentages(stream, sink))
        });
        loop {
            if cancelled.load(Ordering::Relaxed) {
                // Kill first, then wait for the process to really be gone: only
                // after that does Windows release the half-written archive.
                let _ = child.kill();
                let _ = child.wait();
                join_reader(reader);
                return Ok(RunOutcome::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    join_reader(reader);
                    return if status.success() { Ok(RunOutcome::Finished) } else { Err(format!("7-Zip failed ({status})")) };
                }
                Ok(None) => thread::sleep(Duration::from_millis(80)),
                Err(error) => { join_reader(reader); return Err(format!("Could not read 7-Zip status: {error}")); }
            }
        }
    }
    Err("7-Zip executable (7zz or 7z) was not found in PATH".into())
}

fn join_reader(reader: Option<thread::JoinHandle<()>>) {
    if let Some(reader) = reader { let _ = reader.join(); }
}

/// Parse `NN%` tokens from 7-Zip's progress stream and publish the highest one.
/// 7-Zip separates updates with `\r`, `\n` and backspaces, so the stream is
/// scanned byte by byte instead of line by line.
fn read_progress_percentages(mut stream: impl Read, progress: Option<ProgressHandle>) {
    let mut buffer = [0u8; 4096];
    let mut digits = String::new();
    let mut highest: u8 = 0;
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        for &byte in &buffer[..read] {
            if byte.is_ascii_digit() {
                if digits.len() == 3 { digits.remove(0); }
                digits.push(byte as char);
            } else {
                if byte == b'%' {
                    if let Ok(value) = digits.parse::<u8>() {
                        if value <= 100 && value >= highest {
                            highest = value;
                            if let Some(handle) = &progress { handle.store(value, Ordering::Relaxed); }
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
        let handle = Arc::new(AtomicU8::new(0));
        read_progress_percentages(&feed[..], Some(handle.clone()));
        assert_eq!(handle.load(Ordering::Relaxed), 100);
        // Missing handle must be harmless.
        read_progress_percentages(&feed[..], None);
        // Values that cannot be a percentage (or go backwards) are ignored.
        let handle = Arc::new(AtomicU8::new(0));
        read_progress_percentages(&b" 80% then 40% then 999%"[..], Some(handle.clone()));
        assert_eq!(handle.load(Ordering::Relaxed), 80);
    }
}

/// Reject suspicious archive member names *before* asking the external extractor to write files.
/// Defense in depth only: archive metadata/link handling also needs OS-level integration tests.
#[allow(dead_code)]
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

#[cfg(test)]
mod path_safety_tests {
    use super::member_path_looks_safe;
    #[test] fn allows_nested_paths() { assert!(member_path_looks_safe("folder/file.txt")); }
    #[test] fn rejects_parent_escape() { assert!(!member_path_looks_safe("../evil.txt")); }
    #[test] fn rejects_absolute_paths() { assert!(!member_path_looks_safe("/etc/passwd")); }
    #[test] fn rejects_windows_paths() { assert!(!member_path_looks_safe("C:\\Windows\\win.ini")); }
    #[test] fn rejects_windows_backslash_traversal() { assert!(!member_path_looks_safe("..\\evil.txt")); }
}
