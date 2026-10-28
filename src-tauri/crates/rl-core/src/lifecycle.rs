//! Per-user installation and verified self-update. No game files are owned here.
use crate::{
    dto::LauncherUpdate,
    job::{self, Reporter},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const LAUNCHER_REPO: &str = "stormie-labs/RayLink-Launcher";
const MARKER: &str = ".raylink-install.json";
const LAUNCHER: &str = "RayLinkLauncher.exe";
const UNINSTALLER: &str = "uninstall.exe";

const PRODUCT: &str = "RayLink Launcher";
const PENDING: &str = ".raylink-update.json";
const MAX_SETUP_SIZE: u64 = 512 * 1024 * 1024;
const LOCK: &str = ".raylink-lifecycle.lock";

// Windows io::Error uses the OS display language. Keep backend diagnostics stable
// so the bilingual view can translate phases without receiving Russian OS prose.
fn error_text<E: std::fmt::Display + 'static>(error: E) -> String {
    if let Some(io) = (&error as &dyn std::any::Any).downcast_ref::<std::io::Error>() {
        return match io.raw_os_error() {
            Some(code) => format!("OS error {code}"),
            None => format!("I/O error: {:?}", io.kind()),
        };
    }
    format!("{error}")
}

struct LifecycleLock {
    _file: fs::File,
    #[cfg(not(windows))]
    path: PathBuf,
}
impl LifecycleLock {
    fn acquire(home: &Path) -> Result<Self, String> {
        let path = home.join(LOCK);
        ensure_no_reparse(&path)?;
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::{
                Foundation::{GENERIC_READ, GENERIC_WRITE},
                Storage::FileSystem::{
                    DELETE, FILE_FLAG_DELETE_ON_CLOSE, FILE_FLAG_OPEN_REPARSE_POINT,
                },
            };
            options
                .create(true)
                .truncate(false)
                .share_mode(0)
                .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
                .custom_flags(FILE_FLAG_DELETE_ON_CLOSE | FILE_FLAG_OPEN_REPARSE_POINT);
        }
        #[cfg(not(windows))]
        options.create_new(true);
        let file = options
            .open(&path)
            .map_err(|_| "Another install, update or uninstall is using this RayLink directory")?;
        Ok(Self {
            _file: file,
            #[cfg(not(windows))]
            path,
        })
    }
}
#[cfg(not(windows))]
impl Drop for LifecycleLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct InstallMarker {
    schema: u32,
    product: String,
    home: PathBuf,
    language: String,
    desktop: bool,
    test_install: bool,
    version: String,
}
#[derive(Debug, Serialize, Deserialize)]
struct PendingUpdate {
    schema: u32,
    home: PathBuf,
    version: String,
    sha256: String,
}

pub fn is_installer() -> bool {
    std::env::args_os().any(|a| a == "--setup")
        || std::env::current_exe()
            .ok()
            .and_then(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().eq_ignore_ascii_case("RayLinkSetup.exe"))
            })
            .unwrap_or(false)
}
pub fn is_installed(home: &Path) -> bool {
    load_marker(home).is_ok()
}
pub fn installed_language(home: &Path) -> Option<String> {
    load_marker(home).ok().map(|m| m.language)
}
pub fn default_install_dir() -> PathBuf {
    #[cfg(windows)]
    {
        native::local_app_data()
            .unwrap_or_else(|| {
                std::env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(r"C:\RayLink"))
            })
            .join("RayLink")
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join(".local/share")
            })
            .join("RayLink")
    }
}
/// Returns the installed launcher executable, suitable for opening after setup completes.
pub fn install_to(
    destination: &Path,
    language: &str,
    desktop: bool,
    progress: &Reporter,
) -> Result<PathBuf, String> {
    let source = std::env::current_exe()
        .map_err(|e| format!("Cannot locate the installer: {e}", e = error_text(e)))?;
    install_from(&source, destination, language, desktop, false, progress)
        .map(|home| home.join(LAUNCHER))
}

/// Must be called before creating the Tauri application, including single-instance plugins.
pub fn early_mode() -> Result<bool, String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let exe = std::env::current_exe()
        .map_err(|e| format!("Cannot locate the executable: {e}", e = error_text(e)))?;
    if let Some(index) = args.iter().position(|a| a == "--finish-uninstall") {
        let home = args
            .get(index + 1)
            .map(PathBuf::from)
            .ok_or("Missing uninstall directory")?;
        let pid = args
            .get(index + 2)
            .and_then(|a| a.to_str())
            .ok_or("Missing uninstaller process ID")?
            .parse::<u32>()
            .map_err(|_| "Invalid uninstaller process ID")?;
        let original = args
            .get(index + 3)
            .and_then(|a| a.to_str())
            .ok_or("Missing original executable name")?;
        if original != LAUNCHER && original != UNINSTALLER {
            return Err("Invalid original executable name".into());
        }
        load_marker(&home)?;
        let parent = exe.parent().ok_or("Missing uninstall helper directory")?;
        if exe.file_name().is_none_or(|n| n != "RayLinkCleanup.exe")
            || !parent
                .parent()
                .is_some_and(|p| same_path(p, &home.join("cache")))
            || !parent
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".launcher-uninstall-"))
        {
            return Err("The uninstall helper is outside the owned launcher cache".into());
        }
        wait_for_launcher(pid, &home.join(original))?;
        uninstall_from(&home)?;
        return Ok(true);
    }
    if let Some(index) = args.iter().position(|a| a == "--apply-update") {
        let home = args
            .get(index + 1)
            .map(PathBuf::from)
            .ok_or("Missing update installation directory")?;
        let pid: u32 = args
            .get(index + 2)
            .and_then(|a| a.to_str())
            .ok_or("Missing original process ID")?
            .parse()
            .map_err(|_| "Invalid original process ID")?;
        let pending = validate_pending(&exe, &home)?;
        wait_for_launcher(pid, &home.join(LAUNCHER))?;
        apply_payload(&exe, &home, &pending)?;
        start_launcher(&home)?;
        return Ok(true);
    }
    if let Some(index) = args.iter().position(|a| a == "--silent-install") {
        let home = args
            .get(index + 1)
            .map(PathBuf::from)
            .ok_or("Missing installation directory")?;
        let language = match args.iter().position(|a| a == "--language") {
            Some(i) => args
                .get(i + 1)
                .and_then(|a| a.to_str())
                .ok_or("Missing installation language")?,
            None => "en",
        };
        install_from(
            &exe,
            &home,
            language,
            args.iter().any(|a| a == "--desktop-shortcut"),
            args.iter().any(|a| a == "--test-install"),
            &|_| {},
        )?;
        return Ok(true);
    }
    let uninstall = args.iter().any(|a| a == "--uninstall")
        || exe
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(UNINSTALLER));
    if uninstall {
        let home = exe
            .parent()
            .ok_or("Cannot determine the installation directory")?;
        let marker = load_marker(home)?;
        #[cfg(windows)]
        if !marker.test_install
            && !args.iter().any(|a| a == "--yes")
            && !native::confirm_uninstall(&marker.language)
        {
            return Ok(true);
        }
        let _ = marker;
        let original = if same_path(&exe, &home.join(UNINSTALLER)) {
            UNINSTALLER
        } else if same_path(&exe, &home.join(LAUNCHER)) {
            LAUNCHER
        } else {
            return Err("Run the uninstaller from the installed RayLink directory".into());
        };
        // A separate copy can remove either installed executable after it exits without elevation.
        let cache = home.join("cache");
        ensure_dir(&cache)?;
        let helper = new_work_dir(&cache, "launcher-uninstall")?.join("RayLinkCleanup.exe");
        fs::copy(&exe, &helper)
            .map_err(|e| format!("Cannot stage the uninstall helper: {e}", e = error_text(e)))?;
        Command::new(&helper)
            .arg("--finish-uninstall")
            .arg(home)
            .arg(std::process::id().to_string())
            .arg(original)
            .current_dir(home)
            .spawn()
            .map_err(|e| format!("Cannot start the uninstall helper: {e}", e = error_text(e)))?;
        return Ok(true);
    }
    Ok(false)
}

