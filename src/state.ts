import type { Language, Snapshot } from './contract';
export type PrimaryAction = 'setup' | 'prepare' | 'play' | 'running' | 'busy';
export function primaryAction(snapshot: Snapshot, busy: boolean): PrimaryAction {
  if (busy) return 'busy';
  if (snapshot.running) return 'running';
  const s = snapshot.settings;
  if (!s.anomalyDir || (s.game === 'gamma' && (!s.mo2Dir || !s.profile))) return 'setup';
  return snapshot.standReady ? 'play' : 'prepare';
}
export function progressPercent(value: { current: number; total: number }): number | null {
  if (value.total <= 0 || !Number.isFinite(value.current) || !Number.isFinite(value.total)) return null;
  return Math.max(0, Math.min(100, value.current / value.total * 100));
}
export function safeLocale(value: string): Language { return value === 'ru' ? 'ru' : 'en'; }
export function isStandVolumeMismatch(message: string): boolean { return message.startsWith('STAND_VOLUME_MISMATCH:'); }
