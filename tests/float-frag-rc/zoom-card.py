# Crop the anomaly region from the user's screenshot and upscale it for identification.
from PIL import Image, ImageEnhance

p = r"C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T13-29-55-288Z-ceeca50f.png"
im = Image.open(p).convert("RGB")

crops = {
    "zoom-card-8x.png": ((24, 28, 110, 74), 8),
    "zoom-full-6x.png": ((20, 24, 115, 110), 6),
}
for name, (box, k) in crops.items():
    c = im.crop(box)
    c = c.resize((c.width * k, c.height * k), Image.NEAREST)
    c.save(r"C:\dev\rust\web-app\tests\float-frag-rc\\" + name)
    print(f"{name}: crop={box} scale={k} -> {c.size}")

# contrast-boosted version of the card area (levels stretched) to reveal structure
c = im.crop((24, 28, 110, 74))
c = ImageEnhance.Contrast(c).enhance(3.0)
c = c.resize((c.width * 8, c.height * 8), Image.NEAREST)
c.save(r"C:\dev\rust\web-app\tests\float-frag-rc\zoom-card-contrast-8x.png")
print("zoom-card-contrast-8x.png written")

# classify: for the card band, print a fine RGB grid
px = im.load()
print("\nRGB grid, x 28..104 step 4, y 30..68 step 4 (only pixels OUTSIDE the blue ball)")
for y in range(30, 69, 4):
    row = []
    for x in range(28, 105, 4):
        r, g, b = px[x, y]
        row.append(f"{x:>3}:{r:>3},{g:>3},{b:>3}")
    print(f"y={y:>3} " + " | ".join(row))
