param(
    [Parameter(Mandatory)][string]$Dem14,
    [Parameter(Mandatory)][string]$WorkDirectory,
    [Parameter(Mandatory)][string]$Version,
    [string]$WorldCoverDirectory,
    [ValidateRange(1,64)][int]$Jobs = 4
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$surface = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$Dem14 = (Resolve-Path -LiteralPath $Dem14).Path
if ($WorldCoverDirectory) { $WorldCoverDirectory = (Resolve-Path -LiteralPath $WorldCoverDirectory).Path }
$WorkDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($WorkDirectory)
if (Test-Path -LiteralPath $WorkDirectory) { throw "新しい作業ディレクトリを指定してください: $WorkDirectory" }
New-Item -ItemType Directory -Path $WorkDirectory | Out-Null
$target = Join-Path $surface 'target'
cargo build --locked --release --manifest-path (Join-Path $surface 'Cargo.toml') --target-dir $target
$suffix = if ($IsWindows) { '.exe' } else { '' }
$cli = Join-Path $target "release/surface-esa-worldcover$suffix"
$tool = Join-Path $target "release/surface-tool$suffix"
$raster = Join-Path $target "release/surface-esa-masks$suffix"
$shade = Join-Path $target "release/surface-esa-shade$suffix"
Push-Location $WorkDirectory
try {
    & $cli prepare-dem $Dem14 dem.sqlite3
    if (!$WorldCoverDirectory) {
        $WorldCoverDirectory = Join-Path $WorkDirectory 'worldcover'
        & "$PSScriptRoot/download_esa_worldcover.ps1" -TerrainDatabase dem.sqlite3 -Output $WorldCoverDirectory
    }
    & $cli plan-surface dem.sqlite3 | Set-Content -Encoding utf8NoBOM work-areas.json
    '[]' | Set-Content -Encoding utf8NoBOM source-gaps.json
    $maskOptions = @(
        '--png','--lpf','--layer','land','--indexed-land','--water-over-bare',
        '--remove-isolated-source-pixels','--suppress-urban-forest','--urban-forest-relief-dem','dem.sqlite3',
        '--coastal-forest-median-radius','32','--full-forested-inland-water','--esa-zero-land-dem','dem.sqlite3',
        '--compact-zooms','--include-l2','--source',$WorldCoverDirectory,
        '--plan','work-areas.json','--source-gaps','source-gaps.json',
        '--astcenc','unused-in-png-mode','--output','masks','--jobs',[string]$Jobs
    )
    & $raster @maskOptions
    $masks = 'masks/landcover-national-urban-forest-relief.sqlite3'
    & $shade --dem dem.sqlite3 --national-landcover $masks --adaptive-graded --output shade
    & $shade --merge-hillshade shade/hillshade-national.sqlite3 --landcover $masks --omit-bare --output combined
    & $shade --pack-ground-512 combined/landcover-with-hillshade.sqlite3 --l2-384 --output packed
    & $shade --compress-hillshade packed/landcover-with-hillshade.sqlite3 --quality 89 --output compressed-shade
    & $shade --compress-hillshade compressed-shade/landcover-with-hillshade.sqlite3 --lossless-masks --output compressed
    & $cli pack-esa compressed/landcover-with-hillshade.sqlite3 esa-worldcover.sqlite3 $Version
    $dataTimestamp = [datetime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ssZ", [Globalization.CultureInfo]::InvariantCulture)
    & $tool validate esa-worldcover.sqlite3
    & $tool manifest esa-worldcover.sqlite3 https://github.com/shohei0205/yamamuki-data/releases/download $dataTimestamp | Set-Content -Encoding utf8NoBOM manifest.json
    Get-Item -LiteralPath 'esa-worldcover.sqlite3','manifest.json'
} finally {
    Pop-Location
}
