import sys,re,math
def rr(m):
    x,y,w,h,r=[float(v) for v in m.group(1).split(',')]
    r=min(r,w/2,h/2); pts=[]
    for cx,cy,a0 in ((x+w-r,y+r,-90),(x+w-r,y+h-r,0),(x+r,y+h-r,90),(x+r,y+r,180)):
        for i in range(7):
            a=math.radians(a0+i*15); pts.append(f"{cx+r*math.cos(a):.2f},{cy+r*math.sin(a):.2f}")
    return "poly("+";".join(pts)+")"
print(re.sub(r'rrect\(([^)]*)\)',rr,sys.stdin.read()),end='')
