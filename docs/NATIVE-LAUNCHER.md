# Native RayLink Launcher 0.2

The approved target is `docs/mockups/concepts-2026-10-28/raylink-b-cinematic.png` with the original `design/shared/img/zone_bunker_raw.jpg`. The application icon is the approved four-node mark with a play cutout on a white disk.

## Delivery contract

- Rust/Tauri 2 owns filesystem, GitHub, game launching, installation and self-update. A strict TypeScript/Vite view receives typed DTOs and progress events; it gets no generic filesystem or shell capability.
- English is the installer and application default. Russian is available at installation and in settings. Store the choice inside the selected installation home.
- One public executable asset, `RayLinkSetup.exe`. It embeds the complete interface and copies itself as `RayLinkLauncher.exe` after the user chooses the installation folder. Normal users receive a shortcut, not a portable executable.
- All downloaded builds, cache and default game stands live under the selected home. Preserve an explicitly migrated legacy stand location. Cross-volume source files are copied rather than silently writing a new home on another volume.
- At startup, check the launcher repository for a higher semantic version. Download the installer over verified HTTPS, require GitHub asset SHA256, hand off to a local updater process, replace the closed executable atomically with rollback, then relaunch. Network failure leaves the existing app usable.
- No host/join, address, account or second-instance controls. The engine's in-game menu handles networking.

## Implementation and acceptance

1. Freeze shared Rust/TypeScript DTOs and migration tests. Validate locale, repo, nickname and game choice. Import nonsecret legacy preferences without writing or printing legacy credentials.
2. Port safe stand layering and mutable-file rules from `lite/RayLinkLauncher.cs` into `rl-core::stand`/`gamma` and `rl-fs`. Test source protection, priorities, cancellation, stale readiness, missing mods and reparse/manifest attacks on temporary fixtures.
3. Port release discovery and transactional installation into `channel`/`install`. Test traversal, reserved Windows names, duplicate archive paths, digest mismatch and interrupted updates. Do not replace a valid build with a partial one.
4. Implement setup/update/uninstall in `lifecycle`, preserving user data and checking owned markers before mutation. Test version ordering, unsafe destinations and rollback. Exercise silent test installation only inside project `temp`.
5. Integrate background workers through narrow Tauri commands. Serialize mutations; support cancellation; block stand rebuild while a game process uses it. Return accurate errors and emit progress without stalling the window.
6. Build the image-led UI, settings/setup/update sheets and installer. Use 120ms button feedback, 200–320ms panel transitions, pointer response on the main action, progress feedback and reduced-motion support. The original photograph remains static and readable.
7. Run Rust tests/clippy, TypeScript tests/typecheck/build, independent security review, browser visual acceptance in both locales, real public GitHub download/hash verification and fixture install/build/launch smoke. Build the Windows release and single installer asset. Report any runtime test that remains unverified.

## Ownership

Stand worker: `rl-fs/src/lib.rs`, `rl-core/src/{stand,gamma}`. Channel worker: `rl-core/src/{channel,install,relpath,manifest,redact}`. Lifecycle worker: `rl-core/src/lifecycle.rs`, `scripts/build-release.ps1`. Lead: manifests, DTOs, settings/launch, Tauri shell, frontend and integration. Workers do not modify Git HEAD/index. Existing design drafts and source game installations are preserved.
