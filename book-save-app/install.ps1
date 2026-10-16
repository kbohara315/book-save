$ErrorActionPreference = 'Stop'

if (-not $env:LOCALAPPDATA) {
    throw 'Windows did not provide the LOCALAPPDATA folder for this user.'
}

$projectRoot = $PSScriptRoot

function Resolve-AppExe {
    param([string]$Name, [string]$PackagedPath, [string]$ReleasePath)
    if (Test-Path -LiteralPath $PackagedPath) { return $PackagedPath }
    return $ReleasePath
}

$hostPackaged = Join-Path $projectRoot 'book-save.exe'
$uiPackaged = Join-Path $projectRoot 'book-save-ui.exe'
$hostRelease = Join-Path $projectRoot 'target\release\book-save.exe'
$uiRelease = Join-Path $projectRoot 'target\release\book-save-ui.exe'
$hostSource = Resolve-AppExe 'book-save' $hostPackaged $hostRelease
$uiSource = Resolve-AppExe 'book-save-ui' $uiPackaged $uiRelease
if (-not (Test-Path -LiteralPath $hostSource) -or -not (Test-Path -LiteralPath $uiSource)) {
    Write-Output 'Building the Book Save release app...'
    & (Join-Path $projectRoot 'run.ps1') -Command release
    if ($LASTEXITCODE -ne 0) {
        throw "The Book Save build failed with exit code $LASTEXITCODE."
    }
    $hostSource = $hostRelease
    $uiSource = $uiRelease
}
if (-not (Test-Path -LiteralPath $hostSource)) {
    throw "The Book Save host was not found or built: $hostSource"
}
if (-not (Test-Path -LiteralPath $uiSource)) {
    throw "The Book Save window app was not found or built: $uiSource"
}

$running = Get-Process -Name 'book-save', 'book-save-ui' -ErrorAction SilentlyContinue
if ($running) {
    throw 'Book Save is still running. Select Exit in the tray menu and wait for Book Save to close, then run the installer again. The installer will not force-close the app.'
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

$installedHost = Join-Path $installRoot 'book-save.exe'
$installedUi = Join-Path $installRoot 'book-save-ui.exe'
Copy-Item -LiteralPath $hostSource -Destination $installedHost -Force
Copy-Item -LiteralPath $uiSource -Destination $installedUi -Force

$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $installedHost
$shortcut.WorkingDirectory = $installRoot
$shortcut.Description = 'Book Save capture list'
$shortcut.IconLocation = "$installedHost, 0"
$shortcut.Save()

$background = $shell.CreateShortcut($startupShortcutPath)
$background.TargetPath = $installedHost
$background.Arguments = '--background'
$background.WorkingDirectory = $installRoot
$background.Description = 'Book Save background capture (starts at sign-in)'
$background.IconLocation = "$installedHost, 0"
$background.Save()

Write-Output 'Book Save is installed for this Windows user.'
Write-Output "App host: $installedHost"
Write-Output "App window: $installedUi"
Write-Output "Saved list: $userDataFile"
Write-Output "Start menu: $shortcutPath"
Write-Output "Sign-in startup: $startupShortcutPath"
