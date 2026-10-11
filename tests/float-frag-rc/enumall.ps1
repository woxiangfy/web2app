# 全量枚举顶层窗口：hwnd / 类名 / 标题 / 矩形 / 可见性 / 扩展样式
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WE3 {
    public delegate bool P(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(P cb, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int m);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int m);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
    [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
}
"@

$out = New-Object System.Collections.ArrayList
$cb = [WE3+P]{
    param($h, $l)
    $r = New-Object WE3+R
    [void][WE3]::GetWindowRect($h, [ref]$r)
    $w = $r.Rt - $r.L; $hh = $r.B - $r.T
    $cls = New-Object System.Text.StringBuilder 256
    [void][WE3]::GetClassNameW($h, $cls, 256)
    $tt = New-Object System.Text.StringBuilder 256
    [void][WE3]::GetWindowTextW($h, $tt, 256)
    $pid = 0
    [void][WE3]::GetWindowThreadProcessId($h, [ref]$pid)
    $ex = [WE3]::GetWindowLongPtrW($h, -20)
    $vis = [WE3]::IsWindowVisible($h)
    [void]$out.Add([pscustomobject]@{
        hwnd = ('0x{0:X}' -f $h.ToInt64())
        cls = $cls.ToString()
        title = $tt.ToString()
        x = $r.L; y = $r.T; w = $w; h = $hh
        vis = $vis
        ex = ('0x{0:X}' -f $ex.ToInt64())
        pid = $pid
    })
    return $true
}
[void][WE3]::EnumWindows($cb, [IntPtr]::Zero)

Write-Output ("TOTAL={0}" -f $out.Count)
Write-Output "=== 小窗口 (w<=400 且 h<=400, 含不可见) ==="
$out | Where-Object { $_.w -le 400 -and $_.h -le 400 -and $_.w -gt 0 -and $_.h -gt 0 } |
    Sort-Object y, x | ForEach-Object {
        "{0,-12} vis={1,-5} {2,5},{3,-5} {4,4}x{5,-4} cls={6,-28} title={7}" -f $_.hwnd, $_.vis, $_.x, $_.y, $_.w, $_.h, $_.cls, $_.title
    }
Write-Output "=== 全部窗口 (前 60) ==="
$out | Sort-Object y | Select-Object -First 60 | ForEach-Object {
    "{0,-12} vis={1,-5} {2,5},{3,-5} {4,4}x{5,-4} cls={6,-28} title={7}" -f $_.hwnd, $_.vis, $_.x, $_.y, $_.w, $_.h, $_.cls, $_.title
}
