# 启动应用 → 等待 → 枚举窗口 + 截图 → 收尾
$RC = 'C:\dev\rust\web-app\tests\float-frag-rc'
$exe = 'C:\dev\rust\web-app\tests\pack\tauri\osfipin.mark.exe'

$p = Start-Process -FilePath $exe -WorkingDirectory (Split-Path $exe) -PassThru
Start-Sleep -Seconds 20
Write-Output ("app pid={0} hasExited={1}" -f $p.Id, $p.HasExited)

& powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $RC 'enum2.ps1')

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap($b.Width, $b.Height)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
$bmp.Save("$RC\live2-full.png", [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output ("saved live2-full.png {0}x{1}" -f $b.Width, $b.Height)

if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
Get-Process msedgewebview2 -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Write-Output 'done'
