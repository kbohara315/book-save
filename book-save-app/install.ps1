$ErrorActionPreference = 'Stop'

if (-not $env:LOCALAPPDATA) {
    throw 'Windows did not provide the LOCALAPPDATA folder for this user.'
}

$projectRoot = $PSScriptRoot
$packagedExe = Join-Path $projectRoot 'book-save.exe'
$releaseExe = Join-Path $projectRoot 'target\release\book-save.exe'
$sourceExe = if (Test-Path -LiteralPath $packagedExe) { $packagedExe } else { $releaseExe }
if (-not (Test-Path -LiteralPath $sourceExe)) {
    Write-Output 'Building the Book Save release app...'
    & (Join-Path $projectRoot 'run.ps1') -Command release
    if ($LASTEXITCODE -ne 0) {
        throw "The Book Save build failed with exit code $LASTEXITCODE."
    }
    $sourceExe = $releaseExe
}
if (-not (Test-Path -LiteralPath $sourceExe)) {
    throw "The Book Save app was not found or built: $sourceExe"
}

$running = Get-Process -Name 'book-save' -ErrorAction SilentlyContinue
if ($running) {
    throw 'Book Save is still running. Open Book Save from the Start menu (or its taskbar button) and select Quit in the app header so pending saves can finish, then run the installer again. The installer will not force-close the app.'
}

$installRoot = Join-Path $env:LOCALAPPDATA 'Programs\Book Save'
$userDataRoot = Join-Path $env:LOCALAPPDATA 'Book Save'
$userDataFile = Join-Path $userDataRoot 'captures.json'
$legacyDataFile = Join-Path $projectRoot 'data\captures.json'
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$startupDir = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Startup'
$shortcutPath = Join-Path $startMenu 'Book Save.lnk'
$startupShortcutPath = Join-Path $startupDir 'Book Save Background.lnk'

New-Item -ItemType Directory -Path $installRoot -Force | Out-Null
New-Item -ItemType Directory -Path $startMenu -Force | Out-Null
New-Item -ItemType Directory -Path $startupDir -Force | Out-Null

if (-not (Test-Path -LiteralPath $userDataFile) -and (Test-Path -LiteralPath $legacyDataFile)) {
    New-Item -ItemType Directory -Path $userDataRoot -Force | Out-Null
    Copy-Item -LiteralPath $legacyDataFile -Destination $userDataFile
    Write-Output "Copied the existing list to $userDataFile"
}

$installedExe = Join-Path $installRoot 'book-save.exe'
Copy-Item -LiteralPath $sourceExe -Destination $installedExe -Force

$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $installedExe
$shortcut.WorkingDirectory = $installRoot
$shortcut.Description = 'Book Save capture list'
$shortcut.IconLocation = "$installedExe, 0"
$shortcut.Save()

$background = $shell.CreateShortcut($startupShortcutPath)
$background.TargetPath = $installedExe
$background.Arguments = '--background'
$background.WorkingDirectory = $installRoot
$background.Description = 'Book Save background capture (starts at sign-in)'
$background.IconLocation = "$installedExe, 0"
$background.Save()

Write-Output 'Book Save is installed for this Windows user.'
Write-Output "App: $installedExe"
Write-Output "Saved list: $userDataFile"
Write-Output "Start menu: $shortcutPath"
Write-Output "Sign-in startup: $startupShortcutPath"
