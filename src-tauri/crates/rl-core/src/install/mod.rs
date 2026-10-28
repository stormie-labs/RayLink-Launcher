//! Transactional installation into immutable build directories.
use crate::{
    job::{Reporter, check_cancel, report},
    manifest, relpath,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

const ENGINE: &str = "Anomaly Coop - Engine/bin/AnomalyCoopDX11AVX.exe";
const CORE: &str = "Anomaly Coop - Core/gamedata";

#[derive(Clone, Copy)]
struct Limits {
    entries: usize,
    bytes: u64,
    file_bytes: u64,
}
const LIMITS: Limits = Limits {
    entries: manifest::MAX_ENTRIES,
    bytes: manifest::MAX_BYTES,
    file_bytes: manifest::MAX_FILE_BYTES,
};

pub fn install_archive(
    archive: &Path,
    builds_dir: &Path,
    tag: &str,
    cancel: &AtomicBool,
    progress: &Reporter,
) -> Result<PathBuf, String> {
    install_limited(archive, builds_dir, tag, cancel, progress, LIMITS)
}

fn install_limited(
    archive: &Path,
    builds_dir: &Path,
    tag: &str,
    cancel: &AtomicBool,
    progress: &Reporter,
    limits: Limits,
) -> Result<PathBuf, String> {
    check_cancel(cancel)?;
    relpath::validate_tag(tag)?;
    relpath::no_reparse_ancestors(builds_dir)?;
    let target = builds_dir.join(tag);
    relpath::no_reparse_ancestors(archive)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)] {
        use std::os::windows::fs::OpenOptionsExt;
        // The receipt and extracted bytes must refer to the same immutable file.
        options.share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ);
    }
    let mut file = options.open(archive).map_err(|_| "Cannot open protected build archive")?;
    let archive_hash = hash_archive(&mut file, cancel)?;
    use std::io::Seek;
    file.rewind().map_err(|_| "Cannot rewind build archive")?;
    let mut existing_legacy = false;
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            verify_with_cancel(&target, cancel)?;
            match manifest::archive_hash(&target)? {
                Some(hash) if hash == archive_hash => return Ok(target),
                Some(_) => return Err("This build tag already belongs to a different archive".into()),
                None => existing_legacy = true,
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Cannot inspect installed build".into()),
    }
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "Invalid or corrupt ZIP archive")?;
    if zip.is_empty() || zip.len() > limits.entries {
        return Err("ZIP entry count exceeds limit".into());
    }
    let mut paths = Vec::with_capacity(zip.len());
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    let mut spellings: BTreeMap<String, (String, bool)> = BTreeMap::new();
    // Validate the entire central directory before creating the staging directory.
    for index in 0..zip.len() {
        check_cancel(cancel)?;
        let entry = zip.by_index(index).map_err(|_| "Cannot read ZIP entry")?;
        let raw = std::str::from_utf8(entry.name_raw()).map_err(|_| "ZIP filename is not UTF-8")?;
        let name = if entry.is_dir() {
            raw.strip_suffix('/').ok_or("Invalid ZIP directory")?
        } else {
            raw
        };
        let path = relpath::validate(name)?;
        if name.eq_ignore_ascii_case(manifest::MARKER) || name.starts_with(".raylink-") {
            return Err("ZIP contains a reserved launcher filename".into());
        }
        let mode = entry.unix_mode().unwrap_or(0) & 0o170000;
        if entry.is_symlink() || entry.encrypted() || !matches!(mode, 0 | 0o100000 | 0o040000) {
            return Err("ZIP links, encrypted entries and special files are forbidden".into());
        }
        if !names.insert(name.to_uppercase()) {
            return Err("ZIP contains duplicate paths".into());
        }
        let mut prefix = String::new();
        let components: Vec<_> = name.split('/').collect();
        for (index, component) in components.iter().enumerate() {
            if index > 0 {
                prefix.push('/');
            }
            prefix.push_str(component);
            let directory = index + 1 < components.len() || entry.is_dir();
            let key = prefix.to_uppercase();
            if let Some((prior, was_directory)) = spellings.get(&key) {
                if prior != &prefix || *was_directory != directory {
                    return Err("ZIP contains path aliases or file/directory conflicts".into());
                }
            } else {
                spellings.insert(key, (prefix.clone(), directory));
            }
        }
        if spellings.len() > limits.entries {
            return Err("ZIP implied entry count exceeds limit".into());
        }
        if entry.is_dir() && entry.size() != 0 {
            return Err("ZIP directory contains data".into());
        }
        total = total.checked_add(entry.size()).ok_or("ZIP size overflow")?;
        if entry.size() > limits.file_bytes || total > limits.bytes {
            return Err("Decompressed ZIP size exceeds limit".into());
        }
        paths.push((path, entry.is_dir(), entry.size()));
    }
    fs::create_dir_all(builds_dir).map_err(|_| "Cannot create builds directory")?;
    relpath::no_reparse_ancestors(builds_dir)?;
    let stage = builds_dir.join(format!(".raylink-install-{}", manifest::unique_id()));
    fs::create_dir(&stage).map_err(|_| "Cannot create unique installation staging directory")?;
    let mut published = false;
    let result = (|| {
        let mut written = 0u64;
        report(
            progress,
            "install_extract",
            "Extracting build",
            0,
            total,
            true,
        );
        for (index, (relative, directory, expected)) in paths.iter().enumerate() {
            check_cancel(cancel)?;
            let path = stage.join(relative);
            relpath::no_reparse_ancestors(&path)?;
            if *directory {
                fs::create_dir_all(&path).map_err(|_| "Cannot create package directory")?;
                continue;
            }
            let parent = path.parent().ok_or("Invalid package destination")?;
            fs::create_dir_all(parent).map_err(|_| "Cannot create package directory")?;
            relpath::no_reparse_ancestors(&path)?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|_| "Cannot create package file")?;
            let mut entry = zip.by_index(index).map_err(|_| "Cannot open ZIP entry")?;
            let mut actual = 0u64;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                check_cancel(cancel)?;
                let count = entry
                    .read(&mut buffer)
                    .map_err(|_| "ZIP data or checksum is corrupt")?;
                if count == 0 {
                    break;
                }
                actual += count as u64;
                written += count as u64;
                if actual > *expected || actual > limits.file_bytes || written > limits.bytes {
                    return Err("Decompressed ZIP data exceeds declared size or limit".into());
                }
                output
                    .write_all(&buffer[..count])
                    .map_err(|_| "Cannot write package file")?;
                report(
                    progress,
                    "install_extract",
                    "Extracting build",
                    written,
                    total,
                    true,
                );
            }
            if actual != *expected {
                return Err("ZIP entry is truncated".into());
            }
            output.sync_all().map_err(|_| "Cannot flush package file")?;
        }
        check_cancel(cancel)?;
        structure(&stage)?;
        report(
            progress,
            "install_verify",
            "Verifying installed files",
            written,
            total,
            true,
        );
        manifest::record_archive(&stage, &archive_hash, cancel)?;
        manifest::verify(&stage, cancel)?;
        check_cancel(cancel)?;
        relpath::no_reparse_ancestors(&target)?;
        // No removal or replacement: concurrent successful installation wins.
        if existing_legacy {
            verify_with_cancel(&target, cancel)?;
            if !manifest::same_files(&stage, &target, cancel)? { return Err("Existing legacy build differs from the downloaded archive".into()); }
            return Ok(target);
        }
        if fs::symlink_metadata(&target).is_ok() {
            return Err("Build tag was installed concurrently".into());
        }
        publish(&stage, &target, cancel)?;
        published = true;
        report(
            progress,
            "install_complete",
            "Build installed",
            total,
            total,
            false,
        );
        Ok(target)
    })();
    if !published {
        cleanup_owned_stage(&stage);
    }
    result
}

