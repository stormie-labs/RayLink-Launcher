# RayLink Launcher

Windows launcher for RayLink co-op on S.T.A.L.K.E.R. Anomaly and G.A.M.M.A. Rust handles installation, downloads, game files and launching; Tauri hosts the bilingual desktop interface.

Download **RayLinkSetup.exe** from [Releases](https://github.com/stormie-labs/RayLink-Launcher/releases/latest). Choose English (default) or Russian and an installation folder. That folder holds the launcher, downloaded builds, cache, scripts and prepared games. Use the desktop or Start menu shortcut afterwards.

The launcher checks its own repository at startup and updates itself through a verified installer. Game builds come from `stormie-labs/RayLink-Coop`. Original Anomaly/GAMMA files are not modified. Saves and settings survive rebuilds, updates and uninstalling the launcher. Files on the same volume are hard linked when safe; files that can change are copied. Choosing another drive can require substantially more disk space.

The interface uses the original bunker photograph and the selected image-led layout. Setup, settings and update progress share the same design, with keyboard focus, reduced motion and local fonts. Network sessions are managed inside the game.

## Development

Windows 10/11, WebView2, Node24+, Rust1.88+ and the Visual Studio C++ build tools are required.

```powershell
npm ci
npm run dev
npm run tauri dev
```

In a development shell, set `CARGO_TARGET_DIR` to this project's `temp/cargo-target` to keep artifacts together. The Vite watcher deliberately ignores `temp` and Rust/build outputs: watching extraction directories can prevent atomic directory renames on Windows.

```powershell
npm run check
npm test
npm run e2e
cargo test --workspace --manifest-path src-tauri/Cargo.toml
cargo clippy --workspace --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
npm run build
./scripts/build-release.ps1
```

The release script produces only `releases/v<version>/RayLinkSetup.exe`. Do not publish the raw launcher binary. The same executable runs setup before installation and launcher mode from its owned installation directory. Installation is per-user; no administrator account or GitHub token is required for the public channels.

Core smoke commands are available through `rl-tool`: `channel`, `install`, `verify`, `prepare` and `launch-args`. Tests use temporary fixtures and never need to write to the original games. `--silent-install <absolute-directory> --test-install --language en` exercises setup without shortcuts or registry changes.

The previous C# launcher remains under `lite/` as the compatibility reference. Older design experiments remain under `design/`; the implemented contract is documented in [NATIVE-LAUNCHER.md](docs/NATIVE-LAUNCHER.md).
