$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$compiler = Join-Path $root 'build/zig-x86_64-windows-0.15.2/zig.exe'
if (!(Test-Path -LiteralPath $compiler)) { throw 'Zig 0.15.2 を build に用意してください。' }
$env:ZIG_GLOBAL_CACHE_DIR = Join-Path $root 'build/zig-wasm-cache'
$env:ZIG_LOCAL_CACHE_DIR = Join-Path $root 'build/zig-wasm-local'
& $compiler cc -target x86_64-windows-gnu -O3 -shared (Join-Path $PSScriptRoot 'sibson_global_c1_eval.c') -o (Join-Path $root 'build/sibson_global_c1_eval.dll')
if ($LASTEXITCODE -ne 0) { throw '全域C1評価器のビルドに失敗しました。' }
