param(
    [ValidateSet('run', 'build', 'release', 'check', 'fetch')]
    [string]$Command = 'run'
)

$ErrorActionPreference = 'Stop'
$projectRoot = $PSScriptRoot
$savedEnvironment = @{}
foreach ($name in @('CARGO_HOME', 'CARGO_TARGET_DIR', 'RUSTUP_HOME', 'TEMP', 'TMP', 'PATH')) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    $env:CARGO_HOME = Join-Path $projectRoot '.local\cargo'
    $env:CARGO_TARGET_DIR = Join-Path $projectRoot 'target'
    $env:TEMP = Join-Path $projectRoot '.local\tmp'
    $env:TMP = $env:TEMP
    New-Item -ItemType Directory -Path $env:TEMP -Force | Out-Null
    $isolatedRust = Join-Path $projectRoot '.local\rustup'
    if (Test-Path (Join-Path $isolatedRust 'toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe')) {
        $env:RUSTUP_HOME = $isolatedRust
    }
    if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
        if (Test-Path $vswhere) {
            $visualStudio = & $vswhere -latest -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
            $cmakeBin = Join-Path $visualStudio 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
            if (Test-Path (Join-Path $cmakeBin 'cmake.exe')) {
                $env:PATH = "$cmakeBin;$env:PATH"
            }
        }
    }
    if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
        throw 'CMake is required. Add CMake to PATH before running this script.'
    }
    Push-Location $projectRoot
    try {
        $ErrorActionPreference = 'Continue'
        $cargoArgs = if ($Command -eq 'release') { @('build', '--release', '--locked') } else { @($Command, '--locked') }
        if ($env:RUSTUP_HOME -eq $isolatedRust) {
            & rustup run stable-x86_64-pc-windows-msvc cargo @cargoArgs 2>&1 | ForEach-Object { "$_" }
        } else {
            & cargo @cargoArgs 2>&1 | ForEach-Object { "$_" }
        }
        $result = $LASTEXITCODE
    } finally {
        Pop-Location
    }
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }
}
exit $result
