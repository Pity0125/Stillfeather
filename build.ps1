# 建置 Stillfeather（Rust 版）並輸出到 app\FeatherFiles.exe。
#   .\build.ps1          沒有 Rust 時會詢問是否自動安裝
#   .\build.ps1 -Yes     不詢問，直接安裝並建置
#   .\build.ps1 -Setup   另外產生安裝程式 dist\StillfeatherSetup.exe（內含剛建置好的 Stillfeather），
#                        以及發佈到 GitHub Releases 用的 dist\update.json（更新說明取自 -Notes 或 release-notes.txt）、
#                        update.json.sig（私鑰在 %USERPROFILE%\Stillfeather-signing\update-signing.key 或
#                        STILLFEATHER_SIGNING_KEY 指定的位置時自動簽章，見 tools\Sign-Update.ps1）與 RestoreExplorer.cmd
# 另外建置 app\StillfeatherPicker.dll（picker-dll\：讓其他程式的「開啟檔案」視窗改用 Stillfeather，在「設定」開啟）。
# 完整的編譯輸出會寫到 build.log，方便回報問題。
param([switch]$Yes, [switch]$Setup, [string]$Notes = '')
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$root = $PSScriptRoot
$log = Join-Path $root 'build.log'
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
$gnu = 'stable-x86_64-pc-windows-gnu'
$msvc = 'stable-x86_64-pc-windows-msvc'

function Add-CargoPath {
    if ((Test-Path $cargoBin) -and -not (($env:Path -split ';') -contains $cargoBin)) {
        $env:Path = "$cargoBin;$env:Path"
    }
}

# 有 Visual Studio 的 C++ 工具時用 MSVC 工具鏈；否則用自帶連結器的 GNU 工具鏈（不需要 Visual Studio）。
function Test-MsvcTools {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { return $false }
    $found = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
    return -not [string]::IsNullOrWhiteSpace("$found")
}

function Install-Rustup {
    Write-Host '正在下載 Rust 安裝程式（rustup）…'
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        $installer = Join-Path $env:TEMP 'rustup-init.exe'
        Invoke-WebRequest -UseBasicParsing -Uri 'https://win.rustup.rs/x86_64' -OutFile $installer
        & $installer -y --default-toolchain none --profile minimal
    } catch {
        Write-Host "直接下載失敗（$($_.Exception.Message)），改用 winget…" -ForegroundColor Yellow
        if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
            throw '無法下載 rustup，也找不到 winget。請到 https://rustup.rs 手動安裝後再執行一次。'
        }
        & winget install --id Rustlang.Rustup -e --silent --accept-source-agreements --accept-package-agreements
    }
    Add-CargoPath
    if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
        throw 'rustup 安裝沒有成功，請到 https://rustup.rs 手動安裝後再執行一次。'
    }
}

# rustup 若還沒有預設工具鏈，就設成這次成功使用的工具鏈（不更動既有設定）。
function Set-DefaultToolchainIfMissing([string]$Toolchain) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & rustup default 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { & rustup default $Toolchain 2>&1 | Out-Null }
    } finally {
        $ErrorActionPreference = $previous
    }
}

# 執行 cargo，同時顯示並寫入 build.log。回傳結束代碼。
function Invoke-Logged([string[]]$CargoArgs) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & cargo @CargoArgs 2>&1 | ForEach-Object { $line = "$_"; Write-Host $line; $line } | Out-File -FilePath $log -Encoding utf8 -Append
        return $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
}

