//! Secret-free contract between the Rust backend and the desktop view.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub game: String, pub anomaly_dir: String, pub mo2_dir: String,
    pub profile: String, pub stand_root: String, pub nick: String,
    pub repo: String, pub installed: String, pub language: String, pub reduced_motion: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self { game: "gamma".into(), anomaly_dir: String::new(), mo2_dir: String::new(), profile: String::new(), stand_root: String::new(), nick: "stalker".into(), repo: "stormie-labs/RayLink-Coop".into(), installed: String::new(), language: "en".into(), reduced_motion: false }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub tag: String, pub name: String, pub body: String, pub published_at: String,
    pub page_url: String, pub asset_name: String, pub asset_url: String,
    pub asset_size: u64, pub sha256: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: String, pub message: String, pub current: u64,
    pub total: u64, pub cancellable: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StandSummary { pub root: String, pub linked: u64, pub copied: u64, pub unchanged: u64, pub removed: u64 }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub settings: Settings, pub home_dir: String, pub profiles: Vec<String>,
    pub gamma_version: String, pub stand_path: String, pub stand_ready: bool,
    pub running: bool, pub release: Option<Release>, pub launcher_version: String,
    pub installer: bool, pub installed_app: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherUpdate { pub version: String, pub release: Release }
