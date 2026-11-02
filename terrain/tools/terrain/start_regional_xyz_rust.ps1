param(
    [string]$Output = 'build/natural-xyz-tight-errors-v1',
    [int]$Workers = [Math]::Max(1, [Environment]::ProcessorCount / 2),
    [switch]$Build,
    [string]$LevelProfile = 'original',
    [switch]$Hierarchy,
    [switch]$HierarchyLocalBudget,
    [switch]$FineFitFullBudget,
    [double]$UpwardErrorRatio = 1.0,
    [string]$ReuseFinest
)
$ErrorActionPreference = 'Stop'
$taskTerrainRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$taskTarget = Join-Path $taskTerrainRoot 'build/natural-fixed-lpf-target'
if ($Build) {
    & (Join-Path $taskTerrainRoot 'tools/terrain-rs/build.ps1') -NaturalRegional -TargetDirectory $taskTarget
    if ($LASTEXITCODE -ne 0) { throw 'Rustのビルドに失敗しました。' }
}
$taskBinary = Join-Path $taskTarget 'release/terrain-natural-regional.exe'
if (-not (Test-Path -LiteralPath $taskBinary)) { throw '先に -Build を指定してreleaseビルドを作成してください。' }
$taskOutput = if ([IO.Path]::IsPathRooted($Output)) { [IO.Path]::GetFullPath($Output) } else { [IO.Path]::GetFullPath((Join-Path $taskTerrainRoot $Output)) }
$taskExisting = Get-CimInstance Win32_Process | Where-Object { ($_.Name -like 'engine-rust-*.exe' -or $_.Name -eq 'terrain-natural-regional.exe') -and $_.CommandLine -like "*$taskOutput*" }
if ($taskExisting) { throw 'この出力先のRust生成プロセスは既に実行中です。' }
New-Item -ItemType Directory -Path $taskOutput -Force | Out-Null
$taskStamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$taskEngine = Join-Path $taskOutput "engine-rust-$taskStamp.exe"
Copy-Item -LiteralPath $taskBinary -Destination $taskEngine
$taskStdout = "rust-$taskStamp.stdout.log"
$taskStderr = "rust-$taskStamp.stderr.log"
$taskArgs = @('--output', ('"' + $taskOutput + '"'), '--cache', ('"' + (Join-Path $taskTerrainRoot 'build/terrain-cache') + '"'), '--coverage', ('"' + (Join-Path $taskTerrainRoot 'build/terrain-national-natural-20261029/coverage-source.sqlite3') + '"'), '--workers', "$Workers")
if ($Hierarchy) {
    $taskArgs += @('--hierarchy','--level-profile',$LevelProfile,'--upward-error-ratio',$UpwardErrorRatio.ToString([Globalization.CultureInfo]::InvariantCulture))
    # 再開時も、保存した局所誤差予算の設定を引き継ぐ。
    $taskSavedSettings = Join-Path $taskOutput 'settings.json'
    if ($HierarchyLocalBudget -or ((Test-Path -LiteralPath $taskSavedSettings) -and ((Get-Content -LiteralPath $taskSavedSettings -Raw | ConvertFrom-Json).hierarchyErrorPolicy -eq 'local-dem-envelope'))) {
        $taskArgs += '--hierarchy-local-budget'
    }
    if ($FineFitFullBudget -or ((Test-Path -LiteralPath $taskSavedSettings) -and ((Get-Content -LiteralPath $taskSavedSettings -Raw | ConvertFrom-Json).fineFitFullBudget -eq $true))) {
        $taskArgs += '--fine-fit-full-budget'
    }
    if ($ReuseFinest) {
        $taskReuse = if ([IO.Path]::IsPathRooted($ReuseFinest)) { [IO.Path]::GetFullPath($ReuseFinest) } else { [IO.Path]::GetFullPath((Join-Path $taskTerrainRoot $ReuseFinest)) }
        $taskArgs += @('--reuse-finest', ('"' + $taskReuse + '"'))
    }
}
$taskProcess = Start-Process -FilePath $taskEngine -ArgumentList $taskArgs -WorkingDirectory $taskTerrainRoot -WindowStyle Hidden -RedirectStandardOutput (Join-Path $taskOutput $taskStdout) -RedirectStandardError (Join-Path $taskOutput $taskStderr) -PassThru
$taskLaunch = @{pid=$taskProcess.Id;engine=$taskEngine;stdout=$taskStdout;stderr=$taskStderr;workers=$Workers;implementation=$(if ($Hierarchy) { 'Rust release fine-to-coarse' } else { 'Rust release' });restarted=(Get-Date).ToString('o')}
$taskLaunch | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $taskOutput 'launch.json') -Encoding utf8
$taskLaunch | ConvertTo-Json
