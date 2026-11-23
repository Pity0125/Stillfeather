# 用離線保存的私鑰簽 dist\update.json，產生 dist\update.json.sig（和 update.json 一起上傳到 GitHub Releases）。
#
#   .\tools\Sign-Update.ps1                                 私鑰在 %USERPROFILE%\Stillfeather-signing\update-signing.key
#   .\tools\Sign-Update.ps1 -KeyPath E:\keys\update-signing.key
#
# 簽的是 update.json 的原始位元組：簽完之後就不要再改 update.json（改了要重新簽）。
# 會先確認私鑰和 assets\update-signing-key.txt 的公鑰是同一組；app\FeatherFiles.exe 在的話，
# 再用它內建的公鑰驗證一次（--verify-update），確認已安裝的 Stillfeather 會接受這次的更新。
param(
    [string]$KeyPath = $(if ($env:STILLFEATHER_SIGNING_KEY) { $env:STILLFEATHER_SIGNING_KEY } else { Join-Path $env:USERPROFILE 'Stillfeather-signing\update-signing.key' }),
    [string]$Manifest = ''
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $Manifest) { $Manifest = Join-Path $root 'dist\update.json' }
$Manifest = [System.IO.Path]::GetFullPath($Manifest)
$publicPath = Join-Path $root 'assets\update-signing-key.txt'

function Read-Base64([string]$Path) {
    $line = Get-Content -LiteralPath $Path -Encoding UTF8 | Where-Object { $_.Trim() -and -not $_.TrimStart().StartsWith('#') } | Select-Object -First 1
    if (-not $line) { return $null }
    # 逗號：整個位元組陣列當成一個值傳回（PowerShell 不會把它拆成一個一個的位元組）
    return ,([Convert]::FromBase64String($line.Trim()))
}

if (-not (Test-Path -LiteralPath $Manifest)) { throw "找不到 $Manifest（先執行 BuildSetup.cmd）。" }
if (-not (Test-Path -LiteralPath $KeyPath)) { throw "找不到私鑰：$KeyPath（插上存放私鑰的隨身碟，或用 -KeyPath 指定位置）。" }
$expected = Read-Base64 $publicPath
if (-not $expected) { throw 'assets\update-signing-key.txt 還沒有公鑰：先執行 tools\New-UpdateSigningKey.ps1。' }

$key = [System.Security.Cryptography.CngKey]::Import((Read-Base64 $KeyPath), [System.Security.Cryptography.CngKeyBlobFormat]::EccPrivateBlob)
try {
    $public = $key.Export([System.Security.Cryptography.CngKeyBlobFormat]::EccPublicBlob)
    if ([Convert]::ToBase64String($public) -ne [Convert]::ToBase64String($expected)) {
        throw '這把私鑰和 assets\update-signing-key.txt 的公鑰不是同一組，不能用它簽。'
    }
    $data = [System.IO.File]::ReadAllBytes($Manifest)
    $ecdsa = New-Object System.Security.Cryptography.ECDsaCng($key)
    try {
        $ecdsa.HashAlgorithm = [System.Security.Cryptography.CngAlgorithm]::Sha256
        $signature = $ecdsa.SignData($data)
        if ($signature.Length -ne 64 -or -not $ecdsa.VerifyData($data, $signature)) { throw '簽章產生後驗證失敗。' }
    } finally {
        $ecdsa.Dispose()
    }
} finally {
    $key.Dispose()
}
$sigPath = "$Manifest.sig"
[System.IO.File]::WriteAllText($sigPath, [Convert]::ToBase64String($signature) + "`n", (New-Object System.Text.UTF8Encoding($false)))
Write-Host "簽章：$sigPath" -ForegroundColor Green

# 用建置好的 Stillfeather 內建的公鑰再驗證一次（和已安裝的版本檢查更新時的做法相同）
$exe = Join-Path $root 'app\FeatherFiles.exe'
if (Test-Path -LiteralPath $exe) {
    $p = Start-Process -FilePath $exe -ArgumentList @('--verify-update', "`"$Manifest`"") -Wait -PassThru -WindowStyle Hidden
    switch ($p.ExitCode) {
        0 { Write-Host 'Stillfeather 驗證通過：已安裝的版本會接受這次更新。' -ForegroundColor Green }
        1 { throw 'app\FeatherFiles.exe 沒有內建公鑰：產生金鑰之後要重新建置（BuildSetup.cmd）。' }
        3 { throw 'app\FeatherFiles.exe 驗證不過簽章：它內建的公鑰和這把私鑰不是同一組（產生新金鑰之後要重新建置）。' }
        4 { throw 'update.json 的格式不正確。' }
        default { throw "app\FeatherFiles.exe --verify-update 失敗（結束代碼 $($p.ExitCode)）。" }
    }
}
