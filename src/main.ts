import '@fontsource-variable/manrope';
import './style.css';
import { command, snapshot, onProgress, chooseFolder, windowAction, native, preview, changeInstallerLanguage } from './api';
import { translate, type MessageKey } from './i18n';
import { defaultSettings, type Snapshot, type Settings, type Progress, type LauncherUpdate, type Release } from './contract';
import { primaryAction, progressPercent } from './state';

const root = document.querySelector<HTMLDivElement>('#app');
if (!root) throw new Error('Application root is missing');
const app = root;
let state: Snapshot;
let panel: 'none' | 'settings' | 'updates' | 'setup' = 'none';
let draft: Settings = { ...defaultSettings };
let busy = false;
let progress: Progress | null = null;
let error = '';
let toast = '';
let toastTimer: ReturnType<typeof setTimeout> | undefined;
let installStep = 0;
let installPath = '';
let installedPath = '';
let desktop = true;
let pendingUpdate: LauncherUpdate | null = null;
let profiles: string[] = [];
let focusedBeforePanel: HTMLElement | null = null;
let unlisten: (() => void) | undefined;
let actionAbort = false;
const svg = (body: string) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;
const icons = {
  play: svg('<path fill="currentColor" stroke="none" d="M8 5.5v13l11-6.5z"/>'),
  settings: svg('<path d="m10 3-.7 2.2-2 .9-2.2-.5-2 3.4 1.5 1.7v2.6L3.1 15l2 3.4 2.2-.5 2 .9.7 2.2h4l.7-2.2 2-.9 2.2.5 2-3.4-1.5-1.7v-2.6L20.9 9l-2-3.4-2.2.5-2-.9L14 3z"/><circle cx="12" cy="12" r="3"/>'),
  arrow: svg('<path d="M5 12h14m-6-6 6 6-6 6"/>'),
  back: svg('<path d="M19 12H5m6-6-6 6 6 6"/>'),
  close: svg('<path d="m6 6 12 12M6 18 18 6"/>'),
  minus: svg('<path d="M5 12h14"/>'),
  chevron: svg('<path d="m7 10 5 5 5-5"/>'),
  folder: svg('<path d="M3 7V5h6l2 2h10v12H3z"/>'),
  refresh: svg('<path d="M20 7v5h-5M4 17v-5h5"/><path d="M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1"/>'),
  check: svg('<path d="m5 12 4 4L19 6"/>'),
  file: svg('<path d="M6 3h8l4 4v14H6zM14 3v5h4M9 12h6m-6 4h6"/>'),
  globe: svg('<circle cx="12" cy="12" r="9"/><ellipse cx="12" cy="12" rx="4" ry="9"/><path d="M3 12h18"/>'),
};
function t(key: MessageKey): string { return translate(state?.settings.language ?? 'en', key); }
function e(value: unknown): string { return String(value ?? '').replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char] ?? char); }
function gameName(): string { return state.settings.game === 'gamma' ? 'G.A.M.M.A.' : 'Anomaly'; }
function buildLabel(tag: string): string { return tag ? `${t('build')} ${tag.replace(/^b(?=\d)/, '')}` : t('noBuild'); }
function flash(key: MessageKey): void { toast = t(key); clearTimeout(toastTimer); renderNotifications(); toastTimer = setTimeout(() => { toast = ''; renderNotifications(); }, 4500); }
function phaseLabel(phase: string): string {
  if (phase.startsWith('install')) return t(phase.toLowerCase().includes('complete') ? 'complete' : phase.includes('verify') ? 'verifying' : 'installing');
  if (phase.startsWith('stand_')) return t(phase === 'stand_complete' ? 'complete' : phase === 'stand_verify' ? 'verifying' : phase === 'stand_scan' ? 'scanning' : 'materializing');
  if (phase.startsWith('launcher_')) return t('updateLauncher');
  const phases: Record<string, MessageKey> = { download: 'downloading', downloading: 'downloading', verify: 'verifying', verifying: 'verifying', scan: 'scanning', scanning: 'scanning', build: 'materializing', materialize: 'materializing', materializing: 'materializing', install: 'installing', installing: 'installing', complete: 'complete', update: 'updateLauncher' };
  return t(phases[phase] ?? 'progressUnknown');
}
function chrome(): string {
  return `<header class="titlebar"><button class="brand" data-action="home" aria-label="RayLink"><img src="./assets/launcher-symbol.png" alt=""><span>RayLink</span></button><div class="drag-space" data-action="drag"></div><div class="window-actions"><button data-action="minimize" aria-label="${t('minimize')}">${icons.minus}</button><button data-action="close" aria-label="${t('close')}">${icons.close}</button></div></header>`;
}
function progressMarkup(): string {
  if (!progress) return '';
  const percent = progressPercent(progress);
  return `<div class="job" role="status" aria-live="polite"><div class="job-top"><span class="spinner"></span><strong>${e(phaseLabel(progress.phase))}</strong><span class="job-percent">${percent === null ? '' : `${Math.round(percent)}%`}</span>${progress.cancellable ? `<button class="text-button" data-action="cancel">${t('cancel')}</button>` : ''}</div><div class="progress-track ${percent === null ? 'indeterminate' : ''}" role="progressbar" aria-label="${e(phaseLabel(progress.phase))}" ${percent === null ? '' : `aria-valuenow="${Math.round(percent)}" aria-valuemin="0" aria-valuemax="100"`}><span style="width:${percent ?? 25}%"></span></div><small>${percent === null ? t('progressUnknown') : `${formatCount(progress.current)} / ${formatCount(progress.total)}`}</small></div>`;
}
function formatCount(value: number): string { if (progress?.phase.includes('download')) return `${(value / 1048576).toFixed(1)} MB`; return new Intl.NumberFormat(state.settings.language).format(value); }
function launcher(): string {
  const action = primaryAction(state, busy);
  const key: MessageKey = action === 'play' ? 'play' : action === 'prepare' ? 'prepare' : action === 'setup' ? 'setup' : action === 'running' ? 'running' : 'busy';
  return `${chrome()}<nav class="top-nav" aria-label="${t('game')}"><div class="game-switch"><button data-game="anomaly" class="${state.settings.game === 'anomaly' ? 'selected' : ''}" ${busy ? 'disabled' : ''}>Anomaly</button><button data-game="gamma" class="${state.settings.game === 'gamma' ? 'selected' : ''}" ${busy ? 'disabled' : ''}>GAMMA</button><span class="game-underline ${state.settings.game === 'gamma' ? 'at-gamma' : ''}"></span></div><div class="top-links"><button data-panel="updates" class="${panel === 'updates' ? 'active' : ''}">${t('updates')}</button><span></span><button data-panel="settings" class="${panel === 'settings' ? 'active' : ''}">${t('settings')}</button></div></nav><main class="landscape" aria-label="${e(gameName())}"><div class="first-run-note ${!state.settings.anomalyDir && panel === 'none' ? 'visible' : ''}"><span class="tiny-mark"></span><p>${t('firstRunBody')}</p></div></main><footer class="play-dock"><div class="game-status"><h1>${e(gameName())}</h1><p><span class="status-dot ${state.standReady ? 'ready' : ''}"></span>${e(buildLabel(state.settings.installed))}<span class="dot-separator">·</span><span>${state.running ? t('running') : state.standReady ? t('ready') : t('notReady')}</span></p></div><button class="news-toggle" data-panel="updates">${t('whatsNew')}${icons.chevron}</button><div class="play-actions"><button class="primary magnetic" data-action="primary" ${(action === 'busy' || action === 'running') ? 'disabled' : ''}>${busy ? '<span class="spinner dark"></span>' : icons.play}<span>${t(key)}</span><i class="button-shine"></i></button><button class="settings-button" data-panel="settings" aria-label="${t('settings')}">${icons.settings}</button></div></footer><div id="job-slot" class="job-floating">${progressMarkup()}</div>${panelMarkup()}`;
}
function field(key: keyof Settings, label: MessageKey, hint?: MessageKey, browse = false): string {
  return `<label class="field"><span>${t(label)}</span><div class="input-row"><input aria-label="${t(label)}" name="${key}" value="${e(draft[key])}" autocomplete="off" spellcheck="false" ${key === 'nick' ? 'maxlength="24"' : ''}>${browse ? `<button type="button" class="browse" data-browse="${key}" aria-label="${t('choose')}">${icons.folder}</button>` : ''}</div>${hint ? `<small>${t(hint)}</small>` : ''}</label>`;
}
function folderFields(): string {
  return `${draft.game === 'gamma' ? `${field('mo2Dir', 'gammaPath', 'gammaHint', true)}<label class="field"><span>${t('profile')}</span><select aria-label="${t('profile')}" name="profile" ${profiles.length ? '' : 'disabled'}>${profiles.length ? profiles.map(profile => `<option value="${e(profile)}" ${profile === draft.profile ? 'selected' : ''}>${e(profile)}</option>`).join('') : `<option>${t('noProfile')}</option>`}</select></label>` : ''}${field('anomalyDir', 'anomalyPath', 'anomalyHint', true)}`;
}
function panelMarkup(): string {
  if (panel === 'none') return '';
  const title = panel === 'updates' ? t('whatsNew') : panel === 'setup' ? t('firstRun') : t('settings');
  const content = panel === 'updates' ? updatesContent() : `<form id="settings-form"><div class="sheet-intro">${panel === 'setup' ? t('firstRunBody') : t('safety')}</div>${panel === 'setup' ? `<div class="choice-row"><button type="button" data-draft-game="anomaly" class="${draft.game === 'anomaly' ? 'chosen' : ''}">Anomaly</button><button type="button" data-draft-game="gamma" class="${draft.game === 'gamma' ? 'chosen' : ''}">G.A.M.M.A.</button></div>` : ''}<section class="form-section"><h3>${t('paths')}</h3>${folderFields()}</section><section class="form-section"><h3>${t('general')}</h3>${field('nick', 'nickname', 'nicknameHint')}<label class="field"><span>${t('language')}</span><select aria-label="${t('language')}" name="language"><option value="en" ${draft.language === 'en' ? 'selected' : ''}>English</option><option value="ru" ${draft.language === 'ru' ? 'selected' : ''}>Русский</option></select></label><label class="check-field"><input type="checkbox" name="reducedMotion" ${draft.reducedMotion ? 'checked' : ''}><span>${t('motion')}<small>${t('motionHint')}</small></span></label></section>${panel === 'settings' ? `<section class="form-section"><h3>${t('storage')}</h3><div class="location-readout"><span>${t('home')}</span><code>${e(state.homeDir)}</code></div>${field('standRoot', 'stand', 'standHint', true)}${field('repo', 'channel', 'channelHint')}<div class="maintenance"><button type="button" data-action="rebuild" ${busy || state.running ? 'disabled' : ''}>${icons.refresh}${t('rebuild')}</button><button type="button" data-action="verify" ${busy ? 'disabled' : ''}>${icons.check}${t('verify')}</button><button type="button" data-action="logs">${icons.file}${t('logs')}</button><button type="button" data-action="open-home">${icons.folder}${t('openHome')}</button></div></section>` : ''}<div class="sheet-bottom"><button type="button" class="secondary" data-action="dismiss">${t('cancel')}</button><button class="primary" type="submit" ${busy ? 'disabled' : ''}>${t('save')}${icons.check}</button></div></form>`;
  return `<div class="scrim" data-action="dismiss"></div><aside class="sheet" role="dialog" aria-modal="true" aria-labelledby="sheet-title"><div class="sheet-heading"><div><span class="sheet-brand">RayLink</span><h2 id="sheet-title">${title}</h2></div><button class="icon-button" data-action="dismiss" aria-label="${t('close')}">${icons.close}</button></div><div class="sheet-scroll">${content}</div></aside>`;
}
function updatesContent(): string {
  const release = state.release;
  return `<div class="release-head"><span class="release-label">${t('latestBuild')}</span><h3>${e(release?.name || buildLabel(state.settings.installed))}</h3>${release?.publishedAt ? `<time>${e(new Date(release.publishedAt).toLocaleDateString(state.settings.language, { year: 'numeric', month: 'long', day: 'numeric' }))}</time>` : ''}</div><div class="release-notes">${e(release?.body || t('noNotes'))}</div><div class="update-actions"><button class="secondary" data-action="check" ${busy ? 'disabled' : ''}>${icons.refresh}${t('check')}</button>${release && release.tag !== state.settings.installed ? `<button class="primary" data-action="install-build" ${busy ? 'disabled' : ''}>${t('installBuild')}${icons.arrow}</button><small>${t('downloadSize')}: ${(release.assetSize / 1048576).toFixed(1)} MB</small>` : ''}</div><div class="launcher-version"><span>${t('version')} ${e(state.launcherVersion)}</span><button class="text-button" data-action="check-launcher">${t('check')}</button></div>`;
}
function installer(): string {
  const heading = installStep === 0 ? t('installTitle') : installStep === 1 ? t('installFolderTitle') : t('installReady');
  const body = installStep === 0 ? t('installBody') : installStep === 1 ? t('installFolderBody') : t('installDone');
  return `${chrome()}<main class="installer-content"><div class="installer-copy" key="${installStep}"><div class="install-eyebrow"><span class="status-dot ready"></span>${t('installWelcome')}</div><h1>${heading}</h1><p class="install-description">${body}</p>${installStep === 0 ? `<div class="language-picker" aria-label="${t('installLanguage')}"><span>${icons.globe}${t('installLanguage')}</span><div class="language-options"><button data-language="en" class="${state.settings.language === 'en' ? 'chosen' : ''}"><strong>English</strong><small>English</small>${state.settings.language === 'en' ? icons.check : ''}</button><button data-language="ru" class="${state.settings.language === 'ru' ? 'chosen' : ''}"><strong>Русский</strong><small>Russian</small>${state.settings.language === 'ru' ? icons.check : ''}</button></div></div>` : installStep === 1 ? `<label class="field install-location"><span>${t('installDestination')}</span><div class="input-row"><input id="install-path" value="${e(installPath)}" autocomplete="off" spellcheck="false" ${busy ? 'disabled' : ''}><button class="browse" data-action="browse-install" ${busy ? 'disabled' : ''} aria-label="${t('choose')}">${icons.folder}</button></div></label><label class="check-field"><input id="desktop-shortcut" type="checkbox" ${desktop ? 'checked' : ''} ${busy ? 'disabled' : ''}><span>${t('desktopShortcut')}<small>${t('startMenuShortcut')}</small></span></label><div id="job-slot">${progressMarkup()}</div>` : `<div class="install-success"><span>${icons.check}</span><code>${e(installedPath || installPath)}</code></div>`}<div class="install-actions">${installStep === 1 ? `<button class="secondary" data-action="install-back" ${busy ? 'disabled' : ''}>${icons.back}${t('back')}</button>` : ''}<button class="primary magnetic" data-action="${installStep === 0 ? 'install-next' : installStep === 1 ? 'install-app' : 'launch-installed'}" ${busy ? 'disabled' : ''}>${busy ? '<span class="spinner dark"></span>' : ''}<span>${installStep === 0 ? t('next') : installStep === 1 ? t('installButton') : t('launchApp')}</span>${icons.arrow}<i class="button-shine"></i></button></div></div><div class="installer-brand-caption"><span>RayLink</span><p>${t('announcement')}</p></div></main><footer class="installer-footer"><div class="install-steps">${(['installStepLanguage', 'installStepFolder', 'installStepFinish'] as const).map((key, index) => `<span class="${index === installStep ? 'current' : index < installStep ? 'done' : ''}"><i>${index < installStep ? icons.check : index + 1}</i>${t(key)}</span>`).join('')}</div><span class="version-tag">${e(state.launcherVersion)}</span></footer>`;
}
function render(): void {
  if (!state) return;
  document.documentElement.lang = state.settings.language;
  document.documentElement.classList.toggle('reduce-motion', state.settings.reducedMotion);
  app.className = `${state.installer ? 'installer-app' : 'launcher-app'} ${panel !== 'none' ? 'panel-open' : ''}`;
  app.innerHTML = `<div class="background-photo"></div><div class="image-shade"></div>${preview ? `<div class="preview-label">${t('preview')}</div>` : ''}${state.installer ? installer() : launcher()}<div id="notification-slot"></div>`;
  renderNotifications();
  if (panel !== 'none') app.querySelector<HTMLElement>('.sheet button')?.focus({ preventScroll: true });
}
function renderNotifications(): void {
  const slot = document.querySelector('#notification-slot');
  if (!slot) return;
  slot.innerHTML = `${toast ? `<div class="toast" role="status">${icons.check}<span>${e(toast)}</span></div>` : ''}${error ? `<div class="error-card" role="alert"><div><strong>${t('operationFailed')}</strong><button data-action="clear-error" class="icon-button" aria-label="${t('close')}">${icons.close}</button></div><p>${t('retryHint')}</p><details><summary>${t('details')}</summary><pre>${e(error)}</pre></details></div>` : ''}${pendingUpdate && !busy ? `<div class="update-notice"><strong>${t('launcherAvailable')} ${e(pendingUpdate.version)}</strong><button class="primary" data-action="apply-update">${t('restart')}</button><button class="text-button" data-action="later-update">${t('later')}</button></div>` : ''}`;
}
async function refresh(): Promise<void> { state = await snapshot(); }
async function operate(work: () => Promise<void>): Promise<void> {
  if (busy) return;
  busy = true; error = ''; progress = null; actionAbort = false; render();
  try { await work(); } catch (reason) { const message = reason instanceof Error ? reason.message : String(reason); if (!actionAbort || message !== 'Operation cancelled') error = message; }
  finally { busy = false; progress = null; render(); }
}
function readDraft(): void {
  const form = document.querySelector<HTMLFormElement>('#settings-form');
  if (!form) return;
  const fields = new FormData(form);
  for (const key of ['anomalyDir', 'mo2Dir', 'profile', 'standRoot', 'nick', 'repo'] as const) { const value = fields.get(key); if (typeof value === 'string') draft[key] = value.trim(); }
  const lang = fields.get('language'); if (lang === 'en' || lang === 'ru') draft.language = lang;
  draft.reducedMotion = fields.get('reducedMotion') === 'on';
}
async function refreshProfiles(): Promise<void> {
  if (!draft.mo2Dir || !native) { profiles = []; return; }
  const result = await command<{ profiles: string[]; anomalyDir: string | null; selected: string | null }>('inspect_mo2', { path: draft.mo2Dir });
  profiles = result.profiles;
  if (!profiles.includes(draft.profile)) draft.profile = result.selected && profiles.includes(result.selected) ? result.selected : profiles[0] ?? '';
  if (!draft.anomalyDir && result.anomalyDir) draft.anomalyDir = result.anomalyDir;
}
async function openPanel(value: typeof panel): Promise<void> {
  if (value === panel) { dismiss(); return; }
  focusedBeforePanel = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  if (panel === 'settings' || panel === 'setup') readDraft();
  panel = value; draft = { ...state.settings }; profiles = state.profiles; render();
  if (value === 'updates' && !state.release && native) await checkRelease();
}
function dismiss(): void { panel = 'none'; render(); focusedBeforePanel?.focus({ preventScroll: true }); }
async function checkRelease(): Promise<void> {
  await operate(async () => { await command<Release>('check_build'); await refresh(); if (state.release?.tag === state.settings.installed) flash('noUpdate'); });
}
async function checkLauncher(auto: boolean): Promise<void> {
  try { pendingUpdate = await command<LauncherUpdate | null>('check_launcher_update'); if (pendingUpdate && auto) await applyUpdate(); else if (pendingUpdate) renderNotifications(); else if (!auto) flash('noUpdate'); }
  catch (reason) { if (!auto) { error = String(reason); renderNotifications(); } }
}
async function applyUpdate(): Promise<void> { await operate(async () => { await command('apply_launcher_update'); }); }

