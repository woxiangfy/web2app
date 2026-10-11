# 球体特写对比：s1 vs s2 vs s3（物理 750,500-820,570 ± 40px）
from PIL import Image, ImageDraw
OUT = r'C:\dev\rust\web-app\tests\float-frag-rc'
L, T, R, B = 700, 460, 880, 610
names = ['s1-nohover', 's2-hover', 's3-after-leave', 's4-near-right', 's5-after-click-leave']
crops = []
for n in names:
    im = Image.open(OUT + '\\' + n + '.png').convert('RGB')
    crops.append((n, im.crop((L, T, R, B))))
w, h = crops[0][1].size
S = 6
sheet = Image.new('RGB', ((w * S + 14) * len(crops), h * S + 40), (24, 24, 24))
d = ImageDraw.Draw(sheet)
for i, (n, c) in enumerate(crops):
    cc = c.resize((w * S, h * S), Image.NEAREST)
    x = i * (w * S + 14)
    sheet.paste(cc, (x, 34))
    d.text((x + 6, 10), '%d:%s' % (i + 1, n), fill=(255, 255, 255))
sheet.save(OUT + '\\zoom.png')
print('saved', sheet.size)

# 逐像素 diff：s1 vs s2（hover 应有 scale 效果或 tooltip）
import itertools
a = Image.open(OUT + '\\s1-nohover.png').convert('RGB').crop((L, T, R, B))
b = Image.open(OUT + '\\s2-hover.png').convert('RGB').crop((L, T, R, B))
pa, pb = a.load(), b.load()
diff = [(x + L, y + T) for y in range(B - T) for x in range(R - L) if pa[x, y] != pb[x, y]]
print('s1 vs s2 diff pixels:', len(diff))
if diff:
    xs = [p[0] for p in diff]; ys = [p[1] for p in diff]
    print('  diff bbox: x[%d..%d] y[%d..%d]' % (min(xs), max(xs), min(ys), max(ys)))
