param([string]$Database, [switch]$Build)
$ErrorActionPreference = 'Stop'
$terrain = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (-not $Database) {
    $Database = Join-Path $PSScriptRoot 'terrain-shared.sqlite3'
    if (-not (Test-Path -LiteralPath $Database -PathType Leaf)) {
        $Database = Join-Path $terrain 'build/national-shared-z6-z13-v1/terrain-shared.sqlite3'
    }
}
$binary = Join-Path $terrain 'build/terrain-viewer-target/release/terrain-shared-viewer.exe'
if ($Build -or -not (Test-Path -LiteralPath $binary)) {
    & (Join-Path $PSScriptRoot 'build.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'ビューワーのビルドが失敗しました。' }
}
if (-not (Test-Path -LiteralPath $Database -PathType Leaf)) { throw "SQLiteがありません: $Database" }
Start-Process -FilePath $binary -ArgumentList @('--database', '"' + (Resolve-Path -LiteralPath $Database).Path + '"') -WorkingDirectory $terrain
