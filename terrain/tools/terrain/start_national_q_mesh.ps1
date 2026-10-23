param([int]$Workers = 4, [string]$Output = 'build/terrain-q4-q12-basic.sqlite3')
$ErrorActionPreference = 'Stop'
$terrainRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$outputPath = [System.IO.Path]::GetFullPath((Join-Path $terrainRoot $Output))
$pidFile = [System.IO.Path]::ChangeExtension($outputPath, 'pid')
if (Test-Path -LiteralPath $pidFile) {
    $existingId = [int](Get-Content -LiteralPath $pidFile)
    $existing = Get-CimInstance Win32_Process -Filter "ProcessId=$existingId"
    if ($existing -and ($existing.CommandLine -like '*national_q_mesh.py*' -or $existing.CommandLine -like '*terrain-q-national*')) {
        throw '同じ全国生成が起動しています。進捗とログを確認してください。'
    }
}
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$stdout = Join-Path $terrainRoot "build/terrain-q4-q12-$stamp.log"
$stderr = Join-Path $terrainRoot "build/terrain-q4-q12-$stamp.err"
$binary = Join-Path $terrainRoot 'build/terrain-q-rust-target/release/terrain-q-national.exe'
if (!(Test-Path -LiteralPath $binary)) { throw '先に tools/terrain-rs/build.ps1 -QNational で生成器をビルドしてください。' }
$process = Start-Process -FilePath $binary -ArgumentList @('--workers',"$Workers",'--output',('"' + $outputPath + '"')) -WorkingDirectory $terrainRoot -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
$process.Id | Set-Content -LiteralPath $pidFile -Encoding ascii
@{ engine='rust'; pid=$process.Id; stdout=$stdout; stderr=$stderr; stage='basic-before-national-pool'; output=$outputPath } | ConvertTo-Json | Set-Content -LiteralPath ([System.IO.Path]::ChangeExtension($outputPath, 'launch.json')) -Encoding utf8
Write-Output "全国基本生成を非表示で起動しました。PID=$($process.Id) ログ=$stdout"
