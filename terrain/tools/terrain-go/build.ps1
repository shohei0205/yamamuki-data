param([switch]$Test, [switch]$Vet, [string]$Output)
$ErrorActionPreference = 'Stop'
$terrainRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$localGo = Join-Path $terrainRoot 'build/go-tools/go/bin/go.exe'
$go = if (Test-Path -LiteralPath $localGo) { $localGo } else { (Get-Command go -ErrorAction Stop).Source }
$env:GOCACHE = Join-Path $terrainRoot 'build/go-tools/cache'
$env:GOMODCACHE = Join-Path $terrainRoot 'build/go-tools/modules'
$env:GOTOOLCHAIN = 'local'
if ($Test) { & $go -C $PSScriptRoot test -count=1 ./... }
elseif ($Vet) { & $go -C $PSScriptRoot vet ./... }
else {
    if (!$Output) { $Output = Join-Path $terrainRoot 'build/terrain-go/terrain-q-national.exe' }
    $outputPath = [System.IO.Path]::GetFullPath($Output)
    New-Item -ItemType Directory -Force -Path (Split-Path $outputPath -Parent) | Out-Null
    & $go -C $PSScriptRoot build -trimpath -o $outputPath .
}
exit $LASTEXITCODE
