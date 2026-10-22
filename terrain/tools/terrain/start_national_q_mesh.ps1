param([int]$Workers = 1)
$ErrorActionPreference = 'Stop'
$terrainRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$pidFile = Join-Path $terrainRoot 'build/terrain-q4-q12-basic.pid'
if (Test-Path -LiteralPath $pidFile) {
    $existingId = [int](Get-Content -LiteralPath $pidFile)
    $existing = Get-CimInstance Win32_Process -Filter "ProcessId=$existingId"
    if ($existing -and $existing.CommandLine -like '*national_q_mesh.py*') {
        throw '同じ全国生成が起動しています。進捗とログを確認してください。'
    }
}
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$stdout = Join-Path $terrainRoot "build/terrain-q4-q12-$stamp.log"
$stderr = Join-Path $terrainRoot "build/terrain-q4-q12-$stamp.err"
$python = Join-Path $terrainRoot 'build/terrain-venv/Scripts/python.exe'
$process = Start-Process -FilePath $python -ArgumentList @('-X','utf8','-u','tools/terrain/national_q_mesh.py','--workers',"$Workers") -WorkingDirectory $terrainRoot -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
$process.Id | Set-Content -LiteralPath $pidFile -Encoding ascii
@{ pid=$process.Id; stdout=$stdout; stderr=$stderr; stage='basic-before-national-pool'; output='build/terrain-q4-q12-basic.sqlite3' } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $terrainRoot 'build/terrain-q4-q12-basic.launch.json') -Encoding utf8
Write-Output "全国基本生成を非表示で起動しました。PID=$($process.Id) ログ=$stdout"
