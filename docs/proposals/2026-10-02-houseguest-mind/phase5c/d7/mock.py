# True-size mock of D7 treatments: TV (6x4 cells at 9x19 px = 54x76) on the room background.
import base64, io, re, subprocess, sys, os
from PIL import Image, ImageEnhance, ImageFilter, ImageOps
import numpy as np
D = os.path.dirname(os.path.abspath(__file__))
PROPS = open('/home/svein/dev/dessplay/dessplay/src/ui/houseguest/art/props.svg').read()
inner = PROPS
BG = (30, 33, 39)
W, H = 54, 76           # TV box px at 9x19
S = 0.45                # rasterize scale (art.rs rasterize: min(54/120, 76/168))
DY = H - 168 * S        # bottom-aligned
GX0, GY0, GW, GH = 26, 72, 58, 46
# glass in px, snapped outward to whole pixels
px0, py0 = int(GX0*S), int(GY0*S+DY)
px1, py1 = int(np.ceil((GX0+GW)*S)), int(np.ceil((GY0+GH)*S+DY))
PW, PH = px1-px0, py1-py0
print('glass px', (GX0*S, GY0*S+DY, (GX0+GW)*S, (GY0+GH)*S+DY), 'snapped', (px0,py0,PW,PH))

def crop_aspect(im, aspect, zoom=1.0):
    w, h = im.size
    if w/h > aspect: cw, ch = h*aspect, h
    else: cw, ch = w, w/aspect
    cw, ch = cw/zoom, ch/zoom
    l, t = (w-cw)/2, (h-ch)/2
    return im.crop((int(l), int(t), int(l+cw), int(t+ch)))

def letterbox(im):
    w, h = im.size
    th = round(PW * h / w)
    out = Image.new('RGB', (PW, PH), (10, 10, 12))
    out.paste(im.resize((PW, th), Image.BOX), (0, (PH-th)//2))
    return out

def plain(im, zoom=1.0):
    return crop_aspect(im, PW/PH, zoom).resize((PW, PH), Image.BOX)

def poster(im, n=8, zoom=1.0):
    small = plain(im, zoom)
    small = ImageEnhance.Color(small).enhance(1.25)
    small = ImageEnhance.Contrast(small).enhance(1.1)
    return small.quantize(n, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE).convert('RGB')

def crt(im):
    a = np.asarray(im).astype(float)
    a[1::2] *= 0.82                      # scanlines
    a = a*0.92 + np.array([8, 14, 22])*1.0  # cool cast
    return Image.fromarray(a.clip(0,255).astype('uint8'))

def sheen_layer():
    svg = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 168" width="{W}" height="{round(168*S)}"><path d="M 32 80 C 38 76 46 75 52 76" fill="none" stroke="#ffffff" stroke-opacity="0.55" stroke-width="2" stroke-linecap="round"/></svg>'
    p = os.path.join(D, '_sh.svg'); open(p, 'w').write(svg)
    subprocess.run(['resvg', p, os.path.join(D, '_sh.png')], check=True)
    t = Image.open(os.path.join(D, '_sh.png')).convert('RGBA')
    out = Image.new('RGBA', (W, H), (0,0,0,0)); out.alpha_composite(t, (0, H - t.size[1])); return out

def lifted(im, zoom=1.3):
    small = plain(im, zoom)
    # one linear stretch for all channels, from luma's 2nd/98th percentiles (keeps hue)
    a = np.asarray(small).astype(float)
    y = a @ np.array([0.299, 0.587, 0.114])
    lo, hi = np.percentile(y, 2), np.percentile(y, 98)
    gain = min(255.0 / max(hi - lo, 1.0), 3.0)
    small = Image.fromarray(((a - lo) * gain).clip(0, 255).astype('uint8'))
    return ImageEnhance.Color(small).enhance(1.3)

def tv(picture=None, card=None, sheen=False):
    """Render the TV at 54x76; picture (PW x PH) pasted into the glass by mask; card = drawn programme id."""
    pic = f'<use href="#{card}"/>' if card else '<rect x="26" y="72" width="58" height="46" fill="#ff00ff"/>'
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 168" width="{W}" height="{round(168*S)}" color="#1d1714">{inner}
<defs><clipPath id="g"><rect x="26" y="72" width="58" height="46" rx="8"/></clipPath></defs>
<use href="#tv"/><g clip-path="url(#g)">{pic}</g>
<rect x="26" y="72" width="58" height="46" rx="8" fill="none" stroke="currentColor" stroke-width="1.6"/></svg>'''
    p = os.path.join(D, '_tv.svg'); open(p, 'w').write(svg)
    subprocess.run(['resvg', p, os.path.join(D, '_tv.png')], check=True)
    t = Image.open(os.path.join(D, '_tv.png')).convert('RGBA')
    out = Image.new('RGBA', (W, H), BG + (255,))
    out.alpha_composite(t, (0, H - t.size[1]))
    if picture is not None:
        # magenta-ish pixels are glass; blend picture by magenta coverage
        a = np.asarray(out).astype(float)
        m = np.clip(((a[...,0] + a[...,2])/2 - a[...,1]) / 255.0, 0, 1)  # magenta strength
        canvas = np.array(Image.new('RGB', (W, H), (0,0,0))).astype(float)
        canvas[py0:py0+PH, px0:px0+PW] = np.asarray(picture.convert('RGB')).astype(float)
        a[..., :3] = a[..., :3]*(1-m[...,None]) + canvas*m[...,None]
        out = Image.fromarray(a.clip(0,255).astype('uint8'))
    if sheen:
        out = out.convert('RGBA'); out.alpha_composite(sheen_layer())
    return out.convert('RGB')

frames = sorted(f for f in os.listdir(os.path.join(D, 'frames')) if f.endswith('.png'))
treat = [
    ('plain', lambda im: plain(im)),
    ('letterbox', lambda im: letterbox(im)),
    ('poster8', lambda im: poster(im, 8)),
    ('poster5', lambda im: poster(im, 5)),
    ('zoom1.5+poster8', lambda im: poster(im, 8, 1.5)),
    ('poster8+crt', lambda im: crt(poster(im, 8))),
    ('zoom1.3+lift', lambda im: lifted(im)),
    ('zoom1.3+lift+sheen', None),
]
GAP = 10
cards = ['tv-news', 'tv-weather', 'tv-penguin', 'tv-cooking']
cols = max(len(treat), len(cards))
sheet = Image.new('RGB', (GAP + cols*(W+GAP) + 140, GAP + (len(frames)+1)*(H+GAP)), BG)
for i, c in enumerate(cards):
    sheet.paste(tv(card=c), (GAP + i*(W+GAP), GAP))
for r, f in enumerate(frames):
    im = Image.open(os.path.join(D, 'frames', f)).convert('RGB')
    for c, (name, fn) in enumerate(treat):
        pic = tv(lifted(im), sheen=True) if fn is None else tv(fn(im))
        sheet.paste(pic, (GAP + c*(W+GAP), GAP + (r+1)*(H+GAP)))
    # source thumbnail for reference (what the frame is), small
    ref = im.copy(); ref.thumbnail((130, H))
    sheet.paste(ref, (GAP + cols*(W+GAP), GAP + (r+1)*(H+GAP)))
sheet.save(os.path.join(D, 'd7-mock-1x.png'))
sheet.resize((sheet.width*3, sheet.height*3), Image.NEAREST).save(os.path.join(D, 'd7-mock-1x-nn3x.png'))
print('rows:', ['cards'] + frames); print('cols:', [t[0] for t in treat])
