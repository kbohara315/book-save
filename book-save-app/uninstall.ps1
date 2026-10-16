param(
    [switch]$DeleteSavedList
)

$ErrorActionPreference = 'Stop'
if (-not $env:LOCALAPPDATA -or -not $env:APPDATA) {
    throw 'Windows did not provide the user data folders.'
}

$running = Get-Process -Name 'book-save', 'book-save-ui' -ErrorAction SilentlyContinue
if ($running) {
    throw 'Book Save is still running. Select Exit in the tray menu and wait for Book Save to close, then run the uninstaller again. The uninstaller will not force-close the app.'
}

$installRoot = Join-Path $env:LOCALAPPDATA 'Programs\Book Save'
$startMenuLink = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Book Save.lnk'
$startupLink = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Startup\Book Save Background.lnk'
$userDataRoot = Join-Path $env:LOCALAPPDATA 'Book Save'

if (Test-Path -LiteralPath $installRoot) {
    Remove-Item -LiteralPath $installRoot -Recurse -Force
}
if (Test-Path -LiteralPath $startMenuLink) {
    Remove-Item -LiteralPath $startMenuLink -Force
}
if (Test-Path -LiteralPath $startupLink) {
    Remove-Item -LiteralPath $startupLink -Force
}
if ($DeleteSavedList -and (Test-Path -LiteralPath $userDataRoot)) {
    Remove-Item -LiteralPath $userDataRoot -Recurse -Force
    Write-Output 'The saved list was deleted.'
} else {
    Write-Output "The saved list remains at $(Join-Path $userDataRoot 'captures.json')"
}
Write-Output 'Book Save is uninstalled.'
