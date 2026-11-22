@echo off
chcp 65001 >nul
setlocal
rem Stillfeather：診斷 Win+E 為什麼沒有改開 Stillfeather。只讀取設定，不會修改任何東西
rem （最後會詢問是否重新啟動 Windows 檔案總管來測試，選 N 就不會）。
rem 結果寫到桌面的 Stillfeather-WinE.txt，內容包含使用者名稱（出現在路徑裡）。

set "OUT=%USERPROFILE%\Desktop\Stillfeather-WinE.txt"
set "WINE=HKCU\Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}"
echo 正在收集 Win+E 相關設定…

(
  echo ==== Stillfeather Win+E 診斷 ====
  echo 時間：%DATE% %TIME%
  echo.
  echo ---- Windows 版本
  ver
  reg query "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion" /v ProductName
  reg query "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion" /v DisplayVersion
  reg query "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion" /v CurrentBuild
  reg query "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion" /v UBR
  echo.
  echo ---- 使用者帳戶控制：EnableLUA 是 0 表示 UAC 已關閉
  reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System" /v EnableLUA
  reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System" /v ConsentPromptBehaviorAdmin
  echo ---- 檔案總管的權限等級：S-1-16-8192 是一般，S-1-16-12288 是系統管理員
  whoami /groups | findstr /c:"S-1-16-"
  echo.
  echo ---- Win+E：目前使用者
  reg query "%WINE%" /s
  echo.
  echo ---- Win+E：整台電腦
  reg query "HKLM\Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}" /s
  echo.
  echo ---- 開啟資料夾：目前使用者
  reg query "HKCU\Software\Classes\Directory\shell" /ve
  reg query "HKCU\Software\Classes\Directory\shell\Stillfeather\command" /ve
  echo.
  echo ---- Stillfeather 的安裝記錄與備份
  reg query "HKCU\Software\Stillfeather" /s
  echo.
  echo ---- 背景攔截 Win+E（新版 Windows 11 的 Win+E 不看登錄，要靠它）：登入時啟動的命令
  reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v Stillfeather
  echo ---- 執行中的 Stillfeather 與命令列：有 --win-e 的就是背景攔截
  powershell -NoProfile -Command "Get-CimInstance Win32_Process | Where-Object { $_.Name -in 'Stillfeather.exe','FeatherFiles.exe' } | ForEach-Object { $_.CommandLine }"
  echo.
  echo ---- 檔案總管設定
  reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced" /v LaunchTo
  reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced" /v SeparateProcess
  reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer" /v NoWinKeys
  echo.
  echo ---- 可能攔截 Win+E 的程式：有列出才表示正在執行
  tasklist /nh /fo csv | findstr /i "PowerToys AutoHotkey dopus XYplorer OneCommander Files.App TotalCmd Stillfeather"
  echo ---- 檔案總管的行程數
  tasklist /nh /fo csv | find /c /i "explorer.exe"
) > "%OUT%" 2>&1

echo.
echo 已寫到桌面的 Stillfeather-WinE.txt。
echo.
echo 接下來可以重新啟動 Windows 檔案總管再測一次 Win+E。
echo 工作列會消失幾秒再出現，開著的檔案總管視窗會關閉。
choice /c YN /m "要重新啟動檔案總管嗎（Y 是／N 否）"
if errorlevel 2 goto done
taskkill /f /im explorer.exe >nul 2>&1
timeout /t 2 /nobreak >nul
start "" explorer.exe
echo.
echo 檔案總管已重新啟動。請等工作列出現後按 Win+E，看開的是不是 Stillfeather。
>> "%OUT%" echo.
>> "%OUT%" echo ---- 已重新啟動檔案總管：請回報之後按 Win+E 開的是哪一個

:done
start "" notepad "%OUT%"
echo.
pause
