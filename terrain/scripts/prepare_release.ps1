# 共通SQLiteのRelease準備処理を地形用設定で実行する。
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Dataset,
    [string]$Package = "all",
    [Parameter(Mandatory)][string]$Database,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [string]$DataTimestamp,
    [switch]$CreateDraft,
    [switch]$ProcessDraft,
    [ValidatePattern('^[0-9a-fA-F]{40}$')][string]$TargetCommit
)
& (Join-Path $PSScriptRoot '../../surface/scripts/prepare_release.ps1') -Kind terrain @PSBoundParameters
