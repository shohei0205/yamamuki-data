[CmdletBinding(DefaultParameterSetName='Csv')]
param(
    [Parameter(Mandatory)][string]$Output,
    [Parameter(Mandatory, ParameterSetName='Csv')][string]$TilesCsv,
    [Parameter(Mandatory, ParameterSetName='Bounds')][ValidateRange(-180,180)][double]$West,
    [Parameter(Mandatory, ParameterSetName='Bounds')][ValidateRange(-85.0511287798,85.0511287798)][double]$South,
    [Parameter(Mandatory, ParameterSetName='Bounds')][ValidateRange(-180,180)][double]$East,
    [Parameter(Mandatory, ParameterSetName='Bounds')][ValidateRange(-85.0511287798,85.0511287798)][double]$North,
    [ValidateRange(0,10000)][int]$DelayMs = 100,
    [switch]$PlanOnly
)
$ErrorActionPreference = 'Stop'
$gridSize = 1 -shl 14
$base = 'https://cyberjapandata.gsi.go.jp/xyz/dem_png/14'

function Read-TileCoordinate([string]$value) {
    $n = 0
    if (![int]::TryParse($value, [ref]$n) -or $n -lt 0 -or $n -ge $gridSize) {
        throw "z14のXYZ座標が不正です: '$value'"
    }
    $n
}

function Get-TileY([double]$latitude) {
    $radians = $latitude * [Math]::PI / 180
    (1 - [Math]::Asinh([Math]::Tan($radians)) / [Math]::PI) / 2 * $gridSize
}

function Test-DemPng([string]$path) {
    $stream = [IO.File]::OpenRead($path)
    try {
        if ($stream.Length -lt 45) { return $false }
        $header = [byte[]]::new(24)
        $end = [byte[]]::new(12)
        $stream.ReadExactly($header)
        $stream.Position = $stream.Length - $end.Length
        $stream.ReadExactly($end)
        [Convert]::ToHexString($header) -eq '89504E470D0A1A0A0000000D494844520000010000000100' -and
            [Convert]::ToHexString($end) -eq '0000000049454E44AE426082'
    } finally {
        $stream.Dispose()
    }
}

function Get-DemTile([string]$url, [string]$partial, [string]$destination) {
    try {
        for ($attempt = 1; $attempt -le 4; $attempt++) {
            $status = 0
            $message = ''
            try {
                if ($DelayMs -gt 0) { Start-Sleep -Milliseconds $DelayMs }
                $response = Invoke-WebRequest -Uri $url -OutFile $partial -PassThru -SkipHttpErrorCheck -TimeoutSec 45
                $status = [int]$response.StatusCode
                $message = "HTTP $status"
                if ($status -eq 404) {
                    return @{ result = 'missing'; httpStatus = $status; message = '' }
                }
                if ($status -eq 200) {
                    if (Test-DemPng $partial) {
                        [IO.File]::Move($partial, $destination)
                        return @{ result = 'downloaded'; httpStatus = $status; message = '' }
                    }
                    $message = '256×256の完全なPNGを取得できませんでした。'
                } elseif ($status -ne 408 -and $status -ne 429 -and $status -lt 500) {
                    break
                }
            } catch {
                $message = $_.Exception.Message
            }
            if ($attempt -lt 4) { Start-Sleep -Seconds $attempt }
        }
        @{ result = 'failed'; httpStatus = $status; message = $message }
    } finally {
        if (Test-Path -LiteralPath $partial -PathType Leaf) { Remove-Item -LiteralPath $partial }
    }
}

if ($PSCmdlet.ParameterSetName -eq 'Csv') {
    $unique = [Collections.Generic.HashSet[int]]::new()
    $tiles = @(foreach ($row in Import-Csv -LiteralPath $TilesCsv) {
        $x = Read-TileCoordinate $row.x
        $y = Read-TileCoordinate $row.y
        if ($unique.Add($x * $gridSize + $y)) { [pscustomobject]@{x=$x; y=$y} }
    })
    if (!$tiles.Count) { throw 'CSVにz14のXYZ座標がありません。ヘッダーは x,y を指定してください。' }
} else {
    if ($West -ge $East -or $South -ge $North) { throw '西端・南端はそれぞれ東端・北端より小さくしてください。' }
    $left = [int][Math]::Clamp([Math]::Floor(($West + 180) / 360 * $gridSize), 0, $gridSize - 1)
    $right = [int][Math]::Clamp([Math]::Ceiling(($East + 180) / 360 * $gridSize) - 1, 0, $gridSize - 1)
    $top = [int][Math]::Clamp([Math]::Floor((Get-TileY $North)), 0, $gridSize - 1)
    $bottom = [int][Math]::Clamp([Math]::Ceiling((Get-TileY $South)) - 1, 0, $gridSize - 1)
}

function Get-RequestedTiles {
    if ($PSCmdlet.ParameterSetName -eq 'Csv') {
        $tiles
    } else {
        for ($x = $left; $x -le $right; $x++) {
            for ($y = $top; $y -le $bottom; $y++) {
                [pscustomobject]@{x=$x; y=$y}
            }
        }
    }
}

if ($PlanOnly) {
    Get-RequestedTiles | ConvertTo-Csv -NoTypeInformation
    return
}

$root = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Output)
[IO.Directory]::CreateDirectory($root) | Out-Null
$lock = [IO.File]::Open((Join-Path $root '.dem10b-download.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
$report = Join-Path $root ('dem10b-download-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff') + '.csv')
$counts = @{ downloaded = 0; reused = 0; missing = 0; failed = 0 }
try {
    Get-RequestedTiles | ForEach-Object {
        $x = $_.x
        $y = $_.y
        $column = Join-Path $root "$x"
        $destination = Join-Path $column "$y.png"
        if (Test-Path -LiteralPath $destination) {
            $result = if ((Test-Path -LiteralPath $destination -PathType Leaf) -and (Test-DemPng $destination)) {
                @{ result = 'reused'; httpStatus = ''; message = '' }
            } else {
                @{ result = 'failed'; httpStatus = ''; message = '既存のPNGが不正です。' }
            }
        } else {
            [IO.Directory]::CreateDirectory($column) | Out-Null
            $partial = Join-Path $column ".$y.png.part"
            $result = Get-DemTile "$base/$x/$y.png" $partial $destination
        }
        $counts[$result.result]++
        [pscustomobject]@{
            z = 14; x = $x; y = $y; result = $result.result
            httpStatus = $result.httpStatus; url = "$base/$x/$y.png"
            utc = [DateTimeOffset]::UtcNow.ToString('o'); message = $result.message
        }
    } | Export-Csv -LiteralPath $report -NoTypeInformation -Encoding utf8BOM
} finally {
    $lock.Dispose()
}
Write-Host "取得 $($counts.downloaded) / 再利用 $($counts.reused) / 配信なし $($counts.missing) / 失敗 $($counts.failed)"
Write-Host "取得記録 $report"
if ($counts.failed) { throw '取得に失敗したタイルがあります。取得記録を確認してください。' }
