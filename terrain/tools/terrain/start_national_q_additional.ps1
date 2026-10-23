param(
    [string]$BaseInput = 'build/terrain-q4-q12-basic-rust-20261022.sqlite3',
    [string]$Output = 'build/terrain-q4-q12-additional-rust-20261022.sqlite3',
    [double]$DensityFreeVertices = 0,
    [string]$FieldCacheInput = '',
    [double]$MaxSeconds = 0,
    [ValidateRange(1, 65535)]
    [int]$Workers = [Math]::Max(1, [Math]::Floor([Environment]::ProcessorCount / 2))
)
$ErrorActionPreference = 'Stop'
$terrainRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$outputPath = [System.IO.Path]::GetFullPath((Join-Path $terrainRoot $Output))
$basePath = [System.IO.Path]::GetFullPath((Join-Path $terrainRoot $BaseInput))
$pidFile = [System.IO.Path]::ChangeExtension($outputPath, 'pid')
if (Test-Path -LiteralPath $pidFile) {
    $existingId = [int](Get-Content -LiteralPath $pidFile)
    $existing = Get-CimInstance Win32_Process -Filter "ProcessId=$existingId"
    if ($existing -and $existing.CommandLine -like '*terrain-q-national*') {
        throw '同じ追加工程が起動しています。進捗とログを確認してください。'
    }
}
$binary = Join-Path $terrainRoot 'build/terrain-q-additional-target/release/terrain-q-national.exe'
if (!(Test-Path -LiteralPath $binary)) { throw '追加工程用のRust生成器を先にビルドしてください。' }
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$stdout = [System.IO.Path]::ChangeExtension($outputPath, "$stamp.log")
$stderr = [System.IO.Path]::ChangeExtension($outputPath, "$stamp.err")
$stopFile = [System.IO.Path]::ChangeExtension($outputPath, 'stop')
if (Test-Path -LiteralPath $stopFile) { throw '停止要求ファイルが残っています。再開前に内容を確認してください。' }
$arguments = @('--additional','--base-input',('"' + $basePath + '"'),'--output',('"' + $outputPath + '"'),'--max-additional-seconds',"$MaxSeconds",'--workers',"$Workers",'--density-free-vertices',"$DensityFreeVertices",'--stop-file',('"' + $stopFile + '"'))
if ($FieldCacheInput) {
    $cachePath = [System.IO.Path]::GetFullPath((Join-Path $terrainRoot $FieldCacheInput))
    $arguments += @('--field-cache-input',('"' + $cachePath + '"'))
}
$process = Start-Process -FilePath $binary -ArgumentList $arguments -WorkingDirectory $terrainRoot -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
$process.Id | Set-Content -LiteralPath $pidFile -Encoding ascii
@{ engine='rust'; pid=$process.Id; base=$basePath; output=$outputPath; stdout=$stdout; stderr=$stderr; maxSeconds=$MaxSeconds; workers=$Workers; densityFreeVertices=$DensityFreeVertices; fieldCacheInput=$FieldCacheInput; stage='national-additional' } | ConvertTo-Json | Set-Content -LiteralPath ([System.IO.Path]::ChangeExtension($outputPath, 'launch.json')) -Encoding utf8
Write-Output "全国追加工程を非表示で起動しました。PID=$($process.Id) ログ=$stdout"
