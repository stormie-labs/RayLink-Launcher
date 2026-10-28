import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { defaultSettings, type Snapshot, type Progress, type Language } from './contract';

export const native = isTauri();
export const preview = !native && import.meta.env.DEV;
const params = new URLSearchParams(location.search);
export async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!native) throw new Error('Native actions are unavailable in the browser preview.');
  return invoke<T>(name, args);
}
export async function snapshot(): Promise<Snapshot> {
  if (native) return command<Snapshot>('snapshot');
  if (!preview) throw new Error('Start RayLink from its installed Windows application.');
  return {
    settings: { ...defaultSettings, language: params.get('lang') === 'ru' ? 'ru' : 'en' },
    homeDir: 'D:\\RayLink', profiles: [], gammaVersion: '', standPath: 'D:\\RayLink\\gamma',
    standReady: false, running: false, release: null, launcherVersion: '0.2.0',
    installer: params.get('preview') === 'installer', installedApp: true,
  };
}
export async function onProgress(callback: (value: Progress) => void): Promise<() => void> {
  if (!native) return () => undefined;
  return listen<Progress>('raylink-progress', event => callback(event.payload));
}
export async function chooseFolder(title: string): Promise<string | null> { return command('choose_folder', { title }); }
export async function windowAction(action: 'close' | 'minimize' | 'drag'): Promise<void> {
  if (!native) return;
  const window = getCurrentWindow();
  if (action === 'close') await window.close();
  if (action === 'minimize') await window.minimize();
  if (action === 'drag') await window.startDragging();
}
export async function changeInstallerLanguage(language: Language): Promise<void> {
  if (native) await command('set_installer_language', { language });
}
