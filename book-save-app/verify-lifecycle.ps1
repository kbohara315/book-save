param(
    [switch]$Worker,      # compat: ignored (single-level probe, no nested worker)
    [string]$StateFile    # compat: ignored
)

# Bounded single-process lifecycle probe. Worst-case wall time ~35s; every
# wait carries an explicit timeout and only owned PIDs are ever touched.
# Cycle: host starts -> Open -> visible window answers WM_NULL -> WM_CLOSE
# exits UI -> host survives -> tray Exit ends host. Every wait is bounded and
# only PIDs launched by this run (path-checked) are ever killed.
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
public static class LifecycleProbe {
    public delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr state);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr state);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int count);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int n);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int left, top, right, bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int x, y; }
    public static IntPtr Find(string title, string className, uint ownerPid, bool visible) {
        IntPtr found = IntPtr.Zero;
        var owned = new System.Collections.Generic.List<IntPtr>();
        EnumWindows((hwnd, _) => { uint pid; GetWindowThreadProcessId(hwnd, out pid); if (pid == ownerPid) { owned.Add(hwnd); } return true; }, IntPtr.Zero);
        foreach (var hwnd in owned) {
            if (IsWindowVisible(hwnd) != visible) continue;
            var text = new StringBuilder(256); GetWindowText(hwnd, text, text.Capacity);
            var cls = new StringBuilder(128); GetClassName(hwnd, cls, cls.Capacity);
            if ((string.IsNullOrEmpty(title) || text.ToString() == title) && (string.IsNullOrEmpty(className) || cls.ToString() == className)) { found = hwnd; break; }
        }
        return found;
    }
    public static string Describe(uint ownerPid) {        var rows = new System.Collections.Generic.List<string>();
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
    public static uint PidOf(IntPtr hwnd) { uint pid; GetWindowThreadProcessId(hwnd, out pid); return pid; }
    [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool GetExitCodeProcess(IntPtr h, out uint code);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
    public static IntPtr OpenForExitCode(uint pid) { return OpenProcess(0x1000, false, pid); }
    public static string ExitCodeOf(IntPtr h) {
        if (h == IntPtr.Zero) return "no-handle";
        uint code; if (!GetExitCodeProcess(h, out code)) return "query-failed";
        if (code == 259) return "STILL_ACTIVE";
        return code.ToString();
    }
    public static bool ClickMenu(IntPtr menu, bool last) {        RECT r; POINT prior;
        if (!GetWindowRect(menu, out r) || !GetCursorPos(out prior)) return false;
        SetCursorPos((r.left + r.right) / 2, last ? r.bottom - 10 : r.top + 12);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero); mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
        SetCursorPos(prior.x, prior.y); return true;
    }
}
'@

