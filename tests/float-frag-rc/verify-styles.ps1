# v1.77 运行时验证：启动 debug 构建，读回球窗的真实扩展样式（GWL_EXSTYLE / 标题 / 矩形）。
$RC = 'C:\dev\rust\web-app\tests\float-frag-rc'
$exe = 'C:\dev\rust\web-app\webview-app\target\debug\web2app.exe'
$out = Join-Path $RC 'styles.txt'

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WE5 {
    public delegate bool P(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(P cb, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, P cb, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int m);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int m);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
    [DllImport("user32.dll")] public static extern long GetWindowLongPtrW(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
}
"@

$p = Start-Process -FilePath $exe -WorkingDirectory (Split-Path $exe) -PassThru
Start-Sleep -Seconds 16
$appPid = $p.Id

$lines = New-Object System.Collections.ArrayList
[void]$lines.Add(("app pid={0} hasExited={1}" -f $appPid, $p.HasExited))

$cb = [WE5+P]{
    param($h, $l)
    $procId = 0
    [void][WE5]::GetWindowThreadProcessId($h, [ref]$procId)
    if ($procId -ne $script:appPid) { return $true }
    $r = New-Object WE5+R
    [void][WE5]::GetWindowRect($h, [ref]$r)
    $cls = New-Object System.Text.StringBuilder 256
    [void][WE5]::GetClassNameW($h, $cls, 256)
    $tt = New-Object System.Text.StringBuilder 512
    [void][WE5]::GetWindowTextW($h, $tt, 512)
    $ex = [WE5]::GetWindowLongPtrW($h, -20)
    $st = [WE5]::GetWindowLongPtrW($h, -16)
    $vis = [WE5]::IsWindowVisible($h)
    $w = $r.Rt - $r.L; $hh = $r.B - $r.T
    $flags = @()
    if (($ex -band 0x40000) -ne 0) { $flags += 'APPWINDOW' }
    if (($ex -band 0x80) -ne 0)    { $flags += 'TOOLWINDOW' }
    if (($ex -band 0x8) -ne 0)     { $flags += 'TOPMOST' }
    if (($ex -band 0x8000000) -ne 0) { $flags += 'NOACTIVATE' }
    $fl = if ($flags.Count) { $flags -join '|' } else { '(none)' }
    [void]$script:lines.Add(("hwnd=0x{0:X} vis={1,-5} rect={2},{3} {4}x{5} st={6} ex=0x{7:X} [{8}] title=[{9}] cls={10}" -f `
        $h.ToInt64(), $vis, $r.L, $r.T, $w, $hh, ('0x{0:X}' -f $st), $ex, $fl, $tt.ToString(), $cls.ToString()))
    return $true
}
[void][WE5]::EnumWindows($cb, [IntPtr]::Zero)

$lines -join "`r`n" | Out-File -FilePath $out -Encoding UTF8

if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
Get-Process msedgewebview2 -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Write-Output ("wrote {0}" -f $out)
