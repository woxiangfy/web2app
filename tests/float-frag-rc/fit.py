"""逐行测球体边界与浅色环带外边界，拟合圆心/半径。"""
from PIL import Image
import math

SRC = r'C:\Users\a03\.workbuddy\clipboard-images\clipboard-2026-10-11T11-39-41-208Z-ebb693c1.png'
im = Image.open(SRC).convert('RGB')
W, H = im.size
px = im.load()

def is_lightish(r, g, b):
    return min(r, g, b) >= 120 and max(r, g, b) - min(r, g, b) < 70 and b >= r

def is_ball(r, g, b):
    return b > 120 and b - r > 30 and b - g > 20

rows = []
for y in range(H):
    bx = [x for x in range(W) if is_ball(*px[x, y])]
    lx = [x for x in range(W) if is_lightish(*px[x, y])]
    if bx or lx:
        rows.append((y,
                     (min(bx), max(bx)) if bx else None,
                     (min(lx), max(lx)) if lx else None))

print('y | ball[x0,x1] | light[x0,x1]')
for y, b, l in rows:
    print(f'{y:3d} | {b} | {l}')

# 用最小二乘拟合球体边界圆
pts = []
for y, b, l in rows:
    if b and 24 < y < 76:
        pts.append((b[1], y))   # 右侧边界点
n = len(pts)
sx = sum(p[0] for p in pts); sy = sum(p[1] for p in pts)
sxx = sum(p[0]**2 for p in pts); syy = sum(p[1]**2 for p in pts)
sxy = sum(p[0]*p[1] for p in pts)
sxz = sum(p[0]*(p[0]**2+p[1]**2) for p in pts); syz = sum(p[1]*(p[0]**2+p[1]**2) for p in pts)
sz = sum(p[0]**2+p[1]**2 for p in pts)
# 解 A*x + B*y + C = x^2+y^2
A = [[sxx, sxy, sx], [sxy, syy, sy], [sx, sy, n]]
rhs = [sxz, syz, sz]
# 高斯消元
for i in range(3):
    p = max(range(i, 3), key=lambda r: abs(A[r][i])); A[i], A[p] = A[p], A[i]; rhs[i], rhs[p] = rhs[p], rhs[i]
    for r in range(i+1, 3):
        f = A[r][i]/A[i][i]
        for c in range(i, 3): A[r][c] -= f*A[i][c]
        rhs[r] -= f*rhs[i]
a = rhs[2]/A[2][2]
bcoef = (rhs[1]-A[1][2]*a)/A[1][1]
c = (rhs[0]-A[0][1]*bcoef-A[0][2]*a)/A[0][0]
cx, cy = a/2, bcoef/2
R = math.sqrt(cx*cx+cy*cy-c)
print(f'\n球体边界圆: center=({cx:.1f},{cy:.1f}) R={R:.1f}  (n={n})')
