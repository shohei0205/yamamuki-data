param([string]$Output = 'build/landcover-raster-fuji-v4',[int]$Size=2048,[int]$MinimumCells=9,[switch]$Build,[string]$OnlyFilter,[switch]$ProgressiveLod,[int]$WaterDetail=1,[double]$FilterSpanM=0,[double]$ExtentM=0,[string]$Source='build/terrain-worldcover-source/ESA_WorldCover_10m_2021_v200_N33E138_Map.tif')
$ErrorActionPreference='Stop'
$root=Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
Set-Location $root
if($Build){& ./tools/terrain-rs/build.ps1 -LandcoverRaster;if($LASTEXITCODE){throw 'Rust build failed'}}
$libs=Join-Path $root 'build/terrain-venv/Lib/site-packages/rasterio.libs'
$env:PATH="$libs;$env:PATH"
$gdal=(Get-ChildItem $libs -Filter 'gdal-*.dll' | Select-Object -First 1).FullName
$geos=(Get-ChildItem $libs -Filter 'geos_c-*.dll' | Select-Object -First 1).FullName
$filterArgs=@();if($FilterSpanM -gt 0){$filterArgs+=@('--filter-span-m',[string]$FilterSpanM)};if($OnlyFilter){$filterArgs+=@('--only-filter',$OnlyFilter)};if($ProgressiveLod){$filterArgs+=@('--progressive-lod')}
if($ExtentM -gt 0){$filterArgs+=@('--extent-m',[string]$ExtentM)}
& ./build/terrain-rust-target/release/terrain-landcover-raster.exe --gdal $gdal --geos $geos --source $Source --coast build/terrain-coast-source.union.wkb --baseline build/landcover-vector-fuji-v2 --output $Output --size $Size --minimum-cells $MinimumCells --water-detail $WaterDetail @filterArgs
if($LASTEXITCODE){throw "Rust generation failed: $LASTEXITCODE"}
