# 取证驱动：真实鼠标三态对照（移开 -> hover -> 再移开 -> hover后点击再移开）
# 进程先声明 DPI 感知，SetCursorPos 用物理像素；ImageGrab 全屏抓后裁剪。
import ctypes, ctypes.wintypes as wt, time, sys
from PIL import ImageGrab

# per-monitor v2 DPI 感知（物理像素坐标系）
try:
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)

user32 = ctypes.windll.user32
sx = user32.GetSystemMetrics(0)  # 主屏物理宽
sy = user32.GetSystemMetrics(1)

# 球窗 DIP (600,400) 56x56 —— 物理 = DIP * scale；用 GetDpiForSystem 近似拿不到 per-monitor，
# 直接用监控点换算更稳：先拿主屏 DPI
class MONITORINFO(ctypes.Structure):
    _fields_ = [("cbSize", ctypes.c_uint), ("rcMonitor", wt.RECT), ("rcWork", wt.RECT),
                ("dwFlags", ctypes.c_uint)]
class POINT(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]
pt = POINT(0, 0)
hmon = user32.MonitorFromPoint(ctypes.byref(pt), 1)  # MONITOR_DEFAULTTOPRIMARY
mi = MONITORINFO(); mi.cbSize = ctypes.sizeof(mi)
user32.GetMonitorInfoW(hmon, ctypes.byref(mi))
hdc = ctypes.windll.user32.GetDC(0)
LOGPIXELSX = 88
dpi = ctypes.windll.gdi32.GetDeviceCaps(hdc, LOGPIXELSX)
ctypes.windll.user32.ReleaseDC(0, hdc)
scale = dpi / 96.0
print('screen phys: %dx%d dpi=%.0f scale=%.2f' % (sx, sy, dpi, scale))

# 球心 DIP (600+28, 400+28) -> 物理
bx = int((600 + 28) * scale)
by = int((400 + 28) * scale)
far_x, far_y = 60, int(sy * 0.85)  # 远离球的角落

OUT = r'C:\dev\rust\web-app\tests\float-frag-rc'

def grab(name):
    time.sleep(0.3)
    im = ImageGrab.grab(bbox=None, all_screens=True)
    im.save(OUT + '\\' + name + '.png')
    print('saved', name, im.size)

def move(x, y):
    user32.SetCursorPos(int(x), int(y))

# ① 初始：光标远离
move(far_x, far_y)
time.sleep(1.5)
grab('s1-nohover')

# ② 悬停球心，等原生提示出现（Chromium 默认 ~0.5-1s；留足 4s）
move(bx, by)
time.sleep(4.0)
grab('s2-hover')

# ③ 移开（悬停后自然移开 —— 观察 tooltip 是否隐藏、有无残留）
move(far_x, far_y)
time.sleep(2.5)
grab('s3-after-leave')

# ④ 再悬停 -> 短暂停留 -> 移开一小段（模拟「看过提示后又操作」）
move(bx, by)
time.sleep(2.0)
move(bx + int(90 * scale), by)  # 移到球右侧近处（约 90 DIP 外）
time.sleep(1.5)
grab('s4-near-right')

# ⑤ 悬停球心 -> 按下左键（模拟点击主键而不松）-> 移开（模拟拖动后松手在别处）
move(bx, by)
time.sleep(1.5)
user32.mouse_event(2, 0, 0, 0, 0)   # down
time.sleep(0.2)
user32.mouse_event(4, 0, 0, 0, 0)   # up（点击球：探针窗无主窗可切，无副作用）
time.sleep(0.5)
move(far_x, far_y)
time.sleep(2.0)
grab('s5-after-click-leave')

print('DONE')
