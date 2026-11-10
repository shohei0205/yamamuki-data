param([Parameter(Mandatory)][string]$Database)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$scripts = Join-Path (Split-Path $PSScriptRoot -Parent) 'scripts'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('surface-publish-test-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $testRoot | Out-Null
$inputDir = Join-Path $testRoot 'input'
$commit = '1' * 40
$repo = 'shohei0205/yamamuki-data'
$state = @{ draft = $true; assets = @{}; nextId = 1; uploaded = @(); failUpload = $true }
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

function gh {
    if ($args[0] -eq 'api') {
        if ($args[1] -eq '--paginate') {
            return '[[{"id":42,"tag_name":"' + $tag + '"}]]'
        }
        if ($args[1] -like '*/git/ref/tags/*') {
            return @{object=@{type='commit';sha=$commit}} | ConvertTo-Json -Compress
        }
        if ($args[1] -eq '--method') {
            if ($args[2] -ne 'PATCH' -or $state.assets.Count -ne 9) { throw '不完全なReleaseを公開しようとしました。' }
            $state.draft = $false
            return @{draft=$false;tag_name=$tag;html_url="https://github.com/$repo/releases/tag/$tag"} | ConvertTo-Json -Compress
        }
        return @{draft=$state.draft;prerelease=$false;tag_name=$tag;target_commitish=$commit;assets=@($state.assets.Values)} |
            ConvertTo-Json -Compress -Depth 4
    }
    if ($args[0] -eq 'release' -and $args[1] -eq 'download') {
        $dir = [array]::IndexOf($args, '--dir')
        for ($i = 0; $i -lt $args.Count; $i++) {
            if ($args[$i] -ne '--pattern') { continue }
            $source = Join-Path $inputDir $args[$i + 1]
            $output = [array]::IndexOf($args, '--output')
            $destination = if ($output -ge 0) { $args[$output + 1] } else { $args[$dir + 1] }
            Copy-Item -LiteralPath $source -Destination $destination
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
    & (Join-Path $scripts 'prepare_release.ps1') -Dataset esa_worldcover -Database $Database -OutputDirectory $inputDir
    Get-ChildItem -LiteralPath $inputDir -File | ForEach-Object { Add-TestAsset $_ }
    $manifest = Get-Content -LiteralPath (Join-Path $inputDir 'manifest.json') -Raw | ConvertFrom-Json
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
    if ($state.draft -or $state.assets.Count -ne 9 -or $state.uploaded.Count -ne 5 -or
        $state.uploaded -notcontains 'esa-worldcover-z9.manifest.json' -or
        $state.uploaded -contains 'esa-worldcover-z9.sqlite3') {
        throw '再実行時の添付・公開結果が不正です。'
    }
    foreach ($z in 9..12) {
        $name = "esa-worldcover-z$z"
        $manifestName = if ($z -eq 12) { 'manifest.json' } else { "$name.manifest.json" }
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
    Write-Output '公開Actionの分割、転送失敗、再実行を確認しました。'
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
