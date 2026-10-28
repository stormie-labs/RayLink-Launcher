[CmdletBinding()]
param(
    [string]$CargoExecutable = 'C:/Users/serge/.cargo/bin/cargo.exe'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifest = Join-Path $projectRoot 'src-tauri/Cargo.toml'
$frontend = Join-Path $projectRoot 'dist/index.html'
if (-not (Test-Path -LiteralPath $frontend -PathType Leaf)) {
    throw 'Build the frontend with npm run build before building the release.'
}

# Keep compiler outputs within the project; never publish the raw target executable.
$env:CARGO_TARGET_DIR = Join-Path $projectRoot 'temp/cargo-target'
$metadataJson = & $CargoExecutable metadata --no-deps --format-version 1 --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
$metadata = ($metadataJson -join "`n") | ConvertFrom-Json
$appPackage = @($metadata.packages | Where-Object name -eq 'raylink-launcher')
if ($appPackage.Count -ne 1) { throw 'Cannot identify the launcher package.' }
$appVersion = [string]$appPackage[0].version
if ($appVersion -notmatch '^\d+\.\d+\.\d+$') { throw 'A public release requires a stable semantic version.' }
$config = Get-Content -LiteralPath (Join-Path $projectRoot 'src-tauri/tauri.conf.json') -Raw | ConvertFrom-Json
if ($config.version -ne $appVersion) { throw 'Tauri and Cargo package versions must match.' }
$corePackage = @($metadata.packages | Where-Object name -eq 'rl-core')
if ($corePackage.Count -ne 1 -or $corePackage[0].version -ne $appVersion) {
    throw 'rl-core and launcher versions must match: installation markers use the core package version.'
}

& $CargoExecutable build --release --locked --features custom-protocol --package raylink-launcher --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw 'The release build failed.' }
$binary = Join-Path $env:CARGO_TARGET_DIR 'release/raylink-launcher.exe'
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'Cargo did not produce the launcher executable.' }
$releaseDir = Join-Path $projectRoot "releases/v$appVersion"
New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
$unexpected = @(Get-ChildItem -LiteralPath $releaseDir -Force | Where-Object Name -ne 'RayLinkSetup.exe')
if ($unexpected.Count -ne 0) {
    throw 'The release folder contains other assets. Only RayLinkSetup.exe belongs in the public launcher release.'
}
$setup = Join-Path $releaseDir 'RayLinkSetup.exe'
Copy-Item -LiteralPath $binary -Destination $setup -Force
$setupHash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Output "Release installer: $setup"
Write-Output "SHA-256: $setupHash"
Write-Output 'Ready for review. Nothing has been published.'
