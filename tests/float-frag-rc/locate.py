"""定位全屏截图中的浅色残片（纯 Python，无 numpy）。"""
from PIL import Image
from collections import Counter

SRC = r'C:\dev\rust\web-app\tests\float-frag-rc\tauri-full.png'
im = Image.open(SRC).convert('RGB')
W, H = im.size
px = im.load()

cells = Counter()
total = 0
for y in range(0, H, 2):
    for x in range(W // 2, W, 2):
        r, g, b = px[x, y]
        lo, hi = min(r, g, b), max(r, g, b)
        if lo > 190 and (hi - lo) < 32:
            total += 1
            cells[(x // 40, y // 40)] += 1

print('size', W, H, 'bright(low-sat) px (sampled)', total)
for (gx, gy), n in cells.most_common(10):
    print(f'  cell x={gx*40}-{gx*40+40} y={gy*40}-{gy*40+40} n={n}')