app.addEventListener('submit', event => { event.preventDefault(); if (!(event.target instanceof HTMLFormElement) || event.target.id !== 'settings-form') return; readDraft(); void operate(async () => { state = await command<Snapshot>('save_settings', { settings: draft }); panel = 'none'; flash('saved'); }); });
app.addEventListener('input', event => { const input = event.target; if (!(input instanceof HTMLInputElement)) return; if (input.id === 'install-path') installPath = input.value; if (input.id === 'desktop-shortcut') desktop = input.checked; });
app.addEventListener('change', event => {
  const target = event.target;
  if (!(target instanceof HTMLInputElement || target instanceof HTMLSelectElement)) return;
  if (target.name === 'mo2Dir') { readDraft(); void refreshProfiles().then(render).catch(reason => { error = String(reason); renderNotifications(); }); }
  if (target.name === 'language') { readDraft(); state.settings.language = draft.language; render(); }
});
app.addEventListener('click', event => {
  const element = event.target instanceof Element ? event.target.closest<HTMLElement>('button,[data-action]') : null;
  if (!element || element.hasAttribute('disabled')) return;
  void handleClick(element).catch(reason => { error = String(reason); renderNotifications(); });
});
async function handleClick(element: HTMLElement): Promise<void> {
  if (element.dataset.panel) { await openPanel(element.dataset.panel as 'settings' | 'updates'); return; }
  if (element.dataset.game) {
    if (busy || state.running || element.dataset.game === state.settings.game) return;
    const game = element.dataset.game === 'anomaly' ? 'anomaly' : 'gamma';
    await operate(async () => { state = await command<Snapshot>('save_settings', { settings: { ...state.settings, game } }); });
    if (primaryAction(state, false) === 'setup') await openPanel('setup'); return;
  }
  if (element.dataset.draftGame) { readDraft(); draft.game = element.dataset.draftGame === 'gamma' ? 'gamma' : 'anomaly'; render(); return; }
  if (element.dataset.language) { state.settings.language = element.dataset.language === 'ru' ? 'ru' : 'en'; await changeInstallerLanguage(state.settings.language); render(); return; }
  if (element.dataset.browse) {
    readDraft(); const key = element.dataset.browse as 'anomalyDir' | 'mo2Dir' | 'standRoot';
    const selected = await chooseFolder(t(key === 'anomalyDir' ? 'anomalyPath' : key === 'mo2Dir' ? 'gammaPath' : 'stand'));
    if (selected) { draft[key] = selected; if (key === 'mo2Dir') await refreshProfiles(); render(); } return;
  }
  switch (element.dataset.action) {
    case 'home': if (!state.installer) dismiss(); break;
    case 'close': if (!busy) await windowAction('close'); break;
    case 'minimize': await windowAction('minimize'); break;
    case 'dismiss': dismiss(); break;
    case 'clear-error': error = ''; renderNotifications(); break;
    case 'check': await checkRelease(); break;
    case 'primary': {
      const action = primaryAction(state, busy);
      if (action === 'setup') await openPanel('setup');
      else if (action === 'prepare' || action === 'play') await operate(async () => { state = await command<Snapshot>(action === 'play' ? 'launch_game' : 'prepare_game'); });
      break;
    }
    case 'install-build': await operate(async () => { state = await command<Snapshot>('install_build'); flash('buildInstalled'); }); break;
    case 'rebuild': await operate(async () => { state = await command<Snapshot>('prepare_game'); }); break;
    case 'verify': await operate(async () => { await command('verify_build'); flash('verificationDone'); }); break;
    case 'logs': await command('open_folder', { kind: 'logs' }); break;
    case 'open-home': await command('open_folder', { kind: 'home' }); break;
    case 'cancel': actionAbort = true; await command('cancel_job'); flash('cancelRequested'); break;
    case 'check-launcher': await checkLauncher(false); break;
    case 'apply-update': await applyUpdate(); break;
    case 'later-update': pendingUpdate = null; renderNotifications(); break;
    case 'install-next': installStep = 1; render(); break;
    case 'install-back': if (!busy) { installStep = 0; render(); } break;
    case 'browse-install': { const selected = await chooseFolder(t('installDestination')); if (selected) { installPath = selected; render(); } break; }
    case 'install-app': if (!installPath.trim()) { error = t('folderRequired'); renderNotifications(); } else await operate(async () => { installedPath = await command<string>('install_app', { destination: installPath.trim(), language: state.settings.language, desktop }); installStep = 2; }); break;
    case 'launch-installed': await command('open_installed'); await windowAction('close'); break;
  }
}
app.addEventListener('pointerdown', event => { if (event.button === 0 && event.target instanceof HTMLElement && event.target.dataset.action === 'drag') void windowAction('drag'); });
let framePending = false;
let lastPointer = { x: 0, y: 0, target: null as HTMLElement | null };
app.addEventListener('pointermove', event => {
  if (state?.settings.reducedMotion || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  const target = event.target instanceof Element ? event.target.closest<HTMLElement>('.magnetic') : null;
  if (!target) return;
  lastPointer = { x: event.clientX, y: event.clientY, target };
  if (framePending) return;
  framePending = true;
  requestAnimationFrame(() => { framePending = false; const { target: button, x, y } = lastPointer; if (!button?.isConnected) return; const rect = button.getBoundingClientRect(); button.style.setProperty('--pointer-x', `${x - rect.left}px`); button.style.setProperty('--pointer-y', `${y - rect.top}px`); });
});
document.addEventListener('keydown', event => {
  if (event.key === 'Escape' && panel !== 'none') { event.preventDefault(); dismiss(); }
  if (event.key === 'Tab' && panel !== 'none') {
    const nodes = [...document.querySelectorAll<HTMLElement>('.sheet button:not(:disabled),.sheet input:not(:disabled),.sheet select:not(:disabled),.sheet summary')];
    const first = nodes[0], last = nodes.at(-1);
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
  }
});
window.addEventListener('beforeunload', () => unlisten?.());
async function boot(): Promise<void> {
  state = await snapshot();
  if (state.installer && native) installPath = await command<string>('default_install_dir');
  if (state.installer && preview) installPath = 'D:\\RayLink';
  render();
  unlisten = await onProgress(value => {
    progress = value;
    const slot = document.querySelector('#job-slot');
    if (!slot) return;
    if (!slot.querySelector('.job')) { slot.innerHTML = progressMarkup(); return; }
    const percent = progressPercent(value);
    const title = slot.querySelector('.job-top strong'); if (title) title.textContent = phaseLabel(value.phase);
    const percentage = slot.querySelector('.job-percent'); if (percentage) percentage.textContent = percent === null ? '' : `${Math.round(percent)}%`;
    const track = slot.querySelector<HTMLElement>('.progress-track');
    if (track) { track.classList.toggle('indeterminate', percent === null); track.setAttribute('aria-label', phaseLabel(value.phase)); if (percent === null) track.removeAttribute('aria-valuenow'); else track.setAttribute('aria-valuenow', String(Math.round(percent))); }
    const fill = slot.querySelector<HTMLElement>('.progress-track > span'); if (fill) fill.style.width = `${percent ?? 25}%`;
    const details = slot.querySelector('.job > small'); if (details) details.textContent = percent === null ? t('progressUnknown') : `${formatCount(value.current)} / ${formatCount(value.total)}`;
    const cancel = slot.querySelector<HTMLElement>('[data-action="cancel"]'); if (cancel) cancel.hidden = !value.cancellable;
  });
  if (native && !state.installer) {
    void checkLauncher(true);
    void command<Release>('check_build').then(refresh).then(() => { if (!busy) render(); }).catch(() => { toast = t('offline'); renderNotifications(); });
    setInterval(() => { if (state.running && !busy && panel === 'none') void refresh().then(render).catch(() => undefined); }, 8000);
  }
}
void boot().catch(reason => { app.innerHTML = `<main class="fatal"><h1>RayLink</h1><p>${e(reason)}</p></main>`; });
