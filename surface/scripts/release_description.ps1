function New-SurfaceReleaseDescription($Manifest, $Variants) {
    $entry = $Manifest.datasets[0]
    $generatedAt = if ($Manifest.dataTimestamp -is [datetime]) {
        $Manifest.dataTimestamp.ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ", [Globalization.CultureInfo]::InvariantCulture)
    } else { [string]$Manifest.dataTimestamp }
    @(
        "## $($entry.title)"
        ''
        "生成日時（UTC）: $generatedAt"
        ''
        '地表画像タイルを収録したSQLiteです。必要なレイヤーと最大ズームに合うファイルを選んでください。'
        ''
        '## 配布ファイル'
        ''
        '| SQLite | レイヤー別の収録ズーム | サイズ | manifest |'
        '|---|---|---:|---|'
        foreach ($variant in $Variants) {
            $data = $variant.manifest
            $zooms = foreach ($id in ($variant.levels.Keys | Sort-Object)) {
                $values = @($variant.levels[$id] | Sort-Object)
                # 非連続のレベルセットも範囲に丸めず、そのまま示す。
                $labels = @($values | ForEach-Object { "z$_" }) -join ', '
                "$id`: $labels"
            }
            $size = ([double]$data.sizeBytes / 1MB).ToString('F2', [Globalization.CultureInfo]::InvariantCulture)
            "| [$($data.fileName)]($($data.downloadUrl)) | $($zooms -join '<br>') | $size MiB | [$($variant.manifestName)]($($variant.manifestUrl)) |"
        }
        ''
        '## 利用方法'
        ''
        'SQLiteと対応するmanifestを取得し、manifestのサイズ・SHA-256と照合してください。ネイティブの山むきデータビューワでは「地表SQLiteを開く」から読み込めます。'
        ''
        '同じレイヤー・ズームを重複して取得する必要はありません。'
        ''
        "[地表SQLiteの仕様](https://github.com/shohei0205/yamamuki-data/blob/main/surface/formats/sqlite.md) ／ [ライセンスと帰属表示]($($Manifest.licenseUrl))"
        ''
        '## 出典と利用条件'
        foreach ($source in $Manifest.metadata.sources) {
            ''
            "### $($source.name)"
            ''
            "提供元: $($source.source)"
            ''
            "利用条件: $($source.license)"
            ''
            $source.attribution
            ''
            "加工内容: $($source.changes)"
        }
        ''
        '## 検証情報'
        ''
        ('- SHA-256: ' + [char]96 + $Manifest.sha256 + [char]96)
    ) -join [Environment]::NewLine
}
