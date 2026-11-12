[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Dataset,
    [string]$Package = "all",
    [Parameter(Mandatory)][string]$Database,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [string]$DataTimestamp,
    [switch]$CreateDraft,
    [switch]$ProcessDraft,
    [ValidatePattern('^[0-9a-fA-F]{40}$')][string]$TargetCommit
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
. (Join-Path $PSScriptRoot 'release_config.ps1')
$config = Read-SurfaceReleaseConfig $Dataset $Package
if ($CreateDraft -and $ProcessDraft) { throw '-CreateDraft と -ProcessDraft は同時に指定できません。' }
$uploadDraft = $CreateDraft -or $ProcessDraft
if ($uploadDraft -and !$TargetCommit) { throw '下書きを作るには、生成に使ったコミットのSHAを指定してください。' }
if ($TargetCommit -and !$uploadDraft) { throw '-TargetCommit は -CreateDraft または -ProcessDraft とともに指定してください。' }

$repo = 'shohei0205/yamamuki-data'
$releaseRoot = "https://github.com/$repo/releases/download"
$inputFile = Get-Item -LiteralPath $Database
if ($inputFile.PSIsContainer) { throw '地表SQLiteのファイルを指定してください。' }
$generationManifest = $null
if (!$DataTimestamp) {
    $manifestPath = Join-Path $inputFile.DirectoryName ([IO.Path]::GetFileNameWithoutExtension($inputFile.Name) + '.manifest.json')
    if (!(Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        $manifestPath = Join-Path $inputFile.DirectoryName 'manifest.json'
    }
    if (Test-Path -LiteralPath $manifestPath -PathType Leaf) {
        $generationManifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json -AsHashtable
        $recordedTimestamp = $generationManifest.dataTimestamp
        $DataTimestamp = if ($recordedTimestamp -is [datetime]) {
            $recordedTimestamp.ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ss.FFFFFFFZ", [Globalization.CultureInfo]::InvariantCulture)
        } else { $recordedTimestamp }
    }
}
if ($DataTimestamp -cnotmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$') {
    throw '生成時のmanifestを同じ場所に置くか、-DataTimestamp に実際の生成日時（UTC）を指定してください。'
}
[void][DateTimeOffset]::Parse($DataTimestamp, [Globalization.CultureInfo]::InvariantCulture)
$output = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
if (Test-Path -LiteralPath $output) { throw "新しい出力ディレクトリを指定してください: $output" }
$parent = Split-Path -Parent $output
if (!(Test-Path -LiteralPath $parent -PathType Container)) { throw "出力先の親ディレクトリがありません: $parent" }

$surface = Split-Path $PSScriptRoot -Parent
$target = Join-Path $surface 'target'
$suffix = if ($IsWindows) { '.exe' } else { '' }
cargo build --locked --release --manifest-path (Join-Path $surface 'Cargo.toml') --target-dir $target --bin surface-tool
$tool = Join-Path $target "release/surface-tool$suffix"

$stage = Join-Path $parent ('.surface-release-' + [guid]::NewGuid())
$databaseName = $config.fileName
$stagedDatabase = Join-Path $stage $databaseName
$stagedLicense = Join-Path $stage 'LICENSE.txt'
$stagedManifest = Join-Path $stage $config.manifestName
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    Copy-Item -LiteralPath $inputFile.FullName -Destination $stagedDatabase
    & $tool validate $stagedDatabase
    $manifest = & $tool manifest $stagedDatabase $releaseRoot $DataTimestamp $config.releaseId | ConvertFrom-Json -AsHashtable
    if ($generationManifest -and $generationManifest.sha256 -cne $manifest.sha256) {
        throw '生成時のmanifestとSQLiteのSHA-256が一致しません。'
    }
    $levels = & $tool surface-levels $stagedDatabase | ConvertFrom-Json -AsHashtable
    $tag = Get-SurfaceReleaseTag $config $manifest $levels
    $entry = $manifest.datasets[0]
    $version = $entry.dataVersion
    $releaseBase = "$releaseRoot/$tag"
    $manifest.dataTimestamp = $DataTimestamp
    $manifest.licenseUrl = "$releaseBase/LICENSE.txt"

    $sources = @($entry.metadata.sources)
    if ($sources.Count -eq 0) { throw '出典がSQLiteに記録されていません。' }
    foreach ($source in $sources) {
        foreach ($field in @('name', 'source', 'license', 'attribution', 'changes')) {
            if ([string]::IsNullOrWhiteSpace($source[$field])) { throw "出典に $field がありません。" }
        }
    }
    $license = @(
        "$($entry.title) $version"
        ''
        'このデータは出典ごとの利用条件に従って利用・再配布してください。'
        '表示には、使用したレイヤーの出典・帰属表示と加工内容を示してください。'
        ''
        foreach ($author in $entry.metadata.authors) {
            "作成者: $($author.name) $($author.url)"
        }
        foreach ($source in $sources) {
            ''
            "出典: $($source.name)"
            "提供元: $($source.source)"
            "利用条件: $($source.license)"
            "帰属表示: $($source.attribution)"
            "加工内容: $($source.changes)"
        }
    )
    $license | Set-Content -LiteralPath $stagedLicense -Encoding utf8NoBOM
    $manifest | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $stagedManifest -Encoding utf8NoBOM
    [IO.Directory]::Move($stage, $output)
} finally {
    if (Test-Path -LiteralPath $stage) {
        foreach ($file in @($stagedDatabase, $stagedLicense, $stagedManifest)) {
            if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file }
        }
        [IO.Directory]::Delete($stage)
    }
}

