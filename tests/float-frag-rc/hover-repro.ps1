# Reproduce the user's screenshot: capture the ball area, hover the real cursor over
# the ball, capture again, and list every window that became visible near the ball.
$RC = 'C:\dev\rust\web-app\tests\float-frag-rc'
$log = Join-Path $RC 'hover-repro.txt'

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class HR {
    public delegate bool P(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(P cb, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int m);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int m);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
    [DllImport("user32.dll")] public static extern long GetWindowLongPtrW(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out System.Drawing.Point pt);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(System.Drawing.Point pt);
    [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
}
"@

function Shot([string]$name, [int]$x, [int]$y, [int]$w, [int]$h) {
    $rect = New-Object System.Drawing.Rectangle($x, $y, $w, $h)
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($rect.Location, [System.Drawing.Point]::Empty, $rect.Size)
    $p = Join-Path $RC $name
    $bmp.Save($p, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    return $p
}

# the live ball window (from inspect-live): 1383,74 136x56, live pillar is 56 logical wide
$bx = 1383; $by = 74; $bw = 136; $bh = 56

$lines = New-Object System.Collections.ArrayList
[void]$lines.Add("ball host rect = $bx,$by ${bw}x${bh}")

$cur = New-Object System.Drawing.Point
[void][HR]::GetCursorPos([ref]$cur)
[void]$lines.Add("cursor before = $($cur.X),$($cur.Y)")

[void]$lines.Add("shot1 (no hover) -> " + (Shot 'hover-1-nohover.png' ($bx - 60) ($by - 40) 260 170))

# hover the live pillar centre: try left-half centre then right-half centre
foreach ($pt in @(@($bx + 28, $by + 28), @($bx + 108, $by + 28))) {
    [void][HR]::SetCursorPos($pt[0], $pt[1])
    Start-Sleep -Milliseconds 1200
    $h = [HR]::WindowFromPoint((New-Object System.Drawing.Point($pt[0], $pt[1])))
    $c = New-Object System.Text.StringBuilder 256
    [void][HR]::GetClassNameW($h, $c, 256)
    [void]$lines.Add("cursor at $($pt[0]),$($pt[1]) -> windowUnder=0x$($h.ToInt64().ToString('X')) cls=$($c.ToString())")
}
[void]$lines.Add("shot2 (hover) -> " + (Shot 'hover-2-hover.png' ($bx - 60) ($by - 40) 260 170))

# windows visible near the ball, all processes
$script:bx = $bx; $script:by = $by; $script:bw = $bw; $script:bh = $bh
$cb = [HR+P]{
    param($h, $l)
    if (-not [HR]::IsWindowVisible($h)) { return $true }
    $r = New-Object HR+R
    [void][HR]::GetWindowRect($h, [ref]$r)
    if ($r.Rt -le $script:bx -or $r.L -ge ($script:bx + $script:bw)) { return $true }
    if ($r.B -le $script:by -or $r.T -ge ($script:by + $script:bh)) { return $true }
    $procId = 0
    [void][HR]::GetWindowThreadProcessId($h, [ref]$procId)
    $c = New-Object System.Text.StringBuilder 256
    [void][HR]::GetClassNameW($h, $c, 256)
    $tt = New-Object System.Text.StringBuilder 512
    [void][HR]::GetWindowTextW($h, $tt, 512)
    $ex = [HR]::GetWindowLongPtrW($h, -20)
    $pn = ''
    try { $pn = (Get-Process -Id $procId -ErrorAction Stop).ProcessName } catch { $pn = '?' }
    [void]$script:lines.Add(("  overlap VISIBLE hwnd=0x{0:X} pid={1}({2}) rect={3},{4} {5}x{6} ex=0x{7:X} cls={8} title=[{9}]" -f `
        $h.ToInt64(), $procId, $pn, $r.L, $r.T, ($r.Rt - $r.L), ($r.B - $r.T), $ex, $c.ToString(), $tt.ToString()))
    return $true
}
[void]$lines.Add("--- visible windows overlapping ball rect (hover state) ---")
[void][HR]::EnumWindows($cb, [IntPtr]::Zero)

[void][HR]::SetCursorPos($cur.X, $cur.Y)
$lines -join "`r`n" | Out-File -FilePath $log -Encoding UTF8
Write-Output "wrote $log"
