param(
    [string]$Source = 'build/terrain-national-natural-20261029/coverage-source.sqlite3',
    [string]$Cache = 'build/terrain-cache',
    [string]$Output = 'build/national-shared-z6-z13-v1',
    [string]$Settings = 'NATIONAL_SHARED_SETTINGS.json'
)
$ErrorActionPreference = 'Stop'
$taskTerrainRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$taskPython = Join-Path $taskTerrainRoot 'build/terrain-venv/Scripts/python.exe'
function Resolve-TaskPath([string]$Value) {
    if ([IO.Path]::IsPathRooted($Value)) { return [IO.Path]::GetFullPath($Value) }
    return [IO.Path]::GetFullPath((Join-Path $taskTerrainRoot $Value))
}
# 元データの取得・変更、旧生成器の起動は行わない。
& $taskPython -X utf8 (Join-Path $PSScriptRoot 'prepare_national_shared.py') --source (Resolve-TaskPath $Source) --cache (Resolve-TaskPath $Cache) --output (Resolve-TaskPath $Output) --settings (Resolve-TaskPath $Settings)
if ($LASTEXITCODE -ne 0) { throw '全国共通曲面の入力準備に失敗しました。' }
$taskJob = Get-Content -LiteralPath (Join-Path (Resolve-TaskPath $Output) 'job.json') -Raw | ConvertFrom-Json
if (-not $taskJob.ready_to_generate) { Write-Host '入力準備済み。全国生成は未開始です。job.json の実装ゲートを確認してください。' }
