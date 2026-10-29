$ErrorActionPreference = 'Stop'
$terrainRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$existing = Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'python.exe' -and $_.CommandLine -match 'generate_errorfit_full_c1_128[.]py' }
if ($existing) { $existing | Select-Object ProcessId,CommandLine; return }
$runRoot = Join-Path $terrainRoot 'build/natural-raw-c1-128-errorfit-full-v1'
New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
$tag = Get-Date -Format 'yyyyMMdd-HHmmss'
$stdoutPath = Join-Path $runRoot "generation-$tag.stdout.log"
$stderrPath = Join-Path $runRoot "generation-$tag.stderr.log"
$pythonPath = Join-Path $terrainRoot 'build/terrain-venv/Scripts/python.exe'
$workerProcess = Start-Process -FilePath $pythonPath -ArgumentList '-X','utf8','tools/terrain/generate_errorfit_full_c1_128.py' -WorkingDirectory $terrainRoot -WindowStyle Hidden -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath -PassThru
@{pid=$workerProcess.Id;stdout=$stdoutPath;stderr=$stderrPath;workers=[Math]::Max(1,[Math]::Floor([Environment]::ProcessorCount/2));script='tools/terrain/generate_errorfit_full_c1_128.py'} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $runRoot 'launch.json') -Encoding UTF8
$workerProcess.Id
