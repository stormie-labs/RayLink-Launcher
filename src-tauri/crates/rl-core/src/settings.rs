use crate::dto::Settings;
use std::{fs, path::Path};
pub fn parse_legacy(text: &str) -> Settings {
    let mut settings = Settings::default();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "game" => settings.game = if value == "anomaly" { "anomaly" } else { "gamma" }.into(),
            "anomaly" => settings.anomaly_dir = value.into(), "mo2" => settings.mo2_dir = value.into(),
            "profile" => settings.profile = value.into(), "stand" => settings.stand_root = value.into(),
            "nick" => settings.nick = clean_nick(value), "installed" => settings.installed = value.into(),
            "repo" if value != "stormie-labs/RayLink-Engine" && valid_repo(value) => settings.repo = value.into(),
            _ => {}, // Deliberately exclude credentials, address, join and second-window preferences.
        }
    }
    settings
}
pub fn clean_nick(value: &str) -> String {
    let result: String = value.chars().filter(|c| c.is_ascii_alphanumeric() || "_.-".contains(*c)).take(24).collect();
    if result.len() < 2 { "stalker".into() } else { result }
}
fn valid_repo(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    parts.len() == 2 && parts.iter().all(|part| !part.is_empty() && *part != "." && *part != ".." && part.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)))
}
pub fn validate(settings: &Settings) -> Result<(), String> {
    if settings.game != "anomaly" && settings.game != "gamma" { return Err("Unsupported game selection".into()); }
    if settings.language != "en" && settings.language != "ru" { return Err("Unsupported language".into()); }
    if !valid_repo(&settings.repo) { return Err("Use a GitHub owner/repository, not a URL".into()); }
    if settings.profile.contains(['/', '\\', '\r', '\n', ':']) || settings.profile == "." || settings.profile == ".." { return Err("Invalid MO2 profile name".into()); }
    if !settings.installed.is_empty() && (!settings.installed.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)) || settings.installed == "." || settings.installed == "..") { return Err("Invalid installed build tag".into()); }
    for path in [&settings.anomaly_dir, &settings.mo2_dir, &settings.stand_root] {
        if !path.is_empty() && (!Path::new(path).is_absolute() || path.contains(['\r', '\n', '\0'])) { return Err("Choose an absolute local folder path".into()); }
    }
    Ok(())
}
pub fn check_plain_ancestors(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                #[cfg(windows)] let reparse = { use std::os::windows::fs::MetadataExt; meta.file_attributes() & 0x400 != 0 };
                #[cfg(not(windows))] let reparse = false;
                if meta.file_type().is_symlink() || reparse { return Err(format!("Refusing to write through a filesystem link: {}", ancestor.display())); }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {},
            Err(err) => return Err(err.to_string()),
        }
    }
    Ok(())
}
pub fn load(home: &Path) -> Result<Settings, String> {
    let path = home.join("settings.json");
    check_plain_ancestors(&path)?;
    let settings = if path.exists() {
        serde_json::from_slice(&fs::read(&path).map_err(|err| err.to_string())?).map_err(|err| format!("Cannot read RayLink settings: {err}"))?
    } else {
        let legacy = std::env::var_os("APPDATA").map(|root| std::path::PathBuf::from(root).join("RayLink/settings.txt"));
        let mut settings = match legacy.filter(|file| file.is_file()) {
            Some(file) => parse_legacy(&fs::read_to_string(file).map_err(|err| err.to_string())?),
            None => Settings::default(),
        };
        if let Some(language) = crate::lifecycle::installed_language(home) { settings.language = language; }
        if !settings.installed.is_empty() && !home.join("builds").join(&settings.installed).is_dir() { settings.installed.clear(); }
        settings
    };
    validate(&settings)?;
    Ok(settings)
}
pub fn save(home: &Path, settings: &Settings) -> Result<(), String> {
    validate(settings)?;
    let path = home.join("settings.json");
    let temporary = home.join("settings.json.new");
    check_plain_ancestors(&path)?; check_plain_ancestors(&temporary)?;
    fs::create_dir_all(home).map_err(|err| err.to_string())?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|err| err.to_string())?;
    use std::io::Write;
    let mut file = fs::File::create(&temporary).map_err(|err| err.to_string())?;
    file.write_all(&bytes).and_then(|()| file.sync_all()).map_err(|err| err.to_string())?;
    drop(file);
    fs::rename(temporary, path).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn legacy_migration_keeps_choices_but_not_credentials() {
        let settings = parse_legacy("\u{feff}game=anomaly\nanomaly=D:\\Anomaly\nmo2=D:\\GAMMA\nprofile=G.A.M.M.A\nstand=D:\\RayLink\nnick=Alice\nrepo=stormie-labs/RayLink-Engine\ninstalled=b24\ntoken=do-not-import\njoin=1\n");
        assert_eq!(settings.game, "anomaly"); assert_eq!(settings.anomaly_dir, "D:\\Anomaly");
        assert_eq!(settings.stand_root, "D:\\RayLink"); assert_eq!(settings.nick, "Alice");
        assert_eq!(settings.repo, "stormie-labs/RayLink-Coop"); assert_eq!(settings.language, "en");
        assert!(!serde_json::to_string(&settings).unwrap().contains("do-not-import"));
    }
    #[test] fn nick_is_a_bounded_argument() { assert_eq!(clean_nick("a b;с"), "ab"); assert_eq!(clean_nick("x"), "stalker"); assert_eq!(clean_nick(&"a".repeat(50)).len(), 24); }
    #[test] fn rejects_unsupported_game_locale_and_channel() {
        let mut settings = Settings { game: "../../game".into(), ..Settings::default() };
        assert!(validate(&settings).is_err()); settings.game = "gamma".into(); settings.language = "fr".into();
        assert!(validate(&settings).is_err()); settings.language = "en".into(); settings.repo = "../evil".into(); assert!(validate(&settings).is_err());
    }
    #[test] fn preferences_round_trip_inside_selected_home() {
        let dir = tempfile::tempdir().unwrap(); let settings = Settings { nick: "Alice".into(), language: "ru".into(), ..Settings::default() };
        save(dir.path(), &settings).unwrap(); assert_eq!(load(dir.path()).unwrap(), settings);
    }
}
