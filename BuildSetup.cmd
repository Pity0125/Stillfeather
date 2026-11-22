@echo off
setlocal
cd /d "%~dp0"
rem Builds Stillfeather and the installer: dist\StillfeatherSetup.exe
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0build.ps1" -Setup
if errorlevel 1 (
  echo.
  echo Installer build did not finish. Details: "%~dp0build.log"
  pause
  exit /b 1
)
echo.
echo Installer: "%~dp0dist\StillfeatherSetup.exe"
pause
