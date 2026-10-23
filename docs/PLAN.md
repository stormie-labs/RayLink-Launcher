# RayLink Launcher — план реализации v1

К спецификации `docs/SPEC.md`, 2026-10-23 23:50. Ссылки вида «SPEC §3.7» ведут туда.

## 0. Исходное состояние и правила

**Окружение (проверено 2026-10-23 23:24):**
- репозиторий `raylink-launcher` — git, один коммит `7caa29f` (дизайн), без remote;
- Node 24.13, npm; **Rust 1.99.1** (rustup поставлен ведущей в 23:22, `%USERPROFILE%\.cargo\bin` — в уже открытых
  оболочках вызывать полным путём); WebView2 155; Windows SDK 10.0.26100;
- `gh` 2.101 не залогинен.

**Правила для всех исполнителей:**
- **TDD.** Сначала тест из списка задачи → запуск → тест **падает по нужной причине** → код → зелёный → линтеры.
  В отчёте — команды и их итог.
- **Строгая типизация.** TS: `strict`, `noUncheckedIndexedAccess`, без `any`, `@ts-ignore`, `@ts-expect-error`.
  Rust: `clippy -D warnings`, без `unwrap()` вне тестов.
- **Владение.** У каждой задачи свои файлы (§1). Чужие файлы не правятся — нужна правка контракта → запрос ведущей.
  Параллельная работа — в отдельных worktree и ветках (`git worktree add ../qn-wt-<задача> -b feat/<задача>`).
  Вливает ведущая.
- **Временные файлы** — только в `temp/` проекта.
- Загрузки на GitHub, `gh release create`, `-Upload` — **только по слову владельца**.

**Команды проверки:**

| Что | Команда |
|---|---|
| Фронтенд | `npm run check` (svelte-check + `tsc --noEmit`) · `npm run lint` · `npm run test` (Vitest) · `npm run e2e` (Playwright по Vite с `?mock=`) |
| Rust | `cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace` (из `src-tauri/`) |
| anomaly-coop | `python -m pytest tests/tools/test_publish.py -q` · `tools\build_tests.cmd` → `temp\tests\coop_tests.exe` |

## 1. Структура репозитория и владение

```
raylink-launcher/
  package.json, package-lock.json, tsconfig*.json, vite.config.ts, svelte.config.js,
  vitest.config.ts, playwright.config.ts, eslint.config.js, index.html            T1
  contracts/
    manifest.schema.json                                                         T1 (правки — T6 по согласованию)
    fixtures/*.json        образцы всех DTO команд и событий                     T1, дополняют владельцы типов
    fixtures/manifest/*.json   валидные и вредные манифесты                       T6
    fixtures/{addresses,nicks,relpaths,launch_args}.json   общие векторы TS/Rust  T1 (relpaths — T6)
    probe/vectors.json     байты QNP v1                                          T1 (сверяет T9, копирует E1)
  src/
    main.ts, App.svelte                                                          T1 (каркас), T2 (оболочка)
    lib/api/{contract.ts, schema.ts, backend.ts, tauri.ts, errors.ts}            T1 (tauri.ts финализирует T10)
    lib/api/mock/**        подменный бэкенд и сценарии                           T1
    styles/**, assets/**                                                         T2
    lib/ui/**              кнопки, сегменты, поля, диалог, тост, бейдж, штриховка T2
    lib/shell/**           заголовок, компас, фон, переключение экранов           T2
    lib/state/{nav,motion,bg}.ts                                                 T2
    lib/state/{dock,hostcheck,news}.ts, screens/home/**                          T3
    lib/state/{wizard,validators}.ts, screens/wizard/**, screens/settings/**     T4
    lib/state/{builds,logs}.ts, lib/i18n/errors.ts, screens/updates/**, screens/help/**   T5
    lib/i18n/ru.ts         общие строки                                          T2
  tests/unit/**            по задачам: <модуль>.test.ts                          владельцы модулей
  tests/e2e/**             по экранам: shell (T2), home (T3), wizard/settings (T4), updates/help/errors (T5)
  src-tauri/
    Cargo.toml (workspace), crates/*/Cargo.toml (каркас и зависимости)           T1 (зависимости дополняют задачи, сводит ведущая)
    crates/qn-fs/**                                                              T7
    crates/qn-core/src/lib.rs (объявления модулей)                               T1
    crates/qn-core/src/{relpath,manifest,secret,redact}.rs, channel/**           T6
    crates/qn-core/src/{stand,gamma}/**                                          T7
    crates/qn-core/src/install/**                                                T8
    crates/qn-core/src/{probe,launch}/**                                         T9
    crates/qn-core/tests/**  по модулям                                          владельцы модулей
    crates/qn-tool/**      CLI: manifest check, sign, fakegame (для тестов)      T6 (manifest), T9 (fakegame), T11 (sign)
    src/**, build.rs, tauri.conf.json, capabilities/**, icons/**                 T10 (секции bundle/updater — T11)
    src/self_update.rs                                                           T11
  scripts/release-launcher.ps1, channel.default.json                             T11
  temp/                                                                          все (gitignore)
```

## 2. Интерфейсы между задачами

### 2.1 Команды Tauri
Все команды — `async`, результат `Result<T, AppError>`. JSON в camelCase (`#[serde(rename_all = "camelCase")]`).
Объединения — с тегом `kind`/`state`.
- Список команд закрепляется в `build.rs` через `tauri_build::AppManifest::commands(&[…])`.
- Разрешения — в `capabilities/default.json` явным списком.

| Команда | Аргументы | Результат | Делает (Rust) | Использует (UI) |
|---|---|---|---|---|
| `app_info` | — | `AppInfo` | T10 | T2, T5 |
| `status_get` | — | `LauncherStatus` | T10 (сводит T6–T9) | все экраны |
| `settings_get` | — | `Settings` | T10 | T4 |
| `settings_set` | `{ patch: SettingsPatch }` | `Settings` | T10 | T2 (анимация), T4 |
| `discover_installs` | — | `Discovery` | T10 (`discovery.rs`) | T4 |
| `paths_validate` | `{ input: PathsInput }` | `PathsReport` | T7 (логика), T10 (команда) | T4 |
| `stand_setup` | `{ input: StandSetup }` | `JobRef` | T7 + T8, T10 | T4 |
| `stand_rebuild_gamma` | — | `JobRef` | T7, T10 | T3, T4 |
| `stand_verify` | — | `JobRef` | T7 + T8, T10 | T5 |
| `stand_reset_game_settings` | — | `void` | T7, T10 | T4 |
| `gamma_check` | `{ deep: boolean }` | `GammaState` | T7, T10 | T3, T4 |
| `channel_set` | `{ repo: string; flavor: ChannelFlavor }` | `ChannelHealth` | T6, T10 | T4 |
| `channel_set_token` | `{ token: string }` | `ChannelHealth` | T6, T10 | T4 |
| `channel_clear_token` | — | `ChannelHealth` | T6, T10 | T4 |
| `update_check` | — | `UpdateInfo` | T6, T10 | T3, T5 |
| `update_start` | `{ build: BuildNo \| null }` | `JobRef` | T8, T10 | T3, T5 |
| `job_cancel` | `{ jobId: string }` | `void` | T10 | T3, T5 |
| `builds_list` | — | `BuildEntry[]` | T6 + T8, T10 | T5 |
| `rollback_start` | `{ build: BuildNo }` | `JobRef` | T8, T10 | T3, T5 |
| `pin_set` | `{ build: BuildNo \| null }` | `Settings` | T10 | T5 |
| `host_probe` | `{ address: string }` | `ProbeResult` | T9, T10 | T3 |
| `local_addresses` | — | `LocalAddress[]` | T9, T10 | T3 |
| `public_address` | — | `string` | T9, T10 (только по кнопке) | T3 |
| `game_launch` | `{ req: LaunchRequest }` | `GameSession` | T9, T10 | T3 |
| `game_focus` | `{ sessionId: string }` | `void` | T9, T10 | T3 |
| `game_close` | `{ sessionId: string; force: boolean }` | `void` | T9, T10 | T3 |
| `logs_preview` | — | `LogPreview` | T10 | T5 |
| `logs_export` | `{ target: LogTarget }` | `LogBundle` | T10 | T5 |
| `open_folder` | `{ which: FolderKind }` | `void` | T10 | T2 (подвал), T5 |
| `motion_system_reduced` | — | `boolean` | T10 | T2 |
| `launcher_update_check` | — | `LauncherSelfUpdate` | T11 | T4, T5 |
| `launcher_update_install` | — | `void` | T11 | T4, T5 |

