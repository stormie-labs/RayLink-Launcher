use super::{Entry, Reporter, check_cancel, hash_file, report};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

#[derive(Debug, Default)]
pub(super) struct Counts {
    pub(super) linked: u64,
    pub(super) copied: u64,
    pub(super) unchanged: u64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    entries: &BTreeMap<String, Entry>,
    applied: &BTreeMap<String, Entry>,
    root: &Path,
    generated: &[u8],
    cancel: &AtomicBool,
    progress: &Reporter,
    start: u64,
    total: u64,
) -> Result<Counts, String> {
    let jobs: Vec<(&String, &Entry)> = entries.iter().collect();
    let next = AtomicUsize::new(0);
    let state = Mutex::new((Counts::default(), None::<String>));
    let workers = thread::available_parallelism()
        .map_or(1, usize::from)
        .min(8);
    thread::scope(|scope| {
        for _ in 0..workers {
            let jobs = &jobs;
            let next = &next;
            let state = &state;
            scope.spawn(move || {
                let mut files = rl_fs::Batch::new();
                loop {
                    let job = {
                        // Admission shares the completion/error lock: no new job is
                        // assigned after an observed failure, even on another worker.
                        let Ok(mut state) = state.lock() else { return };
                        if state.1.is_some() {
                            return;
                        }
                        if let Err(error) = check_cancel(cancel) {
                            state.1 = Some(error);
                            return;
                        }
                        jobs.get(next.fetch_add(1, Ordering::Relaxed)).copied()
                    };
                    let Some((key, entry)) = job else { return };
                    let result = apply_one(&mut files, key, entry, applied, root, generated);
                    let Ok(mut state) = state.lock() else { return };
                    match result {
                        Ok((counts, fallback)) => {
                            let current =
                                start + state.0.linked + state.0.copied + state.0.unchanged;
                            if let Some(message) = fallback {
                                report(
                                    progress,
                                    "stand_copy_fallback",
                                    &message,
                                    current,
                                    total,
                                    true,
                                );
                            }
                            state.0.linked += counts.linked;
                            state.0.copied += counts.copied;
                            state.0.unchanged += counts.unchanged;
                            report(
                                progress,
                                "stand_apply",
                                &format!("Installing {}", entry.rel),
                                current + 1,
                                total,
                                true,
                            );
                        }
                        Err(error) => {
                            // In-flight IO failures outrank cancellation observed by
                            // another worker. Never replace an actual IO error.
                            if state
                                .1
                                .as_deref()
                                .is_none_or(|e| e == "Operation cancelled")
                            {
                                state.1 = Some(error);
                            }
                            return;
                        }
                    }
                }
            });
        }
    }); // scope joins every worker and drops every worker-local Batch before returning.
    let (counts, error) = state
        .into_inner()
        .map_err(|_| "File worker state is unavailable".to_string())?;
    match error {
        Some(error) => Err(error),
        None => Ok(counts),
    }
}

