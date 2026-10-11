# v1.77b decisive experiment:
#   1) start the debug build with float enabled, find the ball HWND;
#   2) screenshot the taskbar strip (baseline);
#   3) call ITaskbarList::DeleteTab WITHOUT HrInit  -> log HRESULT -> screenshot;
#   4) call HrInit() then DeleteTab on a fresh instance -> log HRESULT -> screenshot.
# If (3) fails and (4) succeeds while the button is visible in the baseline,
# the ball window really did own a taskbar button.
$RC = 'C:\dev\rust\web-app\tests\float-frag-rc'
$exe = 'C:\dev\rust\web-app\webview-app\target\debug\web2app.exe'
$log = Join-Path $RC 'taskbar-probe.txt'

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class TB {
    public delegate bool P(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(P cb, IntPtr l);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int m);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
    [DllImport("user32.dll")] public static extern long GetWindowLongPtrW(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
}
"@

Add-Type @"
using System;
using System.Runtime.InteropServices;
[ComImport, Guid("56FDF342-FD6D-11d0-958A-006097C9A090"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface ITaskbarListX {
    void HrInit();
    void AddTab(IntPtr hwnd);
    void DeleteTab(IntPtr hwnd);
    void ActivateTab(IntPtr hwnd);
    void SetActiveAlt(IntPtr hwnd);
}
public static class TBX {
    public static ITaskbarListX Create() {
        Type t = Type.GetTypeFromCLSID(new Guid("56FDF344-FD6D-11d0-958A-006097C9A090"));
        return (ITaskbarListX)Activator.CreateInstance(t);
    }
}
"@

function Shot([string]$name) {
    $scr = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $h = 90
    $rect = New-Object System.Drawing.Rectangle($scr.X, ($scr.Y + $scr.Height - $h), $scr.Width, $h)
    $bmp = New-Object System.Drawing.Bitmap($rect.Width, $rect.Height)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($rect.Location, [System.Drawing.Point]::Empty, $rect.Size)
    $p = Join-Path $RC $name
    $bmp.Save($p, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    return $p
}

$lines = New-Object System.Collections.ArrayList

$p = Start-Process -FilePath $exe -WorkingDirectory (Split-Path $exe) -PassThru
Start-Sleep -Seconds 20
$appPid = $p.Id
[void]$lines.Add("app pid=$appPid hasExited=$($p.HasExited)")
[void]$lines.Add("screen=$([System.Windows.Forms.Screen]::PrimaryScreen.Bounds)")

# find the ball window (Tauri Window, ~136x56, vis=True)
$script:ballHwnd = [IntPtr]::Zero
$script:ballInfo = ''
$cb = [TB+P]{
    param($h, $l)
    $procId = 0
    [void][TB]::GetWindowThreadProcessId($h, [ref]$procId)
    if ($procId -ne $script:appPid) { return $true }
    $r = New-Object TB+R
    [void][TB]::GetWindowRect($h, [ref]$r)
    $w = $r.Rt - $r.L; $hh = $r.B - $r.T
    if ($w -ge 120 -and $w -le 150 -and $hh -ge 50 -and $hh -le 62) {
        $cls = New-Object System.Text.StringBuilder 256
        [void][TB]::GetClassNameW($h, $cls, 256)
        $ex = [TB]::GetWindowLongPtrW($h, -20)
        $script:ballHwnd = $h
        $script:ballInfo = "ball hwnd=0x$($h.ToInt64().ToString('X')) rect=$($r.L),$($r.T) ${w}x${hh} ex=0x$($ex.ToString('X')) cls=$($cls.ToString())"
    }
    return $true
}
[void][TB]::EnumWindows($cb, [IntPtr]::Zero)
[void]$lines.Add($script:ballInfo)

if ($script:ballHwnd -ne [IntPtr]::Zero) {
    [void]$lines.Add("shot baseline -> " + (Shot 'taskbar-1-baseline.png'))

    # (3) DeleteTab WITHOUT HrInit
    try {
        $tl = [TBX]::Create()
        $tl.DeleteTab($script:ballHwnd)
        [void]$lines.Add("DeleteTab(no HrInit) = OK (no exception)")
    } catch {
        [void]$lines.Add(("DeleteTab(no HrInit) FAILED hr=0x{0:X8} msg={1}" -f $_.Exception.HResult, $_.Exception.Message))
    }
    Start-Sleep -Milliseconds 800
    [void]$lines.Add("shot after no-HrInit -> " + (Shot 'taskbar-2-no-hrinit.png'))

    # (4) fresh instance + HrInit + DeleteTab
    try {
        $tl2 = [TBX]::Create()
        $tl2.HrInit()
        [void]$lines.Add("HrInit() = OK")
        $tl2.DeleteTab($script:ballHwnd)
        [void]$lines.Add("DeleteTab(after HrInit) = OK")
    } catch {
        [void]$lines.Add(("HrInit/DeleteTab FAILED hr=0x{0:X8} msg={1}" -f $_.Exception.HResult, $_.Exception.Message))
    }
    Start-Sleep -Milliseconds 800
    [void]$lines.Add("shot after HrInit -> " + (Shot 'taskbar-3-hrinit.png'))
} else {
    [void]$lines.Add("BALL WINDOW NOT FOUND")
}

$lines -join "`r`n" | Out-File -FilePath $log -Encoding UTF8

if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
Get-Process msedgewebview2 -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Write-Output "wrote $log"