Окно (свернуть, закрыть, перетаскивание) — штатный API `@tauri-apps/api/window` за абстракцией
`backend.window`; в моке — счётчики вызовов.

### 2.2 События
| Событие | Полезная нагрузка | Когда | Кто шлёт |
|---|---|---|---|
| `qn://status` | `LauncherStatus` | При любом изменении снимка: найдена сборка, GAMMA изменилась, игра вышла, сменился канал | T10 |
| `qn://job` | `JobEvent` | Прогресс задачи, не чаще 10/с; первое и последнее событие — всегда | T10 (из `Progress` T7/T8/T6) |
| `qn://game` | `GameEvent` | `starting` / `running` / `exited` | T10 (из T9) |

### 2.3 Типы — `src/lib/api/contract.ts` (источник истины, T1)
Rust повторяет их в `qn-core::dto` с `serde`; совпадение проверяют общие фикстуры (§2.5).

```ts
export type BuildNo = number;      // целое ≥ 1
export type Sha256Hex = string;    // 64 hex, нижний регистр
export type IsoTime = string;      // RFC 3339

export const ERROR_CODES = [
  'CHANNEL_OFFLINE', 'CHANNEL_AUTH', 'CHANNEL_NOT_FOUND', 'CHANNEL_RATE_LIMITED', 'CHANNEL_EMPTY',
  'MANIFEST_INVALID', 'MANIFEST_SIGNATURE', 'LAUNCHER_TOO_OLD',
  'DOWNLOAD_FAILED', 'HASH_MISMATCH', 'ARCHIVE_UNSAFE',
  'DISK_FULL', 'NOT_NTFS', 'CROSS_VOLUME', 'LINK_FAILED', 'STAND_FOREIGN', 'STAND_OVERLAP', 'REPARSE_IN_STAND',
  'FILE_LOCKED', 'GAME_RUNNING', 'MO2_RUNNING',
  'ANOMALY_NOT_FOUND', 'MO2_NOT_FOUND', 'PROFILE_NOT_FOUND', 'NO_AVX',
  'TXN_RECOVERED', 'BUILD_NOT_CACHED',
  'ADDRESS_INVALID', 'PROBE_TIMEOUT', 'PROBE_UNRESOLVED', 'PROBE_BAD_REPLY',
  'GAME_START_FAILED', 'EXE_MISSING', 'TOKEN_STORE_FAILED', 'INTERNAL',
] as const;
export type ErrorCode = (typeof ERROR_CODES)[number];
export type ErrorAction =
  | 'retry' | 'openSettings' | 'enterToken' | 'closeGame' | 'updateLauncher' | 'chooseStand' | 'openHelp' | 'download';
export interface AppError {
  code: ErrorCode;
  message: string;          // по-русски, для игрока
  detail: string | null;    // техническое, для логов и «подробно»
  retryable: boolean;
  action: ErrorAction | null;
}

export type ChannelFlavor = 'stable' | 'fresh';
export type MotionMode = 'full' | 'calm' | 'off' | 'system';
export type Smap = 1024 | 1536 | 2048 | 2560 | 3072 | 4096;

export interface AppInfo { launcherVersion: string; defaultRepo: string | null; os: string; avx: boolean }

export interface Settings {
  schema: 1;
  paths: { anomaly: string | null; mo2: string | null; profile: string | null; stand: string | null };
  nick: string;
  channel: { repo: string; flavor: ChannelFlavor };   // ключа здесь нет никогда
  secondInstance: boolean;
  motion: MotionMode;
  autoDownload: boolean;
  installBeforePlay: 'always' | 'ask';
  keepBuilds: number;                                 // 2..10
  pinnedBuild: BuildNo | null;
  hostPort: number;                                   // 1..65535, по умолчанию 27960
  recentAddresses: string[];                          // ≤ 8
  launch: { smap: Smap; autostatusSec: number };      // autostatusSec 0 = выкл
}
export interface SettingsPatch {
  nick?: string; secondInstance?: boolean; motion?: MotionMode; autoDownload?: boolean;
  installBeforePlay?: 'always' | 'ask'; keepBuilds?: number; hostPort?: number;
  launch?: { smap?: Smap; autostatusSec?: number };
  recentAddresses?: string[];
}

export interface ChangeItem { kind: 'new' | 'fix' | 'change' | 'known'; text: string; hint: string | null }
export interface InstalledBuild {
  build: BuildNo; protocol: number; maxPlayers: number; publishedAt: IsoTime; installedAt: IsoTime;
  layerSha: Sha256Hex; changelog: ChangeItem[];
}
export interface ReleaseSummary {
  build: BuildNo; protocol: number; publishedAt: IsoTime; prerelease: boolean; downloadBytes: number;
  minLauncher: string; changelog: ChangeItem[]; protocolChange: boolean;
}
export type UpdateInfo =
  | { state: 'unknown' }
  | { state: 'upToDate'; checkedAt: IsoTime }
  | { state: 'available'; latest: ReleaseSummary; checkedAt: IsoTime }
  | { state: 'downloaded'; latest: ReleaseSummary }                // проверена, ждёт «ставлю»
  | { state: 'pinned'; pinned: BuildNo; latest: ReleaseSummary | null }
  | { state: 'launcherTooOld'; latest: ReleaseSummary; need: string };
export interface ChannelHealth {
  repo: string; flavor: ChannelFlavor;
  state: 'ok' | 'offline' | 'auth' | 'notFound' | 'rateLimited' | 'empty' | 'unset';
  hasToken: boolean; isPrivate: boolean | null; checkedAt: IsoTime | null; retryAt: IsoTime | null;
  error: AppError | null;
}
export interface StandState {
  path: string | null;
  state: 'none' | 'ok' | 'building' | 'txnPending' | 'broken';
  mode: 'gamma' | 'stock' | null;
  secondInstance: boolean; gammaBuiltAt: IsoTime | null; freeBytes: number | null; problems: AppError[];
}
export interface GammaChanges { added: string[]; removed: string[]; changed: string[]; orderChanged: boolean; filesChanged: number }
export interface GammaState {
  mode: 'gamma' | 'stock'; profile: string | null;
  state: 'same' | 'changed' | 'unknown' | 'sourceMissing';
  changedAt: IsoTime | null; modsCount: number; fingerprint16: string;
  lastQuickCheck: IsoTime | null; lastDeepCheck: IsoTime | null; changes: GammaChanges | null;
}
export type JobKind = 'standSetup' | 'update' | 'rollback' | 'gammaRebuild' | 'standVerify';
export type JobStep = 'download' | 'verify' | 'install' | 'scan' | 'link';
export interface JobEvent {
  jobId: string; kind: JobKind; targetBuild: BuildNo | null;
  steps: JobStep[]; step: JobStep; stepIndex: number;
  state: 'running' | 'waitingGame' | 'done' | 'failed' | 'cancelled';
  done: number; total: number; unit: 'bytes' | 'files';
  ratePerSec: number | null; etaSec: number | null;
  error: AppError | null; rolledBackTo: BuildNo | null;
}
export interface JobRef { jobId: string }
export interface GameSession {
  sessionId: string; instance: 'a' | 'b'; mode: 'host' | 'join'; pid: number; startedAt: IsoTime;
  state: 'starting' | 'running' | 'exited'; exitCode: number | null; crashed: boolean; ownedByLauncher: boolean;
}
export interface GameEvent { session: GameSession }
export interface LauncherSelfUpdate { current: string; state: 'none' | 'available' | 'downloading' | 'ready' | 'error'; available: string | null }
export interface LauncherStatus {
  setup: 'wizard' | 'ready';
  installed: InstalledBuild | null;
  update: UpdateInfo;
  channel: ChannelHealth;
  stand: StandState;
  gamma: GammaState;
  games: GameSession[];
  job: JobEvent | null;
  rollbackTo: BuildNo | null;
  launcher: LauncherSelfUpdate;
}
export interface BuildEntry {
  build: BuildNo; protocol: number; publishedAt: IsoTime; prerelease: boolean; changelog: ChangeItem[];
  where: Array<'installed' | 'cached' | 'remote'>; pinned: boolean; canRollback: boolean;
  commits: { engine: string[]; project: string[] };
}

export interface HostStand { build: BuildNo; layerSha12: string; mode: 'gamma' | 'stock'; gammaFp16: string; modsCount: number }
export interface HostInfo {
  protocol: number; buildId: string; contentFp: string; contentEntries: number; nick: string;
  players: number; maxPlayers: number; phase: 'loading' | 'live'; stand: HostStand | null;
}
export interface ModDiff { total: number; onlyHere: string[]; onlyHost: string[]; differ: string[] }
export interface ProbeVerdict {
  canPlay: boolean;
  protocol: 'same' | 'differs';
  build: 'same' | 'hostOlder' | 'hostNewer' | 'unknown';
  mods: 'same' | 'differs' | 'modeDiffers' | 'unknown';
  modDiff: ModDiff | null;
}
export type ProbeResult =
  | { kind: 'ok'; address: string; at: IsoTime; rttMs: number; host: HostInfo; verdict: ProbeVerdict }
  | { kind: 'error'; address: string; at: IsoTime; error: AppError };
export interface LocalAddress { ip: string; adapter: string; kind: 'lan' | 'vpn' | 'other'; vpnName: string | null }
export interface LaunchRequest { mode: 'host' | 'join'; address: string | null; instance: 'a' | 'b' }

export interface AnomalyCandidate { path: string; version: string | null; volume: string; evidence: string[] }
export interface Mo2Candidate {
  path: string; gamePath: string | null; profiles: string[]; selectedProfile: string | null; isGamma: boolean; volume: string;
}
export interface Discovery { anomaly: AnomalyCandidate[]; mo2: Mo2Candidate[]; scannedMs: number; mo2Running: boolean }
export interface PathsInput { anomaly: string; mo2: string | null; profile: string | null; stand: string }
export interface PathsReport {
  ok: boolean; issues: AppError[];
  volumes: { anomaly: string; mo2: string | null; stand: string };
  fileSystem: string; linkProbe: 'ok' | 'failed' | 'skipped';
  freeBytes: number; needBytes: number; standState: 'empty' | 'raylink' | 'foreign' | 'missing';
  modsCount: number | null; missingMods: string[];
}
export interface StandSetup { paths: PathsInput; nick: string; secondInstance: boolean }

export interface LogPreview { files: Array<{ name: string; bytes: number; note: string | null }>; totalBytes: number; warnings: string[] }
export type LogTarget = { kind: 'folder'; path: string | null } | { kind: 'clipboard' };
export interface LogBundle { path: string; bytes: number; id: string; copiedToClipboard: boolean }
export type FolderKind = 'stand' | 'standLogs' | 'launcherLogs' | 'builds';

// Карта команд и событий: типизированный invoke без any
export interface CommandMap {
  app_info: { args: Record<string, never>; result: AppInfo };
  status_get: { args: Record<string, never>; result: LauncherStatus };
  settings_get: { args: Record<string, never>; result: Settings };
  settings_set: { args: { patch: SettingsPatch }; result: Settings };
  discover_installs: { args: Record<string, never>; result: Discovery };
  paths_validate: { args: { input: PathsInput }; result: PathsReport };
  stand_setup: { args: { input: StandSetup }; result: JobRef };
  stand_rebuild_gamma: { args: Record<string, never>; result: JobRef };
  stand_verify: { args: Record<string, never>; result: JobRef };
  stand_reset_game_settings: { args: Record<string, never>; result: null };
  gamma_check: { args: { deep: boolean }; result: GammaState };
  channel_set: { args: { repo: string; flavor: ChannelFlavor }; result: ChannelHealth };
  channel_set_token: { args: { token: string }; result: ChannelHealth };
  channel_clear_token: { args: Record<string, never>; result: ChannelHealth };
  update_check: { args: Record<string, never>; result: UpdateInfo };
  update_start: { args: { build: BuildNo | null }; result: JobRef };
  job_cancel: { args: { jobId: string }; result: null };
  builds_list: { args: Record<string, never>; result: BuildEntry[] };
  rollback_start: { args: { build: BuildNo }; result: JobRef };
  pin_set: { args: { build: BuildNo | null }; result: Settings };
  host_probe: { args: { address: string }; result: ProbeResult };
  local_addresses: { args: Record<string, never>; result: LocalAddress[] };
  public_address: { args: Record<string, never>; result: string };
  game_launch: { args: { req: LaunchRequest }; result: GameSession };
  game_focus: { args: { sessionId: string }; result: null };
  game_close: { args: { sessionId: string; force: boolean }; result: null };
  logs_preview: { args: Record<string, never>; result: LogPreview };
  logs_export: { args: { target: LogTarget }; result: LogBundle };
  open_folder: { args: { which: FolderKind }; result: null };
  motion_system_reduced: { args: Record<string, never>; result: boolean };
  launcher_update_check: { args: Record<string, never>; result: LauncherSelfUpdate };
  launcher_update_install: { args: Record<string, never>; result: null };
}
export interface EventMap { 'qn://status': LauncherStatus; 'qn://job': JobEvent; 'qn://game': GameEvent }
export type CommandName = keyof CommandMap;
export type EventName = keyof EventMap;

export interface Backend {
  call<K extends CommandName>(cmd: K, args: CommandMap[K]['args']): Promise<CommandMap[K]['result']>;
  on<E extends EventName>(event: E, handler: (payload: EventMap[E]) => void): Promise<() => void>;
  window: { minimize(): Promise<void>; close(): Promise<void> };
}
```

