export type Language = 'en' | 'ru';
export type Game = 'gamma' | 'anomaly';
export interface Settings {
  game: Game; anomalyDir: string; mo2Dir: string; profile: string; standRoot: string;
  nick: string; repo: string; installed: string; language: Language; reducedMotion: boolean;
}
export interface Release {
  tag: string; name: string; body: string; publishedAt: string; pageUrl: string;
  assetName: string; assetUrl: string; assetSize: number; sha256: string | null;
}
export interface Snapshot {
  settings: Settings; homeDir: string; profiles: string[]; gammaVersion: string;
  standPath: string; standReady: boolean; running: boolean; release: Release | null;
  launcherVersion: string; installer: boolean; uninstaller: boolean; installedApp: boolean; settingsPath: string;
}
export interface Progress { phase: string; message: string; current: number; total: number; cancellable: boolean }
export interface LauncherUpdate { version: string; release: Release }
export const defaultSettings: Settings = { game: 'gamma', anomalyDir: '', mo2Dir: '', profile: '', standRoot: '', nick: 'stalker', repo: 'stormie-labs/RayLink-Coop', installed: '', language: 'en', reducedMotion: false };