fn apply_one(
    files: &mut rl_fs::Batch,
    key: &str,
    entry: &Entry,
    applied: &BTreeMap<String, Entry>,
    root: &Path,
    generated: &[u8],
) -> Result<(Counts, Option<String>), String> {
    let target = files.join(root, &entry.rel)?;
    let unchanged = applied.get(key).is_some_and(|e| e == entry)
        && target.is_file()
        && match &entry.hash {
            Some(hash) => hash_file(&target)? == *hash,
            None => true,
        };
    let mut counts = Counts::default();
    if unchanged {
        counts.unchanged = 1;
        return Ok((counts, None));
    }
    files.create_dir_all(
        target
            .parent()
            .ok_or_else(|| "Managed file has no parent".to_string())?,
    )?;
    let fallback = if let Some(source) = &entry.source {
        let (linked, fallback) = files.place(source, &target, entry.copy)?;
        if linked {
            counts.linked = 1;
        } else {
            counts.copied = 1;
        }
        fallback
    } else {
        // Replace the name instead of writing through a possible hard link.
        files.remove_file(&target)?;
        files.atomic_write(&target, generated)?;
        counts.copied = 1;
        None
    };
    Ok((counts, fallback))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../temp")
                .join(format!(
                    "parallel-apply-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            fs::create_dir_all(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
        fn entries(&self, count: usize) -> BTreeMap<String, Entry> {
            let source = self.0.join("source");
            fs::write(&source, b"original source").unwrap();
            (0..count)
                .map(|i| {
                    let rel = format!("shared/nested/{i:04}.dat");
                    (
                        rel.clone(),
                        Entry {
                            rel,
                            source: Some(source.clone()),
                            size: 15,
                            modified: 0,
                            copy: i % 2 == 0,
                            hash: Some(super::super::hash_file(&source).unwrap()),
                        },
                    )
                })
                .collect()
        }
        fn assert_sources(&self) {
            assert_eq!(fs::read(self.0.join("source")).unwrap(), b"original source");
            assert_eq!(fs::read(self.0.join("sentinel")).unwrap(), b"untouched");
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn many_independent_links_copies_generated_and_unchanged() {
        let f = Fixture::new();
        let mut entries = f.entries(96);
        let root = f.0.join("output");
        fs::create_dir(&root).unwrap();
        fs::write(f.0.join("sentinel"), b"untouched").unwrap();
        let rel = "generated/config.ltx".to_string();
        entries.insert(
            rel.clone(),
            Entry {
                rel,
                source: None,
                size: 9,
                modified: 0,
                copy: true,
                hash: None,
            },
        );
        let seen = Mutex::new(Vec::new());
        let reporter = |p: crate::dto::Progress| {
            assert_eq!(p.total, 110);
            seen.lock().unwrap().push(p.current);
        };
        let cancel = AtomicBool::new(false);
        let counts = run(
            &entries,
            &BTreeMap::new(),
            &root,
            b"generated",
            &cancel,
            &reporter,
            7,
            110,
        )
        .unwrap();
        assert_eq!(
            (counts.linked, counts.copied, counts.unchanged),
            (48, 49, 0)
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 97);
        assert!(seen.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(seen.last(), Some(&104));
        drop(seen);
        for entry in entries.values() {
            let expected: &[u8] = if entry.source.is_some() {
                b"original source"
            } else {
                b"generated"
            };
            assert_eq!(fs::read(root.join(&entry.rel)).unwrap(), expected);
        }
        let counts = run(
            &entries,
            &entries,
            &root,
            b"generated",
            &cancel,
            &|_| {},
            0,
            97,
        )
        .unwrap();
        assert_eq!((counts.linked, counts.copied, counts.unchanged), (0, 0, 97));
        // An applied entry alone cannot bypass the target hash check.
        fs::write(root.join("shared/nested/0000.dat"), b"changed content").unwrap();
        let counts = run(
            &entries,
            &entries,
            &root,
            b"generated",
            &cancel,
            &|_| {},
            0,
            97,
        )
        .unwrap();
        assert_eq!((counts.linked, counts.copied, counts.unchanged), (0, 1, 96));
        f.assert_sources();
    }

    #[test]
    fn cancellation_joins_all_writers_before_return() {
        let f = Fixture::new();
        let entries = f.entries(128);
        let root = f.0.join("output");
        fs::create_dir(&root).unwrap();
        fs::write(f.0.join("sentinel"), b"untouched").unwrap();
        let cancel = AtomicBool::new(false);
        let completed = AtomicUsize::new(0);
        let result = run(
            &entries,
            &BTreeMap::new(),
            &root,
            b"",
            &cancel,
            &|p| {
                if p.phase == "stand_apply" {
                    completed.fetch_add(1, Ordering::Relaxed);
                    cancel.store(true, Ordering::Relaxed);
                }
            },
            0,
            128,
        );
        assert_eq!(result.unwrap_err(), "Operation cancelled");
        let count = completed.load(Ordering::Relaxed);
        assert!((1..=8).contains(&count));
        let moved = f.0.join("finished");
        fs::rename(&root, &moved).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(completed.load(Ordering::Relaxed), count);
        assert!(!root.exists());
        assert_eq!(
            fs::read_dir(moved.join("shared/nested")).unwrap().count(),
            count
        );
        f.assert_sources();
    }

    #[test]
    fn directory_collision_joins_workers_and_preserves_sources() {
        let f = Fixture::new();
        let entries = f.entries(128);
        let root = f.0.join("output");
        fs::create_dir_all(root.join("shared/nested/0000.dat")).unwrap();
        fs::write(root.join("shared/nested/0000.dat/unmanaged"), b"keep").unwrap();
        fs::write(f.0.join("sentinel"), b"untouched").unwrap();
        let completed = AtomicUsize::new(0);
        let error = run(
            &entries,
            &BTreeMap::new(),
            &root,
            b"",
            &AtomicBool::new(false),
            &|_| {
                completed.fetch_add(1, Ordering::Relaxed);
            },
            0,
            128,
        )
        .unwrap_err();
        assert_ne!(error, "Operation cancelled");
        assert!(error.contains("0000.dat"), "{error}");
        assert_eq!(
            fs::read(root.join("shared/nested/0000.dat/unmanaged")).unwrap(),
            b"keep"
        );
        let count = completed.load(Ordering::Relaxed);
        let moved = f.0.join("finished");
        fs::rename(&root, &moved).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(completed.load(Ordering::Relaxed), count);
        assert!(!root.exists());
        assert_eq!(
            fs::read_dir(moved.join("shared/nested")).unwrap().count(),
            count + 1
        );
        f.assert_sources();
    }
}
