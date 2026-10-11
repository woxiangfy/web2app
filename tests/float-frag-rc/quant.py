"""精确量化用户截图：分类像素并输出各元素的包围盒。"""
from PIL import Image

SRC = r'C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T11-39-41-208Z-ebb693c1.png'
im = Image.open(SRC).convert('RGB')
W, H = im.size
px = im.load()
print('size', W, H)

def cls(r, g, b):
    lo, hi = min(r, g, b), max(r, g, b)
    # 蓝球：蓝通道显著大于红/绿
    if b > 120 and b - r > 35 and b - g > 25:
        return 'BALL_BLUE'
    # 白色元素（书签图标）
    if lo > 225 and hi - lo < 22:
        return 'WHITE'
    # 浅灰卡片
    if 170 <= lo and hi - lo < 26:
        return 'CARD'
    # 暗/透明
    if hi < 80:
        return 'DARK'
    return 'OTHER'

boxes = {}
counts = {}
for y in range(H):
    for x in range(W):
        c = cls(*px[x, y])
        counts[c] = counts.get(c, 0) + 1
        if c not in ('DARK', 'OTHER'):
            x0, y0, x1, y1 = boxes.get(c, (x, y, x, y))
            boxes[c] = (min(x0, x), min(y0, y), max(x1, x), max(y1, y))

for c, n in sorted(counts.items(), key=lambda kv: -kv[1]):
    bb = boxes.get(c)
    print(f'{c:10s} n={n:5d} bbox={bb}')

# 打印一张 ASCII 分类图（每 2px 采样）
for y in range(0, H, 2):
    row = ''
    for x in range(0, W, 2):
        c = cls(*px[x, y])
        row += {'BALL_BLUE': 'B', 'WHITE': 'W', 'CARD': 'c', 'DARK': '.', 'OTHER': '?'}[c]
    print(row)
