param(
    [string]$TerrainDatabase = 'build/national-shared-z6-z13-v1/terrain-shared.sqlite3',
    [string]$Output = 'build/terrain-worldcover-source',
    [string]$Python = 'build/terrain-venv/Scripts/python.exe',
    [ValidateRange(0,32)][int]$HaloTiles = 4,
    [switch]$PlanOnly
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent)
$TerrainDatabase = (Resolve-Path -LiteralPath $TerrainDatabase).Path
$keys = @'
import math, sqlite3, sys
from pathlib import Path

database, halo = Path(sys.argv[1]), int(sys.argv[2])
width = 1 << 11

def latitude(y):
    return math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * y / width))))

def longitude(x):
    return x * 360 / width - 180

with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True) as connection:
    tiles = connection.execute('SELECT x,y FROM tiles WHERE z=11').fetchall()
keys = set()
for x, y in tiles:
    south = math.floor(latitude(y + 1 + halo) / 3) * 3
    north = math.floor(latitude(y - halo) / 3) * 3
    west = math.floor(longitude(x - halo) / 3) * 3
    east = math.floor(longitude(x + 1 + halo) / 3) * 3
    for lat in range(south, north + 1, 3):
        for lon in range(west, east + 1, 3):
            keys.add(f'{"N" if lat >= 0 else "S"}{abs(lat):02d}{"E" if lon >= 0 else "W"}{abs(lon):03d}')
print('\n'.join(sorted(keys)))
'@ | & $Python - $TerrainDatabase $HaloTiles
if ($LASTEXITCODE -or !$keys) { throw 'WorldCoverタイル候補を算出できませんでした。' }
if ($PlanOnly) { $keys; return }

$bucket = 'https://esa-worldcover.s3.eu-central-1.amazonaws.com'
$prefix = 'v200/2021/map'
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
                & curl.exe --fail --retry 3 --connect-timeout 30 --silent --show-error --continue-at - --output $partial "$bucket/$object"
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
