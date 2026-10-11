# 全量枚举顶层窗口 → 落盘（避免 stdout 截断）。修复 $pid 保留名冲突。
$RC = 'C:\dev\rust\web-app\tests\float-frag-rc'
$outFile = Join-Path $RC 'enum.txt'

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WE4 {
    public delegate bool P(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(P cb, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int m);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int m);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
    [DllImport("user32.dll")] public static extern long GetWindowLongPtrW(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint c);
    [DllImport("user32.dll")] public static extern int GetWindowLongW(IntPtr h, int i);
    [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
}
"@

$list = New-Object System.Collections.ArrayList
$cb = [WE4+P]{
    param($h, $l)
    $r = New-Object WE4+R
    [void][WE4]::GetWindowRect($h, [ref]$r)
    $cls = New-Object System.Text.StringBuilder 256
    [void][WE4]::GetClassNameW($h, $cls, 256)
    $tt = New-Object System.Text.StringBuilder 256
    [void][WE4]::GetWindowTextW($h, $tt, 256)
    $procId = 0
    [void][WE4]::GetWindowThreadProcessId($h, [ref]$procId)
    $ex = [WE4]::GetWindowLongPtrW($h, -20)
    $st = [WE4]::GetWindowLongW($h, -16)
    $owner = [WE4]::GetWindow($h, 4)   # GW_OWNER
    [void]$list.Add([pscustomobject]@{
        hwnd  = ('0x{0:X}' -f $h.ToInt64())
        vis   = [WE4]::IsWindowVisible($h)
        x     = $r.L
        y     = $r.T
        w     = $r.Rt - $r.L
        h     = $r.B - $r.T
        cls   = $cls.ToString()
        title = $tt.ToString()
        ex    = ('0x{0:X}' -f $ex)
        st    = ('0x{0:X}' -f $st)
        pid   = $procId
        owner = ('0x{0:X}' -f $owner.ToInt64())
    })
    return $true
}
[void][WE4]::EnumWindows($cb, [IntPtr]::Zero)

$lines = New-Object System.Collections.ArrayList
[void]$lines.Add(("TOTAL={0}" -f $list.Count))
[void]$lines.Add("=== 全部窗口（按 y 排序，非 0x0 的优先）===")
foreach ($e in ($list | Sort-Object @{e={if($_.w -eq 0){1}else{0}}}, y, x)) {
    [void]$lines.Add(("hwnd={0,-12} vis={1,-5} rect={2,6},{3,-6} {4,5}x{5,-5} ex={6,-10} pid={7,-6} cls={8} | title={9}" -f `
        $e.hwnd, $e.vis, $e.x, $e.y, $e.w, $e.h, $e.ex, $e.pid, $e.cls, $e.title))
}
$lines -join "`r`n" | Out-File -FilePath $outFile -Encoding UTF8
Write-Output ("wrote {0} windows to {1}" -f $list.Count, $outFile)
