import { test, expect } from '@playwright/test';
import { defaultSettings, type Snapshot } from '../../src/contract';

test('a cross-volume stand offers a clear drive correction before retry', async ({ page }) => {
  const fixture: Snapshot = { settings: { ...defaultSettings, game: 'anomaly', language: 'ru', anomalyDir: 'E:\\Anomaly' }, homeDir: 'C:\\RayLink', settingsPath: 'AppData/RayLink/settings.json', profiles: [], gammaVersion: '', standPath: 'C:\\RayLink\\anomaly', standReady: false, running: false, release: null, launcherVersion: '0.2.3', installer: false, uninstaller: false, installedApp: true };
  await page.route('**/src/api.ts*', route => route.fulfill({ contentType: 'application/javascript', body: `
    export const native = false, preview = true;
    export async function snapshot() { return ${JSON.stringify(fixture)}; }
    export async function command() { throw new Error('STAND_VOLUME_MISMATCH: source E and target C'); }
    export async function onProgress() { return () => {}; }
    export async function chooseFolder() { return null; }
    export async function windowAction() {}
    export async function changeInstallerLanguage() {}
  ` }));
  await page.goto('/?preview=launcher');
  await page.getByRole('button', { name: 'Подготовить игру', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('Выбери диск с игрой');
  await expect(page.getByRole('alert')).toContainText('Полное копирование игры не начиналось');
  await page.screenshot({ path: 'temp/ui/stand-volume-mismatch.png' });
  await page.getByRole('button', { name: 'Выбрать папку стенда', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.locator('input[name="standRoot"]')).toBeFocused();
});
test('launcher matches the selected image-led layout and settings are keyboard accessible', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', reason => errors.push(reason.message));
  await page.goto('/?preview=launcher');
  await expect(page.getByRole('button', { name: 'Set up game', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Updates', exact: true })).toBeVisible();
  await page.screenshot({ path: 'temp/ui/launcher-en.png' });
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByLabel('Anomaly folder', { exact: true })).toBeVisible();
  await page.screenshot({ path: 'temp/ui/settings-en.png' });
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(errors).toEqual([]);
});
test('installer defaults to English, switches all visible copy to Russian and keeps folder choice', async ({ page }) => {
  await page.goto('/?preview=installer');
  await expect(page.getByRole('heading', { name: 'Your way into the Zone.' })).toBeVisible();
  await page.screenshot({ path: 'temp/ui/installer-en.png' });
  await page.getByRole('button', { name: /Русский/ }).click();
  await expect(page.getByRole('heading', { name: 'Твой путь в Зону.' })).toBeVisible();
  await page.getByRole('button', { name: 'Продолжить', exact: true }).click();
  await expect(page.getByLabel('Папка установки', { exact: true })).toHaveValue('D:\\RayLink');
  await page.getByLabel('Папка установки', { exact: true }).fill('E:\\Games\\RayLink');
  await page.screenshot({ path: 'temp/ui/installer-location-ru.png' });
  await page.getByRole('button', { name: 'Назад', exact: true }).click();
  await page.getByRole('button', { name: 'Продолжить', exact: true }).click();
  await expect(page.getByLabel('Папка установки', { exact: true })).toHaveValue('E:\\Games\\RayLink');
});
test('Russian launcher and narrow window remain usable', async ({ page }) => {
  await page.goto('/?preview=launcher&lang=ru');
  await expect(page.getByRole('button', { name: 'Настроить игру', exact: true })).toBeVisible();
  await page.screenshot({ path: 'temp/ui/launcher-ru.png' });
  await page.setViewportSize({ width: 960, height: 640 });
  await page.getByRole('button', { name: 'Настройки', exact: true }).first().click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await dialog.getByLabel('Язык', { exact: true }).selectOption('en');
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await page.screenshot({ path: 'temp/ui/launcher-960.png' });
});
test('OS reduced-motion preference disables expressive motion', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/?preview=installer');
  const duration = await page.locator('.installer-copy').evaluate(element => getComputedStyle(element).animationDuration);
  expect(parseFloat(duration)).toBeLessThan(0.01);
});
test('uninstaller keeps preferences by default and exposes the deletion choice in both languages', async ({ page }) => {
  await page.goto('/?preview=uninstaller');
  await expect(page.getByRole('heading', { name: 'Leaving the Zone?' })).toBeVisible();
  const checkbox = page.getByRole('checkbox', { name: /Delete launcher settings/ });
  await expect(checkbox).not.toBeChecked();
  await checkbox.check();
  await expect(checkbox).toBeChecked();
  await page.screenshot({ path: 'temp/ui/uninstaller-en.png' });
  await page.goto('/?preview=uninstaller&lang=ru');
  await expect(page.getByRole('heading', { name: 'Покидаешь Зону?' })).toBeVisible();
  await expect(page.getByRole('checkbox', { name: /Удалить настройки лаунчера/ })).not.toBeChecked();
  await page.screenshot({ path: 'temp/ui/uninstaller-ru.png' });
});