Push-Location $root
try {
    # 安裝程式裡的 Stillfeather 要能驗證更新的簽章：沒有公鑰就不建置（見 DEVELOPMENT.md「更新資訊的簽章」）
    if ($Setup) {
        $publicKey = @(Get-Content -LiteralPath (Join-Path $root 'assets\update-signing-key.txt') -Encoding UTF8 -ErrorAction SilentlyContinue |
            Where-Object { $_.Trim() -and -not $_.TrimStart().StartsWith('#') })
        if ($publicKey.Count -eq 0) {
            throw '還沒有更新簽章的金鑰：先執行 .\tools\New-UpdateSigningKey.ps1（只需要一次），commit assets\update-signing-key.txt 之後再建置安裝程式。沒有內建公鑰的 Stillfeather 不會接受任何更新。'
        }
    }
    Add-CargoPath
    $exe = Join-Path $root 'app\FeatherFiles.exe'
    # 背景攔截 Win+E（FeatherFiles.exe --win-e，沒有視窗）會鎖住 exe：先結束它，建置後開啟 Stillfeather 時會再啟動
    Get-CimInstance Win32_Process -Filter "Name = 'FeatherFiles.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -eq $exe -and $_.CommandLine -match '--win-e' } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 300
    $running = Get-Process FeatherFiles -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }
    if ($running) { throw '請先關閉 Stillfeather（Rust 版）再重新建置。' }

    $hasRustup = [bool](Get-Command rustup -ErrorAction SilentlyContinue)
    $hasCargo = [bool](Get-Command cargo -ErrorAction SilentlyContinue)
    if (-not $hasRustup -and -not $hasCargo) {
        Write-Host ''
        Write-Host '這台電腦還沒有 Rust 工具鏈。' -ForegroundColor Yellow
        Write-Host '可以現在自動安裝：rustup 會裝在你的使用者資料夾（%USERPROFILE%\.cargo 與 .rustup），'
        Write-Host '不需要系統管理員權限，約需下載 150–300 MB，之後的建置不需要再下載。'
        if (-not $Yes) {
            $answer = Read-Host '要現在安裝嗎？(Y/N)'
            if ($answer -notmatch '^[Yy]') {
                Write-Host '已取消。也可以到 https://rustup.rs 手動安裝後再執行一次。'
                exit 1
            }
        }
        Install-Rustup
        $hasRustup = $true
    }

    Remove-Item $log -ErrorAction SilentlyContinue
    if ($hasRustup) {
        $toolchains = if (Test-MsvcTools) { @($msvc, $gnu) } else { @($gnu) }
    } else {
        $toolchains = @('')
    }

    $built = $false
    $usedToolchain = ''
    foreach ($tc in $toolchains) {
        if ($tc) {
            Write-Host ''
            Write-Host "使用工具鏈：$tc（第一次會先下載，請稍候）"
            & rustup toolchain install $tc --profile minimal
            if ($LASTEXITCODE -ne 0) {
                Write-Host "無法安裝 $tc。" -ForegroundColor Yellow
                continue
            }
            "== cargo +$tc build --release ==" | Out-File -FilePath $log -Encoding utf8 -Append
            $code = Invoke-Logged @("+$tc", 'build', '--release')
        } else {
            $code = Invoke-Logged @('build', '--release')
        }
        # 嵌入 exe 圖示（.res）失敗時，略過圖示再試一次；視窗與工作列仍會有圖示。
        if ($code -ne 0 -and -not $env:FEATHER_NO_ICON_RES) {
            $text = Get-Content -LiteralPath $log -Raw -Encoding UTF8
            if ($text -match 'feather_icon|feather_setup|cvtres|CVT\d{4}') {
                Write-Host '無法把圖示嵌入 exe，略過圖示重新建置…' -ForegroundColor Yellow
                $env:FEATHER_NO_ICON_RES = '1'
                if ($tc) {
                    $code = Invoke-Logged @("+$tc", 'build', '--release')
                } else {
                    $code = Invoke-Logged @('build', '--release')
                }
            }
        }
        if ($code -eq 0) {
            $built = $true
            $usedToolchain = $tc
            if ($tc) { Set-DefaultToolchainIfMissing $tc }
            break
        }
        # 只有連結器或 Windows SDK 的問題才改用 GNU 工具鏈重試；程式碼錯誤就直接回報。
        $text = Get-Content -LiteralPath $log -Raw -Encoding UTF8
        if ($text -notmatch 'link\.exe|LNK\d{4}|linker') { break }
        Write-Host 'MSVC 連結器無法使用，改用 GNU 工具鏈重試…' -ForegroundColor Yellow
    }

    if (-not $built) {
        Write-Host ''
        Write-Host "建置失敗。完整訊息在：$log" -ForegroundColor Red
        Write-Host '把 build.log 交給 Claude 就能據此修正。'
        exit 1
    }
    New-Item -ItemType Directory -Force -Path (Join-Path $root 'app') | Out-Null
    Copy-Item -Force (Join-Path $root 'target\release\FeatherFiles.exe') $exe
    $size = [Math]::Round((Get-Item $exe).Length / 1KB)
    Write-Host ''
    Write-Host "完成：$exe（$size KB）" -ForegroundColor Green

    # 「其他程式開啟檔案時也用 Stillfeather」的取代元件（picker-dll\，獨立建置）。
    # 失敗時 Stillfeather 照常可用，只是不能開這個功能。
    $pickerDll = Join-Path $root 'app\StillfeatherPicker.dll'
    $pickerBuilt = Join-Path $root 'picker-dll\target\release\stillfeather_picker.dll'
    $pickerArgs = @('build', '--release', '--manifest-path', (Join-Path $root 'picker-dll\Cargo.toml'))
    if ($usedToolchain) { $pickerArgs = @("+$usedToolchain") + $pickerArgs }
    Write-Host ''
    Write-Host '建置開啟檔案視窗元件（StillfeatherPicker.dll）…'
    "== cargo $($pickerArgs -join ' ') ==" | Out-File -FilePath $log -Encoding utf8 -Append
    $code = Invoke-Logged $pickerArgs
    # 嵌入版本資訊失敗時，略過資源再試一次（元件本身不受影響）
    if ($code -ne 0 -and -not $env:FEATHER_NO_ICON_RES) {
        $text = Get-Content -LiteralPath $log -Raw -Encoding UTF8
        if ($text -match 'picker_version|cvtres|CVT\d{4}') {
            Write-Host '無法把版本資訊嵌入 StillfeatherPicker.dll，略過資源重新建置…' -ForegroundColor Yellow
            $env:FEATHER_NO_ICON_RES = '1'
            $code = Invoke-Logged $pickerArgs
        }
    }
    if ($code -eq 0 -and (Test-Path -LiteralPath $pickerBuilt)) {
        Copy-Item -Force $pickerBuilt $pickerDll
        Write-Host "完成：$pickerDll" -ForegroundColor Green
    } else {
        Write-Host "開啟檔案視窗元件建置失敗（Stillfeather 本身不受影響）。完整訊息在：$log" -ForegroundColor Yellow
        $pickerBuilt = ''
    }

    if ($Setup) {
        # 安裝程式把剛建置好的 Stillfeather 嵌進去（編譯時讀取 STILLFEATHER_PAYLOAD 指定的檔案）
        $env:STILLFEATHER_PAYLOAD = Join-Path $root 'target\release\FeatherFiles.exe'
        # 開啟檔案視窗元件也一起嵌入（沒有建置成功時安裝程式就不含它）
        if ($pickerBuilt) {
            $env:STILLFEATHER_PICKER_PAYLOAD = $pickerBuilt
        } else {
            Remove-Item Env:STILLFEATHER_PICKER_PAYLOAD -ErrorAction SilentlyContinue
        }
        $setupArgs = @('build', '--release', '--features', 'setup', '--bin', 'StillfeatherSetup')
        if ($usedToolchain) { $setupArgs = @("+$usedToolchain") + $setupArgs }
        Write-Host ''
        Write-Host '建置安裝程式…'
        "== cargo $($setupArgs -join ' ') ==" | Out-File -FilePath $log -Encoding utf8 -Append
        $code = Invoke-Logged $setupArgs
        if ($code -ne 0 -and -not $env:FEATHER_NO_ICON_RES) {
            $text = Get-Content -LiteralPath $log -Raw -Encoding UTF8
            if ($text -match 'feather_setup|cvtres|CVT\d{4}') {
                Write-Host '無法把圖示與資訊清單嵌入安裝程式，略過資源重新建置…' -ForegroundColor Yellow
                $env:FEATHER_NO_ICON_RES = '1'
                $code = Invoke-Logged $setupArgs
            }
        }
        if ($code -ne 0) {
            Write-Host ''
            Write-Host "安裝程式建置失敗。完整訊息在：$log" -ForegroundColor Red
            exit 1
        }
        $dist = Join-Path $root 'dist'
        New-Item -ItemType Directory -Force -Path $dist | Out-Null
        $setupExe = Join-Path $dist 'StillfeatherSetup.exe'
        Copy-Item -Force (Join-Path $root 'target\release\StillfeatherSetup.exe') $setupExe
        $setupSize = [Math]::Round((Get-Item $setupExe).Length / 1KB)
        Write-Host "安裝程式：$setupExe（$setupSize KB）" -ForegroundColor Green

        # 更新資訊：和安裝程式一起上傳到 GitHub Releases，已安裝的 Stillfeather 會據此提醒更新（見 DEVELOPMENT.md「自動更新」）
        $cargoToml = Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw -Encoding UTF8
        $version = if ($cargoToml -match '(?m)^version\s*=\s*"([^"]+)"') { $Matches[1] } else { '' }
        $repo = if ($cargoToml -match '(?m)^repository\s*=\s*"([^"]+)"') { ($Matches[1].Trim().TrimEnd('/')) -replace '\.git$', '' } else { '' }
        if ($repo -match '^https://github\.com/[^/]+/[^/]+$') {
            $notesFile = Join-Path $root 'release-notes.txt'
            $releaseNotes = if ($Notes) { $Notes } elseif (Test-Path -LiteralPath $notesFile) { (Get-Content -LiteralPath $notesFile -Raw -Encoding UTF8).Trim() } else { '' }
            $manifest = [ordered]@{
                version = $version
                url     = "$repo/releases/download/v$version/StillfeatherSetup.exe"
                sha256  = (Get-FileHash -LiteralPath $setupExe -Algorithm SHA256).Hash.ToLowerInvariant()
                size    = (Get-Item -LiteralPath $setupExe).Length
                notes   = $releaseNotes
            }
            $manifestPath = Join-Path $dist 'update.json'
            [System.IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json), (New-Object System.Text.UTF8Encoding($false)))
            Write-Host "更新資訊：$manifestPath" -ForegroundColor Green

            # 簽章：已安裝的 Stillfeather 只接受有正確簽章的 update.json（舊的簽章對新的 update.json 一定不符，先刪掉）
            Remove-Item -LiteralPath "$manifestPath.sig" -ErrorAction SilentlyContinue
            $signingKey = if ($env:STILLFEATHER_SIGNING_KEY) { $env:STILLFEATHER_SIGNING_KEY } else { Join-Path $env:USERPROFILE 'Stillfeather-signing\update-signing.key' }
            if (Test-Path -LiteralPath $signingKey) {
                & (Join-Path $root 'tools\Sign-Update.ps1') -KeyPath $signingKey -Manifest $manifestPath
            } else {
                Write-Host ''
                Write-Host 'update.json 還沒簽章：插上存放私鑰的隨身碟後執行' -ForegroundColor Yellow
                Write-Host '  .\tools\Sign-Update.ps1 -KeyPath <私鑰的位置>' -ForegroundColor Yellow
                Write-Host '沒有 update.json.sig 的話，已安裝的 Stillfeather（0.1.3 起）不會接受這次更新。' -ForegroundColor Yellow
            }

            # 救援腳本：Stillfeather 打不開、或安裝資料夾被直接刪掉時，用它改回 Windows 檔案總管
            Copy-Item -Force (Join-Path $root 'RestoreExplorer.cmd') $dist
            Write-Host ''
            Write-Host "發佈：在 GitHub 建立標籤 v$version 的正式版 Release，上傳 dist 裡的 StillfeatherSetup.exe、update.json、update.json.sig 與 RestoreExplorer.cmd 四個檔案。"
            Write-Host '發佈前先把 StillfeatherSetup.exe 丟到 https://www.virustotal.com 掃描一次（見 DEVELOPMENT.md「發佈新版本」）。'
        } else {
            Write-Host '沒有產生 update.json：Cargo.toml 的 repository 還沒填 GitHub 專案網址（例如 https://github.com/帳號/專案）。' -ForegroundColor Yellow
        }
    }
} finally {
    Pop-Location
}
