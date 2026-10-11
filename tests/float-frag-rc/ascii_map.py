# 放大截图 + ASCII 分类图
from PIL import Image

SRC = r'C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T11-39-41-208Z-ebb693c1.png'
im = Image.open(SRC).convert('RGB')
W, H = im.size
px = im.load()

def classify(c):
    r, g, b = c
    if r < 40 and g < 40 and b < 40: return '.'      # 近黑背景
    if b > 120 and b > r + 30 and b >= g: return 'B' # 蓝（球）
    if r > 235 and g > 235 and b > 235: return 'W'   # 白
    if b > r + 8 and r > 140: return 'f'             # 浅蓝灰（残片候选）
    if r > 100 or g > 100 or b > 100: return 'o'     # 其他亮色
    return ','                                       # 暗色非纯黑

for y in range(H):
    print('%2d %s' % (y, ''.join(classify(px[x, y]) for x in range(W))))

up = im.resize((W * 8, H * 8), Image.NEAREST)
up.save(r'C:\dev\rust\web-app\tests\float-frag-rc\clip-x8.png')
print('saved x8')
