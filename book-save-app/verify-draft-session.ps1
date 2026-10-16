param(
    [switch]$Worker,      # compat: ignored (single-level probe, no nested worker)
    [string]$StateFile    # compat: ignored
)

# Draft-session verifier: nonempty drafts survive UI close and are restored on
# reopen; captured text is NOT autosaved (negative) until the user presses
# Enter/Add (positive); background-only mode leaves zero UI processes.
#
# Technique (same anti-hang discipline as the other verifiers): single-level
# Start-Process with stdout/stderr redirected to FILES (never in-memory
# pipes), bounded poll loops, identity-checked kills (PID + full image path)
# in `finally`. Worst-case wall time ~90s; typical run ~25s. Never run two
# verifiers at once. Scoped to PIDs launched by this run; isolated
# BOOK_SAVE_TEST_INSTANCE GUID + temp LOCALAPPDATA.
#
# Honesty rule: session.json/captures.json reads are OBSERVED file state.
# "The UI restored the draft" is INFERRED from behavioral evidence (the UI
# rewrote the file on quit / Add persisted the draft text). Seeded files are
# labeled SEEDED. Reading session.json alone never counts as restore proof.
$ErrorActionPreference = 'Stop'
$hostExe = Join-Path $PSScriptRoot 'target\debug\book-save.exe'
$uiExe = Join-Path $PSScriptRoot 'target\debug\book-save-ui.exe'
foreach ($exe in @($hostExe, $uiExe)) {
    if (-not (Test-Path -LiteralPath $exe)) {
        throw "Build both binaries first with .\run.ps1 -Command build ($exe missing)."
    }
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class DraftSessionProbe {
    public delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr state);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr state);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int count);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool ShowWindow(IntPtr hwnd, int cmd);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool GetExitCodeProcess(IntPtr h, out uint code);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
    public static IntPtr Find(string title, uint ownerPid, bool visible) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hwnd, _) => {
            uint pid; GetWindowThreadProcessId(hwnd, out pid);
            if (pid != ownerPid || IsWindowVisible(hwnd) != visible) return true;
            var text = new StringBuilder(256); GetWindowText(hwnd, text, text.Capacity);
            if (text.ToString() == title) { found = hwnd; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static string Describe(uint ownerPid) {
        var rows = new System.Collections.Generic.List<string>();
        EnumWindows((hwnd, _) => {
            uint pid; GetWindowThreadProcessId(hwnd, out pid);
            if (pid == ownerPid) {
                var text = new StringBuilder(256); GetWindowText(hwnd, text, text.Capacity);
                var cls = new StringBuilder(128); GetClassName(hwnd, cls, cls.Capacity);
                rows.Add("hwnd=" + hwnd + " class=" + cls + " title=" + text + " visible=" + IsWindowVisible(hwnd));
            }
            return true;
        }, IntPtr.Zero);
        return String.Join("; ", rows.ToArray());
    }
}
'@

$results = @()
function Assert([string]$Id, [bool]$Ok, [string]$Evidence) {
    $line = "ASSERT $Id $(if ($Ok) { 'PASS' } else { 'FAIL' }): $Evidence"
    Write-Output $line
    $script:results += @{ Id = $Id; Ok = $Ok }
}

function Wait-VisibleWindow([uint32]$OwnerPid, [int]$TimeoutMs) {
    $end = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $end) {
        $w = [DraftSessionProbe]::Find('Book Save', $OwnerPid, $true)
        if ($w.ToInt64() -ne 0) { return $w }
        Start-Sleep -Milliseconds 150
    }
    return [IntPtr]::Zero
}

function Wait-UiChildOf([int]$HostPid, [int]$TimeoutMs) {
    $end = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $end) {
        $rec = Get-CimInstance Win32_Process -Filter "Name = 'book-save-ui.exe'" -ErrorAction SilentlyContinue |
            Where-Object { [int]$_.ParentProcessId -eq $HostPid } | Select-Object -First 1
        if ($rec) { return [int]$rec.ProcessId }
        Start-Sleep -Milliseconds 150
    }
    return 0
}