fn hash_archive(file: &mut fs::File, cancel: &AtomicBool) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop { check_cancel(cancel)?; let count = file.read(&mut buffer).map_err(|_| "Cannot read build archive")?; if count == 0 { break; } hash.update(&buffer[..count]); }
    Ok(manifest::hex(&hash.finalize()))
}

fn publish(stage: &Path, target: &Path, cancel: &AtomicBool) -> Result<(), String> {
    for attempt in 0..150 {
        check_cancel(cancel)?;
        relpath::no_reparse_ancestors(target)?;
        if fs::symlink_metadata(target).is_ok() {
            return Err("Build tag was installed concurrently".into());
        }
        match fs::rename(stage, target) {
            Ok(()) => return Ok(()),
            // Antivirus/indexers can briefly hold a newly extracted executable
            // without FILE_SHARE_DELETE. Retry the atomic operation, never copy
            // into the final directory or remove an existing build.
            Err(e) if attempt < 149 && matches!(e.raw_os_error(), Some(5 | 32 | 33)) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                #[cfg(test)]
                eprintln!("publish failed: stage={} exists={}, target={} metadata={:?}", stage.display(), stage.exists(), target.display(), fs::symlink_metadata(target));
                return Err(format!(
                    "Cannot publish installed build (OS error {:?})",
                    e.raw_os_error()
                ));
            }
        }
    }
    Err("Cannot publish installed build".into())
}