Write-Output "タグ: $tag"
Write-Output "配布ファイル: $output"
Write-Output "公開後のmanifest: $releaseBase/$($config.manifestName)"
foreach ($split in $config.splitPackages) {
    Write-Output "加工・検査Actionが作る分割版manifest: $releaseBase/$($split.manifestName)"
}

if ($uploadDraft) {
    $workflow = 'publish-surface.yml'
    if ($ProcessDraft -and (gh api "repos/$repo/actions/workflows/$workflow" --jq .state) -cne 'active') {
        throw '加工・検査Actionを先にmainに配置して有効にしてください。'
    }
    $remoteSha = gh api "repos/$repo/commits/$TargetCommit" --jq .sha
    if ($remoteSha -ne $TargetCommit) { throw '生成に使ったコミットが公開先に見つかりません。' }
    $existingTag = git ls-remote --refs "https://github.com/$repo.git" "refs/tags/$tag"
    if ($existingTag) {
        throw "公開先に同じタグがあります: $tag"
    }

    $releasePages = gh api --paginate --slurp "repos/$repo/releases?per_page=100" | ConvertFrom-Json -AsHashtable -NoEnumerate
    foreach ($page in $releasePages) {
        if ($page | Where-Object { $_.tag_name -ceq $tag }) { throw "同じタグのReleaseがすでにあります: $tag" }
    }

    $assets = @($databaseName, $config.manifestName, 'LICENSE.txt') | ForEach-Object { Join-Path $output $_ }
    $releaseOptions = @(
        '--repo', $repo, '--target', $TargetCommit, '--draft', '--latest=false'
        '--title', ('draft-' + $Dataset.Replace('_', '-') + '-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ'))
        '--notes', "<!-- yamamuki-data:temporary-draft:v1 -->`nローカルで生成した地表SQLiteの受け渡し用Draftです。加工・検査Actionを実行してください。"
    )
    # Draftではタグ名と対象SHAだけを記録し、Gitタグの実体は作らない。
    gh release create $tag @assets @releaseOptions
    if ($ProcessDraft) {
        $sha = $manifest.sha256
        Write-Output "Actionの再起動: gh workflow run $workflow --repo $repo --ref main -f dataset=$Dataset -f package=$Package -f tag=$tag -f sha256=$sha -f target_commit=$TargetCommit"
        gh workflow run $workflow --repo $repo --ref main -f "dataset=$Dataset" -f "package=$Package" -f "tag=$tag" -f "sha256=$sha" -f "target_commit=$TargetCommit"
    }
}
