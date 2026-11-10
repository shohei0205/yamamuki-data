[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Dataset,
    [string]$Package = "all",
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._-]*$')][string]$Tag,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$Sha256,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{40}$')][string]$TargetCommit
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
. (Join-Path $PSScriptRoot 'release_config.ps1')
$config = Read-SurfaceReleaseConfig $Dataset $Package
$repo = $env:GITHUB_REPOSITORY
if ($repo -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' -or $env:GITHUB_REF -cne 'refs/heads/main') {
    throw '正式版の公開は対象リポジトリのmainから実行してください。'
}

$prefix = "repos/$repo"
$releaseRoot = "https://github.com/$repo/releases/download"
$tagPrefix = "$($config.releaseId)-"
if (!$Tag.StartsWith($tagPrefix, [StringComparison]::Ordinal) -or
    $Tag.Substring($tagPrefix.Length) -cnotmatch $config.versionPattern) {
    throw "タグが公開設定と一致しません: $Tag"
}
$files = @($config.fileName, 'manifest.json', 'LICENSE.txt')
$splitFiles = @(foreach ($split in $config.splitPackages) {
    $split.fileName
    $split.manifestName
})
$allFiles = $files + $splitFiles
$releasePages = gh api --paginate --slurp "$prefix/releases?per_page=100" | ConvertFrom-Json -AsHashtable -NoEnumerate
$candidates = @(foreach ($page in $releasePages) {
    $page | Where-Object { $_.tag_name -ceq $Tag }
})
if ($candidates.Count -ne 1) { throw "下書きを一意に特定できません: $Tag" }
$releasePath = "$prefix/releases/$($candidates[0].id)"

function Read-DraftAssets {
    $draft = gh api $releasePath | ConvertFrom-Json -AsHashtable
    if (!$draft.draft -or $draft.prerelease -or $draft.tag_name -cne $Tag) {
        throw "対象は正式版の下書きではありません: $Tag"
    }
    if ($draft.target_commitish -ne $TargetCommit) {
        throw '下書きの対象コミットが実行時の指定と一致しません。'
    }
    $byName = [Collections.Generic.Dictionary[string,object]]::new([StringComparer]::Ordinal)
    foreach ($asset in $draft.assets) {
        if ($allFiles -cnotcontains $asset.name -or !$byName.TryAdd($asset.name, $asset)) {
            throw "予定外または重複した添付です: $($asset.name)"
        }
    }
    foreach ($name in $files) {
        $asset = $byName[$name]
        if (!$asset -or $asset.state -cne 'uploaded' -or $asset.size -le 0) {
            throw "元の添付が不正です: $name"
        }
    }
    return $byName
}

function Confirm-SourceAssets($Current) {
    foreach ($name in $files) {
        $before = $assets[$name]
        $after = $Current[$name]
        if ($before.id -ne $after.id -or $before.size -ne $after.size -or $before.digest -cne $after.digest) {
            throw "検査中に元の添付が変更されました: $name"
        }
    }
}

function Test-SplitAsset($Asset, $LocalFile, [string]$Hash) {
    if (!$Asset -or $Asset.state -cne 'uploaded' -or $Asset.size -ne $LocalFile.Length) {
        return $false
    }
    if ($Asset.digest) {
        return $Asset.digest -ieq "sha256:$Hash"
    }
    $check = Join-Path $download ('.verify-' + $LocalFile.Name)
    try {
        gh release download $Tag --repo $repo --pattern $LocalFile.Name --output $check | Out-Null
        return (Get-FileHash -LiteralPath $check -Algorithm SHA256).Hash -ieq $Hash
    } finally {
        if (Test-Path -LiteralPath $check) { Remove-Item -LiteralPath $check }
    }
}

function Confirm-TagCommit {
    $tagRef = gh api "$prefix/git/ref/tags/$Tag" | ConvertFrom-Json
    if ($tagRef.object.type -cne 'commit' -or $tagRef.object.sha -ne $TargetCommit) {
        throw "タグが指定コミットを直接指していません: $Tag"
    }
}

Confirm-TagCommit
$assets = Read-DraftAssets
$surface = Split-Path $PSScriptRoot -Parent
$target = Join-Path $surface 'target'
$suffix = if ($IsWindows) { '.exe' } else { '' }
cargo build --locked --release --manifest-path (Join-Path $surface 'Cargo.toml') --target-dir $target --bin surface-tool
$tool = Join-Path $target "release/surface-tool$suffix"

