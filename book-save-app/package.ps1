$ErrorActionPreference = 'Stop'
$projectRoot = $PSScriptRoot

& (Join-Path $projectRoot 'run.ps1') -Command release
if ($LASTEXITCODE -ne 0) {
    throw "The Book Save release build failed with exit code $LASTEXITCODE."
}

$exe = Join-Path $projectRoot 'target\release\book-save.exe'
if (-not (Test-Path -LiteralPath $exe)) {
    throw "The release executable was not found: $exe"
}

$stagingRoot = Join-Path $projectRoot '.local\package\book-save-windows-x64'
$distRoot = Join-Path $projectRoot 'dist'
$archive = Join-Path $distRoot 'Book-Save-Windows-x64.zip'
New-Item -ItemType Directory -Path $stagingRoot -Force | Out-Null
New-Item -ItemType Directory -Path $distRoot -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $stagingRoot 'book-save.exe') -Force
Copy-Item -LiteralPath (Join-Path $projectRoot 'install.ps1') -Destination $stagingRoot -Force
Copy-Item -LiteralPath (Join-Path $projectRoot 'uninstall.ps1') -Destination $stagingRoot -Force
Copy-Item -LiteralPath (Join-Path $projectRoot '..\docs\INSTALL_WINDOWS.md') -Destination (Join-Path $stagingRoot 'INSTALL_WINDOWS.md') -Force
if (Test-Path -LiteralPath $archive) {
    Remove-Item -LiteralPath $archive -Force
}
Compress-Archive -Path (Join-Path $stagingRoot '*') -DestinationPath $archive -CompressionLevel Optimal
Write-Output "Created $archive"
