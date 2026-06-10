#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Register thumb_viewer.dll for development/testing.

.DESCRIPTION
    Copies pdfium.dll alongside the release DLL, then calls regsvr32 to invoke
    DllRegisterServer.  Registers HKCR\SystemFileAssociations\.<ext>\ShellEx for
    .ai/.psd/.psb/.eps/.svg pointing to CLSID {90E3822E-5F70-40BF-884E-77544797AD41}.

    Run from the workspace root or pass -BuildDir explicitly.
    Requires elevation (HKCR writes need admin).

.PARAMETER BuildDir
    Directory containing thumb_viewer.dll and pdfium.dll.
    Default: <workspace root>\target\release
#>
param(
    [string]$BuildDir = (Join-Path $PSScriptRoot '..\target\release')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$BuildDir = Resolve-Path $BuildDir

$DllPath    = Join-Path $BuildDir 'thumb_viewer.dll'
$PdfiumSrc  = Join-Path $PSScriptRoot '..\third_party\bin\pdfium.dll'
$PdfiumDest = Join-Path $BuildDir 'pdfium.dll'

if (-not (Test-Path $DllPath)) {
    Write-Error "thumb_viewer.dll not found at $DllPath. Run 'cargo build --release' first."
}

# Copy pdfium.dll alongside thumb_viewer.dll so the shell extension can find it.
if (Test-Path $PdfiumSrc) {
    Write-Host "Copying pdfium.dll -> $PdfiumDest"
    Copy-Item -Path $PdfiumSrc -Destination $PdfiumDest -Force
} else {
    Write-Warning "pdfium.dll not found at $PdfiumSrc — PDF/AI thumbnails may not render."
}

Write-Host "Registering $DllPath ..."
regsvr32.exe /s $DllPath
if ($LASTEXITCODE -ne 0) {
    Write-Error "regsvr32 /s failed with exit code $LASTEXITCODE"
}

Write-Host "Registration complete."
Write-Host "Run scripts\clear-thumb-cache.ps1 to flush the thumbnail cache and test in Explorer."
