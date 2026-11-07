param(
    [Parameter(Mandatory)][string]$Source,
    [Parameter(Mandatory)][string]$Plan,
    [Parameter(Mandatory)][string]$Output,
    [string]$OnlyArea,
    [string]$FitExecutable = 'build/landcover-wide-target/release/terrain-landcover-bezier-fit.exe',
    [ValidateRange(1, 4)][int]$FitJobs = 2,
    [ValidateRange(1, 2)][int]$WorkerCount = 1,
    [ValidateRange(0, 1)][int]$WorkerIndex = 0
)
$ErrorActionPreference = 'Stop'
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
Set-Location $root
$Source = (Resolve-Path -LiteralPath $Source).Path
$qgis = 'C:/Program Files/QGIS 3.44.15'
$gdal = Join-Path $qgis 'bin/gdal313.dll'
$geos = Join-Path $qgis 'bin/geos_c.dll'
$env:PATH = "$qgis/bin;$env:PATH"
$env:PROJ_DATA = Join-Path $qgis 'share/proj'
$env:RAYON_NUM_THREADS = [string][Math]::Max(1, [Math]::Floor([Environment]::ProcessorCount / (2 * $WorkerCount)))
$raster = 'build/landcover-wide-target/release/terrain-landcover-raster.exe'
$fit = (Resolve-Path -LiteralPath $FitExecutable).Path
$coast = 'build/terrain-coast-source.union.wkb'
$world = 40075016.68557849
$side = 224000.0
$resolution = 15.625
$areas = @(Get-Content -LiteralPath $Plan -Raw | ConvertFrom-Json)
if ($WorkerIndex -ge $WorkerCount) { throw 'ワーカー番号が範囲外です。' }
if ($OnlyArea) {
    if ($WorkerCount -ne 1) { throw '単一区画指定と複数ワーカーは併用できません。' }
    $areas = @($areas | Where-Object { "$($_.x)-$($_.y)" -eq $OnlyArea })
} elseif ($WorkerCount -gt 1) {
    $allAreas = $areas
    $areas = @(for ($i = $WorkerIndex; $i -lt $allAreas.Count; $i += $WorkerCount) { $allAreas[$i] })
}
if (!$areas.Count) { throw '作業区画がありません。' }
New-Item -ItemType Directory -Force -Path $Output | Out-Null
$share = if ($WorkerCount -gt 1) { 'ReadWrite' } else { 'None' }
$lock = [IO.File]::Open((Join-Path $Output 'run.lock'), 'OpenOrCreate', 'ReadWrite', $share)
$workerLock = $null
function Find-CompletedOutput([string]$Directory, [string]$Name) {
    $candidate = Get-ChildItem -LiteralPath $Directory -Directory -Filter "$Name*" |
        Where-Object { ($_.Name -eq $Name -or $_.Name.StartsWith("$Name-retry-")) -and
                       (Test-Path -LiteralPath (Join-Path $_.FullName 'manifest.json')) } |
        Sort-Object Name -Descending | Select-Object -First 1
    if ($candidate) { return $candidate.FullName }
}
try {
    if ($WorkerCount -gt 1) {
        $workerLock = [IO.File]::Open((Join-Path $Output "worker-$WorkerIndex.lock"), 'OpenOrCreate', 'ReadWrite', 'None')
    }
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
            $existingRawL3 = Join-Path $region 'raw-l3'
            $reuseL3 = Test-Path -LiteralPath (Join-Path $existingRawL3 'manifest.json')
            $raw = Find-CompletedOutput $region 'raw-shared'
            if (!$raw) {
                $raw = Join-Path $region 'raw-shared'
                if (Test-Path -LiteralPath $raw) {
                    $raw = "$raw-retry-$([DateTime]::UtcNow.Ticks)"
                }
                $levels = if ($reuseL3) { '4,5' } else { '3,4,5' }
                Write-Output "$(Get-Date -Format o) area=$name raw L$levels"
                & $raster --source $Source --coast $coast --baseline $baseline --output $raw --gdal $gdal --geos $geos --size 14336 --only-filter majority-clean --progressive-lod --filter-span-m 62.5 --levels $levels
                if ($LASTEXITCODE) { throw "Raster failed L$levels" }
            }
            $tasks = foreach ($level in 3..6) {
                if (Find-CompletedOutput $region "l$level") { continue }
                $stage = Find-CompletedOutput $region "stage-l$level"
                $stageReady = [bool]$stage
                if (!$stage) {
                    $stage = Join-Path $region "stage-l$level"
                    if (Test-Path -LiteralPath $stage) { $stage = "$stage-retry-$([DateTime]::UtcNow.Ticks)" }
                }
                $final = Join-Path $region "l$level"
                if (Test-Path -LiteralPath $final) { $final = "$final-retry-$([DateTime]::UtcNow.Ticks)" }
                [pscustomobject]@{
                    Level = $level
                    Raw = if ($level -eq 3 -and $reuseL3) { $existingRawL3 } else { $raw }
                    Stage = $stage
                    StageReady = $stageReady
                    Final = $final
                }
            }
            $fitErrors = [Collections.Generic.List[string]]::new()
            @($tasks | Where-Object { $null -ne $_ }) | ForEach-Object -ThrottleLimit $FitJobs -Parallel {
                $ErrorActionPreference = 'Continue'
                $PSNativeCommandUseErrorActionPreference = $false
                Set-Location $using:root
                $task = $_
                $engine = $using:fit
                $flags = @('--geos', $using:geos, '--filters', 'majority-clean', '--source-levels',
                           '--only-level', "$($task.Level)", '--compact', '--implicit-background')
                try {
                    Write-Output "$(Get-Date -Format o) area=$using:name L$($task.Level)"
                    if (!$task.StageReady) {
                        & $engine @flags --source-root $task.Raw --output $task.Stage --water-overlay --unified-water --direct-lines --common-lod
                        if ($LASTEXITCODE) { throw "Fit failed L$($task.Level)" }
                    }
                    & $engine @flags --source-root $task.Stage --output $task.Final --linear-source --common-finish
                    if ($LASTEXITCODE) { throw "Finish failed L$($task.Level)" }
                    if (!(Test-Path -LiteralPath (Join-Path $task.Final 'manifest.json'))) {
                        throw "Missing final manifest L$($task.Level)"
                    }
                } catch {
                    [pscustomobject]@{ FitError = $_.Exception.Message }
                }
            } | ForEach-Object {
                if ($_ -is [string]) { Write-Output $_ }
                elseif ($_.FitError) { $fitErrors.Add($_.FitError) }
                else { Write-Output $_ }
            }
            if ($fitErrors.Count) { throw ($fitErrors -join ', ') }
            @{x=$area.x;y=$area.y;stage='regional-vectors-ready'} | ConvertTo-Json -Compress |
                Set-Content -Encoding utf8 (Join-Path $region 'finished.json')
        } catch {
            $failed += $name
            Write-Warning "$name $($_.Exception.Message)"
        }
    }
    if ($failed.Count) { throw "Failed areas: $($failed -join ', ')" }
} finally {
    if ($workerLock) { $workerLock.Dispose() }
    $lock.Dispose()
}