- `tauri.ts` валидирует **каждый** ответ и событие схемой `valibot` из `schema.ts`. Отказ команды приходит как
  `AppError` → `BackendError` с полем `error`.
- Мок (`mock/backend.ts`) реализует тот же `Backend`.
  - Сценарий выбирается `?mock=<имя>`.
  - Тесты управляют им через `window.__qnMock`: `emit(event, payload)`, `advance(ms)`, `resolveNext(cmd, value)`.
- Сценарии, общие для T3–T5 и e2e: `fresh`, `ready`, `updateAvailable`, `updating`, `verifying`, `installing`,
  `waitingGame`, `updateFailedHash`, `gammaChanged`, `txnRecovery`, `offline`, `channelAuth`, `channelEmpty`,
  `rateLimited`, `hostOk`, `hostOlder`, `hostNewer`, `hostModsDiffer`, `hostNoLauncher`, `hostTimeout`,
  `protocolMismatch`, `gameRunning`, `launcherTooOld`, `pinned`, `noRollback`.

### 2.4 Внутренние интерфейсы Rust
```rust
// qn-fs (T7): единственный путь к диску для stand/gamma/install
pub enum Kind { File, Dir, Reparse }
pub struct Meta { pub kind: Kind, pub len: u64, pub mtime_ns: i128, pub readonly: bool,
                  pub file_id: Option<(u32 /*volume serial*/, u128 /*file index*/)> }
pub struct Volume { pub serial: u32, pub fs_name: String, pub free_bytes: u64 }
pub trait Fs: Send + Sync {
    fn stat(&self, p: &Path) -> io::Result<Meta>;             // открывает файл (MFT), не запись каталога
    fn list(&self, dir: &Path) -> io::Result<Vec<(OsString, Kind)>>;
    fn mkdir(&self, p: &Path) -> io::Result<()>;
    fn hard_link(&self, src: &Path, dst: &Path) -> io::Result<()>;
    fn copy(&self, src: &Path, dst: &Path) -> io::Result<()>; // копия всегда записываемая
    fn unlink(&self, p: &Path) -> io::Result<()>;             // read-only: FileDispositionInfoEx, атрибут не трогает
    fn rmdir_empty(&self, p: &Path) -> io::Result<()>;
    fn rename_replace(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn read(&self, p: &Path) -> io::Result<Vec<u8>>;
    fn write_new(&self, p: &Path, bytes: &[u8]) -> io::Result<()>; // CREATE_NEW
    fn volume(&self, p: &Path) -> io::Result<Volume>;
}
// MemFs: тома, inode (ссылка = общий inode), junction, read-only, отказ после k-й мутации,
// ERROR_SHARING_VIOLATION n раз, журнал мутаций по томам (для инварианта «источники не тронуты»)

// qn-core::relpath (T6)
pub struct SafeRel(String);  // только через SafeRel::parse(&str) -> Result<SafeRel, RelPathError>

// qn-core::channel (T6)
pub trait ReleaseSource {
    async fn releases(&self, flavor: Flavor, etag: Option<&str>) -> Result<Releases, ChannelError>;
    async fn manifest(&self, r: &ReleaseRef) -> Result<ValidManifest, ChannelError>;
    async fn download(&self, a: &AssetRef, to: &Path, p: &dyn Progress, c: &Cancel) -> Result<Sha256, ChannelError>;
}
pub trait CredentialStore { fn get(&self, target: &str) -> Result<Option<SecretString>, CredError>;
                            fn set(&self, target: &str, v: &SecretString) -> Result<(), CredError>;
                            fn delete(&self, target: &str) -> Result<(), CredError>; }

// qn-core::stand / gamma (T7)
pub struct StandSpec { pub root: StandRoot, pub game: PathBuf, pub mo2: Option<Mo2Source>, pub second_instance: bool }
pub fn scan_sources(fs: &dyn Fs, spec: &StandSpec) -> Result<SourceIndex, StandError>;
pub fn want(index: &SourceIndex, ours: &OurLayer) -> WantMap;          // кто победил на каждом пути
pub fn plan(want: &WantMap, current: &StandManifest) -> Plan;           // операции place/remove
pub fn apply(fs: &dyn Fs, root: &StandRoot, plan: &Plan, p: &dyn Progress, c: &Cancel) -> Result<StandManifest, StandError>;
pub fn quick_fingerprint(fs: &dyn Fs, spec: &StandSpec) -> Result<Fingerprint, StandError>;
pub fn deep_check(fs: &dyn Fs, index: &SourceIndex) -> Result<GammaChanges, StandError>;
pub fn portable_fingerprint(index: &SourceIndex) -> PortableFp;      // gammaFp + h_mod
pub fn probe_file(summary: &ProbeSummary, detail: &ProbeDetail) -> Vec<u8>;   // "QNPF" v1

// qn-core::install (T8)
pub struct Installer<'a, F: Fs> { fs: &'a F, root: StandRoot, cache: BuildCache }
impl Installer { fn prepare(&self, m: &ValidManifest, packs: &PackCache) -> Result<VerifiedBuild, InstallError>; // «проверяю»
                 fn install(&self, b: &VerifiedBuild, idx: &SourceIndex, guard: &dyn GameGuard, p, c) -> Result<Installed, InstallError>;
                 fn recover(&self, idx: &SourceIndex) -> Result<Option<Recovered>, InstallError>;
                 fn gc(&self, keep: usize, pinned: Option<BuildNo>) -> Result<(), InstallError>; }
pub trait GameGuard { fn running_in(&self, game_root: &Path) -> Vec<u32>; }

// qn-core::probe / launch (T9)
pub fn encode_request(kind: ProbeKind, page: u8, nonce: u32) -> [u8; 1200];
pub fn decode_reply(bytes: &[u8], expect_nonce: u32) -> Result<Reply, ProbeError>;
pub fn verdict(local: &LocalStand, host: &HostInfo, detail: Option<&ProbeDetail>) -> ProbeVerdict;
pub trait ProbeTransport { fn exchange(&self, to: SocketAddrV4, req: &[u8], timeout: Duration)
                               -> io::Result<Option<(Vec<u8>, Duration)>>; }
pub fn args(req: &LaunchRequest, s: &LaunchSettings) -> Vec<OsString>;
pub trait GameProcess { fn spawn(&self, exe: &Path, args: &[OsString], cwd: &Path) -> io::Result<u32>;
                        fn wait(&self, pid: u32) -> io::Result<i32>; fn focus(&self, pid: u32) -> bool;
                        fn close(&self, pid: u32, force: bool) -> io::Result<()>; }

// общие
pub trait Progress: Sync { fn report(&self, step: JobStep, done: u64, total: u64); }
pub struct Cancel(Arc<AtomicBool>);
pub trait Clock { fn now(&self) -> SystemTime; }
```
- `async fn` в трейтах — нативные (Rust ≥ 1.75), подстановка через дженерики, без `dyn` для async-трейтов.
- DTO (`qn-core::dto`) повторяют §2.3 и закрепляются фикстурами.

