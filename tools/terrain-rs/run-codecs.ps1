param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)
$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$binary = Join-Path $repo 'build/terrain-rust-target/release/terrain-codec-bench.exe'
if (-not (Test-Path -LiteralPath $binary)) { throw '先に tools/terrain-rs/build.ps1 -Codecs を実行してください。' }
$compiler = Get-ChildItem (Join-Path $repo 'build/rust-tools/llvm-mingw') -Directory -ErrorAction SilentlyContinue | Select-Object -First 1
if ($compiler) { $env:PATH = "$($compiler.FullName)/bin;$env:PATH" }
& $binary @Arguments
exit $LASTEXITCODE
