from PIL import Image
SRC = r'C:\dev\rust\web-app\tests\float-frag-rc\live-full.png'
im = Image.open(SRC).convert('RGB')
print('size', im.size)
W, H = im.size
box = (W - 200, H - 140, W, H)
c = im.crop(box)
c.resize((c.width * 6, c.height * 6), Image.NEAREST).save(
    r'C:\dev\rust\web-app\tests\float-frag-rc\live-ball.png')
print('saved', box)