### 2.5 Общие фикстуры и векторы
- **`contracts/fixtures/<Тип>.<вариант>.json`** — образцы DTO.
  - Vitest: каждая проходит свою `valibot`-схему; `*.invalid.json` — не проходит.
  - Rust (T10 `tests/contract.rs`): образец, построенный в Rust, после `serde_json::to_value` равен фикстуре;
    фикстура после десериализации и обратной сериализации — та же.
- **`contracts/fixtures/{addresses,nicks,relpaths,launch_args}.json`** — одни и те же векторы для TS и Rust.
  - `relpaths` использует и pytest `publish.ps1`.
  - `launch_args` выведен из логики `launch.ps1`: порядок `-fsltx -smap -coop_host/-coop_connect -coop_nick
    -coop_autostatus`.
- **`contracts/probe/vectors.json`** — запросы и ответы QNP v1 в hex с расшифровкой полей.
  - E1 копирует файл в `anomaly-coop/tests/data/probe_vectors.json` с пометкой источника и sha.
  - Тест T9 сверяет копию, если рядом есть репозиторий `anomaly-coop`, иначе пропуск.
- **`contracts/manifest.schema.json`** — JSON Schema манифеста. Это документация и проверка для внешних
  инструментов; закон — валидатор `qn-core::manifest`.

## 3. Задачи

Размер: S ≈ до полудня работы агента, M ≈ день, L ≈ 2+ дня. Модель — по CLAUDE §8: дизайн, ФС, установка,
безопасность — opus; сеть по готовому контракту, PS, экраны по готовому набору — sonnet.

