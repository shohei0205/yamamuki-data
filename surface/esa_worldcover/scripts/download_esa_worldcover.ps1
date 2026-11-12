param(
    [Parameter(Mandatory)][string]$TerrainDatabase,
    [Parameter(Mandatory)][string]$Output,
    [ValidateRange(0,32)][int]$HaloTiles = 4,
    [switch]$PlanOnly
)
$ErrorActionPreference = 'Stop'
$TerrainDatabase = (Resolve-Path -LiteralPath $TerrainDatabase).Path
$surface = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$suffix = if ($IsWindows) { '.exe' } else { '' }
$generator = Join-Path $surface "target/release/surface-esa-worldcover$suffix"
$keys = @(& $generator plan-worldcover $TerrainDatabase --halo-tiles $HaloTiles)
if ($LASTEXITCODE -or !$keys.Count) { throw 'WorldCoverタイル候補を算出できませんでした。' }
if ($PlanOnly) { $keys; return }

$bucket = 'https://esa-worldcover.s3.eu-central-1.amazonaws.com'
$prefix = 'v200/2021/map'
$curl = if ($IsWindows) { 'curl.exe' } else { 'curl' }
New-Item -ItemType Directory -Force -Path $Output | Out-Null
$lock = [IO.File]::Open((Join-Path $Output 'worldcover-download.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
try {
    $files = @()
    $absent = @()
    foreach ($key in $keys) {
        $name = "ESA_WorldCover_10m_2021_v200_$($key)_Map.tif"
        $object = "$prefix/$name"
        $list = Invoke-RestMethod -Uri "$bucket/?list-type=2&prefix=$object" -MaximumRetryCount 3 -TimeoutSec 30
        $entry = $list.ListBucketResult.Contents | Where-Object { $_.Key -eq $object }
        if (!$entry) {
            $absent += $key
            Write-Host "$key はESAカタログにありません。"
            continue
        }
        $expected = [long]$entry.Size
        $path = Join-Path $Output $name
        $partial = "$path.part"
        if (!(Test-Path -LiteralPath $path)) {
            if (!(Test-Path -LiteralPath $partial) -or (Get-Item -LiteralPath $partial).Length -ne $expected) {
                Write-Host "$key を取得中"
                & $curl --fail --retry 3 --connect-timeout 30 --silent --show-error --continue-at - --output $partial "$bucket/$object"
                if ($LASTEXITCODE) { throw "取得に失敗しました。 $key" }
            }
            if ((Get-Item -LiteralPath $partial).Length -ne $expected) { throw "取得サイズがESAカタログと一致しません。 $key" }
            Move-Item -LiteralPath $partial -Destination $path
        }
        if ((Get-Item -LiteralPath $path).Length -ne $expected) { throw "既存ファイルのサイズがESAカタログと異なります。 $path" }
        $files += @{key=$key;file=$name;bytes=$expected}
    }
    @{
        product='ESA WorldCover 2021 v200'
        terrainDatabase=$TerrainDatabase
        haloZ11Tiles=$HaloTiles
        files=$files
        absentFromCatalog=$absent
    } | ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8 (Join-Path $Output 'japan-download-manifest.json')
    Write-Host "$($files.Count)枚を確認済み、カタログに存在しない候補は$($absent.Count)件。"
} finally {
    $lock.Dispose()
}
