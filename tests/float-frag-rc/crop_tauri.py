"""从 Tauri 全屏截图中裁出球区域并放大，做残片取证。"""
from PIL import Image

SRC = r'C:\dev\rust\web-app\tests\float-frag-rc\tauri-full.png'
im = Image.open(SRC).convert('RGB')
print('full size', im.size)

# 球在整屏中的大致位置（肉眼估）——先裁一个大范围
# 整屏宽度先打印出来，用于按比例定位
W, H = im.size
# 球目测在 (875, 435) 附近（以当前显示分辨率为准）
cx, cy = 875, 435
# 若截图已被缩放，按比例换算
# 先直接裁
box = (cx - 80, cy - 80, cx + 80, cy + 80)
box = (max(0, box[0]), max(0, box[1]), min(W, box[2]), min(H, box[3]))
crop = im.crop(box)
print('crop box', box, 'crop size', crop.size)
crop = crop.resize((crop.width * 6, crop.height * 6), Image.NEAREST)
crop.save(r'C:\dev\rust\web-app\tests\float-frag-rc\tauri-ball-zoom.png')
print('saved zoom')

# 同时裁一个更宽的上下文
box2 = (cx - 200, cy - 200, cx + 200, cy + 200)
box2 = (max(0, box2[0]), max(0, box2[1]), min(W, box2[2]), min(H, box2[3]))
crop2 = im.crop(box2)
crop2 = crop2.resize((crop2.width * 2, crop2.height * 2), Image.LANCZOS)
crop2.save(r'C:\dev\rust\web-app\tests\float-frag-rc\tauri-ball-ctx.png')
print('saved ctx', box2)
