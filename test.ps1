$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw '找不到 cargo，請先執行 .\build.ps1 查看安裝說明。' }
    & cargo test --lib
    if ($LASTEXITCODE -ne 0) { throw '核心測試失敗。' }
    & cargo test --bin FeatherFiles
    if ($LASTEXITCODE -ne 0) { throw 'Win32 版面測試失敗。' }
    Write-Host '全部測試通過。'
} finally {
    Pop-Location
}
