# 產生 Stillfeather 自動更新用的簽章金鑰（ECDSA P-256，Windows 內建的 CNG）。只需要做一次。
#
#   .\tools\New-UpdateSigningKey.ps1                       私鑰寫到 %USERPROFILE%\Stillfeather-signing\update-signing.key
#   .\tools\New-UpdateSigningKey.ps1 -KeyPath E:\keys\update-signing.key   直接寫到隨身碟
#
# - 私鑰：用來簽 update.json（tools\Sign-Update.ps1）。不要放進專案資料夾、不要上傳到任何地方；
#   備份到隨身碟或密碼管理器，平常可以不留在電腦上。私鑰不見了就不能再發佈更新（已安裝的 Stillfeather 不會接受）。
# - 公鑰：寫進 assets\update-signing-key.txt，要 commit 到 GitHub。建置時嵌進 Stillfeather，用來確認更新是你發佈的。
# - 已經有金鑰時不會覆寫。換金鑰之後，已經安裝的 Stillfeather 就收不到更新（要使用者手動重新安裝），
#   真的要換（例如私鑰外流）才加 -Force。
param(
    [string]$KeyPath = (Join-Path $env:USERPROFILE 'Stillfeather-signing\update-signing.key'),
    [switch]$Force
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$publicPath = Join-Path $root 'assets\update-signing-key.txt'
$KeyPath = [System.IO.Path]::GetFullPath($KeyPath)

if ($KeyPath.StartsWith($root.TrimEnd('\') + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "私鑰不能放在專案資料夾裡（會被一起上傳到 GitHub）。請用 -KeyPath 指定其他位置，例如隨身碟。"
}
$header = @(Get-Content -LiteralPath $publicPath -Encoding UTF8 -ErrorAction SilentlyContinue | Where-Object { $_.TrimStart().StartsWith('#') })
$existing = @(Get-Content -LiteralPath $publicPath -Encoding UTF8 -ErrorAction SilentlyContinue | Where-Object { $_.Trim() -and -not $_.TrimStart().StartsWith('#') })
if ($existing.Count -gt 0 -and -not $Force) {
    throw "assets\update-signing-key.txt 已經有公鑰。換金鑰之後，已經安裝的 Stillfeather 就收不到更新；真的要換才加 -Force。"
}
if ((Test-Path -LiteralPath $KeyPath) -and -not $Force) {
    throw "已經有私鑰：$KeyPath（不會覆寫）。"
}

$params = New-Object System.Security.Cryptography.CngKeyCreationParameters
$params.ExportPolicy = [System.Security.Cryptography.CngExportPolicies]::AllowPlaintextExport
$params.KeyUsage = [System.Security.Cryptography.CngKeyUsages]::Signing
$key = [System.Security.Cryptography.CngKey]::Create([System.Security.Cryptography.CngAlgorithm]::ECDsaP256, $null, $params)
try {
    $private = $key.Export([System.Security.Cryptography.CngKeyBlobFormat]::EccPrivateBlob)
    $public = $key.Export([System.Security.Cryptography.CngKeyBlobFormat]::EccPublicBlob)
} finally {
    $key.Dispose()
}
# BCRYPT_ECCPRIVATE_BLOB（ECS2，8 + 32×3 位元組）與 BCRYPT_ECCPUBLIC_BLOB（ECS1，8 + 32×2 位元組）
if ($private.Length -ne 104 -or $public.Length -ne 72 -or [System.Text.Encoding]::ASCII.GetString($public, 0, 4) -ne 'ECS1') {
    throw "產生的金鑰格式不符（私鑰 $($private.Length)、公鑰 $($public.Length) 位元組）。"
}

$keyDir = Split-Path -Parent $KeyPath
New-Item -ItemType Directory -Force -Path $keyDir | Out-Null
$privateText = "# Stillfeather 更新簽章的私鑰（ECDSA P-256，BCRYPT_ECCPRIVATE_BLOB，Base64）。不要上傳、不要分享。`r`n" +
    "# 建立時間：$((Get-Date).ToString('yyyy-MM-dd HH:mm'))；對應的公鑰在 Stillfeather 專案的 assets\update-signing-key.txt。`r`n" +
    [Convert]::ToBase64String($private) + "`r`n"
[System.IO.File]::WriteAllText($KeyPath, $privateText, (New-Object System.Text.UTF8Encoding($false)))

if ($header.Count -eq 0) {
    $header = @('# Stillfeather 自動更新的公鑰（ECDSA P-256，Windows CNG 的 BCRYPT_ECCPUBLIC_BLOB，Base64）。')
}
$publicText = (($header + [Convert]::ToBase64String($public)) -join "`n") + "`n"
[System.IO.File]::WriteAllText($publicPath, $publicText, (New-Object System.Text.UTF8Encoding($false)))

Write-Host ''
Write-Host "私鑰：$KeyPath" -ForegroundColor Green
Write-Host "公鑰：$publicPath" -ForegroundColor Green
Write-Host ''
Write-Host '接下來：'
Write-Host '  1. 把私鑰備份到隨身碟或密碼管理器（不見了就不能再發佈更新）。平常可以不留在這台電腦上。'
Write-Host '  2. 用 GitHub Desktop commit 並 push assets\update-signing-key.txt。'
Write-Host '  3. 之後每次 BuildSetup.cmd 會自動簽 update.json（私鑰在預設位置時）；否則執行 tools\Sign-Update.ps1 -KeyPath <私鑰>。'
