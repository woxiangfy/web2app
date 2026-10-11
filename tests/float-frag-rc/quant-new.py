# Quantify the new screenshot: size, ball bbox, "card" bbox, RGB grid, ASCII map.
import sys
from PIL import Image

p = r"C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T13-29-55-288Z-ceeca50f.png"
im = Image.open(p).convert("RGB")
W, H = im.size
print(f"size = {W} x {H}")

px = im.load()

def is_blue(c):
    r, g, b = c
    return b > 110 and b - r > 40 and b - g > 25

def is_ball_bg(c):
    # the soft dark-blue glow around the ball
    r, g, b = c
    return b > 60 and b - r > 25 and b - g > 10

def is_dark(c):
    r, g, b = c
    return r < 45 and g < 45 and b < 45

ball = []
card = []
for y in range(H):
    for x in range(W):
        c = px[x, y]
        if is_ball_bg(c):
            ball.append((x, y))
        elif not is_dark(c) and not is_blue(c):
            card.append((x, y))

def bbox(pts):
    if not pts:
        return None
    xs = [q[0] for q in pts]; ys = [q[1] for q in pts]
    return (min(xs), min(ys), max(xs), max(ys))

bb_ball = bbox(ball)
bb_card = bbox(card)
print(f"ball(blue incl. glow) bbox = {bb_ball}  n={len(ball)}")
print(f"card(light non-blue)  bbox = {bb_card}  n={len(card)}")

# ASCII classified map (downsample x2)
print("\nASCII map  B=blue-ish  C=card(light)  .=dark bg")
step = 2
for y in range(0, H, step):
    row = []
    for x in range(0, W, step):
        c = px[x, y]
        if is_blue(c) or is_ball_bg(c):
            row.append("B")
        elif is_dark(c):
            row.append(".")
        else:
            row.append("C")
    print("".join(row))

# grey histogram restricted to the card bbox, excluding the ball's own pixels
if bb_card:
    x0, y0, x1, y1 = bb_card
    from collections import Counter
    cnt = Counter()
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            c = px[x, y]
            if is_ball_bg(c):
                continue
            cnt[(c[0] // 16 * 16, c[1] // 16 * 16, c[2] // 16 * 16)] += 1
    print("\ncard-area color buckets (16-quantized, top 12):")
    for c, n in cnt.most_common(12):
        print(f"  rgb~{c}  n={n}")
