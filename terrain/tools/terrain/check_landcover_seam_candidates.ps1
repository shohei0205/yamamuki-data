param(
    [Parameter(Mandatory)][string]$Generator,
    [Parameter(Mandatory)][string]$Geos,
    [Parameter(Mandatory)][string]$Regions,
    [Parameter(Mandatory)][string]$Output,
    [ValidateRange(1, 8)][int]$Jobs = 4
)
$ErrorActionPreference = 'Stop'
$Generator = (Resolve-Path -LiteralPath $Generator).Path
$Geos = (Resolve-Path -LiteralPath $Geos).Path
$Regions = (Resolve-Path -LiteralPath $Regions).Path
$env:PATH = "$(Split-Path -Parent $Geos);$env:PATH"
$audit = Get-Content (Join-Path $Regions 'seam-audit.json') -Raw | ConvertFrom-Json

function Find-Level([int[]]$Key, [int]$Level) {
    $name = "l$Level"
    $region = Join-Path $Regions ($Key -join '-')
    $candidate = Get-ChildItem -LiteralPath $region -Directory -Filter "$name*" |
        Where-Object {
            ($_.Name -eq $name -or $_.Name.StartsWith("$name-retry-")) -and
            (Test-Path -LiteralPath (Join-Path $_.FullName 'manifest.json'))
        } | Sort-Object Name -Descending | Select-Object -First 1
    if (!$candidate) { throw "Missing level $Level in $region" }
    Join-Path $candidate.FullName 'majority-clean'
}
$tasks = foreach ($level in $audit.levels) {
    foreach ($edge in $level.edges) {
        $id = "L$($level.level)-$($edge.from -join '-')-$($edge.to -join '-')"
        [pscustomobject]@{
            Level = $level.level
            Id = $id
            Keys = (@($edge.from) + @($edge.to)) -join ','
            First = Find-Level $edge.from $level.level
            Second = Find-Level $edge.to $level.level
            Output = Join-Path $Output $id
        }
    }
}
New-Item -ItemType Directory -Force $Output | Out-Null
$tasks | ForEach-Object -ThrottleLimit $Jobs -Parallel {
    $task = $_
    $engine = $using:Generator
    $library = $using:Geos
    $manifest = Join-Path $task.Output 'manifest.json'
    if (!(Test-Path -LiteralPath $manifest)) {
        New-Item -ItemType Directory -Force $task.Output | Out-Null
        & $engine --geos $library --only-level $task.Level --seam-source $task.First --seam-source $task.Second --seam-keys $task.Keys --seam-report-only --output $task.Output *> (Join-Path $task.Output 'run.log')
        if ($LASTEXITCODE) { Write-Warning "Seam check failed: $($task.Id) ($LASTEXITCODE)" }
    }
}
$results = foreach ($task in $tasks) {
    $manifest = Join-Path $task.Output 'manifest.json'
    $report = if (Test-Path -LiteralPath $manifest) { Get-Content $manifest -Raw | ConvertFrom-Json } else { $null }
    [pscustomobject]@{id=$task.Id; level=$task.Level; report=$report}
}
$results | ConvertTo-Json -Depth 10 | Set-Content -Encoding utf8 (Join-Path $Output 'summary.json')
$results | Group-Object level | ForEach-Object {
    [pscustomobject]@{
        Level=$_.Name
        Total=$_.Count
        Matched=@($_.Group | Where-Object {$_.report.passed}).Count
        Failed=@($_.Group | Where-Object {!$_.report}).Count
    }
} | Format-Table
