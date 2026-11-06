param(
    [Parameter(Mandatory)][string]$Generator,
    [Parameter(Mandatory)][string]$Geos,
    [Parameter(Mandatory)][string]$SourceRoot,
    [Parameter(Mandatory)][string]$CoarseSourceRoot,
    [Parameter(Mandatory)][string]$Output
)

$ErrorActionPreference = 'Stop'
$Generator = (Resolve-Path -LiteralPath $Generator).Path
$Geos = (Resolve-Path -LiteralPath $Geos).Path
$env:PATH = "$(Split-Path -Parent $Geos);$env:PATH"
if (Test-Path -LiteralPath $Output) {
    throw '既存の出力先とは別の場所を指定してください。'
}
foreach ($level in 3..6) {
    $source = if ($level -le 4) { $SourceRoot } else { $CoarseSourceRoot }
    $stage = Join-Path $Output "stage-l$level"
    $final = Join-Path $Output "l$level"
    $flags = @('--geos', $Geos, '--filters', 'majority-clean', '--source-levels',
               '--only-level', "$level", '--compact', '--implicit-background')
    & $Generator @flags --source-root $source --output $stage --water-overlay --unified-water --direct-lines --common-lod
    if ($LASTEXITCODE) { throw "L$level の共通生成に失敗しました。" }
    & $Generator @flags --source-root $stage --output $final --linear-source --common-finish
    if ($LASTEXITCODE) { throw "L$level の仕上げに失敗しました。" }
}
