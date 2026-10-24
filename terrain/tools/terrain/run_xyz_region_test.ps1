param(
    [string]$Output = 'build/xyz-regional-200km-20261024',
    [double]$Extent = 200000,
    [string]$TargetDirectory = 'build/terrain-xyz-test-target'
)
$ErrorActionPreference = 'Stop'
$terrainRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
Set-Location -LiteralPath $terrainRoot
$outputPath = [IO.Path]::GetFullPath((Join-Path $terrainRoot $Output))
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
$runLock = [IO.File]::Open((Join-Path $outputPath 'run.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
$exe = Join-Path $terrainRoot "$TargetDirectory/release/terrain-xyz-region-test.exe"
$python = Join-Path $terrainRoot 'build/terrain-venv/Scripts/python.exe'
$source = Join-Path $terrainRoot 'build/terrain-lod.sqlite3'
$sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
$started = (Get-Date).ToString('o')
$results = @()
foreach ($regionName in @('tsukuba', 'yari', 'fuji')) {
    $regionPath = Join-Path $outputPath $regionName
    New-Item -ItemType Directory -Path $regionPath -Force | Out-Null
    $free = [IO.DriveInfo]::new([IO.Path]::GetPathRoot($outputPath)).AvailableFreeSpace
    if ($free -lt 8GB) { throw '空き容量が8 GiB未満のため停止します。保存済み結果は保持します。' }
    $arguments = @('--source', ('"{0}"' -f $source), '--output', ('"{0}"' -f $outputPath), '--regions', $regionName, '--extent', $Extent.ToString([Globalization.CultureInfo]::InvariantCulture), '--source-sha256', $sourceHash)
    $process = Start-Process -FilePath $exe -ArgumentList $arguments -WorkingDirectory $terrainRoot -WindowStyle Hidden -RedirectStandardOutput (Join-Path $regionPath 'generate.log') -RedirectStandardError (Join-Path $regionPath 'generate.err') -PassThru
    @{ pid=$process.Id; exe=$exe; arguments=$arguments; sourceSha256=$sourceHash; started=(Get-Date).ToString('o') } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $regionPath 'generate.launch.json') -Encoding utf8
    @{ stage='generating'; region=$regionName; pid=$process.Id; started=$started; results=$results } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $outputPath 'run.json') -Encoding utf8
    $process.WaitForExit()
    $process.Refresh()
    $generationExit = $process.ExitCode
    if ($null -eq $generationExit) { throw "終了コードを取得できませんでした: $regionName" }
    $verificationExit = $null
    if ($generationExit -eq 0) {
        & $python -X utf8 tools/terrain/verify_xyz_region.py (Join-Path $regionPath 'terrain.sqlite3') *> (Join-Path $regionPath 'verify.log')
        $verificationExit = $LASTEXITCODE
    }
    $results += @{ region=$regionName; generationExit=$generationExit; verificationExit=$verificationExit }
}
@{ stage='tests-finished'; started=$started; finished=(Get-Date).ToString('o'); results=$results; passed=(@($results | Where-Object { $_.generationExit -ne 0 -or $_.verificationExit -ne 0 }).Count -eq 0) } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $outputPath 'run.json') -Encoding utf8
