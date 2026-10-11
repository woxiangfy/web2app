# 取证：枚举悬浮球区域 (1943,393,70,70) 附近的所有顶层窗口（抓「背景异样卡片」现行）
# 用法：直接跑；球窗若开着、残片若还在，会把肇事窗口的 类名/标题/进程/样式/矩形 全打出来
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class WinEnum {
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lp);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern int GetWindowTextW(IntPtr hWnd, [MarshalAs(UnmanagedType.LPWStr)] StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern int GetClassNameW(IntPtr hWnd, [MarshalAs(UnmanagedType.LPWStr)] StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hwnd, int attr, out int val, int size);
    [DllImport("user32.dll")] public static extern int GetWindowLongW(IntPtr hWnd, int idx);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@

$rows = New-Object System.Collections.ArrayList
$cb = [WinEnum+EnumWindowsProc]{
    param($h, $l)
    if (-not [WinEnum]::IsWindowVisible($h)) { return $true }
    $r = New-Object WinEnum+RECT
    [void][WinEnum]::GetWindowRect($h, [ref]$r)
    $cloaked = 0
    [void][WinEnum]::DwmGetWindowAttribute($h, 14, [ref]$cloaked, 4)  # DWMWA_CLOAKED
    $title = New-Object System.Text.StringBuilder 512
    [void][WinEnum]::GetWindowTextW($h, $title, 512)
    $cls = New-Object System.Text.StringBuilder 256
    [void][WinEnum]::GetClassNameW($h, $cls, 256)
    $pid2 = 0
    [void][WinEnum]::GetWindowThreadProcessId($h, [ref]$pid2)
    $proc = try { (Get-Process -Id $pid2 -ErrorAction Stop).ProcessName } catch { '?' }
    [void]$rows.Add([pscustomobject]@{
        HWND = ('0x{0:X}' -f $h.ToInt64()); Class = $cls.ToString(); Title = $title.ToString()
        Proc = $proc; PID = $pid2
        Rect = ('({0},{1})-({2},{3}) {4}x{5}' -f $r.L, $r.T, $r.R, $r.B, ($r.R-$r.L), ($r.B-$r.T))
        Cloaked = $cloaked
        Style = ('0x{0:X8}' -f ([WinEnum]::GetWindowLongW($h, -16)))
        ExStyle = ('0x{0:X8}' -f ([WinEnum]::GetWindowLongW($h, -20)))
    })
    return $true
}
[void][WinEnum]::EnumWindows($cb, [IntPtr]::Zero)

# 球的落盘位置（tests/pack/tauri/data/settings.json）：活柱 (1943,393) 70x70
$bx = 1943; $by = 393; $bw = 70; $bh = 70
Write-Output '===== 与球区域相交的可见窗口 ====='
$rows | Where-Object {
    $m = [regex]::Match($_.Rect, '\((-?\d+),(-?\d+)\)-\((-?\d+),(-?\d+)\)')
    if (-not $m.Success) { return $false }
    $l = [int]$m.Groups[1].Value; $t = [int]$m.Groups[2].Value
    $rt = [int]$m.Groups[3].Value; $b = [int]$m.Groups[4].Value
    ($l -lt ($bx + $bw + 40)) -and ($rt -gt ($bx - 40)) -and ($t -lt ($by + $bh + 40)) -and ($b -gt ($by - 40))
} | Format-Table -AutoSize | Out-String -Width 260 | Write-Output

Write-Output '===== 全部可见、宽高都在 [40..300] 的小窗口（卡片嫌疑） ====='
$rows | Where-Object {
    $m = [regex]::Match($_.Rect, '\((-?\d+),(-?\d+)\)-\((-?\d+),(-?\d+)\)')
    if (-not $m.Success) { return $false }
    $w = [int]$m.Groups[3].Value - [int]$m.Groups[1].Value
    $h = [int]$m.Groups[4].Value - [int]$m.Groups[2].Value
    $w -ge 40 -and $h -ge 40 -and $w -le 300 -and $h -le 300
} | Format-Table -AutoSize | Out-String -Width 260 | Write-Output

Write-Output '===== msedgewebview2 进程 ====='
Get-Process -Name msedgewebview2 -ErrorAction SilentlyContinue | Select-Object Id, MainWindowTitle, StartTime | Format-Table -AutoSize | Out-String | Write-Output
Write-Output ('now = ' + (Get-Date -Format 'HH:mm:ss'))
