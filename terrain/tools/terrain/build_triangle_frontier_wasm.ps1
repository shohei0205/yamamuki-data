param([string]$NodeExecutable)
$ErrorActionPreference = 'Stop'
if (!$NodeExecutable) { $NodeExecutable = (Get-Command node -ErrorAction Stop).Source }
& $NodeExecutable (Join-Path $PSScriptRoot 'build_triangle_frontier_wasm.mjs')
exit $LASTEXITCODE
