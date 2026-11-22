# Stillfeather（Rust 版）效能量測，結果寫到 app\perf-report.txt。
#   .\perf.ps1            先建置，再量測
#   .\perf.ps1 -NoBuild   直接用現有的 app\FeatherFiles.exe
#
# 量測內容：
#   1. 核心邏輯（cargo run --release --example perf_core）：排序、篩選、搜尋比對、語法上色…
#   2. Windows 程式路徑（FeatherFiles.exe --perf）：資料夾列舉、側欄、檔名搜尋、圖示、快速預覽
#   3. 實際開啟 Stillfeather 視窗：啟動時間、記憶體、閒置 CPU、開大資料夾、重複重新整理後的資源洩漏
# 第 3 步會開一個 Stillfeather 視窗並自動操作約 1 分鐘，結束後自動關閉；量測期間請不要動它。
# 只讀取檔案，不會修改、移動或刪除任何東西。
param([switch]$NoBuild, [int]$IdleSeconds = 30, [int]$Reloads = 20)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$root = $PSScriptRoot
$exe = Join-Path $root 'app\FeatherFiles.exe'
$trace = Join-Path $root 'app\trace.log'
$coreFile = Join-Path $root 'app\perf-core.txt'
$reportFile = Join-Path $root 'app\perf-report.txt'
$lines = New-Object System.Collections.Generic.List[string]

function Out-Line([string]$text = '') {
    Write-Host $text
    $lines.Add($text)
}

