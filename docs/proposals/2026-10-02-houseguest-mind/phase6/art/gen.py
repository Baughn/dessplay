#!/usr/bin/env python3
"""Generates the gremlin figurine groups (40 x 84 units, base on the
frame's bottom edge). At a 9 x 19 cell a standing piece is drawn at
scale 0.45 with dy = 9.2 px, so whole pixels fall on x = k*20/9 and
y = 84 - k*20/9."""
import sys

P = 20 / 9


def X(k):
    return round(k * P, 2)


def Y(k):
    return round(84 - k * P, 2)


def mirror_path(d):
    """Mirror an absolute path (M/L/C/Q/Z, x y pairs) about x = 20."""
    out, toks, i = [], d.split(), 0
    expect_x = True
    for t in toks:
        try:
            v = float(t)
        except ValueError:
            out.append(t)
            expect_x = True
            continue
        if expect_x:
            out.append(f'{round(40 - v, 2):g}')
        else:
            out.append(f'{v:g}')
        expect_x = not expect_x
    return ' '.join(out)


def both(d, attrs):
    return (f'    <path d="{d}" {attrs}/>\n'
            f'    <path d="{mirror_path(d)}" {attrs}/>\n')


def plinth(wood, lip):
    return (
        f'    <rect x="{X(2)}" y="{Y(4)}" width="{X(14)}" height="{round(4*P,2)}" rx="1.2" fill="currentColor"/>\n'
        f'    <rect x="{X(3)}" y="{Y(3)}" width="{X(12)}" height="{round(2*P,2)}" fill="{wood}"/>\n'
        f'    <rect x="{X(3)}" y="{Y(3)}" width="{X(12)}" height="{round(1*P,2)}" fill="{lip}"/>\n'
    )


def tube(d, fill, w=2.6, o=4.4):
    return (f'    <path d="{d}" fill="none" stroke="currentColor" stroke-width="{o}" stroke-linecap="round" stroke-linejoin="round"/>\n'
            f'    <path d="{d}" fill="none" stroke="{fill}" stroke-width="{w}" stroke-linecap="round" stroke-linejoin="round"/>\n')


def standing(body, shade, belly, ear, wood, lip, tail='point', arms=True, lift=0.0, ears='short', brows='pinched'):
    s = ''
    if lift:
        s += f'    <g transform="translate(0 {-lift})">\n'
    # Tail, behind him: out off his right hip, ending in a point.
    if tail == 'point':
        s += tube('M 26 72 C 31.5 73.4 35.6 70.6 35.4 65.6', shade, 2.4, 4.2)
        s += f'    <path d="M 35.4 61.4 L 38.2 66.6 L 32.8 66.4 Z" fill="{shade}" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/>\n'
    elif tail == 'curl':
        s += tube('M 26 72 C 34.5 73 37.5 64 33 60.5', shade)
    # Feet.
    for cx in (14.6, 25.4):
        s += f'    <ellipse cx="{cx}" cy="{Y(4)-1.6}" rx="4.2" ry="2.5" fill="{shade}" stroke="currentColor" stroke-width="1.6"/>\n'
    # Body, pot-bellied.
    s += f'    <path d="M 12.6 74.6 C 10.4 66 12.4 54 20 53.6 C 27.6 54 29.6 66 27.4 74.6 Z" fill="{body}" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/>\n'
    s += f'    <ellipse cx="20" cy="69.8" rx="4.4" ry="4.2" fill="{belly}"/>\n'
    if arms:
        # Arms in to his chest, rubbing his hands together.
        s += tube('M 13.4 61.2 C 12.4 64.4 14.6 66 17.8 65', body, 2.4, 3.2)
        s += tube('M 26.6 61.2 C 27.6 64.4 25.4 66 22.2 65', body, 2.4, 3.2)
        s += f'    <circle cx="18.4" cy="64.4" r="2.3" fill="{body}" stroke="currentColor" stroke-width="0.9"/>\n'
        s += f'    <circle cx="21.6" cy="64.4" r="2.3" fill="{body}" stroke="currentColor" stroke-width="0.9"/>\n'
    # Ears, out wide, pink inside.
    if ears == 'short':
        s += both('M 10.6 50 C 6.2 49.4 2.4 45 1 37.6 C 4.8 38.2 9 40 12 42.6 Z',
                  f'fill="{body}" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"')
        s += both('M 9.8 47.6 C 6.6 47 4 44 2.9 40.2 C 5.8 41 8.4 42.4 10.6 44.4 Z',
                  f'fill="{ear}"')
    else:
        s += both('M 10.6 50.4 C 3 50.2 -4 46 -7.8 38.4 C -1.6 38.2 5 40 12 42.6 Z',
                  f'fill="{body}" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"')
        s += both('M 9.8 48 C 3.6 47.8 -1.8 45 -5.2 40.4 C 0 40.6 5 42 10.6 44.4 Z',
                  f'fill="{ear}"')
    # A curl of hair on top.
    s += f'    <path d="M 18 37 C 17.4 32.6 20 29.6 24 30 C 21.6 31.6 21.2 34 22.2 37 Z" fill="{body}" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/>\n'
    # Head, a little jowly.
    s += f'    <path d="M 20 35.6 C 27.2 35.6 31.2 40.2 31.2 46.6 C 31.2 52.8 26.6 57.4 20 57.4 C 13.4 57.4 8.8 52.8 8.8 46.6 C 8.8 40.2 12.8 35.6 20 35.6 Z" fill="{body}" stroke="currentColor" stroke-width="2"/>\n'
    # Eyes: big and yellow, both pupils slid to one side (sly).
    for cx in (15.0, 25.0):
        s += f'    <ellipse cx="{cx}" cy="45.2" rx="3.8" ry="3.6" fill="#f5d04a" stroke="currentColor" stroke-width="1"/>\n'
        s += f'    <circle cx="{round(cx+1.3,2)}" cy="45.8" r="1.7" fill="currentColor"/>\n'
    # Brows pinched in over them: up to no good.
    if brows == 'pinched':
        s += both('M 11.8 40.6 L 18.8 42.8', 'fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"')
    elif brows == 'thin':
        s += both('M 12.2 39.4 L 18.2 41.4', 'fill="none" stroke="currentColor" stroke-width="1.1" stroke-linecap="round"')
    # The grin, ear to ear, and its teeth.
    s += f'    <path d="M 11.6 49.4 Q 20 51.8 28.4 49.4 C 27.4 54.4 24 56 20 56 C 16 56 12.6 54.4 11.6 49.4 Z" fill="#7a1f24" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/>\n'
    s += f'    <path d="M 12.6 50.2 Q 20 52.4 27.4 50.2 L 26.4 52.8 Q 20 54.4 13.6 52.8 Z" fill="#ffffff"/>\n'
    s += f'    <path d="M 16.6 51.4 L 16.8 53.6 M 23.4 51.4 L 23.2 53.6" fill="none" stroke="#7a1f24" stroke-width="0.9"/>\n'
    if lift:
        s += '    </g>\n'
    return s


