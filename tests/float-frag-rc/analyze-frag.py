#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""对用户真机截图做逐像素分类，确定「残片」的几何形状。

分类：
  B = 球本体（蓝色系：b - r 大）
  W = 浅色残片（很亮、低饱和、偏中性灰白）—— 即「背景异样」的那块
  . = 背景（深色桌面 / 截图留白，亮度低）
  w = 白色图标（球内 icon，高亮且被球包围）
"""
import sys
from PIL import Image

path = sys.argv[1]
im = Image.open(path).convert("RGB")
W, H = im.size
print(f"image {path}  size={W}x{H}")

px = im.load()


def classify(r, g, b):
    lum = (r * 299 + g * 587 + b * 114) // 1000
    if b - r > 25 and lum > 60:
        return "B"          # 球：蓝多于红
    if lum >= 160:
        # 亮：是残片还是球内图标？图标被球包围，稍后按连通性区分；先统一 W
        return "W"
    if r > 205 and g > 205 and b > 205:
        return "W"
    return "."


grid = [[classify(*px[x, y]) for x in range(W)] for y in range(H)]

# 残片（W）包围盒 + 球（B）包围盒
def bbox(ch):
    xs = [x for y in range(H) for x in range(W) if grid[y][x] == ch]
    ys = [y for y in range(H) for x in range(W) if grid[y][x] == ch]
    if not xs:
        return None
    return (min(xs), min(ys), max(xs), max(ys), len(xs))

print("BBox B(球) =", bbox("B"))
print("BBox W(亮) =", bbox("W"))

bx = bbox("B")
if bx:
    cx = (bx[0] + bx[2]) / 2
    cy = (bx[1] + bx[3]) / 2
    print(f"球心 = ({cx:.1f}, {cy:.1f})  球径 = {bx[2]-bx[0]+1} x {bx[3]-bx[1]+1}")
    rad = ((bx[2] - bx[0]) + (bx[3] - bx[1])) / 4
    print(f"球半径 ≈ {rad:.1f}")
    # 对每个 W 像素算到球心的距离，看是否恒定（恒定=环带；不恒定=别的形状）
    ds = []
    for y in range(H):
        for x in range(W):
            if grid[y][x] == "W":
                d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
                ds.append((round(d), x, y))
    if ds:
        ds.sort()
        print("W 像素到球心距离：min=%d  max=%d" % (ds[0][0], ds[-1][0]))
        from collections import Counter
        c = Counter(d for d, _, _ in ds)
        print("距离直方图（前 25 档）：", sorted(c.items())[:25])
        # 按极角看是否只在某一侧
        import math
        ang = Counter()
        for d, x, y in ds:
            a = int(math.degrees(math.atan2(y - cy, x - cx)) // 30) * 30
            ang[a] += 1
        print("W 像素极角分布（每 30°）：", sorted(ang.items()))

print()
print("ASCII 图（每字符 1 像素；W=亮残片 B=球 .=背景）：")
for y in range(H):
    print("".join(grid[y]))
