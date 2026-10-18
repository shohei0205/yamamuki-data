param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)
$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$binary = Join-Path $repo 'build/terrain-rust-target/release/terrain-generator.exe'
if (-not (Test-Path -LiteralPath $binary)) { throw '先に tools/terrain-rs/build.ps1 を実行してください。' }
$native = if ($env:TERRAIN_GEOS_DIR) { $env:TERRAIN_GEOS_DIR } else { Join-Path $repo 'build/rust-native' }
$env:PATH = "$native;$env:PATH"
if ($Arguments.Count -and $Arguments[0] -in @('generate', 'cluster', 'coast') -and '--library' -notin $Arguments) {
    $library = Get-ChildItem -LiteralPath $native -Filter 'geos_c*.dll' | Select-Object -First 1
    if (-not $library) { throw 'GEOS の DLL を TERRAIN_GEOS_DIR に指定してください。' }
    $Arguments += @('--library', $library.FullName)
}
& $binary @Arguments
exit $LASTEXITCODE
