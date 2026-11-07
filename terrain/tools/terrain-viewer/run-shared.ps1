param([string]$Database, [switch]$Build, [string]$Landcover)
$ErrorActionPreference = 'Stop'
$terrain = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (-not $Database -and -not $Landcover) {
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
$arguments = @()
foreach ($entry in @(@('--database', $Database), @('--landcover', $Landcover))) {
    if (!$entry[1]) { continue }
    if (!(Test-Path -LiteralPath $entry[1] -PathType Leaf)) { throw "SQLiteがありません: $($entry[1])" }
    $arguments += $entry[0], ('"' + (Resolve-Path -LiteralPath $entry[1]).Path + '"')
}
Start-Process -FilePath $binary -ArgumentList $arguments -WorkingDirectory $terrain