$download = Join-Path ([IO.Path]::GetTempPath()) ('surface-publish-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $download | Out-Null
try {
    $patterns = foreach ($name in $files) { '--pattern'; $name }
    gh release download $Tag --repo $repo --dir $download @patterns
    foreach ($name in $files) {
        $asset = $assets[$name]
        $path = Join-Path $download $name
        if ((Get-Item -LiteralPath $path).Length -ne $asset.size) { throw "添付サイズが一致しません: $($asset.name)" }
        $actualHash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
        if ($asset.digest -and $asset.digest -ne "sha256:$actualHash") { throw "添付ハッシュが一致しません: $($asset.name)" }
        if ($asset.name -ceq $files[0] -and $actualHash -ne $Sha256) { throw 'SQLiteのSHA-256が入力値と一致しません。' }
    }

    $rawManifest = Get-Content -LiteralPath (Join-Path $download 'manifest.json') -Raw -Encoding utf8
    $manifest = ConvertFrom-Json -InputObject $rawManifest -AsHashtable
    $document = [System.Text.Json.JsonDocument]::Parse($rawManifest)
    try { $timestamp = $document.RootElement.GetProperty('dataTimestamp').GetString() }
    finally { $document.Dispose() }
    if ($timestamp -cnotmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$') {
        throw 'manifestの生成日時がUTC形式ではありません。'
    }
    [void][DateTimeOffset]::Parse($timestamp, [Globalization.CultureInfo]::InvariantCulture)

    $database = Join-Path $download $files[0]
    & $tool validate $database
    $expected = & $tool manifest $database $releaseRoot $timestamp $config.releaseId | ConvertFrom-Json -AsHashtable
    $sourceLevels = & $tool surface-levels $database | ConvertFrom-Json -AsHashtable
    if ((Get-SurfaceReleaseTag $config $expected $sourceLevels) -cne $Tag) {
        throw 'SQLiteのデータセット・版がタグと一致しません。'
    }
    $expected.licenseUrl = "$releaseRoot/$Tag/LICENSE.txt"
    if ($manifest.Contains('surface') -or $manifest.Contains('surfaceLayerIds') -or $manifest.Contains('surfaceLevels')) {
        throw '公開準備をやり直し、現行形式のmanifestを作成してください。'
    }
    foreach ($field in $expected.Keys) {
        if (!$manifest.Contains($field) -or
            (ConvertTo-Json -InputObject $manifest[$field] -Depth 32 -Compress) -cne
            (ConvertTo-Json -InputObject $expected[$field] -Depth 32 -Compress)) {
            throw "manifestがSQLiteまたは配布先と一致しません: $field"
        }
    }
    $license = Get-Content -LiteralPath (Join-Path $download 'LICENSE.txt') -Raw -Encoding utf8
    $sources = @($expected.metadata.sources)
    if (!$sources.Count) { throw 'SQLiteに出典がありません。' }
    foreach ($source in $sources) {
        foreach ($field in @('name', 'source', 'license', 'attribution', 'changes')) {
            if ([string]::IsNullOrWhiteSpace($source[$field]) -or !$license.Contains($source[$field])) {
                throw "LICENSE.txtに出典の $field がありません。"
            }
        }
    }
    foreach ($author in $expected.metadata.authors) {
        if (!$license.Contains($author.name)) { throw 'LICENSE.txtに作成者がありません。' }
    }

    $splitHashes = @{}
    foreach ($split in $config.splitPackages) {
        $splitDb = Join-Path $download $split.fileName
        $selection = ConvertTo-Json -InputObject $split.selection -Depth 6 -Compress
        & $tool select-surface $database $splitDb $selection
        & $tool validate $splitDb
        $splitManifest = & $tool manifest $splitDb $releaseRoot $timestamp $config.releaseId | ConvertFrom-Json -AsHashtable
        $actualLevels = & $tool surface-levels $splitDb | ConvertFrom-Json -AsHashtable
        foreach ($id in $split.layers) {
            $wanted = $split.selection[$id] ?? $sourceLevels[$id]
            if (!$actualLevels.Contains($id) -or (($actualLevels[$id] -join ',') -cne ($wanted -join ','))) {
                throw "分割SQLiteのレベルが設定と一致しません: $($split.fileName) / $id"
            }
        }
        $splitConfig = $config.Clone()
        $splitConfig.layers = $split.layers
        $splitConfig.selection = $split.selection
        if ((Get-SurfaceReleaseTag $splitConfig $splitManifest $actualLevels) -cne $Tag -or
            $splitManifest.downloadUrl -cne "$releaseRoot/$Tag/$($split.fileName)") {
            throw "分割SQLiteの配布情報が一致しません: $($split.fileName)"
        }
        $splitManifest.licenseUrl = $expected.licenseUrl
        $splitManifest | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath (Join-Path $download $split.manifestName) -Encoding utf8NoBOM
        $splitHashes[$split.fileName] = $splitManifest.sha256
        $splitHashes[$split.manifestName] = (Get-FileHash -LiteralPath (Join-Path $download $split.manifestName) -Algorithm SHA256).Hash
    }

    Confirm-TagCommit
    $current = Read-DraftAssets
    Confirm-SourceAssets $current
    foreach ($name in $splitFiles) {
        $localFile = Get-Item -LiteralPath (Join-Path $download $name)
        if (!(Test-SplitAsset $current[$name] $localFile $splitHashes[$name])) {
            gh release upload $Tag $localFile.FullName --repo $repo --clobber
        }
    }

    Confirm-TagCommit
    $current = Read-DraftAssets
    Confirm-SourceAssets $current
    foreach ($name in $splitFiles) {
        $localFile = Get-Item -LiteralPath (Join-Path $download $name)
        if (!(Test-SplitAsset $current[$name] $localFile $splitHashes[$name])) {
            throw "分割版の添付が検証結果と一致しません: $name"
        }
    }
    $published = gh api --method PATCH $releasePath -F draft=false -f make_latest=false | ConvertFrom-Json
    if ($published.draft -or $published.tag_name -cne $Tag) { throw '公開結果が一致しません。' }
    @(
        '## 地表SQLiteの公開'
        ''
        "- Release: $($published.html_url)"
        "- Manifest: $releaseRoot/$Tag/manifest.json"
        "- SHA-256: $($expected.sha256)"
        foreach ($split in $config.splitPackages) {
            "- 分割版 manifest: $releaseRoot/$Tag/$($split.manifestName)"
        }
    ) | Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Encoding utf8NoBOM
} finally {
    foreach ($name in $allFiles) {
        $path = Join-Path $download $name
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
    [IO.Directory]::Delete($download)
}
