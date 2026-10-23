param([switch]$Test, [switch]$NoRun, [switch]$Check, [switch]$Codecs, [switch]$AdaptiveTin, [switch]$AdaptiveNational, [switch]$QNational, [switch]$QPack, [string]$TargetDirectory)
$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$localCargo = Join-Path $repo 'build/rust-tools/cargo/bin/cargo.exe'
if (Test-Path -LiteralPath $localCargo) {
    $env:CARGO_HOME = Join-Path $repo 'build/rust-tools/cargo'
    $env:RUSTUP_HOME = Join-Path $repo 'build/rust-tools/rustup'
    $compiler = Get-ChildItem (Join-Path $repo 'build/rust-tools/llvm-mingw') -Directory | Select-Object -First 1
    if (-not $compiler) { throw 'ローカルの LLVM MinGW がありません。' }
    $env:PATH = "$($compiler.FullName)/bin;$env:PATH"
    $env:CC = 'clang'
    $env:CXX = 'clang++'
    $env:AR = 'llvm-ar'
    $env:RUSTFLAGS = '-C link-self-contained=yes -C linker=rust-lld'
    if ($Codecs) {
        $cppLibrary = Join-Path $compiler.FullName 'x86_64-w64-mingw32/lib'
        # LLVM の C++ 実行時だけを明示し、Rust 付属の MinGW ライブラリは置き換えない。
        $env:RUSTFLAGS += " -C link-arg=$cppLibrary/libc++.dll.a"
    }
    $cargo = $localCargo
} else {
    $cargo = (Get-Command cargo -ErrorAction Stop).Source
}
$env:CARGO_TARGET_DIR = if ($TargetDirectory) { $TargetDirectory } else { Join-Path $repo 'build/terrain-rust-target' }
$manifest = Join-Path $PSScriptRoot 'Cargo.toml'
$binaryName = if ($QPack) { 'terrain-q-pack' } elseif ($QNational) { 'terrain-q-national' } elseif ($AdaptiveNational) { 'terrain-adaptive-national' } elseif ($AdaptiveTin) { 'terrain-adaptive-tin' } elseif ($Codecs) { 'terrain-codec-bench' } else { 'terrain-generator' }
if ($Check) { & $cargo check --locked --manifest-path $manifest --bin $binaryName --tests }
elseif ($Test -and $NoRun) { & $cargo test --locked --no-run --manifest-path $manifest --bin $binaryName }
elseif ($Test) { & $cargo test --locked --manifest-path $manifest --bin $binaryName }
else { & $cargo build --locked --release --manifest-path $manifest --bin $binaryName }
exit $LASTEXITCODE
