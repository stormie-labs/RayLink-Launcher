[CmdletBinding()]
param(
    [ValidateRange(100, 100000)][int]$FileCount = 3000,
    [string]$NativeExecutable = '',
    [string]$CargoExecutable = '',
    [switch]$Enforce,
    [ValidateRange(1, 3600)][double]$MaxSeconds = 30,
    [ValidateRange(0, 100)][double]$MaxRatio = 0
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $NativeExecutable) {
    if (-not $CargoExecutable) {
        $cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
        $CargoExecutable = if ($cargoCommand) { $cargoCommand.Source } else { Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe' }
    }
    $env:CARGO_TARGET_DIR = Join-Path $projectRoot 'temp/cargo-target'
    & $CargoExecutable build --release --locked -p rl-tool --manifest-path (Join-Path $projectRoot 'src-tauri/Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'Native benchmark build failed' }
    $NativeExecutable = Join-Path $env:CARGO_TARGET_DIR 'release/rl-tool.exe'
}
$NativeExecutable = [IO.Path]::GetFullPath($NativeExecutable)
if (-not (Test-Path -LiteralPath $NativeExecutable -PathType Leaf)) { throw 'Native executable not found' }
$fixture = Join-Path $projectRoot ('temp/stand-benchmark-' + [guid]::NewGuid().ToString('N'))
$source = Join-Path $fixture 'source'
$nativeRoot = Join-Path $fixture 'native'
$build = Join-Path $nativeRoot 'builds/b24'
$engine = Join-Path $build 'Anomaly Coop - Engine/bin'
$core = Join-Path $build 'Anomaly Coop - Core/gamedata'
foreach ($directory in @($source, "$source/db", "$source/bin", "$source/gamedata", $engine, "$build/Anomaly Coop - Engine/gamedata", "$core/scripts")) {
    [IO.Directory]::CreateDirectory($directory) | Out-Null
}
for ($index = 0; $index -lt $FileCount; $index++) {
    $directory = Join-Path $source ('gamedata/scripts/group{0}/nested/assets' -f ($index % 24))
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    $path = Join-Path $directory ('script{0:D6}.script' -f $index)
    [IO.File]::WriteAllText($path, ('x' * 4096))
    [IO.File]::SetAttributes($path, [IO.FileAttributes]::ReadOnly)
}
[IO.File]::WriteAllText((Join-Path $source 'fsgame.ltx'), '$app_data_root$ = true|false|$fs_root$|appdata\')
[IO.File]::WriteAllText((Join-Path $engine 'AnomalyCoopDX11AVX.exe'), 'MZfixture')
[IO.File]::WriteAllText((Join-Path $core 'scripts/coop.script'), 'fixture')
$settings = Join-Path $fixture 'settings.json'
[IO.File]::WriteAllText($settings, (@{game='anomaly';anomalyDir=$source;installed='b24';language='en'} | ConvertTo-Json))
$harness = @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
namespace RayLink {
    static class LegacyBenchmark {
        static int Main(string[] args) {
            string root = AppDomain.CurrentDomain.BaseDirectory;
            foreach (string arg in args) if (!Path.GetFullPath(arg).StartsWith(root, StringComparison.OrdinalIgnoreCase)) return 2;
            var layers = new List<Layer> {
                new Layer { Root=args[0], Name="game", Kind="game" },
                new Layer { Root=args[2], Name="engine", Prefix="bin", Kind="ours", Copy=true },
                new Layer { Root=args[3], Name="core", Prefix="gamedata", Kind="ours", Copy=true }
            };
            var stand = new Stand(message => Console.Error.WriteLine(message));
            for (int pass=1;pass<=2;pass++) {
                var timer = Stopwatch.StartNew(); stand.Materialize(layers,args[1]);
                Console.WriteLine(timer.Elapsed.TotalMilliseconds.ToString(System.Globalization.CultureInfo.InvariantCulture));
            }
            return 0;
        }
    }
}
'@
$harnessPath = Join-Path $fixture 'LegacyBenchmark.cs'
[IO.File]::WriteAllText($harnessPath, $harness)
$legacyExecutable = Join-Path $fixture 'legacy-benchmark.exe'
$compiler = Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
if (-not (Test-Path -LiteralPath $compiler)) { throw '.NET Framework C# compiler not found' }
& $compiler /nologo /target:exe /optimize+ /codepage:65001 /main:RayLink.LegacyBenchmark "/out:$legacyExecutable" /r:System.dll /r:System.Core.dll /r:System.Drawing.dll /r:System.Windows.Forms.dll /r:System.Security.dll /r:System.IO.Compression.dll /r:System.IO.Compression.FileSystem.dll /r:System.Web.Extensions.dll (Join-Path $projectRoot 'lite/RayLinkLauncher.cs') $harnessPath
if ($LASTEXITCODE -ne 0) { throw 'Legacy benchmark build failed' }
$measurements = @()
for ($pass = 1; $pass -le 2; $pass++) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    & $NativeExecutable prepare $settings $nativeRoot *> (Join-Path $fixture "native-$pass.log")
    $exitCode = $LASTEXITCODE; $timer.Stop()
    if ($exitCode -ne 0) { throw "Native pass $pass failed; inspect $fixture/native-$pass.log" }
    $measurements += $timer.Elapsed.TotalMilliseconds
}
$legacyTimes = @(& $legacyExecutable $source (Join-Path $fixture 'legacy') $engine $core 2> (Join-Path $fixture 'legacy.log'))
if ($LASTEXITCODE -ne 0 -or $legacyTimes.Count -ne 2) { throw 'Legacy benchmark failed' }
$rows = for ($index=0; $index -lt 2; $index++) {
    $legacyMs = [double]::Parse($legacyTimes[$index], [Globalization.CultureInfo]::InvariantCulture)
    [pscustomobject]@{Pass=$index+1;NativeMs=[math]::Round($measurements[$index],1);LegacyMs=[math]::Round($legacyMs,1);Ratio=[math]::Round($measurements[$index]/$legacyMs,2)}
}
$rows | Format-Table
$rows | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $fixture 'results.json')
Write-Output "Fixture: $fixture"
Write-Output 'Same source files and layers; native additionally generates instance configs and verifies source stability.'
if ($Enforce -and @($rows | Where-Object NativeMs -gt ($MaxSeconds * 1000)).Count) { throw "Native stand exceeds the allowed $MaxSeconds second budget" }
if ($Enforce -and $MaxRatio -gt 0 -and @($rows | Where-Object Ratio -gt $MaxRatio).Count) { throw "Native stand exceeds the explicitly selected ${MaxRatio}x legacy budget" }
