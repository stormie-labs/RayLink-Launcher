//! Portable Windows-safe package paths.
use std::path::{Path, PathBuf};

pub fn validate(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.len() > 2048 || name.contains('\\') {
        return Err("Invalid package path".into());
    }
    let parts: Vec<_> = name.split('/').collect();
    if parts.len() > 32 {
        return Err("Package path is too deep".into());
    }
    for part in parts {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.encode_utf16().count() > 240
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "<>:\"|?*".contains(c))
        {
            return Err("Unsafe Windows package path".into());
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end_matches(' ')
            .to_uppercase();
        let device = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
        if device {
            return Err("Windows device names are forbidden".into());
        }
    }
    Ok(PathBuf::from(name))
}

pub fn validate_tag(tag: &str) -> Result<(), String> {
    if tag.is_empty()
        || tag.len() > 128
        || !tag.as_bytes()[0].is_ascii_alphanumeric()
        || !tag
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        return Err("Invalid release tag".into());
    }
    validate(tag).map(|_| ())
}

/// Check the path itself and every existing ancestor, including Windows junctions.
/// Call again immediately before filesystem mutations. This is not a sandbox for
/// a hostile process that has concurrent write access to the launcher home.
pub fn no_reparse_ancestors(path: &Path) -> Result<(), String> {
    let absolute = std::path::absolute(path).map_err(|_| "Cannot resolve destination path")?;
    for ancestor in absolute.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) if is_reparse(&meta) => {
                return Err("Symbolic links and reparse points are forbidden".into());
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect destination ancestors".into()),
        }
    }
    Ok(())
}

pub fn is_reparse(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0 || meta.file_type().is_symlink()
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_windows_aliases_traversal_and_device_names() {
        for bad in [
            "../escape",
            "/absolute",
            "C:/escape",
            "a\\b",
            "a//b",
            "a/./b",
            "file:stream",
            "CON",
            "con.txt",
            "NUL .txt",
            "COM1.exe",
            "LPT9",
            "COM¹.log",
            "trailing.",
            "trailing ",
            "a/<b",
            "a\0b",
            "",
            "a/../b",
        ] {
            assert!(validate(bad).is_err(), "accepted {bad:?}");
        }
        assert_eq!(
            validate("Anomaly Coop - Core/gamedata/тест.script").unwrap(),
            std::path::PathBuf::from("Anomaly Coop - Core/gamedata/тест.script")
        );
    }
}
