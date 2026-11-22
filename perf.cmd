@echo off
rem Feather Files performance test: double-click to build and measure.
rem Report: app\perf-report.txt (see perf.ps1 for what is measured).
setlocal
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0perf.ps1" %*
echo.
echo Report: "%~dp0app\perf-report.txt"
pause
