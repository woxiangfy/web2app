# Row-by-row boundaries: for each row, find the blue-ball extent and the grey extent.
from PIL import Image
import os

FILES = {
    "new(13:29)": r"C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T13-29-55-288Z-ceeca50f.png",
    "old(11:39)": r"C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T11-39-41-208Z-ebb693c1.png",
}

def is_blue(c):
    r, g, b = c
    return b > 120 and (b - r) > 35 and (b - g) > 20

def is_grey(c):
    # light, low-saturation-ish but slightly blue: the "card"
    r, g, b = c
    lum = (r * 299 + g * 587 + b * 114) // 1000
    return lum > 120 and abs(r - g) < 40 and (b - r) < 60 and (b - r) > 0

for name, p in FILES.items():
    if not os.path.exists(p):
        print(f"{name}: missing {p}")
        continue
    im = Image.open(p).convert("RGB")
    W, H = im.size
    px = im.load()
    print(f"\n===== {name}  {W}x{H} =====")
    print(" y | blue  L..R  (w) | grey  L..R  (w)  [grey pixels outside blue]")
    rows = []
    for y in range(H):
        bl = [x for x in range(W) if is_blue(px[x, y])]
        gr = [x for x in range(W) if is_grey(px[x, y])]
        if not bl and not gr:
            continue
        blr = f"{min(bl):>3}..{max(bl):<3} ({max(bl)-min(bl)+1:>3})" if bl else "   -        "
        if gr:
            gl, grr = min(gr), max(gr)
            outside = [x for x in gr if not (bl and min(bl) <= x <= max(bl))]
            outr = f"out={min(outside)}..{max(outside)}" if outside else "out=-"
            grr_s = f"{gl:>3}..{grr:<3} ({grr-gl+1:>3}) {outr}"
        else:
            grr_s = "   -        "
        rows.append((y, blr, grr_s))
    for y, a, b in rows:
        print(f"{y:>3} | {a} | {b}")
    # summary
    allb = [y for y, a, b in rows if "(" in a]
    allg = [y for y, a, b in rows if b != "   -        "]
    if allb:
        print(f"   blue rows : {min(allb)}..{max(allb)}  ({max(allb)-min(allb)+1} rows)")
    if allg:
        print(f"   grey rows : {min(allg)}..{max(allg)}  ({max(allg)-min(allg)+1} rows)")
