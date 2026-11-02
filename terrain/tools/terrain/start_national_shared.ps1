param(
    [string]$Prepared = 'build/national-shared-z6-z13-v1',
    [ValidateSet('Init','Run','Verify')][string]$Mode = 'Init',
    [int[]]$Window = @(),
    [int]$LocalPointLimit = 131072,
    [switch]$Build,
    [switch]$Foreground
)
$ErrorActionPreference = 'Stop'
$taskTerrainRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$taskOutput = if ([IO.Path]::IsPathRooted($Prepared)) { [IO.Path]::GetFullPath($Prepared) } else { [IO.Path]::GetFullPath((Join-Path $taskTerrainRoot $Prepared)) }
$taskJobPath = Join-Path $taskOutput 'job.json'
if (-not (Test-Path -LiteralPath $taskJobPath)) { throw '先にprepare_national_shared.ps1で入力を検証してください。' }
$taskJob = Get-Content -LiteralPath $taskJobPath -Raw | ConvertFrom-Json
if (-not $taskJob.inputs_verified) { throw '全PNGのSHA256照合が必要です。' }
if ($Window.Count -and ($Mode -ne 'Run' -or $Window.Count -ne 4)) { throw 'WindowはRunのときにz14の左上・右下4値を指定してください。' }
$taskTarget = Join-Path $taskTerrainRoot 'build/natural-fixed-lpf-target'
if ($Build) {
    & (Join-Path $taskTerrainRoot 'tools/terrain-rs/build.ps1') -NaturalSharedNational -TargetDirectory $taskTarget
    if ($LASTEXITCODE -ne 0) { throw 'Rust releaseビルドが失敗しました。' }
}
$taskBinary = Join-Path $taskTarget 'release/terrain-natural-shared-national.exe'
if (-not (Test-Path -LiteralPath $taskBinary)) { throw 'Buildを指定してRust release版を作成してください。' }
if (Test-Path -LiteralPath (Join-Path $taskOutput 'stop-requested')) { throw '停止要求が残っています。再開時はstop-requestedを取り除いてください。' }
# 二重起動の最終判定はエンジン側のSQLite排他ロック。
$taskExisting = Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -like '*shared-national*' -and $_.CommandLine -like ('*' + $taskOutput + '*') -and ($_.Name -eq 'terrain-natural-shared-national.exe' -or $_.Name -like 'engine-shared-national-*.exe') }
if ($taskExisting) { throw '同じ作業出力のプロセスが動いています。' }
$taskStamp = Get-Date -Format 'yyyyMMdd-HHmmss-ffff'
$taskEngine = Join-Path $taskOutput ("engine-shared-national-$taskStamp.exe")
Copy-Item -LiteralPath $taskBinary -Destination $taskEngine
$taskArgs = @('--prepared',$taskOutput,'--local-point-limit',"$LocalPointLimit",$Mode.ToLowerInvariant())
if ($Window.Count) { $taskArgs += '--window'; $taskArgs += $Window | ForEach-Object { "$_" } }
if ($Foreground) {
    Push-Location -LiteralPath $taskTerrainRoot
    try { & $taskEngine @taskArgs; if ($LASTEXITCODE -ne 0) { throw '生成処理が停止しました。progress.jsonとチェックポイントを確認してください。' } }
    finally { Pop-Location }
    return
}
$taskQuotedArgs = $taskArgs | ForEach-Object { if ($_ -match '"') { throw '引数に二重引用符は使えません。' }; '"' + $_ + '"' }
$taskStdout = Join-Path $taskOutput "engine-$taskStamp.stdout.log"
$taskStderr = Join-Path $taskOutput "engine-$taskStamp.stderr.log"
$taskProcess = Start-Process -FilePath $taskEngine -ArgumentList $taskQuotedArgs -WorkingDirectory $taskTerrainRoot -WindowStyle Hidden -RedirectStandardOutput $taskStdout -RedirectStandardError $taskStderr -PassThru
$taskLaunch = @{pid=$taskProcess.Id;mode=$Mode;engine=$taskEngine;engine_sha256=(Get-FileHash -LiteralPath $taskEngine -Algorithm SHA256).Hash;stdout=$taskStdout;stderr=$taskStderr;prepared=$taskOutput;window=$Window;started=(Get-Date).ToUniversalTime().ToString('o')}
[IO.File]::WriteAllText((Join-Path $taskOutput 'launch.json'),($taskLaunch | ConvertTo-Json -Depth 5) + "`n",[Text.UTF8Encoding]::new($false))
$taskLaunch | ConvertTo-Json -Depth 5
