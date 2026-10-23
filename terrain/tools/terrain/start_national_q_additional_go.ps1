param(
    [string]$BaseInput = 'build/terrain-q4-q12-basic-rust-20261022.sqlite3',
    [string]$Output = 'build/terrain-q4-q12-additional-go-free4096-20261023.sqlite3',
    [string]$FieldCacheInput = 'build/terrain-q4-q12-additional-rust-20261022.sqlite3',
    [double]$DensityFreeVertices = 4096,
    [double]$CacheGiB = 8,
    [ValidateRange(1, 65535)]
    [int]$Workers = [Math]::Max(1, [Math]::Floor([Environment]::ProcessorCount / 2))
)
$ErrorActionPreference = 'Stop'
$terrainRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$basePath = [System.IO.Path]::GetFullPath((Join-Path $terrainRoot $BaseInput))
$outputPath = [System.IO.Path]::GetFullPath((Join-Path $terrainRoot $Output))
$cachePath = [System.IO.Path]::GetFullPath((Join-Path $terrainRoot $FieldCacheInput))
$binary = Join-Path $terrainRoot 'build/terrain-go/terrain-q-national.exe'
if (!(Test-Path -LiteralPath $binary)) { throw 'tools/terrain-go/build.ps1で先にビルドしてください。' }
$stopFile = [System.IO.Path]::ChangeExtension($outputPath, 'stop')
if (Test-Path -LiteralPath $stopFile) { throw '停止要求が残っています。内容を確認してください。' }
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$stdout = [System.IO.Path]::ChangeExtension($outputPath, "$stamp.log")
$stderr = [System.IO.Path]::ChangeExtension($outputPath, "$stamp.err")
$arguments = @('--base-input',('"'+$basePath+'"'),'--output',('"'+$outputPath+'"'),'--field-cache-input',('"'+$cachePath+'"'),'--density-free-vertices',"$DensityFreeVertices",'--workers',"$Workers",'--cache-gib',"$CacheGiB",'--stop-file',('"'+$stopFile+'"'))
$process = Start-Process -FilePath $binary -ArgumentList $arguments -WorkingDirectory $terrainRoot -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
$process.Id | Set-Content -LiteralPath ([System.IO.Path]::ChangeExtension($outputPath, 'pid')) -Encoding ascii
@{ engine='go'; pid=$process.Id; base=$basePath; output=$outputPath; fieldCacheInput=$cachePath; workers=$Workers; cacheGiB=$CacheGiB; densityFreeVertices=$DensityFreeVertices; stdout=$stdout; stderr=$stderr; timeLimit=$false } | ConvertTo-Json | Set-Content -LiteralPath ([System.IO.Path]::ChangeExtension($outputPath, 'launch.json')) -Encoding utf8
Write-Output "全国追加Go版を起動しました。PID=$($process.Id) ログ=$stdout"
