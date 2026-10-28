use crate::dto::Settings;
use std::{fs, path::Path};
pub fn parse_legacy(text: &str) -> Settings {
    let mut settings = Settings::default();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "game" => {
                settings.game = if value == "anomaly" {
                    "anomaly"
                } else {
                    "gamma"
                }
                .into()
            }
            "anomaly" => settings.anomaly_dir = value.into(),
            "mo2" => settings.mo2_dir = value.into(),
            "profile" => settings.profile = value.into(),
            "stand" => settings.stand_root = value.into(),
            "nick" => settings.nick = clean_nick(value),
            "installed" => settings.installed = value.into(),
            "repo" if value != "stormie-labs/RayLink-Engine" && valid_repo(value) => {
                settings.repo = value.into()
            }
            _ => {} // Deliberately exclude credentials, address, join and second-window preferences.
        }
    }
    settings
}
pub fn clean_nick(value: &str) -> String {
    let result: String = value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || "_.-".contains(*c))
        .take(24)
        .collect();
    if result.len() < 2 {
        "stalker".into()
    } else {
        result
    }
}
fn valid_repo(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        })
}
pub fn validate(settings: &Settings) -> Result<(), String> {
    if settings.game != "anomaly" && settings.game != "gamma" {
        return Err("Unsupported game selection".into());
    }
    if settings.language != "en" && settings.language != "ru" {
        return Err("Unsupported language".into());
    }
    if !valid_repo(&settings.repo) {
        return Err("Use a GitHub owner/repository, not a URL".into());
    }
    if settings.profile.contains(['/', '\\', '\r', '\n', ':'])
        || settings.profile == "."
        || settings.profile == ".."
    {
        return Err("Invalid MO2 profile name".into());
    }
    if !settings.installed.is_empty()
        && (!settings
            .installed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
            || settings.installed == "."
            || settings.installed == "..")
    {
        return Err("Invalid installed build tag".into());
    }
    for path in [
        &settings.anomaly_dir,
        &settings.mo2_dir,
        &settings.stand_root,
    ] {
        if !path.is_empty() && (!Path::new(path).is_absolute() || path.contains(['\r', '\n', '\0']))
        {
            return Err("Choose an absolute local folder path".into());
        }
    }
    Ok(())
}
pub fn check_plain_ancestors(path: &Path) -> Result<(), String> {
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
                        "Refusing to write through a filesystem link: {}",
                        ancestor.display()
                    ));
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.to_string()),
        }
    }
    Ok(())
}
fn load_local(home: &Path) -> Result<Settings, String> {
    let path = home.join("settings.json");
    check_plain_ancestors(&path)?;
    let mut settings = if path.exists() {
        serde_json::from_slice(&fs::read(&path).map_err(|err| err.to_string())?)
            .map_err(|err| format!("Cannot read RayLink settings: {err}"))?
    } else {
        // A fresh installation is independent. Global lite preferences belong
        // to that older installation and are never imported implicitly.
        let mut settings = Settings::default();
        if let Some(language) = crate::lifecycle::installed_language(home) {
            settings.language = language;
        }
        settings
    };
    validate(&settings)?;
    // Repair the path imported by 0.2.0/0.2.1. Do not adopt, copy or mutate the
    // old stand: prepare a fresh one inside this installation instead.
    if !settings.stand_root.is_empty()
        && Path::new(&settings.stand_root)
            .join(&settings.game)
            .join(".raylink_stand.tsv")
            .is_file()
    {
        settings.stand_root.clear();
    }
    Ok(settings)
}
#[cfg(test)]
fn save_local(home: &Path, settings: &Settings) -> Result<(), String> {
    validate(settings)?;
    let path = home.join("settings.json");
    let temporary = home.join("settings.json.new");
    check_plain_ancestors(&path)?;
    check_plain_ancestors(&temporary)?;
    fs::create_dir_all(home).map_err(|err| err.to_string())?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|err| err.to_string())?;
    use std::io::Write;
    let mut file = fs::File::create(&temporary).map_err(|err| err.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|err| err.to_string())?;
    drop(file);
    fs::rename(temporary, path).map_err(|err| err.to_string())
}