### T1 — Каркас и контракт · ведущая (opus) · M
- **Зависит:** —.
- **Файлы:**
  - каркас фронтенда (§1);
  - `src/lib/api/**`, `src/lib/api/mock/**`, `contracts/**`;
  - Rust-воркспейс: `src-tauri/Cargo.toml`, пустые `qn-fs`, `qn-core` (все `pub mod` с заглушками), `qn-tool`.
    Зависимости SPEC §1.5 объявлены сразу.
  - `.gitignore` (`node_modules`, `dist`, `src-tauri/target`, `temp/`).
- **Сначала тесты:**
  1. `contract.fixtures.test.ts` — каждая фикстура против своей схемы, негативные отвергаются;
  2. `mock.test.ts` — у каждого сценария `status_get` проходит схему. Сценарий `updating` с мок-часами даёт события
     по порядку `download → verify → install → done`;
  3. `address.test.ts`, `nick.test.ts`, `repo.test.ts` — по общим векторам;
  4. `errors.test.ts` — `ERROR_CODES` совпадает с объединением `ErrorCode` (проверка исчерпанности на уровне
     типов и в рантайме).
- **Проверка:** `npm run check && npm run lint && npm run test`; `cargo check --workspace`.
- **Готово:** фикстуры на каждый тип результата и события и на каждый `ErrorCode`; контракт заморожен до конца
  волны 1. Правки потом — только через ведущую.

### T2 — Дизайн-система и оболочка · A (opus) · M
- **Зависит:** T1.
- **Файлы:** `src/styles/**`, `src/assets/**`, `src/lib/ui/**`, `src/lib/shell/**`,
  `src/lib/state/{nav,motion,bg}.ts`, `src/lib/i18n/ru.ts`, `tests/unit/{nav,motion,segments}.test.ts`,
  `tests/e2e/{fixtures.ts,shell.spec.ts}`.
- **Сначала тесты:**
  1. `nav` — индекс по кругу, Q/E/←/→, прозрачность по расстоянию (0 → 1; 1 → 0,55; 2 → 0,15);
  2. `motion` — (режим × системный reduce × `navigator.webdriver`) → `full | calm | off`. Под автоматизацией по
     умолчанию «как в Windows» — фикстуры детерминированы (runbook vault);
  3. `segments` — веса 1,3 : 1 : 1, (шаг, доля) → `scaleX` каждого сегмента, квантование `steps()`;
  4. e2e `shell` — окно 1280×760 без прокрутки; `data-tauri-drag-region` только у заголовка; кольцо фокуса по Tab;
     кнопки окна вызывают `backend.window`; компас клавишами меняет экран; базовый снимок `shell.png`.
- **Реализация:**
  - токены DESIGN §4, §5, §8;
  - иконки из `design/shared/icons.js` → `sprite.svg`;
  - знак «три звена» по построению DESIGN §10;
  - фон с ротацией кадров;
  - страница `?gallery` с компонентами для просмотра;
  - шрифты-заглушки Bahnschrift до «ок» на Sofia Sans.
- **Готово:** снимок оболочки совпадает с `home_b.png` по сетке: поля 56/32, полоса заголовка 36, компас,
  подвал 36.

### T3 — Главный экран · A (opus) · L
- **Зависит:** T2.
- **Файлы:** `src/screens/home/**`, `src/lib/state/{dock,hostcheck,news}.ts`, `tests/unit/{dock,hostcheck,news}.test.ts`,
  `tests/e2e/home.spec.ts`.
- **Сначала тесты:**
  1. `dock` — таблица: 14 строк SPEC §2.2, правило «одной кнопки» с исключениями (закрепление; хост на нашей
     сборке, проверка ≤ 60 с), автопроверка хоста при «ИГРАТЬ» в режиме «Подключиться», «хост не ответил» не
     блокирует;
  2. `hostcheck` — `ProbeVerdict` → строки и `aria-live`-текст; галочки по одной (80 мс), при reduce — сразу;
  3. `news` — ставящаяся сборка сверху с меткой «ставится», «у тебя» у установленной, пустое описание;
  4. e2e по сценариям `ready`, `updateAvailable`, `updating` (сегмент 1, «качаю 6,2 из 9,4 МБ · 3,1 МБ/с»),
     `waitingGame`, `txnRecovery`, `gammaChanged`, `hostOk`, `hostOlder`, `hostNewer` («ОБНОВИТЬ ДО 11»),
     `hostModsDiffer` (список отличий), `protocolMismatch`, `hostTimeout`, `hostNoLauncher`, `gameRunning`,
     `launcherTooOld`; снимки `home_ready.png`, `home_update.png`.
- **Реализация:**
  - крупный номер сборки; «10 → 11» со штриховкой через `background-clip: text`;
  - «до N игроков» из `installed.maxPlayers`;
  - режимы хост/подключение, адреса для друга (`local_addresses`), «узнать внешний адрес» с пояснением;
  - недавние адреса, второе окно.
- **Готово:** владелец видит `home_ready.png` и `home_update.png` рядом с макетами B.

### T4 — Мастер первого запуска и настройки · A (opus) · M
- **Зависит:** T2 (параллельно с T3 при втором UI-исполнителе).
- **Файлы:** `src/screens/wizard/**`, `src/screens/settings/**`, `src/lib/state/{wizard,validators}.ts`,
  `tests/unit/{wizard,validators}.test.ts`, `tests/e2e/{wizard,settings}.spec.ts`.
- **Сначала тесты:**
  1. `wizard` — шаги и запреты перехода; ветка «без GAMMA»; «Назад» сохраняет ввод; `NO_AVX` блокирует;
     `MO2_RUNNING` предупреждает;
  2. `validators` — ник, `owner/repo`, порт;
  3. e2e:
     - счастливый путь `fresh` → сборка стенда (сегменты «качаю · связываю · ставлю») → главный;
     - ошибки `CROSS_VOLUME`, `NOT_NTFS`, `STAND_FOREIGN`;
     - `CHANNEL_AUTH` → появляется поле ключа → ключ принят. После отправки значения нет ни в поле, ни в DOM, ни в
       store — проверка поиском строки;
     - настройки: второе окно → задача; режим анимации меняет `data-motion` у `<html>`.
- **Готово:** мастер проходит без прокрутки на 1280×760, все ошибки шага 2 показаны на месте.

### T5 — «Обновления», «Помощь», ошибки и пустые состояния · B (sonnet, ревью A) · M
- **Зависит:** T2.
- **Файлы:** `src/screens/updates/**`, `src/screens/help/**`, `src/lib/state/{builds,logs}.ts`, `src/lib/i18n/errors.ts`,
  `tests/unit/{builds,errorsText}.test.ts`, `tests/e2e/{updates,help,errors}.spec.ts`.
- **Сначала тесты:**
  1. `builds` — слияние «у тебя / в кэше / на GitHub», значки протокола, `canRollback`, закрепление;
  2. тексты диалога отката: протокол другой, оговорка про сохранения;
  3. `errorsText` — на каждый код из `ERROR_CODES` есть заголовок, текст и действие;
  4. e2e:
     - откат: подтверждение → задача → «закреплена»;
     - закрепить и снять;
     - логи: превью → «в папку» → сообщение с id; «в буфер»;
     - состояния канала: `offline`, `channelAuth`, `rateLimited` (время), `channelEmpty`;
     - галерея ошибок `?gallery=errors` (снимок).
- **Готово:** все пустые состояния SPEC §2.8 видны в галерее.

### T6 — Манифест, пути, канал GitHub, ключ · B (sonnet; ревью безопасности — opus) · L
- **Зависит:** T1.
- **Файлы:** `qn-core/src/{relpath,manifest,secret,redact}.rs`, `qn-core/src/channel/**`,
  `qn-core/tests/{relpath,manifest,channel_http,redact}.rs`, `qn-tool` (подкоманда `manifest check`),
  `contracts/fixtures/{relpaths.json,manifest/*.json}`.
