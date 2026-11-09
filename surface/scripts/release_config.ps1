function Read-SurfaceReleaseConfig([string]$Dataset, [string]$Package) {
    if ($Dataset -cnotmatch '^[a-z][a-z0-9_]*$' -or $Package -cnotmatch '^[a-z][a-z0-9_-]*$') {
        throw 'データセットまたは配布単位の名前が不正です。'
    }
    $path = Join-Path (Split-Path $PSScriptRoot -Parent) "$Dataset/release.json"
    $settings = Get-Content -LiteralPath $path -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
    if (!$settings.packages -or !$settings.packages.Contains($Package)) {
        throw "配布単位が定義されていません: $Dataset/$Package"
    }
    $config = $settings.packages[$Package]
    if ($settings.datasetId -cnotmatch '^[a-z][a-z0-9_-]*$' -or
        $config.releaseId -cnotmatch '^[a-z][a-z0-9_-]*$' -or
        $config.fileName -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._-]*\.sqlite3$' -or
        [string]::IsNullOrWhiteSpace($settings.versionPattern)) {
        throw "地表データの公開設定が不正です: $path ($Package)"
    }
    @{
        datasetId = $settings.datasetId
        versionPattern = $settings.versionPattern
        releaseId = $config.releaseId
        fileName = $config.fileName
        layers = $config.layers
    }
}

function Get-SurfaceReleaseTag($Config, $Manifest) {
    $datasets = @($Manifest.datasets)
    if ($datasets.Count -ne 1 -or $datasets[0].id -cne $Config.datasetId -or $datasets[0].kind -cne 'surface') {
        throw "公開設定に対応する地表データセットだけを収録したSQLiteを指定してください: $($Config.datasetId)"
    }
    if ($null -ne $Config.layers -and
        (Compare-Object -ReferenceObject @($Config.layers) -DifferenceObject @($Manifest.surfaceLayerIds) -CaseSensitive)) {
        throw 'SQLiteの収録レイヤーが配布単位の設定と一致しません。'
    }
    $version = $datasets[0].dataVersion
    if ($version -cnotmatch $Config.versionPattern) {
        throw "データ版が公開設定の規則に合いません: $version"
    }
    "$($Config.releaseId)-$version"
}
