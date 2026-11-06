param(
    [Parameter(Mandatory)][string]$Source,
    [Parameter(Mandatory)][string]$Plan,
    [Parameter(Mandatory)][string]$Output,
    [string]$OnlyArea
)
$ErrorActionPreference = 'Stop'
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
Set-Location $root
$Source = (Resolve-Path -LiteralPath $Source).Path
$libs = Join-Path $root 'build/terrain-venv/Lib/site-packages/rasterio.libs'
$gdal = (Get-ChildItem $libs -Filter 'gdal-*.dll' | Select-Object -First 1).FullName
$geos = 'C:/Program Files/QGIS 3.44.15/bin/geos_c.dll'
$env:PATH = "$libs;$(Split-Path -Parent $geos);$env:PATH"
$env:PROJ_DATA = Join-Path $root "build/terrain-venv/Lib/site-packages/rasterio/proj_data"
$raster = 'build/landcover-wide-target/release/terrain-landcover-raster.exe'
$fit = 'build/landcover-wide-target/release/terrain-landcover-bezier-fit.exe'
$coast = 'build/terrain-coast-source.union.wkb'
$world = 40075016.68557849
$side = 224000.0
$resolution = 15.625
$areas = @(Get-Content -LiteralPath $Plan -Raw | ConvertFrom-Json)
if ($OnlyArea) { $areas = @($areas | Where-Object { "$($_.x)-$($_.y)" -eq $OnlyArea }) }
if (!$areas.Count) { throw '作業区画がありません。' }
New-Item -ItemType Directory -Force -Path $Output | Out-Null
$lock = [IO.File]::Open((Join-Path $Output 'run.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
try {
    $failed = @()
    foreach ($area in $areas) {
        $name = "$($area.x)-$($area.y)"
        $region = Join-Path $Output $name
        if (Test-Path -LiteralPath (Join-Path $region 'finished.json')) { continue }
        New-Item -ItemType Directory -Force -Path $region | Out-Null
        try {
            $cx = [Math]::Round((($area.x + 0.5) * $world / 256 - $world / 2) / $resolution) * $resolution
            $cy = [Math]::Round(($world / 2 - ($area.y + 0.5) * $world / 256) / $resolution) * $resolution
            $half = $side / 2
            $baseline = Join-Path $region 'baseline'
            New-Item -ItemType Directory -Force -Path $baseline | Out-Null
            $metadata = Get-Content 'build/landcover-vector-fuji-v2/manifest.json' -Raw | ConvertFrom-Json -AsHashtable
            $metadata.bounds = @(($cx-$half), ($cy-$half), ($cx+$half), ($cy+$half))
            $metadata.centerLonLat = @(($cx / $world * 360), ([Math]::Atan([Math]::Sinh($cy / $world * 2 * [Math]::PI)) * 180 / [Math]::PI))
            $metadata.title = "全国地表 作業区画 z8/$($area.x)/$($area.y)"
            $metadata.side = $side
            $metadata.sourceFile = [IO.Path]::GetFileName($Source)
            $metadata.Remove('relief')
            $metadata | ConvertTo-Json -Depth 30 | Set-Content -Encoding utf8 (Join-Path $baseline 'manifest.json')
            foreach ($level in 3..6) {
                $sourceLevel = [Math]::Min(5,$level)
                $raw = Join-Path $region "raw-l$sourceLevel"
                $stage = Join-Path $region "stage-l$level"
                $final = Join-Path $region "l$level"
                if (Test-Path -LiteralPath (Join-Path $final 'manifest.json')) { continue }
                Write-Output "$(Get-Date -Format o) area=$name L$level"
                if (!(Test-Path -LiteralPath (Join-Path $raw 'manifest.json'))) {
                    if (Test-Path -LiteralPath $raw) {
                        $raw = "$raw-retry-$([DateTime]::UtcNow.Ticks)"
                    }
                    & $raster --source $Source --coast $coast --baseline $baseline --output $raw --gdal $gdal --geos $geos --size 14336 --only-filter majority-clean --progressive-lod --filter-span-m 62.5 --only-level $sourceLevel
                    if ($LASTEXITCODE) { throw "Raster failed L$sourceLevel" }
                }
                $flags = @('--geos', $geos, '--filters', 'majority-clean', '--source-levels',
                           '--only-level', "$level", '--compact', '--implicit-background')
                if (!(Test-Path -LiteralPath (Join-Path $stage 'manifest.json'))) {
                    if (Test-Path -LiteralPath $stage) { $stage = "$stage-retry-$([DateTime]::UtcNow.Ticks)" }
                    & $fit @flags --source-root $raw --output $stage --water-overlay --unified-water --direct-lines --common-lod
                    if ($LASTEXITCODE) { throw "Fit failed L$level" }
                }
                if (Test-Path -LiteralPath $final) { $final = "$final-retry-$([DateTime]::UtcNow.Ticks)" }
                & $fit @flags --source-root $stage --output $final --linear-source --common-finish
                if ($LASTEXITCODE) { throw "Finish failed L$level" }
            }
            @{x=$area.x;y=$area.y;stage='regional-vectors-ready'} | ConvertTo-Json -Compress |
                Set-Content -Encoding utf8 (Join-Path $region 'finished.json')
        } catch {
            $failed += $name
            Write-Warning "$name $($_.Exception.Message)"
        }
    }
    if ($failed.Count) { throw "Failed areas: $($failed -join ', ')" }
} finally {
    $lock.Dispose()
}