- **Сначала тесты:**
  1. `relpath` — таблица ≥ 40 случаев SPEC §8.4 (`..`, `C:`, `a:b`, `con.txt`, `aux`, `a\b`, `a/./b`, точка или
     пробел в конце, управляющие символы, не-NFC, длина 241) + `proptest`: что бы ни пришло, `SafeRel` не выходит
     за корень;
  2. `manifest` — валидные и вредные фикстуры:
     - неизвестная `schema`;
     - дубликат пути без учёта регистра;
     - путь вне разрешённого места слоя (`bin/dxgi.dll` у engine, `bin/…` у core);
     - ссылка на несуществующий пак;
     - `exeSha256` не совпадает;
     - размер больше лимита;
     - `minLauncher` не semver;
  3. выбор релиза — `stable` против `fresh`, только теги `build-N`, draft пропускаются, закрепление, «лаунчер
     устарел»;
  4. HTTP (`httpmock`):
     - `latest` 200, затем 304 с ETag;
     - 401 → `auth`, 404 → `notFound`;
     - 403 с `x-ratelimit-remaining: 0` → `rateLimited` с `retryAt`;
     - ассет: 302 на чужой хост из белого списка — сервер **проверяет отсутствие** `Authorization`;
     - 302 на хост вне списка → ошибка;
     - обрыв тела → докачка `Range` → верный sha;
     - 200 вместо 206 → заново;
     - sha не совпал → `.part` удалён, `HASH_MISMATCH`;
  5. `redact` — строки с `ghp_…`, `github_pat_…`, `Bearer …` через `tracing` и `Debug` для `SecretString` и
     заголовков — значение не появляется;
  6. `CredentialStore` — фейк в памяти; на Windows реальный keyring под `#[ignore]`, ручной прогон.
- **Проверка:** `cargo test -p qn-core --test relpath --test manifest --test channel_http --test redact`; clippy.
- **Готово:** `qn-tool manifest check` принимает пример SPEC §3.3 (с настоящими sha из теста) и отвергает все
  вредные фикстуры.

### T7 — ФС, сборка стенда, слой GAMMA · C (opus) · L
- **Зависит:** T1.
- **Файлы:** `crates/qn-fs/**`, `qn-core/src/stand/**`, `qn-core/src/gamma/**`,
  `qn-core/tests/{stand_mem,stand_win,gamma,ltx}.rs`, `qn-core/tests/data/**` — очищенный кусок `modlist.txt`
  GAMMA с двумя «битыми» строками.
- **Сначала тесты:**
  1. **Перенос 29 тестов `test_mo2_stand.py` 1:1** на `MemFs` (таблица имён в шапке файла):
     - порядок `modlist`, `trim` и `*`, регистр, слой `bin`, удаление лишнего, юникод;
     - `meta.ini`, `.mohidden`, `appdata` игры, отсутствующий мод;
     - runtime-копии и сохранение правок игры в копиях, «никогда не писать сквозь ссылку»;
     - наши слои под префиксом, порядок слоёв, разбор спецификации слоя;
     - повторный прогон без изменений, изменённый источник, межтомовая ошибка;
     - пересечение со слоем, junction внутри, непустая папка без манифеста, junction до первой записи, удаление не
       через junction;
     - восстановление после обрыва, read-only, `.git`, «запись через ссылку видна в отчёте», отчёт;
  2. то же на настоящем NTFS в `%TEMP%` (`stand_win.rs`, `cfg(windows)`):
     - ссылка делит данные;
     - удаление read-only ссылки не меняет атрибут оригинала;
     - отказ на junction;
     - `fs::metadata` против `DirEntry::metadata` для ссылки — закрепить грабли;
     - межтомовая ошибка — если есть второй том, иначе пропуск;
  3. новое:
     - файлы игрока (`axr_options.ltx`, `appdata_*`) переживают пересборку;
     - каталоги MO2 из `ModOrganizer.ini` (`@ByteArray(...)`);
     - `fsgame_a/_b.ltx` из образца cp1251 и ошибка на неузнанном формате;
     - `user.ltx`: копия, ник, окно `b` в оконном режиме;
     - `bin_b`: ссылки, сверка exe, копия, повторное расхождение → ошибка;
  4. `gamma`:
     - QF меняется от mtime `meta.ini`, порядка `modlist`, файла в `overwrite`, архива `db`;
     - QF не меняется от файлов игрока;
     - DC находит правку файла мода на месте;
     - PF одинаков у двух `MemFs` с разными mtime и разный при другом размере файла;
     - файл пробы: формат `QNPF` v1, ≤ 64 КиБ, summary ≤ 512;
  5. **инвариант** — после любой операции журнал мутаций `MemFs` по томам-источникам пуст;
  6. скорость — план и `want` на 62 тыс. записей в `MemFs` ≤ 1 с; реальный NTFS-бенчмарк под `#[ignore]`.
- **Проверка:** `cargo test -p qn-fs -p qn-core --test stand_mem --test stand_win --test gamma --test ltx`; clippy.
- **Готово:** 29/29 перенесённых и все новые зелёные; бенчмарк на `E:` вручную ≤ 60 с первая сборка.

### T8 — Проверка сборки, установка, откат, восстановление · C (opus) · L
- **Зависит:** T6 (манифест, `SafeRel`), T7 (`Fs`, `want/plan/apply`).
- **Файлы:** `qn-core/src/install/**`, `qn-core/tests/{install,zip_slip,recover}.rs`.
- **Сначала тесты:**
  1. **zip slip** — архивы собираются в коде теста:
     - `../x`, `/x`, `C:/x`, `a\..\x`, `x:stream`, `CON`, `nul.txt`;
     - символическая ссылка;
     - запись не из манифеста, недостающая запись, дубликат без учёта регистра;
     - заявленный размер меньше фактического (бомба).
     - Каждый случай → `ARCHIVE_UNSAFE`, на диске ничего вне `builds\<N>.tmp`, папка `.tmp` удалена;
  2. **проверка** — неверный sha файла → `HASH_MISMATCH`, стенд не тронут; успешная → `rename` и `.verified`;
  3. **установка:**
     - план обновления затрагивает только наш слой и пути, где сменился победитель;
     - `txn.json` пишется до первой операции и удаляется после фиксации;
     - exe в `bin` и `bin_b` одинаковы;
     - файл пробы пересоздан;
  4. **обрыв** — для каждого k от 1 до числа мутаций `MemFs` обрывает работу после k-й операции → `recover()` →
     стенд равен `want(to)` (сборка `to` проверена) или `want(from)`; `stand.json` согласован; инвариант
     «источники не тронуты»;
  5. **ошибка на шаге «ставлю»** — автоматический возврат на `from`, `rolledBackTo` в событии;
  6. **занятый файл** — `MemFs` отдаёт sharing violation n раз → успех при n ≤ 4, `FILE_LOCKED` при n = 5;
  7. **игра запущена** — `GameGuard` возвращает pid → `waitingGame`; закрыли → ставит;
  8. **откат** на сборку в кэше → закрепление; не в кэше → `BUILD_NOT_CACHED`;
  9. **сборка мусора** — хранит N, установленную, предыдущую и закреплённую; паки без ссылок удалены.
- **Проверка:** `cargo test -p qn-core --test install --test zip_slip --test recover`; clippy.
- **Готово:** все k-обрывы восстанавливаются; ни один вредный архив не пишет вне `.tmp`.

### T9 — Проверка хоста (QNP v1) и запуск игры · B (sonnet) · M
- **Зависит:** T1 (векторы). Параллельно с E1.
- **Файлы:** `qn-core/src/probe/**`, `qn-core/src/launch/**` (вместе с `process_win.rs`, `netif_win.rs` за
  трейтами), `qn-tool` (подкоманда `fakegame`), `qn-core/tests/{probe,launch}.rs`.
