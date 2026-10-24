@echo off
rem RayLink Launcher lite: builds lite\out\RayLinkLauncher.exe with the .NET Framework 4.x compiler that ships with Windows.
rem   lite\build.cmd [owner/RayLink]     the default build channel baked into the exe (Settings can change it)
setlocal
set "HERE=%~dp0"
set "CSC=%WINDIR%\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
if not exist "%CSC%" set "CSC=%WINDIR%\Microsoft.NET\Framework\v4.0.30319\csc.exe"
if not exist "%CSC%" (echo csc.exe of .NET Framework 4 not found & exit /b 2)
set "REPO=%~1"
if "%REPO%"=="" set "REPO=stormie-labs/RayLink-Coop"
if not exist "%HERE%out" mkdir "%HERE%out"
powershell -NoProfile -Command "(Get-Content -Raw -Encoding UTF8 '%HERE%RayLinkLauncher.cs') -replace 'REPLACE_OWNER/RayLink', '%REPO%' | Set-Content -Encoding UTF8 '%HERE%out\RayLinkLauncher.gen.cs'"
set "ICON="
if exist "%HERE%..\src-tauri\icons\icon.ico" set "ICON=/win32icon:%HERE%..\src-tauri\icons\icon.ico"
"%CSC%" /nologo /target:winexe /optimize+ /codepage:65001 /out:"%HERE%out\RayLinkLauncher.exe" %ICON% ^
  /r:System.dll /r:System.Core.dll /r:System.Drawing.dll /r:System.Windows.Forms.dll /r:System.Security.dll ^
  /r:System.IO.Compression.dll /r:System.IO.Compression.FileSystem.dll /r:System.Web.Extensions.dll ^
  "%HERE%out\RayLinkLauncher.gen.cs"
exit /b %ERRORLEVEL%
