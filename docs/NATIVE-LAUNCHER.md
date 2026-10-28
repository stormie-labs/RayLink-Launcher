# Native RayLink Launcher 0.2

The approved target is `docs/mockups/concepts-2026-10-28/raylink-b-cinematic.png` with the original `design/shared/img/zone_bunker_raw.jpg`. The application icon is the approved four-node mark with a play cutout on a white disk.

## Delivery contract

- Rust/Tauri 2 owns filesystem, GitHub, game launching, installation and self-update. A strict TypeScript/Vite view receives typed DTOs and progress events; it gets no generic filesystem or shell capability.
- English is the installer and application default. Russian is available at installation and in settings. Modern preferences live in `%APPDATA%/RayLink/settings.json`, with schema and owning installation home.
- One public executable asset, `RayLinkSetup.exe`. It embeds the complete interface and copies itself as `RayLinkLauncher.exe` after the user chooses the installation folder. Normal users receive a shortcut, not a portable executable.
- All downloaded builds, cache and default game stands live under the selected home. Reusing preferences in a different home resets its installed build and stand location. Never import the old C# settings.txt. Game and mod layers require the stand to be on their volume, matching the legacy hardlink contract. Check native volume identities before mutation and offer the explicit stand-folder setting when they differ; never silently copy the complete game to another drive. Small engine/Core overlays remain private copies.
- The bilingual native uninstaller preserves preferences by default. Its opt-in checkbox removes only preferences owned by that home. Downloaded builds, cache, stands and game saves are preserved in both modes.
- At startup, check the launcher repository for a higher semantic version. Download the installer over verified HTTPS, require GitHub asset SHA256, hand off to a local updater process, replace the closed executable atomically with rollback, then relaunch. Network failure leaves the existing app usable.
- No host/join, address, account or second-instance controls. The engine's in-game menu handles networking.

## Implementation and acceptance

1. Freeze shared Rust/TypeScript DTOs and migration tests. Validate locale, repo, nickname and game choice. Store schema-2 preferences in AppData, bind installed/stand fields to their home, and migrate native local JSON only from a marked owned installation. Repair the external TSV stand path imported by 0.2.0/0.2.1 by preparing a fresh stand at this home; do not copy or delete old game data.
2. Port safe stand layering and mutable-file rules from `lite/RayLinkLauncher.cs` into `rl-core::stand`/`gamma` and `rl-fs`. Missing MO2 mod directories are skipped with a visible warning and each missing path is logged, matching the existing C# behavior. Existing invalid files, overlaps and filesystem links still fail safely. Test source protection, priorities, cancellation, stale readiness and reparse/manifest attacks on temporary fixtures.
3. Port release discovery and transactional installation into `channel`/`install`. Test traversal, reserved Windows names, duplicate archive paths, digest mismatch and interrupted updates. Do not replace a valid build with a partial one.
4. Implement setup/update/uninstall in `lifecycle`, preserving user data and checking owned markers before mutation. Test version ordering, unsafe destinations and rollback. Exercise silent test installation only inside project `temp`.
5. Integrate background workers through narrow Tauri commands. Serialize mutations; support cancellation; block stand rebuild while a game process uses it. Return accurate errors and emit progress without stalling the window.
6. Build the image-led UI, settings/setup/update sheets and installer. Use 120ms button feedback, 200–320ms panel transitions, pointer response on the main action, progress feedback and reduced-motion support. The original photograph remains static and readable.
7. Run Rust tests/clippy, TypeScript tests/typecheck/build, independent security review, browser visual acceptance in both locales, real public GitHub download/hash verification and fixture install/build/launch smoke. Build the Windows release and single installer asset. Report any runtime test that remains unverified.

## Stand performance acceptance

Compare native materialization with the actual legacy `Stand.Materialize` using `scripts/benchmark-stand.ps1`. The script compiles the unchanged C# source with a separate entry point and runs first/repeat passes over the same synthetic readonly files and Engine/Core layers. Generated files, logs and stands stay under `temp/`; it never uses installed game folders. Native also generates instance configuration and verifies source stability. Measurements remain available as JSON for investigation. Run the benchmark on an otherwise idle disk; use a release executable, and record first and unchanged passes separately. The owner's acceptance target is around 30 seconds for the real GAMMA stand with all validation preserved, rather than matching legacy duration at the cost of checks. `-Enforce -MaxSeconds 30` can gate a representative fixture; a ratio gate is optional only when explicitly set with `-MaxRatio`.

The Windows filesystem pass reuses directory handles while refreshing ancestor attributes for each guarded operation. It never caches trust in reparse attributes: attribute-only reparse mutation is possible despite sharing flags. Directory identity handles deny rename/delete, leaf attributes remain fresh, and the cache ends with the pass. Outside a pass, regular full-path guards remain available.

Create and validate each unique output parent before applying file jobs, as the legacy builder did. Independent entries use at most eight scoped workers with their own guarded filesystem pass. Progress and error admission are synchronized; cancellation stops new jobs and every in-flight worker is joined before return. Keep seed-only user data and the final source fingerprint after the worker scope. An incomplete journal still forces replacement on retry; concurrency never marks partial files as a completed stand.

## Ownership

Stand worker: `rl-fs/src/lib.rs`, `rl-core/src/{stand,gamma}`. Channel worker: `rl-core/src/{channel,install,relpath,manifest,redact}`. Lifecycle worker: `rl-core/src/lifecycle.rs`, `scripts/build-release.ps1`. Lead: manifests, DTOs, settings/launch, Tauri shell, frontend and integration. Workers do not modify Git HEAD/index. Existing design drafts and source game installations are preserved.
