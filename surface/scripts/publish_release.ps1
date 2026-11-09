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
    if ($draft.assets.Count -ne $files.Count) { throw '下書きの添付数が一致しません。' }
    foreach ($name in $files) {
        $found = @($draft.assets | Where-Object { $_.name -ceq $name -and $_.state -ceq 'uploaded' })
        if ($found.Count -ne 1 -or $found[0].size -le 0) { throw "添付が不正です: $name" }
        $found[0] | Select-Object id, name, size, digest
    }
}

function Confirm-TagCommit {
    $tagRef = gh api "$prefix/git/ref/tags/$Tag" | ConvertFrom-Json
    if ($tagRef.object.type -cne 'commit' -or $tagRef.object.sha -ne $TargetCommit) {
        throw "タグが指定コミットを直接指していません: $Tag"
    }
}

Confirm-TagCommit
$assets = @(Read-DraftAssets)
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
    foreach ($asset in $assets) {
        $path = Join-Path $download $asset.name
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
    if ((Get-SurfaceReleaseTag $config $expected) -cne $Tag) {
        throw 'SQLiteのデータセット・版がタグと一致しません。'
    }
    $expected.licenseUrl = "$releaseRoot/$Tag/LICENSE.txt"
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

    Confirm-TagCommit
    $current = @(Read-DraftAssets)
    if ((ConvertTo-Json -InputObject $assets -Compress) -cne (ConvertTo-Json -InputObject $current -Compress)) {
        throw '検査中に下書きの添付が変更されました。'
    }
    $published = gh api --method PATCH $releasePath -F draft=false -f make_latest=false | ConvertFrom-Json
    if ($published.draft -or $published.tag_name -cne $Tag) { throw '公開結果が一致しません。' }
    @(
        '## 地表SQLiteの公開'
        ''
        "- Release: $($published.html_url)"
        "- Manifest: $releaseRoot/$Tag/manifest.json"
        "- SHA-256: $($expected.sha256)"
    ) | Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Encoding utf8NoBOM
} finally {
    foreach ($name in $files) {
        $path = Join-Path $download $name
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
    [IO.Directory]::Delete($download)
}
