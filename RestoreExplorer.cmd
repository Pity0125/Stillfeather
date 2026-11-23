<# : batch portion - Stillfeather restore Windows File Explorer, PowerShell below
@echo off
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -Command "& ([ScriptBlock]::Create((Get-Content -LiteralPath '%~f0' -Raw -Encoding UTF8)))"
echo.
pause
exit /b
#>

# Stillfeather：還原 Windows 檔案總管（獨立的救援腳本，不需要 Stillfeather 還在）。
#
# 用在 Stillfeather 打不開、安裝資料夾被直接刪掉而沒有執行解除安裝，或其他原因讓資料夾、Win+E 打不開的時候。
# 只修改目前使用者的登錄（HKCU），不需要系統管理員權限，也不會動到你的檔案。它會：
#   1. 結束執行中的 Stillfeather（包含背景攔截 Win+E）
#   2. 拿掉登入時啟動的背景攔截 Win+E
#   3. 把開啟資料夾、磁碟與 Win+E 改回 Windows 檔案總管（安裝前另一套檔案管理程式的設定會還原）
#   4. 把其他程式的「開啟檔案」視窗改回 Windows 原本的
#   5. Stillfeather 的程式已經不在時，也拿掉「已安裝的應用程式」裡留下的項目
# 和 Stillfeather 的解除安裝（src/win/shellreg.rs 的 restore_default_file_manager、disable_picker）寫的是同樣的登錄位置。

$ErrorActionPreference = 'Continue'
$hkcu = [Microsoft.Win32.Registry]::CurrentUser

function Get-Val([string]$key, [string]$name) {
    $k = $hkcu.OpenSubKey($key)
    if ($null -eq $k) { return $null }
    try { return $k.GetValue($name, $null, 'DoNotExpandEnvironmentNames') } finally { $k.Close() }
}
function Set-Val([string]$key, [string]$name, [string]$data) {
    $k = $hkcu.CreateSubKey($key)
    try { $k.SetValue($name, $data, [Microsoft.Win32.RegistryValueKind]::String) } finally { $k.Close() }
}
function Remove-Val([string]$key, [string]$name) {
    $k = $hkcu.OpenSubKey($key, $true)
    if ($null -eq $k) { return }
    try { $k.DeleteValue($name, $false) } finally { $k.Close() }
}
function Remove-Tree([string]$key) {
    try { $hkcu.DeleteSubKeyTree($key, $false) } catch { Write-Host "  無法刪除 HKCU\$key：$($_.Exception.Message)" }
}
function Remove-IfEmpty([string]$key) {
    $k = $hkcu.OpenSubKey($key)
    if ($null -eq $k) { return }
    $empty = ($k.SubKeyCount -eq 0 -and $k.ValueCount -eq 0)
    $k.Close()
    if ($empty) { try { $hkcu.DeleteSubKey($key, $false) } catch { } }
}
function Test-Ours($cmd) { ($null -ne $cmd) -and ([string]$cmd -match '(?i)(Stillfeather|FeatherFiles)\.exe') }

$appKey    = 'Software\Stillfeather'
$backupKey = 'Software\Stillfeather\DefaultFileManagerBackup'
$wineClsid = 'Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}'
$wineKey   = "$wineClsid\shell\opennewwindow\command"
$folderKeys = @('Software\Classes\Directory\shell', 'Software\Classes\Drive\shell')
$pickerKey = 'Software\Classes\CLSID\{DC1C5A9C-E88A-4dde-A5A1-60F82A20AEF7}'
$runKey    = 'Software\Microsoft\Windows\CurrentVersion\Run'
$uninstKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Stillfeather'

Write-Host '== Stillfeather：還原 Windows 檔案總管 =='
Write-Host ''

# 1. 結束執行中的 Stillfeather（背景攔截 Win+E 也是 Stillfeather.exe）
$procs = @(Get-Process -Name 'Stillfeather', 'FeatherFiles' -ErrorAction SilentlyContinue)
if ($procs.Count -gt 0) {
    $procs | Stop-Process -Force -ErrorAction SilentlyContinue
    Write-Host "已結束 $($procs.Count) 個 Stillfeather 程序。"
}

# 2. 登入時啟動的背景攔截 Win+E（只拿掉 Stillfeather 自己的）
$run = Get-Val $runKey 'Stillfeather'
if ((Test-Ours $run) -and ([string]$run -match '--win-e')) {
    Remove-Val $runKey 'Stillfeather'
    Write-Host '已拿掉登入時啟動的背景攔截 Win+E。'
}

