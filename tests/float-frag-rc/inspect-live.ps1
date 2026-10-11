# Read-only: inspect the LIVE running instance's ball window (no launching, no killing).
$RC = 'C:\dev\rust\web-app\tests\float-frag-rc'
$out = Join-Path $RC 'live-inspect.txt'

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class LV {
    public delegate bool P(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(P cb, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int m);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int m);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
    [DllImport("user32.dll")] public static extern long GetWindowLongPtrW(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint f);
    [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
}
"@

$lines = New-Object System.Collections.ArrayList

$targets = Get-Process | Where-Object { $_.ProcessName -like "*osfipin*" -or $_.ProcessName -like "*web2app*" }
foreach ($t in $targets) {
    [void]$lines.Add("=== pid=$($t.Id) name=$($t.ProcessName) path=$($t.Path)")
}

$script:pidList = @($targets | ForEach-Object { [uint32]$_.Id })
[void]$lines.Add("target pids: $($script:pidList -join ',')")

# EnumWindows order == Z order from top to bottom. Record global index for overlap analysis.
$script:idx = 0
$script:all = New-Object System.Collections.ArrayList
$cb = [LV+P]{
    param($h, $l)
    $script:idx = $script:idx + 1
    $procId = 0
    [void][LV]::GetWindowThreadProcessId($h, [ref]$procId)
    $r = New-Object LV+R
    [void][LV]::GetWindowRect($h, [ref]$r)
    $cls = New-Object System.Text.StringBuilder 256
    [void][LV]::GetClassNameW($h, $cls, 256)
    $tt = New-Object System.Text.StringBuilder 512
    [void][LV]::GetWindowTextW($h, $tt, 512)
    $ex = [LV]::GetWindowLongPtrW($h, -20)
    $st = [LV]::GetWindowLongPtrW($h, -16)
    $vis = [LV]::IsWindowVisible($h)
    $own = $false
    if ($script:pidList -contains $procId) { $own = $true }
    [void]$script:all.Add([pscustomobject]@{
        i=$script:idx; z=$script:idx; h=$h; pid=$procId; own=$own; cls=$cls.ToString(); t=$tt.ToString();
        x=$r.L; y=$r.T; w=($r.Rt-$r.L); hh=($r.B-$r.T); ex=$ex; st=$st; vis=$vis
    })
    return $true
}
[void][LV]::EnumWindows($cb, [IntPtr]::Zero)

[void]$lines.Add("")
[void]$lines.Add("--- windows of target process (own=true), Z order ---")
foreach ($o in $script:all) {
    if ($o.own) {
        $flags = @()
        if (($o.ex -band 0x40000) -ne 0) { $flags += 'APPWINDOW' }
        if (($o.ex -band 0x80) -ne 0)    { $flags += 'TOOLWINDOW' }
        if (($o.ex -band 0x8) -ne 0)     { $flags += 'TOPMOST' }
        if (($o.ex -band 0x8000000) -ne 0) { $flags += 'NOACTIVATE' }
        $fl = if ($flags.Count) { $flags -join '|' } else { '(none)' }
        [void]$lines.Add(("z={0,4} hwnd=0x{1:X} vis={2,-5} rect={3},{4} {5}x{6} st=0x{7:X} ex=0x{8:X} [{9}] title=[{10}] cls={11}" -f `
            $o.z, $o.h.ToInt64(), $o.vis, $o.x, $o.y, $o.w, $o.hh, $o.st, $o.ex, $fl, $o.t, $o.cls))
    }
}

# For each own-window that looks like the ball, list the 6 windows right BELOW it in Z order
# (those are the ones that could peek out from behind an always-on-top transparent ball).
[void]$lines.Add("")
[void]$lines.Add("--- windows immediately BELOW each ball candidate (Z order) ---")
foreach ($o in $script:all) {
    if ($o.own -and $o.vis -and $o.w -ge 120 -and $o.w -le 150 -and $o.hh -ge 50 -and $o.hh -le 62) {
        [void]$lines.Add(("BALL z={0} hwnd=0x{1:X} rect={2},{3} {4}x{5}" -f $o.z, $o.h.ToInt64(), $o.x, $o.y, $o.w, $o.hh))
        $below = $script:all | Where-Object { $_.z -gt $o.z } | Select-Object -First 8
        foreach ($b in $below) {
            $flags = @()
            if (($b.ex -band 0x40000) -ne 0) { $flags += 'APPWINDOW' }
            if (($b.ex -band 0x80) -ne 0)    { $flags += 'TOOLWINDOW' }
            if (($b.ex -band 0x8) -ne 0)     { $flags += 'TOPMOST' }
            if (($b.ex -band 0x8000000) -ne 0) { $flags += 'NOACTIVATE' }
            $fl = if ($flags.Count) { $flags -join '|' } else { '(none)' }
            $ov = 'no-overlap'
            if (-not ($b.x -ge ($o.x + $o.w) -or ($b.x + $b.w) -le $o.x -or $b.y -ge ($o.y + $o.hh) -or ($b.y + $b.hh) -le $o.y)) { $ov = 'OVERLAPS' }
            [void]$lines.Add(("   below z={0,4} hwnd=0x{1:X} pid={2} vis={3,-5} rect={4},{5} {6}x{7} [{8}] cls={9} title=[{10}] {11}" -f `
                $b.z, $b.h.ToInt64(), $b.pid, $b.vis, $b.x, $b.y, $b.w, $b.hh, $fl, $b.cls, $b.t, $ov))
        }
    }
}

# Also: every visible top-level window that overlaps the ball rect (any process).
[void]$lines.Add("")
[void]$lines.Add("--- ALL visible windows overlapping a ball rect (any process) ---")
foreach ($o in $script:all) {
    if ($o.own -and $o.vis -and $o.w -ge 120 -and $o.w -le 150 -and $o.hh -ge 50 -and $o.hh -le 62) {
        foreach ($b in $script:all) {
            if ($b.z -le $o.z) { continue }
            if (-not $b.vis) { continue }
            if ($b.x -ge ($o.x + $o.w) -or ($b.x + $b.w) -le $o.x -or $b.y -ge ($o.y + $o.hh) -or ($b.y + $b.hh) -le $o.y) { continue }
            [void]$lines.Add(("   z={0,4} hwnd=0x{1:X} pid={2} rect={3},{4} {5}x{6} cls={7} title=[{8}]" -f `
                $b.z, $b.h.ToInt64(), $b.pid, $b.x, $b.y, $b.w, $b.hh, $b.cls, $b.t))
        }
    }
}

$lines -join "`r`n" | Out-File -FilePath $out -Encoding UTF8
Write-Output "wrote $out"
