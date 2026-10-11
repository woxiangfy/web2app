# 裁剪 5 态截图的球区域并拼成对比大图
from PIL import Image
OUT = r'C:\dev\rust\web-app\tests\float-frag-rc'
# 球 DIP(600,400) 56x56, scale 1.25 -> phys (750,500)-(820,570)。裁 generous 区域
L, T, R, B = 680, 430, 1000, 680
names = ['s1-nohover', 's2-hover', 's3-after-leave', 's4-near-right', 's5-after-click-leave']
crops = []
for n in names:
    im = Image.open(OUT + '\\' + n + '.png').convert('RGB')
    c = im.crop((L, T, R, B))
    crops.append((n, c))

w, h = crops[0][1].size
S = 3
sheet = Image.new('RGB', ((w * S + 12) * len(crops), h * S + 40), (24, 24, 24))
from PIL import ImageDraw
d = ImageDraw.Draw(sheet)
for i, (n, c) in enumerate(crops):
    cc = c.resize((w * S, h * S), Image.NEAREST)
    x = i * (w * S + 12)
    sheet.paste(cc, (x, 32))
    d.text((x + 6, 8), '%d:%s' % (i + 1, n), fill=(255, 255, 255))
sheet.save(OUT + '\\compare.png')
print('saved compare.png', sheet.size)
