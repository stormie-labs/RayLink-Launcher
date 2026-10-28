//! One synchronized owner for the selected channel and its fetched release.
use crate::dto::{Settings, Release};
pub struct Selection { pub settings: Settings, pub release: Option<Release>, pub generation: u64 }
impl Selection {
    pub fn new(settings: Settings) -> Self { Self { settings, release: None, generation: 0 } }
    pub fn replace(&mut self, mut settings: Settings) {
        if !self.settings.repo.eq_ignore_ascii_case(&settings.repo) { settings.installed.clear(); self.release = None; self.generation = self.generation.wrapping_add(1); }
        self.settings = settings;
    }
    pub fn cache(&mut self, repo: &str, generation: u64, release: Release) -> bool {
        if generation != self.generation || !self.settings.repo.eq_ignore_ascii_case(repo) { return false; }
        self.release = Some(release); true
    }
}
pub fn build_root(home: &std::path::Path, settings: &Settings) -> std::path::PathBuf {
    if settings.repo.eq_ignore_ascii_case("stormie-labs/RayLink-Coop") { home.join("builds") }
    else { home.join("builds/channels").join(settings.repo.to_ascii_lowercase()) }
}
pub fn build_directory(home: &std::path::Path, settings: &Settings) -> std::path::PathBuf { build_root(home, settings).join(&settings.installed) }
#[cfg(test)] mod tests {
    use super::*;
    fn release() -> Release { Release { tag: "b24".into(), name: "test".into(), body: String::new(), published_at: String::new(), page_url: String::new(), asset_name: String::new(), asset_url: String::new(), asset_size: 1, sha256: None } }
    #[test] fn late_release_from_old_channel_is_discarded() {
        let mut selection = Selection::new(Settings::default()); let original = selection.settings.repo.clone(); let generation = selection.generation;
        selection.replace(Settings { repo: "someone/Other".into(), ..Settings::default() });
        assert!(!selection.cache(&original, generation, release())); assert!(selection.release.is_none());
    }
    #[test] fn channels_cannot_reuse_each_others_tag() {
        let home = std::path::Path::new("D:/RayLink"); let a = Settings { installed: "b24".into(), ..Settings::default() };
        let b = Settings { repo: "someone/Other".into(), ..a.clone() };
        assert_ne!(build_directory(home, &a), build_directory(home, &b));
        let mut selection = Selection::new(a); selection.replace(b); assert!(selection.settings.installed.is_empty());
    }
}
