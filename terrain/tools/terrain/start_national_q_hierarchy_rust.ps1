param(
    [string]$BaseInput = 'build/terrain-q4-q12-basic-rust-20261022.sqlite3',
    [string]$Output = 'build/terrain-q4-q12-hierarchy-rust-20261023.sqlite3',
    [string]$FieldCacheInput = 'build/terrain-q4-q12-additional-rust-20261022.sqlite3',
    [ValidateRange(4, 12)][int]$MinimumQ = 4,
    [ValidateRange(4, 12)][int]$MaximumQ = 12,
    [ValidateRange(1, 65535)][int]$Workers = [Math]::Max(1, [Math]::Floor([Environment]::ProcessorCount / 2)),
    [double]$MinFreeGiB = 8
)
$ErrorActionPreference = 'Stop'
$terrainRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if ($MinimumQ -gt $MaximumQ) { throw 'レベル範囲が逆転しています。' }
if ([double]::IsNaN($MinFreeGiB) -or [double]::IsInfinity($MinFreeGiB) -or $MinFreeGiB -lt 0) { throw '空き容量の設定が不正です。' }
$outputPath = [IO.Path]::GetFullPath((Join-Path $terrainRoot $Output))
$basePath = (Resolve-Path -LiteralPath (Join-Path $terrainRoot $BaseInput)).Path
if ($outputPath -eq $basePath) { throw '基本入力と階層出力は分けてください。' }
$binary = Join-Path $terrainRoot 'build/terrain-q-additional-target/release/terrain-q-national.exe'
if (!(Test-Path -LiteralPath $binary)) { throw '階層生成対応のRust生成器をビルドしてください。' }
$pidFile = [IO.Path]::ChangeExtension($outputPath, 'pid')
if (Test-Path -LiteralPath $pidFile) {
    $existingId = [int](Get-Content -LiteralPath $pidFile)
    $existing = Get-CimInstance Win32_Process -Filter "ProcessId=$existingId"
    if ($existing -and $existing.CommandLine -like '*terrain-q-national*') { throw '同じ階層生成が起動しています。' }
}
$stopFile = [IO.Path]::ChangeExtension($outputPath, 'stop')
if (Test-Path -LiteralPath $stopFile) { throw '停止要求ファイルが残っています。内容を確認してください。' }
[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($outputPath)) | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$stdout = [IO.Path]::ChangeExtension($outputPath, "$stamp.log")
$stderr = [IO.Path]::ChangeExtension($outputPath, "$stamp.err")
$arguments = @('--hierarchical', '--base-input', ('"' + $basePath + '"'), '--output', ('"' + $outputPath + '"'), '--source', ('"' + (Join-Path $terrainRoot 'build/terrain-lod.sqlite3') + '"'), '--minimum-q', "$MinimumQ", '--maximum-q', "$MaximumQ", '--workers', "$Workers", '--min-free-gib', "$MinFreeGiB", '--density-free-vertices', '4096', '--max-additional-seconds', '0', '--stop-file', ('"' + $stopFile + '"'))
if ($FieldCacheInput) {
    $cachePath = (Resolve-Path -LiteralPath (Join-Path $terrainRoot $FieldCacheInput)).Path
    $arguments += @('--field-cache-input', ('"' + $cachePath + '"'))
}
$launch = @{ engine='rust'; mode='hierarchical-final-parent'; base=$basePath; output=$outputPath; stdout=$stdout; stderr=$stderr; workers=$Workers; minimumQ=$MinimumQ; maximumQ=$MaximumQ; started=(Get-Date).ToUniversalTime().ToString('o'); stage='launching' }
$launchPath = [IO.Path]::ChangeExtension($outputPath, 'launch.json')
try {
    $process = Start-Process -FilePath $binary -ArgumentList $arguments -WorkingDirectory $terrainRoot -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    $process.Id | Set-Content -LiteralPath $pidFile -Encoding ascii
    $launch.pid = $process.Id
    $launch.stage = 'started'
    @{ output=$outputPath; basic=$basePath } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $terrainRoot 'build/q-progress-selection.json') -Encoding utf8
} catch {
    $launch.stage = 'launch-failed'
    $launch.error = $_.Exception.Message
    throw
} finally {
    $launch | ConvertTo-Json | Set-Content -LiteralPath $launchPath -Encoding utf8
}
Write-Output "Rust階層生成を非表示で起動しました。PID=$($process.Id) ログ=$stdout"
