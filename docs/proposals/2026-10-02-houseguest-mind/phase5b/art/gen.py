#!/usr/bin/env python3
"""Generate the wall clock and window SVG groups for props.svg.

Geometry is laid out in 1x pixels (a 9 x 19 cell) and converted to SVG
units: a hung piece rasterizes at exactly its footprint, scale 0.45
(width-limited), with dy = 0.1 * rows px. So px x -> x * 20/9 units and
px y -> (y - dy) / 0.45 units. Edges on whole px stay crisp at 1x.
"""
import math
import re
import sys

LINE = "currentColor"
DARK = "#1d1714"


def f(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


class Grid:
    def __init__(self, rows):
        self.dy = 0.1 * rows

    def x(self, px):
        return px * 20 / 9

    def y(self, py):
        return (py - self.dy) / 0.45

    def l(self, px):  # a length
        return px * 20 / 9

    def rect(self, x0, y0, x1, y1, fill, extra=""):
        return (f'<rect x="{f(self.x(x0))}" y="{f(self.y(y0))}" width="{f(self.l(x1 - x0))}" '
                f'height="{f(self.l(y1 - y0))}" fill="{fill}"{extra}/>')

    def circle(self, cx, cy, r, fill, extra=""):
        return f'<circle cx="{f(self.x(cx))}" cy="{f(self.y(cy))}" r="{f(self.l(r))}" fill="{fill}"{extra}/>'

    def poly(self, pts, fill, extra=""):
        d = "M " + " L ".join(f"{f(self.x(a))} {f(self.y(b))}" for a, b in pts) + " Z"
        return f'<path d="{d}" fill="{fill}"{extra}/>'


# ---------------------------------------------------------------- clock

def clock(prefix, rows, cx, cy, R, rim, face_r, mark, hands):
    """A round wall clock centred at (cx, cy) px; R = outer radius px."""
    g = Grid(rows)
    out = []
    body = []
    # A nail and a short brass hanger above the rim.
    body.append(g.circle(cx, cy, R, LINE))  # outline disc
    body.append(g.circle(cx, cy, R - 1, rim))  # the rim
    # One hard shade on the rim's lower right.
    sh = R - 1
    body.append(
        f'<path d="M {f(g.x(cx + sh * math.cos(math.radians(-20))))} {f(g.y(cy - sh * math.sin(math.radians(-20))))} '
        f'A {f(g.l(sh))} {f(g.l(sh))} 0 0 1 {f(g.x(cx + sh * math.cos(math.radians(-160))))} {f(g.y(cy - sh * math.sin(math.radians(-160))))} '
        f'L {f(g.x(cx))} {f(g.y(cy))} Z" fill="{rim_shade(rim)}"/>')
    body.append(g.circle(cx, cy, face_r + 0.7, LINE))  # inner line
    body.append(g.circle(cx, cy, face_r, "#fbf6ea"))  # the face
    # Hour marks: bold bars at 12/3/6/9, dots between.
    m0, m1, mw = mark
    for i in range(12):
        a = math.radians(90 - 30 * i)
        if i % 3 == 0:
            if i == 0:
                body.append(g.rect(cx - mw / 2, cy - m1, cx + mw / 2, cy - m0, LINE))
            elif i == 6:
                body.append(g.rect(cx - mw / 2, cy + m0, cx + mw / 2, cy + m1, LINE))
            elif i == 3:
                body.append(g.rect(cx + m0, cy - mw / 2, cx + m1, cy + mw / 2, LINE))
            else:
                body.append(g.rect(cx - m1, cy - mw / 2, cx - m0, cy + mw / 2, LINE))
        else:
            r = (m0 + m1) / 2 + 0.2
            body.append(g.circle(cx + r * math.cos(a), cy - r * math.sin(a), 0.5, "#b3a48e"))
    # A glint on the glass, upper left.
    gr = face_r - 1.6
    a0, a1 = math.radians(110), math.radians(150)
    body.append(
        f'<path d="M {f(g.x(cx + gr * math.cos(a0)))} {f(g.y(cy - gr * math.sin(a0)))} '
        f'A {f(g.l(gr))} {f(g.l(gr))} 0 0 0 {f(g.x(cx + gr * math.cos(a1)))} {f(g.y(cy - gr * math.sin(a1)))}" '
        f'fill="none" stroke="#ffffff" stroke-width="{f(g.l(0.8))}" stroke-linecap="round"/>')
    out.append(f'  <g id="{prefix}">\n    ' + "\n    ".join(body) + "\n  </g>")
    (hl, hw, ht), (ml, mw2, mt), pin = hands
    X, Y = g.x(cx), g.y(cy)
    # Hands point at 12 from the centre; the dial rotates them about it.
    out.append(
        f'  <g id="{prefix}-hand-hour">\n    <path d="M {f(X)} {f(Y + g.l(ht))} L {f(X)} {f(Y - g.l(hl))}" '
        f'fill="none" stroke="{LINE}" stroke-width="{f(g.l(hw))}" stroke-linecap="round"/>\n  </g>')
    out.append(
        f'  <g id="{prefix}-hand-minute">\n    <path d="M {f(X)} {f(Y + g.l(mt))} L {f(X)} {f(Y - g.l(ml))}" '
        f'fill="none" stroke="{LINE}" stroke-width="{f(g.l(mw2))}"/>\n  </g>')
    out.append(
        f'  <g id="{prefix}-pin">\n    <circle cx="{f(X)}" cy="{f(Y)}" r="{f(g.l(pin))}" fill="{rim}" '
        f'stroke="{LINE}" stroke-width="{f(g.l(0.45))}"/>\n  </g>')
    return out, (X, Y)


def rim_shade(rim):
    return {"#d9655b": "#b84e47", "#5f8f8b": "#4a7471", "#e8b04a": "#c48e2f"}.get(rim, rim)


# --------------------------------------------------------------- window

SKIES = ["night", "dawn", "day", "dusk", "evening"]


def window(prefix, rows, W, frame, sky_box, mullion, sill):
    """A cream two-pane sliding window. All boxes are (x0, y0, x1, y1) px."""
    g = Grid(rows)
    out = []
    fx0, fy0, fx1, fy1 = frame
    sx0, sy0, sx1, sy1 = sky_box
    m0, m1 = mullion
    lx0, ly0, lx1, ly1 = sill
    cream, shade = "#f3ead8", "#d8ccb4"
    # The glass, for the sky groups to clip to.
    out.append(
        f'  <clipPath id="{prefix}-glass">\n    {g.rect(sx0, sy0, sx1, sy1, "#000")}\n  </clipPath>')
    body = []
    # Frame: outline, cream, the glass's outline (an even-odd hole).
    def ring(x0, y0, x1, y1, i0, j0, i1, j1, fill):
        d = (f"M {f(g.x(x0))} {f(g.y(y0))} H {f(g.x(x1))} V {f(g.y(y1))} H {f(g.x(x0))} Z "
             f"M {f(g.x(i0))} {f(g.y(j0))} H {f(g.x(i1))} V {f(g.y(j1))} H {f(g.x(i0))} Z")
        return f'<path d="{d}" fill="{fill}" fill-rule="evenodd"/>'
    body.append(ring(fx0, fy0, fx1, fy1, sx0, sy0, sx1, sy1, LINE))
    body.append(ring(fx0 + 1, fy0 + 1, fx1 - 1, fy1 - 1, sx0 - 1, sy0 - 1, sx1 + 1, sy1 + 1, cream))
    # One hard shade inside the frame's top and left (the reveal).
    body.append(g.rect(fx0 + 1, fy0 + 1, fx1 - 1, fy0 + 2, "#ffffff", ' fill-opacity="0.55"'))
    # The sashes overlap in the middle: outline, cream, outline.
    body.append(g.rect(m0, sy0, m1, sy1, LINE))
    body.append(g.rect(m0 + 1, sy0, m1 - 1, sy1, cream))
    # Two little latches where the sashes meet.
    mid = (sy0 + sy1) / 2
    body.append(g.rect(m0 + 1, mid - 1, m1 - 1, mid + 1, "#c9a14a"))
    # The sill: wider than the frame, with a shaded front.
    body.append(g.rect(lx0, ly0, lx1, ly1, LINE))
    body.append(g.rect(lx0 + 1, ly0 + 1, lx1 - 1, ly1 - 1, cream))
    body.append(g.rect(lx0 + 1, ly1 - 2, lx1 - 1, ly1 - 1, shade))
    out.append(f'  <g id="{prefix}">\n    ' + "\n    ".join(body) + "\n  </g>")
    for name in SKIES:
        out.append(sky(prefix, g, name, sky_box, mullion))
    return out


def sky(prefix, g, name, box, mullion):
    x0, y0, x1, y1 = box
    w, h = x1 - x0, y1 - y0
    m0, m1 = mullion
    lw, rw = m0 - x0, x1 - m1  # pane widths
    parts = []
    R = lambda a, b, c, d, fill, extra="": parts.append(g.rect(a, b, c, d, fill, extra))
    C = lambda cx, cy, r, fill, extra="": parts.append(g.circle(cx, cy, r, fill, extra))
    roof_h = max(3, round(h * 0.16))
    ground = y1 - roof_h

    def star(cx, cy, col="#fff4c2", big=False):
        R(cx, cy, cx + 1, cy + 1, col)
        if big:
            for dx, dy in ((-1, 0), (1, 0), (0, -1), (0, 1)):
                R(cx + dx, cy + dy, cx + dx + 1, cy + dy + 1, col, ' fill-opacity="0.55"')

    def town(col, lit=()):
        # A low skyline: blocks of flats and one gabled house, flat-filled,
        # with lit windows after dark. Laid out for a 24-px-wide sky and
        # scaled to this one.
        sx, sy = w / 24, max(1.0, h / 21)
        blocks = [(0, 2, 2, False), (2, 8, 3, True), (8, 14, 3, False), (14, 17, 5, False),
                  (17, 19, 3, False), (19, 22, 6, False), (22, 24, 2, False)]
        pts = [(x0, y1)]
        for i, (a, b, ht, gable) in enumerate(blocks):
            bx0, bx1 = x0 + round(a * sx), x0 + round(b * sx)
            top = y1 - round(ht * sy)
            if gable:
                pts += [(bx0, top), ((bx0 + bx1) / 2, top - round(2 * sy)), (bx1, top)]
            else:
                pts += [(bx0, top), (bx1, top)]
        pts.append((x1, y1))
        parts.append(g.poly(pts, col))
        for i in lit:
            a, b, ht, _ = blocks[i]
            lx = x0 + round((a + b) / 2 * sx) - 1
            ly = y1 - round(ht * sy) + round(sy)
            R(lx, ly, lx + 1, ly + 1, "#f6d48a")

    if name == "night":
        R(x0, y0, x1, y1, "#2c3f72")
        # A crescent moon in the left pane.
        mx, my, mr = x0 + lw * 0.45, y0 + h * 0.3, max(2.6, w * 0.11)
        C(mx, my, mr, "#f6e7a8")
        C(mx + mr * 0.55, my - mr * 0.3, mr * 0.85, "#2c3f72")
        star(x0 + round(lw * 0.8), y0 + round(h * 0.62))
        star(m1 + round(rw * 0.3), y0 + round(h * 0.18), big=True)
        star(m1 + round(rw * 0.75), y0 + round(h * 0.42))
        star(m1 + round(rw * 0.4), y0 + round(h * 0.62))
        town("#18223f", lit=[5])
    elif name == "evening":
        R(x0, y0, x1, y1, "#36579a")
        R(x0, ground - round(h * 0.22), x1, y1, "#6a5ea8")
        star(m1 + round(rw * 0.55), y0 + round(h * 0.22), big=True)
        town("#232a52", lit=[1, 3, 5])
    elif name == "dusk":
        R(x0, y0, x1, y1, "#7a4f9a")
        R(x0, y0 + round(h * 0.38), x1, y1, "#d0637e")
        R(x0, ground - round(h * 0.2), x1, y1, "#f2924a")
        # A big low sun, red-orange, half down behind the roofs.
        C(m1 + rw * 0.45, ground - 1, max(3.2, w * 0.15), "#ffcf5a")
        town("#4a2c52")
    elif name == "dawn":
        R(x0, y0, x1, y1, "#a9c7ec")
        R(x0, y0 + round(h * 0.45), x1, y1, "#f7bfcc")
        R(x0, ground - round(h * 0.18), x1, y1, "#ffe0a6")
        # A pale sun just clearing the roofs, in the left pane.
        C(x0 + lw * 0.45, ground - 1, max(2.4, w * 0.1), "#fff6d6")
        town("#9a88b0")
    elif name == "day":
        R(x0, y0, x1, y1, "#7cc4ec")
        # The sun, high in the right pane, and a puff of cloud on the left.
        sr = max(2.6, w * 0.11)
        C(m1 + rw * 0.58, y0 + h * 0.3, sr, "#ffd84a", f' stroke="{LINE}" stroke-width="{f(g.l(0.5))}"')
        cx, cy = x0 + lw * 0.45, y0 + h * 0.5
        cr = max(1.6, w * 0.07)
        for dx, dy, r in ((-1.2, 0.3, 1.0), (0.2, -0.4, 1.25), (1.4, 0.3, 0.95)):
            C(cx + dx * cr, cy + dy * cr, r * cr, "#ffffff")
        R(cx - 2.2 * cr, cy + 0.3 * cr, cx + 2.35 * cr, cy + 1.25 * cr, "#ffffff")
        # A bird.
        bx, by = m1 + rw * 0.3, y0 + h * 0.62
        parts.append(f'<path d="M {f(g.x(bx - 1.5))} {f(g.y(by - 1))} Q {f(g.x(bx - 0.7))} {f(g.y(by - 1.2))} {f(g.x(bx))} {f(g.y(by))} Q {f(g.x(bx + 0.7))} {f(g.y(by - 1.2))} {f(g.x(bx + 1.5))} {f(g.y(by - 1))}" fill="none" stroke="#33405a" stroke-width="{f(g.l(0.7))}" stroke-linecap="round" stroke-linejoin="round"/>')
        town("#5d8f7c")
    return (f'  <g id="{prefix}-sky-{name}">\n    <g clip-path="url(#{prefix}-glass)">\n      '
            + "\n      ".join(parts) + "\n    </g>\n  </g>")


def main(svg_path):
    groups = []
    # 3 x 2 clock: 27 x 38 px. Centre on a px centre so 1-px hands at
    # 12/3/6/9 land on whole pixels.
    c3, centre3 = clock("clock", 2, 13.5, 19.5, 13, "#d9655b", 9.6,
                        (7.5, 9.5, 1), ((5.0, 1.6, 1.2), (8.0, 1.0, 1.6), 1.0))
    groups += ["  <!-- A round wall clock, 3 x 2 cells (60 x 84). The dial rotates\n"
               "       clock-hand-hour and clock-hand-minute about the face centre\n"
               f"       ({f(centre3[0])}, {f(centre3[1])}); it is never mirrored. -->"] + c3
    # 4 x 2 clock: 36 x 38 px; centre on a px corner for 2-px hands.
    c4, centre4 = clock("clock4", 2, 18, 20, 16.5, "#d9655b", 12.6,
                        (9.6, 12.2, 2), ((6.6, 2.2, 1.6), (10.6, 1.6, 2.0), 1.4))
    groups += ["  <!-- The same clock at 4 x 2 (80 x 84), for comparison; centre\n"
               f"       ({f(centre4[0])}, {f(centre4[1])}). -->"] + c4
    # 4 x 2 window: 36 x 38 px.
    groups += ["  <!-- A two-pane sliding window, 4 x 2 cells (80 x 84), hung on the\n"
               "       wall: the sky groups (window-sky-*) go behind the frame and\n"
               "       clip themselves to window-glass. Never mirrored. -->"]
    groups += window("window", 2, 36, frame=(2, 3, 34, 32), sky_box=(6, 7, 30, 28),
                     mullion=(16, 20), sill=(1, 31, 35, 36))
    # 5 x 3 window: 45 x 57 px.
    groups += ["  <!-- The same window at 5 x 3 (100 x 126), for comparison. -->"]
    groups += window("window5", 3, 45, frame=(3, 5, 42, 49), sky_box=(7, 9, 38, 45),
                     mullion=(21, 24), sill=(2, 48, 43, 54))
    block = "\n".join(groups)
    src = open(svg_path).read()
    begin, end = "  <!-- gen:wall -->", "  <!-- /gen:wall -->"
    if begin in src:
        src = re.sub(re.escape(begin) + r".*?" + re.escape(end), lambda _: f"{begin}\n{block}\n{end}", src, flags=re.S)
    else:
        src = src.replace("</defs>", f"{begin}\n{block}\n{end}\n</defs>")
    open(svg_path, "w").write(src)
    print("centres", centre3, centre4)


if __name__ == "__main__":
    main(sys.argv[1])