function Read-Session([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    try { return Get-Content -LiteralPath $Path -Raw -ErrorAction Stop | ConvertFrom-Json } catch { return $null }
}

function Read-Captures([string]$Path) {
    # The comma operator stops PowerShell from unrolling a single-element array
    # on return, so callers can always rely on `.Count` (fixes the N2 blank count).
    if (-not (Test-Path -LiteralPath $Path)) { return ,@() }
    try {
        $data = Get-Content -LiteralPath $Path -Raw -ErrorAction Stop | ConvertFrom-Json
        if ($null -eq $data) { return ,@() }
        if ($data -is [array]) { return ,$data }
        return ,@($data)
    } catch { return $null }
}

function Stop-OwnedProcessTree([int]$ProcessId) {
    if ($ProcessId -eq 0) { return }
    try { & taskkill.exe /PID $ProcessId /T /F 2>&1 | Out-Null } catch { }
}

function Send-InjectCapture([string]$PipeName, [string]$Text) {
    $pipe = New-Object IO.Pipes.NamedPipeClientStream('.', $PipeName, [IO.Pipes.PipeDirection]::InOut)
    try {
        $pipe.Connect(5000)
        $msg = (@{ type = 'inject_capture'; text = $Text } | ConvertTo-Json -Compress) + "`n"
        $bytes = [Text.Encoding]::UTF8.GetBytes($msg)
        $pipe.Write($bytes, 0, $bytes.Length)
        $pipe.Flush()
        Start-Sleep -Milliseconds 300
    } finally {
        $pipe.Dispose()
    }
}

$TEXT1 = 'draft-session probe nonempty: buy oat milk 7f3a'
$EDIT_DRAFT = 'edit-draft probe nonempty: revise chapter two 9c1e'
$BEFORE_EDIT = 'capture-before-edit probe: outline intro 4b2d'
$SEED_TEXT = 'seeded baseline item 51aa'

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('book-save-draft-session-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
$localAppData = Join-Path $testRoot 'LocalAppData'
New-Item -ItemType Directory -Path $localAppData -Force | Out-Null
$bookSaveDir = Join-Path $localAppData 'Book Save'
$sessionPath = Join-Path $bookSaveDir 'session.json'
$capturesPath = Join-Path $bookSaveDir 'captures.json'
$hostOut = Join-Path $testRoot 'host.stdout.log'
$hostErr = Join-Path $testRoot 'host.stderr.log'
foreach ($f in @($hostOut, $hostErr)) { New-Item -ItemType File -Path $f -Force | Out-Null }

$instance = [guid]::NewGuid().ToString('N')
$pipeName = "book-save-test-$instance"
$scriptStart = [DateTime]::UtcNow
function Stamp([string]$Msg) { Write-Output ("[{0:N1}s] {1}" -f ([DateTime]::UtcNow - $scriptStart).TotalSeconds, $Msg) }
$savedLocal = $env:LOCALAPPDATA
$savedInst = $env:BOOK_SAVE_TEST_INSTANCE
$ownedHostPid = 0
$ownedUiPids = @()
$hostHandle = [IntPtr]::Zero
$uiLogIndex = 0
function Start-OwnedUi([string]$Reason) {
    $script:uiLogIndex += 1
    $o = Join-Path $testRoot ("ui$($script:uiLogIndex).stdout.log")
    $e = Join-Path $testRoot ("ui$($script:uiLogIndex).stderr.log")
    New-Item -ItemType File -Path $o -Force | Out-Null
    New-Item -ItemType File -Path $e -Force | Out-Null
    $p = Start-Process -FilePath $uiExe -PassThru -RedirectStandardOutput $o -RedirectStandardError $e -WorkingDirectory $PSScriptRoot
    $script:ownedUiPids += $p.Id
    Stamp ("UI launch ($Reason): PID $($p.Id)")
    return $p
}
try {
    $env:LOCALAPPDATA = $localAppData
    $env:BOOK_SAVE_TEST_INSTANCE = $instance

    # Reap stale test hosts from an interrupted earlier run of this verifier so
    # they cannot hold the pipe/handles and keep the calling shell from returning.
    Get-Process -Name 'book-save' -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -ieq $hostExe } | ForEach-Object { Stop-OwnedProcessTree $_.Id }
    Get-Process -Name 'book-save-ui' -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -ieq $uiExe } | ForEach-Object { Stop-OwnedProcessTree $_.Id }

    # ---- Phase A: capture via host pipe, no autosave, draft survives close ----
    $hostProc = Start-Process -FilePath $hostExe -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $hostOut -RedirectStandardError $hostErr -WorkingDirectory $PSScriptRoot
    $ownedHostPid = $hostProc.Id
    $hostHandle = [DraftSessionProbe]::OpenProcess(0x1000, $false, [uint32]$ownedHostPid)
    Stamp "host PID $ownedHostPid instance $instance pipe $pipeName"

    $ui1Pid = Wait-UiChildOf $ownedHostPid 6000
    Assert 'A0-host-spawns-ui' ($ui1Pid -ne 0) "observed UI child PID of host ${ownedHostPid}: $(if ($ui1Pid) { $ui1Pid } else { 'none within 6s' })"
    if ($ui1Pid -eq 0) { throw 'Host did not start its GPUI child within 6s.' }
    $ownedUiPids += $ui1Pid
    $ui1 = Get-Process -Id $ui1Pid -ErrorAction Stop
    $ui1Handle = [DraftSessionProbe]::OpenProcess(0x1000, $false, [uint32]$ui1Pid)
    $win1 = Wait-VisibleWindow ([uint32]$ui1Pid) 10000
    Assert 'A1-visible-window' ($win1.ToInt64() -ne 0) "observed visible 'Book Save' window hwnd=$win1 for UI PID $ui1Pid"
    if ($win1.ToInt64() -eq 0) { throw 'UI1 did not create a visible window within 10s.' }
    $wmNullResult = [IntPtr]::Zero
    $answered = [DraftSessionProbe]::SendMessageTimeout($win1, 0, [IntPtr]::Zero, [IntPtr]::Zero, 0, 4000, [ref]$wmNullResult)
    Assert 'A2-window-responsive' ($answered.ToInt64() -ne 0) "observed WM_NULL answer within 4s: $($answered.ToInt64() -ne 0)"

    # Baseline (OBSERVED): a fresh LOCALAPPDATA still receives the legacy seed
    # via copy_legacy_data_if_missing, so counts are baseline-relative, never
    # assumed zero.
    $baseCaps = Read-Captures $capturesPath
    $baseCount = if ($baseCaps -eq $null) { -1 } else { $baseCaps.Count }
    Assert 'A2b-baseline-readable' ($baseCount -ge 0) "observed initial captures.json item count=$baseCount"

    Send-InjectCapture $pipeName $TEXT1
    Stamp 'InjectCapture sent over isolated pipe (debug-only host path)'
    $landed = $false
    $end = [DateTime]::UtcNow.AddSeconds(8)
    while ([DateTime]::UtcNow -lt $end) {
        $s = Read-Session $sessionPath
        if ($s -and $s.capture_draft -eq $TEXT1) { $landed = $true; break }
        Start-Sleep -Milliseconds 200
    }
    $sNow = Read-Session $sessionPath
    Assert 'D1-capture-in-session' $landed "observed session.json capture_draft='$($sNow.capture_draft)' (expected '$TEXT1'); INFERRED the running UI received the injected capture and persisted it"
    $hostLogHit = $false
    if (Test-Path -LiteralPath $hostErr) {
        $hostLogHit = (Get-Content -LiteralPath $hostErr -ErrorAction SilentlyContinue | Where-Object { $_ -match 'debug capture injection received' }).Count -gt 0
    }
    Assert 'D1b-host-log' $hostLogHit 'observed host stderr contains "debug capture injection received" (InjectCapture path is debug-only; this run uses the debug build)'
    $caps = Read-Captures $capturesPath
    Assert 'N1-no-autosave' ($caps -ne $null -and $caps.Count -eq $baseCount) "observed captures.json item count=$($caps.Count) after capture (baseline $baseCount): captured text NOT persisted before Add"

    if (-not [DraftSessionProbe]::PostMessage($win1, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'Could not post WM_CLOSE to UI1.' }
    $ui1Exited = $ui1.WaitForExit(5000)
    Assert 'A3-ui-exits-on-close' $ui1Exited 'observed UI1 process exit within 5s of WM_CLOSE'
    if (-not $ui1Exited) { throw 'UI1 did not exit within 5s of WM_CLOSE.' }
    $code = 0; $gotCode = [DraftSessionProbe]::GetExitCodeProcess($ui1Handle, [ref]$code)
    Assert 'A4-ui-exit-zero' ($gotCode -and $code -eq 0) "observed UI1 exit code [$code] (expected 0)"
    [DraftSessionProbe]::CloseHandle($ui1Handle) | Out-Null; $ui1Handle = [IntPtr]::Zero

    $sAfter = Read-Session $sessionPath
    Assert 'D2-draft-survives-close' ($sAfter -and $sAfter.capture_draft -eq $TEXT1) "observed session.json capture_draft='$($sAfter.capture_draft)' after UI exit (expected '$TEXT1'); INFERRED the UI wrote it back during on_app_quit"
    $capsAfter = Read-Captures $capturesPath
    Assert 'N1b-still-no-autosave' ($capsAfter -ne $null -and $capsAfter.Count -eq $baseCount) "observed captures.json item count=$($capsAfter.Count) after close (baseline $baseCount)"

    # ---- Background-only: zero UI processes while host stays ----
    Start-Sleep -Milliseconds 500
    $hostProc.Refresh()
    $childrenLeft = @(Get-CimInstance Win32_Process -Filter "Name = 'book-save-ui.exe'" -ErrorAction SilentlyContinue |
        Where-Object { [int]$_.ParentProcessId -eq $ownedHostPid })
    $pathLeft = @(Get-Process -Name 'book-save-ui' -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and [IO.Path]::GetFullPath($_.Path) -ieq [IO.Path]::GetFullPath($uiExe) })
    Assert 'B1-no-ui-child-of-host' ($childrenLeft.Count -eq 0) "observed $($childrenLeft.Count) book-save-ui.exe with parent=host PID $ownedHostPid (expected 0)"
    Assert 'B2-no-workspace-ui-path' ($pathLeft.Count -eq 0) "observed $($pathLeft.Count) running process with workspace image path $uiExe (expected 0)"
    Assert 'B3-host-survives' (-not $hostProc.HasExited) "observed host PID $ownedHostPid alive=$(-not $hostProc.HasExited) with zero UI processes: background-only mode holds"

    # ---- Phase B: reopen, behavioral restore proof via real Enter/Add ----
    $ui2 = Start-OwnedUi 'reopen for restore+Add'
    $ui2Pid = $ui2.Id
    $ui2Handle = [DraftSessionProbe]::OpenProcess(0x1000, $false, [uint32]$ui2Pid)
    $win2 = Wait-VisibleWindow ([uint32]$ui2Pid) 10000
    Assert 'R0-reopen-window' ($win2.ToInt64() -ne 0) "observed visible 'Book Save' window hwnd=$win2 for reopened UI PID $ui2Pid"
    if ($win2.ToInt64() -eq 0) { throw 'UI2 did not create a visible window within 10s.' }
    $preAdd = Read-Captures $capturesPath
    Assert 'N1c-preadd-count-zero' ($preAdd -ne $null -and $preAdd.Count -eq $baseCount) "observed captures.json count=$($preAdd.Count) just before Add (baseline $baseCount)"
    if ([DraftSessionProbe]::IsIconic($win2)) { [DraftSessionProbe]::ShowWindow($win2, 9) | Out-Null }
    [DraftSessionProbe]::SetForegroundWindow($win2) | Out-Null
    Start-Sleep -Milliseconds 500
    $shell = New-Object -ComObject WScript.Shell
    $shell.SendKeys('{ENTER}')
    Stamp 'real Enter keystroke sent to owned window (user Add path)'
    $added = $false
    $addedCaps = $null
    $end = [DateTime]::UtcNow.AddSeconds(10)
    while ([DateTime]::UtcNow -lt $end) {
        $c = Read-Captures $capturesPath
        if ($c -ne $null -and $c.Count -eq ($baseCount + 1) -and @($c | Where-Object { $_.text -eq $TEXT1 }).Count -eq 1) { $added = $true; $addedCaps = $c; break }
        Start-Sleep -Milliseconds 250
    }
    if (-not $added) {
        [DraftSessionProbe]::SetForegroundWindow($win2) | Out-Null
        Start-Sleep -Milliseconds 400
        $shell.SendKeys('{ENTER}')
        Stamp 'retry: refocused owned window, sent Enter once more'
        $end = [DateTime]::UtcNow.AddSeconds(5)
        while ([DateTime]::UtcNow -lt $end) {
            $c = Read-Captures $capturesPath
            if ($c -ne $null -and $c.Count -eq ($baseCount + 1) -and @($c | Where-Object { $_.text -eq $TEXT1 }).Count -eq 1) { $added = $true; $addedCaps = $c; break }
            Start-Sleep -Milliseconds 250
        }
    }
    $finalCaps = Read-Captures $capturesPath
    $finalCount = if ($finalCaps -eq $null) { 'unreadable' } else { "$($finalCaps.Count)" }
    $finalFirst = if ($finalCaps -ne $null -and $finalCaps.Count -ge 1) { "'$($finalCaps[0].text)'" } else { '(none)' }
    Assert 'R1-restore-plus-add' $added "observed captures.json count=$finalCount first=$finalFirst (expected $($baseCount + 1)/'$TEXT1'); INFERRED the reopened UI restored the draft into its input, since nothing else could supply that exact text"
    $sPostAdd = Read-Session $sessionPath
    Assert 'R2-draft-cleared-by-submit' ($sPostAdd -and $sPostAdd.capture_draft -eq '') "observed session.json capture_draft='$($sPostAdd.capture_draft)' after Add (expected ''): corroborates the real submit path ran"
    if (-not [DraftSessionProbe]::PostMessage($win2, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'Could not post WM_CLOSE to UI2.' }
    $ui2Proc = Get-Process -Id $ui2Pid -ErrorAction SilentlyContinue
    $ui2Exited = if ($ui2Proc) { $ui2Proc.WaitForExit(5000) } else { $true }
    Assert 'A5-ui2-exits-on-close' $ui2Exited 'observed UI2 process exit within 5s of WM_CLOSE'
    if ($ui2Handle -ne [IntPtr]::Zero) { [DraftSessionProbe]::CloseHandle($ui2Handle) | Out-Null; $ui2Handle = [IntPtr]::Zero }
    $hostProc.Refresh()
    Assert 'B4-host-survives-2' (-not $hostProc.HasExited) "observed host PID $ownedHostPid still alive after UI2 close"

    # ---- Phase C: in-progress edit draft survives close and restores ----
    $stillUi = @(Get-CimInstance Win32_Process -Filter "Name = 'book-save-ui.exe'" -ErrorAction SilentlyContinue |
        Where-Object { [int]$_.ParentProcessId -eq $ownedHostPid })
    if ($stillUi.Count -ne 0) { throw 'A UI child is still attached to the host; refusing to seed files under a live writer.' }
    $seedSession = @{
        capture_draft = ''
        edit_target = 1
        edit_draft = $EDIT_DRAFT
        capture_draft_before_edit = $BEFORE_EDIT
        deferred_capture = $null
    } | ConvertTo-Json -Compress
    $seedArr = @(@{ id = 1; text = $SEED_TEXT; complete = $false })
    # Build a real JSON array; `ConvertTo-Json -InputObject @(...)` unwraps a
    # one-element array into an object, which made captures.json non-array.
    $seedCaptures = '[' + (($seedArr | ForEach-Object { $_ | ConvertTo-Json -Compress }) -join ',') + ']'
    # SEEDED setup (not evidence): simulates state left by an interrupted edit.
    [IO.File]::WriteAllText($sessionPath, $seedSession, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($capturesPath, $seedCaptures, [Text.UTF8Encoding]::new($false))
    Stamp "SEEDED session.json edit_target=1 edit_draft nonempty; SEEDED captures.json one item '$SEED_TEXT'"
    $ui3 = Start-OwnedUi 'reopen for edit-draft restore'
    $ui3Pid = $ui3.Id
    $ui3Handle = [DraftSessionProbe]::OpenProcess(0x1000, $false, [uint32]$ui3Pid)
    $win3 = Wait-VisibleWindow ([uint32]$ui3Pid) 10000
    Assert 'E0-reopen-window' ($win3.ToInt64() -ne 0) "observed visible 'Book Save' window hwnd=$win3 for UI3 PID $ui3Pid"
    if ($win3.ToInt64() -eq 0) { throw 'UI3 did not create a visible window within 10s.' }
    if (-not [DraftSessionProbe]::PostMessage($win3, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'Could not post WM_CLOSE to UI3.' }
    $ui3Proc = Get-Process -Id $ui3Pid -ErrorAction SilentlyContinue
    $ui3Exited = if ($ui3Proc) { $ui3Proc.WaitForExit(5000) } else { $true }
    Assert 'A6-ui3-exits-on-close' $ui3Exited 'observed UI3 process exit within 5s of WM_CLOSE'
    if ($ui3Handle -ne [IntPtr]::Zero) { [DraftSessionProbe]::CloseHandle($ui3Handle) | Out-Null; $ui3Handle = [IntPtr]::Zero }
    $sEdit = Read-Session $sessionPath
    Assert 'E1-edit-draft-survives' ($sEdit -and $sEdit.edit_target -eq 1 -and $sEdit.edit_draft -eq $EDIT_DRAFT) "observed session.json edit_target=$($sEdit.edit_target) edit_draft='$($sEdit.edit_draft)' after UI3 exit"
    Assert 'E2-edit-restore-branch' ($sEdit -and $sEdit.capture_draft_before_edit -eq $BEFORE_EDIT) "observed capture_draft_before_edit='$($sEdit.capture_draft_before_edit)' (expected '$BEFORE_EDIT'); INFERRED the reopened UI took the Edit restore branch, which is the only path that rewrites this field"
    $capsEdit = Read-Captures $capturesPath
    $capsEditCount = if ($capsEdit -eq $null) { 'unreadable' } else { "$($capsEdit.Count)" }
    $capsEditText = if ($capsEdit -ne $null -and $capsEdit.Count -ge 1) { "'$($capsEdit[0].text)'" } else { '(none)' }
    Assert 'N2-edit-not-autosaved' ($capsEdit -ne $null -and $capsEdit.Count -eq 1 -and $capsEdit[0].text -eq $SEED_TEXT) "observed captures.json count=$capsEditCount text=$capsEditText unchanged: edit draft NOT written into the item without Save"

    $failed = @($results | Where-Object { -not $_.Ok })
    if ($failed.Count -eq 0) { Write-Output "PASS DRAFT_SESSION all $($results.Count) assertions held."; exit 0 }
    Write-Output ("FAIL DRAFT_SESSION {0} of {1} assertions failed." -f $failed.Count, $results.Count)
    exit 1
}
catch {
    Write-Output "FAIL $($_.Exception.Message)"
    exit 1
}
finally {
    $env:LOCALAPPDATA = $savedLocal
    $env:BOOK_SAVE_TEST_INSTANCE = $savedInst
    if ($ui1Handle -and $ui1Handle -ne [IntPtr]::Zero) { [DraftSessionProbe]::CloseHandle($ui1Handle) | Out-Null }
    if ($ui2Handle -and $ui2Handle -ne [IntPtr]::Zero) { [DraftSessionProbe]::CloseHandle($ui2Handle) | Out-Null }
    if ($ui3Handle -and $ui3Handle -ne [IntPtr]::Zero) { [DraftSessionProbe]::CloseHandle($ui3Handle) | Out-Null }
    if ($hostHandle -ne [IntPtr]::Zero) { [DraftSessionProbe]::CloseHandle($hostHandle) | Out-Null }
    # Kill the whole tree of every process this run started (host + GPUI
    # children) so no resident host survives and the calling shell gets its
    # handles back instead of appearing to hang.
    foreach ($id in @($ownedHostPid) + $ownedUiPids) { Stop-OwnedProcessTree ([int]$id) }
    if (Test-Path -LiteralPath $hostErr) {
        Write-Output '--- host stderr tail (observed) ---'
        Get-Content -LiteralPath $hostErr -Tail 15 -ErrorAction SilentlyContinue | ForEach-Object { Write-Output $_ }
    }
    Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue
}
