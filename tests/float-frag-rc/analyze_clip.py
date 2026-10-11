# 一次性取证：分析悬浮球截图里的「背景异样」残片（v1.77 线索收集）
# 输入：使用者剪贴板截图（含球 + 右侧残片）
# 输出：球的 bbox / 残片的 bbox 与色彩特征 / 边缘直线 or 椭圆裁剪判定
from PIL import Image
import sys

im = Image.open(r'C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T11-39-41-208Z-ebb693c1.png').convert('RGB')
W, H = im.size
px = im.load()
print('image size:', W, H)

def is_blue(c):
    r, g, b = c
    return b > 120 and b > r + 30 and b >= g

def is_whitish(c):
    r, g, b = c
    return r > 180 and g > 180 and b > 180 and max(r,g,b)-min(r,g,b) < 60

# 1) 蓝色球 bbox
xs, ys = [], []
for y in range(H):
    for x in range(W):
        if is_blue(px[x, y]):
            xs.append(x); ys.append(y)
if xs:
    print('blue ball bbox: x[%d..%d] y[%d..%d]  (w=%d h=%d)' % (min(xs), max(xs), min(ys), max(ys), max(xs)-min(xs)+1, max(ys)-min(ys)+1))
else:
    print('no blue pixels found')

bx0, bx1, by0, by1 = (min(xs), max(xs), min(ys), max(ys)) if xs else (0, W-1, 0, H-1)

# 2) 残片：球 bbox 右侧且非蓝非白的非背景像素
# 先看整图右半部的颜色分布（量化）
from collections import Counter
cnt = Counter()
for y in range(H):
    for x in range(W):
        cnt[px[x, y]] += 1
print('top colors:', cnt.most_common(8))

# 找出「浅色但非纯背景」的像素（残片应为 (184,197,208) 一类浅蓝灰，v1.73 取证值附近）
def is_fragment(c):
    r, g, b = c
    # 浅蓝灰：亮度高但蓝分量略高于红（白×深蓝投影 ≈ (183,199,217)）
    return 150 < r < 235 and 165 < g < 240 and 190 < b < 255 and b > r + 8 and b > g - 5

fxs, fys = [], []
for y in range(H):
    for x in range(W):
        if is_fragment(px[x, y]):
            fxs.append(x); fys.append(y)
if fxs:
    print('fragment-ish bbox: x[%d..%d] y[%d..%d] (w=%d h=%d, n=%d)' % (min(fxs), max(fxs), min(fys), max(fys), max(fxs)-min(fxs)+1, max(fys)-min(fys)+1, len(fxs)))
else:
    print('no fragment-ish pixels')

# 3) 逐行打印残片区间（判断左边缘是弧还是直线、右边缘是否齐直）
print('--- per-row fragment x-range (every 2 rows) ---')
for y in range(0, H, 2):
    row = [x for x in range(W) if is_fragment(px[x, y])]
    if row:
        print('y=%3d  x[%3d..%3d]  n=%3d' % (y, min(row), max(row), len(row)))

# 4) 残片内的深色像素（文字碎片?）
dark = [(x, y) for y in range(H) for x in range(W)
        if is_fragment(px[x, y]) and sum(px[x, y]) < 420]
print('dark pixels inside fragment region:', len(dark))
if dark:
    dxs = [p[0] for p in dark]; dys = [p[1] for p in dark]
    print('  dark bbox: x[%d..%d] y[%d..%d]' % (min(dxs), max(dxs), min(dys), max(dys)))
