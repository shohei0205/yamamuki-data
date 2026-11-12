param([Parameter(Mandatory)][string]$Database, [string]$DataTimestamp)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$scripts = Join-Path (Split-Path $PSScriptRoot -Parent) 'scripts'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('surface-publish-test-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $testRoot | Out-Null
$inputDir = Join-Path $testRoot 'input'
$commit = '1' * 40
$repo = 'shohei0205/yamamuki-data'
$state = @{ draft = $true; prerelease = $false; body = '<!-- yamamuki-data:temporary-draft:v1 -->'; assets = @{}; nextId = 1; uploaded = @(); failUpload = $true; created = $false; started = $false; pendingCommit = $null; hiddenVerifications = 0 }
$previousEnv = @{}
foreach ($key in @('GITHUB_REPOSITORY', 'GITHUB_REF', 'GITHUB_STEP_SUMMARY')) {
    $previousEnv[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
}

function Add-TestAsset([IO.FileInfo]$File) {
    $name = $File.Name
    $state.assets[$name] = @{
        id = $state.nextId++
        name = $name
        size = $File.Length
        digest = 'sha256:' + (Get-FileHash -LiteralPath $File.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        state = 'uploaded'
    }
}

function git {
    if ($args[0] -eq 'ls-remote') { return }
    throw "未定義のgit呼び出しです: $args"
}

function gh {
    if ($args[0] -eq 'api') {
        if ($args[1] -like '*/actions/workflows/publish-surface.yml') { return 'active' }
        if ($args[1] -like '*/commits/*') { return $commit }
        if ($args[1] -eq '--paginate' -and !$state.created) { return '[[]]' }
        if ($args[1] -eq '--paginate') {
            return '[[{"id":42,"tag_name":"' + $tag + '"}]]'
        }
        if ($args[1] -like '*/git/ref/tags/*') {
            return @{object=@{type='commit';sha=$commit}} | ConvertTo-Json -Compress
        }
        if ($args[1] -eq '--method' -and $args[2] -eq 'POST') {
            if ($args[3] -notlike '*/git/refs' -or $args[-1] -cne "sha=$commit") { throw 'タグの事前作成が不正です。' }
            $state.pendingCommit = $commit
            return '{}'
        }
        if ($args[1] -eq '--method') {
            if ($args[2] -ne 'PATCH' -or $state.assets.Count -ne 9) { throw '不完全なReleaseを公開しようとしました。' }
            $patch = Get-Content -LiteralPath $args[5] -Raw | ConvertFrom-Json -AsHashtable
            foreach ($key in @('draft','prerelease','body')) {
                if ($patch.Contains($key)) { $state[$key] = $patch[$key] }
            }
            return @{id=42;draft=$state.draft;prerelease=$state.prerelease;body=$state.body;tag_name=$tag;html_url="https://github.com/$repo/releases/tag/$tag"} | ConvertTo-Json -Compress
        }
        return @{id=42;draft=$state.draft;prerelease=$state.prerelease;body=$state.body;tag_name=$tag;target_commitish=$commit;assets=@($state.assets.Values);html_url="https://github.com/$repo/releases/tag/$tag"} |
            ConvertTo-Json -Compress -Depth 4
    }
    if ($args[0] -eq 'release' -and $args[1] -eq 'create') {
        $notes = $args[[array]::IndexOf($args, '--notes') + 1]
        if (!$notes.Contains('<!-- yamamuki-data:temporary-draft:v1 -->') -or
            $args -notcontains '--draft' -or $args -notcontains '--verify-tag' -or
            $state.pendingCommit -cne $commit) { throw '受け渡しDraftの設定が不正です。' }
        $state.created = $true
        $state.body = $notes
        return
    }
    if ($args[0] -eq 'workflow' -and $args[1] -eq 'run') {
        if (!$state.created -or $args[2] -cne 'publish-surface.yml' -or
            $args[[array]::IndexOf($args, '--ref') + 1] -cne 'main') { throw '加工Actionの起動が不正です。' }
        $state.started = $true
        return
    }
    if ($args[0] -eq 'release' -and $args[1] -eq 'download') {
        $dir = [array]::IndexOf($args, '--dir')
        for ($i = 0; $i -lt $args.Count; $i++) {
            if ($args[$i] -ne '--pattern') { continue }
            $source = Join-Path $inputDir $args[$i + 1]
            $output = [array]::IndexOf($args, '--output')
            $destination = if ($output -ge 0) { $args[$output + 1] } else { $args[$dir + 1] }
            Copy-Item -LiteralPath $source -Destination $destination
            if ($output -ge 0 -and [IO.Path]::GetFileName($destination).StartsWith('.verify-', [StringComparison]::Ordinal)) {
                # WindowsでもLinuxのドットファイルと同じ隠し属性で検査する。
                if ($IsWindows) { [IO.File]::SetAttributes($destination, [IO.FileAttributes]::Hidden) }
                $state.hiddenVerifications++
            }
        }
        return
    }
    if ($args[0] -eq 'release' -and $args[1] -eq 'upload') {
        if (!$state.draft) { throw '公開済みReleaseへの添付です。' }
        $file = Get-Item -LiteralPath $args[3]
        if ($state.failUpload -and $state.uploaded.Count -eq 2) { throw 'テスト用の転送エラー' }
        Copy-Item -LiteralPath $file.FullName -Destination $inputDir -Force
        Add-TestAsset $file
        $state.uploaded += $file.Name
        return
    }
    throw "未定義のgh呼び出しです: $args"
}

try {
    $timestamp = @{}
    if ($DataTimestamp) { $timestamp.DataTimestamp = $DataTimestamp }
    & (Join-Path $scripts 'prepare_release.ps1') -Dataset esa_worldcover -Database $Database -OutputDirectory $inputDir -TargetCommit $commit -ProcessDraft @timestamp
    if (!$state.created -or !$state.started) { throw '受け渡しDraftまたは加工Actionが作成されませんでした。' }
    Get-ChildItem -LiteralPath $inputDir -File | ForEach-Object { Add-TestAsset $_ }
    $manifest = Get-Content -LiteralPath (Join-Path $inputDir 'esa-worldcover-z12.manifest.json') -Raw | ConvertFrom-Json
    $suffix = if ($IsWindows) { ".exe" } else { "" }
    $tool = Join-Path (Split-Path $PSScriptRoot -Parent) "target/release/surface-tool$suffix"
    $tag = "esa-worldcover-japan-$($manifest.version)"
    $env:GITHUB_REPOSITORY = $repo
    $env:GITHUB_REF = 'refs/heads/main'
    $env:GITHUB_STEP_SUMMARY = Join-Path $testRoot 'summary.md'
    $publish = @{
        Dataset = 'esa_worldcover'; Tag = $tag; Sha256 = $manifest.sha256; TargetCommit = $commit
    }
    try {
        & (Join-Path $scripts 'publish_release.ps1') @publish
        throw '転送失敗を検出できませんでした。'
    } catch {
        if ($_.Exception.Message -ne 'テスト用の転送エラー') { throw }
    }
    if (!$state.draft -or $state.assets.Count -ne 5) { throw '途中失敗時に下書きが保持されませんでした。' }
    # 正常なSQLiteは再利用し、壊れたmanifestだけを差し替える。
    $z9Manifest = Join-Path $inputDir 'esa-worldcover-z9.manifest.json'
    Set-Content -LiteralPath $z9Manifest -Value '{}' -Encoding utf8NoBOM
    Add-TestAsset (Get-Item -LiteralPath $z9Manifest)
    $state.assets['esa-worldcover-z9.sqlite3'].digest = $null
    $state.failUpload = $false
    $state.uploaded = @()
    & (Join-Path $scripts 'publish_release.ps1') @publish
    if (!$state.draft -or $state.prerelease -or $state.assets.Count -ne 9 -or $state.uploaded.Count -ne 5 -or
        $state.uploaded -notcontains 'esa-worldcover-z9.manifest.json' -or
        $state.uploaded -contains 'esa-worldcover-z9.sqlite3') {
        throw '再実行時の添付・公開結果が不正です。'
    }
    if ($state.hiddenVerifications -eq 0) { throw '隠しファイルによる添付の再検証が実行されませんでした。' }
    if (!$state.body.Contains('<!-- yamamuki-data:temporary-draft:v1 -->') -or
        !$state.body.Contains('<!-- yamamuki-data:surface-draft:v1 ')) {
        throw '加工後のDraftに整理マーカーまたは検査記録がありません。'
    }
    try {
        & (Join-Path $scripts 'publish_release.ps1') @publish -Operation Formalize
        throw 'Draftからの正式公開を拒否できませんでした。'
    } catch {
        if ($_.Exception.Message -notmatch 'Releaseの状態が指定操作に適合しません') { throw }
    }
    $state.uploaded = @()
    & (Join-Path $scripts 'publish_release.ps1') @publish -Operation Preview
    if ($state.draft -or !$state.prerelease -or $state.uploaded.Count -ne 0 -or
        $state.body.Contains('<!-- yamamuki-data:temporary-draft:v1 -->')) {
        throw '試用公開の状態、整理マーカーまたは添付の不変性が不正です。'
    }
    & (Join-Path $scripts 'publish_release.ps1') @publish -Operation Formalize -Reason 'アプリで確認'
    if ($state.draft -or $state.prerelease -or $state.uploaded.Count -ne 0 -or
        !$state.body.Contains('アプリで確認')) {
        throw '正式公開の状態または添付の不変性が不正です。'
    }
    foreach ($operation in @('Prepare','Preview')) {
        try {
            & (Join-Path $scripts 'publish_release.ps1') @publish -Operation $operation
            throw '正式公開済みのReleaseに不正な操作が通りました。'
        } catch {
            if ($_.Exception.Message -notmatch 'Releaseの状態が指定操作に適合しません') { throw }
        }
    }
    foreach ($z in 9..12) {
        $name = "esa-worldcover-z$z"
        $manifestName = "$name.manifest.json"
        $variant = Get-Content -LiteralPath (Join-Path $inputDir $manifestName) -Raw | ConvertFrom-Json -AsHashtable
        $database = Join-Path $inputDir "$name.sqlite3"
        $hash = (Get-FileHash -LiteralPath $database).Hash
        $levels = & $tool surface-levels $database | ConvertFrom-Json -AsHashtable
        if ($variant.Contains('surface') -or $variant.Contains('surfaceLayerIds') -or $variant.Contains('surfaceLevels') -or
            (Compare-Object @('forest','hillshade','water') @($levels.Keys) -CaseSensitive) -or
            $variant.downloadUrl -cne "https://github.com/$repo/releases/download/$tag/$name.sqlite3" -or
            $variant.licenseUrl -cne "https://github.com/$repo/releases/download/$tag/LICENSE.txt" -or
            $variant.sha256 -ine $hash) {
            throw "配布manifestが不正です: $name"
        }
        foreach ($layer in @('forest','hillshade','water')) {
            if (($levels[$layer] -join ',') -cne ((5..$z) -join ',')) {
                throw "配布SQLiteの収録レベルが不正です: $name / $layer"
            }
        }
    }
    Write-Output 'Draftの加工、転送失敗と再実行、試用・正式公開、マーカーと添付の不変性を確認しました。'
} finally {
    foreach ($key in $previousEnv.Keys) { [Environment]::SetEnvironmentVariable($key, $previousEnv[$key], 'Process') }
    if (Test-Path -LiteralPath $inputDir -PathType Container) {
        Get-ChildItem -LiteralPath $inputDir -File | Remove-Item
        [IO.Directory]::Delete($inputDir)
    }
    $summary = Join-Path $testRoot 'summary.md'
    if (Test-Path -LiteralPath $summary) { Remove-Item -LiteralPath $summary }
    [IO.Directory]::Delete($testRoot)
}
