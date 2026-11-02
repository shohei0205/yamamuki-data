param(
    [string]$Output = 'build/natural-xyz-hierarchy-z13-25-upquarter-local-v1',
    [string]$ReuseFinest = '',
    [int]$Workers = [Math]::Max(1, [Math]::Floor([Environment]::ProcessorCount / 2)),
    [switch]$Build,
    [switch]$HierarchyLocalBudget = $true,
    [switch]$FineFitFullBudget = $true,
    [double]$UpwardErrorRatio = 0.25,
    [string]$LevelProfile = 'z13-25'
)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'start_regional_xyz_rust.ps1') -Output $Output -ReuseFinest $ReuseFinest -Workers $Workers -Hierarchy -LevelProfile $LevelProfile -Build:$Build -UpwardErrorRatio $UpwardErrorRatio -HierarchyLocalBudget:$HierarchyLocalBudget -FineFitFullBudget:$FineFitFullBudget

$taskTerrainRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$taskActiveOutput = if ([IO.Path]::IsPathRooted($Output)) { [IO.Path]::GetFullPath($Output) } else { [IO.Path]::GetFullPath((Join-Path $taskTerrainRoot $Output)) }
$taskActivePath = Join-Path $taskTerrainRoot 'build/natural-active-generation.json'
$taskActiveTemporary = $taskActivePath + '.next'
@{output=$taskActiveOutput;generationMode='fine-to-coarse'} | ConvertTo-Json | Set-Content -LiteralPath $taskActiveTemporary -Encoding utf8
Move-Item -LiteralPath $taskActiveTemporary -Destination $taskActivePath -Force
