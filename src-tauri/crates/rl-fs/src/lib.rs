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
    inspect_full(path).map(|_| ())
}

fn inspect_full(path: &Path) -> Result<Option<fs::Metadata>, String> {
    let path = absolute(path)?;
    let mut leaf = None;
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
                if ancestor == path {
                    leaf = Some(meta);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(format!("Cannot inspect {}: {e}", ancestor.display())),
        }
    }
    Ok(leaf)
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

/// Compares filesystem volume identities, walking to the nearest existing ancestor
/// for an output that has not been created. Reparse paths are always rejected.
pub fn same_volume(a: &Path, b: &Path) -> Result<bool, String> {
    Ok(volume_identity(a)? == volume_identity(b)?)
}

fn volume_identity(path: &Path) -> Result<u64, String> {
    let mut path = absolute(path)?;
    guard(&path)?;
    loop {
        match fs::symlink_metadata(&path) {
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if !path.pop() {
                    return Err("No existing filesystem ancestor".into());
                }
            }
            Err(e) => return Err(format!("Cannot inspect {}: {e}", path.display())),
        }
    }
    #[cfg(windows)]
    {
        windows::volume_identity(&path)
            .map_err(|e| format!("Cannot identify volume {}: {e}", path.display()))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::symlink_metadata(&path)
            .map(|m| m.dev())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(any(windows, unix)))]
    {
        Err("Volume identity is unsupported on this platform".into())
    }
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
    remove_checked_leaf(path)
}

