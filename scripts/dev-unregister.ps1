#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Unregister thumb_viewer.dll (removes all Shell registry keys written by DllRegisterServer).

.PARAMETER BuildDir
    Directory containing thumb_viewer.dll.
    Default: <workspace root>\target\release
#>
param(
    [string]$BuildDir = (Join-Path $PSScriptRoot '..\target\release')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$BuildDir = Resolve-Path $BuildDir
$DllPath  = Join-Path $BuildDir 'thumb_viewer.dll'

if (-not (Test-Path $DllPath)) {
    Write-Error "thumb_viewer.dll not found at $DllPath"
}

Write-Host "Unregistering $DllPath ..."
regsvr32.exe /s /u $DllPath
if ($LASTEXITCODE -ne 0) {
    Write-Error "regsvr32 /s /u failed with exit code $LASTEXITCODE"
}

Write-Host "Unregistration complete."
Write-Host "Run scripts\clear-thumb-cache.ps1 to flush the thumbnail cache."
