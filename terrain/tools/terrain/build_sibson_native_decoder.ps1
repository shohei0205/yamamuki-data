param([switch]$Test)
$ErrorActionPreference='Stop'
$terrainRoot=(Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$env:RUSTUP_HOME=Join-Path $terrainRoot 'build/rust-tools/rustup'
$env:CARGO_HOME=Join-Path $terrainRoot 'build/rust-tools/cargo'
$compiler=Join-Path $env:CARGO_HOME 'bin/rustc.exe'
$source=Join-Path $terrainRoot 'tools/terrain-rs/src/sibson_native_decoder.rs'
$output=Join-Path $terrainRoot 'build/sibson-native-codec-v1'
New-Item -ItemType Directory -Force $output | Out-Null
if($Test){
 & $compiler --edition=2021 --test -C opt-level=3 -C link-self-contained=yes -C linker=rust-lld $source -o (Join-Path $output 'decoder-tests.exe')
 if($LASTEXITCODE -ne 0){throw 'Rustテストのビルド失敗'}
 & (Join-Path $output 'decoder-tests.exe')
 if($LASTEXITCODE -ne 0){throw 'Rustテスト失敗'}
}
& $compiler --edition=2021 --crate-type=cdylib -C opt-level=3 -C lto=fat -C panic=abort -C link-self-contained=yes -C linker=rust-lld $source -o (Join-Path $output 'sibson_native_decoder.dll')
if($LASTEXITCODE -ne 0){throw 'Rust DLLビルド失敗'}
& $compiler --edition=2021 --crate-type=cdylib --target wasm32-unknown-unknown -C opt-level=3 -C lto=fat -C panic=abort $source -o (Join-Path $output 'sibson_native_decoder.wasm')
if($LASTEXITCODE -ne 0){throw 'Rust WASMビルド失敗'}
