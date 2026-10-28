//! Guarded native filesystem operations for the launcher.
use std::{
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub fn validate_relative(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains(['\\', ':'])
        || path
            .chars()
            .any(|c| c.is_control() || "<>\"|?*".contains(c))
    {
        return Err(format!("Unsafe relative path: {path:?}"));
    }
    for part in path.split('/') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches(' ')
            .to_uppercase();
        let reserved = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
        if part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' ']) || reserved
        {
            return Err(format!("Unsafe Windows path component: {part:?}"));
        }
    }
    Ok(())
}

pub fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("Path is empty".into());
    }
    let path =
        std::path::absolute(path).map_err(|e| format!("Invalid path {}: {e}", path.display()))?;
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!(
            "Parent traversal is not allowed: {}",
            path.display()
        ));
    }
    Ok(path)
}

/// Checks every existing ancestor, including ancestors above a not-yet-created output.
pub fn guard(path: &Path) -> Result<(), String> {
    let path = absolute(path)?;
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                #[cfg(windows)]
                let reparse = {
                    use std::os::windows::fs::MetadataExt;
                    meta.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let reparse = false;
                if meta.file_type().is_symlink() || reparse {
                    return Err(format!(
                        "Refusing a symbolic link or reparse point: {}",
                        ancestor.display()
                    ));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(format!("Cannot inspect {}: {e}", ancestor.display())),
        }
    }
    Ok(())
}

pub fn join(root: &Path, rel: &str) -> Result<PathBuf, String> {
    validate_relative(rel)?;
    let path = root.join(rel);
    guard(&path)?;
    Ok(path)
}

pub fn overlaps(a: &Path, b: &Path) -> Result<bool, String> {
    let a = absolute(a)?;
    let b = absolute(b)?;
    let ac: Vec<_> = a
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    let bc: Vec<_> = b
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    Ok(ac.starts_with(&bc) || bc.starts_with(&ac))
}

pub fn create_dir_all(path: &Path) -> Result<(), String> {
    guard(path)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
        && !parent.exists()
    {
        create_dir_all(parent)?;
    }
    match fs::create_dir(path) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => (),
        Err(e) => return Err(format!("Cannot create directory {}: {e}", path.display())),
    }
    guard(path)
}

pub fn remove_file(path: &Path) -> Result<(), String> {
    guard(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            #[cfg(windows)]
            if fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().readonly()) {
                return windows::unlink_readonly(path).map_err(|native| {
                    format!(
                        "Cannot remove {}: {native}. Close any game using this stand.",
                        path.display()
                    )
                });
            }
            Err(format!(
                "Cannot remove {}: {e}. Close any game using this stand.",
                path.display()
            ))
        }
    }
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
fn temporary(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Output has no parent directory".to_string())?;
    Ok(parent.join(format!(
        ".raylink-tmp-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    )))
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    guard(path)?;
    let temp = temporary(path)?;
    guard(&temp)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| format!("Cannot stage {}: {e}", path.display()))?;
    let result = (|| {
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("Cannot write {}: {e}", temp.display()))?;
        drop(file);
        guard(path)?;
        // Rename replaces a name; it never truncates a hard-linked source inode.
        fs::rename(&temp, path).map_err(|e| format!("Cannot replace {}: {e}", path.display()))
    })();
    if result.is_err() {
        let _ = remove_file(&temp);
    }
    result
}

/// Returns true for a hard link, false for a private copy, plus a reason if linking fell back.
pub fn place(source: &Path, target: &Path, copy: bool) -> Result<(bool, Option<String>), String> {
    place_using(source, target, copy, |source, stage| {
        fs::hard_link(source, stage)
    })
}

fn place_using(
    source: &Path,
    target: &Path,
    copy: bool,
    link: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(bool, Option<String>), String> {
    guard(source)?;
    guard(target)?;
    if !source.is_file() {
        return Err(format!("Source file is missing: {}", source.display()));
    }
    let parent = target
        .parent()
        .ok_or_else(|| "Output has no parent".to_string())?;
    create_dir_all(parent)?;
    let stage = temporary(target)?;
    guard(&stage)?;
    let mut fallback = None;
    let linked = if copy {
        false
    } else {
        match link(source, &stage) {
            Ok(()) => true,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(format!("Staging path already exists: {}", stage.display()));
            }
            Err(e) => {
                fallback = Some(format!(
                    "Hard link unavailable for {}: {e}; using a copy with additional storage cost",
                    source.display()
                ));
                false
            }
        }
    };
    let result = (|| {
        if !linked {
            let mut input = fs::File::open(source)
                .map_err(|e| format!("Cannot read {}: {e}", source.display()))?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stage)
                .map_err(|e| format!("Cannot stage {}: {e}", target.display()))?;
            io::copy(&mut input, &mut output)
                .and_then(|_| output.sync_all())
                .map_err(|e| format!("Cannot copy {}: {e}", source.display()))?;
        }
        guard(target)?;
        remove_file(target)?;
        fs::rename(&stage, target)
            .map_err(|e| format!("Cannot install {}: {e}", target.display()))?;
        Ok((linked, fallback))
    })();
    if result.is_err() {
        let _ = remove_file(&stage);
    }
    result
}

