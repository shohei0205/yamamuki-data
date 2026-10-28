$ErrorActionPreference='Stop'
$terrainRoot=(Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$compiler=Join-Path $terrainRoot 'build/zig-x86_64-windows-0.15.2/zig.exe'
if(-not(Test-Path -LiteralPath $compiler)){throw '公式Zig 0.15.2のCコンパイラーをbuildへ用意してください'}
$env:ZIG_GLOBAL_CACHE_DIR=Join-Path $terrainRoot 'build/zig-wasm-cache'
$env:ZIG_LOCAL_CACHE_DIR=Join-Path $terrainRoot 'build/zig-wasm-local'
& $compiler cc -target wasm32-freestanding -O3 -ffp-contract=off -fno-builtin -nostdlib '-Wl,--no-entry' '-Wl,--export=prepare_neighbors' '-Wl,--export=build_meshes' '-Wl,--export=__heap_base' '-Wl,--export-memory' '-Wl,-z,stack-size=1048576' '-Wl,--initial-memory=2097152' '-Wl,--max-memory=1073741824' (Join-Path $PSScriptRoot 'sibson_variable_mesh.c') -o (Join-Path $terrainRoot 'build/sibson_variable_mesh.wasm')
if($LASTEXITCODE -ne 0){throw 'WASMビルドに失敗しました'}
