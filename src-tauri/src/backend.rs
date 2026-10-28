use std::{fs, path::{Path, PathBuf}, process::Command, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};
use rl_core::{channel, dto::{Settings, Snapshot, Release, LauncherUpdate}, gamma, install, job::Reporter, lifecycle, settings, stand, selection::{self, Selection}};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

pub struct Backend {
    home: PathBuf, selection: Mutex<Selection>, installed_destination: Mutex<Option<PathBuf>>,
    update: Mutex<Option<LauncherUpdate>>, installer: bool, uninstaller: bool,
    pub busy: AtomicBool, cancel: AtomicBool,
}
fn lock<T>(value: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, String> { value.lock().map_err(|_| "Application state is unavailable. Restart RayLink.".into()) }
impl Backend {
    pub fn is_installer(&self) -> bool { self.installer }
    pub fn is_uninstaller(&self) -> bool { self.uninstaller }
    pub fn webview_directory(&self) -> Result<PathBuf, String> {
        let path = if self.installer {
            PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("Local application cache is unavailable")?).join("RayLinkSetup/WebView")
        } else if self.uninstaller { self.home.join("cache/uninstall-webview") } else { self.home.join("webview") };
        settings::check_plain_ancestors(&path)?;
        Ok(path)
    }
    pub fn new() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        let home = exe.parent().ok_or("Cannot locate the launcher folder")?.to_path_buf();
        let uninstaller = lifecycle::is_uninstaller();
        if uninstaller && !lifecycle::is_installed(&home) { return Err("Run the uninstaller from its installed RayLink folder".into()); }
        let installer = !uninstaller && (lifecycle::is_installer() || (!cfg!(debug_assertions) && !lifecycle::is_installed(&home)));
        let prefs = if installer || uninstaller {
            let mut prefs = Settings::default();
            if uninstaller && let Some(language) = lifecycle::installed_language(&home) { prefs.language = language; }
            prefs
        } else { settings::load(&home)? };
        Ok(Self { home, selection: Mutex::new(Selection::new(prefs)), installed_destination: Mutex::new(None), update: Mutex::new(None), installer, uninstaller, busy: AtomicBool::new(false), cancel: AtomicBool::new(false) })
    }
    fn prefs(&self) -> Result<Settings, String> { Ok(lock(&self.selection)?.settings.clone()) }
    fn log(&self, message: &str) {
        if !lifecycle::is_installed(&self.home) { return; }
        let path = self.home.join("logs/launcher.log");
        if settings::check_plain_ancestors(&path).is_err() { return; }
        if let Some(parent) = path.parent() && fs::create_dir_all(parent).is_err() { return; }
        use std::io::Write;
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |value| value.as_secs());
            let _ = writeln!(file, "{time} {}", rl_core::redact::redact(message));
        }
    }
    fn require_installed(&self) -> Result<(), String> { if self.installer || self.uninstaller || !lifecycle::is_installed(&self.home) { Err("Open the installed RayLink launcher to use this action".into()) } else { Ok(()) } }
    fn snapshot(&self) -> Result<Snapshot, String> {
        let (prefs, release) = { let selected = lock(&self.selection)?; (selected.settings.clone(), selected.release.clone()) };
        let root = stand::stand_path(&prefs, &self.home);
        let profiles = if prefs.mo2_dir.is_empty() { Vec::new() } else { gamma::profiles(Path::new(&prefs.mo2_dir)).unwrap_or_default() };
        let running = root.is_dir() && !stand::running_processes(&root)?.is_empty();
        Ok(Snapshot { gamma_version: gamma::version(Path::new(&prefs.mo2_dir)), stand_ready: stand::is_ready(&prefs, &self.home),
            stand_path: root.to_string_lossy().into_owned(), settings: prefs, home_dir: self.home.to_string_lossy().into_owned(), profiles, running,
            release, launcher_version: env!("CARGO_PKG_VERSION").into(), installer: self.installer, uninstaller: self.uninstaller, settings_path: settings::location()?.to_string_lossy().into_owned(), installed_app: lifecycle::is_installed(&self.home) })
    }
    fn persist(&self, mut prefs: Settings) -> Result<(), String> { let mut selected = lock(&self.selection)?; if !selected.settings.repo.eq_ignore_ascii_case(&prefs.repo) { prefs.installed.clear(); } settings::save(&self.home, &prefs)?; selected.replace(prefs); Ok(()) }
    fn install_build(&self, reporter: &Reporter) -> Result<(), String> {
        self.require_installed()?;
        let mut prefs = self.prefs()?;
        let cached = lock(&self.selection)?.release.clone();
        let release = match cached { Some(release) => release, None => channel::latest(&prefs.repo)? };
        let cache = self.home.join("cache/builds").join(prefs.repo.to_ascii_lowercase()).join(&release.tag);
        settings::check_plain_ancestors(&cache)?;
        fs::create_dir_all(&cache).map_err(|err| err.to_string())?;
        let archive = cache.join(&release.asset_name);
        channel::download(&release, &archive, &self.cancel, reporter)?;
        install::install_archive(&archive, &selection::build_root(&self.home, &prefs), &release.tag, &self.cancel, reporter)?;
        prefs.installed = release.tag.clone(); self.persist(prefs)?; lock(&self.selection)?.release = Some(release); Ok(())
    }
    fn prepare(&self, reporter: &Reporter) -> Result<(), String> {
        self.require_installed()?;
        let prefs = self.prefs()?;
        let root = stand::stand_path(&prefs, &self.home);
        if !stand::running_processes(&root)?.is_empty() { return Err("Close the game before preparing or updating its files".into()); }
        if prefs.installed.is_empty() || !selection::build_directory(&self.home, &prefs).is_dir() { self.install_build(reporter)?; }
        let prefs = self.prefs()?;
        let build = selection::build_directory(&self.home, &prefs);
        install::verify_build(&build)?;
        stand::materialize(&prefs, &build, &self.home, &self.cancel, reporter)?;
        Ok(())
    }
}
struct BusyGuard(Arc<Backend>);
impl Drop for BusyGuard { fn drop(&mut self) { self.0.busy.store(false, Ordering::Release); } }
async fn work<T: Send + 'static>(app: AppHandle, operation: impl FnOnce(&Backend, &Reporter) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let state = Arc::clone(app.state::<Arc<Backend>>().inner());
    if state.busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed).is_err() { return Err("Another operation is already running".into()); }
    state.cancel.store(false, Ordering::Release);
    let guard = BusyGuard(Arc::clone(&state));
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let last = Mutex::new((String::new(), std::time::Instant::now()));
        let emitter = |progress: rl_core::dto::Progress| {
            if progress.phase == "stand_missing_mod" { state.log(&progress.message); }
            if let Ok(mut latest) = last.lock() {
                let changed = latest.0 != progress.phase;
                if changed || latest.1.elapsed().as_millis() >= 50 || (progress.total > 0 && progress.current >= progress.total) {
                    if changed && progress.phase != "stand_missing_mod" { state.log(&progress.message); }
                    *latest = (progress.phase.clone(), std::time::Instant::now());
                    let _ = app.emit("raylink-progress", progress);
                }
            }
        };
        let result = operation(&state, &emitter);
        if let Err(error) = &result { state.log(&format!("Operation failed: {error}")); }
        result
    }).await.map_err(|err| format!("Background operation failed: {err}"))?
}
#[tauri::command] pub async fn snapshot(app: AppHandle) -> Result<Snapshot, String> {
    let state = Arc::clone(app.state::<Arc<Backend>>().inner());
    tauri::async_runtime::spawn_blocking(move || state.snapshot()).await.map_err(|err| err.to_string())?
}
#[tauri::command] pub async fn save_settings(app: AppHandle, mut settings: Settings) -> Result<Snapshot, String> {
    work(app, move |state, _| { state.require_installed()?; settings::validate(&settings)?;
        if !settings.anomaly_dir.is_empty() { gamma::validate_anomaly(Path::new(&settings.anomaly_dir))?; }
        if settings.game == "gamma" && !settings.mo2_dir.is_empty() { gamma::validate_mo2(Path::new(&settings.mo2_dir))?;
            if !settings.profile.is_empty() && !gamma::profiles(Path::new(&settings.mo2_dir))?.contains(&settings.profile) { return Err("Choose an existing MO2 profile".into()); }
        }
        settings.installed = state.prefs()?.installed; settings.nick = settings::clean_nick(&settings.nick);
        state.persist(settings)?;
        state.snapshot()
    }).await
}
#[derive(Serialize)] #[serde(rename_all = "camelCase")] pub struct Mo2Info { profiles: Vec<String>, anomaly_dir: Option<String>, selected: Option<String> }
#[tauri::command] pub async fn inspect_mo2(path: String) -> Result<Mo2Info, String> {
    tauri::async_runtime::spawn_blocking(move || { let root = Path::new(&path); gamma::validate_mo2(root)?; Ok(Mo2Info { profiles: gamma::profiles(root)?, anomaly_dir: gamma::suggested_anomaly(root).map(|p| p.to_string_lossy().into_owned()), selected: gamma::selected_profile(root) }) }).await.map_err(|err| err.to_string())?
}
#[tauri::command] pub async fn choose_folder(app: AppHandle, title: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || app.dialog().file().set_title(title).blocking_pick_folder().map(|p| p.into_path().map(|path| path.to_string_lossy().into_owned())).transpose().map_err(|err| err.to_string())).await.map_err(|err| err.to_string())?
}
#[tauri::command] pub async fn check_build(app: AppHandle) -> Result<Release, String> {
    let state = Arc::clone(app.state::<Arc<Backend>>().inner());
    tauri::async_runtime::spawn_blocking(move || { let (repo, generation) = { let selected = lock(&state.selection)?; (selected.settings.repo.clone(), selected.generation) }; let release = channel::latest(&repo)?;
        if !lock(&state.selection)?.cache(&repo, generation, release.clone()) { return Err("Build channel changed; check again".into()); } Ok(release) }).await.map_err(|err| err.to_string())?
}
#[tauri::command] pub async fn install_build(app: AppHandle) -> Result<Snapshot, String> { work(app, |state, reporter| { state.install_build(reporter)?; state.snapshot() }).await }
#[tauri::command] pub async fn prepare_game(app: AppHandle) -> Result<Snapshot, String> { work(app, |state, reporter| { state.prepare(reporter)?; state.snapshot() }).await }
#[tauri::command] pub async fn launch_game(app: AppHandle) -> Result<Snapshot, String> { work(app, |state, reporter| {
    state.require_installed()?; let prefs = state.prefs()?; if !stand::verify_ready(&prefs, &state.home) { state.prepare(reporter)?; }
    let prefs = state.prefs()?; rl_core::launch::game(&stand::stand_path(&prefs, &state.home), &prefs.nick)?; state.snapshot()
}).await }
#[tauri::command] pub async fn verify_build(app: AppHandle) -> Result<(), String> { work(app, |state, _| { state.require_installed()?; let prefs = state.prefs()?; if prefs.installed.is_empty() { return Err("Install a game build first".into()); } install::verify_build(&selection::build_directory(&state.home, &prefs)) }).await }
#[tauri::command] pub fn cancel_job(state: tauri::State<'_, Arc<Backend>>) { state.cancel.store(true, Ordering::Release); }
#[tauri::command] pub async fn open_folder(app: AppHandle, kind: String) -> Result<(), String> { work(app, move |state, _| {
    state.require_installed()?;
    let path = match kind.as_str() { "home" => state.home.clone(), "logs" => state.home.join("logs"), _ => return Err("Unknown folder action".into()) };
    if !path.is_dir() { return Err("This folder has not been created yet".into()); }
    #[cfg(windows)] { let explorer = std::env::var_os("WINDIR").map(PathBuf::from).ok_or("Windows directory is unavailable")?.join("explorer.exe"); Command::new(explorer).arg(&path).spawn().map_err(|err| err.to_string())?; }
    Ok(())
}).await }
#[tauri::command] pub async fn check_launcher_update(app: AppHandle) -> Result<Option<LauncherUpdate>, String> {
    let state = Arc::clone(app.state::<Arc<Backend>>().inner());
    tauri::async_runtime::spawn_blocking(move || { let update = lifecycle::check_update(env!("CARGO_PKG_VERSION"))?; *lock(&state.update)? = update.clone(); Ok(update) }).await.map_err(|err| err.to_string())?
}
#[tauri::command] pub async fn apply_launcher_update(app: AppHandle) -> Result<(), String> {
    let handle = app.clone(); work(app, |state, reporter| { state.require_installed()?; let update = lock(&state.update)?.clone().ok_or("Check for a launcher update first")?; lifecycle::begin_update(&update, &state.home, &state.cancel, reporter) }).await?;
    handle.exit(0); Ok(())
}
#[tauri::command] pub fn default_install_dir() -> String { lifecycle::default_install_dir().to_string_lossy().into_owned() }
#[tauri::command] pub fn set_installer_language(state: tauri::State<'_, Arc<Backend>>, language: String) -> Result<(), String> {
    if !state.installer { return Err("Language selection is available in installer mode only".into()); }
    if language != "en" && language != "ru" { return Err("Unsupported language".into()); }
    lock(&state.selection)?.settings.language = language; Ok(())
}
#[tauri::command] pub async fn install_app(app: AppHandle, destination: String, language: String, desktop: bool) -> Result<String, String> { work(app, move |state, reporter| {
    if !state.installer { return Err("Start RayLinkSetup.exe to install the launcher".into()); }
    let path = Path::new(&destination); let exe = lifecycle::install_to(path, &language, desktop, reporter)?;
    let mut prefs = settings::load(path)?; prefs.language = language; settings::save(path, &prefs)?;
    let root = exe.parent().ok_or("Invalid installation directory")?.to_path_buf(); *lock(&state.installed_destination)? = Some(root.clone()); Ok(root.to_string_lossy().into_owned())
}).await }
#[tauri::command] pub async fn open_installed(app: AppHandle) -> Result<(), String> {
    work(app, |state, _| {
        if !state.installer { return Err("This action is only available after setup".into()); }
        let root = lock(&state.installed_destination)?.clone().ok_or("Complete installation before opening RayLink")?;
        if !lifecycle::is_installed(&root) { return Err("The installed RayLink home is unavailable".into()); }
        Command::new(root.join("RayLinkLauncher.exe")).current_dir(root).spawn().map_err(|err| err.to_string())?; Ok(())
    }).await
}

#[tauri::command] pub async fn open_uninstaller(app: AppHandle) -> Result<(), String> {
    let handle = app.clone();
    work(app, |state, _| { state.require_installed()?; Command::new(state.home.join("uninstall.exe")).current_dir(&state.home).spawn().map_err(|err| err.to_string())?; Ok(()) }).await?;
    handle.exit(0); Ok(())
}
#[tauri::command] pub async fn uninstall_app(app: AppHandle, delete_settings: bool) -> Result<(), String> {
    let handle = app.clone();
    work(app, move |state, _| { if !state.uninstaller { return Err("Open the RayLink uninstaller first".into()); } lifecycle::begin_uninstall(&state.home, delete_settings) }).await?;
    handle.exit(0); Ok(())
}