// Private: the caller must freshly guard this path before removing its leaf.
fn remove_checked_leaf(path: &Path) -> Result<(), String> {
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

/// A filesystem pass. Windows caches stable directory handles, never attributes:
/// every guard refreshes every ancestor handle and inspects the leaf by name.
#[derive(Default)]
pub struct Batch {
    #[cfg(windows)]
    directories: std::collections::HashMap<PathBuf, fs::File>,
    #[cfg(windows)]
    full_guard_prefixes: Vec<PathBuf>,
}
impl Batch {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn guard(&mut self, path: &Path) -> Result<(), String> {
        self.inspect(path).map(|_| ())
    }
    /// Returns the same fresh leaf metadata inspected by this operation's guard.
    /// Missing leaves are errors here, while guard alone permits missing paths.
    pub fn metadata(&mut self, path: &Path) -> Result<fs::Metadata, String> {
        self.inspect(path)?
            .ok_or_else(|| format!("Cannot inspect {}: file not found", path.display()))
    }
    fn inspect(&mut self, path: &Path) -> Result<Option<fs::Metadata>, String> {
        #[cfg(not(windows))]
        {
            inspect_full(path)
        }
        #[cfg(windows)]
        {
            let path = absolute(path)?;
            if self.requires_full_guard(&path) {
                return inspect_full(&path);
            }
            if let Some(parent) = path.parent()
                && !self.pin_directory_using(parent, &mut windows::open_directory)?
            {
                // An absent parent cannot authorize opening a descendant by name.
                // A binding mismatch instead requires full validation of this operation.
                return if self.requires_full_guard(&path) {
                    inspect_full(&path)
                } else {
                    Ok(None)
                };
            }
            match fs::symlink_metadata(&path) {
                Ok(meta) => {
                    windows::check_directory_attributes(&meta)?;
                    if meta.is_dir() {
                        self.pin_directory_using(&path, &mut windows::open_directory)?;
                        if self.requires_full_guard(&path) {
                            return inspect_full(&path);
                        }
                    }
                    Ok(Some(meta))
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(format!("Cannot inspect {}: {e}", path.display())),
            }
        }
    }
    #[cfg(windows)]
    fn requires_full_guard(&self, path: &Path) -> bool {
        self.full_guard_prefixes
            .iter()
            .any(|prefix| path.starts_with(prefix))
    }
    #[cfg(windows)]
    fn pin_directory_using(
        &mut self,
        path: &Path,
        open: &mut impl FnMut(&Path) -> io::Result<fs::File>,
    ) -> Result<bool, String> {
        if self.requires_full_guard(path) {
            return Ok(false);
        }
        if let Some(parent) = path.parent()
            && !self.pin_directory_using(parent, open)?
        {
            return Ok(false);
        }
        if let Some(file) = self.directories.get(path) {
            windows::refresh_directory(file)
                .map_err(|e| format!("Cannot inspect directory {}: {e}", path.display()))?;
            return Ok(true);
        }
        match open(path) {
            Ok(file) => {
                windows::refresh_directory(&file)?;
                if windows::bind_directory(&file, path).is_err() {
                    // Spelling aliases and transient redirection are indistinguishable
                    // here. Drop ALL pins, then bypass caching for this exact requested
                    // prefix and descendants. Other paths must bind independently.
                    self.directories.clear();
                    self.full_guard_prefixes.push(path.to_path_buf());
                    return Ok(false);
                }
                self.directories.insert(path.to_path_buf(), file);
                Ok(true)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("Cannot pin directory {}: {e}", path.display())),
        }
    }
    pub fn same_volume(&mut self, a: &Path, b: &Path) -> Result<bool, String> {
        same_volume(a, b)
    }
    pub fn join(&mut self, root: &Path, rel: &str) -> Result<PathBuf, String> {
        validate_relative(rel)?;
        let path = root.join(rel);
        self.guard(&path)?;
        Ok(path)
    }
    pub fn create_dir_all(&mut self, path: &Path) -> Result<(), String> {
        self.guard(path)?;
        #[cfg(windows)]
        if self.directories.contains_key(&absolute(path)?) {
            // guard just refreshed the complete chain, including this directory.
            return Ok(());
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
            && !parent.exists()
        {
            self.create_dir_all(parent)?;
        }
        match fs::create_dir(path) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => (),
            Err(e) => return Err(format!("Cannot create directory {}: {e}", path.display())),
        }
        self.guard(path)
    }

    pub fn remove_file(&mut self, path: &Path) -> Result<(), String> {
        self.guard(path)?;
        remove_checked_leaf(path)
    }

    pub fn atomic_write(&mut self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        self.guard(path)?;
        let temp = temporary(path)?;
        self.guard(&temp)?;
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
            self.guard(path)?;
            // Rename replaces a name; it never truncates a hard-linked source inode.
            fs::rename(&temp, path).map_err(|e| format!("Cannot replace {}: {e}", path.display()))
        })();
        if result.is_err() {
            let _ = self.remove_file(&temp);
        }
        result
    }

    /// Returns true for a hard link, false for a private copy, plus a reason if linking fell back.
    pub fn place(
        &mut self,
        source: &Path,
        target: &Path,
        copy: bool,
    ) -> Result<(bool, Option<String>), String> {
        self.place_using(source, target, copy, |source, stage| {
            fs::hard_link(source, stage)
        })
    }

    fn place_using(
        &mut self,
        source: &Path,
        target: &Path,
        copy: bool,
        link: impl FnOnce(&Path, &Path) -> io::Result<()>,
    ) -> Result<(bool, Option<String>), String> {
        self.guard(source)?;
        self.guard(target)?;
        if !source.is_file() {
            return Err(format!("Source file is missing: {}", source.display()));
        }
        let parent = target
            .parent()
            .ok_or_else(|| "Output has no parent".to_string())?;
        self.create_dir_all(parent)?;
        let stage = temporary(target)?;
        self.guard(&stage)?;
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
        let mut owns_stage = linked;
        let result = (|| {
            if !linked {
                let mut input = fs::File::open(source)
                    .map_err(|e| format!("Cannot read {}: {e}", source.display()))?;
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&stage)
                    .map_err(|e| format!("Cannot stage {}: {e}", target.display()))?;
                owns_stage = true;
                io::copy(&mut input, &mut output)
                    .and_then(|_| output.sync_all())
                    .map_err(|e| format!("Cannot copy {}: {e}", source.display()))?;
            }
            self.guard(target)?;
            remove_checked_leaf(target)?;
            fs::rename(&stage, target)
                .map_err(|e| format!("Cannot install {}: {e}", target.display()))?;
            Ok((linked, fallback))
        })();
        if result.is_err() && owns_stage {
            let _ = self.remove_file(&stage);
        }
        result
    }
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
    fn normalized_dos_path(wide: &[u16]) -> Vec<u16> {
        // Preserve UTF-16 and spelling exactly: Windows can enable case-sensitive
        // directories. Only structural prefix/separator aliases are normalized.
        let mut path: Vec<u16> = wide.iter().map(|&c| if c == 47 { 92 } else { c }).collect();
        if path.starts_with(&[92, 92, 63, 92, 85, 78, 67, 92]) {
            path = [vec![92, 92], path[8..].to_vec()].concat();
        } else if path.starts_with(&[92, 92, 63, 92]) {
            path.drain(..4);
        }
        while path.last() == Some(&92) {
            path.pop();
        }
        path
    }

    pub(super) fn bind_directory(file: &fs::File, expected: &Path) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
        };
        let mut buffer = vec![0u16; 256];
        let actual = loop {
            // The query uses only the live handle: do not canonicalize the pathname,
            // which could follow a transient junction a second time during binding.
            let length = unsafe {
                GetFinalPathNameByHandleW(
                    file.as_raw_handle(),
                    buffer.as_mut_ptr(),
                    buffer.len() as u32,
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            };
            if length == 0 {
                return Err(io::Error::last_os_error().to_string());
            }
            if length as usize >= buffer.len() {
                buffer.resize(length as usize + 1, 0);
                continue;
            }
            break normalized_dos_path(&buffer[..length as usize]);
        };
        let expected_wide: Vec<u16> = expected.as_os_str().encode_wide().collect();
        let expected_wide = normalized_dos_path(&expected_wide);
        // Exact comparison fails closed for spelling aliases and distinguishes
        // different entries in directories with Windows case sensitivity enabled.
        let same = actual == expected_wide;
        if !same {
            return Err(format!(
                "Directory handle does not match expected path: {}",
                expected.display()
            ));
        }
        Ok(())
    }

    pub(super) fn refresh_directory(file: &fs::File) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
            FileAttributeTagInfo, GetFileInformationByHandleEx,
        };
        let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
        // Query only current attributes/tag; full File::metadata needs more information.
        // File owns the live handle and info is an appropriately sized native buffer.
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileAttributeTagInfo,
                (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error().to_string());
        }
        if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("Refusing a symbolic link or reparse point".into());
        }
        if info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err("Not a directory".into());
        }
        Ok(())
    }

    pub(super) fn open_directory(path: &Path) -> io::Result<fs::File> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
        };
        fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }
    pub(super) fn check_directory_attributes(meta: &fs::Metadata) -> Result<(), String> {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || meta.file_type().is_symlink()
        {
            return Err("Refusing a symbolic link or reparse point".into());
        }
        Ok(())
    }

    pub(super) fn volume_identity(path: &Path) -> io::Result<u64> {
        use std::os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_READ_ATTRIBUTES, GetFileInformationByHandle,
        };
        let file = fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::other("Refusing a reparse point"));
        }
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // The handle remains owned by File for the entire native query.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(u64::from(info.dwVolumeSerialNumber))
    }

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
    fn fixture() -> tempfile::TempDir {
        tempfile::tempdir_in(
            std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp"))
                .unwrap(),
        )
        .unwrap()
    }

    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let output = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
    }

    #[cfg(windows)]
    #[test]
    fn batch_pins_allow_child_writes_and_release_on_drop() {
        let t = fixture();
        let source_dir = t.path().join("source");
        let output_dir = t.path().join("output");
        std::fs::create_dir(&source_dir).unwrap();
        std::fs::write(source_dir.join("file"), b"original").unwrap();
        let mut batch = super::Batch::new();
        batch.create_dir_all(&output_dir).unwrap();
        // An ordinary directory enumerator must coexist with the pins.
        let enumerator = std::fs::read_dir(&source_dir).unwrap();
        let linked = batch.join(&output_dir, "linked").unwrap();
        assert!(
            batch
                .place(&source_dir.join("file"), &linked, false)
                .unwrap()
                .0
        );
        batch
            .place(&source_dir.join("file"), &output_dir.join("copy"), true)
            .unwrap();
        batch
            .atomic_write(&output_dir.join("new"), b"generated")
            .unwrap();
        batch.atomic_write(&linked, b"replacement").unwrap();
        assert_eq!(std::fs::read(source_dir.join("file")).unwrap(), b"original");
        batch.remove_file(&output_dir.join("copy")).unwrap();
        batch
            .create_dir_all(&output_dir.join("nested/deeper"))
            .unwrap();
        assert!(std::fs::rename(&source_dir, t.path().join("moved-source")).is_err());
        assert!(std::fs::rename(&output_dir, t.path().join("moved-output")).is_err());
        assert!(std::fs::remove_dir(output_dir.join("nested/deeper")).is_err());
        drop(enumerator);
        drop(batch);
        std::fs::rename(&source_dir, t.path().join("moved-source")).unwrap();
        std::fs::rename(&output_dir, t.path().join("moved-output")).unwrap();
        junction(&output_dir, &t.path().join("moved-source"));
        assert!(super::Batch::new().guard(&output_dir.join("file")).is_err());
        std::fs::remove_dir(&output_dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn batch_rejects_junction_leaf_and_ancestors_even_for_missing_paths() {
        let t = fixture();
        let real = t.path().join("real");
        let link = t.path().join("junction");
        std::fs::create_dir(&real).unwrap();
        junction(&link, &real);
        let mut batch = super::Batch::default();
        for path in [&link, &link.join("missing/deep/file")] {
            assert!(batch.guard(path).is_err());
            assert!(batch.create_dir_all(path).is_err());
        }
        assert!(batch.join(t.path(), "../escape").is_err());
        // Reusing a Batch must still inspect its leaf and all ancestors.
        batch.guard(&real).unwrap();
        let child = real.join("junction");
        junction(&child, t.path());
        assert!(batch.guard(&child).is_err());
        assert!(batch.atomic_write(&child, b"no").is_err());
        std::fs::remove_dir(&child).unwrap();
        std::fs::remove_dir(&link).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn batch_rechecks_attribute_only_reparse_mutation_on_cached_handle() {
        use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
        use windows_sys::Win32::{
            Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
                FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
            },
            System::{IO::DeviceIoControl, Ioctl::FSCTL_SET_REPARSE_POINT},
        };
        let t = fixture();
        let dir = t.path().join("empty");
        let target = t.path().join("target");
        std::fs::create_dir(&dir).unwrap();
        std::fs::create_dir(&target).unwrap();
        let mut batch = super::Batch::new();
        batch.guard(&dir).unwrap();
        let writer = std::fs::OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&dir)
            .unwrap();
        let target_text = target.to_string_lossy();
        let substitute = format!("\\??\\{}", target_text.trim_start_matches("\\\\?\\"));
        let substitute: Vec<u16> = std::ffi::OsStr::new(&substitute).encode_wide().collect();
        let print: Vec<u16> = target.as_os_str().encode_wide().collect();
        let sub_bytes = (substitute.len() * 2) as u16;
        let print_bytes = (print.len() * 2) as u16;
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&0xa0000003u32.to_le_bytes());
        for value in [
            8 + sub_bytes + 2 + print_bytes + 2,
            0,
            0,
            sub_bytes,
            sub_bytes + 2,
            print_bytes,
        ] {
            buffer.extend_from_slice(&value.to_le_bytes());
        }
        for value in substitute.into_iter().chain([0]).chain(print).chain([0]) {
            buffer.extend_from_slice(&value.to_le_bytes());
        }
        let mut returned = 0;
        let result = unsafe {
            DeviceIoControl(
                writer.as_raw_handle(),
                FSCTL_SET_REPARSE_POINT,
                buffer.as_ptr().cast(),
                buffer.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(result, 0, "{}", std::io::Error::last_os_error());
        use std::os::windows::fs::MetadataExt;
        assert_ne!(
            batch.directories[&super::absolute(&dir).unwrap()]
                .metadata()
                .unwrap()
                .file_attributes()
                & 0x400,
            0
        );

        assert!(
            batch
                .directories
                .contains_key(&super::absolute(&dir).unwrap())
        );
        assert!(batch.guard(&dir).is_err());
        assert!(batch.guard(&dir.join("missing/deep")).is_err());
        assert!(batch.metadata(&dir).is_err());
        assert!(batch.same_volume(&dir, &target).is_err());
        drop(writer);
        drop(batch);
        std::fs::remove_dir(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn batch_casing_alias_uses_full_guard_fallback() {
        let t = fixture();
        let real = t.path().join("MiXeD");
        let alias = t.path().join("mixed");
        std::fs::create_dir(&real).unwrap();
        let mut batch = super::Batch::new();
        batch.guard(&alias.join("file")).unwrap();
        assert_eq!(
            batch.full_guard_prefixes,
            vec![super::absolute(&alias).unwrap()]
        );
        assert!(batch.directories.is_empty());
        batch
            .atomic_write(&alias.join("file"), b"compatible")
            .unwrap();
        assert_eq!(std::fs::read(real.join("file")).unwrap(), b"compatible");
        assert_eq!(batch.metadata(&alias.join("file")).unwrap().len(), 10);
        let unrelated = t.path().join("mixed-other");
        std::fs::create_dir(&unrelated).unwrap();
        batch.guard(&unrelated).unwrap();
        assert!(
            batch
                .directories
                .contains_key(&super::absolute(&unrelated).unwrap())
        );
        let count = batch.directories.len();
        batch.guard(&alias.join("another/missing")).unwrap();
        assert_eq!(batch.directories.len(), count);
        let link = real.join("junction");
        junction(&link, t.path());
        assert!(batch.guard(&alias.join("junction/missing")).is_err());
        assert!(batch.metadata(&alias.join("junction")).is_err());
        std::fs::remove_dir(&link).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn batch_rejects_wrong_handle_before_cache_insertion() {
        let t = fixture();
        let expected = super::absolute(&t.path().join("expected")).unwrap();
        let elsewhere = t.path().join("elsewhere");
        std::fs::create_dir(&expected).unwrap();
        std::fs::create_dir(&elsewhere).unwrap();
        let mut batch = super::Batch::new();
        let result = batch.pin_directory_using(&expected, &mut |path| {
            if path == expected {
                super::windows::open_directory(&elsewhere)
            } else {
                super::windows::open_directory(path)
            }
        });
        assert!(!result.unwrap());
        assert!(batch.requires_full_guard(&expected));
        assert!(batch.directories.is_empty());
        assert!(batch.guard(&expected).is_ok());
        assert!(batch.directories.is_empty());
        batch.guard(&elsewhere).unwrap();
        assert!(
            batch
                .directories
                .contains_key(&super::absolute(&elsewhere).unwrap())
        );
        let link = expected.join("junction");
        junction(&link, &elsewhere);
        assert!(batch.guard(&link.join("missing")).is_err());
        assert!(
            !batch
                .directories
                .keys()
                .any(|path| path.starts_with(&expected))
        );
        std::fs::remove_dir(&link).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn batch_missing_parent_never_opens_or_caches_descendant() {
        let t = fixture();
        let absent = t.path().join("absent");
        let real = t.path().join("real");
        std::fs::create_dir_all(real.join("child")).unwrap();
        let absent = super::absolute(&absent).unwrap();
        let child = absent.join("child");
        let mut batch = super::Batch::new();
        let mut opened_child = false;
        let mut injected = false;
        let exists = batch
            .pin_directory_using(&child, &mut |path| {
                if path == child {
                    opened_child = true;
                }
                if path == absent {
                    // Simulate creation of a junction immediately after the open
                    // has observed a missing parent, before that result is handled.
                    junction(&absent, &real);
                    injected = true;
                    return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
                }
                super::windows::open_directory(path)
            })
            .unwrap();
        assert!(injected);
        assert!(!exists);
        assert!(!opened_child);
        assert!(!batch.directories.contains_key(&absent));
        assert!(!batch.directories.contains_key(&child));
        assert!(batch.guard(&child.join("missing")).is_err());
        drop(batch);
        std::fs::remove_dir(&absent).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn batch_reuses_handles_but_inspects_new_leaves() {
        let t = fixture();
        let mut batch = super::Batch::new();
        batch.create_dir_all(&t.path().join("output")).unwrap();
        let count = batch.directories.len();
        for n in 0..12 {
            batch
                .atomic_write(&batch_path(t.path(), n), b"child")
                .unwrap();
        }
        assert!(count > 0);
        assert_eq!(batch.directories.len(), count);
        let leaf = t.path().join("output/junction");
        junction(&leaf, t.path());
        assert!(batch.guard(&leaf).is_err());
        assert!(batch.guard(&leaf.join("missing")).is_err());
        drop(batch);
        std::fs::remove_dir(&leaf).unwrap();
    }

    #[cfg(windows)]
    fn batch_path(root: &Path, n: usize) -> std::path::PathBuf {
        root.join(format!("output/file-{n}"))
    }

    #[test]
    fn batch_place_preserves_preexisting_staging_collision() {
        let t = fixture();
        let source = t.path().join("source");
        let target = t.path().join("target");
        std::fs::write(&source, b"source").unwrap();
        let mut collision = None;
        let mut batch = super::Batch::new();
        assert!(
            batch
                .place_using(&source, &target, false, |_, stage| {
                    std::fs::write(stage, b"someone else's staging file").unwrap();
                    collision = Some(stage.to_path_buf());
                    Err(std::io::Error::other("different volume"))
                })
                .is_err()
        );
        assert_eq!(
            std::fs::read(collision.unwrap()).unwrap(),
            b"someone else's staging file"
        );
        assert_eq!(std::fs::read(&source).unwrap(), b"source");
        assert!(!target.exists());
    }

    #[cfg(windows)]
    #[test]
    fn batch_place_rechecks_target_after_staging() {
        let t = fixture();
        let source = t.path().join("source");
        let target = t.path().join("target");
        let outside = t.path().join("outside");
        std::fs::write(&source, b"source").unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep"), b"unmanaged").unwrap();
        let mut staged = None;
        let mut batch = super::Batch::new();
        assert!(
            batch
                .place_using(&source, &target, false, |src, stage| {
                    std::fs::hard_link(src, stage)?;
                    staged = Some(stage.to_path_buf());
                    junction(&target, &outside);
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(std::fs::read(outside.join("keep")).unwrap(), b"unmanaged");
        assert!(!staged.unwrap().exists());
        drop(batch);
        std::fs::remove_dir(&target).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn batch_directory_attribute_query_rejects_regular_file() {
        let t = fixture();
        let path = t.path().join("file");
        std::fs::write(&path, b"file").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        assert!(super::windows::refresh_directory(&file).is_err());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "explicit metadata microbenchmark; no timing assertion"]
    fn batch_metadata_microbenchmark() {
        use std::os::windows::fs::MetadataExt;
        use std::{hint::black_box, time::Instant};
        let t = fixture();
        let dir = t.path().join("a/b/c/d/e/f");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("file");
        std::fs::write(&file, b"tiny").unwrap();
        let handle = super::windows::open_directory(&dir).unwrap();
        let rounds = 10000;
        let start = Instant::now();
        for _ in 0..rounds {
            black_box(std::fs::symlink_metadata(&dir).unwrap().file_attributes());
        }
        let path_time = start.elapsed();
        let start = Instant::now();
        for _ in 0..rounds {
            black_box(handle.metadata().unwrap().file_attributes());
        }
        let handle_time = start.elapsed();
        let start = Instant::now();
        for _ in 0..rounds {
            super::windows::refresh_directory(black_box(&handle)).unwrap();
        }
        let attribute_time = start.elapsed();
        let mut batch = super::Batch::new();
        batch.guard(&file).unwrap();
        let start = Instant::now();
        for _ in 0..2000 {
            super::guard(&file).unwrap();
        }
        let full_guard = start.elapsed();
        let start = Instant::now();
        for _ in 0..2000 {
            batch.guard(&file).unwrap();
        }
        let batch_guard = start.elapsed();
        eprintln!(
            "metadata {rounds}: path={path_time:?}, handle={handle_time:?}, attributes={attribute_time:?}; guard 2000: full={full_guard:?}, batch={batch_guard:?}"
        );
    }

    #[test]
    fn batch_metadata_returns_checked_fresh_leaf_and_errors_for_missing() {
        let t = fixture();
        let path = t.path().join("file");
        std::fs::write(&path, b"fresh").unwrap();
        let mut batch = super::Batch::new();
        assert_eq!(batch.metadata(&path).unwrap().len(), 5);
        std::fs::write(&path, b"new contents").unwrap();
        assert_eq!(batch.metadata(&path).unwrap().len(), 12);
        std::fs::remove_file(&path).unwrap();
        assert!(batch.metadata(&path).is_err());
        assert!(batch.guard(&path).is_ok());
    }

    #[test]
    fn fs_same_volume_uses_existing_ancestor_for_missing_output() {
        let t = fixture();
        let mut batch = super::Batch::new();
        assert!(
            batch
                .same_volume(t.path(), &t.path().join("missing/deep/output"))
                .unwrap()
        );
        assert!(super::same_volume(t.path(), t.path()).unwrap());
        assert!(batch.same_volume(Path::new(""), t.path()).is_err());
    }

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
        let mut batch = super::Batch::new();
        let batch_target = t.path().join("batch-target");
        let (linked, reason) = batch
            .place_using(&source, &batch_target, false, |_, _| {
                Err(std::io::Error::other("different volume"))
            })
            .unwrap();
        assert!(!linked);
        assert!(reason.unwrap().contains("storage cost"));
        assert_eq!(std::fs::read(&batch_target).unwrap(), b"source bytes");
        std::fs::write(&batch_target, b"batch local edit").unwrap();
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
        std::fs::hard_link(&src, &dst).unwrap();
        let mut batch = super::Batch::new();
        batch.remove_file(&dst).unwrap();
        assert!(!dst.exists());
        assert!(std::fs::metadata(&src).unwrap().permissions().readonly());
        assert_eq!(std::fs::read(&src).unwrap(), b"original");
        super::remove_file(&src).unwrap();
    }
}
