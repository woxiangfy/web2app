# 枚举器自检：统计可见窗口总数 + 40..300px 小窗口数（验证 EnumWindows 在本会话可用）
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WE3 {
    public delegate bool P(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(P cb, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern int GetClassNameW(IntPtr h, [MarshalAs(UnmanagedType.LPWStr)] StringBuilder s, int m);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
    [StructLayout(LayoutKind.Sequential)] public struct R { public int L,T,Rt,B; }
}
"@
$script:n = 0
$script:small = New-Object System.Collections.ArrayList
$cb = [WE3+P]{
    param($h, $l)
    if ([WE3]::IsWindowVisible($h)) {
        $script:n++
        $r = New-Object WE3+R
        [void][WE3]::GetWindowRect($h, [ref]$r)
        $w = $r.Rt - $r.L; $hh = $r.B - $r.T
        if ($w -ge 40 -and $hh -ge 40 -and $w -le 300 -and $hh -le 300) {
            $cls = New-Object System.Text.StringBuilder 256
            [void][WE3]::GetClassNameW($h, $cls, 256)
            [void]$script:small.Add(('{0} {1}x{2} @({3},{4})' -f $cls.ToString(), $w, $hh, $r.L, $r.T))
        }
    }
    return $true
}
[void][WE3]::EnumWindows($cb, [IntPtr]::Zero)
Write-Output ('visible total = ' + $script:n)
Write-Output ('small 40..300px count = ' + $script:small.Count)
$script:small | Select-Object -First 40 | ForEach-Object { Write-Output $_ }
