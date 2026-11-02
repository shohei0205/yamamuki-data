param([string]$Prepared = 'build/national-shared-z6-z13-v1')
$ErrorActionPreference = 'Stop'
$taskTerrainRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$taskPrepared = if ([IO.Path]::IsPathRooted($Prepared)) { [IO.Path]::GetFullPath($Prepared) } else { [IO.Path]::GetFullPath((Join-Path $taskTerrainRoot $Prepared)) }
$taskTarget = Join-Path $taskTerrainRoot 'build/natural-fixed-lpf-target'
$taskTestLog = Join-Path $taskTerrainRoot 'build/shared-national-qualification-tests.log'
& (Join-Path $taskTerrainRoot 'tools/terrain-rs/build.ps1') -NaturalSharedNational -Test -TargetDirectory $taskTarget *> $taskTestLog
if ($LASTEXITCODE -ne 0) { throw 'Rustテストが未達です。' }
& (Join-Path $taskTerrainRoot 'tools/terrain-rs/build.ps1') -NaturalSharedNational -TargetDirectory $taskTarget
if ($LASTEXITCODE -ne 0) { throw 'releaseビルドが未達です。' }
$taskEngine = Join-Path $taskTarget 'release/terrain-natural-shared-national.exe'
foreach ($taskRegion in @('yari','fuji','tsukuba')) {
    & $taskEngine --prepared (Join-Path $taskTerrainRoot "build/national-shared-$taskRegion-pilot-v1") verify
    if ($LASTEXITCODE -ne 0) { throw "$taskRegion の最終監査が未達です。" }
}
& (Join-Path $taskTerrainRoot 'build/terrain-venv/Scripts/python.exe') -X utf8 (Join-Path $PSScriptRoot 'qualify_national_shared.py') --prepared $taskPrepared --engine $taskEngine --test-log $taskTestLog
if ($LASTEXITCODE -ne 0) { throw '全国実行の事前検証が未達です。' }
