# Render the home-screen prototypes to PNG with the installed Edge (headless, no downloads).
# Usage: powershell -File design/tools/render.ps1 [-Only a]   -> docs/mockups/home_<x>.png and home_<x>_update.png
param([string]$Only = "")
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$edge = "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
if (-not (Test-Path $edge)) { throw "Edge not found: $edge" }
$out = Join-Path $root "docs\mockups"
New-Item -ItemType Directory -Force $out | Out-Null
$profile = Join-Path $root "temp\edge-profile"
$pages = @{ a = "home-a-pda.html"; b = "home-b-hero.html"; c = "home-c-marking.html" }
foreach ($k in ($pages.Keys | Sort-Object)) {
  if ($Only -and $k -ne $Only) { continue }
  $html = Join-Path $root ("design\" + $pages[$k])
  if (-not (Test-Path $html)) { Write-Host "skip $k (no $html)"; continue }
  $url = "file:///" + ($html -replace "\\", "/")
  foreach ($state in @("", "update")) {
    $png = Join-Path $out ("home_" + $k + $(if ($state) { "_" + $state } else { "" }) + ".png")
    $u = $url + $(if ($state) { "?state=" + $state } else { "" })
    $args = @("--headless=new", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1",
              "--user-data-dir=$profile", "--virtual-time-budget=3000",
              "--screenshot=$png", "--window-size=1280,760", $u)
    $p = Start-Process -FilePath $edge -ArgumentList $args -Wait -PassThru -WindowStyle Hidden
    if ($p.ExitCode -ne 0 -or -not (Test-Path $png)) { throw "render failed: $k $state (exit $($p.ExitCode))" }
    Write-Host ("ok " + (Split-Path $png -Leaf) + " " + (Get-Item $png).Length)
  }
}
