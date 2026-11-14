function Read-SurfaceLevelSelection($Layers, $Levels) {
    $selection = [Collections.Generic.Dictionary[string,object]]::new([StringComparer]::Ordinal)
    foreach ($id in $Layers) {
        if ($id -isnot [string] -or !$selection.TryAdd($id, $null)) {
            throw "SQLiteのレイヤー指定が不正または重複しています: $id"
        }
    }
    if ($null -ne $Levels) {
        if ($Levels -isnot [Collections.IDictionary]) {
            throw 'levelsにはレイヤーID別のズーム配列を指定してください。'
        }
        foreach ($id in $Levels.Keys) {
            $zooms = $Levels[$id]
            if (!$selection.ContainsKey($id) -or $zooms -isnot [array] -or !$zooms.Count) {
                throw "SQLiteのレベル指定が不正です: $id"
            }
            $unique = [Collections.Generic.HashSet[int]]::new()
            foreach ($z in $zooms) {
                if (($z -isnot [int] -and $z -isnot [long]) -or $z -lt 0 -or $z -gt 30 -or !$unique.Add($z)) {
                    throw "SQLiteのズームが不正または重複しています: $id"
                }
            }
            $selection[$id] = @($zooms | Sort-Object)
        }
    }
    return ,$selection
}

function Read-SurfaceReleaseConfig([string]$Dataset, [string]$Package, [ValidateSet("surface","terrain")][string]$Kind = "surface") {
    if ($Dataset -cnotmatch '^[a-z][a-z0-9_]*$' -or $Package -cnotmatch '^[a-z][a-z0-9_-]*$') {
        throw 'データセットまたは配布単位の名前が不正です。'
    }
    $root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
    $path = Join-Path $root "$Kind/$Dataset/release.json"
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
    $splits = @()
    if ($config.Contains('splitPackages')) {
        if ($config.splitPackages -isnot [array] -or !$config.splitPackages.Count -or !$config.layers.Count) {
            throw '分割配布には収録レイヤーと分割単位の配列が必要です。'
        }
        $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        [void]$names.Add($config.fileName)
        foreach ($split in $config.splitPackages) {
            if ($split -isnot [Collections.IDictionary] -or
                $split.fileName -isnot [string] -or $split.fileName -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._-]*\.sqlite3$' -or
                !$names.Add($split.fileName) -or $split.layers -isnot [array] -or !$split.layers.Count) {
                throw '分割SQLiteのファイル名またはレイヤー指定が不正です。'
            }
            foreach ($id in $split.layers) {
                if ($config.layers -cnotcontains $id) {
                    throw "分割SQLiteのレイヤーが未定義です: $id"
                }
            }
            $selection = Read-SurfaceLevelSelection $split.layers $split.levels
            $splits += @{
                fileName = $split.fileName
                manifestName = [IO.Path]::GetFileNameWithoutExtension($split.fileName) + '.manifest.json'
                layers = $split.layers
                selection = $selection
            }
        }
    }
    @{
        kind = $Kind
        datasetId = $settings.datasetId
        versionPattern = $settings.versionPattern
        releaseId = $config.releaseId
        fileName = $config.fileName
        manifestName = [IO.Path]::GetFileNameWithoutExtension($config.fileName) + '.manifest.json'
        layers = $config.layers
        selection = Read-SurfaceLevelSelection $config.layers $config.levels
        splitPackages = $splits
    }
}

function Get-SurfaceReleaseTag($Config, $Manifest, $Levels) {
    $datasets = @($Manifest.datasets)
    if ($datasets.Count -ne 1 -or $datasets[0].id -cne $Config.datasetId -or $datasets[0].kind -cne $(if ($Config.kind) { $Config.kind } else { "surface" })) {
        throw "公開設定に対応する地表データセットだけを収録したSQLiteを指定してください: $($Config.datasetId)"
    }
    if ($null -ne $Config.layers -and
        (Compare-Object -ReferenceObject @($Config.layers) -DifferenceObject @($Levels.Keys) -CaseSensitive)) {
        throw 'SQLiteの収録レイヤーが配布単位の設定と一致しません。'
    }
    foreach ($id in $Config.selection.Keys) {
        $wanted = $Config.selection[$id]
        if ($null -ne $wanted -and (($Levels[$id] -join ',') -cne ($wanted -join ','))) {
            throw "SQLiteの収録レベルが配布単位の設定と一致しません: $id"
        }
    }
    if ($datasets[0].metadata.fixture -eq $true) { throw "検査用fixtureは配布できません。" }
    if ($Config.kind -eq "terrain" -and $datasets[0].metadata.generation.scope -cne "national") { throw "全国生成が完了した地形データを指定してください。" }
    $version = $datasets[0].dataVersion
    if ($version -cnotmatch $Config.versionPattern) {
        throw "データ版が公開設定の規則に合いません: $version"
    }
    "$($Config.releaseId)-$version"
}