/// Cleanup only the exact unique staging directory created by this invocation.
fn cleanup_owned_stage(stage: &Path) {
    #[cfg(test)]
    if std::env::var_os("RAYLINK_DIAG_KEEP_STAGE").is_some() { return; }
    if relpath::no_reparse_ancestors(stage).is_err() {
        return;
    }
    // remove_dir_all does not follow links, but refuse all reparse points anyway.
    let mut pending = vec![stage.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            return;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return;
            };
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                return;
            };
            if relpath::is_reparse(&meta) {
                return;
            }
            if meta.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    let _ = fs::remove_dir_all(stage);
}

fn structure(build: &Path) -> Result<(), String> {
    relpath::no_reparse_ancestors(build)?;
    for (name, directory) in [(ENGINE, false), (CORE, true)] {
        let path = build.join(name);
        relpath::no_reparse_ancestors(&path)?;
        let meta = fs::symlink_metadata(path)
            .map_err(|_| "Build package is missing the engine or Core/gamedata")?;
        if (directory && !meta.is_dir()) || (!directory && (!meta.is_file() || meta.len() == 0)) {
            return Err("Invalid build package structure".into());
        }
    }
    Ok(())
}

/// Import a structurally valid legacy directory as an explicitly labelled local
/// baseline. Subsequent checks compare every installed file against that baseline.
pub fn verify_build(build: &Path) -> Result<(), String> {
    verify_with_cancel(build, &AtomicBool::new(false))
}

