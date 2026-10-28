//! Local integrity inventory. A legacy baseline records current bytes; it does
//! not authenticate their publisher. Local hashes are not a signature scheme.
use crate::{job::check_cancel, relpath};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

pub const MARKER: &str = ".raylink-build.json";
pub(crate) const MAX_ENTRIES: usize = 100_000;
pub(crate) const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub(crate) const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct FileHash {
    size: u64,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    version: u32,
    origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archive_sha256: Option<String>,
    files: BTreeMap<String, FileHash>,
}

pub(crate) fn unique_id() -> String {
    // create_new/create_dir are still the authority for uniqueness across runs.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |t| t.as_nanos());
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn inventory(root: &Path, cancel: &AtomicBool) -> Result<BTreeMap<String, FileHash>, String> {
    relpath::no_reparse_ancestors(root)?;
    let mut stack = vec![(root.to_path_buf(), String::new())];
    let mut files = BTreeMap::new();
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    while let Some((dir, prefix)) = stack.pop() {
        check_cancel(cancel)?;
        relpath::no_reparse_ancestors(&dir)?;
        for entry in fs::read_dir(&dir).map_err(|_| "Cannot read build directory")? {
            check_cancel(cancel)?;
            let entry = entry.map_err(|_| "Cannot read build entry")?;
            let leaf = entry
                .file_name()
                .into_string()
                .map_err(|_| "Build filename is not valid Unicode")?;
            let name = if prefix.is_empty() {
                leaf
            } else {
                format!("{prefix}/{leaf}")
            };
            relpath::validate(&name)?;
            let meta =
                fs::symlink_metadata(entry.path()).map_err(|_| "Cannot inspect build entry")?;
            if relpath::is_reparse(&meta) {
                return Err("Build contains a reparse point".into());
            }
            if name == MARKER && meta.is_file() {
                continue;
            }
            if name.eq_ignore_ascii_case(MARKER) {
                return Err("Reserved build manifest name".into());
            }
            if !names.insert(name.to_uppercase()) {
                return Err("Case-insensitive build path collision".into());
            }
            if names.len() > MAX_ENTRIES {
                return Err("Build has too many entries".into());
            }
            if meta.is_dir() {
                stack.push((entry.path(), name));
            } else if meta.is_file() {
                if meta.len() > MAX_FILE_BYTES {
                    return Err("Build file exceeds size limit".into());
                }
                total = total.checked_add(meta.len()).ok_or("Build size overflow")?;
                if total > MAX_BYTES {
                    return Err("Build exceeds size limit".into());
                }
                let mut file =
                    fs::File::open(entry.path()).map_err(|_| "Cannot open build file")?;
                let mut hash = Sha256::new();
                let mut size = 0u64;
                let mut buffer = [0u8; 64 * 1024];
                loop {
                    check_cancel(cancel)?;
                    let count = file
                        .read(&mut buffer)
                        .map_err(|_| "Cannot hash build file")?;
                    if count == 0 {
                        break;
                    }
                    size += count as u64;
                    if size > meta.len() {
                        return Err("Build changed while being verified".into());
                    }
                    hash.update(&buffer[..count]);
                }
                if size != meta.len() {
                    return Err("Build changed while being verified".into());
                }
                files.insert(
                    name,
                    FileHash {
                        size,
                        sha256: hex(&hash.finalize()),
                    },
                );
            } else {
                return Err("Build contains a special file".into());
            }
        }
    }
    Ok(files)
}

/// Create a marker only if absent. Never replace an existing inventory.
pub(crate) fn record(root: &Path, origin: &str, cancel: &AtomicBool) -> Result<(), String> {
    record_bound(root, origin, None, cancel)
}
pub(crate) fn record_archive(root: &Path, hash: &str, cancel: &AtomicBool) -> Result<(), String> {
    record_bound(root, "archive", Some(hash.into()), cancel)
}
fn record_bound(root: &Path, origin: &str, archive_sha256: Option<String>, cancel: &AtomicBool) -> Result<(), String> {
    let data = Inventory {
        version: 1,
        origin: origin.into(),
        archive_sha256,
        files: inventory(root, cancel)?,
    };
    let bytes = serde_json::to_vec(&data).map_err(|_| "Cannot encode build manifest")?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("Build manifest is too large".into());
    }
    let part = root.join(format!(".raylink-manifest-{}.part", unique_id()));
    let mut owned = false;
    let result = (|| {
        relpath::no_reparse_ancestors(root)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .map_err(|_| "Cannot create build manifest")?;
        owned = true;
        file.write_all(&bytes)
            .map_err(|_| "Cannot write build manifest")?;
        file.sync_all().map_err(|_| "Cannot flush build manifest")?;
        drop(file);
        check_cancel(cancel)?;
        let target = root.join(MARKER);
        relpath::no_reparse_ancestors(&target)?;
        // hard_link is an atomic no-clobber publication on both Unix and Windows.
        fs::hard_link(&part, &target).map_err(|_| "Cannot publish build manifest")?;
        Ok(())
    })();
    if owned && relpath::no_reparse_ancestors(&part).is_ok() {
        let _ = fs::remove_file(&part);
    }
    result
}

pub(crate) fn archive_hash(root: &Path) -> Result<Option<String>, String> {
    let marker = root.join(MARKER);
    relpath::no_reparse_ancestors(&marker)?;
    let mut bytes = Vec::new();
    fs::File::open(marker).map_err(|_| "Cannot read build origin receipt")?.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes).map_err(|_| "Cannot read build origin receipt")?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES { return Err("Build receipt is too large".into()); }
    let data: Inventory = serde_json::from_slice(&bytes).map_err(|_| "Invalid build origin receipt")?;
    Ok(data.archive_sha256)
}
pub(crate) fn same_files(a: &Path, b: &Path, cancel: &AtomicBool) -> Result<bool, String> { Ok(inventory(a, cancel)? == inventory(b, cancel)?) }

pub(crate) fn verify(root: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let marker = root.join(MARKER);
    relpath::no_reparse_ancestors(&marker)?;
    let mut bytes = Vec::new();
    fs::File::open(&marker)
        .map_err(|_| "Cannot read build manifest")?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read build manifest")?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("Build manifest is too large".into());
    }
    let data: Inventory = serde_json::from_slice(&bytes).map_err(|_| "Invalid build manifest")?;
    if data.archive_sha256.as_ref().is_some_and(|hash| hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit())) { return Err("Invalid archive digest in build manifest".into()); }
    if data.version != 1
        || !matches!(data.origin.as_str(), "archive" | "legacy-baseline")
        || data.files.is_empty()
        || data.files.len() > MAX_ENTRIES
    {
        return Err("Unsupported build manifest".into());
    }
    for (name, hash) in &data.files {
        relpath::validate(name)?;
        if hash.sha256.len() != 64 || !hash.sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid file hash in build manifest".into());
        }
    }
    if inventory(root, cancel)? != data.files {
        return Err("Build integrity check failed: files changed, missing, or unexpected".into());
    }
    Ok(())
}
