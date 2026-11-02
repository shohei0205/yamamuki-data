param([int]$Port=61493,[string]$Database='build/natural-shared-packed-v1/terrain-shared-xyz.sqlite3')
$ErrorActionPreference='Stop'
$taskRoot=Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$taskDatabase=if([IO.Path]::IsPathRooted($Database)){$Database}else{Join-Path $taskRoot $Database}
if(-not(Test-Path -LiteralPath $taskDatabase)){throw 'SQLite is not found.'}
$taskExisting=Get-CimInstance Win32_Process | Where-Object {$_.CommandLine -like '*natural_shared_sqlite_viewer.py*' -and $_.CommandLine -match "--port\s+$Port\b"}
if($taskExisting){Write-Output "http://127.0.0.1:$Port/";return}
$taskLog=Join-Path $taskRoot "build/natural-shared-sqlite-viewer-$Port"
$taskProcess=Start-Process -FilePath (Join-Path $taskRoot 'build/terrain-venv/Scripts/python.exe') -ArgumentList @('-X','utf8','tools/terrain/natural_shared_sqlite_viewer.py','--port',"$Port",'--database',('"'+$taskDatabase+'"')) -WorkingDirectory $taskRoot -WindowStyle Hidden -RedirectStandardOutput "$taskLog.stdout.log" -RedirectStandardError "$taskLog.stderr.log" -PassThru
@{pid=$taskProcess.Id;port=$Port;database=$taskDatabase} | ConvertTo-Json | Set-Content -LiteralPath "$taskLog.launch.json" -Encoding utf8
Write-Output "http://127.0.0.1:$Port/"
