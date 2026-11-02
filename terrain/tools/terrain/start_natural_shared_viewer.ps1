param([int]$Port=61492,[string]$Output='build/natural-shared-128-v1')
$ErrorActionPreference='Stop'
$taskRoot=Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$taskOutput=if([IO.Path]::IsPathRooted($Output)){$Output}else{Join-Path $taskRoot $Output}
if(-not(Test-Path -LiteralPath (Join-Path $taskOutput 'summary.json'))){throw 'Comparison data is not ready.'}
$taskExisting=Get-CimInstance Win32_Process | Where-Object {$_.CommandLine -like '*natural_shared_viewer_server.py*' -and $_.CommandLine -match "--port\s+$Port\b"}
if($taskExisting){Write-Output "http://127.0.0.1:$Port/";return}
$taskPython=Join-Path $taskRoot 'build/terrain-venv/Scripts/python.exe'
$taskLog=Join-Path $taskOutput "viewer-$Port"
$taskProcess=Start-Process -FilePath $taskPython -ArgumentList @('-X','utf8','tools/terrain/natural_shared_viewer_server.py','--port',"$Port",'--output',('"'+$taskOutput+'"')) -WorkingDirectory $taskRoot -WindowStyle Hidden -RedirectStandardOutput "$taskLog.stdout.log" -RedirectStandardError "$taskLog.stderr.log" -PassThru
@{pid=$taskProcess.Id;port=$Port;output=$taskOutput} | ConvertTo-Json | Set-Content -LiteralPath "$taskLog.launch.json" -Encoding utf8
Write-Output "http://127.0.0.1:$Port/"
