# Brightness map + hue map: find the grey card without misclassifying it as ball glow.
from PIL import Image

p = r"C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T13-29-55-288Z-ceeca50f.png"
im = Image.open(p).convert("RGB")
W, H = im.size
px = im.load()
print(f"size = {W} x {H}")

# 1) brightness map (0-9). '.' = <20, '9' = >=230
print("\nbrightness map (0-9, '.'=dark<20)")
for y in range(0, H, 2):
    row = []
    for x in range(0, W, 2):
        r, g, b = px[x, y]
        v = (r * 299 + g * 587 + b * 114) // 1000
        if v < 20:
            row.append(".")
        else:
            row.append(str(min(9, v // 26)))
    print("".join(row))

# 2) count pixels in brightness bands
bands = [0] * 10
for y in range(H):
    for x in range(W):
        r, g, b = px[x, y]
        v = (r * 299 + g * 587 + b * 114) // 1000
        bands[min(9, v // 26)] += 1
print("\nbrightness histogram (band i = v in [i*26,(i+1)*26)):")
for i, n in enumerate(bands):
    print(f"  band {i} ({i*26:>3}-{i*26+25:>3}): {n}")

# 3) sample a horizontal scanline through the card area (upper part of the ball)
for yy in (34, 38, 42, 46, 50, 60):
    vals = []
    for x in range(20, 115, 6):
        vals.append(f"{x}:{px[x, yy]}")
    print(f"\ny={yy}: " + "  ".join(vals))
