//! Native, journaled stand materialization.
mod apply;
use crate::{
    dto::{Settings, StandSummary},
    gamma,
    job::{Reporter, check_cancel, report},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::UNIX_EPOCH,
};
const MANIFEST: &str = ".raylink-stand.json";
const JOURNAL: &str = ".raylink-stand-journal.json";
const LOCK: &str = ".raylink-stand.lock";
const READY: &str = ".raylink-ready.json";
const EXE: &str = "bin/AnomalyCoopDX11AVX.exe";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Entry {
    rel: String,
    source: Option<PathBuf>,
    size: u64,
    modified: u64,
    copy: bool,
    hash: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: u32,
    owner: String,
    root: PathBuf,
    build: PathBuf,
    fingerprint: String,
    complete: bool,
    entries: Vec<Entry>,
}
struct Plan {
    root: PathBuf,
    build: PathBuf,
    fingerprint: String,
    entries: BTreeMap<String, Entry>,
    seeds: BTreeMap<String, Entry>,
    generated: Vec<u8>,
    user: Vec<u8>,
}
struct Layer {
    root: PathBuf,
    prefix: &'static str,
    copy: bool,
    game: bool,
    is_mod: bool,
}

fn require_link_volumes(
    layers: &[Layer],
    root: &Path,
    mut same_volume: impl FnMut(&Path, &Path) -> Result<bool, String>,
) -> Result<(), String> {
    for layer in layers.iter().filter(|layer| !layer.copy) {
        if !same_volume(&layer.root, root)? {
            return Err(format!(
                "STAND_VOLUME_MISMATCH: Game/mod source {} and stand {} are on different volumes. Choose a stand folder on the game/mod volume; a full game copy was not started.",
                layer.root.display(),
                root.display()
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct FileStamp {
    size: u64,
    modified: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    format: u32,
    root: PathBuf,
    build: PathBuf,
    signature: String,
    manifest: FileStamp,
    critical: Vec<Entry>,
}

/// Bounded UI readiness hint. Full source-tree validation belongs to `verify_ready` before launch.
/// This reads a small certificate, settings/MO2 list and essential files, never the 62k-file manifest.
pub fn is_ready(settings: &Settings, home: &Path) -> bool {
    fn ready(settings: &Settings, home: &Path) -> Result<bool, String> {
        let root = rl_fs::absolute(&stand_path(settings, home))?;
        rl_fs::guard(&root)?;
        for rel in [JOURNAL, LOCK, ".raylink_stand.tsv"] {
            if rl_fs::join(&root, rel)?.exists() {
                return Ok(false);
            }
        }
        let cert_path = rl_fs::join(&root, READY)?;
        if fs::metadata(&cert_path).map_err(|e| e.to_string())?.len() > 65536 {
            return Ok(false);
        }
        let cert: Ready = serde_json::from_slice(&fs::read(cert_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if cert.format != 1
            || cert.root != root
            || cert.manifest != stamp(&rl_fs::join(&root, MANIFEST)?)?
        {
            return Ok(false);
        }
        let mut critical = BTreeMap::new();
        for entry in &cert.critical {
            if ![
                EXE.to_lowercase(),
                "fsgame.ltx".into(),
                "fsgame_a.ltx".into(),
            ]
            .contains(&entry.rel.to_lowercase())
                || critical
                    .insert(entry.rel.to_lowercase(), entry.clone())
                    .is_some()
            {
                return Ok(false);
            }
            let path = rl_fs::join(&root, &entry.rel)?;
            let Some(hash) = &entry.hash else {
                return Ok(false);
            };
            if cached_hash(&path)? != *hash {
                return Ok(false);
            }
        }
        if critical.len() != 3 || !rl_fs::join(&root, "appdata_a/user.ltx")?.is_file() {
            return Ok(false);
        }
        Ok(cert.signature == quick_signature(settings, &root, &cert.build, &critical)?)
    }
    ready(settings, home).unwrap_or(false)
}

pub fn stand_path(settings: &Settings, home: &Path) -> PathBuf {
    if settings.stand_root.trim().is_empty() {
        home.join(&settings.game)
    } else {
        Path::new(&settings.stand_root).join(&settings.game)
    }
}
/// Full acceptance check for launch: all sources, manifest entries and critical content hashes.
pub fn verify_ready(settings: &Settings, home: &Path) -> bool {
    fn ready(settings: &Settings, home: &Path) -> Result<bool, String> {
        let mut files = rl_fs::Batch::new();
        let root = rl_fs::absolute(&stand_path(settings, home))?;
        rl_fs::guard(&root)?;
        if root.join(JOURNAL).exists()
            || root.join(LOCK).exists()
            || root.join(".raylink_stand.tsv").exists()
        {
            return Ok(false);
        }
        let Some(state) = load(&root, MANIFEST)? else {
            return Ok(false);
        };
        if !state.complete {
            return Ok(false);
        }
        let plan = plan(
            settings,
            &state.build,
            home,
            &AtomicBool::new(false),
            &|_| {},
            &mut files,
        )?;
        if state.fingerprint != plan.fingerprint {
            return Ok(false);
        }
        let recorded = entries(&state)?;
        if recorded != plan.entries {
            return Ok(false);
        }
        for entry in recorded.values() {
            let target = files.join(&root, &entry.rel)?;
            if !target.is_file() {
                return Ok(false);
            }
            if let Some(hash) = &entry.hash
                && hash_file(&target)? != *hash
            {
                return Ok(false);
            }
        }
        Ok(root.join(EXE).is_file()
            && root.join("fsgame_a.ltx").is_file()
            && rl_fs::join(&root, "appdata_a/user.ltx")?.is_file())
    }
    ready(settings, home).unwrap_or(false)
}

pub fn materialize(
    settings: &Settings,
    build: &Path,
    home: &Path,
    cancel: &AtomicBool,
    progress: &Reporter,
) -> Result<StandSummary, String> {
    check_cancel(cancel)?;
    report(
        progress,
        "stand_scan",
        "Inspecting game and enabled layers",
        0,
        0,
        true,
    );
    let mut files = rl_fs::Batch::new();
    let plan = plan(settings, build, home, cancel, progress, &mut files)?;
    let root = &plan.root;
    let processes = running_processes(root)?;
    if !processes.is_empty() {
        return Err(format!(
            "Close processes running from this stand before updating: {}",
            processes.join(", ")
        ));
    }
    files.create_dir_all(root)?;
    let _lock = StandLock::acquire(root)?;
    let old = load(root, MANIFEST)?;
    let pending = load(root, JOURNAL)?;
    let mut owned = BTreeMap::new();
    if let Some(old) = &old {
        owned.extend(entries(old)?);
    }
    if let Some(pending) = &pending {
        owned.extend(entries(pending)?);
    }
    // A journal records intent, not proof that each replacement finished. Reapply on recovery.
    let applied = if pending.is_some() {
        BTreeMap::new()
    } else if let Some(old) = &old {
        entries(old)?
    } else {
        BTreeMap::new()
    };
    if root.exists() && old.is_none() && pending.is_none() {
        for item in
            fs::read_dir(root).map_err(|e| format!("Cannot inspect stand directory: {e}"))?
        {
            let item = item.map_err(|e| e.to_string())?;
            if item.file_name() != LOCK {
                return Err(format!(
                    "Stand directory is not empty and has no native ownership manifest: {}. Choose an empty stand directory; existing files were preserved.",
                    root.display()
                ));
            }
        }
    }
    // Check ALL old and new destinations before any mutation, including paths only removed by this update.
    let mut intended = owned.clone();
    intended.extend(plan.entries.clone());
    let destinations: BTreeMap<_, _> = owned
        .values()
        .chain(plan.entries.values())
        .map(|entry| (entry.rel.as_str(), entry))
        .collect();
    for entry in destinations.values() {
        let dst = files.join(root, &entry.rel)?;
        if dst.exists() && !dst.is_file() {
            return Err(format!(
                "A directory occupies a managed file path: {}",
                dst.display()
            ));
        }
        if plan.entries.contains_key(&entry.rel.to_lowercase())
            && dst.exists()
            && !owned.contains_key(&entry.rel.to_lowercase())
        {
            return Err(format!(
                "Refusing to replace an unmanaged file: {}",
                dst.display()
            ));
        }
    }
    for seed in plan.seeds.values() {
        files.join(root, &seed.rel)?;
    }
    files.join(root, "appdata_a/user.ltx")?;
    check_cancel(cancel)?;
    save(
        root,
        JOURNAL,
        &plan,
        intended.values().cloned().collect(),
        false,
    )?;
    let mut summary = StandSummary {
        root: root.to_string_lossy().into_owned(),
        linked: 0,
        copied: 0,
        unchanged: 0,
        removed: 0,
    };
    let total = (owned.len() + plan.entries.len() + plan.seeds.len() + 1) as u64;
    let mut current = 0;
    for (key, entry) in &owned {
        check_cancel(cancel)?;
        if !plan.entries.contains_key(key) {
            let target = files.join(root, &entry.rel)?;
            if target.exists() {
                files.remove_file(&target)?;
                summary.removed += 1;
            }
        }
        current += 1;
    }
    // Match the legacy builder: create/check each shared parent once before
    // independent file jobs begin. Every entry was validated in the preflight.
    let directories: std::collections::BTreeSet<_> = plan
        .entries
        .values()
        .filter_map(|entry| root.join(&entry.rel).parent().map(Path::to_path_buf))
        .collect();
    for directory in directories {
        check_cancel(cancel)?;
        files.create_dir_all(&directory)?;
    }
    let placed = apply::run(
        &plan.entries,
        &applied,
        root,
        &plan.generated,
        cancel,
        progress,
        current,
        total,
    )?;
    summary.linked += placed.linked;
    summary.copied += placed.copied;
    summary.unchanged += placed.unchanged;
    current += plan.entries.len() as u64;
    for seed in plan.seeds.values() {
        check_cancel(cancel)?;
        let target = files.join(root, &seed.rel)?;
        if !target.exists()
            && let Some(source) = &seed.source
        {
            files.place(source, &target, true)?;
            summary.copied += 1;
        }
        current += 1;
        report(
            progress,
            "stand_apply",
            &format!("Seeding {}", seed.rel),
            current,
            total,
            true,
        );
    }
    check_cancel(cancel)?;
    let user = files.join(root, "appdata_a/user.ltx")?;
    files.create_dir_all(
        user.parent()
            .ok_or_else(|| "User config has no parent".to_string())?,
    )?;
    if !user.exists() {
        files.atomic_write(&user, &plan.user)?;
        summary.copied += 1;
    }
    // Reject a source changed during installation rather than recording a mixed stand as ready.
    check_cancel(cancel)?;
    report(
        progress,
        "stand_verify",
        "Verifying source stability",
        current,
        total,
        true,
    );
    let after = plan_fingerprint(settings, &plan, home, cancel, &mut files)?;
    if after != plan.fingerprint {
        return Err("A source changed while assembling the stand. Close MO2 updates and retry; saves were preserved.".into());
    }
    check_cancel(cancel)?;
    save(
        root,
        MANIFEST,
        &plan,
        plan.entries.values().cloned().collect(),
        true,
    )?;
    let critical: BTreeMap<_, _> = plan
        .entries
        .iter()
        .filter(|(k, _)| {
            [
                EXE.to_lowercase(),
                "fsgame.ltx".into(),
                "fsgame_a.ltx".into(),
            ]
            .contains(k)
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let cert = Ready {
        format: 1,
        root: root.clone(),
        build: plan.build.clone(),
        signature: quick_signature(settings, root, &plan.build, &critical)?,
        manifest: stamp(&rl_fs::join(root, MANIFEST)?)?,
        critical: critical.into_values().collect(),
    };
    rl_fs::atomic_write(
        &rl_fs::join(root, READY)?,
        &serde_json::to_vec(&cert).map_err(|e| e.to_string())?,
    )?;
    rl_fs::remove_file(&rl_fs::join(root, JOURNAL)?)?;
    report(
        progress,
        "stand_complete",
        "Stand is ready",
        total,
        total,
        false,
    );
    Ok(summary)
}

pub fn running_processes(root: &Path) -> Result<Vec<String>, String> {
    rl_fs::running_processes(root)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn hash_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn stamp(path: &Path) -> Result<FileStamp, String> {
    rl_fs::guard(path)?;
    let meta = fs::metadata(path).map_err(|e| format!("Cannot inspect {}: {e}", path.display()))?;
    let modified = meta
        .modified()
        .map_err(|e| e.to_string())?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    Ok(FileStamp {
        size: meta.len(),
        modified: u64::try_from(modified).map_err(|_| "Timestamp is out of range".to_string())?,
    })
}
fn cached_hash(path: &Path) -> Result<String, String> {
    type HashCache = std::sync::Mutex<BTreeMap<PathBuf, (FileStamp, String)>>;
    static CACHE: std::sync::OnceLock<HashCache> = std::sync::OnceLock::new();
    let before = stamp(path)?;
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()));
    if let Some((previous, hash)) = cache.lock().map_err(|e| e.to_string())?.get(path)
        && *previous == before
    {
        return Ok(hash.clone());
    }
    let hash = hash_file(path)?;
    if before != stamp(path)? {
        return Err("An essential file changed during readiness verification".into());
    }
    let mut cache = cache.lock().map_err(|e| e.to_string())?;
    if cache.len() >= 256 {
        cache.clear();
    }
    cache.insert(path.to_path_buf(), (before, hash.clone()));
    Ok(hash)
}
fn quick_signature(
    settings: &Settings,
    root: &Path,
    build: &Path,
    critical: &BTreeMap<String, Entry>,
) -> Result<String, String> {
    if !matches!(settings.game.as_str(), "gamma" | "anomaly") || settings.installed.is_empty() {
        return Err("No valid installed game is selected".into());
    }
    rl_fs::guard(build)?;
    let (layers, mo2) = layers(settings, build, None)?;
    let mut dirs = Vec::new();
    for layer in layers {
        if !layer.root.is_dir() || rl_fs::overlaps(root, &layer.root)? {
            return Err("A source layer is missing or overlaps the stand".into());
        }
        dirs.push((layer.root.clone(), stamp(&layer.root)?));
    }
    let mut essential_sources = Vec::new();
    for entry in critical.values() {
        if let Some(source) = &entry.source {
            essential_sources.push((source, stamp(source)?, cached_hash(source)?));
        }
    }
    let bytes = serde_json::to_vec(&(
        &settings.game,
        &settings.installed,
        &settings.profile,
        &settings.repo,
        rl_fs::absolute(Path::new(&settings.anomaly_dir))?,
        if settings.game == "gamma" {
            Some(rl_fs::absolute(Path::new(&settings.mo2_dir))?)
        } else {
            None
        },
        root,
        build,
        dirs,
        mo2,
        essential_sources,
    ))
    .map_err(|e| e.to_string())?;
    Ok(hash_bytes(&bytes))
}
fn hash_file(path: &Path) -> Result<String, String> {
    use std::io::Read;
    rl_fs::guard(path)?;
    let mut input =
        fs::File::open(path).map_err(|e| format!("Cannot hash {}: {e}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let size = input
            .read(&mut buffer)
            .map_err(|e| format!("Cannot hash {}: {e}", path.display()))?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    Ok(hex(&hash.finalize()))
}
fn preserved(rel: &str) -> bool {
    let low = rel.to_lowercase();
    matches!(
        low.split('/').next().unwrap_or_default(),
        "appdata" | "appdata_a" | "appdata_b" | "saves" | "savedgames"
    ) || matches!(
        low.rsplit('/').next().unwrap_or_default(),
        "user.ltx" | "user.cfg"
    )
}
fn runtime_written(rel: &str) -> bool {
    let low = rel.to_lowercase();
    let parts: Vec<_> = low.split('/').collect();
    parts.len() == 1
        || preserved(&low)
        || (parts[0] == "bin"
            && parts.len() == 2
            && [".ini", ".log", ".txt"].iter().any(|e| low.ends_with(e)))
        || (parts[0] == "gamedata"
            && (parts.len() == 2 || (parts.len() == 3 && parts[1] == "configs")))
}
fn managed_relative(rel: &str) -> Result<(), String> {
    rl_fs::validate_relative(rel)?;
    if preserved(rel)
        || rel.split('/').any(|p| {
            p.to_lowercase().starts_with(".raylink")
                || p.eq_ignore_ascii_case(".git")
                || p.eq_ignore_ascii_case("bin_b")
        })
    {
        return Err(format!("Unsafe managed manifest path: {rel}"));
    }
    Ok(())
}
fn layers(
    settings: &Settings,
    build: &Path,
    progress: Option<&Reporter>,
) -> Result<(Vec<Layer>, String), String> {
    gamma::validate_anomaly(Path::new(&settings.anomaly_dir))?;
    let mut layers = vec![
        Layer {
            root: rl_fs::absolute(Path::new(&settings.anomaly_dir))?,
            prefix: "",
            copy: false,
            game: true,
            is_mod: false,
        },
        Layer {
            root: build.join("Anomaly Coop - Engine/bin"),
            prefix: "bin",
            copy: true,
            game: false,
            is_mod: false,
        },
        Layer {
            root: build.join("Anomaly Coop - Engine/gamedata"),
            prefix: "gamedata",
            copy: true,
            game: false,
            is_mod: false,
        },
    ];
    let mut mo2_signature = String::new();
    if settings.game == "gamma" {
        let mo2 = Path::new(&settings.mo2_dir);
        gamma::validate_mo2(mo2)?;
        let (modlist, mods) = gamma::enabled_mods(mo2, &settings.profile)?;
        mo2_signature = hash_bytes(
            format!(
                "{}\n{}",
                gamma::read_text(&rl_fs::join(mo2, "ModOrganizer.ini")?)?,
                gamma::read_text(&modlist)?
            )
            .as_bytes(),
        );
        let mods_root = gamma::folder(mo2, "mod_directory", "mods")?;
        for name in mods {
            let path = rl_fs::join(&mods_root, &name)?;
            if !path.exists() {
                if let Some(progress) = progress {
                    report(
                        progress,
                        "stand_missing_mod",
                        &format!(
                            "Enabled MO2 mod folder was not found and was skipped: {name} ({})",
                            path.display()
                        ),
                        0,
                        0,
                        true,
                    );
                }
                continue;
            }
            if !path.is_dir() {
                return Err(format!(
                    "Enabled MO2 mod path is not a directory: {}",
                    path.display()
                ));
            }
            layers.push(Layer {
                root: path,
                prefix: "",
                copy: false,
                game: false,
                is_mod: true,
            });
        }
        let overwrite = gamma::folder(mo2, "overwrite_directory", "overwrite")?;
        if overwrite.exists() {
            layers.push(Layer {
                root: overwrite,
                prefix: "",
                copy: false,
                game: false,
                is_mod: false,
            });
        }
    }
    layers.push(Layer {
        root: build.join("Anomaly Coop - Core/gamedata"),
        prefix: "gamedata",
        copy: true,
        game: false,
        is_mod: false,
    });
    let gamma = build.join("Anomaly Coop - GAMMA/gamedata");
    rl_fs::guard(&gamma)?;
    if settings.game == "gamma" && gamma.exists() {
        layers.push(Layer {
            root: gamma,
            prefix: "gamedata",
            copy: true,
            game: false,
            is_mod: false,
        });
    }
    Ok((layers, mo2_signature))
}
fn scan(
    layer: &Layer,
    want: &mut BTreeMap<String, Entry>,
    seeds: &mut BTreeMap<String, Entry>,
    cancel: &AtomicBool,
    files: &mut rl_fs::Batch,
) -> Result<(), String> {
    files.guard(&layer.root)?;
    if !layer.root.is_dir() {
        return Err(format!(
            "Required layer directory is missing: {}",
            layer.root.display()
        ));
    }
    let mut stack = vec![(layer.root.clone(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        check_cancel(cancel)?;
        for item in fs::read_dir(&dir).map_err(|e| format!("Cannot scan {}: {e}", dir.display()))? {
            check_cancel(cancel)?;
            let item = item.map_err(|e| e.to_string())?;
            let name = item
                .file_name()
                .into_string()
                .map_err(|_| "Source filename is not valid Unicode".to_string())?;
            let low = name.to_lowercase();
            if low == ".git"
                || low == "bin_b"
                || low == "appdata_b"
                || low.ends_with(".mohidden")
                || (layer.game
                    && prefix.is_empty()
                    && matches!(low.as_str(), "appdata" | "appdata_a"))
            {
                continue;
            }
            if layer.is_mod && prefix.is_empty() && low == "meta.ini" {
                continue;
            }
            let path = item.path();
            let metadata = files.metadata(&path)?;
            let inner = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            rl_fs::validate_relative(&inner)?;
            if metadata.is_dir() {
                stack.push((path, inner));
                continue;
            }
            if !metadata.is_file() {
                return Err(format!("Source is not a regular file: {}", path.display()));
            }
            let rel = if layer.prefix.is_empty() {
                inner
            } else {
                format!("{}/{inner}", layer.prefix)
            };
            if rel.eq_ignore_ascii_case("fsgame_a.ltx") {
                continue;
            }
            let modified = metadata
                .modified()
                .map_err(|e| e.to_string())?
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            let hash = if rel.eq_ignore_ascii_case(EXE) || rel.eq_ignore_ascii_case("fsgame.ltx") {
                Some(hash_file(&path)?)
            } else {
                None
            };
            let entry = Entry {
                copy: layer.copy || runtime_written(&rel),
                rel: rel.clone(),
                source: Some(path),
                size: metadata.len(),
                modified: u64::try_from(modified)
                    .map_err(|_| "Source timestamp is out of range".to_string())?,
                hash,
            };
            if preserved(&rel) {
                // Mutable game data is seeded once and never journal-owned, overwritten or deleted.
                if !matches!(low.as_str(), "user.ltx" | "user.cfg") {
                    seeds.insert(rel.to_lowercase(), entry.clone());
                    if let Some(rest) = rel.strip_prefix("appdata/") {
                        let mut a = entry;
                        a.rel = format!("appdata_a/{rest}");
                        seeds.insert(a.rel.to_lowercase(), a);
                    }
                }
            } else {
                managed_relative(&rel)?;
                want.insert(rel.to_lowercase(), entry);
            }
        }
    }
    Ok(())
}
fn plan(
    settings: &Settings,
    build: &Path,
    home: &Path,
    cancel: &AtomicBool,
    progress: &Reporter,
    files: &mut rl_fs::Batch,
) -> Result<Plan, String> {
    if !matches!(settings.game.as_str(), "gamma" | "anomaly") {
        return Err("Unsupported game; expected gamma or anomaly".into());
    }
    if settings.installed.trim().is_empty() {
        return Err("No installed build tag is selected".into());
    }
    let root = rl_fs::absolute(&stand_path(settings, home))?;
    let build = rl_fs::absolute(build)?;
    rl_fs::guard(&root)?;
    rl_fs::guard(&build)?;
    let legacy = rl_fs::join(&root, ".raylink_stand.tsv")?;
    if legacy.exists() {
        return Err(format!(
            "Legacy TSV stand detected at {}. Automatic import is unsafe; choose an empty stand root and copy saves manually. No files were deleted.",
            root.display()
        ));
    }
    let (layers, mo2_signature) = layers(settings, &build, Some(progress))?;
    let mut protected_roots = vec![
        build.clone(),
        rl_fs::absolute(Path::new(&settings.anomaly_dir))?,
    ];
    if settings.game == "gamma" {
        protected_roots.push(rl_fs::absolute(Path::new(&settings.mo2_dir))?);
    }
    protected_roots.extend(layers.iter().map(|l| l.root.clone()));
    for source in &protected_roots {
        if rl_fs::overlaps(&root, source)? {
            return Err(format!(
                "Stand output overlaps source directory: {}",
                source.display()
            ));
        }
    }
    require_link_volumes(&layers, &root, rl_fs::same_volume)?;
    let mut want = BTreeMap::new();
    let mut seeds = BTreeMap::new();
    for (index, layer) in layers.iter().enumerate() {
        check_cancel(cancel)?;
        report(
            progress,
            "stand_scan",
            &format!("Scanning {}", layer.root.display()),
            index as u64,
            layers.len() as u64,
            true,
        );
        scan(layer, &mut want, &mut seeds, cancel, files)?;
    }
    if !want.contains_key(&EXE.to_lowercase()) {
        return Err(format!("Build is missing {EXE}"));
    }
    let fs_source = want
        .get("fsgame.ltx")
        .and_then(|e| e.source.as_ref())
        .ok_or_else(|| "No fsgame.ltx found in source layers".to_string())?;
    let bytes = fs::read(fs_source).map_err(|e| format!("Cannot read fsgame.ltx: {e}"))?;
    let generated = instance_config(&bytes)?;
    want.insert(
        "fsgame_a.ltx".into(),
        Entry {
            rel: "fsgame_a.ltx".into(),
            source: None,
            size: generated.len() as u64,
            modified: 0,
            copy: true,
            hash: Some(hash_bytes(&generated)),
        },
    );
    let user = user_config(Path::new(&settings.anomaly_dir), &settings.nick)?;
    let fingerprint = fingerprint(settings, &root, &build, &want, &seeds, &mo2_signature)?;
    Ok(Plan {
        root,
        build,
        fingerprint,
        entries: want,
        seeds,
        generated,
        user,
    })
}
fn fingerprint(
    settings: &Settings,
    root: &Path,
    build: &Path,
    entries: &BTreeMap<String, Entry>,
    seeds: &BTreeMap<String, Entry>,
    mo2: &str,
) -> Result<String, String> {
    let bytes = serde_json::to_vec(&(
        &settings.game,
        &settings.installed,
        &settings.profile,
        &settings.repo,
        rl_fs::absolute(Path::new(&settings.anomaly_dir))?,
        if settings.game == "gamma" {
            Some(rl_fs::absolute(Path::new(&settings.mo2_dir))?)
        } else {
            None
        },
        root,
        build,
        entries,
        seeds,
        mo2,
    ))
    .map_err(|e| e.to_string())?;
    Ok(hash_bytes(&bytes))
}
fn plan_fingerprint(
    settings: &Settings,
    existing: &Plan,
    home: &Path,
    cancel: &AtomicBool,
    files: &mut rl_fs::Batch,
) -> Result<String, String> {
    Ok(plan(settings, &existing.build, home, cancel, &|_| {}, files)?.fingerprint)
}
fn instance_config(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let text = encoding_rs::WINDOWS_1251.decode(bytes).0;
    let mut found = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if line
            .split_once('=')
            .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("$app_data_root$"))
        {
            if found {
                return Err("fsgame.ltx has duplicate app_data_root entries".into());
            }
            let (prefix, _) = line
                .rsplit_once('|')
                .ok_or_else(|| "fsgame.ltx app_data_root has no path field".to_string())?;
            lines.push(format!("{prefix}|appdata_a\\"));
            found = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if !found {
        return Err("fsgame.ltx has no app_data_root entry".into());
    }
    let text = lines.join("\r\n") + "\r\n";
    Ok(encoding_rs::WINDOWS_1251.encode(&text).0.into_owned())
}
fn user_config(anomaly: &Path, nick: &str) -> Result<Vec<u8>, String> {
    let source = rl_fs::join(anomaly, "appdata/user.ltx")?;
    let bytes = if source.is_file() {
        fs::read(&source).map_err(|e| format!("Cannot read source user.ltx: {e}"))?
    } else {
        Vec::new()
    };
    let text = encoding_rs::WINDOWS_1251.decode(&bytes).0;
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| {
            !matches!(
                l.split_whitespace().next(),
                Some("coop_nick" | "g_always_active")
            )
        })
        .map(String::from)
        .collect();
    let nick: String = nick
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || "_-.".contains(*c))
        .take(32)
        .collect();
    lines.push("g_always_active on".into());
    lines.push(format!(
        "coop_nick {}",
        if nick.is_empty() { "stalker" } else { &nick }
    ));
    Ok(encoding_rs::WINDOWS_1251
        .encode(&(lines.join("\r\n") + "\r\n"))
        .0
        .into_owned())
}
fn entries(state: &Manifest) -> Result<BTreeMap<String, Entry>, String> {
    let mut entries = BTreeMap::new();
    for entry in &state.entries {
        managed_relative(&entry.rel)?;
        if entries
            .insert(entry.rel.to_lowercase(), entry.clone())
            .is_some()
        {
            return Err("Manifest contains duplicate file paths".into());
        }
    }
    Ok(entries)
}
fn load(root: &Path, name: &str) -> Result<Option<Manifest>, String> {
    let path = rl_fs::join(root, name)?;
    let metadata = match fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Cannot read ownership manifest: {e}")),
    };
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err("Invalid ownership manifest size or file type".into());
    }
    let state: Manifest = serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Invalid native stand manifest; no files were deleted: {e}"))?;
    if state.format != 1 || state.owner != "RayLink native stand" || state.root != root {
        return Err(
            "Ownership manifest does not belong to this stand directory; no files were deleted"
                .into(),
        );
    }
    entries(&state)?;
    Ok(Some(state))
}
fn save(
    root: &Path,
    name: &str,
    plan: &Plan,
    entries: Vec<Entry>,
    complete: bool,
) -> Result<(), String> {
    let state = Manifest {
        format: 1,
        owner: "RayLink native stand".into(),
        root: root.to_path_buf(),
        build: plan.build.clone(),
        fingerprint: plan.fingerprint.clone(),
        complete,
        entries,
    };
    let bytes = serde_json::to_vec(&state).map_err(|e| e.to_string())?;
    rl_fs::atomic_write(&rl_fs::join(root, name)?, &bytes)
}
struct StandLock {
    file: Option<fs::File>,
    path: PathBuf,
}
impl StandLock {
    fn acquire(root: &Path) -> Result<Self, String> {
        let path = rl_fs::join(root, LOCK)?;
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let file = options
            .open(&path)
            .map_err(|e| format!("Cannot lock stand (another build may be running): {e}"))?;
        Ok(Self {
            file: Some(file),
            path,
        })
    }
}
impl Drop for StandLock {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = rl_fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    #[test]
    fn stand_volume_mismatch_is_rejected_before_output_creation() {
        let (fixture, settings, build, home) = fixture();
        let (layers, _) = layers(&settings, &build, None).unwrap();
        let root = fixture.path().join("stand-on-another-volume");
        let error = require_link_volumes(&layers, &root, |_, _| Ok(false)).unwrap_err();
        assert!(error.starts_with("STAND_VOLUME_MISMATCH:"));
        assert!(!root.exists());
        assert!(!stand_path(&settings, &home).exists());
        let copy_only = layers
            .into_iter()
            .filter(|layer| layer.copy)
            .collect::<Vec<_>>();
        require_link_volumes(&copy_only, &root, |_, _| {
            panic!("Explicit engine/core copies may cross volumes")
        })
        .unwrap();
    }
    fn fixture() -> (tempfile::TempDir, Settings, PathBuf, PathBuf) {
        let t = tempfile::tempdir_in(
            fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp")).unwrap(),
        )
        .unwrap();
        let a = t.path().join("anomaly");
        let m = t.path().join("mo2");
        let b = t.path().join("build");
        let h = t.path().join("home");
        for path in [
            "db/base.db",
            "gamedata/scripts/order.script",
            "gamedata/configs/runtime.ltx",
            "bin/config.ini",
            "appdata/user.ltx",
            "fsgame.ltx",
        ] {
            put(
                &a,
                path,
                if path == "fsgame.ltx" {
                    "$app_data_root$ = true|false|$fs_root$|appdata\\\n$game_data$ = true|true|$fs_root$|gamedata\\\n"
                } else {
                    "anomaly"
                },
            );
        }
        put(
            &b,
            "Anomaly Coop - Engine/bin/AnomalyCoopDX11AVX.exe",
            "exe",
        );
        put(
            &b,
            "Anomaly Coop - Engine/gamedata/scripts/order.script",
            "engine",
        );
        put(
            &b,
            "Anomaly Coop - Core/gamedata/scripts/core.script",
            "core",
        );
        put(
            &b,
            "Anomaly Coop - GAMMA/gamedata/scripts/gamma.script",
            "gamma",
        );
        put(&m, "ModOrganizer.ini", "selected_profile=Default\n");
        put(
            &m,
            "profiles/Default/modlist.txt",
            "+high\n+Group_separator\n-low-disabled\n+low\n",
        );
        put(&m, "mods/low/gamedata/scripts/order.script", "low");
        put(&m, "mods/high/gamedata/scripts/order.script", "high");
        put(&m, "mods/high/meta.ini", "meta");
        put(
            &m,
            "mods/high/gamedata/scripts/hidden.script.mohidden",
            "hidden",
        );
        put(&m, "mods/high/gamedata/scripts/core.script", "mod");
        put(&m, "mods/high/gamedata/scripts/gamma.script", "mod");
        let settings = Settings {
            anomaly_dir: a.to_string_lossy().into(),
            mo2_dir: m.to_string_lossy().into(),
            profile: "Default".into(),
            installed: "b17".into(),
            ..Settings::default()
        };
        (t, settings, b, h)
    }
    fn put(root: &Path, path: &str, text: &str) {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    #[test]
    fn stand_layers_preserve_source_and_user_data_and_detect_stale() {
        let (_t, mut s, b, h) = fixture();
        let cancel = AtomicBool::new(false);
        let summary = materialize(&s, &b, &h, &cancel, &|_| {}).unwrap();
        let root = stand_path(&s, &h);
        assert!(summary.linked > 0 && summary.copied > 0);
        assert_eq!(
            std::fs::read(root.join("gamedata/scripts/order.script")).unwrap(),
            b"high"
        );
        assert_eq!(
            std::fs::read(root.join("gamedata/scripts/core.script")).unwrap(),
            b"core"
        );
        assert_eq!(
            std::fs::read(root.join("gamedata/scripts/gamma.script")).unwrap(),
            b"gamma"
        );
        assert!(!root.join("meta.ini").exists());
        assert!(!root.join("bin_b").exists());
        assert!(
            !root
                .join("gamedata/scripts/hidden.script.mohidden")
                .exists()
        );
        assert!(
            std::fs::read_to_string(root.join("fsgame_a.ltx"))
                .unwrap()
                .contains("appdata_a\\")
        );
        assert!(is_ready(&s, &h));
        put(&root, "bin/config.ini", "local edit");
        assert_eq!(
            std::fs::read(Path::new(&s.anomaly_dir).join("bin/config.ini")).unwrap(),
            b"anomaly"
        );
        put(&root, "appdata_a/user.ltx", "my settings");
        put(&root, "appdata_a/savedgames/a.scop", "save");
        put(&root, "unmanaged.txt", "keep");
        s.installed = "b18".into();
        assert!(!is_ready(&s, &h));
        materialize(&s, &b, &h, &cancel, &|_| {}).unwrap();
        assert_eq!(
            std::fs::read(root.join("appdata_a/user.ltx")).unwrap(),
            b"my settings"
        );
        assert!(root.join("appdata_a/savedgames/a.scop").exists());
        assert!(root.join("unmanaged.txt").exists());
        put(
            Path::new(&s.mo2_dir),
            "profiles/Default/modlist.txt",
            "+low\n",
        );
        assert!(!is_ready(&s, &h));
        materialize(&s, &b, &h, &cancel, &|_| {}).unwrap();
        assert_eq!(
            std::fs::read(root.join("gamedata/scripts/order.script")).unwrap(),
            b"low"
        );
    }
    #[test]
    #[cfg(windows)]
    fn stand_pins_source_and_output_only_during_the_filesystem_pass() {
        let (fixture, settings, build, home) = fixture();
        let source = Path::new(&settings.anomaly_dir).join("gamedata/scripts");
        let root = stand_path(&settings, &home);
        let moved_source = fixture.path().join("moved-source");
        let moved_root = fixture.path().join("moved-output");
        let checked = AtomicBool::new(false);
        materialize(
            &settings,
            &build,
            &home,
            &AtomicBool::new(false),
            &|progress| {
                if progress.phase == "stand_apply" && !checked.swap(true, Ordering::Relaxed) {
                    assert!(
                        fs::rename(&source, &moved_source).is_err(),
                        "A cached source parent must stay pinned"
                    );
                    assert!(
                        fs::rename(&root, &moved_root).is_err(),
                        "A cached output parent must stay pinned"
                    );
                }
            },
        )
        .unwrap();
        assert!(checked.load(Ordering::Relaxed));
        fs::rename(&source, &moved_source).unwrap();
        fs::rename(&moved_source, &source).unwrap();
        fs::rename(&root, &moved_root).unwrap();
        fs::rename(&moved_root, &root).unwrap();
        assert!(verify_ready(&settings, &home));
    }
    #[test]
    fn stand_missing_mod_is_reported_and_skipped_like_mo2() {
        let (_t, s, b, h) = fixture();
        put(
            Path::new(&s.mo2_dir),
            "profiles/Default/modlist.txt",
            "+missing\n+low\n",
        );
        let warned = std::sync::atomic::AtomicBool::new(false);
        materialize(&s, &b, &h, &AtomicBool::new(false), &|p| {
            if p.phase == "stand_missing_mod" && p.message.contains("missing") {
                warned.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        })
        .unwrap();
        assert!(warned.load(std::sync::atomic::Ordering::Relaxed));
        assert!(stand_path(&s, &h).join(EXE).is_file());
        assert!(verify_ready(&s, &h));
    }
    #[test]
    fn stand_overlap_fails_before_output_mutation() {
        let (_t, mut s, b, h) = fixture();
        s.game = "anomaly".into();
        s.stand_root = Path::new(&s.anomaly_dir)
            .parent()
            .unwrap()
            .to_string_lossy()
            .into();
        assert!(materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).is_err());
    }
    #[test]
    fn stand_cancelled_apply_retries_and_preserves_saves() {
        let (_t, s, b, h) = fixture();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let callback_cancel = cancel.clone();
        let root = stand_path(&s, &h);
        assert!(
            materialize(&s, &b, &h, &cancel, &move |p| {
                if p.phase == "stand_apply" && p.current > 1 {
                    callback_cancel.store(true, Ordering::Relaxed);
                }
            })
            .is_err()
        );
        assert!(!is_ready(&s, &h));
        put(&root, "appdata_a/user.ltx", "keep on retry");
        cancel.store(false, Ordering::Relaxed);
        materialize(&s, &b, &h, &cancel, &|_| {}).unwrap();
        assert!(is_ready(&s, &h));
        assert_eq!(
            std::fs::read(root.join("appdata_a/user.ltx")).unwrap(),
            b"keep on retry"
        );
    }
    #[test]
    fn stand_cancelled_update_reapplies_unfinished_replacements() {
        let (_t, mut s, b, h) = fixture();
        materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        put(
            &b,
            "Anomaly Coop - Engine/bin/AnomalyCoopDX11AVX.exe",
            "new engine",
        );
        put(
            &b,
            "Anomaly Coop - Core/gamedata/scripts/core.script",
            "new core",
        );
        s.installed = "b18".into();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let callback = cancel.clone();
        assert!(
            materialize(&s, &b, &h, &cancel, &move |p| {
                if p.phase == "stand_apply" {
                    callback.store(true, Ordering::Relaxed);
                }
            })
            .is_err()
        );
        cancel.store(false, Ordering::Relaxed);
        materialize(&s, &b, &h, &cancel, &|_| {}).unwrap();
        assert_eq!(
            fs::read(stand_path(&s, &h).join("gamedata/scripts/core.script")).unwrap(),
            b"new core"
        );
        assert!(is_ready(&s, &h));
    }
    #[test]
    fn stand_manifest_traversal_and_user_paths_are_rejected_without_deletion() {
        for rel in [
            "../outside.txt",
            "db/base.db:stream",
            "CON",
            "appdata_a/user.ltx",
            "savedgames/save.scop",
        ] {
            let (t, s, b, h) = fixture();
            materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
            let root = stand_path(&s, &h);
            put(t.path(), "outside.txt", "safe");
            put(&root, "appdata_a/user.ltx", "safe config");
            let path = root.join(MANIFEST);
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            value["entries"][0]["rel"] = rel.into();
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(!is_ready(&s, &h));
            assert!(materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).is_err());
            assert_eq!(fs::read(t.path().join("outside.txt")).unwrap(), b"safe");
            assert_eq!(
                fs::read(root.join("appdata_a/user.ltx")).unwrap(),
                b"safe config"
            );
        }
    }
    #[test]
    fn stand_overwrite_core_gamma_priority_and_cp1251_compatibility() {
        let (_t, s, b, h) = fixture();
        let mo2 = Path::new(&s.mo2_dir);
        for name in ["order", "core", "gamma"] {
            put(
                mo2,
                &format!("overwrite/gamedata/scripts/{name}.script"),
                "overwrite",
            );
        }
        put(mo2, "mods/high/appdata/mod_config.ltx", "seed once");
        let a = Path::new(&s.anomaly_dir);
        let text = "$app_data_root$ = true|false|$fs_root$|appdata\\\r\n; Кириллица\r\n";
        fs::write(
            a.join("fsgame.ltx"),
            encoding_rs::WINDOWS_1251.encode(text).0.as_ref(),
        )
        .unwrap();
        fs::write(
            a.join("appdata/user.ltx"),
            encoding_rs::WINDOWS_1251
                .encode("; Настройки\r\ncoop_nick old\r\n")
                .0
                .as_ref(),
        )
        .unwrap();
        materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        let root = stand_path(&s, &h);
        for (name, want) in [("order", "overwrite"), ("core", "core"), ("gamma", "gamma")] {
            assert_eq!(
                fs::read_to_string(root.join(format!("gamedata/scripts/{name}.script"))).unwrap(),
                want
            );
        }
        assert!(
            encoding_rs::WINDOWS_1251
                .decode(&fs::read(root.join("fsgame_a.ltx")).unwrap())
                .0
                .contains("Кириллица")
        );
        assert!(
            encoding_rs::WINDOWS_1251
                .decode(&fs::read(root.join("appdata_a/user.ltx")).unwrap())
                .0
                .contains("Настройки")
        );
        put(&root, "appdata_a/mod_config.ltx", "my custom settings");
        put(mo2, "mods/high/appdata/mod_config.ltx", "updated defaults");
        materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        assert_eq!(
            fs::read(root.join("appdata_a/mod_config.ltx")).unwrap(),
            b"my custom settings"
        );
    }
    #[test]
    fn stand_legacy_tsv_is_never_imported_or_deleted() {
        let (_t, s, b, h) = fixture();
        let root = stand_path(&s, &h);
        put(
            &root,
            ".raylink_stand.tsv",
            "evil\t../outside\tignored\t0\t0\tlink",
        );
        let error = materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap_err();
        assert!(error.contains("Legacy TSV"));
        assert!(root.join(".raylink_stand.tsv").exists());
    }
    #[test]
    fn stand_unmanaged_collision_is_preserved() {
        let (_t, s, b, h) = fixture();
        materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        let root = stand_path(&s, &h);
        put(&root, "gamedata/scripts/new.script", "user owned");
        put(
            &b,
            "Anomaly Coop - Core/gamedata/scripts/new.script",
            "release file",
        );
        assert!(
            materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {})
                .unwrap_err()
                .contains("unmanaged")
        );
        assert_eq!(
            fs::read(root.join("gamedata/scripts/new.script")).unwrap(),
            b"user owned"
        );
    }
    #[test]
    fn stand_ready_tracks_source_paths_profile_and_critical_files() {
        let (_t, mut s, b, h) = fixture();
        materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        let root = stand_path(&s, &h);
        let orig_profile = s.profile.clone();
        s.profile = "Other".into();
        put(
            Path::new(&s.mo2_dir),
            "profiles/Other/modlist.txt",
            "+high\n+low\n",
        );
        assert!(!is_ready(&s, &h));
        s.profile = orig_profile;
        put(&root, "fsgame_a.ltx", "tampered config");
        assert!(!is_ready(&s, &h));
        materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        put(&root, EXE, "tampered exe");
        assert!(!is_ready(&s, &h));
        materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        s.anomaly_dir = Path::new(&s.anomaly_dir)
            .with_file_name("other-anomaly")
            .to_string_lossy()
            .into_owned();
        assert!(!is_ready(&s, &h));
    }
    #[test]
    fn stand_display_preferences_do_not_invalidate_installed_content() {
        let (_t, mut s, b, h) = fixture();
        materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        s.language = "ru".into();
        s.reduced_motion = true;
        assert!(is_ready(&s, &h));
    }
    #[test]
    fn stand_fast_ui_hint_requires_full_source_verification_before_launch() {
        let (_t, s, b, h) = fixture();
        materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        assert!(verify_ready(&s, &h));
        let start = std::time::Instant::now();
        for _ in 0..20 {
            assert!(is_ready(&s, &h));
        }
        eprintln!("20 bounded readiness checks: {:?}", start.elapsed());
        // Nested in-place edits are deliberately inspected by the full prelaunch check.
        put(
            Path::new(&s.mo2_dir),
            "mods/high/gamedata/scripts/order.script",
            "changed mod content",
        );
        assert!(is_ready(&s, &h));
        assert!(!verify_ready(&s, &h));
        materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        assert!(verify_ready(&s, &h));
    }
    #[test]
    fn stand_repo_change_invalidates_same_tag_and_preserves_explicit_namespaced_build() {
        let (_t, mut s, b, h) = fixture();
        materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        s.repo = "other-owner/custom-channel".into();
        assert!(!is_ready(&s, &h));
        assert!(!verify_ready(&s, &h));
        let custom = h.join("builds/channels/other-owner/custom-channel/b17");
        put(
            &custom,
            "Anomaly Coop - Engine/bin/AnomalyCoopDX11AVX.exe",
            "custom executable",
        );
        put(
            &custom,
            "Anomaly Coop - Engine/gamedata/scripts/order.script",
            "custom engine",
        );
        put(
            &custom,
            "Anomaly Coop - Core/gamedata/scripts/core.script",
            "custom core",
        );
        put(
            &custom,
            "Anomaly Coop - GAMMA/gamedata/scripts/gamma.script",
            "custom gamma",
        );
        materialize(&s, &custom, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        assert!(is_ready(&s, &h));
        assert!(verify_ready(&s, &h));
        let root = stand_path(&s, &h);
        assert_eq!(
            fs::read(root.join("gamedata/scripts/core.script")).unwrap(),
            b"custom core"
        );
        let cert: Ready = serde_json::from_slice(&fs::read(root.join(READY)).unwrap()).unwrap();
        assert_eq!(cert.build, rl_fs::absolute(&custom).unwrap());
        s.installed.clear();
        assert!(!is_ready(&s, &h));
        assert!(!verify_ready(&s, &h));
        s.installed = "b17".into();
        s.repo = Settings::default().repo;
        assert!(!is_ready(&s, &h));
        assert!(!verify_ready(&s, &h));
    }
    #[cfg(windows)]
    #[test]
    fn stand_rejects_output_parent_junction_and_managed_ancestor_junction() {
        let (t, mut s, b, h) = fixture();
        let elsewhere = t.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        let link = t.path().join("junction");
        let status = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(&elsewhere)
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        s.stand_root = link.join("not-created-yet").to_string_lossy().into();
        assert!(materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).is_err());
        assert!(!elsewhere.join("not-created-yet").exists());
        fs::remove_dir(&link).unwrap();
        s.stand_root.clear();
        materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).unwrap();
        let root = stand_path(&s, &h);
        let db = root.join("db");
        fs::remove_file(db.join("base.db")).unwrap();
        fs::remove_dir(&db).unwrap();
        put(&elsewhere, "base.db", "external");
        assert!(
            std::process::Command::new("cmd.exe")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(&db)
                .arg(&elsewhere)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(materialize(&s, &b, &h, &AtomicBool::new(false), &|_| {}).is_err());
        assert_eq!(fs::read(elsewhere.join("base.db")).unwrap(), b"external");
        fs::remove_dir(db).unwrap();
    }
}
