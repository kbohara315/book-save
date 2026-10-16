param(
    [switch]$Worker,          # compat: ignored (single-level probe, no nested worker)
    [switch]$RedirectStdio,   # compat: ignored (always file-redirected)
    [string]$StateFile,       # compat: ignored
    [switch]$ShowNormal,      # compat: default behavior now; kept as alias
    [switch]$Hidden           # launch with -WindowStyle Hidden (window stays hidden by launch flag)
)

# Bounded single-process probe. MUST exit within ~30s no matter what.
# Technique: Start-Process with stdout/stderr to FILES (never an in-memory
# pipe, so no pipe-buffer deadlock), bounded poll, identity-checked kill.
$ErrorActionPreference = 'Stop'
$uiExe = Join-Path $PSScriptRoot 'target\debug\book-save-ui.exe'
if (-not (Test-Path -LiteralPath $uiExe)) { throw "Build first: $uiExe is missing." }

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class WindowStartupProbe {
  public delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr state);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr state);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int count);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder text, int count);
  public static IntPtr Find(uint ownerPid) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((hwnd, _) => { uint pid; GetWindowThreadProcessId(hwnd, out pid); var text = new StringBuilder(256); GetWindowText(hwnd, text, text.Capacity); if (pid == ownerPid && IsWindowVisible(hwnd) && text.ToString() == "Book Save") { found = hwnd; return false; } return true; }, IntPtr.Zero);
    return found;
  }
  public static string Describe(uint ownerPid) {
    var rows = new System.Collections.Generic.List<string>();
    EnumWindows((hwnd, _) => { uint pid; GetWindowThreadProcessId(hwnd, out pid); if (pid == ownerPid) { var text = new StringBuilder(256); GetWindowText(hwnd, text, text.Capacity); var cls = new StringBuilder(128); GetClassName(hwnd, cls, cls.Capacity); rows.Add("hwnd=" + hwnd + " class=" + cls + " title=" + text + " visible=" + IsWindowVisible(hwnd)); } return true; }, IntPtr.Zero);
    return String.Join("; ", rows.ToArray());
  }
}
'@

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('book-save-window-startup-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
$localAppData = Join-Path $testRoot 'LocalAppData'
New-Item -ItemType Directory -Path $localAppData -Force | Out-Null
$outFile = Join-Path $testRoot 'ui.stdout.log'
$errFile = Join-Path $testRoot 'ui.stderr.log'
New-Item -ItemType File -Path $outFile -Force | Out-Null
New-Item -ItemType File -Path $errFile -Force | Out-Null

$instance = [guid]::NewGuid().ToString('N')
$savedLocal = $env:LOCALAPPDATA
$savedInst = $env:BOOK_SAVE_TEST_INSTANCE
$savedNoHost = $env:BOOK_SAVE_TEST_NO_HOST
$ownedPid = 0
$hardDeadline = [DateTime]::UtcNow.AddSeconds(20)
try {
    $env:LOCALAPPDATA = $localAppData
    $env:BOOK_SAVE_TEST_INSTANCE = $instance
    $env:BOOK_SAVE_TEST_NO_HOST = '1'
    $startArgs = @{
        FilePath = $uiExe
        PassThru = $true
        RedirectStandardOutput = $outFile
        RedirectStandardError = $errFile
        WorkingDirectory = $PSScriptRoot
    }
    if ($Hidden -and -not $ShowNormal) { $startArgs['WindowStyle'] = 'Hidden' }
    $p = Start-Process @startArgs
    $ownedPid = $p.Id
    Write-Output "UI PID $ownedPid image $uiExe instance $instance"

    $foundHwnd = [IntPtr]::Zero
    $pollEnd = [DateTime]::UtcNow.AddSeconds(12)
    while ([DateTime]::UtcNow -lt $pollEnd -and [DateTime]::UtcNow -lt $hardDeadline) {
        $p.Refresh()
        if ($p.HasExited) { break }
        $hwnd = [WindowStartupProbe]::Find([uint32]$ownedPid)
        if ($hwnd -ne [IntPtr]::Zero) { $foundHwnd = $hwnd; break }
        Start-Sleep -Milliseconds 150
    }

    $stages = @()
    if (Test-Path -LiteralPath $errFile) {
        $stages = Get-Content -LiteralPath $errFile -ErrorAction SilentlyContinue |
            Where-Object { $_ -match 'lifecycle-stage:' }
    }
    Write-Output '--- stderr lifecycle stages (observed) ---'
    if ($stages) { $stages | ForEach-Object { Write-Output $_ } } else { Write-Output '(none)' }

    $p.Refresh()
    $windows = [WindowStartupProbe]::Describe([uint32]$ownedPid)
    Write-Output "windows-owned(pid=$ownedPid): $windows"
    Write-Output "exited=$($p.HasExited) responding=$($p.Responding) mainWindow=$($p.MainWindowHandle)"

    if ($foundHwnd -ne [IntPtr]::Zero) {
        Write-Output "PASS VISIBLE_WINDOW hwnd=$foundHwnd"
        exit 0
    }
    if ($Hidden) {
        $completed = ($stages -match 'open-window-complete').Count -gt 0
        Write-Output "HIDDEN_BY_LAUNCH_FLAG open-window-complete=$completed (window hidden because of -WindowStyle Hidden, not a GPUI stall)"
        if ($completed) { exit 0 }
        exit 1
    }
    Write-Output "FAIL NO_VISIBLE_WINDOW exited=$($p.HasExited) responding=$($p.Responding)"
    exit 1
}
finally {
    $env:LOCALAPPDATA = $savedLocal
    $env:BOOK_SAVE_TEST_INSTANCE = $savedInst
    $env:BOOK_SAVE_TEST_NO_HOST = $savedNoHost
    if ($ownedPid -ne 0) {
        try {
            $proc = Get-Process -Id $ownedPid -ErrorAction Stop
            $image = $proc.Path
            if ($image -and [IO.Path]::GetFullPath($image) -ieq [IO.Path]::GetFullPath($uiExe)) {
                Stop-Process -Id $ownedPid -Force -ErrorAction SilentlyContinue
                $proc.WaitForExit(3000) | Out-Null
                Write-Output 'cleanup: owned UI process stopped.'
            } else {
                Write-Output "cleanup: PID $ownedPid is not $uiExe (actual: $image); left alone."
            }
        } catch { Write-Output "cleanup: PID $ownedPid already gone." }
    }
    if (Test-Path -LiteralPath $errFile) {
        Write-Output '--- stderr tail (observed) ---'
        Get-Content -LiteralPath $errFile -Tail 25 -ErrorAction SilentlyContinue | ForEach-Object { Write-Output $_ }
    }
    Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue
}