function Out-Check([string]$label, [string]$value, [bool]$ok, [string]$target) {
    $mark = if ($ok) { 'OK  ' } else { '注意' }
    Out-Line ('{0} {1,-30} {2,-22} 目標 {3}' -f $mark, $label, $value, $target)
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class FeatherPerf {
    [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr hProcess, uint flags);
}
'@

function Get-Snapshot([System.Diagnostics.Process]$p) {
    $p.Refresh()
    [pscustomobject]@{
        WorkingSetMB = [math]::Round($p.WorkingSet64 / 1MB, 1)
        PrivateMB    = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
        Handles      = $p.HandleCount
        Threads      = $p.Threads.Count
        GDI          = [FeatherPerf]::GetGuiResources($p.Handle, 0)
        USER         = [FeatherPerf]::GetGuiResources($p.Handle, 1)
    }
}

# trace.log 的效能記錄：「時間 [執行緒] 效能：名稱 數值」
function Get-TraceValues([string]$name) {
    if (-not (Test-Path -LiteralPath $trace)) { return @() }
    $stream = [System.IO.File]::Open($trace, 'Open', 'Read', 'ReadWrite')
    try {
        $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
        $text = $reader.ReadToEnd()
    } finally {
        $stream.Dispose()
    }
    $result = @()
    foreach ($line in ($text -split "`r?`n")) {
        $i = $line.IndexOf("] 效能：$name ")
        if ($i -ge 0) { $result += [int64]($line.Substring($i).Split(' ')[-1]) }
    }
    return ,$result
}

function Wait-TraceCount([string]$name, [int]$count, [int]$timeoutSec = 60) {
    $deadline = (Get-Date).AddSeconds($timeoutSec)
    while ((Get-Date) -lt $deadline) {
        if ((Get-TraceValues $name).Count -ge $count) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

function Quote([string]$s) { return '"' + $s + '"' }

# 從使用者資料夾找一個本機上（不是只在雲端）的範例檔案，給快速預覽量測用
function Find-Sample([string[]]$filters) {
    $dirs = @("$env:USERPROFILE\Videos", "$env:USERPROFILE\Pictures", "$env:USERPROFILE\Documents", "$env:USERPROFILE\Downloads", "$env:USERPROFILE\Desktop", $env:OneDrive) |
        Where-Object { $_ -and (Test-Path -LiteralPath $_) }
    foreach ($d in $dirs) {
        foreach ($f in $filters) {
            $hit = Get-ChildItem -LiteralPath $d -Filter $f -File -Recurse -Depth 3 -ErrorAction SilentlyContinue |
                Where-Object { ([int]$_.Attributes -band 0x441000) -eq 0 -and $_.Length -gt 0 } |   # 略過離線、雲端未下載的檔案
                Select-Object -First 1
            if ($hit) { return $hit.FullName }
        }
    }
    return $null
}

Push-Location $root
try {
    $running = Get-Process FeatherFiles -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }
    if ($running) { throw '請先關閉 Stillfeather（Rust 版）再量測：量測會開一個新的 Stillfeather 視窗。' }
    if (-not $NoBuild) {
        & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root 'build.ps1')
        if ($LASTEXITCODE -ne 0) { throw '建置失敗，請看 build.log。' }
    }
    if (-not (Test-Path -LiteralPath $exe)) { throw '找不到 app\FeatherFiles.exe，請先執行 .\build.ps1。' }

    $cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
    $os = Get-CimInstance Win32_OperatingSystem
    $ram = [math]::Round($os.TotalVisibleMemorySize / 1MB, 1)
    $disk = try { (Get-PhysicalDisk | Where-Object { $_.DeviceId -eq '0' } | Select-Object -First 1).MediaType } catch { '' }
    Out-Line "Stillfeather 效能量測  $(Get-Date -Format 'yyyy/MM/dd HH:mm')"
    Out-Line "電腦：$cpu · 記憶體 $ram GB · $($os.Caption) (組建 $($os.BuildNumber)) $disk"
    Out-Line "程式：$exe（$([math]::Round((Get-Item -LiteralPath $exe).Length / 1KB)) KB）"

    # ---------------------------------------------------------------- 1. 核心邏輯
    Out-Line ''
    Out-Line '# 1. 核心邏輯（cargo run --release --example perf_core）'
    if (Get-Command cargo -ErrorAction SilentlyContinue) {
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $core = & cargo run --release --quiet --example perf_core 2>$null
            foreach ($l in $core) { Out-Line "$l" }
        } finally {
            $ErrorActionPreference = $previous
        }
    } else {
        Out-Line '（找不到 cargo，略過）'
    }

    # ---------------------------------------------------------------- 2. Windows 程式路徑
    Out-Line ''
    Out-Line '# 2. Windows 程式路徑（FeatherFiles.exe --perf）'
    Write-Host '正在找快速預覽用的範例檔案…'
    $samples = @(
        (Find-Sample @('*.mp4', '*.mov', '*.mkv')),
        (Find-Sample @('*.jpg', '*.jpeg', '*.png', '*.heic')),
        (Find-Sample @('*.pdf'))
    ) | Where-Object { $_ }
    Remove-Item -LiteralPath $coreFile -ErrorAction SilentlyContinue
    $perfArgs = @('--perf', (Quote $coreFile)) + ($samples | ForEach-Object { Quote $_ })
    Write-Host '正在量測資料夾讀取、搜尋、圖示與預覽（約 10–60 秒）…'
    $p = Start-Process -FilePath $exe -ArgumentList ($perfArgs -join ' ') -PassThru -Wait
    if (Test-Path -LiteralPath $coreFile) {
        foreach ($l in (Get-Content -LiteralPath $coreFile -Encoding UTF8)) { Out-Line $l }
    } else {
        Out-Line "（--perf 沒有產生結果，結束代碼 $($p.ExitCode)）"
    }

    # ---------------------------------------------------------------- 3. 實際開啟 Stillfeather
    Out-Line ''
    Out-Line '# 3. 實際開啟 Stillfeather 視窗'
    Write-Host '正在開啟 Stillfeather 並量測，請不要操作它…'
    # 舊的 trace.log 可能還留著上一次的數字；Stillfeather 啟動時也會清空它，但先刪掉才不會在那之前讀到舊值
    Remove-Item -LiteralPath $trace -ErrorAction SilentlyContinue
    $start = [System.Diagnostics.Stopwatch]::StartNew()
    $app = Start-Process -FilePath $exe -ArgumentList (Quote $env:USERPROFILE) -PassThru
    $shown = Wait-TraceCount '第一個資料夾顯示（啟動後毫秒）' 1 30
    $wall = $start.ElapsedMilliseconds
    if (-not $shown) { throw 'Stillfeather 啟動後 30 秒內沒有顯示資料夾。' }
    $window = (Get-TraceValues '主視窗顯示（啟動後毫秒）')[0]
    $first = (Get-TraceValues '第一個資料夾顯示（啟動後毫秒）')[0]
    Out-Check '啟動到主視窗出現' "$window ms" ($window -le 300) '≤ 300 ms'
    Out-Check '啟動到第一個資料夾顯示' "$first ms（外部量 $wall ms）" ($first -le 600) '≤ 600 ms'

    Start-Sleep -Seconds 2
    $s0 = Get-Snapshot $app

    # 閒置 CPU：什麼都不做的時候不該耗電
    $app.Refresh()
    $c0 = $app.TotalProcessorTime
    Start-Sleep -Seconds $IdleSeconds
    $app.Refresh()
    $idleMs = ($app.TotalProcessorTime - $c0).TotalMilliseconds
    $idle = $idleMs / ($IdleSeconds * 1000) * 100
    # Windows 以 15.6 ms 為單位記錄行程用掉的 CPU 時間，量不到比這更小的量：容許一個單位
    $idleOk = $idleMs -le ($IdleSeconds * 1000 * 0.001 + 15.7)
    Out-Check "閒置 CPU（$IdleSeconds 秒）" ('{0:N2} %（{1:N0} ms）' -f $idle, $idleMs) $idleOk '≤ 0.1 %（容許 1 個 15.6 ms 計時單位）'

    # 大資料夾：用第二個 Stillfeather 把路徑交給已開啟的視窗（與從檔案總管開啟相同）
    $big = @()
    foreach ($dir in @("$env:SystemRoot\System32", "$env:SystemRoot\WinSxS")) {
        if (-not (Test-Path -LiteralPath $dir)) { continue }
        $before = (Get-TraceValues '資料夾讀取完成（毫秒）').Count
        Start-Process -FilePath $exe -ArgumentList (Quote $dir) -Wait
        if (Wait-TraceCount '資料夾讀取完成（毫秒）' ($before + 1) 60) {
            $msList = Get-TraceValues '資料夾讀取完成（毫秒）'
            $counts = Get-TraceValues '項目數'
            $ms = $msList[$msList.Count - 1]
            $n = $counts[$counts.Count - 1]
            $big += [pscustomobject]@{ Dir = $dir; Ms = $ms; Count = $n }
            Out-Check "開啟 $(Split-Path $dir -Leaf)（$n 項）" "$ms ms" ($ms -le [math]::Max(300, $n / 50)) "≤ $([math]::Max(300, [int]($n / 50))) ms"
        }
    }
    Start-Sleep -Seconds 1
    $s1 = Get-Snapshot $app

    # 重複重新整理：開檔案路徑會讓 Stillfeather 重新讀取所在資料夾並選取該檔案
    Write-Host "正在重複重新整理 $Reloads 次（檢查資源洩漏）…"
    $files = @("$env:SystemRoot\System32\notepad.exe", "$env:SystemRoot\explorer.exe") | Where-Object { Test-Path -LiteralPath $_ }
    $reloadTimes = @()
    if ($files.Count -gt 0) {
        # 先各開一次，讓分頁都建立好，之後每次都是重新讀取
        foreach ($f in $files) { Start-Process -FilePath $exe -ArgumentList (Quote $f) -Wait }
        Start-Sleep -Milliseconds 800
        $base = (Get-TraceValues '資料夾讀取完成（毫秒）').Count
        $mid = Get-Snapshot $app
        for ($i = 0; $i -lt $Reloads; $i++) {
            $f = $files[$i % $files.Count]
            Start-Process -FilePath $exe -ArgumentList (Quote $f) -Wait
            [void](Wait-TraceCount '資料夾讀取完成（毫秒）' ($base + $i + 1) 30)
        }
        Start-Sleep -Seconds 2
        $s2 = Get-Snapshot $app
        $all = Get-TraceValues '資料夾讀取完成（毫秒）'
        if ($all.Count -gt $base) { $reloadTimes = $all[$base..($all.Count - 1)] }
        $avg = if ($reloadTimes.Count) { ($reloadTimes | Measure-Object -Average).Average } else { 0 }
        $max = if ($reloadTimes.Count) { ($reloadTimes | Measure-Object -Maximum).Maximum } else { 0 }
        Out-Check "重新讀取 $($reloadTimes.Count) 次（平均／最慢）" ('{0:N0} / {1} ms' -f $avg, $max) ($avg -le 300) '平均 ≤ 300 ms'
        $dGdi = $s2.GDI - $mid.GDI
        $dUser = $s2.USER - $mid.USER
        $dHandles = $s2.Handles - $mid.Handles
        $dPriv = $s2.PrivateMB - $mid.PrivateMB
        Out-Check 'GDI 物件增加' "$dGdi" ($dGdi -le 10) '≤ 10（否則可能洩漏）'
        Out-Check 'USER 物件增加' "$dUser" ($dUser -le 10) '≤ 10'
        Out-Check '核心物件代號增加' "$dHandles" ($dHandles -le 30) '≤ 30'
        Out-Check '私用記憶體增加' "$dPriv MB" ($dPriv -le 20) '≤ 20 MB'
    } else {
        $s2 = Get-Snapshot $app
    }

    Out-Line ''
    Out-Line ('{0,-24} {1,10} {2,10} {3,8} {4,6} {5,6} {6,6}' -f '時間點', '工作集 MB', '私用 MB', '代號', '執行緒', 'GDI', 'USER')
    foreach ($row in @(@('啟動後', $s0), @('開過大資料夾', $s1), @('重新整理之後', $s2))) {
        $s = $row[1]
        Out-Line ('{0,-24} {1,10} {2,10} {3,8} {4,6} {5,6} {6,6}' -f $row[0], $s.WorkingSetMB, $s.PrivateMB, $s.Handles, $s.Threads, $s.GDI, $s.USER)
    }
    Out-Check '啟動後記憶體（工作集）' "$($s0.WorkingSetMB) MB" ($s0.WorkingSetMB -le 60) '≤ 60 MB'
    Out-Check '開過大資料夾後（私用）' "$($s1.PrivateMB) MB" ($s1.PrivateMB -le 150) '≤ 150 MB'

    # 關閉
    $t = [System.Diagnostics.Stopwatch]::StartNew()
    [void]$app.CloseMainWindow()
    if (-not $app.WaitForExit(10000)) {
        Out-Line '注意 Stillfeather 10 秒內沒有關閉，已強制結束。'
        Stop-Process -Id $app.Id -Force
    } else {
        Out-Check '關閉視窗到程式結束' "$($t.ElapsedMilliseconds) ms" ($t.ElapsedMilliseconds -le 1000) '≤ 1000 ms'
    }

    $lines | Set-Content -LiteralPath $reportFile -Encoding UTF8
    Write-Host ''
    Write-Host "完成。報告：$reportFile" -ForegroundColor Green
} finally {
    if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
    Pop-Location
}
