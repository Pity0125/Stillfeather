@echo off
chcp 65001 >nul
rem Stillfeather：診斷「在資料夾中顯示」（瀏覽器的下載等）為什麼沒有選取檔案。只讀取，不會修改任何東西。
rem 先在 Stillfeather 開著「下載」資料夾再執行。可以把要測試的檔案路徑當參數傳入；沒有的話用「下載」裡最新的檔案。
rem 結果寫到桌面的 Stillfeather-ShowInFolder.txt，內容包含使用者名稱（出現在路徑裡）。
powershell -NoProfile -ExecutionPolicy Bypass -STA -File "%~dp0tools\DiagnoseShowInFolder.ps1" %*
echo.
pause
