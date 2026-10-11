from PIL import Image
SRC = r'C:\dev\rust\web-app\tests\float-frag-rc\tauri-full.png'
im = Image.open(SRC).convert('RGB')
cx, cy = 1715, 864
box = (cx - 70, cy - 70, cx + 70, cy + 70)
c = im.crop(box)
c.resize((c.width * 6, c.height * 6), Image.NEAREST).save(
    r'C:\dev\rust\web-app\tests\float-frag-rc\tauri-ball-x6.png')
print('saved', box, c.size)
