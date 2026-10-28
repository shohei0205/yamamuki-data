param(
    [string]$Source = 'build/terrain-lod.sqlite3',
    [string]$Cache = 'build/terrain-cache',
    [string]$OutputDirectory = 'build/terrain-national-natural-v1',
    [switch]$Resume,
    [switch]$Background
)
$ErrorActionPreference = 'Stop'
$terrainRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
Set-Location -LiteralPath $terrainRoot
$python = Join-Path $terrainRoot 'build/terrain-venv/Scripts/python.exe'
if (-not (Test-Path -LiteralPath $python -PathType Leaf)) { throw 'terrain-venvのPythonがありません' }
$target = [IO.Path]::GetFullPath((Join-Path $terrainRoot $OutputDirectory))
$arguments = @('-X', 'utf8', (Join-Path $PSScriptRoot 'prepare_national_natural.py'), '--source', [IO.Path]::GetFullPath($Source), '--cache', [IO.Path]::GetFullPath($Cache), '--output', $target)
if ($Resume) { $arguments += '--resume' }
if (-not $Background) {
    & $python @arguments
    if ($LASTEXITCODE -ne 0) { throw "全国入力の準備に失敗しました: $LASTEXITCODE" }
    return
}
New-Item -ItemType Directory -Path $target -Force | Out-Null
$tag = Get-Date -Format 'yyyyMMdd-HHmmss-ffff'
$stdout = Join-Path $target "prepare-$tag.stdout.log"
$stderr = Join-Path $target "prepare-$tag.stderr.log"
# 実行中の同一出力はPython側のSQLite排他ロックで拒否する。
$quoted = $arguments | ForEach-Object {
    if ($_ -match '"') { throw '引数には二重引用符を含められません' }
    '"' + $_ + '"'
}
$job = Start-Process -FilePath $python -ArgumentList $quoted -WorkingDirectory $terrainRoot -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
$launch = [ordered]@{pid=$job.Id; started=(Get-Date).ToString('o'); stage='inventory-only'; arguments=$arguments; stdout=$stdout; stderr=$stderr; output=$target}
$json = ($launch | ConvertTo-Json -Depth 5) + "`n"
[IO.File]::WriteAllText((Join-Path $target "launch-$tag.json"), $json, [Text.UTF8Encoding]::new($false))
$json
