@echo off
setlocal
cd /d "%~dp0"
set "FF_ROOT=%~dp0"

rem No exe yet, or src\ / Cargo.toml newer than the exe: rebuild first.
if not exist "%~dp0app\FeatherFiles.exe" (
  echo app\FeatherFiles.exe not found - running build.ps1 first...
  echo.
  goto build
)
powershell -NoProfile -ExecutionPolicy Bypass -Command "$e = (Get-Item -LiteralPath (Join-Path $env:FF_ROOT 'app\FeatherFiles.exe')).LastWriteTimeUtc; $src = @('src', 'assets', 'Cargo.toml', 'build.rs', 'picker-dll\src', 'picker-dll\Cargo.toml') | ForEach-Object { Join-Path $env:FF_ROOT $_ } | Where-Object { Test-Path -LiteralPath $_ }; if (Get-ChildItem -LiteralPath $src -Recurse -File | Where-Object { $_.LastWriteTimeUtc -gt $e } | Select-Object -First 1) { exit 1 }; exit 0"
if errorlevel 1 (
  echo Source files changed - rebuilding...
  echo.
  goto build
)
goto run

:build
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0build.ps1"
if errorlevel 1 (
  echo.
  echo Build did not finish. Details: "%~dp0build.log"
  pause
  exit /b 1
)
if not exist "%~dp0app\FeatherFiles.exe" (
  echo.
  echo Build did not produce app\FeatherFiles.exe. Details: "%~dp0build.log"
  pause
  exit /b 1
)

:run
if "%~1"=="" (
  start "" "%~dp0app\FeatherFiles.exe" "%~dp0."
) else (
  start "" "%~dp0app\FeatherFiles.exe" %*
)
