# 只读取证：量用户现场悬浮球窗的**窗口区域**（SetWindowRgn 设的那个），确认
#   「区域 = 活柱内的圆」且直径 = 活柱边长。
#
# 为什么需要：v1.77b 的根因判定是「球（46 逻辑）比区域圆（56 逻辑）小 ⇒ 中间那圈
# **透明环带**在 Tauri 侧透出浅色残片」。真机截图的几何已经拟合出「残片内边界 = 球边缘、
# 外边界与球同心」，本脚本从**窗口自身的区域**独立复核另一半：区域的包围盒是否
# 等于 活柱边长² 、以及区域是不是一个圆（用 PtInRegion 网格重建）。
#
# 全程只读：EnumWindows / GetWindowRect / GetWindowRgn / GetRgnBox / PtInRegion。
# 输出写到同目录 region-probe.txt（本机 PowerShell stdout 会被吞，必须落盘）。

$ErrorActionPreference = 'Stop'
$out = Join-Path $PSScriptRoot 'region-probe.txt'
Remove-Item $out -ErrorAction SilentlyContinue

Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public class W32 {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern int GetWindowLongPtrW(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint c);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateRectRgn(int l, int t, int r, int b);
  [DllImport("user32.dll")] public static extern int GetWindowRgn(IntPtr h, IntPtr hrgn);
  [DllImport("gdi32.dll")] public static extern int GetRgnBox(IntPtr hrgn, out RECT r);
  [DllImport("gdi32.dll")] public static extern bool PtInRegion(IntPtr hrgn, int x, int y);
  [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr o);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
'@

$lines = New-Object System.Collections.Generic.List[string]
function W($s) { $lines.Add($s) }

$targets = @()
$cb = [W32+EnumProc]{
  param($h, $l)
  $pid_ = 0
  [void][W32]::GetWindowThreadProcessId($h, [ref]$pid_)
  if ($pid_ -eq 9864) {
    $cls = New-Object System.Text.StringBuilder 256
    [void][W32]::GetClassNameW($h, $cls, 256)
    $ttl = New-Object System.Text.StringBuilder 256
    [void][W32]::GetWindowTextW($h, $ttl, 256)
    $r = New-Object W32+RECT
    [void][W32]::GetWindowRect($h, [ref]$r)
    $ex = [W32]::GetWindowLongPtrW($h, -20)
    $script:targets += [pscustomobject]@{
      H = $h; Cls = $cls.ToString(); Title = $ttl.ToString()
      Rect = "$($r.L),$($r.T) $($r.R - $r.L)x$($r.B - $r.T)"
      Ex = ('0x{0:x}' -f $ex); Vis = [W32]::IsWindowVisible($h)
    }
  }
  return $true
}
[void][W32]::EnumWindows($cb, [IntPtr]::Zero)

W "=== pid 9864 的顶层窗口（EnumWindows 顺序 = Z 序，从上到下）==="
foreach ($t in $targets) {
  W ("hwnd={0} vis={1} rect={2} ex={3} cls=[{4}] title=[{5}]" -f $t.H, $t.Vis, $t.Rect, $t.Ex, $t.Cls, $t.Title)
}

# 悬浮球：56 物理高的小窗（矩形高度 ~56）。逐个量它的窗口区域。
foreach ($t in $targets) {
  $m = [regex]::Match($t.Rect, '^(\d+),(\d+) (\d+)x(\d+)$')
  if (-not $m.Success) { continue }
  $w = [int]$m.Groups[3].Value; $h = [int]$m.Groups[4].Value
  if ($h -ne 56 -or $w -lt 56) { continue }

  W ""
  W "=== 区域取证：hwnd=$($t.H) rect=$($t.Rect) ==="
  $rgn = [W32]::CreateRectRgn(0, 0, 0, 0)
  $kind = [W32]::GetWindowRgn($t.H, $rgn)
  if ($kind -le 0) {
    W "GetWindowRgn 返回 $kind（<=0 表示该窗没有区域 / 调用失败）"
    [void][W32]::DeleteObject($rgn)
    continue
  }
  $rb = New-Object W32+RECT
  $rt = [W32]::GetRgnBox($rgn, [ref]$rb)
  W "GetWindowRgn kind=$kind（1=NULLREGION 2=SIMPLEREGION 3=COMPLEXREGION 4=ERROR）"
  W "GetRgnBox=$($rb.L),$($rb.T) .. $($rb.R),$($rb.B)  ⇒ 区域 ${($rb.R)}x$($rb.B)（相对窗口客户区原点）"

  # 网格重建：对区域坐标 0..W/H 逐点 PtInRegion，按行输出 x 起止，即可判断是不是圆。
  $W = [Math]::Max($rb.R + 2, $w); $H = [Math]::Max($rb.B + 2, $h)
  W "PtInRegion 网格（每 4 行一条，列出该行的 x 起止；行首/行尾贴边即代表圆）"
  for ($y = 0; $y -lt $H; $y += 4) {
    $xs = @()
    for ($x = 0; $x -lt $W; $x++) { if ([W32]::PtInRegion($rgn, $x, $y)) { $xs += $x } }
    if ($xs.Count -eq 0) { W ("y={0,4}  -" -f $y); continue }
    W ("y={0,4}  x={1}..{2}  宽={3}" -f $y, $xs[0], $xs[-1], $xs[-1] - $xs[0] + 1)
  }
  [void][W32]::DeleteObject($rgn)
}

[IO.File]::WriteAllLines($out, $lines)
Write-Output "WROTE $out"
