param([switch]$ScanOnly)

$ErrorActionPreference = 'Stop'
$projectRoot = $PSScriptRoot
$savedEnvironment = @{}
foreach ($name in @('CARGO_HOME', 'CARGO_TARGET_DIR', 'RUSTUP_HOME')) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    $env:CARGO_HOME = Join-Path $projectRoot '.local\cargo'
    $env:CARGO_TARGET_DIR = Join-Path $projectRoot 'target'
    $isolatedRust = Join-Path $projectRoot '.local\rustup'
    $isolatedCargo = Join-Path $isolatedRust 'toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe'
    if (Test-Path -LiteralPath $isolatedCargo) {
        $env:RUSTUP_HOME = $isolatedRust
        $env:PATH = (Split-Path $isolatedCargo) + ';' + $env:PATH
    }
    $verificationMode = if ($ScanOnly) { 'scan' } else { 'verify' }
    $manifest = Join-Path $projectRoot 'verification\Cargo.toml'
    $outputPath = Join-Path $projectRoot '.local\clipboard-verification.txt'
    New-Item -ItemType Directory -Path (Split-Path $outputPath) -Force | Out-Null
    $ErrorActionPreference = 'Continue'
    if (Test-Path -LiteralPath $isolatedCargo) {
        & $isolatedCargo run --locked --offline --manifest-path $manifest -- $verificationMode 2>&1 |
            ForEach-Object { "$_" } | Tee-Object -FilePath $outputPath
    } else {
        & cargo run --locked --offline --manifest-path $manifest -- $verificationMode 2>&1 |
            ForEach-Object { "$_" } | Tee-Object -FilePath $outputPath
    }
    $result = $LASTEXITCODE
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }
}
exit $result