# 3. 預設檔案總管：先判斷每個值是不是 Stillfeather 寫的，再一起還原（DelegateExecute 要看改之前的命令）
$wineCmd = Get-Val $wineKey ''
$wineOurs = Test-Ours $wineCmd
$decisions = @()
foreach ($k in $folderKeys) {
    $decisions += ,@($k, '', ((Get-Val $k '') -eq 'Stillfeather'))
}
$delegate = Get-Val $wineKey 'DelegateExecute'
$decisions += ,@($wineKey, 'DelegateExecute', ($wineOurs -and ($null -ne $delegate) -and ([string]$delegate -eq '')))
$decisions += ,@($wineKey, '', $wineOurs)
$changed = $false
foreach ($d in $decisions) {
    $key = $d[0]; $name = $d[1]; $ours = $d[2]
    if (-not $ours) { continue }
    $saved = Get-Val $backupKey "$key|$name"
    if ($null -ne $saved) { Set-Val $key $name ([string]$saved) } else { Remove-Val $key $name }
    $changed = $true
}
foreach ($k in $folderKeys) {
    $verb = $hkcu.OpenSubKey("$k\Stillfeather")
    if ($null -ne $verb) { $verb.Close(); Remove-Tree "$k\Stillfeather"; $changed = $true }
}
foreach ($k in @($wineKey, "$wineClsid\shell\opennewwindow", "$wineClsid\shell", $wineClsid,
                 $folderKeys[0], $folderKeys[1], 'Software\Classes\Directory', 'Software\Classes\Drive')) {
    Remove-IfEmpty $k
}
Remove-Tree $backupKey
if ($changed) { Write-Host '開啟資料夾、磁碟與 Win+E 已改回 Windows 檔案總管。' } else { Write-Host '開啟資料夾與 Win+E 原本就不是 Stillfeather。' }

# 4. 其他程式的「開啟檔案」視窗
$dll = Get-Val "$pickerKey\InprocServer32" ''
if (($null -ne $dll) -and ([string]$dll -match '(?i)StillfeatherPicker\.dll')) {
    Remove-Tree "$pickerKey\InprocServer32"
    Remove-IfEmpty $pickerKey
    Write-Host '其他程式的開啟檔案視窗已改回 Windows 原本的（已經開著的程式要重新開啟才會套用）。'
} else {
    Write-Host '其他程式的開啟檔案視窗原本就是 Windows 的。'
}

# 記下「關」：Stillfeather 下次啟動不會自己又設回去（之後可以在它的設定重新勾選）
$installDir = Get-Val $appKey 'InstallDir'
$programThere = ($null -ne $installDir) -and (Test-Path -LiteralPath (Join-Path ([string]$installDir) 'Stillfeather.exe'))
$app = $hkcu.OpenSubKey($appKey)
if ($null -ne $app) {
    $app.Close()
    Set-Val $appKey 'DefaultExplorer' '0'
    Set-Val $appKey 'FilePicker' '0'
    Remove-Val $appKey 'PickerExe'
}

# 5. 程式已經不在（安裝資料夾被直接刪掉）：拿掉「已安裝的應用程式」裡的項目與 Stillfeather 的登錄
$location = Get-Val $uninstKey 'InstallLocation'
if (($null -ne $location) -and -not (Test-Path -LiteralPath (Join-Path ([string]$location) 'Stillfeather.exe'))) {
    Remove-Tree $uninstKey
    if (-not $programThere) { Remove-Tree $appKey }
    Write-Host '已拿掉「已安裝的應用程式」裡留下的 Stillfeather 項目（程式已經不在）。'
}

# 讓檔案總管重新讀取關聯（不重新啟動檔案總管）
try {
    Add-Type -Namespace Stillfeather -Name Shell -MemberDefinition '[DllImport("shell32.dll")] public static extern void SHChangeNotify(int e, uint f, System.IntPtr a, System.IntPtr b);'
    [Stillfeather.Shell]::SHChangeNotify(0x08000000, 0, [System.IntPtr]::Zero, [System.IntPtr]::Zero)
} catch { }

Write-Host ''
Write-Host '完成。現在雙擊資料夾與 Win+E 都會開 Windows 檔案總管。'
$data = Join-Path $env:LOCALAPPDATA 'Stillfeather'
if (Test-Path -LiteralPath $data) {
    Write-Host "Stillfeather 的設定與記錄還留在 $data；不再使用的話可以直接刪掉這個資料夾。"
}
