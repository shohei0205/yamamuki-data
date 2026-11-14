function Format-SurfaceReleaseZooms($Levels) {
    $values = @($Levels | Sort-Object -Unique)
    $segments = @()
    for ($i = 0; $i -lt $values.Count; $i++) {
        $start = $values[$i]
        $end = $start
        while ($i + 1 -lt $values.Count -and $values[$i + 1] -eq $end + 1) {
            $end = $values[++$i]
        }
        $segments += if ($start -eq $end) { "z$start" } else { "z$start〜z$end" }
    }
    $segments -join ', '
}

function New-SurfaceReleaseDescription($Manifest, $Variants) {
    $entry = $Manifest.datasets[0]
    $generatedAt = if ($Manifest.dataTimestamp -is [datetime]) {
        $Manifest.dataTimestamp.ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ", [Globalization.CultureInfo]::InvariantCulture)
    } else { [string]$Manifest.dataTimestamp }
    $sortedVariants = @($Variants | Sort-Object @{
        Expression = { ($_.levels.Values | ForEach-Object { $_ } | Measure-Object -Maximum).Maximum }
    }, @{ Expression = { $_.manifest.fileName } })
    @(
        "## $($entry.title)"
        ''
        "生成日時（UTC）: $generatedAt"
        ''
        if ($entry.kind -ceq 'terrain') {
            '標高の自由配置制御点を収録したSQLiteです。自然近傍補間で地形を再構成します。'
        } else {
            '地表画像タイルを収録したSQLiteです。必要なレイヤーと最大ズームに合うファイルを選んでください。'
        }
        ''
        '## 配布ファイル'
        ''
        'ファイルは、このページ下部のAssetsから取得してください。'
        ''
        '| SQLite | 収録ズーム | サイズ | manifest |'
        '|---|---|---:|---|'
        foreach ($variant in $sortedVariants) {
            $data = $variant.manifest
            $ids = @($variant.levels.Keys | Sort-Object)
            $sets = @($ids | ForEach-Object { (@($variant.levels[$_] | Sort-Object -Unique) -join ',') } | Select-Object -Unique)
            $zooms = if ($sets.Count -eq 1) {
                Format-SurfaceReleaseZooms $variant.levels[$ids[0]]
            } else {
                @($ids | ForEach-Object { "$_`: $(Format-SurfaceReleaseZooms $variant.levels[$_])" }) -join '<br>'
            }
            $size = ([double]$data.sizeBytes / 1MB).ToString('F2', [Globalization.CultureInfo]::InvariantCulture)
            $sqliteLabel = [char]96 + $data.fileName + [char]96
            $manifestLabel = [char]96 + $variant.manifestName + [char]96
            "| $sqliteLabel | $zooms | $size MiB | $manifestLabel |"
        }
        ''
        '## 出典'
        ''
        foreach ($source in $Manifest.metadata.sources) {
            "- $($source.name): $($source.attribution) / $($source.license)"
        }
        ''
        '## 検証情報'
        ''
        ('- SHA-256: ' + [char]96 + $Manifest.sha256 + [char]96)
    ) -join [Environment]::NewLine
}
