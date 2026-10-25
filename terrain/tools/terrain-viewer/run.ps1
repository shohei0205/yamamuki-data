param([string]$Database, [switch]$Build)
$ErrorActionPreference = 'Stop'
$terrain = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (-not $Database) { $Database = Join-Path $terrain 'build/terrain-national-xyz-20261024/terrain.sqlite3' }
$binary = Join-Path $terrain 'build/terrain-viewer-target/release/terrain-viewer.exe'
if ($Build -or -not (Test-Path -LiteralPath $binary)) {
    & (Join-Path $PSScriptRoot 'build.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'ビルドに失敗しました。' }
}
if (-not (Test-Path -LiteralPath $Database -PathType Leaf)) { throw "SQLite がありません: $Database" }
Start-Process -FilePath $binary -ArgumentList @('"' + (Resolve-Path -LiteralPath $Database).Path + '"') -WorkingDirectory $terrain
