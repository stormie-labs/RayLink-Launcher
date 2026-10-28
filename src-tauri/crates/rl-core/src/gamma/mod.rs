//! Native Anomaly and Mod Organizer discovery.
use std::fs;
use std::path::{Path, PathBuf};
pub fn validate_anomaly(root: &Path) -> Result<(), String> {
    rl_fs::guard(root)?;
    for dir in ["db", "gamedata", "bin"] {
        let path = rl_fs::join(root, dir)?;
        if !path.is_dir() {
            return Err(format!("Anomaly directory is missing: {}", path.display()));
        }
    }
    let fs = rl_fs::join(root, "fsgame.ltx")?;
    if !fs.is_file() {
        return Err(format!("Anomaly fsgame.ltx is missing: {}", fs.display()));
    }
    Ok(())
}
pub fn validate_mo2(root: &Path) -> Result<(), String> {
    let ini = rl_fs::join(root, "ModOrganizer.ini")?;
    if !ini.is_file() {
        return Err(format!("ModOrganizer.ini is missing: {}", ini.display()));
    }
    if profiles(root)?.is_empty() {
        return Err("MO2 has no profiles with modlist.txt".into());
    }
    Ok(())
}
pub fn profiles(root: &Path) -> Result<Vec<String>, String> {
    let folder = folder(root, "profiles_directory", "profiles")?;
    if !folder.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for dir in fs::read_dir(&folder)
        .map_err(|e| format!("Cannot read MO2 profiles {}: {e}", folder.display()))?
    {
        let dir = dir.map_err(|e| e.to_string())?;
        rl_fs::guard(&dir.path())?;
        let name = dir
            .file_name()
            .into_string()
            .map_err(|_| "Profile name is not valid Unicode".to_string())?;
        rl_fs::validate_relative(&name)?;
        if dir.path().is_dir() && rl_fs::join(&dir.path(), "modlist.txt")?.is_file() {
            names.push(name);
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    Ok(names)
}
pub fn version(root: &Path) -> String {
    rl_fs::join(
        root,
        ".Grok's Modpack Installer/G.A.M.M.A_definition_version.txt",
    )
    .and_then(|p| read_text(&p))
    .map(|s| s.trim().to_string())
    .unwrap_or_default()
}
pub fn suggested_anomaly(root: &Path) -> Option<PathBuf> {
    let value = ini(root, "gamePath").ok().flatten()?;
    resolve(root, &value).ok()
}
pub fn selected_profile(root: &Path) -> Option<String> {
    let value = ini(root, "selected_profile").ok().flatten()?;
    if value.contains(['/', '\\']) || rl_fs::validate_relative(&value).is_err() {
        return None;
    }
    Some(value)
}
pub(crate) fn read_text(path: &Path) -> Result<String, String> {
    rl_fs::guard(path)?;
    let bytes = fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    match String::from_utf8(bytes) {
        Ok(text) => Ok(text.trim_start_matches('\u{feff}').to_string()),
        Err(e) => Ok(encoding_rs::WINDOWS_1251
            .decode(e.as_bytes())
            .0
            .into_owned()),
    }
}
pub(crate) fn ini(root: &Path, key: &str) -> Result<Option<String>, String> {
    let text = read_text(&rl_fs::join(root, "ModOrganizer.ini")?)?;
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case(key)
        {
            let value = v.trim();
            let value = value
                .strip_prefix("@ByteArray(")
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or(value);
            return Ok((!value.is_empty()).then(|| value.replace("\\\\", "\\")));
        }
    }
    Ok(None)
}
fn resolve(root: &Path, value: &str) -> Result<PathBuf, String> {
    let value = value.replace("%BASE_DIR%", &root.to_string_lossy());
    #[cfg(windows)]
    let value = value.replace('/', "\\");
    #[cfg(not(windows))]
    let value = value.replace('\\', "/");
    let p = Path::new(&value);
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };
    rl_fs::guard(&p)?;
    Ok(p)
}
pub(crate) fn folder(root: &Path, key: &str, default: &str) -> Result<PathBuf, String> {
    resolve(
        root,
        &ini(root, key)?.unwrap_or_else(|| default.to_string()),
    )
}
pub(crate) fn enabled_mods(root: &Path, profile: &str) -> Result<(PathBuf, Vec<String>), String> {
    rl_fs::validate_relative(profile)?;
    if profile.contains('/') {
        return Err("Profile must be a single directory name".into());
    }
    let profiles = folder(root, "profiles_directory", "profiles")?;
    let list = rl_fs::join(&profiles, &format!("{profile}/modlist.txt"))?;
    let text = read_text(&list)?;
    let mut mods = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.trim_start_matches('\u{feff}').strip_prefix('+') {
            let name = name.trim();
            if name.is_empty() || name.to_lowercase().ends_with("_separator") {
                continue;
            }
            rl_fs::validate_relative(name)?;
            if name.contains('/') {
                return Err(format!(
                    "Enabled mod must be a single directory name: {name}"
                ));
            }
            mods.push(name.to_string());
        }
    }
    mods.reverse();
    Ok((list, mods))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gamma_reads_custom_profiles_and_bytearray_paths() {
        let t = tempfile::tempdir_in(
            fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp")).unwrap(),
        )
        .unwrap();
        let mo2 = t.path();
        std::fs::create_dir_all(mo2.join("custom profiles/English")).unwrap();
        std::fs::write(
            mo2.join("custom profiles/English/modlist.txt"),
            "+Example\n",
        )
        .unwrap();
        std::fs::write(mo2.join("ModOrganizer.ini"), "profiles_directory=%BASE_DIR%/custom profiles\nselected_profile=@ByteArray(English)\ngamePath=@ByteArray(Anomaly)\n").unwrap();
        assert_eq!(profiles(mo2).unwrap(), vec!["English"]);
        assert_eq!(selected_profile(mo2).as_deref(), Some("English"));
        assert_eq!(suggested_anomaly(mo2), Some(mo2.join("Anomaly")));
        assert!(validate_mo2(mo2).is_ok());
        assert!(validate_anomaly(mo2).is_err());
    }
}
