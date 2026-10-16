param(
    [int]$Runs = 5,
    [int]$IdleSeconds = 12,
    [string]$ExePath = 'target\release\book-save.exe',
    [string]$OutCsv = '.local\bench-results.csv'
)

# Measures the resident host only: the tray + capture process with no window.
# It launches the host with --background under an isolated instance token and
# user profile, proves the GPUI window process never starts, and samples CPU
# seconds and memory for the whole host process. Nothing here touches the
# installed app or the real saved list.

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$exe = Join-Path $root $ExePath
if (-not (Test-Path $exe)) { throw "Missing exe: $exe. Build release first." }
$uiExe = Join-Path $root 'target\release\book-save-ui.exe'

$exeBytes = (Get-Item $exe).Length
$profile = Join-Path $root ('.local\bench-profile-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $profile -Force | Out-Null
$instance = 'bench-' + [guid]::NewGuid().ToString('N')

$savedLocal = $env:LOCALAPPDATA
$savedInstance = $env:BOOK_SAVE_TEST_INSTANCE
$env:LOCALAPPDATA = $profile
$env:BOOK_SAVE_TEST_INSTANCE = $instance

$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$cores = $cpu.NumberOfCores
$logical = $cpu.NumberOfLogicalProcessors
Write-Output "CPU: $($cpu.Name) cores=$cores logical=$logical"
Write-Output "Exe bytes: $exeBytes"
Write-Output "Mode: --background (tray + capture resident host, no window process)"
Write-Output "Runs=$Runs idle=${IdleSeconds}s (first run = cold-ish, rest = warm)"

function Get-Median($values) {
    $s = $values | Sort-Object
    $n = $s.Count
    if ($n -eq 0) { return $null }
    if ($n % 2 -eq 1) { return $s[[int]($n/2)] }
    return ($s[$n/2 - 1] + $s[$n/2]) / 2.0
}

function Get-OwnedUiProcesses([int]$HostPid) {
    $rows = Get-CimInstance Win32_Process -Filter "Name = 'book-save-ui.exe'" -ErrorAction SilentlyContinue |
        Where-Object { [int]$_.ParentProcessId -eq $HostPid -and $_.ExecutablePath -eq $uiExe }
    foreach ($row in $rows) {
        try { Get-Process -Id ([int]$row.ProcessId) -ErrorAction Stop } catch { }
    }
}

function Get-OwnedProcesses([Diagnostics.Process]$HostProcess) {
    $hostProc = $null
    try { $HostProcess.Refresh(); if (-not $HostProcess.HasExited) { $hostProc = $HostProcess } } catch { }
    @($hostProc) + @(Get-OwnedUiProcesses -HostPid $HostProcess.Id) | Where-Object { $_ -ne $null }
}

function Get-CpuTicks($processes) {
    $ticks = [long]0
    foreach ($process in $processes) { $process.Refresh(); $ticks += $process.TotalProcessorTime.Ticks }
    return $ticks
}

$rows = @()
try {
    for ($i = 1; $i -le $Runs; $i++) {
        Start-Sleep -Milliseconds 800
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $p = Start-Process -FilePath $exe -ArgumentList '--background' -PassThru
        $readyMs = $null
        $err = ''
        $uiSeen = $false
        try {
            $deadline = 20
            $elapsed = 0
            while ($elapsed -lt $deadline) {
                Start-Sleep -Milliseconds 50
                $elapsed += 0.05
                if (@(Get-OwnedUiProcesses -HostPid $p.Id).Count -gt 0) { $uiSeen = $true; $err = 'ui-process-started'; break }
                if ($p.HasExited) { $err = "exited-early code=$($p.ExitCode)"; break }
                if ($elapsed -ge 3) { $readyMs = $sw.Elapsed.TotalMilliseconds; break }
            }
            if ($null -eq $readyMs -and $err -eq '') { $err = 'timeout' }
            Start-Sleep -Seconds 2
            $ownedStart = @(Get-OwnedProcesses -HostProcess $p)
            $t0 = Get-CpuTicks $ownedStart
            $watch = [Diagnostics.Stopwatch]::StartNew()
            Start-Sleep -Seconds $IdleSeconds
            $watch.Stop()
            $ownedEnd = @(Get-OwnedProcesses -HostProcess $p)
            $t1 = Get-CpuTicks $ownedEnd
            if (@(Get-OwnedUiProcesses -HostPid $p.Id).Count -gt 0) { $uiSeen = $true; $err = 'ui-process-started' }
            $wall = $watch.Elapsed.TotalSeconds
            $cpuSec = ($t1 - $t0) / [TimeSpan]::TicksPerSecond
            $oneCorePct = if ($wall -gt 0) { $cpuSec / $wall * 100.0 } else { 0 }
            $wsMiB = (($ownedEnd | Measure-Object -Property WorkingSet64 -Sum).Sum) / 1MB
            $privMiB = (($ownedEnd | Measure-Object -Property PrivateMemorySize64 -Sum).Sum) / 1MB
        } catch {
            $err = $_.Exception.Message
            $oneCorePct = $null; $wsMiB = $null; $privMiB = $null
        }
        try {
            if (-not $p.HasExited) {
                Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
                $p.WaitForExit(5000) | Out-Null
            }
        } catch { }
        if ($uiSeen) { throw 'The window process started during a background run; expected no GPUI process.' }
        $cache = if ($i -eq 1) { 'cold-ish' } else { 'warm' }
        $row = [pscustomobject]@{
            run = $i; cache = $cache; settle_ms = $readyMs; idle_one_core_pct = $oneCorePct
            cpu_seconds = $cpuSec; workingset_mib = $wsMiB; private_mib = $privMiB
            ui_process = $uiSeen; exe_bytes = $exeBytes; error = $err
        }
        $rows += $row
        Write-Output ("run {0} [{1}] settle={2}ms cpu_s={3} cpu={4}% ws={5}MiB priv={6}MiB ui={7} err={8}" -f $i, $cache,
            ([math]::Round($readyMs,1)), ([math]::Round($cpuSec,3)), ([math]::Round($oneCorePct,2)),
            ([math]::Round($wsMiB,1)), ([math]::Round($privMiB,1)), $uiSeen, $err)
        Start-Sleep -Milliseconds 1000
    }
} finally {
    if ($p) {
        foreach ($child in @(Get-OwnedUiProcesses -HostPid $p.Id)) {
            Stop-Process -Id $child.Id -Force -ErrorAction SilentlyContinue
        }
        if (-not $p.HasExited) {
            Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
            $p.WaitForExit(5000) | Out-Null
        }
    }
    $env:LOCALAPPDATA = $savedLocal
    $env:BOOK_SAVE_TEST_INSTANCE = $savedInstance
    if (Test-Path -LiteralPath $profile) { Remove-Item -LiteralPath $profile -Recurse -Force -ErrorAction SilentlyContinue }
}

New-Item -ItemType Directory -Path (Split-Path (Join-Path $root $OutCsv)) -Force | Out-Null
$rows | Export-Csv -Path (Join-Path $root $OutCsv) -NoTypeInformation
Write-Output "CSV: $OutCsv"

function Summ($vals) {
    $v = $vals | Where-Object { $_ -ne $null }
    if ($v.Count -eq 0) { return 'n/a' }
    $med = Get-Median $v
    $mn = ($v | Measure-Object -Minimum).Minimum
    $mx = ($v | Measure-Object -Maximum).Maximum
    return ("median={0} range={1}..{2} n={3}" -f [math]::Round($med,2), [math]::Round($mn,2), [math]::Round($mx,2), $v.Count)
}
Write-Output ("settle_ms: " + (Summ ($rows.settle_ms)))
Write-Output ("idle_one_core_pct: " + (Summ ($rows.idle_one_core_pct)))
Write-Output ("cpu_seconds_idle_${IdleSeconds}s: " + (Summ ($rows.cpu_seconds)))
Write-Output ("workingset_mib: " + (Summ ($rows.workingset_mib)))
Write-Output ("private_mib: " + (Summ ($rows.private_mib)))
Write-Output ("exe_bytes: $exeBytes")
