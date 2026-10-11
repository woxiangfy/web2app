# 提取残片关键像素值（放大图右侧橙红+蓝小图形区域、卡片边缘、卡片底色）
from PIL import Image

SRC = r'C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T11-39-41-208Z-ebb693c1.png'
im = Image.open(SRC).convert('RGB')
px = im.load()
W, H = im.size

print('== 残片右上区 (x74..95, y18..62) 非黑像素明细 ==')
for y in range(16, 64):
    row = []
    for x in range(72, 96):
        r, g, b = px[x, y]
        if r + g + b > 90:
            row.append('%d,%d,%d' % (r, g, b) + f'@x{x}')
    if row:
        print('y=%2d: %s' % (y, '  '.join(row)))

print()
print('== 残片左上区 (x8..22, y17..34) 非黑像素 ==')
for y in range(17, 34):
    row = []
    for x in range(8, 23):
        r, g, b = px[x, y]
        if r + g + b > 90:
            row.append('%d,%d,%d' % (r, g, b))
    if row:
        print('y=%2d: %s' % (y, '  '.join(row)))
