param(
    [string]$Output = 'build/xyz-regional-200km-local-fast-20261024',
    [string]$TargetDirectory = 'build/xyz-regional-200km-local-fast-20261024/engine-sharing-retry'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
Set-Location -LiteralPath $root
$out = [IO.Path]::GetFullPath((Join-Path $root $Output))
# 既存の地域生成・検証がすべて終了してから同じ出力を使用する。
while ($true) {
    try { $lock = [IO.File]::Open((Join-Path $out 'run.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None); break }
    catch [IO.IOException] { Start-Sleep -Seconds 1 }
}
$state = Get-Content (Join-Path $out 'run.json') -Raw | ConvertFrom-Json
if ($state.stage -ne 'tests-finished') { throw '既存試験の終了状態が確認できません' }
$region = Join-Path $out 'yari'
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
foreach ($name in @('generate.log','generate.err','generate.launch.json','progress.json')) {
    $path = Join-Path $region $name
    if (Test-Path -LiteralPath $path) { Copy-Item -LiteralPath $path -Destination ($path+'.failed-'+$stamp) }
}
$exe = Join-Path $root "$TargetDirectory/release/terrain-xyz-region-test.exe"
$source = Join-Path $root 'build/terrain-lod.sqlite3'
$hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
$generatorArguments = @('--source', ('"{0}"' -f $source), '--output', ('"{0}"' -f $out), '--regions', 'yari', '--extent', '200000', '--source-sha256', $hash)
$p = Start-Process -FilePath $exe -ArgumentList $generatorArguments -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput (Join-Path $region 'generate.log') -RedirectStandardError (Join-Path $region 'generate.err') -PassThru
@{pid=$p.Id;exe=$exe;arguments=$generatorArguments;started=(Get-Date).ToString('o');sourceSha256=$hash} | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $region 'generate.launch.json') -Encoding utf8
$results = @($state.results | Where-Object {$_.region -ne 'yari'})
@{stage='generating';region='yari';pid=$p.Id;results=$results;started=$state.started;retryStarted=(Get-Date).ToString('o')} | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $out 'run.json') -Encoding utf8
$p.WaitForExit(); $p.Refresh()
$generationExit = $p.ExitCode
if ($null -eq $generationExit) { throw '終了コードを取得できません' }
$verificationExit = $null
if ($generationExit -eq 0) {
    & (Join-Path $root 'build/terrain-venv/Scripts/python.exe') -X utf8 tools/terrain/verify_xyz_region.py (Join-Path $region 'terrain.sqlite3') *> (Join-Path $region 'verify.log')
    $verificationExit = $LASTEXITCODE
}
$results += @{region='yari';generationExit=$generationExit;verificationExit=$verificationExit}
@{stage='tests-finished';started=$state.started;finished=(Get-Date).ToString('o');results=$results;passed=(@($results|Where-Object {$_.generationExit -ne 0 -or $_.verificationExit -ne 0}).Count -eq 0)} | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $out 'run.json') -Encoding utf8
