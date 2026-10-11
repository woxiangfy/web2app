$ErrorActionPreference = 'Continue'
$RC = 'C:\dev\rust\web-app\tests\float-frag-rc'
$exe = 'C:\dev\rust\web-app\tests\pack\tauri\osfipin.mark.exe'

$p = Start-Process -FilePath $exe -WorkingDirectory (Split-Path $exe) -PassThru
Start-Sleep -Seconds 18

Write-Output ("app pid={0} hasExited={1}" -f $p.Id, $p.HasExited)

# 截图（用 .NET，避免额外依赖）
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap($b.Width, $b.Height)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
$bmp.Save("$RC\live-full.png", [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output ("saved live-full.png {0}x{1} at ({2},{3})" -f $b.Width, $b.Height, $b.Left, $b.Top)

# 枚举窗口
Write-Output '=== windows ==='
& powershell -NoProfile -ExecutionPolicy Bypass -File "$RC\enumall.ps1"

# 收尾：关掉应用
if (-not $p.HasExited) { $p.CloseMainWindow() | Out-Null; Start-Sleep -Seconds 2 }
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
Get-Process msedgewebview2 -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Write-Output 'done'