def pedestal(wood, lip):
    """A two-step plinth, 6 px tall: base 14 px wide, top step 12."""
    return (
        f'    <rect x="{X(2)}" y="{Y(3)}" width="{X(14)}" height="{round(3*P,2)}" rx="1.2" fill="currentColor"/>\n'
        f'    <rect x="{X(3)}" y="{Y(2)}" width="{X(12)}" height="{round(1*P,2)}" fill="{wood}"/>\n'
        f'    <rect x="{X(3)}" y="{Y(6)}" width="{X(12)}" height="{round(4*P,2)}" rx="1" fill="currentColor"/>\n'
        f'    <rect x="{X(4)}" y="{Y(5)}" width="{X(10)}" height="{round(2*P,2)}" fill="{wood}"/>\n'
        f'    <rect x="{X(4)}" y="{Y(5)}" width="{X(10)}" height="{round(1*P,2)}" fill="{lip}"/>\n'
    )


VARIANTS = {
    'gremlin': dict(body='#b29ae0', shade='#9179c4', belly='#d8cbf0', ear='#e7a6a6', wood='#8a5f3c', lip='#b07f52'),
    'gremlin-jade': dict(body='#7fc2a8', shade='#5e9e86', belly='#bfe3d4', ear='#e7a6a6', wood='#8a5f3c', lip='#b07f52'),
    'gremlin-clay': dict(body='#e08e5e', shade='#c06e42', belly='#f3c39c', ear='#f3c39c', wood='#5b6670', lip='#7d8a96'),
}

VIOLET = VARIANTS['gremlin']
JADE = VARIANTS['gremlin-jade']
COMMENTS = {
    'gremlin': """A gremlin figurine for the shelf, 2 x 2 cells (40 x 84): a
       lavender-glazed imp on a wooden plinth, ears out wide, yellow
       eyes slid sideways, brows pinched, grinning ear to ear and
       rubbing his hands; a pointed tail off his hip. The plinth's
       edges sit on whole pixels at a 9 x 19 cell standing on its line
       (x in 20/9 steps, y in 20/9 steps up from 84).""",
    'gremlin-tall': """Variant: the same imp on a two-step pedestal, lifted 2 px.""",
    'gremlin-jade': """Variant: the same imp in a jade glaze.""",
    'gremlin-wide': """Variant, 3 x 2 cells (60 x 84): long goblin ears out to
       the frame's edges; the figure is shifted 4 px right.""",
}
out = ''
for gid, c, kw, base in [
    ('gremlin', VIOLET, {}, 'plinth'),
    ('gremlin-tall', VIOLET, dict(lift=round(2 * P, 2)), 'pedestal'),
    ('gremlin-jade', JADE, {}, 'plinth'),
    ('gremlin-wide', VIOLET, dict(ears='long'), 'plinth'),
]:
    b = plinth(c['wood'], c['lip']) if base == 'plinth' else pedestal(c['wood'], c['lip'])
    inner = b + standing(**c, **kw)
    if gid.endswith('-wide'):
        inner = f'    <g transform="translate({X(4)} 0)">\n' + inner + '    </g>\n'
    out += f'  <!-- {COMMENTS[gid]} -->\n  <g id="{gid}">\n' + inner + '  </g>\n'
if len(sys.argv) > 1:
    # Splice into props.svg, before the clock (replacing any old copy).
    import re
    path = sys.argv[1]
    svg = open(path).read()
    svg = re.sub(r'  <!-- (A gremlin figurine|Variant)[^>]*-->\n  <g id="gremlin[^"]*">.*?\n  </g>\n', '', svg, flags=re.S)
    anchor = '  <!-- A round wall clock'
    svg = svg.replace(anchor, out + anchor, 1)
    open(path, 'w').write(svg)
sys.stdout.write(out)