pub struct Store {
    directory: std::path::PathBuf,
}
struct StoreLock {
    _file: fs::File,
    #[cfg(not(windows))]
    path: std::path::PathBuf,
}
#[cfg(not(windows))]
impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    schema: u32,
    home: std::path::PathBuf,
    settings: Settings,
}
fn identity(home: &Path) -> Result<std::path::PathBuf, String> {
    check_plain_ancestors(home)?;
    fs::canonicalize(home).map_err(|err| err.to_string())
}
fn same_home(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
impl Store {
    pub fn new(directory: std::path::PathBuf) -> Self {
        Self { directory }
    }
    pub fn path(&self) -> std::path::PathBuf {
        self.directory.join("settings.json")
    }
    fn lock(&self) -> Result<StoreLock, String> {
        let path = self.directory.join(".settings.lock");
        check_plain_ancestors(&path)?;
        fs::create_dir_all(&self.directory).map_err(|err| err.to_string())?;
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
        let file = options.open(path.clone()).map_err(|_| {
            "Another RayLink operation is changing preferences. Try again.".to_string()
        })?;
        Ok(StoreLock {
            _file: file,
            #[cfg(not(windows))]
            path,
        })
    }
    fn read(&self) -> Result<Option<Preferences>, String> {
        let path = self.path();
        check_plain_ancestors(&path)?;
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.to_string()),
        };
        if !file.metadata().map_err(|err| err.to_string())?.is_file() {
            return Err("Preferences must be a regular file".into());
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|err| err.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("Preferences file is too large".into());
        }
        let value: Preferences = serde_json::from_slice(&bytes)
            .map_err(|err| format!("Cannot read RayLink preferences: {err}"))?;
        if value.schema != 2 || !value.home.is_absolute() {
            return Err("Unsupported RayLink preferences format".into());
        }
        validate(&value.settings)?;
        Ok(Some(value))
    }
    pub fn load(&self, home: &Path) -> Result<Settings, String> {
        let _lock = self.lock()?;
        let owner = identity(home)?;
        if let Some(saved) = self.read()? {
            let mut prefs = saved.settings;
            if !same_home(&saved.home, &owner) {
                prefs.installed.clear();
                prefs.stand_root.clear();
            }
            return Ok(prefs);
        }
        // Only our marked native 0.2.x installation may migrate its JSON.
        // The C# settings.txt file is never inspected or imported.
        let prefs = if crate::lifecycle::is_installed(home) {
            load_local(home)?
        } else {
            Settings::default()
        };
        if crate::lifecycle::is_installed(home) {
            self.save_locked(home, &prefs)?;
        }
        Ok(prefs)
    }
    pub fn save(&self, home: &Path, settings: &Settings) -> Result<(), String> {
        let _lock = self.lock()?;
        self.save_locked(home, settings)
    }
    fn save_locked(&self, home: &Path, settings: &Settings) -> Result<(), String> {
        validate(settings)?;
        let prefs = Preferences {
            schema: 2,
            home: identity(home)?,
            settings: settings.clone(),
        };
        let path = self.path();
        check_plain_ancestors(&path)?;
        fs::create_dir_all(&self.directory).map_err(|err| err.to_string())?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|err| err.to_string())?
            .as_nanos();
        let temporary = self
            .directory
            .join(format!("settings-{}-{nonce}.new", std::process::id()));
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|err| err.to_string())?;
        let result = (|| {
            file.write_all(&serde_json::to_vec_pretty(&prefs).map_err(|err| err.to_string())?)
                .and_then(|()| file.sync_all())
                .map_err(|err| err.to_string())?;
            drop(file);
            fs::rename(&temporary, &path).map_err(|err| err.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        self.remove_native_local(home)?;
        Ok(())
    }
    fn remove_native_local(&self, home: &Path) -> Result<bool, String> {
        let path = home.join("settings.json");
        if !crate::lifecycle::is_installed(home) || path == self.path() || !path.exists() {
            return Ok(false);
        }
        // Validate the exact native JSON before removing it after migration.
        load_local(home)?;
        fs::remove_file(path).map_err(|err| err.to_string())?;
        Ok(true)
    }
    pub fn remove_for_home(&self, home: &Path) -> Result<bool, String> {
        let _lock = self.lock()?;
        let owner = identity(home)?;
        let mut removed = false;
        if let Some(prefs) = self.read()?
            && same_home(&prefs.home, &owner)
        {
            fs::remove_file(self.path()).map_err(|err| err.to_string())?;
            removed = true;
        }
        Ok(self.remove_native_local(home)? || removed)
    }
}
fn user_store() -> Result<Store, String> {
    Ok(Store::new(
        std::path::PathBuf::from(std::env::var_os("APPDATA").ok_or("AppData is unavailable")?)
            .join("RayLink"),
    ))
}
pub fn location() -> Result<std::path::PathBuf, String> {
    Ok(user_store()?.path())
}
pub fn load(home: &Path) -> Result<Settings, String> {
    user_store()?.load(home)
}
pub fn save(home: &Path, settings: &Settings) -> Result<(), String> {
    user_store()?.save(home, settings)
}
pub fn remove_for_home(home: &Path) -> Result<bool, String> {
    user_store()?.remove_for_home(home)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(windows)]
    fn concurrent_preferences_writer_blocks_save_and_delete() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp");
        fs::create_dir_all(&root).unwrap();
        let fixture = tempfile::tempdir_in(root).unwrap();
        let home = fixture.path().join("home");
        fs::create_dir(&home).unwrap();
        let store = Store::new(fixture.path().join("appdata/RayLink"));
        store.save(&home, &Settings::default()).unwrap();
        let guard = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(store.directory.join(".settings.lock"))
            .unwrap();
        assert!(store.save(&home, &Settings::default()).is_err());
        assert!(store.remove_for_home(&home).is_err());
        assert!(store.path().exists());
        drop(guard);
    }
    fn mark_native(home: &Path) {
        fs::create_dir_all(home).unwrap();
        fs::write(
            home.join(".raylink-install.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema":1,"product":"RayLink Launcher","home":fs::canonicalize(home).unwrap(),
                "language":"ru","desktop":false,"test_install":true,"version":"0.2.1"
            }))
            .unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn marked_native_preferences_move_once_to_appdata_without_resurrection() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp");
        fs::create_dir_all(&root).unwrap();
        let fixture = tempfile::tempdir_in(root).unwrap();
        let home = fixture.path().join("home");
        mark_native(&home);
        let home = fs::canonicalize(home).unwrap();
        let store = Store::new(fixture.path().join("appdata/RayLink"));
        let original = Settings {
            nick: "Alice".into(),
            language: "ru".into(),
            installed: "b24".into(),
            ..Settings::default()
        };
        save_local(&home, &original).unwrap();
        assert_eq!(store.load(&home).unwrap(), original);
        assert!(store.path().exists());
        assert!(!home.join("settings.json").exists());
        assert!(store.remove_for_home(&home).unwrap());
        let fresh = store.load(&home).unwrap();
        assert_eq!(fresh.nick, "stalker");
        assert!(fresh.installed.is_empty());
        assert_eq!(fresh.language, "ru");
    }
    #[test]
    fn unmarked_local_json_is_not_migrated() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp");
        fs::create_dir_all(&root).unwrap();
        let fixture = tempfile::tempdir_in(root).unwrap();
        let home = fixture.path().join("home");
        save_local(
            &home,
            &Settings {
                nick: "SomeoneElse".into(),
                ..Settings::default()
            },
        )
        .unwrap();
        let store = Store::new(fixture.path().join("appdata/RayLink"));
        assert_eq!(store.load(&home).unwrap(), Settings::default());
        assert!(!store.path().exists());
        assert!(home.join("settings.json").exists());
    }
    #[test]
    fn appdata_preferences_are_saved_with_installation_identity() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp");
        fs::create_dir_all(&root).unwrap();
        let fixture = tempfile::tempdir_in(root).unwrap();
        let home = fixture.path().join("home");
        fs::create_dir(&home).unwrap();
        let store = Store::new(fixture.path().join("appdata/RayLink"));
        let prefs = Settings {
            nick: "Alice".into(),
            language: "ru".into(),
            ..Settings::default()
        };
        store.save(&home, &prefs).unwrap();
        assert!(store.path().is_file());
        assert!(!home.join("settings.json").exists());
        assert_eq!(store.load(&home).unwrap(), prefs);
    }
    #[test]
    fn changing_home_reuses_preferences_but_not_old_build_or_stand() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp");
        fs::create_dir_all(&root).unwrap();
        let fixture = tempfile::tempdir_in(root).unwrap();
        let a = fixture.path().join("a");
        let b = fixture.path().join("b");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        let store = Store::new(fixture.path().join("appdata/RayLink"));
        store
            .save(
                &a,
                &Settings {
                    nick: "Alice".into(),
                    installed: "b24".into(),
                    stand_root: fixture
                        .path()
                        .join("old-stand")
                        .to_string_lossy()
                        .into_owned(),
                    ..Settings::default()
                },
            )
            .unwrap();
        let prefs = store.load(&b).unwrap();
        assert_eq!(prefs.nick, "Alice");
        assert!(prefs.installed.is_empty());
        assert!(prefs.stand_root.is_empty());
    }
    #[test]
    fn uninstall_can_remove_only_its_own_preferences() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp");
        fs::create_dir_all(&root).unwrap();
        let fixture = tempfile::tempdir_in(root).unwrap();
        let a = fixture.path().join("a");
        let b = fixture.path().join("b");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        let store = Store::new(fixture.path().join("appdata/RayLink"));
        store.save(&a, &Settings::default()).unwrap();
        assert!(!store.remove_for_home(&b).unwrap());
        assert!(store.path().exists());
        assert!(store.remove_for_home(&a).unwrap());
        assert!(!store.path().exists());
    }
    #[test]
    fn fresh_home_ignores_other_installations_global_settings() {
        let project_temp = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp");
        fs::create_dir_all(&project_temp).unwrap();
        let fixture = tempfile::tempdir_in(project_temp).unwrap();
        let old_appdata = fixture.path().join("old-appdata");
        fs::create_dir_all(old_appdata.join("RayLink")).unwrap();
        let old_stand = fixture.path().join("old-stand");
        fs::write(
            old_appdata.join("RayLink/settings.txt"),
            format!(
                "game=anomaly\nstand={}\nnick=OldPlayer\n",
                old_stand.display()
            ),
        )
        .unwrap();
        let home = fixture.path().join("new-home");
        fs::create_dir(&home).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "settings::tests::isolated_preferences_child",
            ])
            .env("APPDATA", &old_appdata)
            .env("RAYLINK_ISOLATION_HOME", &home)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(old_appdata.join("RayLink/settings.txt").exists());
    }
    #[test]
    #[ignore = "Subprocess helper invoked by isolation regression"]
    fn isolated_preferences_child() {
        let home = std::env::var_os("RAYLINK_ISOLATION_HOME").unwrap();
        let loaded = load(Path::new(&home)).unwrap();
        assert_eq!(
            loaded,
            Settings::default(),
            "A new home must not import another installation"
        );
    }
    #[test]
    fn old_tsv_path_in_local_preferences_is_rebased_without_touching_old_data() {
        let project_temp = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp");
        fs::create_dir_all(&project_temp).unwrap();
        let fixture = tempfile::tempdir_in(project_temp).unwrap();
        let old_base = fixture.path().join("old");
        fs::create_dir_all(old_base.join("gamma/appdata_a/savedgames")).unwrap();
        fs::write(old_base.join("gamma/.raylink_stand.tsv"), "legacy marker").unwrap();
        fs::write(
            old_base.join("gamma/appdata_a/savedgames/old.sav"),
            "old save",
        )
        .unwrap();
        let home = fixture.path().join("new");
        let prefs = Settings {
            stand_root: old_base.to_string_lossy().into_owned(),
            ..Settings::default()
        };
        save_local(&home, &prefs).unwrap();
        let loaded = load_local(&home).unwrap();
        assert!(loaded.stand_root.is_empty());
        assert_eq!(
            fs::read_to_string(old_base.join("gamma/appdata_a/savedgames/old.sav")).unwrap(),
            "old save"
        );
    }
    #[test]
    fn legacy_migration_keeps_choices_but_not_credentials() {
        let settings = parse_legacy(
            "\u{feff}game=anomaly\nanomaly=D:\\Anomaly\nmo2=D:\\GAMMA\nprofile=G.A.M.M.A\nstand=D:\\RayLink\nnick=Alice\nrepo=stormie-labs/RayLink-Engine\ninstalled=b24\ntoken=do-not-import\njoin=1\n",
        );
        assert_eq!(settings.game, "anomaly");
        assert_eq!(settings.anomaly_dir, "D:\\Anomaly");
        assert_eq!(settings.stand_root, "D:\\RayLink");
        assert_eq!(settings.nick, "Alice");
        assert_eq!(settings.repo, "stormie-labs/RayLink-Coop");
        assert_eq!(settings.language, "en");
        assert!(
            !serde_json::to_string(&settings)
                .unwrap()
                .contains("do-not-import")
        );
    }
    #[test]
    fn nick_is_a_bounded_argument() {
        assert_eq!(clean_nick("a b;с"), "ab");
        assert_eq!(clean_nick("x"), "stalker");
        assert_eq!(clean_nick(&"a".repeat(50)).len(), 24);
    }
    #[test]
    fn rejects_unsupported_game_locale_and_channel() {
        let mut settings = Settings {
            game: "../../game".into(),
            ..Settings::default()
        };
        assert!(validate(&settings).is_err());
        settings.game = "gamma".into();
        settings.language = "fr".into();
        assert!(validate(&settings).is_err());
        settings.language = "en".into();
        settings.repo = "../evil".into();
        assert!(validate(&settings).is_err());
    }
    #[test]
    fn preferences_round_trip_inside_selected_home() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            nick: "Alice".into(),
            language: "ru".into(),
            ..Settings::default()
        };
        save_local(dir.path(), &settings).unwrap();
        assert_eq!(load_local(dir.path()).unwrap(), settings);
    }
}