pub fn check_update(current_version: &str) -> Result<Option<LauncherUpdate>, String> {
    let release = crate::channel::latest(LAUNCHER_REPO)?;
    if !newer(current_version, &release.tag)? {
        return Ok(None);
    }
    let update = LauncherUpdate {
        version: parse_version(&release.tag)?.to_string(),
        release,
    };
    validate_update(&update)?;
    Ok(Some(update))
}
pub fn begin_update(
    update: &LauncherUpdate,
    home: &Path,
    cancel: &AtomicBool,
    progress: &Reporter,
) -> Result<(), String> {
    validate_update(update)?;
    let marker = load_marker(home)?;
    if !newer(&marker.version, &update.version)? {
        return Err("This launcher update is not newer than the installed version".into());
    }
    job::check_cancel(cancel)?;
    let cache = home.join("cache");
    ensure_dir(&cache)?;
    let dir = new_work_dir(&cache, "launcher-update")?;
    let setup = dir.join("RayLinkSetup.exe");
    job::report(
        progress,
        "launcherDownload",
        "Downloading the launcher update",
        0,
        update.release.asset_size,
        true,
    );
    // This DTO is retained by the native caller after check_update; it must never be accepted from the webview.
    crate::channel::download(&update.release, &setup, cancel, progress)?;
    job::check_cancel(cancel)?;
    let expected = update
        .release
        .sha256
        .as_deref()
        .ok_or("The launcher update has no SHA-256 digest")?;
    // Keep the verified image and its path immutable until the helper has started.
    let _payload = verify_hash(&setup, expected)?;
    let pending = PendingUpdate {
        schema: 1,
        home: canonical(home)?,
        version: update.version.clone(),
        sha256: expected.to_ascii_lowercase(),
    };
    write_json(&dir.join(PENDING), &pending)?;
    job::check_cancel(cancel)?;
    job::report(
        progress,
        "launcherApply",
        "Restarting to finish the launcher update",
        0,
        1,
        false,
    );
    Command::new(&setup)
        .arg("--apply-update")
        .arg(home)
        .arg(std::process::id().to_string())
        .current_dir(home)
        .spawn()
        .map_err(|e| {
            format!(
                "Cannot start the launcher update helper: {e}",
                e = error_text(e)
            )
        })?;
    Ok(())
}
fn parse_version(version: &str) -> Result<semver::Version, String> {
    semver::Version::parse(version.strip_prefix('v').unwrap_or(version))
        .map_err(|_| "Invalid launcher semantic version".into())
}
fn newer(current: &str, candidate: &str) -> Result<bool, String> {
    let current = parse_version(current)?;
    let candidate = parse_version(candidate)?;
    Ok(candidate.pre.is_empty() && candidate > current)
}
fn validate_update(update: &LauncherUpdate) -> Result<(), String> {
    let version = parse_version(&update.version)?;
    if !version.pre.is_empty() || version != parse_version(&update.release.tag)? {
        return Err("Launcher update versions do not match".into());
    }
    let r = &update.release;
    if r.asset_name != "RayLinkSetup.exe" || r.asset_size == 0 || r.asset_size > MAX_SETUP_SIZE {
        return Err("Invalid launcher installer asset".into());
    }
    let hash = r
        .sha256
        .as_deref()
        .ok_or("The launcher update has no SHA-256 digest")?;
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid launcher SHA-256 digest".into());
    }
    let expected = format!(
        "https://github.com/{LAUNCHER_REPO}/releases/download/{}/RayLinkSetup.exe",
        r.tag
    );
    if r.asset_url != expected {
        return Err("The launcher update URL does not belong to the trusted release".into());
    }
    Ok(())
}