pub fn running_processes(root: &Path) -> Result<Vec<String>, String> {
    guard(root)?;
    #[cfg(windows)]
    {
        windows::running_processes(root)
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_DISPOSITION_FLAG_DELETE,
            FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
            FILE_DISPOSITION_INFO_EX, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, FileDispositionInfoEx, OPEN_EXISTING,
            SetFileInformationByHandle,
        },
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub(super) fn unlink_readonly(path: &Path) -> Result<(), String> {
        let full = fs::canonicalize(path).map_err(|e| e.to_string())?;
        let wide: Vec<_> = full.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let h = CreateFileW(
                wide.as_ptr(),
                0x00010000,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            );
            if h == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error().to_string());
            }
            let h = Handle(h);
            let info = FILE_DISPOSITION_INFO_EX {
                Flags: FILE_DISPOSITION_FLAG_DELETE
                    | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
                    | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
            };
            if SetFileInformationByHandle(
                h.0,
                FileDispositionInfoEx,
                (&info as *const FILE_DISPOSITION_INFO_EX).cast(),
                std::mem::size_of_val(&info) as u32,
            ) == 0
            {
                return Err(io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }
    pub(super) fn running_processes(root: &Path) -> Result<Vec<String>, String> {
        let root = fs::canonicalize(root)
            .or_else(|_| absolute(root).map_err(io::Error::other))
            .map_err(|e| e.to_string())?;
        let root_text = root
            .to_string_lossy()
            .trim_start_matches("\\\\?\\")
            .trim_end_matches('\\')
            .to_lowercase()
            + "\\";
        let mut found = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return Err(format!(
                    "Cannot enumerate processes: {}",
                    io::Error::last_os_error()
                ));
            }
            let snap = Handle(snap);
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap.0, &mut entry);
            while ok != 0 {
                let process =
                    OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                if !process.is_null() {
                    let process = Handle(process);
                    let mut buffer = vec![0u16; 32768];
                    let mut size = buffer.len() as u32;
                    if QueryFullProcessImageNameW(process.0, 0, buffer.as_mut_ptr(), &mut size) != 0
                    {
                        let path = String::from_utf16_lossy(&buffer[..size as usize]);
                        if path
                            .trim_start_matches("\\\\?\\")
                            .to_lowercase()
                            .starts_with(&root_text)
                        {
                            found.push(format!("{path} (PID {})", entry.th32ProcessID));
                        }
                    }
                }
                ok = Process32NextW(snap.0, &mut entry);
            }
        }
        found.sort();
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    #[test]
    fn fs_rejects_windows_escape_and_reserved_paths() {
        for path in [
            "../outside",
            "a/../b",
            "/root",
            "C:/root",
            "a\\b",
            "a:stream",
            "CON",
            "nul.txt",
            "x/COM1.ini",
            "aux ",
            "x.",
            "x//y",
            "CONIN$",
            "CONOUT$",
            "con .txt",
        ] {
            assert!(super::validate_relative(path).is_err(), "accepted {path}");
        }
        assert!(super::validate_relative("gamedata/scripts/test.script").is_ok());
    }
    #[test]
    fn fs_cross_device_link_failure_copies_privately_and_reports_storage_cost() {
        let t = tempfile::tempdir_in(
            std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp"))
                .unwrap(),
        )
        .unwrap();
        let source = t.path().join("source");
        let target = t.path().join("target");
        std::fs::write(&source, b"source bytes").unwrap();
        let (linked, reason) = super::place_using(&source, &target, false, |_, _| {
            Err(std::io::Error::other("different volume"))
        })
        .unwrap();
        assert!(!linked);
        assert!(reason.unwrap().contains("storage cost"));
        assert_eq!(std::fs::read(&target).unwrap(), b"source bytes");
        std::fs::write(&target, b"local edit").unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), b"source bytes");
    }
    #[cfg(windows)]
    #[test]
    fn fs_finds_a_real_process_by_executable_path() {
        let t = tempfile::tempdir_in(
            std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp"))
                .unwrap(),
        )
        .unwrap();
        let exe = t.path().join("synthetic.exe");
        let system = std::env::var_os("SystemRoot").unwrap();
        std::fs::copy(Path::new(&system).join("System32/ping.exe"), &exe).unwrap();
        struct Running(std::process::Child);
        impl Drop for Running {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let running = Running(
            std::process::Command::new(&exe)
                .args(["-t", "127.0.0.1"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let found = super::running_processes(t.path()).unwrap();
        assert!(
            found
                .iter()
                .any(|p| p.contains(&format!("PID {}", running.0.id()))),
            "{found:?}"
        );
        drop(running);
        assert!(super::running_processes(t.path()).unwrap().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn fs_unlinks_readonly_hardlink_without_changing_source() {
        let temp = tempfile::tempdir_in(
            std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp"))
                .unwrap(),
        )
        .unwrap();
        let src = temp.path().join("source");
        let dst = temp.path().join("link");
        std::fs::write(&src, b"original").unwrap();
        std::fs::hard_link(&src, &dst).unwrap();
        let mut permissions = std::fs::metadata(&src).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&src, permissions).unwrap();
        super::remove_file(&dst).unwrap();
        assert!(!dst.exists());
        assert!(std::fs::metadata(&src).unwrap().permissions().readonly());
        assert_eq!(std::fs::read(&src).unwrap(), b"original");
        super::remove_file(&src).unwrap();
    }
}
