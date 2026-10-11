"""裁出球所在的大范围（物理坐标）并放大。"""
from PIL import Image

SRC = r'C:\dev\rust\web-app\tests\float-frag-rc\tauri-full.png'
im = Image.open(SRC).convert('RGB')
W, H = im.size
# 显示坐标 (875,435) * 缩放 2130/1080 ≈ 1.972 -> 物理 (1725, 858)
cx, cy = 1725, 858
box = (cx - 160, cy - 160, cx + 160, cy + 160)
box = (max(0, box[0]), max(0, box[1]), min(W, box[2]), min(H, box[3]))
print('box', box)
crop = im.crop(box)
crop.resize((crop.width * 3, crop.height * 3), Image.LANCZOS).save(
    r'C:\dev\rust\web-app\tests\float-frag-rc\tauri-ball3.png')
print('saved 3x', crop.size)
