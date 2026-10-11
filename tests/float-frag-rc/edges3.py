from PIL import Image
FILES = {
 "new": r"C:/Users/a03/.workbuddy/clipboard-images/clipboard-2026-10-11T13-29-55-288Z-ceeca50f.png",
 "old": r"C:/Users/a03/.workbuddy/clipboard-images/clipboard-2026-10-11T11-39-41-208Z-ebb693c1.png",
}
def is_blue(c):
    r,g,b=c; return b>120 and (b-r)>70 and (b-g)>25
def is_card(c):
    r,g,b=c; lum=(r*299+g*587+b*114)//1000
    return lum>=168 and 20<=(b-r)<=55
for name,p in FILES.items():
    im=Image.open(p).convert("RGB"); W,H=im.size; px=im.load()
    print(f"\n===== {name} {W}x{H}  (card = lum>=168 and 20<=b-r<=55) =====")
    for y in range(H):
        bl=[x for x in range(W) if is_blue(px[x,y])]
        cd=[x for x in range(W) if is_card(px[x,y])]
        if not bl and not cd: continue
        a=f"{min(bl):>3}..{max(bl):<3}({max(bl)-min(bl)+1:>3})" if bl else "  --       "
        b=f"{min(cd):>3}..{max(cd):<3}({max(cd)-min(cd)+1:>3})" if cd else "   --"
        print(f"{y:>3} | {a} | {b}")