function Wait-ForWindow([string]$Title, [string]$Class, [uint32]$OwnerPid, [bool]$Visible, [int]$TimeoutMs) {
    $end = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $end) {
        $w = [LifecycleProbe]::Find($Title, $Class, $OwnerPid, $Visible)
        if ($w.ToInt64() -ne 0) { return $w }
        Start-Sleep -Milliseconds 100
    }
    return [IntPtr]::Zero
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('book-save-lifecycle-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
$localAppData = Join-Path $testRoot 'LocalAppData'
New-Item -ItemType Directory -Path $localAppData -Force | Out-Null
$hostOut = Join-Path $testRoot 'host.stdout.log'
$hostErr = Join-Path $testRoot 'host.stderr.log'
New-Item -ItemType File -Path $hostOut -Force | Out-Null
New-Item -ItemType File -Path $hostErr -Force | Out-Null

$instance = [guid]::NewGuid().ToString('N')
$scriptStart = [DateTime]::UtcNow
function Stamp([string]$Msg) { Write-Output ("[{0:N1}s] {1}" -f ([DateTime]::UtcNow - $scriptStart).TotalSeconds, $Msg) }
$savedLocal = $env:LOCALAPPDATA
$savedInst = $env:BOOK_SAVE_TEST_INSTANCE
$ownedHostPid = 0
$ownedUiPid = 0
$uiHandle = [IntPtr]::Zero
$hostHandle = [IntPtr]::Zero
$failures = @()
try {
    $env:LOCALAPPDATA = $localAppData
    $env:BOOK_SAVE_TEST_INSTANCE = $instance
    $hostProc = Start-Process -FilePath $hostExe -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $hostOut -RedirectStandardError $hostErr `
        -WorkingDirectory $PSScriptRoot
    $ownedHostPid = $hostProc.Id
    $hostHandle = [LifecycleProbe]::OpenForExitCode([uint32]$ownedHostPid)
    Write-Output "host PID $ownedHostPid instance $instance"

    # 1. Host spawns its GPUI child (bounded 6s; observed <1s).
    $uiRec = $null
    $end = [DateTime]::UtcNow.AddSeconds(6)
    while ([DateTime]::UtcNow -lt $end) {
        $uiRec = Get-CimInstance Win32_Process -Filter "Name = 'book-save-ui.exe'" -ErrorAction SilentlyContinue |
            Where-Object { [int]$_.ParentProcessId -eq $ownedHostPid } | Select-Object -First 1
        if ($uiRec) { break }
        $hostProc.Refresh()
        if ($hostProc.HasExited) { break }
        Start-Sleep -Milliseconds 150
    }
    if (-not $uiRec) { throw 'Host did not start its GPUI child within 6s.' }
    $ownedUiPid = [int]$uiRec.ProcessId
    $uiProc = Get-Process -Id $ownedUiPid -ErrorAction Stop
    if (-not ([IO.Path]::GetFullPath($uiProc.Path) -ieq [IO.Path]::GetFullPath($uiExe))) {
        throw "Host child PID $ownedUiPid is not $uiExe (actual: $($uiProc.Path))."
    }
    Write-Output "ui child PID $ownedUiPid"
    $uiHandle = [LifecycleProbe]::OpenForExitCode([uint32]$ownedUiPid)
    if ($uiHandle -eq [IntPtr]::Zero) { throw 'Could not open a handle to the UI child for exit-code observation.' }
    if ($hostProc.HasExited) { throw 'Resident host exited after starting the UI child.' }

    # 2. Visible window + bounded WM_NULL (10s + 4s; window observed ~4s).
    $uiWindow = [IntPtr]::Zero
    $waitEnd = [DateTime]::UtcNow.AddSeconds(10)
    $nextLog = [DateTime]::UtcNow
    while ([DateTime]::UtcNow -lt $waitEnd -and $uiWindow.ToInt64() -eq 0) {
        $uiWindow = [LifecycleProbe]::Find('Book Save', $null, [uint32]$ownedUiPid, $true)
        if ($uiWindow.ToInt64() -eq 0 -and [DateTime]::UtcNow -ge $nextLog) {
            Stamp ("poll: find-miss windows=[" + [LifecycleProbe]::Describe([uint32]$ownedUiPid) + "]")
            $nextLog = [DateTime]::UtcNow.AddSeconds(4)
        }
        if ($uiWindow.ToInt64() -eq 0) { Start-Sleep -Milliseconds 150 }
    }
    if ($uiWindow.ToInt64() -eq 0) {
        $uiProc.Refresh()
        throw "GPUI child did not create its visible Book Save window (responding=$($uiProc.Responding), main=$($uiProc.MainWindowHandle), windows=$([LifecycleProbe]::Describe([uint32]$ownedUiPid)))."
    }
    Write-Output "PASS host starts an independent GPUI child; visible hwnd=$uiWindow"
    $result = [IntPtr]::Zero
    if ([LifecycleProbe]::SendMessageTimeout($uiWindow, 0, [IntPtr]::Zero, [IntPtr]::Zero, 0, 4000, [ref]$result).ToInt64() -eq 0) {
        $uiProc.Refresh()
        throw "UI window did not answer WM_NULL within 4s (lastError=$([Runtime.InteropServices.Marshal]::GetLastWin32Error()), uiResponding=$($uiProc.Responding), hwnd=$uiWindow)."
    }
    Write-Output 'PASS visible window answers bounded WM_NULL.'

    # 3. WM_CLOSE exits UI, host survives (bounded 5s; observed <1s).
    if (-not [LifecycleProbe]::PostMessage($uiWindow, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'Could not post WM_CLOSE to the UI window.' }
    if (-not $uiProc.WaitForExit(5000)) { throw 'WM_CLOSE did not make the UI process exit within 5s.' }
    $uiProc.Refresh()
    Stamp ("ui post-close hasExited=" + $uiProc.HasExited)
    $uiCodeStr = [LifecycleProbe]::ExitCodeOf($uiHandle)
    Stamp ("ui exit code via held handle: [" + $uiCodeStr + "]")
    if ($uiCodeStr -ne '0') { throw "UI exit code [$uiCodeStr] (expected 0 for graceful WM_CLOSE)." }
    $hostProc.Refresh()
    if ($hostProc.HasExited) { throw 'Host did not remain resident after WM_CLOSE closed the UI.' }
    Write-Output 'PASS WM_CLOSE exits the GPUI process while the resident host remains alive.'

    # 4. Tray Exit stops host (bounded 8s).
    # 4. Tray Exit stops host (bounded). The tray menu opens at the current
    # cursor position, so park the cursor at primary-screen center first:
    # fully on-screen menu => deterministic Exit-row geometry. The click
    # targets only the verified host-owned menu rect; cursor is restored.
    $tray = [LifecycleProbe]::Find($null, 'BookSaveTrayWindow', [uint32]$ownedHostPid, $false)
    if ($tray.ToInt64() -eq 0) { throw 'Could not find the host-owned tray window.' }
    $priorPt = New-Object LifecycleProbe+POINT
    [LifecycleProbe]::GetCursorPos([ref]$priorPt) | Out-Null
    $cx = [int]([LifecycleProbe]::GetSystemMetrics(0) / 2)
    $cy = [int]([LifecycleProbe]::GetSystemMetrics(1) / 2)
    [LifecycleProbe]::SetCursorPos($cx, $cy) | Out-Null
    try {
        if (-not [LifecycleProbe]::PostMessage($tray, 0x8001, [IntPtr]1, [IntPtr]0x0205)) { throw 'Could not post tray right-click callback.' }
        $menu = Wait-ForWindow $null '#32768' ([uint32]$ownedHostPid) $true 2000
        if ($menu.ToInt64() -eq 0) { throw 'Owned tray menu did not open within 2s.' }
        Start-Sleep -Milliseconds 250
        if (-not [LifecycleProbe]::ClickMenu($menu, $true)) { throw 'Could not click the owned tray menu.' }
    } finally {
        [LifecycleProbe]::SetCursorPos($priorPt.x, $priorPt.y) | Out-Null
    }
    if (-not $hostProc.WaitForExit(5000)) { throw 'Selecting Exit in the host-owned tray menu did not stop the host within 5s.' }
    $hostProc.Refresh()
    $hostCodeStr = [LifecycleProbe]::ExitCodeOf($hostHandle)
    Stamp ("host exit code via held handle: [" + $hostCodeStr + "]")
    if ($hostCodeStr -ne '0') { throw "Host exit code [$hostCodeStr] (expected 0)." }
    Write-Output 'PASS selecting tray Exit stops the resident host after the UI has already exited.'
}
catch {
    $failures += $_.Exception.Message
    Write-Output "FAIL $($_.Exception.Message)"
    exit 1
}
finally {
    $env:LOCALAPPDATA = $savedLocal
    $env:BOOK_SAVE_TEST_INSTANCE = $savedInst
    if ($uiHandle -ne [IntPtr]::Zero) { [LifecycleProbe]::CloseHandle($uiHandle) | Out-Null }
    if ($hostHandle -ne [IntPtr]::Zero) { [LifecycleProbe]::CloseHandle($hostHandle) | Out-Null }
    foreach ($pair in @(@($ownedUiPid, $uiExe), @($ownedHostPid, $hostExe))) {
        $id = [int]$pair[0]
        $exe = $pair[1]
        if ($id -ne 0) {
            try {
                $proc = Get-Process -Id $id -ErrorAction Stop
                if ($proc.Path -and [IO.Path]::GetFullPath($proc.Path) -ieq [IO.Path]::GetFullPath($exe)) {
                    Stop-Process -Id $id -Force -ErrorAction SilentlyContinue
                    $proc.WaitForExit(3000) | Out-Null
                }
            } catch { }
        }
    }
    Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue
}
