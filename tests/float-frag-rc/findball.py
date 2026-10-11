"""在 tauri-full.png 里精确定位球（深色圆盘 + 高亮图标），8x 放大取证。"""
from PIL import Image

SRC = r'C:\dev\rust\web-app\tests\float-frag-rc\tauri-full.png'
im = Image.open(SRC).convert('RGB')
W, H = im.size
px = im.load()

# 球：深灰近黑圆盘 + 亮青色眼睛。全屏找"非纯黑"的方块区（桌面是黑的）。
best = None
for y in range(0, H - 40, 8):
    for x in range(W // 2, W - 40, 8):
        # 统计 40x40 里非黑像素
        n = 0
        for dy in range(0, 40, 4):
            for dx in range(0, 40, 4):
                r, g, b = px[x + dx, y + dy]
                if max(r, g, b) > 40:
                    n += 1
        if n > 30:
            if best is None or n > best[0]:
                best = (n, x, y)
print('best 40x40 non-black block:', best)

if best:
    _, x, y = best
    cx, cy = x + 20, y + 20
    print('candidate ball center ~', cx, cy)
    box = (cx - 60, cy - 60, cx + 60, cy + 60)
    crop = im.crop(box)
    crop.resize((crop.width * 8, crop.height * 8), Image.NEAREST).save(
        r'C:\dev\rust\web-app\tests\float-frag-rc\tauri-ball-x8.png')
    print('saved 8x', box)