- **Сначала тесты:**
  1. кодек по `contracts/probe/vectors.json`:
     - запрос ровно 1200 байт;
     - разбор ответа INFO и DETAIL;
     - чужая сигнатура, версия или nonce;
     - строка длиннее 63;
     - `summary_len > 512`;
     - страница вне диапазона;
  2. клиент против фейкового UDP-хоста на loopback (поток в тесте):
     - ответ с первой попытки;
     - потеря первых двух → третья;
     - таймаут 2,5 с;
     - ответ с чужого адреса игнорируется;
     - DNS-таймаут;
     - сборка detail из страниц, пропущенная страница → ошибка;
  3. `verdict` — таблица SPEC §5.6, включая хоста без summary и режим gamma против stock;
  4. `launch::args` против `contracts/fixtures/launch_args.json`: хост, подключение, окно `b`, `smap`,
     `autostatus 0`;
  5. адрес — общие векторы с TS (IPv4, имя, порт по умолчанию, `[::1]` → отказ: ENet только IPv4);
  6. процесс — `qn-tool fakegame --exit-after 1 --code 3` → события `starting/running/exited(3)`; закрытие
     `force`;
  7. адаптеры — классификация VPN по подсетям и именам на фейковом списке.
- **Проверка:** `cargo test -p qn-core --test probe --test launch`; clippy.
- **Готово:** векторы общие с E1; копия в `anomaly-coop` сверена тестом.

### T10 — Приложение Tauri: команды, события, платформа · ведущая или C (opus) · L
- **Зависит:** T6–T9; начинать можно после T1 на заглушках модулей.
- **Файлы:** `src-tauri/src/**` (`main.rs`, `app.rs`, `state.rs`, `commands/*.rs`, `events.rs`, `settings.rs`,
  `discovery.rs`, `logs_bundle.rs`, `clipboard.rs`, `logging.rs`), `build.rs`, `tauri.conf.json` (окно, CSP),
  `capabilities/default.json`, `icons/**`, `src/lib/api/tauri.ts` (финал), `src-tauri/tests/contract.rs`.
- **Сначала тесты:**
  1. `contract.rs` — DTO ↔ фикстуры (§2.5);
  2. `settings` — атомарная запись (tmp + rename), неизвестные поля, миграция `schema 1`, ключа в файле нет;
  3. `discovery` на `MemFs`:
     - признаки Anomaly;
     - `ModOrganizer.ini` (`gamePath`, `selected_profile` с `@ByteArray`, `portable.txt`);
     - бюджет 3 с и отмена;
     - только фиксированные диски;
  4. `logs_bundle`:
     - выбор файлов (3 лога на окно, свежий дамп ≤ 64 МБ);
     - лимит 100 МБ;
     - замена `C:\Users\<имя>`;
     - в архиве нет шаблонов ключа;
  5. события — троттлинг ≤ 10/с, первое и последнее всегда;
  6. порядок старта — `recover` → QF → канал, с фейками.
- **Реализация:**
  - окно из Rust: 1280×760, без рамки, `additional_browser_args` по runbook reduced motion;
  - один экземпляр; `motion_system_reduced` через `SPI_GETCLIENTAREAANIMATION`;
  - буфер `CF_HDROP`; `open_folder` по перечислению;
  - журнал `tracing` с фильтром из T6.
- **Проверка:** `cargo test --workspace`; clippy; `npm run tauri dev` вручную:
  - окно, перетаскивание, свернуть и закрыть;
  - мастер на машине владельца читает `E:\GAMMA` только на чтение, стенд в `E:\RayLink-test`.
- **Готово:** все команды §2.1 отвечают по контракту; e2e T3–T5 повторены вручную на настоящем бэкенде (чек-лист).

### T11 — Самообновление и установщик · B (sonnet) · M
- **Зависит:** T10.
- **Файлы:** `src-tauri/src/self_update.rs`, секции `bundle` и `plugins.updater` в `tauri.conf.json` (после
  вливания T10), `scripts/release-launcher.ps1`, `channel.default.json`, `qn-tool sign`.
- **Сначала тесты:**
  1. сравнение версий;
  2. `latest.json` из пробного прогона скрипта;
  3. проверка updater против локального `httpmock` с подписанным `latest.json`, неверная подпись → отказ;
  4. `qn-tool sign`/`verify` — minisign, ключ из файла, пароль из переменной; значение не печатается.
- **Реализация:**
  - NSIS `currentUser`, WebView2 `embedBootstrapper`, `createUpdaterArtifacts`;
  - скрипт по умолчанию — пробный прогон; загрузка — только `-Upload` по слову владельца.
- **Проверка:** вручную —
  - установка на чистом пользователе Windows;
  - обновление 0.1.0 → 0.1.1 с локального эндпоинта;
  - удаление оставляет стенды и ключ (как сказано в «Помощи»).
- **Готово:** обновление лаунчера проходит с подписью, без прав администратора.

### T12 — Интеграция и приёмка · ведущая + независимый ревьюер (opus) · L
- **Зависит:** всё.
- **Файлы:** `tests/e2e/**` (полный прогон), `temp/acceptance/**` (отчёты), `README.md`.
- **Шаги:**
  1. Полные наборы: `npm run check/lint/test/e2e`, `cargo fmt/clippy/test --workspace`.
  2. **Дифференциальная проверка с `mo2_stand.py`.**
     - Сборка из `E:\GAMMA` в `E:\RayLink-diff\py` и `E:\RayLink-diff\rs`; ссылки почти не занимают места.
     - Сравнение наборов `(rel, src, mode)`; ожидание — 0 отличий, кроме задокументированных.
     - Удаление — безопасным удалением по манифесту.
  3. **Живой канал** (загрузка только по слову владельца):
     - тестовый репозиторий, `publish.ps1 -Upload` сборок N и N+1;
     - обновление, откат, закрепление;
     - убить лаунчер посреди «ставлю» → восстановление;
     - проверить 304 и Range у GitHub (SPEC §3.4, §3.6).
  4. **Два окна на одном ПК**: «Создать игру» → «Запустить второе окно» → проверка хоста `127.0.0.1` → вход.
  5. **ПК друга**: мастер с нуля, проверка хоста владельца по интернету или VPN, игра.
  6. **Process Monitor**: фильтр по процессу лаунчера — ни одной записи вне стенда и `%APPDATA%`/`%LOCALAPPDATA%`
     лаунчера (критерий SPEC §0.4).
  7. **Ревью безопасности** (opus, независимо, по коду, без выводов автора): ключ, zip slip, ограничение путей
     `StandRoot`, редиректы, разбор пробы, capabilities и CSP. Найденное — исправить и перепроверить.
  8. Заметки в vault: проверенное (`verified` с датой) и грабли.
- **Готово:** критерии SPEC §0 закрыты результатами команд и живых прогонов; непроверенное явно перечислено.

## 4. Волны (2–3 исполнителя + ведущая)

| Волна | Ведущая | A — UI (opus) | B — сеть, PS, экраны по набору (sonnet) | C — ФС, установка, движок (opus) |
|---|---|---|---|---|
| 0 | **T1** (контракт, каркас TS и Rust, фикстуры, векторы QNP) | — (ждёт T1) | **P1** `publish.ps1` + штамп сборки (anomaly-coop) | **E1** проба в движке (anomaly-coop/engine) |
| 1 | ревью и заморозка контракта, вливания | **T2** | **T6** | **T7** |
| 2 | вливания, общий прогон | **T3** | **T9** | **T8** |
| 3 | **T10** (проводка; C помогает с `discovery`, `logs_bundle`) | **T4** | **T5** | правки T7/T8 по интеграции |
| 4 | **T12** | правки по приёмке | **T11** | правки по ревью безопасности |