fn validate_destination(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path.parent().is_none()
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err("Choose an absolute installation directory below the drive root".into());
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        if !matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        {
            return Err("Choose a directory on a local drive".into());
        }
        for c in path.components() {
            if let Component::Normal(n) = c {
                let n = n.to_string_lossy();
                if n.ends_with(['.', ' ']) || n.contains(':') {
                    return Err("The installation path contains an unsupported component".into());
                }
            }
        }
        if native::desktop().is_some_and(|d| same_path(path, &d)) {
            return Err("Choose a RayLink subdirectory instead of the Desktop itself".into());
        }
    }
    ensure_no_reparse(path)?;
    for ancestor in path.ancestors() {
        if ancestor.join("ModOrganizer.exe").exists()
            || ancestor.join("AnomalyLauncher.exe").exists()
            || ancestor.join("fsgame.ltx").exists()
            || ancestor.join(".Grok's Modpack Installer").exists()
            || (ancestor.join("mods").is_dir() && ancestor.join("profiles").is_dir())
        {
            return Err("Install RayLink outside the Anomaly and Mod Organizer directories".into());
        }
    }
    if path.exists() && !path.is_dir() {
        return Err("The installation destination is not a directory".into());
    }
    Ok(())
}
fn ensure_no_reparse(path: &Path) -> Result<(), String> {
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) => {
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    m.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let linked = m.file_type().is_symlink();
                if linked {
                    return Err(
                        "Installation paths cannot contain symbolic links or reparse points".into(),
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "Cannot inspect the installation path: {e}",
                    e = error_text(e)
                ));
            }
        }
    }
    Ok(())
}
fn canonical(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|e| {
        format!(
            "Cannot resolve the installation path: {e}",
            e = error_text(e)
        )
    })
}
fn same_path(a: &Path, b: &Path) -> bool {
    let a = fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .trim_start_matches(r"\\?\")
            .eq_ignore_ascii_case(b.to_string_lossy().trim_start_matches(r"\\?\"))
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
fn regular(path: &Path) -> Result<(), String> {
    ensure_no_reparse(path)?;
    if !fs::metadata(path)
        .map_err(|e| format!("Cannot read {}: {e}", path.display(), e = error_text(e)))?
        .is_file()
    {
        return Err("A managed installation file is not a regular file".into());
    }
    Ok(())
}
fn ensure_dir(path: &Path) -> Result<(), String> {
    ensure_no_reparse(path)?;
    fs::create_dir_all(path)
        .map_err(|e| format!("Cannot create {}: {e}", path.display(), e = error_text(e)))?;
    ensure_no_reparse(path)
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    regular(path)?;
    let file = fs::File::open(path).map_err(|e| {
        format!(
            "Cannot open the installation metadata: {e}",
            e = error_text(e)
        )
    })?;
    if file.metadata().map_err(error_text)?.len() > 65536 {
        return Err("Installation metadata is too large".into());
    }
    serde_json::from_reader(file)
        .map_err(|e| format!("Invalid installation metadata: {e}", e = error_text(e)))
}
fn load_marker(home: &Path) -> Result<InstallMarker, String> {
    validate_destination(home)?;
    let marker: InstallMarker = read_json(&home.join(MARKER))?;
    if marker.schema != 1 || marker.product != PRODUCT || !same_path(home, &marker.home) {
        return Err("This directory is not an owned RayLink installation".into());
    }
    language(&marker.language)?;
    parse_version(&marker.version)?;
    Ok(marker)
}
fn language(value: &str) -> Result<&str, String> {
    match value {
        "" => Ok("en"),
        "en" | "ru" => Ok(value),
        _ => Err("Unsupported installation language".into()),
    }
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(error_text)?;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            format!(
                "Cannot create the installation metadata: {e}",
                e = error_text(e)
            )
        })?;
    f.write_all(&bytes).and_then(|_| f.sync_all()).map_err(|e| {
        format!(
            "Cannot save the installation metadata: {e}",
            e = error_text(e)
        )
    })
}
fn new_work_dir(home: &Path, prefix: &str) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(error_text)?
        .as_nanos();
    let path = home.join(format!(".{prefix}-{}-{stamp}", std::process::id()));
    fs::create_dir(&path).map_err(|e| {
        format!(
            "Cannot create the installation staging directory: {e}",
            e = error_text(e)
        )
    })?;
    Ok(path)
}
fn install_from(
    source: &Path,
    destination: &Path,
    selected_language: &str,
    desktop: bool,
    test: bool,
    progress: &Reporter,
) -> Result<PathBuf, String> {
    let selected_language = language(selected_language)?;
    validate_destination(destination)?;
    regular(source)?;
    ensure_dir(destination)?;
    let _lock = LifecycleLock::acquire(destination)?;
    let previous = if destination.join(MARKER).exists() {
        Some(load_marker(destination)?)
    } else {
        None
    };
    if previous.is_none()
        && fs::read_dir(destination)
            .map_err(error_text)?
            .any(|entry| entry.map_or(true, |e| e.file_name() != LOCK))
    {
        return Err("Choose an empty directory or an existing RayLink installation".into());
    }
    if previous.as_ref().is_some_and(|m| m.test_install != test) {
        return Err("Test and registered installations cannot replace each other".into());
    }
    job::report(
        progress,
        "installPrepare",
        "Preparing the installation directory",
        0,
        4,
        false,
    );
    ensure_dir(destination)?;
    ensure_dir(&destination.join("builds"))?;
    ensure_dir(&destination.join("cache"))?;
    let home = canonical(destination)?;
    let marker = InstallMarker {
        schema: 1,
        product: PRODUCT.into(),
        home: home.clone(),
        language: selected_language.into(),
        desktop,
        test_install: test,
        version: env!("CARGO_PKG_VERSION").into(),
    };
    let stage = stage_install(source, &home, &marker)?;
    job::report(
        progress,
        "installCopy",
        "Installing the launcher",
        1,
        4,
        false,
    );
    commit_files(&home, &stage)?;
    cleanup_stage(&stage)?;
    job::report(
        progress,
        "installShortcuts",
        "Creating application shortcuts",
        3,
        4,
        false,
    );
    #[cfg(windows)]
    if !test {
        native::register(&home, desktop, &marker.version)?;
    }
    job::report(
        progress,
        "installComplete",
        "Installation complete",
        4,
        4,
        false,
    );
    Ok(home)
}
fn stage_install(source: &Path, home: &Path, marker: &InstallMarker) -> Result<PathBuf, String> {
    let stage = new_work_dir(home, "raylink-stage")?;
    for file in [LAUNCHER, UNINSTALLER] {
        let to = stage.join(file);
        fs::copy(source, &to).map_err(|e| {
            format!(
                "Cannot stage the launcher executable: {e}",
                e = error_text(e)
            )
        })?;
        fs::OpenOptions::new()
            .write(true)
            .open(&to)
            .and_then(|f| f.sync_all())
            .map_err(|e| {
                format!(
                    "Cannot flush the launcher executable: {e}",
                    e = error_text(e)
                )
            })?;
    }
    write_json(&stage.join(MARKER), marker)?;
    Ok(stage)
}
/// Only these three files are replaced. Backups survive any failed rollback.
fn commit_files(home: &Path, stage: &Path) -> Result<(), String> {
    let names = [LAUNCHER, UNINSTALLER, MARKER];
    for name in names {
        ensure_no_reparse(&home.join(name))?;
        if home.join(name).exists() {
            regular(&home.join(name))?;
        }
        regular(&stage.join(name))?;
    }
    let backup = stage.join("backup");
    fs::create_dir(&backup).map_err(error_text)?;
    let mut moved: Vec<(&str, bool)> = Vec::new();
    for name in names {
        let target = home.join(name);
        let had_old = target.exists();
        if had_old {
            if let Err(error) = replace_with_backup(&target, &stage.join(name), &backup.join(name))
            {
                return rollback(home, &backup, &moved, error);
            }
        } else if let Err(error) = move_file(&stage.join(name), &target) {
            return rollback(home, &backup, &moved, error);
        }
        moved.push((name, had_old));
    }
    Ok(())
}
fn rollback(
    home: &Path,
    backup: &Path,
    moved: &[(&str, bool)],
    error: String,
) -> Result<(), String> {
    for (name, had_old) in moved.iter().rev() {
        let target = home.join(name);
        if target.exists() && !*had_old {
            regular(&target)?;
            fs::remove_file(&target).map_err(|e| {
                format!(
                    "{error}; rollback could not remove {}: {e}",
                    target.display(),
                    e = error_text(e)
                )
            })?;
        }
        if *had_old {
            restore_file(&backup.join(name), &target).map_err(|e| {
                format!(
                    "{error}; rollback failed: {e}. Backup: {}",
                    backup.display()
                )
            })?;
        }
    }
    Err(error)
}
fn replace_with_backup(target: &Path, replacement: &Path, backup: &Path) -> Result<(), String> {
    // Snapshot the existing immutable image without renaming its visible path.
    // Hard links are cheap on NTFS; copy+flush also supports other local filesystems.
    if fs::hard_link(target, backup).is_err() {
        fs::copy(target, backup).map_err(error_text)?;
        fs::OpenOptions::new()
            .write(true)
            .open(backup)
            .and_then(|f| f.sync_all())
            .map_err(error_text)?;
    }
    #[cfg(windows)]
    {
        native::restore_file(replacement, target)
    }
    #[cfg(not(windows))]
    {
        fs::rename(replacement, target).map_err(error_text)
    }
}
fn restore_file(backup: &Path, target: &Path) -> Result<(), String> {
    regular(backup)?;
    ensure_no_reparse(target)?;
    #[cfg(windows)]
    {
        native::restore_file(backup, target)
    }
    #[cfg(not(windows))]
    {
        fs::rename(backup, target).map_err(error_text)
    }
}
fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    for attempt in 0..30 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e)
                if attempt < 29
                    && matches!(
                        e.kind(),
                        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::WouldBlock
                    ) =>
            {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(e) => {
                return Err(format!(
                    "Cannot replace an installation file: {e}",
                    e = error_text(e)
                ));
            }
        }
    }
    Err("The installation file remained locked".into())
}
fn cleanup_stage(stage: &Path) -> Result<(), String> {
    // Never recursively delete. Every removable name is a constant owned by this transaction.
    for dir in [stage.join("backup"), stage.to_path_buf()] {
        ensure_no_reparse(&dir)?;
        for name in [LAUNCHER, UNINSTALLER, MARKER] {
            let file = dir.join(name);
            if file.exists() {
                regular(&file)?;
                fs::remove_file(&file).map_err(error_text)?;
            }
        }
        if dir.exists() {
            fs::remove_dir(&dir).map_err(|e| {
                format!(
                    "Cannot remove an empty installation staging directory: {e}",
                    e = error_text(e)
                )
            })?;
        }
    }
    Ok(())
}
fn verify_hash(path: &Path, expected: &str) -> Result<fs::File, String> {
    regular(path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
        // Deny writes and rename/delete while the caller copies or starts these bytes.
        options.share_mode(FILE_SHARE_READ);
    }
    let mut f = options.open(path).map_err(error_text)?;
    if f.metadata().map_err(error_text)?.len() > MAX_SETUP_SIZE {
        return Err("The launcher installer exceeds its size limit".into());
    }
    let mut digest = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf).map_err(error_text)?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    let actual: String = digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual.eq_ignore_ascii_case(expected) {
        Ok(f)
    } else {
        Err("The launcher installer SHA-256 digest does not match".into())
    }
}
fn validate_pending(source: &Path, home: &Path) -> Result<PendingUpdate, String> {
    let marker = load_marker(home)?;
    regular(source)?;
    let dir = source.parent().ok_or("Missing update helper directory")?;
    if source.file_name().is_none_or(|n| n != "RayLinkSetup.exe")
        || !dir
            .parent()
            .is_some_and(|p| same_path(p, &home.join("cache")))
        || !dir
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(".launcher-update-"))
    {
        return Err("The update helper is outside the owned launcher cache".into());
    }
    let pending: PendingUpdate = read_json(&dir.join(PENDING))?;
    if pending.schema != 1
        || !same_path(&pending.home, home)
        || !newer(&marker.version, &pending.version)?
    {
        return Err("Invalid pending launcher update".into());
    }
    if pending.sha256.len() != 64 || !pending.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid pending launcher digest".into());
    }
    verify_hash(source, &pending.sha256)?;
    Ok(pending)
}
fn apply_payload(source: &Path, home: &Path, pending: &PendingUpdate) -> Result<(), String> {
    let _lock = LifecycleLock::acquire(home)?;
    let mut marker = load_marker(home)?;
    if !newer(&marker.version, &pending.version)? {
        return Err("This launcher update has already been applied or superseded".into());
    }
    let _payload = verify_hash(source, &pending.sha256)?;
    marker.version = pending.version.clone();
    let stage = stage_install(source, home, &marker)?;
    commit_files(home, &stage)?;
    cleanup_stage(&stage)?;
    #[cfg(windows)]
    if !marker.test_install {
        native::register(home, marker.desktop, &marker.version)?;
    }
    // The running helper and its metadata are retained in cache. Never delete our own image.
    Ok(())
}
fn start_launcher(home: &Path) -> Result<(), String> {
    Command::new(home.join(LAUNCHER))
        .current_dir(home)
        .spawn()
        .map_err(|e| {
            format!(
                "The launcher was updated but could not restart: {e}",
                e = error_text(e)
            )
        })?;
    Ok(())
}
fn wait_for_launcher(pid: u32, expected: &Path) -> Result<(), String> {
    if pid == 0 || pid == std::process::id() {
        return Err("Invalid original launcher process ID".into());
    }
    #[cfg(windows)]
    {
        native::wait_for_launcher(pid, expected)
    }
    #[cfg(not(windows))]
    {
        let _ = expected;
        Err("Launcher self-update requires Windows".into())
    }
}
fn uninstall_from(home: &Path) -> Result<(), String> {
    load_marker(home)?;
    let _lock = LifecycleLock::acquire(home)?;
    let marker = load_marker(home)?;
    // Retain delete-capable handles through removal. A read/write-only preflight
    // misses handles that allow writing but deny deletion of the second executable.
    let mut owned_files = Vec::new();
    for name in [LAUNCHER, UNINSTALLER, MARKER] {
        let file = home.join(name);
        if file.exists() {
            regular(&file)?;
            let mut options = fs::OpenOptions::new();
            options.read(true).write(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                use windows_sys::Win32::{
                    Foundation::{GENERIC_READ, GENERIC_WRITE},
                    Storage::FileSystem::{
                        DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
                    },
                };
                options
                    .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
            }
            owned_files.push(
                options
                    .open(&file)
                    .map_err(|_| "Close RayLink Launcher before uninstalling")?,
            );
        }
    }
    #[cfg(windows)]
    if !marker.test_install {
        native::unregister(home)?;
    }
    let _ = marker;
    for name in [LAUNCHER, UNINSTALLER] {
        let file = home.join(name);
        if !file.exists() {
            continue;
        }
        fs::remove_file(&file).map_err(|e| {
            format!(
                "Cannot remove an owned launcher executable: {e}",
                e = error_text(e)
            )
        })?;
    }
    fs::remove_file(home.join(MARKER)).map_err(|e| {
        format!(
            "Cannot remove the installation marker: {e}",
            e = error_text(e)
        )
    })?;
    Ok(())
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ffi::{OsStr, OsString, c_void},
        os::windows::ffi::{OsStrExt, OsStringExt},
        ptr,
    };
    use windows_sys::{
        Win32::{
            Foundation::{
                CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER, GetLastError,
                RPC_E_CHANGED_MODE, WAIT_OBJECT_0,
            },
            System::{
                Com::{
                    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance,
                    CoInitializeEx, CoTaskMemFree, CoUninitialize,
                },
                Registry::*,
                Threading::{
                    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
                    QueryFullProcessImageNameW, WaitForSingleObject,
                },
            },
            UI::{
                Shell::{
                    FOLDERID_Desktop, FOLDERID_LocalAppData, FOLDERID_Programs,
                    SHGetKnownFolderPath,
                },
                WindowsAndMessaging::{
                    IDYES, MB_DEFBUTTON2, MB_ICONQUESTION, MB_YESNO, MessageBoxW,
                },
            },
        },
        core::GUID,
    };

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    pub(super) fn restore_file(backup: &Path, target: &Path) -> Result<(), String> {
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let backup = wide(backup.as_os_str());
        let target = wide(target.as_os_str());
        for attempt in 0..30 {
            // SAFETY: These are preflighted owned paths; replacement preserves a visible target.
            if unsafe {
                MoveFileExW(
                    backup.as_ptr(),
                    target.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } != 0
            {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            if attempt == 29 || !matches!(error.raw_os_error(), Some(5 | 32 | 33)) {
                return Err(error_text(error));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err("Cannot restore the launcher backup".into())
    }
    fn folder(id: &GUID) -> Option<PathBuf> {
        let mut out = ptr::null_mut();
        // SAFETY: Windows allocates a null-terminated UTF-16 path; the matching COM allocator frees it.
        unsafe {
            if SHGetKnownFolderPath(id, 0, ptr::null_mut(), &mut out) < 0 || out.is_null() {
                return None;
            }
            let mut len = 0;
            while *out.add(len) != 0 {
                len += 1;
            }
            let result = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(out, len)));
            CoTaskMemFree(out.cast());
            Some(result)
        }
    }
    pub(super) fn local_app_data() -> Option<PathBuf> {
        folder(&FOLDERID_LocalAppData)
    }
    pub(super) fn desktop() -> Option<PathBuf> {
        folder(&FOLDERID_Desktop)
    }
    pub(super) fn confirm_uninstall(language: &str) -> bool {
        let title = wide(OsStr::new(PRODUCT));
        let message = wide(OsStr::new(if language == "ru" {
            "Удалить RayLink Launcher?\n\nСборки, кэш, настройки, подготовленные игры и сохранения останутся на месте."
        } else { "Uninstall RayLink Launcher?\n\nYour builds, cache, settings, game stands and saved games will be kept." }));
        // SAFETY: Both strings are terminated and alive for the modal call; no owner window is required.
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
            ) == IDYES
        }
    }
    pub(super) fn wait_for_launcher(pid: u32, expected: &Path) -> Result<(), String> {
        // SAFETY: Read/query rights only. Keep the handle through the wait to avoid PID reuse.
        unsafe {
            let handle = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            );
            if handle.is_null() {
                return if GetLastError() == ERROR_INVALID_PARAMETER {
                    Ok(())
                } else {
                    Err("Cannot inspect the original launcher process".into())
                };
            }
            let mut buf = vec![0u16; 32768];
            let mut count = buf.len() as u32;
            let query = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut count);
            if query == 0 {
                CloseHandle(handle);
                return Err("Cannot identify the original launcher executable".into());
            }
            let actual = PathBuf::from(OsString::from_wide(&buf[..count as usize]));
            if !same_path(&actual, expected) {
                CloseHandle(handle);
                return Err("The process ID does not belong to the installed launcher".into());
            }
            let result = WaitForSingleObject(handle, 60000);
            CloseHandle(handle);
            if result != WAIT_OBJECT_0 {
                return Err("The original launcher did not exit in time".into());
            }
        }
        Ok(())
    }

    // Minimal ABI for the documented IShellLinkW / IPersistFile interfaces. windows-sys supplies
    // the COM runtime; interfaces are kept here so no second Windows binding crate is needed.
    #[repr(C)]
    struct UnknownV {
        query: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
        _add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }
    #[repr(C)]
    struct ShellV {
        base: UnknownV,
        get_path: unsafe extern "system" fn(*mut c_void, *mut u16, i32, *mut c_void, u32) -> i32,
        _id_list: [usize; 2],
        _get_description: usize,
        set_description: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
        _get_working: usize,
        set_working: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
        _arguments_hotkey_show: [usize; 6],
        _get_icon: usize,
        set_icon: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32,
        _relative_resolve: [usize; 2],
        set_path: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
    }
    #[repr(C)]
    struct PersistV {
        base: UnknownV,
        _class_dirty: [usize; 2],
        load: unsafe extern "system" fn(*mut c_void, *const u16, u32) -> i32,
        save: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32,
        _save_complete_current: [usize; 2],
    }
    struct Apartment(bool);
    impl Apartment {
        fn new() -> Result<Self, String> {
            // SAFETY: The guard balances successful initialization on this thread only.
            let hr = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
            if hr < 0 && hr != RPC_E_CHANGED_MODE {
                return Err(format!("Cannot initialize Windows shortcuts: {hr:#x}"));
            }
            Ok(Self(hr >= 0))
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CoUninitialize();
                }
            }
        }
    }
    struct ComObject(*mut c_void);
    impl ComObject {
        fn base(&self) -> &UnknownV {
            unsafe { &**(self.0.cast::<*const UnknownV>()) }
        }
    }
    impl Drop for ComObject {
        fn drop(&mut self) {
            unsafe {
                (self.base().release)(self.0);
            }
        }
    }
    fn hr(result: i32) -> Result<(), String> {
        if result < 0 {
            Err(format!("Windows shortcut operation failed: {result:#x}"))
        } else {
            Ok(())
        }
    }
    fn link_objects() -> Result<(ComObject, ComObject), String> {
        let class = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
        let shell_iid = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
        let persist_iid = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);
        let mut shell = ptr::null_mut();
        let mut persist = ptr::null_mut();
        // SAFETY: Both interfaces follow the documented COM ABI and are released by RAII guards.
        unsafe {
            hr(CoCreateInstance(
                &class,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &shell_iid,
                &mut shell,
            ))?;
            let shell = ComObject(shell);
            hr((shell.base().query)(shell.0, &persist_iid, &mut persist))?;
            Ok((shell, ComObject(persist)))
        }
    }
    fn shell_path(path: &Path) -> PathBuf {
        let raw: Vec<u16> = path.as_os_str().encode_wide().collect();
        let prefix: Vec<u16> = r"\\?\".encode_utf16().collect();
        let unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
        if raw.starts_with(&unc) {
            let mut result = vec![92, 92]; result.extend_from_slice(&raw[unc.len()..]);
            PathBuf::from(OsString::from_wide(&result))
        } else if raw.starts_with(&prefix) { PathBuf::from(OsString::from_wide(&raw[prefix.len()..])) }
        else { path.to_path_buf() }
    }
    fn shortcut(path: &Path, target: &Path, home: &Path) -> Result<(), String> {
        let _apartment = Apartment::new()?;
        let (shell, persist) = link_objects()?;
        let target = wide(shell_path(target).as_os_str());
        let home = wide(shell_path(home).as_os_str());
        let description = wide(OsStr::new(PRODUCT));
        let path = wide(shell_path(path).as_os_str());
        unsafe {
            let v = &**shell.0.cast::<*const ShellV>();
            hr((v.set_path)(shell.0, target.as_ptr()))?;
            hr((v.set_working)(shell.0, home.as_ptr()))?;
            hr((v.set_description)(shell.0, description.as_ptr()))?;
            hr((v.set_icon)(shell.0, target.as_ptr(), 0))?;
            let p = &**persist.0.cast::<*const PersistV>();
            hr((p.save)(persist.0, path.as_ptr(), 1))?;
        }
        Ok(())
    }
    fn shortcut_target(path: &Path) -> Result<PathBuf, String> {
        regular(path)?;
        let _apartment = Apartment::new()?;
        let (shell, persist) = link_objects()?;
        let path = wide(shell_path(path).as_os_str());
        let mut target = vec![0u16; 32768];
        unsafe {
            let p = &**persist.0.cast::<*const PersistV>();
            hr((p.load)(persist.0, path.as_ptr(), 0))?;
            let s = &**shell.0.cast::<*const ShellV>();
            hr((s.get_path)(
                shell.0,
                target.as_mut_ptr(),
                target.len() as i32,
                ptr::null_mut(),
                4,
            ))?;
        }
        let len = target
            .iter()
            .position(|v| *v == 0)
            .ok_or("Invalid shortcut target")?;
        Ok(PathBuf::from(OsString::from_wide(&target[..len])))
    }
    const REG_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\RayLinkLauncher";
    struct RegKey(HKEY);
    impl Drop for RegKey {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn reg_string(key: HKEY, name: &str, value: &OsStr) -> Result<(), String> {
        let name = wide(OsStr::new(name));
        let value = wide(value);
        let code = unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        };
        if code != 0 {
            return Err(format!(
                "Cannot write the per-user uninstall entry: Windows error {code}"
            ));
        }
        Ok(())
    }
    fn reg_location() -> Option<PathBuf> {
        let path = wide(OsStr::new(REG_PATH));
        let name = wide(OsStr::new("InstallLocation"));
        let mut value = vec![0u16; 32768];
        let mut size = (value.len() * 2) as u32;
        let code = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                value.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if code != 0 {
            return None;
        }
        let len = value.iter().position(|c| *c == 0)?;
        Some(PathBuf::from(OsString::from_wide(&value[..len])))
    }
    pub(super) fn register(home: &Path, on_desktop: bool, version: &str) -> Result<(), String> {
        let programs = folder(&FOLDERID_Programs).ok_or("Cannot locate the user Start Menu")?;
        // A redirected Desktop or Start Menu is legitimate. Only the launcher home must be local.
        fs::create_dir_all(&programs).map_err(error_text)?;
        shortcut(
            &programs.join("RayLink Launcher.lnk"),
            &home.join(LAUNCHER),
            home,
        )?;
        if on_desktop {
            let desktop = desktop().ok_or("Cannot locate the Desktop")?;
            shortcut(
                &desktop.join("RayLink Launcher.lnk"),
                &home.join(LAUNCHER),
                home,
            )?;
        }
        let path = wide(OsStr::new(REG_PATH));
        let mut key = ptr::null_mut();
        let code = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                ptr::null(),
                0,
                KEY_SET_VALUE,
                ptr::null(),
                &mut key,
                ptr::null_mut(),
            )
        };
        if code != 0 {
            return Err(format!(
                "Cannot create the per-user uninstall entry: Windows error {code}"
            ));
        }
        let key = RegKey(key);
        for (name, value) in [
            ("DisplayName", OsString::from(PRODUCT)),
            ("DisplayVersion", OsString::from(version)),
            ("Publisher", OsString::from("stormie-labs")),
            ("InstallLocation", shell_path(home).into_os_string()),
            ("DisplayIcon", shell_path(&home.join(LAUNCHER)).into_os_string()),
            (
                "UninstallString",
                OsString::from(format!(
                    "\"{}\" --uninstall",
                    shell_path(&home.join(UNINSTALLER)).display()
                )),
            ),
            (
                "QuietUninstallString",
                OsString::from(format!(
                    "\"{}\" --uninstall --yes",
                    shell_path(&home.join(UNINSTALLER)).display()
                )),
            ),
        ] {
            reg_string(key.0, name, &value)?;
        }
        for name in ["NoModify", "NoRepair"] {
            let name = wide(OsStr::new(name));
            let value = 1u32;
            let code = unsafe {
                RegSetValueExW(
                    key.0,
                    name.as_ptr(),
                    0,
                    REG_DWORD,
                    (&value as *const u32).cast(),
                    4,
                )
            };
            if code != 0 {
                return Err(format!(
                    "Cannot finish the uninstall entry: Windows error {code}"
                ));
            }
        }
        Ok(())
    }
    pub(super) fn unregister(home: &Path) -> Result<(), String> {
        for folder in [folder(&FOLDERID_Programs), desktop()]
            .into_iter()
            .flatten()
        {
            let link = folder.join("RayLink Launcher.lnk");
            if link.exists() && same_path(&shortcut_target(&link)?, &home.join(LAUNCHER)) {
                fs::remove_file(&link).map_err(|e| {
                    format!(
                        "Cannot remove the launcher shortcut: {e}",
                        e = error_text(e)
                    )
                })?;
            }
        }
        if reg_location().is_some_and(|path| same_path(&path, home)) {
            let path = wide(OsStr::new(REG_PATH));
            let code = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, path.as_ptr()) };
            if code != 0 && code != ERROR_FILE_NOT_FOUND {
                return Err(format!(
                    "Cannot remove the uninstall entry: Windows error {code}"
                ));
            }
        }
        Ok(())
    }
    #[test]
    fn lifecycle_shell_link_roundtrip_stays_inside_fixture() {
        let root = super::tests::fixture();
        let link = root.path().join("RayLink.lnk");
        let target = root.path().join(LAUNCHER);
        fs::write(&target, b"MZfixture").unwrap();
        shortcut(&link, &target, root.path()).unwrap();
        assert!(same_path(&shortcut_target(&link).unwrap(), &target));
    }
    #[test]
    fn lifecycle_shell_link_accepts_canonical_extended_paths() {
        let root = super::tests::fixture();
        let home = fs::canonicalize(root.path()).unwrap();
        let target = home.join(LAUNCHER);
        let link = home.join("Canonical.lnk");
        fs::write(&target, b"MZfixture").unwrap();
        shortcut(&link, &target, &home).unwrap();
        assert!(same_path(&shortcut_target(&link).unwrap(), &target));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::Release;
    use std::fs;
    pub(super) fn fixture() -> tempfile::TempDir {
        let project = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|p| p.join("src-tauri/Cargo.toml").is_file())
            .unwrap();
        let root = project.join("temp/lifecycle-tests");
        fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }
    fn update() -> LauncherUpdate {
        LauncherUpdate { version: "0.3.0".into(), release: Release {
            tag: "v0.3.0".into(), name: "Launcher 0.3.0".into(), body: String::new(), published_at: String::new(),
            page_url: "https://github.com/stormie-labs/RayLink-Launcher/releases/tag/v0.3.0".into(),
            asset_name: "RayLinkSetup.exe".into(), asset_url: "https://github.com/stormie-labs/RayLink-Launcher/releases/download/v0.3.0/RayLinkSetup.exe".into(),
            asset_size: 1024, sha256: Some("a".repeat(64)),
        } }
    }
    #[test]
    fn lifecycle_version_comparison_is_semantic_and_rejects_game_tags() {
        for (current, candidate, expected) in [
            ("0.2.0", "v0.10.0", true),
            ("0.2.0", "v0.2.0", false),
            ("0.3.0", "v0.2.9", false),
            ("0.3.0-beta.1", "v0.3.0", true),
            ("0.3.0", "0.4.0-beta.1", false),
        ] {
            assert_eq!(newer(current, candidate).unwrap(), expected);
        }
        assert!(newer("0.2.0", "b24").is_err());
        assert!(newer("b24", "0.3.0").is_err());
    }
    #[test]
    fn lifecycle_refuses_relative_root_and_game_directories() {
        assert!(validate_destination(Path::new("relative/RayLink")).is_err());
        #[cfg(windows)]
        assert!(validate_destination(Path::new("C:\\")).is_err());
        let f = fixture();
        fs::write(f.path().join("ModOrganizer.exe"), b"game fixture").unwrap();
        assert!(validate_destination(&f.path().join("RayLink")).is_err());
        assert!(validate_destination(&f.path().join("..\\outside")).is_err());
    }
    #[test]
    fn lifecycle_marker_is_required_and_must_belong_to_this_home() {
        let f = fixture();
        assert!(!is_installed(f.path()));
        fs::write(f.path().join(MARKER), b"{}").unwrap();
        assert!(!is_installed(f.path()));
        assert_eq!(installed_language(f.path()), None);
    }
    #[test]
    fn lifecycle_update_rejects_missing_hash_and_untrusted_urls() {
        assert!(validate_update(&update()).is_ok());
        let mut u = update();
        u.release.sha256 = None;
        assert!(validate_update(&u).is_err());
        u = update();
        u.release.sha256 = Some("z".repeat(64));
        assert!(validate_update(&u).is_err());
        u = update();
        u.release.asset_url = "https://attacker.example/RayLinkSetup.exe".into();
        assert!(validate_update(&u).is_err());
        u = update();
        u.release.asset_name = "RayLinkLauncher.exe".into();
        assert!(validate_update(&u).is_err());
        u = update();
        u.release.tag = "b24".into();
        assert!(validate_update(&u).is_err());
    }
    #[test]
    fn lifecycle_test_install_preserves_settings_builds_and_uninstalls_only_owned_files() {
        let f = fixture();
        let home = f.path().join("RayLink");
        let source = f.path().join("RayLinkSetup.exe");
        fs::write(&source, b"MZfixture-one").unwrap();
        install_from(&source, &home, "ru", true, true, &|_| {}).unwrap();
        assert!(is_installed(&home));
        assert_eq!(installed_language(&home).as_deref(), Some("ru"));
        assert_eq!(fs::read(home.join(LAUNCHER)).unwrap(), b"MZfixture-one");
        assert_eq!(fs::read(home.join(UNINSTALLER)).unwrap(), b"MZfixture-one");
        fs::write(home.join("settings.json"), b"{\"nick\":\"stalker\"}").unwrap();
        fs::write(home.join("builds/sentinel"), b"keep").unwrap();
        fs::create_dir(home.join("stand")).unwrap();
        fs::write(home.join("stand/save"), b"keep").unwrap();
        fs::write(&source, b"MZfixture-two").unwrap();
        install_from(&source, &home, "en", false, true, &|_| {}).unwrap();
        assert_eq!(fs::read(home.join(LAUNCHER)).unwrap(), b"MZfixture-two");
        assert_eq!(installed_language(&home).as_deref(), Some("en"));
        uninstall_from(&home).unwrap();
        assert!(!home.join(LAUNCHER).exists());
        assert!(!home.join(MARKER).exists());
        assert!(home.join("settings.json").exists());
        assert!(home.join("builds/sentinel").exists());
        assert!(home.join("stand/save").exists());
        assert!(home.join("cache").exists());
    }
    #[test]
    fn lifecycle_refuses_to_overwrite_unowned_directory_and_bad_language() {
        let f = fixture();
        let source = f.path().join("setup");
        fs::write(&source, b"MZfixture").unwrap();
        let home = f.path().join("occupied");
        fs::create_dir(&home).unwrap();
        fs::write(home.join(LAUNCHER), b"not ours").unwrap();
        assert!(install_from(&source, &home, "en", false, true, &|_| {}).is_err());
        assert_eq!(fs::read(home.join(LAUNCHER)).unwrap(), b"not ours");
        assert!(install_from(&source, &f.path().join("new"), "xx", false, true, &|_| {}).is_err());
    }
    #[test]
    fn lifecycle_update_helper_is_hash_bound_and_preserved_after_apply() {
        let f = fixture();
        let home = f.path().join("RayLink");
        let source = f.path().join("setup.exe");
        fs::write(&source, b"MZold").unwrap();
        install_from(&source, &home, "ru", false, true, &|_| {}).unwrap();
        fs::write(home.join("cache/keep"), b"keep").unwrap();
        let dir = new_work_dir(&home.join("cache"), "launcher-update").unwrap();
        let helper = dir.join("RayLinkSetup.exe");
        fs::write(&helper, b"MZfixture-update").unwrap();
        let pending = PendingUpdate {
            schema: 1,
            home: home.clone(),
            version: "0.3.0".into(),
            sha256: "2c14c92ad35062c388e56996078240e6646c3d8d23ddd192eb5eadd04d1bf3a0".into(),
        };
        write_json(&dir.join(PENDING), &pending).unwrap();
        let trusted = validate_pending(&helper, &home).unwrap();
        fs::write(&helper, b"corrupt").unwrap();
        assert!(validate_pending(&helper, &home).is_err());
        assert_eq!(fs::read(home.join(LAUNCHER)).unwrap(), b"MZold");
        fs::write(&helper, b"MZfixture-update").unwrap();
        apply_payload(&helper, &home, &trusted).unwrap();
        assert_eq!(fs::read(home.join(LAUNCHER)).unwrap(), b"MZfixture-update");
        assert_eq!(
            fs::read(home.join(UNINSTALLER)).unwrap(),
            b"MZfixture-update"
        );
        assert_eq!(load_marker(&home).unwrap().version, "0.3.0");
        assert_eq!(installed_language(&home).as_deref(), Some("ru"));
        assert!(helper.is_file());
        assert!(home.join("cache/keep").is_file());
        assert!(validate_pending(&helper, &home).is_err()); // Cannot replay a completed update.
    }
    #[test]
    fn lifecycle_marker_cannot_be_reused_for_a_different_home() {
        let f = fixture();
        let home = f.path().join("RayLink");
        let source = f.path().join("setup");
        fs::write(&source, b"MZfixture").unwrap();
        install_from(&source, &home, "", false, true, &|_| {}).unwrap();
        assert_eq!(installed_language(&home).as_deref(), Some("en"));
        let other = f.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::copy(home.join(MARKER), other.join(MARKER)).unwrap();
        assert!(!is_installed(&other));
        assert!(uninstall_from(&other).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn lifecycle_verified_payload_stays_immutable_until_its_guard_closes() {
        let f = fixture();
        let source = f.path().join("RayLinkSetup.exe");
        fs::write(&source, b"MZfixture-update").unwrap();
        let bound = verify_hash(
            &source,
            "2c14c92ad35062c388e56996078240e6646c3d8d23ddd192eb5eadd04d1bf3a0",
        )
        .unwrap();
        assert!(
            fs::write(&source, b"corrupt").is_err(),
            "Verified bytes must stay immutable"
        );
        assert!(
            fs::rename(&source, f.path().join("swapped.exe")).is_err(),
            "The verified helper path must not be replaceable"
        );
        drop(bound);
        fs::write(&source, b"corrupt").unwrap();
        assert!(
            verify_hash(
                &source,
                "2c14c92ad35062c388e56996078240e6646c3d8d23ddd192eb5eadd04d1bf3a0"
            )
            .is_err()
        );
    }
    #[cfg(windows)]
    #[test]
    fn lifecycle_uninstall_delete_lock_preserves_every_owned_file() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        let f = fixture();
        let home = f.path().join("RayLink");
        let source = f.path().join("setup");
        fs::write(&source, b"MZold").unwrap();
        install_from(&source, &home, "en", false, true, &|_| {}).unwrap();
        let marker = fs::read(home.join(MARKER)).unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(home.join(UNINSTALLER))
            .unwrap();
        assert!(uninstall_from(&home).is_err());
        assert_eq!(fs::read(home.join(LAUNCHER)).unwrap(), b"MZold");
        assert_eq!(fs::read(home.join(UNINSTALLER)).unwrap(), b"MZold");
        assert_eq!(fs::read(home.join(MARKER)).unwrap(), marker);
        drop(lock);
        uninstall_from(&home).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn lifecycle_locked_second_executable_rolls_back_the_first_and_marker() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = fixture();
        let home = f.path().join("RayLink");
        let source = f.path().join("setup");
        fs::write(&source, b"MZold").unwrap();
        install_from(&source, &home, "en", false, true, &|_| {}).unwrap();
        let original_marker = fs::read(home.join(MARKER)).unwrap();
        fs::write(&source, b"MZnew").unwrap();
        let _lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(home.join(UNINSTALLER))
            .unwrap();
        assert!(install_from(&source, &home, "ru", false, true, &|_| {}).is_err());
        assert_eq!(fs::read(home.join(LAUNCHER)).unwrap(), b"MZold");
        assert_eq!(fs::read(home.join(MARKER)).unwrap(), original_marker);
    }
    #[test]
    fn lifecycle_concurrent_updates_are_exclusive() {
        use std::sync::{Arc, Barrier};
        let f = fixture();
        let home = f.path().join("RayLink");
        let source = f.path().join("setup");
        fs::write(&source, b"MZoriginal").unwrap();
        install_from(&source, &home, "en", false, true, &|_| {}).unwrap();
        let entered = Arc::new(Barrier::new(2));
        let resume = Arc::new(Barrier::new(2));
        let worker = {
            let home = home.clone();
            let source = source.clone();
            let entered = entered.clone();
            let resume = resume.clone();
            std::thread::spawn(move || {
                install_from(&source, &home, "en", false, true, &move |p| {
                    if p.phase == "installCopy" {
                        entered.wait();
                        resume.wait();
                    }
                })
            })
        };
        entered.wait();
        let second = install_from(&source, &home, "ru", false, true, &|_| {});
        resume.wait();
        worker.join().unwrap().unwrap();
        assert!(
            second.is_err(),
            "Two installers must not mutate the same home concurrently"
        );
        assert_eq!(installed_language(&home).as_deref(), Some("en"));
    }
    #[test]
    fn lifecycle_cancelled_update_does_not_stage_or_start_a_helper() {
        let f = fixture();
        let home = f.path().join("RayLink");
        let source = f.path().join("setup");
        fs::write(&source, b"MZfixture").unwrap();
        install_from(&source, &home, "en", false, true, &|_| {}).unwrap();
        let result = begin_update(&update(), &home, &AtomicBool::new(true), &|_| {});
        assert_eq!(result.unwrap_err(), "Operation cancelled");
        assert_eq!(fs::read_dir(home.join("cache")).unwrap().count(), 0);
    }
    #[cfg(windows)]
    #[test]
    fn lifecycle_lock_crash_child() {
        // Invoked in a dedicated test process; no environment changes in the parent.
        let Some(home) = std::env::var_os("RAYLINK_LIFECYCLE_LOCK_FIXTURE") else {
            return;
        };
        let home = PathBuf::from(home);
        let _lock = LifecycleLock::acquire(&home).unwrap();
        fs::write(home.join("ready"), b"locked").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(windows)]
    #[test]
    fn lifecycle_lock_is_released_after_process_crash() {
        use std::{process::Stdio, time::Instant};
        let f = fixture();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "lifecycle::tests::lifecycle_lock_crash_child",
                "--nocapture",
            ])
            .env("RAYLINK_LIFECYCLE_LOCK_FIXTURE", f.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !f.path().join("ready").exists() && Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let ready = f.path().join("ready").exists();
        let exclusive = ready && LifecycleLock::acquire(f.path()).is_err();
        let _ = child.kill();
        child.wait().unwrap();
        assert!(ready, "Child process did not acquire the lifecycle lock");
        assert!(exclusive, "The live child did not hold an exclusive lock");
        assert!(
            !f.path().join(LOCK).exists(),
            "A crashed process left its lock behind"
        );
        let recovered = LifecycleLock::acquire(f.path()).unwrap();
        drop(recovered);
        // Also recover a stale file left by older versions using create_new.
        fs::write(f.path().join(LOCK), b"stale").unwrap();
        let recovered = LifecycleLock::acquire(f.path()).unwrap();
        drop(recovered);
        assert!(!f.path().join(LOCK).exists());
    }
    #[test]
    fn lifecycle_os_failures_use_english_diagnostics_on_localized_windows() {
        let f = fixture();
        let error = install_from(
            &f.path().join("missing.exe"),
            &f.path().join("RayLink"),
            "en",
            false,
            true,
            &|_| {},
        )
        .unwrap_err();
        assert!(
            error.is_ascii(),
            "OS-localized text leaked into the backend diagnostic: {error}"
        );
    }
    #[test]
    fn lifecycle_replacement_never_removes_the_visible_launcher_path() {
        use std::sync::{Arc, Barrier, atomic::Ordering};
        let f = fixture();
        let home = f.path().join("RayLink");
        let source = f.path().join("setup.exe");
        fs::write(&source, b"MZoriginal").unwrap();
        install_from(&source, &home, "en", false, true, &|_| {}).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(Barrier::new(2));
        let reader = {
            let launcher = home.join(LAUNCHER);
            let done = done.clone();
            let ready = ready.clone();
            std::thread::spawn(move || {
                let mut missing = false;
                ready.wait();
                while !done.load(Ordering::Relaxed) {
                    if fs::metadata(&launcher).is_err() {
                        missing = true;
                    }
                }
                missing
            })
        };
        ready.wait();
        for _ in 0..30 {
            install_from(&source, &home, "en", false, true, &|_| {}).unwrap();
        }
        done.store(true, Ordering::Relaxed);
        assert!(
            !reader.join().unwrap(),
            "Replacing the launcher exposed a missing target path"
        );
    }
    #[cfg(windows)]
    #[test]
    fn lifecycle_refuses_junctions_in_destination_and_managed_files() {
        use std::os::windows::fs::symlink_dir;
        let f = fixture();
        let target = f.path().join("target");
        fs::create_dir(&target).unwrap();
        let link = f.path().join("linked");
        if let Err(e) = symlink_dir(&target, &link) {
            assert_eq!(e.raw_os_error(), Some(1314));
            return;
        }
        assert!(validate_destination(&link.join("RayLink")).is_err());
    }
}
