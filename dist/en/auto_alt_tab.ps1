param(
    [Parameter(Mandatory = $true)]
    [int]$TargetPid,
    [int]$WindowWaitSeconds = 90
)

$ErrorActionPreference = 'SilentlyContinue'
$script:targetPid = $TargetPid
$logPath = Join-Path $PSScriptRoot 'battle_instinct_window_focus.log'

function Write-Log([string]$Message) {
    try {
        $line = '{0:yyyy-MM-dd HH:mm:ss.fff} {1}' -f (Get-Date), $Message
        [System.IO.File]::AppendAllText($logPath, $line + "`r`n", [System.Text.Encoding]::UTF8)
    } catch {
    }
}

Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Runtime.InteropServices;

public static class PjFocus {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool IsIconic(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);

    [DllImport("user32.dll")]
    public static extern bool BringWindowToTop(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern IntPtr SetActiveWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    public static extern bool AttachThreadInput(uint idAttach, uint idAttachTo, bool fAttach);

    [DllImport("kernel32.dll")]
    public static extern uint GetCurrentThreadId();

    [DllImport("user32.dll")]
    public static extern void SwitchToThisWindow(IntPtr hWnd, bool fUnknown);

    [DllImport("user32.dll")]
    public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
}
"@

function Find-GameWindow {
    $script:foundWindow = [IntPtr]::Zero
    $callback = [PjFocus+EnumWindowsProc] {
        param($hWnd, $lParam)
        $processId = [uint32]0
        [void][PjFocus]::GetWindowThreadProcessId($hWnd, [ref]$processId)
        if ($processId -eq $script:targetPid -and [PjFocus]::IsWindowVisible($hWnd)) {
            $className = New-Object System.Text.StringBuilder 128
            [void][PjFocus]::GetClassName($hWnd, $className, 128)
            if ($className.ToString() -eq 'CSclass') {
                $script:foundWindow = $hWnd
                return $false
            }
        }
        return $true
    }
    [void][PjFocus]::EnumWindows($callback, [IntPtr]::Zero)
    return $script:foundWindow
}

function Send-AltTab {
    [PjFocus]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)
    [PjFocus]::keybd_event(0x09, 0, 0, [UIntPtr]::Zero)
    [PjFocus]::keybd_event(0x09, 0, 2, [UIntPtr]::Zero)
    [PjFocus]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
}

function Restore-GameWindow([IntPtr]$hWnd) {
    $foreground = [PjFocus]::GetForegroundWindow()
    $foregroundPid = [uint32]0
    $foregroundThread = [PjFocus]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid)
    $currentThread = [PjFocus]::GetCurrentThreadId()

    $attached = $false
    if ($foregroundThread -ne 0 -and $foregroundThread -ne $currentThread) {
        $attached = [PjFocus]::AttachThreadInput($currentThread, $foregroundThread, $true)
    }

    $showCommand = if ([PjFocus]::IsIconic($hWnd)) { 9 } else { 5 }
    [void][PjFocus]::ShowWindowAsync($hWnd, $showCommand)
    [void][PjFocus]::BringWindowToTop($hWnd)
    $setForeground = [PjFocus]::SetForegroundWindow($hWnd)
    [void][PjFocus]::SetActiveWindow($hWnd)

    if ($attached) {
        [void][PjFocus]::AttachThreadInput($currentThread, $foregroundThread, $false)
    }

    Start-Sleep -Milliseconds 250
    $foregroundAfter = [PjFocus]::GetForegroundWindow()
    $visible = [PjFocus]::IsWindowVisible($hWnd)
    $iconic = [PjFocus]::IsIconic($hWnd)
    Write-Log "restore set_foreground=$setForeground attached=$attached foreground=$foregroundAfter visible=$visible iconic=$iconic"

    if ($foregroundAfter -ne $hWnd -or -not $visible -or $iconic) {
        [PjFocus]::SwitchToThisWindow($hWnd, $true)
        Start-Sleep -Milliseconds 300
    }

    $foregroundFinal = [PjFocus]::GetForegroundWindow()
    $visibleFinal = [PjFocus]::IsWindowVisible($hWnd)
    $iconicFinal = [PjFocus]::IsIconic($hWnd)
    return ($foregroundFinal -eq $hWnd -and $visibleFinal -and -not $iconicFinal)
}

Write-Log "start target_pid=$TargetPid"

$gameWindow = [IntPtr]::Zero
$waitIterations = [Math]::Max(1, $WindowWaitSeconds * 2)
for ($i = 0; $i -lt $waitIterations; $i++) {
    $gameWindow = Find-GameWindow
    if ($gameWindow -ne [IntPtr]::Zero) {
        break
    }
    Start-Sleep -Milliseconds 500
}
if ($gameWindow -eq [IntPtr]::Zero) {
    Write-Log 'game window not found'
    exit 1
}

Write-Log "window found hwnd=$gameWindow"
Start-Sleep -Milliseconds 500
Send-AltTab
Write-Log 'sent alt+tab out'
Start-Sleep -Milliseconds 700

$restored = $false
for ($attempt = 1; $attempt -le 5; $attempt++) {
    $restored = Restore-GameWindow $gameWindow
    Write-Log "attempt=$attempt restored=$restored"
    if ($restored) {
        break
    }
    Start-Sleep -Milliseconds 400
}

Write-Log "done restored=$restored"
if ($restored) { exit 0 } else { exit 2 }
