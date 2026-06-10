#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Kill dllhost/prevhost, flush Explorer thumbnail cache, restart Explorer.

.DESCRIPTION
    Use after dev-register.ps1 or dev-unregister.ps1 to force Windows to reload
    the thumbnail provider.  Also rename/copy test files before opening Explorer
    to defeat per-file thumbcache staleness.
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Continue'   # non-fatal — some processes may not exist

Write-Host "Killing dllhost.exe (thumbnail surrogate) ..."
taskkill /f /im dllhost.exe  2>$null | Out-Null

Write-Host "Killing prevhost.exe (preview surrogate) ..."
taskkill /f /im prevhost.exe 2>$null | Out-Null

Write-Host "Stopping Explorer ..."
Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue

Start-Sleep -Milliseconds 500

Write-Host "Deleting thumbcache_*.db ..."
$thumbDir = Join-Path $env:LOCALAPPDATA 'Microsoft\Windows\Explorer'
Get-ChildItem -Path $thumbDir -Filter 'thumbcache_*.db' -ErrorAction SilentlyContinue |
    ForEach-Object {
        Remove-Item -Path $_.FullName -Force -ErrorAction SilentlyContinue
        Write-Host "  Deleted $($_.Name)"
    }

Write-Host "Restarting Explorer ..."
Start-Process explorer.exe

Write-Host "Done. Open a new Explorer window to see fresh thumbnails."
Write-Host "Tip: copy test files with new names to bypass per-item cache staleness."
