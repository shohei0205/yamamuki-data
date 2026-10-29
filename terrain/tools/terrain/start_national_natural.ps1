param(
    [ValidateSet('Prepare','Generate')][string]$Mode = 'Prepare',
    [string]$OutputDirectory = 'build/terrain-national-natural-20261029',
    [string]$Source = 'build/terrain-lod.sqlite3',
    [string]$Cache = 'build/terrain-cache',
    [string]$Records = 'M:/GDIData/yamamuki-terrain/records/dem-downloads.jsonl',
    [int]$Workers = 0,
    [switch]$Resume,
    [switch]$Background,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$terrainRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
Set-Location -LiteralPath $terrainRoot
$target = [IO.Path]::GetFullPath((Join-Path $terrainRoot $OutputDirectory))
if ($Workers -eq 0) { $Workers = [Math]::Max(1,[Math]::Floor([Environment]::ProcessorCount / 2)) }
if ($Workers -lt 1 -or $Workers -gt [Math]::Max(1,[Math]::Floor([Environment]::ProcessorCount / 2))) { throw '並列数は論理CPUの半分以下を指定してください' }
$buildTarget = Join-Path $terrainRoot 'build/natural-national-target'
if (-not $SkipBuild) {
    & (Join-Path $terrainRoot 'tools/terrain-rs/build.ps1') -NaturalNational -Test -TargetDirectory $buildTarget
    if ($LASTEXITCODE) { throw 'Rustテストに失敗しました' }
    & (Join-Path $terrainRoot 'tools/terrain-rs/build.ps1') -NaturalNational -TargetDirectory $buildTarget
    if ($LASTEXITCODE) { throw 'Rust releaseビルドに失敗しました' }
}
$python = Join-Path $terrainRoot 'build/terrain-venv/Scripts/python.exe'
$engine = Join-Path $buildTarget 'release/terrain-natural-national.exe'
if (-not (Test-Path -LiteralPath $engine)) { throw 'Rust生成器がありません' }
New-Item -ItemType Directory -Path $target -Force | Out-Null
$arguments = @('-X','utf8',(Join-Path $PSScriptRoot 'run_national_natural.py'),'--mode',$Mode.ToLowerInvariant(),'--source',[IO.Path]::GetFullPath($Source),'--cache',[IO.Path]::GetFullPath($Cache),'--records',[IO.Path]::GetFullPath($Records),'--engine',$engine,'--output',$target,'--workers',"$Workers")
if ($Resume) { $arguments += '--resume' }
if (-not $Background) { & $python @arguments; if ($LASTEXITCODE) { throw '全国生成の段階実行が失敗しました' }; return }
$tag=Get-Date -Format 'yyyyMMdd-HHmmss-ffff'
$stdout=Join-Path $target "pipeline-$tag.stdout.log"
$stderr=Join-Path $target "pipeline-$tag.stderr.log"
$quoted=$arguments | ForEach-Object { if ($_ -match '"') { throw '引数に二重引用符は指定できません' }; '"'+$_+'"' }
$process=Start-Process -FilePath $python -ArgumentList $quoted -WorkingDirectory $terrainRoot -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
$launch=[ordered]@{pid=$process.Id;mode=$Mode;started=(Get-Date).ToString('o');arguments=$arguments;stdout=$stdout;stderr=$stderr;output=$target}
[IO.File]::WriteAllText((Join-Path $target "launch-pipeline-$tag.json"),($launch|ConvertTo-Json -Depth 5)+"`n",[Text.UTF8Encoding]::new($false))
$launch|ConvertTo-Json -Depth 5
