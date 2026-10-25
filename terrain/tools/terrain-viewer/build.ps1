param([switch]$Test, [switch]$Check)
$ErrorActionPreference = 'Stop'
$terrain = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$localCargo = Join-Path $terrain 'build/rust-tools/cargo/bin/cargo.exe'
if (Test-Path -LiteralPath $localCargo) {
    $env:CARGO_HOME = Join-Path $terrain 'build/rust-tools/cargo'
    $env:RUSTUP_HOME = Join-Path $terrain 'build/rust-tools/rustup'
    $compiler = Get-ChildItem (Join-Path $terrain 'build/rust-tools/llvm-mingw') -Directory | Select-Object -First 1
    $env:PATH = "$($compiler.FullName)/bin;$env:PATH"
    $env:CC = 'clang'
    $env:CXX = 'clang++'
    $env:AR = 'llvm-ar'
    $env:RUSTFLAGS = '-C link-self-contained=yes -C linker=rust-lld'
    $cargo = $localCargo
} else { $cargo = (Get-Command cargo -ErrorAction Stop).Source }
$env:CARGO_TARGET_DIR = Join-Path $terrain 'build/terrain-viewer-target'
$action = if ($Test) { 'test' } elseif ($Check) { 'check' } else { 'build' }
$arguments = @($action, '--locked', '--manifest-path', (Join-Path $PSScriptRoot 'Cargo.toml'))
if (-not $Check) { $arguments += '--release' }
& $cargo @arguments
exit $LASTEXITCODE
