param([int]$Port=61491,[string]$Output='build/natural-xyz-hierarchy-z13-25-upquarter-local-v1')
$ErrorActionPreference='Stop'
$taskRoot=Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$taskPython=Join-Path $taskRoot 'build/terrain-venv/Scripts/python.exe'
$taskOutput=if([IO.Path]::IsPathRooted($Output)){[IO.Path]::GetFullPath($Output)}else{[IO.Path]::GetFullPath((Join-Path $taskRoot $Output))}
if(-not(Test-Path -LiteralPath (Join-Path $taskOutput 'settings.json'))){throw '生成設定が見つかりません。'}
$taskExisting=Get-CimInstance Win32_Process | Where-Object {$_.CommandLine -like '*natural_hierarchy_viewer_server.py*' -and $_.CommandLine -match "--port\s+$Port\b"}
if($taskExisting){Write-Output "起動済み: http://127.0.0.1:$Port/";return}
$taskLog=Join-Path $taskOutput "viewer-$Port"
$taskProcess=Start-Process -FilePath $taskPython -ArgumentList @('-X','utf8','tools/terrain/natural_hierarchy_viewer_server.py','--port',"$Port",'--output',('"'+$taskOutput+'"')) -WorkingDirectory $taskRoot -WindowStyle Hidden -RedirectStandardOutput "$taskLog.stdout.log" -RedirectStandardError "$taskLog.stderr.log" -PassThru
@{pid=$taskProcess.Id;port=$Port;output=$taskOutput} | ConvertTo-Json | Set-Content -LiteralPath "$taskLog.launch.json" -Encoding utf8
Write-Output "http://127.0.0.1:$Port/"
