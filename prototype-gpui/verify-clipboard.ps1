param([switch]$ScanOnly)

$ErrorActionPreference = 'Stop'
$prototypeRoot = $PSScriptRoot
$savedEnvironment = @{}
foreach ($name in @('CARGO_HOME', 'CARGO_TARGET_DIR', 'RUSTUP_HOME')) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    $env:CARGO_HOME = Join-Path $prototypeRoot '.local\cargo'
    $env:CARGO_TARGET_DIR = Join-Path $prototypeRoot 'target'
    $isolatedRust = Join-Path $prototypeRoot '.local\rustup'
    if (Test-Path (Join-Path $isolatedRust 'toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe')) {
        $env:RUSTUP_HOME = $isolatedRust
    }
    $verificationMode = if ($ScanOnly) { 'scan' } else { 'verify' }
    $manifest = Join-Path $prototypeRoot 'verification\Cargo.toml'
    $outputPath = Join-Path $prototypeRoot '.local\clipboard-verification.txt'
    New-Item -ItemType Directory -Path (Split-Path $outputPath) -Force | Out-Null
    $ErrorActionPreference = 'Continue'
    & rustup run stable-x86_64-pc-windows-msvc cargo run --locked --offline --manifest-path $manifest -- $verificationMode 2>&1 |
        ForEach-Object { "$_" } | Tee-Object -FilePath $outputPath
    $result = $LASTEXITCODE
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }
}
exit $result
