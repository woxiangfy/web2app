"""10 倍放大用户截图，逐像素看清残片；并打印右缘像素原始 RGB。"""
from PIL import Image

SRC = r'C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T11-39-41-208Z-ebb693c1.png'
im = Image.open(SRC).convert('RGB')
W, H = im.size
im.resize((W * 10, H * 10), Image.NEAREST).save(
    r'C:\dev\rust\web-app\tests\float-frag-rc\clip-x10.png')

px = im.load()
print('=== 右缘区域 (x=62..95, y=12..64) 原始 RGB ===')
for y in range(12, 65, 2):
    row = []
    for x in range(62, 96, 2):
        r, g, b = px[x, y]
        row.append(f'{r:3d},{g:3d},{b:3d}')
    print(f'y={y:3d} ' + ' | '.join(row))