fn verify_with_cancel(build: &Path, cancel: &AtomicBool) -> Result<(), String> {
    check_cancel(cancel)?;
    structure(build)?;
    match fs::symlink_metadata(build.join(manifest::MARKER)) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            manifest::record(build, "legacy-baseline", cancel)?
        }
        Err(_) => return Err("Cannot inspect build manifest".into()),
    }
    manifest::verify(build, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::Ordering;
    use zip::write::SimpleFileOptions;

    const EXE: &str = "Anomaly Coop - Engine/bin/AnomalyCoopDX11AVX.exe";
    const SCRIPT: &str = "Anomaly Coop - Core/gamedata/scripts/hello.script";

    fn temp() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|p| p.file_name().is_some_and(|n| n == "raylink-launcher"))
            .unwrap()
            .join("temp");
        fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }

    fn package(path: &Path, extras: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, data) in [
            (EXE, b"engine".as_slice()),
            (SCRIPT, b"original".as_slice()),
        ]
        .into_iter()
        .chain(extras.iter().copied())
        {
            zip.start_file(
                name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn install_hashes_every_file_and_refuses_changed_existing_tag() {
        let t = temp();
        let archive = t.path().join("build.zip");
        package(&archive, &[]);
        let builds = t.path().join("builds");
        let cancel = AtomicBool::new(false);
        let build = install_archive(&archive, &builds, "b26", &cancel, &|_| {}).unwrap();
        verify_build(&build).unwrap();
        assert_eq!(
            install_archive(&archive, &builds, "b26", &cancel, &|_| {}).unwrap(),
            build
        );
        fs::write(build.join(SCRIPT), b"modified").unwrap();
        assert!(verify_build(&build).is_err());
        assert!(install_archive(&archive, &builds, "b26", &cancel, &|_| {}).is_err());
        assert_eq!(fs::read(build.join(SCRIPT)).unwrap(), b"modified");
    }

    #[test]
    fn same_tag_different_archive_is_refused_without_modifying_install() {
        let t = temp();
        let a = t.path().join("a.zip");
        let b = t.path().join("b.zip");
        package(&a, &[]);
        package(&b, &[("different.txt", b"different release")]);
        let builds = t.path().join("builds");
        let cancel = AtomicBool::new(false);
        let installed = install_archive(&a, &builds, "same", &cancel, &|_| {}).unwrap();
        assert!(install_archive(&b, &builds, "same", &cancel, &|_| {}).is_err());
        verify_build(&installed).unwrap();
        assert!(!installed.join("different.txt").exists());
        assert_eq!(install_archive(&a, &builds, "same", &cancel, &|_| {}).unwrap(), installed);
    }

    #[test]
    fn adversarial_zip_never_publishes_or_escapes() {
        for bad in [
            "../escape",
            "/escape",
            "C:/escape",
            "dir\\evil",
            "file:ads",
            "CON.txt",
            "a/../escape",
            "Anomaly Coop - Core/GAMEDATA/scripts/HELLO.script",
            ".raylink-build.json",
            "file.",
            "NUL .txt",
        ] {
            let t = temp();
            let archive = t.path().join("bad.zip");
            package(&archive, &[(bad, b"bad")]);
            let builds = t.path().join("builds");
            assert!(
                install_archive(&archive, &builds, "bad", &AtomicBool::new(false), &|_| {})
                    .is_err(),
                "accepted {bad}"
            );
            assert!(!builds.join("bad").exists());
            assert!(!t.path().join("escape").exists());
        }
    }

    #[test]
    fn cancelled_install_keeps_good_build_and_cleans_owned_stage() {
        let t = temp();
        let archive = t.path().join("build.zip");
        package(&archive, &[]);
        let builds = t.path().join("builds");
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let good = install_archive(&archive, &builds, "good", &cancel, &|_| {}).unwrap();
        let signal = cancel.clone();
        assert!(
            install_archive(&archive, &builds, "next", &cancel, &move |_| {
                signal.store(true, Ordering::Relaxed);
            })
            .is_err()
        );
        assert!(!builds.join("next").exists());
        verify_build(&good).unwrap();
        assert_eq!(fs::read_dir(&builds).unwrap().count(), 1);
    }

    #[test]
    fn cancellation_after_writing_and_before_publication_preserves_previous_build() {
        let t = temp();
        let archive = t.path().join("build.zip");
        package(&archive, &[]);
        let builds = t.path().join("builds");
        let good =
            install_archive(&archive, &builds, "good", &AtomicBool::new(false), &|_| {}).unwrap();
        for phase in ["install_extract", "install_verify"] {
            let cancel = AtomicBool::new(false);
            let result = install_archive(&archive, &builds, "next", &cancel, &|p| {
                if p.phase == phase && p.current > 0 {
                    cancel.store(true, Ordering::Relaxed);
                }
            });
            assert!(result.is_err());
            assert!(cancel.load(Ordering::Relaxed));
            assert!(!builds.join("next").exists());
            verify_build(&good).unwrap();
            assert_eq!(fs::read_dir(&builds).unwrap().count(), 1);
        }
    }

    #[test]
    fn verification_rejects_unexpected_files_and_corrupt_manifest() {
        let t = temp();
        let archive = t.path().join("build.zip");
        package(&archive, &[]);
        let build = install_archive(
            &archive,
            &t.path().join("builds"),
            "good",
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap();
        let extra = build.join("unexpected.dll");
        fs::write(&extra, b"unexpected").unwrap();
        assert!(verify_build(&build).is_err());
        fs::remove_file(extra).unwrap();
        verify_build(&build).unwrap();
        fs::write(build.join(manifest::MARKER), b"{}").unwrap();
        assert!(verify_build(&build).is_err());
    }

    #[test]
    fn rejects_symlink_and_corrupt_archive_without_deleting_previous_build() {
        let t = temp();
        let archive = t.path().join("build.zip");
        package(&archive, &[]);
        let builds = t.path().join("builds");
        let cancel = AtomicBool::new(false);
        let good = install_archive(&archive, &builds, "good", &cancel, &|_| {}).unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.add_symlink("link", "../outside", SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();
        assert!(install_archive(&archive, &builds, "linked", &cancel, &|_| {}).is_err());
        fs::write(&archive, b"broken zip").unwrap();
        assert!(install_archive(&archive, &builds, "broken", &cancel, &|_| {}).is_err());
        verify_build(&good).unwrap();
    }

    #[test]
    fn legacy_import_creates_baseline_and_detects_later_changes() {
        let t = temp();
        let build = t.path().join("legacy");
        fs::create_dir_all(build.join("Anomaly Coop - Engine/bin")).unwrap();
        fs::create_dir_all(build.join("Anomaly Coop - Core/gamedata/scripts")).unwrap();
        fs::write(build.join(EXE), b"legacy engine").unwrap();
        fs::write(build.join(SCRIPT), b"legacy script").unwrap();
        verify_build(&build).unwrap();
        let marker = fs::read_to_string(build.join(".raylink-build.json")).unwrap();
        assert!(marker.contains("legacy-baseline"));
        fs::remove_file(build.join(SCRIPT)).unwrap();
        assert!(verify_build(&build).is_err());
    }

    #[test]
    fn rejects_entry_byte_bombs_and_missing_package_structure() {
        let t = temp();
        let archive = t.path().join("build.zip");
        package(&archive, &[]);
        let builds = t.path().join("builds");
        let cancel = AtomicBool::new(false);
        for limits in [
            Limits {
                entries: 1,
                ..LIMITS
            },
            Limits { bytes: 8, ..LIMITS },
            Limits {
                file_bytes: 5,
                ..LIMITS
            },
        ] {
            assert!(
                install_limited(&archive, &builds, "too-big", &cancel, &|_| {}, limits).is_err()
            );
            assert!(!builds.join("too-big").exists());
        }
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file("readme.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"not a build").unwrap();
        zip.finish().unwrap();
        assert!(install_archive(&archive, &builds, "incomplete", &cancel, &|_| {}).is_err());
        assert_eq!(fs::read_dir(&builds).unwrap().count(), 0);
    }

    #[test]
    fn corrupt_file_crc_and_invalid_tag_are_rejected() {
        let t = temp();
        let archive = t.path().join("build.zip");
        package(&archive, &[]);
        let mut bytes = fs::read(&archive).unwrap();
        let offset = bytes.windows(6).position(|w| w == b"engine").unwrap();
        bytes[offset] ^= 1;
        fs::write(&archive, bytes).unwrap();
        let builds = t.path().join("builds");
        let cancel = AtomicBool::new(false);
        assert!(install_archive(&archive, &builds, "crc", &cancel, &|_| {}).is_err());
        for tag in ["../bad", "C:bad", "CON", "bad.", "v1/bad"] {
            assert!(install_archive(&archive, &builds, tag, &cancel, &|_| {}).is_err());
        }
        assert_eq!(fs::read_dir(builds).unwrap().count(), 0);
    }

    #[test]
    fn case_aliases_in_implicit_directories_are_rejected() {
        let t = temp();
        let archive = t.path().join("build.zip");
        package(
            &archive,
            &[("Anomaly Coop - Core/GAMEDATA/other.script", b"alias")],
        );
        let builds = t.path().join("builds");
        assert!(
            install_archive(&archive, &builds, "alias", &AtomicBool::new(false), &|_| {}).is_err()
        );
        assert!(!builds.join("alias").exists());
    }

    #[cfg(windows)]
    #[test]
    fn junction_destination_is_rejected_without_writing_to_its_target() {
        let t = temp();
        let archive = t.path().join("build.zip");
        package(&archive, &[]);
        let outside = t.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let junction = t.path().join("builds");
        let output = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(
            install_archive(
                &archive,
                &junction,
                "escape",
                &AtomicBool::new(false),
                &|_| {}
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        fs::remove_dir(&junction).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn transient_windows_scanner_lock_does_not_break_atomic_publication() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = temp();
        let archive = t.path().join("build.zip");
        package(&archive, &[]);
        let builds = t.path().join("builds");
        let observed = builds.clone();
        let handle = std::sync::Arc::new(std::sync::Mutex::new(None));
        let worker = handle.clone();
        let result = install_archive(
            &archive,
            &builds,
            "scanner",
            &AtomicBool::new(false),
            &move |p| {
                if p.phase != "install_verify" {
                    return;
                }
                let stage = fs::read_dir(&observed)
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                let locked = fs::OpenOptions::new()
                    .read(true)
                    .share_mode(3)
                    .open(stage.join(EXE))
                    .unwrap();
                *worker.lock().unwrap() = Some(std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    drop(locked);
                }));
            },
        );
        handle.lock().unwrap().take().unwrap().join().unwrap();
        verify_build(&result.unwrap()).unwrap();
    }
}
