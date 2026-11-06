param([string]$Output='build/landcover-bezier-fit-fuji-v5',[switch]$Build,[switch]$GeometricFit,[switch]$FairBoundary,[switch]$SourceLevels,[string]$SourceRoot="build/landcover-raster-fuji-v4",[string]$Filters="majority,regions")
$ErrorActionPreference='Stop'
$root=Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
Set-Location $root
if($Build){& ./tools/terrain-rs/build.ps1 -LandcoverBezierFit;if($LASTEXITCODE){throw 'Rust build failed'}}
$libs=Join-Path $root 'build/terrain-venv/Lib/site-packages/rasterio.libs'
$env:PATH="$libs;$env:PATH"
$geos=(Get-ChildItem $libs -Filter 'geos_c-*.dll' | Select-Object -First 1).FullName
$fitArgs=@();if($GeometricFit){$fitArgs+=@('--geometric-fit')};if($FairBoundary){$fitArgs+=@('--fair-boundary')};if($SourceLevels){$fitArgs+=@('--source-levels')}
& ./build/terrain-rust-target/release/terrain-landcover-bezier-fit.exe --geos $geos --source-root $SourceRoot --filters $Filters --output $Output @fitArgs
if($LASTEXITCODE){throw "Bezier generation failed: $LASTEXITCODE"}
