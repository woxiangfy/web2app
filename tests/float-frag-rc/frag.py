"""高倍放大残片局部 + 逐像素 hex 网格，定位蓝/橙色小图形。"""
from PIL import Image
from collections import Counter

SRC = r'C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T11-39-41-208Z-ebb693c1.png'
im = Image.open(SRC).convert('RGB')
px = im.load()

# 饱和 / 非蓝非灰的彩色像素统计
cnt = Counter()
for y in range(im.height):
    for x in range(im.width):
        r, g, b = px[x, y]
        mx, mn = max(r, g, b), min(r, g, b)
        if mx > 60 and (mx - mn) > 30 and not (b >= r and b >= g):
            cnt[(r // 16 * 16, g // 16 * 16, b // 16 * 16)] += 1
print('=== 非蓝色调的有色像素（16 级分桶）top10 ===')
for c, n in cnt.most_common(10):
    print(f'  rgb~{c} n={n}')

# 20x 放大右缘局部
box = (58, 22, 90, 62)
c = im.crop(box)
c.resize((c.width * 20, c.height * 20), Image.NEAREST).save(
    r'C:\dev\rust\web-app\tests\float-frag-rc\frag-x20.png')
print('saved frag-x20.png', box)

print('=== hex 网格 (x=70..90, y=28..58) ===')
hdr = '      ' + ''.join(f'{x:>8}' for x in range(70, 91, 2))
print(hdr)
for y in range(28, 59, 2):
    row = f'y={y:3d} ' + ''.join(f'{px[x,y][0]:02x}{px[x,y][1]:02x}{px[x,y][2]:02x}  ' for x in range(70, 91, 2))
    print(row)
