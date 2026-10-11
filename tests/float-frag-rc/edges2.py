# Stricter classifier: the anomaly grey is ~(180,195,215) => lum 150..235 AND 15 <= b-r <= 60.
# Pure white icon (b-r = 0) and the dark bg are both excluded.
from PIL import Image
import os

FILES = {
    "new(13:29)": r"C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T13-29-55-288Z-ceeca50f.png",
    "old(11:39)": r"C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T11-39-41-208Z-ebb693c1.png",
}

def is_blue(c):
    r, g, b = c
    return b > 120 and (b - r) > 70 and (b - g) > 25

def is_grey(c):
    r, g, b = c
    lum = (r * 299 + g * 587 + b * 114) // 1000
    return 140 <= lum <= 240 and 15 <= (b - r) <= 60

for name, p in FILES.items():
    if not os.path.exists(p):
        continue
    im = Image.open(p).convert("RGB")
    W, H = im.size
    px = im.load()
    print(f"\n===== {name}  {W}x{H} =====")
    print("  y | blue L..R (w) | grey L..R (w)  === rows === ")
    gRows = []
    for y in range(H):
        bl = [x for x in range(W) if is_blue(px[x, y])]
        gr = [x for x in range(W) if is_grey(px[x, y])]
        if not gr and not bl:
            continue
        a = f"{min(bl):>3}..{max(bl):<3}({max(bl)-min(bl)+1:>3})" if bl else "   --      "
        if gr:
            print(f"{y:>3} | {a} | {min(gr):>3}..{max(gr):<3}({max(gr)-min(gr)+1:>3})")
            gRows.append(y)
        else:
            print(f"{y:>3} | {a} |    --")
    if gRows:
        # left/right boundary trace of the grey only
        print(f"  grey rows: {min(gRows)}..{max(gRows)}")
        print("  grey boundary trace (y: left..right):")
        trace = []
        for y in gRows:
            gr = [x for x in range(W) if is_grey(px[x, y])]
            trace.append(f"{y}:{min(gr)}..{max(gr)}")
        print("   " + "  ".join(trace))