- **Зависимости:** T1 → всё. T2 → T3, T4, T5. T6 + T7 → T8. T6–T9 → T10 → T11. Всё → T12.
  E1 → живая проверка хоста в T12. P1 → живой канал в T12.
- **Контракт во время волн 1–2 заморожен.** Нужна правка → ведущая меняет `contract.ts`, фикстуры и мок одним
  коммитом и оповещает всех.
- **Конфликты `Cargo.toml`.** Зависимости объявляет T1. Если задаче нужна новая — добавляет в своей ветке, сводит
  ведущая при вливании.
- **Два исполнителя вместо трёх:** A берёт ещё T5, C — T9 и P1; E1 переходит в волну 1 к C после T7.

## 5. Задачи вне репозитория лаунчера

### E1 — `anomaly-coop/engine`: ответ на пробу QNP v1 · C (opus) · M
- **Файлы:**
  - `engine/src/xrCoop/coop_probe.{h,cpp}` — чистый C++17 без движка: кодек, лимитер, сборка ответа, чтение файла
    пробы;
  - подключение в `coop_transport.cpp`: `m_host->intercept` только у хоста;
  - снимок полей в `coop_session.cpp` / `xrGame/coop/coop_manager.cpp`: `protocol`, `build_id`, `content_fp`,
    `entries`, ник, игроки, `max`, фаза, путь `$fs_root$\raylink_probe.bin`;
  - тесты в `anomaly-coop/tests/` (+ `tests/data/probe_vectors.json` — копия из лаунчера).
- **Сначала тесты** (`coop_tests.exe`):
  1. векторы: разбор запроса, сборка ответа, байт в байт;
  2. отказ без ответа: длина ≠ 1200, чужая сигнатура или версия; ответ ≤ запроса;
  3. лимитер на фейковых часах: 10/с на IP с запасом 40, 200/с всего; сверх — молчание;
  4. файл: нет → нули; > 64 КиБ → игнор и одна строка в логе; смена mtime → перечитать не чаще раза в 5 с;
     страницы DETAIL;
  5. **интеграция с настоящим ENet на loopback:**
     - хост-сессия + проба сырым UDP — ответ за 1 RTT;
     - параллельно обычное рукопожатие `Hello → Welcome` проходит;
     - не-проба (`intercept` → 0) обрабатывается ENet как раньше;
  6. потоки: проба во время `hold()` (keep-alive, `event == NULL`) отвечает из снимка, без гонок. Нагрузочный прогон
     как у W9.
- **Сборка:** `tools\build_tests.cmd`; затем `tools\build_ci.sh` (после P1) → стенд → живая проба из лаунчера (T12).
  `kProtocolVersion` не меняется.

### P1 — `anomaly-coop`: `tools\publish.ps1` и штамп сборки · B (sonnet) · M
- **Файлы:**
  - `tools/publish.ps1`;
  - `tools/build_ci.sh` — перенос из `temp/build_ci.sh`; пишет `engine-ci\_build\_game\bin_dbg\raylink_build.json`;
  - `tests/tools/test_publish.py`;
  - `releases/raylink/` — локальные записи, в `.gitignore` по решению ведущей.
- **Сначала тесты** (pytest, PowerShell как подпроцесс):
  1. пробный прогон в `-OutDir temp\pub_test` → манифест по SPEC §3.3; пути проходят `contracts/fixtures/relpaths.json`;
  2. детерминизм — два прогона дают одинаковые sha паков;
  3. исключения runtime-файлов;
  4. отказы: штамп от другого коммита движка; exe < 8 МБ или моложе 60 с; номер не больше прошлого;
  5. `-Note`: `+ ! ~ ?` и ` — `;
  6. изменение протокола против прошлой записи → предупреждение;
  7. `-Upload` с подставным `-GhPath` (`.cmd` пишет аргументы в файл) → `release create build-N … --prerelease`;
     без `-Yes` и без ввода — отказ;
  8. `qn-tool manifest check` вызывается, если есть, иначе предупреждение.
- **Готово:** `publish.ps1 -Build 11` на текущем HEAD в пробном режиме печатает таблицу ассетов и «Ничего не
  загружено».

### P2 (по желанию, после T12) — `anomaly-coop`
- `deploy.ps1` пишет `raylink_probe.bin` на стендах владельца — чтобы «Проверить» узнавала сборку у хоста без
  лаунчера.
- Позже — `deploy.ps1 -Profile` вызывает `qn-stand.exe` вместо `python mo2_stand.py` (SPEC §4.6).

### O1 — владелец (не код)
- Создать репозиторий сборок (приватный или публичный) и, при желании, публичный репозиторий лаунчера;
  `gh auth login`.
- Для приватного канала — read-only fine-grained ключ для друзей.
- 5–10 скриншотов GAMMA ≥ 1920 px без HUD.
- «Ок» на скачивание шрифтов Sofia Sans и зависимостей npm/cargo.
- Позже (T11): ключ подписи minisign — где хранить.

## 6. Без Rust и с Rust
Rust поставлен 2026-10-23 23:22, поэтому линии Rust начинаются сразу в волне 1. Разделение остаётся полезным для
распределения:

| Делается без Rust | Нужен Rust |
|---|---|
| T1 (TS-часть): каркас Vite/Svelte, `contract.ts`, схемы, мок, фикстуры, векторы QNP | T1 (Rust-каркас воркспейса) |
| T2–T5: все экраны, Vitest, Playwright по Vite с `?mock=` | T6–T9: ядро и его тесты на фейковой ФС и фейковой сети |
| E1 (C++ в движке), P1 (PowerShell + pytest; `qn-tool` по желанию) | T10 (приложение), T11 (установщик и updater), T12 (живые прогоны) |

**Ещё не установлено и нужно скачать** — с «ок» владельца:
- `@tauri-apps/cli` и зависимости npm/cargo — при `npm install` / `cargo fetch` в T1;
- NSIS и загрузчик WebView2 — Tauri качает при первой сборке установщика (T11). NSIS 3.12 уже есть в
  `D:\Projects\Veltrix\temp\toolchains\nsis`.

## 7. Приёмка (сводно к SPEC §0)
| Критерий | Чем подтверждается |
|---|---|
| Мастер → «ИГРАТЬ», стенд ≤ 60 с | T12.2 и живой прогон на `E:\GAMMA`, время из отчёта стенда |
| Обновление одной кнопкой, установка ≤ 5 с | T12.3, время из `launcher.log` |
| Обрыв доводится или откатывается | T8 (все k) + T12.3 (убить процесс) |
| Ни байта вне стенда | T7/T8 (инвариант `MemFs`) + T12.6 (Process Monitor) |
| «Проверить» ≤ 3 с с внятным итогом | T9 + E1 + T12.4/T12.5 |
| Ключа нет в файлах, логах, архиве | T6 `redact`, T10 `logs_bundle`, T12.7 |

## 8. Риски плана
| Риск | Что делаем |
|---|---|
| Контракт меняется посреди волн и ломает соседей | Заморозка после T1; одна точка правки — ведущая; фикстуры ловят расхождение на обеих сторонах |
| Rust-стенд медленнее Python на реальном NTFS | Бенчмарк в T7 до T8; при необходимости — параллельная раскладка по папкам (пул потоков), без смены формата |
| Живые проверки нужны на двух ПК | T12.5 планируется с владельцем и другом; до этого — два окна на одном ПК (T12.4) |
| E1 требует сборки движка, а агенты его меняют | Только закоммиченное: `tools/build_ci.sh` (`ci_sync --committed`), отдельный worktree (vault, xrCoop) |
| Параллельные агенты съедают лимит (урок 2026-10-20) | Не больше трёх исполнителей одновременно; sonnet там, где хватает (B) |
