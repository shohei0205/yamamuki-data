param([string]$Database, [switch]$Build, [string]$Landcover)
$ErrorActionPreference = 'Stop'
$terrain = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (-not $Database -and -not $Landcover) {
    $Database = Join-Path $PSScriptRoot 'terrain-shared.sqlite3'
    if (-not (Test-Path -LiteralPath $Database -PathType Leaf)) {
        $Database = Join-Path $terrain 'build/national-shared-z6-z13-v1/terrain-shared.sqlite3'
    }
}
$release = Join-Path $terrain 'build/terrain-shared-viewer-target/release'
$regular = Join-Path $release 'terrain-shared-viewer.exe'
$staged = Join-Path $release 'terrain-shared-viewer-nearest.exe'
$newest = @($regular, $staged) |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
    ForEach-Object { Get-Item -LiteralPath $_ } |
    Sort-Object LastWriteTimeUtc -Descending |
    Select-Object -First 1
if ($Build -or -not $newest) {
    & (Join-Path $PSScriptRoot 'build.ps1') -Shared
    if ($LASTEXITCODE -ne 0) { throw 'ビューワーのビルドが失敗しました。' }
    $binary = $regular
} else {
    $binary = $newest.FullName
}
$arguments = @()
foreach ($entry in @(@('--database', $Database), @('--landcover', $Landcover))) {
    if (!$entry[1]) { continue }
    if (!(Test-Path -LiteralPath $entry[1] -PathType Leaf)) { throw "SQLiteがありません: $($entry[1])" }
    $arguments += $entry[0], ('"' + (Resolve-Path -LiteralPath $entry[1]).Path + '"')
}
Start-Process -FilePath $binary -ArgumentList $arguments -WorkingDirectory $terrain
