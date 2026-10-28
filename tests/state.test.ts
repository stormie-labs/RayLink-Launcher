import { describe, it, expect } from 'vitest';
import { primaryAction, progressPercent, safeLocale } from '../src/state';
import { defaultSettings, type Snapshot } from '../src/contract';
const snapshot: Snapshot = { settings: { ...defaultSettings }, homeDir: 'D:/RayLink', profiles: [], gammaVersion: '', standPath: '', standReady: false, running: false, release: null, launcherVersion: '0.2.0', installer: false, installedApp: true };
describe('primary action is driven by real installation state', () => {
  it('asks for paths before touching downloads', () => expect(primaryAction(snapshot, false)).toBe('setup'));
  it('builds a configured but unprepared stand', () => expect(primaryAction({ ...snapshot, settings: { ...snapshot.settings, anomalyDir: 'D:/Anomaly', mo2Dir: 'D:/GAMMA', profile: 'G.A.M.M.A' } }, false)).toBe('prepare'));
  it('plays a ready stand', () => expect(primaryAction({ ...snapshot, standReady: true, settings: { ...snapshot.settings, anomalyDir: 'D:/Anomaly', mo2Dir: 'D:/GAMMA', profile: 'G.A.M.M.A' } }, false)).toBe('play'));
  it('never launches into a running or mutating stand', () => { expect(primaryAction({ ...snapshot, running: true }, false)).toBe('running'); expect(primaryAction(snapshot, true)).toBe('busy'); });
  it('requires no MO2 folder for stock Anomaly', () => expect(primaryAction({ ...snapshot, settings: { ...snapshot.settings, game: 'anomaly', anomalyDir: 'D:/Anomaly' } }, false)).toBe('prepare'));
});
it('bounds progress and handles unknown length without fake percentages', () => { expect(progressPercent({current: 5, total: 0})).toBe(null); expect(progressPercent({current: 900, total: 100})).toBe(100); expect(progressPercent({current: -1, total: 10})).toBe(0); });
it('English is the default locale', () => { expect(safeLocale('fr')).toBe('en'); expect(safeLocale('ru')).toBe('ru'); });
