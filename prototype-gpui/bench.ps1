param(
    [int]$Runs = 5,
    [int]$IdleSeconds = 12,
    [string]$ExePath = 'target\release\cooper-gpui-prototype.exe',
    [string]$OutCsv = '.local\bench-results.csv'
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$exe = Join-Path $root $ExePath
if (-not (Test-Path $exe)) { throw "Missing exe: $exe. Build release first." }
$running = Get-Process cooper-gpui-prototype -ErrorAction SilentlyContinue
if ($running) { throw 'Close Cooper before benchmarking; the benchmark will not kill an existing app process.' }
$exeBytes = (Get-Item $exe).Length
$dataFile = Join-Path $root 'data\captures.json'
$dataBackup = Join-Path $root '.local\captures.json.bench-backup'
if (Test-Path $dataFile) { Copy-Item $dataFile $dataBackup -Force }
$beforeHash = if (Test-Path $dataFile) { (Get-FileHash $dataFile -Algorithm SHA256).Hash } else { 'no-file' }

$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$cores = $cpu.NumberOfCores
$logical = $cpu.NumberOfLogicalProcessors
Write-Output "CPU: $($cpu.Name) cores=$cores logical=$logical"
Write-Output "Exe bytes: $exeBytes"
Write-Output "Runs=$Runs idle=${IdleSeconds}s (first run = cold-ish, rest = warm)"

function Get-Median($values) {
    $s = $values | Sort-Object
    $n = $s.Count
    if ($n -eq 0) { return $null }
    if ($n % 2 -eq 1) { return $s[[int]($n/2)] }
    return ($s[$n/2 - 1] + $s[$n/2]) / 2.0
}

$rows = @()
for ($i = 1; $i -le $Runs; $i++) {
    Start-Sleep -Milliseconds 800
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process -FilePath $exe -PassThru
    $readyMs = $null
    $err = ''
    try {
        $deadline = 20
        $elapsed = 0
        while ($elapsed -lt $deadline) {
            Start-Sleep -Milliseconds 50
            $elapsed += 0.05
            $p.Refresh()
            try {
                if ($p.MainWindowHandle -ne 0) { $readyMs = $sw.Elapsed.TotalMilliseconds; break }
            } catch { }
            if ($p.HasExited) { $err = "exited-early code=$($p.ExitCode)"; break }
        }
        if ($null -eq $readyMs -and $err -eq '') { $err = 'timeout-no-window-handle' }
        Start-Sleep -Seconds 2
        $p.Refresh()
        $t0 = $p.TotalProcessorTime
        $wall0 = Get-Date
        Start-Sleep -Seconds $IdleSeconds
        $p.Refresh()
        $t1 = $p.TotalProcessorTime
        $wall1 = Get-Date
        $wall = ($wall1 - $wall0).TotalSeconds
        $cpuSec = ($t1 - $t0).TotalSeconds
        $oneCorePct = if ($wall -gt 0) { $cpuSec / $wall * 100.0 } else { 0 }
        $wsMiB = $p.WorkingSet64 / 1MB
        $privMiB = $p.PrivateMemorySize64 / 1MB
    } catch {
        $err = $_.Exception.Message
        $oneCorePct = $null; $wsMiB = $null; $privMiB = $null
    }
    try {
        if (-not $p.HasExited) {
            $p.CloseMainWindow() | Out-Null
            Start-Sleep -Milliseconds 1500
            if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
        }
    } catch { }
    $cache = if ($i -eq 1) { 'cold-ish' } else { 'warm' }
    $row = [pscustomobject]@{
        run = $i; cache = $cache; ready_ms = $readyMs; idle_one_core_pct = $oneCorePct
        workingset_mib = $wsMiB; private_mib = $privMiB; exe_bytes = $exeBytes; error = $err
    }
    $rows += $row
    Write-Output ("run {0} [{1}] ready={2}ms cpu={3}% ws={4}MiB priv={5}MiB err={6}" -f $i, $cache,
        ([math]::Round($readyMs,1)), ([math]::Round($oneCorePct,2)),
        ([math]::Round($wsMiB,1)), ([math]::Round($privMiB,1)), $err)
    Start-Sleep -Milliseconds 1000
}

New-Item -ItemType Directory -Path (Split-Path (Join-Path $root $OutCsv)) -Force | Out-Null
$rows | Export-Csv -Path (Join-Path $root $OutCsv) -NoTypeInformation
Write-Output "CSV: $OutCsv"

$afterHash = if (Test-Path $dataFile) { (Get-FileHash $dataFile -Algorithm SHA256).Hash } else { 'no-file' }
Write-Output "data captures.json before=$beforeHash after=$afterHash preserved=$($beforeHash -eq $afterHash)"

function Summ($vals) {
    $v = $vals | Where-Object { $_ -ne $null }
    if ($v.Count -eq 0) { return 'n/a' }
    $med = Get-Median $v
    $mn = ($v | Measure-Object -Minimum).Minimum
    $mx = ($v | Measure-Object -Maximum).Maximum
    return ("median={0} range={1}..{2} n={3}" -f [math]::Round($med,2), [math]::Round($mn,2), [math]::Round($mx,2), $v.Count)
}
Write-Output ("ready_ms: " + (Summ ($rows.ready_ms)))
Write-Output ("idle_one_core_pct: " + (Summ ($rows.idle_one_core_pct)))
Write-Output ("workingset_mib: " + (Summ ($rows.workingset_mib)))
Write-Output ("private_mib: " + (Summ ($rows.private_mib)))
Write-Output ("exe_bytes: $exeBytes")
