param(
    [string]$OutputDirectory = 'build/terrain-national-xyz-20261024',
    [ValidateRange(1, 64)][int]$ChunkWorkers = 4,
    [ValidateRange(1, 64)][int]$BoundaryWorkers = 4,
    [ValidateRange(1, 64)][int]$PackWorkers = 4
)
$ErrorActionPreference = 'Stop'
$terrainRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
Set-Location -LiteralPath $terrainRoot
$targetRoot = [IO.Path]::GetFullPath((Join-Path $terrainRoot $OutputDirectory))
if (-not $targetRoot.StartsWith($terrainRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw '出力先は terrain 以下に指定してください'
}
New-Item -ItemType Directory -Path $targetRoot -Force | Out-Null
$running = Get-CimInstance Win32_Process | Where-Object {
    $_.Name -like 'terrain-xyz-region-test*' -and $_.CommandLine -and $_.CommandLine.Contains($targetRoot)
}
if ($running) {
    throw "同じ出力を生成中です。PID: $($running.ProcessId -join ', ')"
}
$sourcePath = Join-Path $terrainRoot 'build/terrain-lod.sqlite3'
$sourceHash = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
$tag = Get-Date -Format 'yyyyMMdd-HHmmss'
$binaryDirectory = Join-Path $targetRoot 'bin'
New-Item -ItemType Directory -Path $binaryDirectory -Force | Out-Null
$binaryPath = Join-Path $binaryDirectory "terrain-xyz-region-test-$tag.exe"
Copy-Item -LiteralPath (Join-Path $terrainRoot 'build/terrain-q-cache-target/release/terrain-xyz-region-test.exe') -Destination $binaryPath
$stdoutPath = Join-Path $targetRoot "generate-$tag.stdout.log"
$stderrPath = Join-Path $targetRoot "generate-$tag.stderr.log"
$arguments = @('--national', '--national-chunk-cells', '4096', '--national-workers', "$ChunkWorkers", '--national-boundary-workers', "$BoundaryWorkers", '--national-pack-workers', "$PackWorkers", '--source', $sourcePath, '--source-sha256', $sourceHash, '--output', $targetRoot)
$quotedArguments = $arguments | ForEach-Object { '"' + $_ + '"' }
$job = Start-Process -FilePath $binaryPath -ArgumentList $quotedArguments -WorkingDirectory $terrainRoot -WindowStyle Hidden -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath -PassThru
$launch = [ordered]@{
    pid = $job.Id
    started = (Get-Date).ToString('o')
    executable = $binaryPath
    executableSha256 = (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    sourceSha256 = $sourceHash
    arguments = $arguments
    stdout = $stdoutPath
    stderr = $stderrPath
    progressUrl = 'http://127.0.0.1:61462/'
}
[IO.File]::WriteAllText((Join-Path $targetRoot 'launch.json'), ($launch | ConvertTo-Json -Depth 5) + "`n", [Text.UTF8Encoding]::new($false))
$launch | ConvertTo-Json -Depth 5
