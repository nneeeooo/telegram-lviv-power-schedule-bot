"""
The README logo and banner of lviv-power-bot: a 32x32 pixel icon in the Windows 98 palette and a
98-style window with the wordmark.
Run:  python3 make_logo.py      (writes logo.svg and banner.svg next to this file)
"""
import math, os

PAL = {
    'k': '#000000', 'w': '#ffffff', 's': '#c0c0c0', 'g': '#808080',
    'r': '#ff0000', 'R': '#800000', 'y': '#ffff00', 'Y': '#808000',
    'l': '#00ff00', 'G': '#008000', 'c': '#00ffff', 'C': '#008080',
    'B': '#000080', 'b': '#1084d0', 'u': '#0000ff', 'o': '#ff8000',
    'd': '#202840', 'D': '#101828',
}

N = 32

def blank(w=N, h=N):
    return [['.'] * w for _ in range(h)]

def px(m, x, y, c):
    if 0 <= y < len(m) and 0 <= x < len(m[0]):
        m[y][x] = c

def rect(m, x, y, w, h, c):
    for j in range(y, y + h):
        for i in range(x, x + w):
            px(m, i, j, c)

def box(m, x, y, w, h, fill, light, dark):
    """A 98-style bevelled block: black outline, light top-left, dark bottom-right."""
    rect(m, x, y, w, h, 'k')
    rect(m, x + 1, y + 1, w - 2, h - 2, fill)
    rect(m, x + 1, y + 1, w - 2, 1, light)
    rect(m, x + 1, y + 1, 1, h - 2, light)
    rect(m, x + 1, y + h - 2, w - 2, 1, dark)
    rect(m, x + w - 2, y + 1, 1, h - 2, dark)

def disc(m, cx, cy, r, c):
    for j in range(-r, r + 1):
        for i in range(-r, r + 1):
            if i * i + j * j <= r * r + r * 0.8:
                px(m, cx + i, cy + j, c)

def ring(m, cx, cy, r, c, dash=0):
    """A circle outline; dash > 0 leaves every other run of `dash` pixels out."""
    pts = set()
    for a in range(720):
        t = a * math.pi / 360
        pts.add((round(cx + r * math.cos(t)), round(cy + r * math.sin(t))))
    for (x, y) in sorted(pts, key=lambda p: math.atan2(p[1] - cy, p[0] - cx)):
        k = int((math.atan2(y - cy, x - cx) + math.pi) / (2 * math.pi) * r * 6.28)
        if dash == 0 or (k // dash) % 2 == 0:
            px(m, x, y, c)

def line(m, x0, y0, x1, y1, c):
    dx, dy = abs(x1 - x0), -abs(y1 - y0)
    sx, sy = (1 if x0 < x1 else -1), (1 if y0 < y1 else -1)
    err = dx + dy
    while True:
        px(m, x0, y0, c)
        if x0 == x1 and y0 == y1:
            break
        e2 = 2 * err
        if e2 >= dy:
            err += dy; x0 += sx
        if e2 <= dx:
            err += dx; y0 += sy

def poly(m, pts, c):
    """Fill a polygon (even-odd, pixel centres)."""
    ys = [p[1] for p in pts]
    for y in range(min(ys), max(ys) + 1):
        for x in range(len(m[0])):
            inside = False
            for (x0, y0), (x1, y1) in zip(pts, pts[1:] + pts[:1]):
                if (y0 > y + 0.5) != (y1 > y + 0.5):
                    xi = x0 + (y + 0.5 - y0) * (x1 - x0) / (y1 - y0)
                    if x + 0.5 < xi:
                        inside = not inside
            if inside:
                px(m, x, y, c)

def outline(m, c='k'):
    """Draw a 1 px outline around every non-transparent shape (outside it)."""
    h, w = len(m), len(m[0])
    add = []
    for y in range(h):
        for x in range(w):
            if m[y][x] == '.':
                for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                    xx, yy = x + dx, y + dy
                    if 0 <= xx < w and 0 <= yy < h and m[yy][xx] not in '.':
                        add.append((x, y)); break
    for x, y in add:
        m[y][x] = c

def tile(m, fill='d'):
    """A dark rounded tile with a 98 bevel, for the radar-style icons."""
    rect(m, 2, 1, 28, 30, 'k'); rect(m, 1, 2, 30, 28, 'k')
    rect(m, 3, 2, 26, 28, fill); rect(m, 2, 3, 28, 26, fill)
    rect(m, 3, 2, 26, 1, 'g'); rect(m, 2, 3, 1, 26, 'g')

# 5x7 pixel letters for the wordmark
FONT = {
    'A': ['01110', '10001', '10001', '11111', '10001', '10001', '10001'],
    'B': ['11110', '10001', '10001', '11110', '10001', '10001', '11110'],
    'C': ['01111', '10000', '10000', '10000', '10000', '10000', '01111'],
    'D': ['11110', '10001', '10001', '10001', '10001', '10001', '11110'],
    'E': ['11111', '10000', '10000', '11110', '10000', '10000', '11111'],
    'G': ['01111', '10000', '10000', '10011', '10001', '10001', '01111'],
    'H': ['10001', '10001', '10001', '11111', '10001', '10001', '10001'],
    'I': ['111', '010', '010', '010', '010', '010', '111'],
    'K': ['10001', '10010', '10100', '11000', '10100', '10010', '10001'],
    'L': ['10000', '10000', '10000', '10000', '10000', '10000', '11111'],
    'M': ['10001', '11011', '10101', '10101', '10001', '10001', '10001'],
    'N': ['10001', '11001', '10101', '10011', '10001', '10001', '10001'],
    'O': ['01110', '10001', '10001', '10001', '10001', '10001', '01110'],
    'P': ['11110', '10001', '10001', '11110', '10000', '10000', '10000'],
    'R': ['11110', '10001', '10001', '11110', '10100', '10010', '10001'],
    'S': ['01111', '10000', '10000', '01110', '00001', '00001', '11110'],
    'T': ['11111', '00100', '00100', '00100', '00100', '00100', '00100'],
    'U': ['10001', '10001', '10001', '10001', '10001', '10001', '01110'],
    'V': ['10001', '10001', '10001', '10001', '01010', '01010', '00100'],
    'W': ['10001', '10001', '10001', '10101', '10101', '11011', '10001'],
    'Y': ['10001', '10001', '01010', '00100', '00100', '00100', '00100'],
    '-': ['000', '000', '000', '111', '000', '000', '000'],
    ' ': ['000'] * 7,
}

def text(m, x, y, s, c, scale=1):
    for ch in s:
        g = FONT[ch]
        for j, row in enumerate(g):
            for i, bit in enumerate(row):
                if bit == '1':
                    rect(m, x + i * scale, y + j * scale, scale, scale, c)
        x += (len(g[0]) + 1) * scale
    return x

def text_width(s, scale=1):
    return sum((len(FONT[ch][0]) + 1) * scale for ch in s) - scale

def svg(m, scale, title):
    h, w = len(m), len(m[0])
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{w * scale}" height="{h * scale}" '
           f'viewBox="0 0 {w} {h}" shape-rendering="crispEdges"><title>{title}</title>']
    for y in range(h):
        x = 0
        while x < w:
            c = m[y][x]
            run = 1
            while x + run < w and m[y][x + run] == c:
                run += 1
            if c != '.':
                out.append(f'<rect x="{x}" y="{y}" width="{run}" height="1" fill="{PAL[c]}"/>')
            x += run
    out.append('</svg>')
    return ''.join(out)

def banner(icon, word, title, accent='B'):
    """A Windows 98 window: navy title bar, the icon, the wordmark at 2x and a segmented progress bar."""
    ww = text_width(word, 2)
    W, H = max(150, 44 + ww + 8), 52
    m = blank(W, H)
    box(m, 0, 0, W, H, 's', 'w', 'g')
    for i in range(2, W - 2):
        rect(m, i, 2, 1, 9, 'B' if i < W * 0.55 else 'b')
    text(m, 5, 3, title, 'w')
    for x in (W - 30, W - 21, W - 12):   # _ [] x
        box(m, x, 3, 8, 7, 's', 'w', 'g')
    rect(m, W - 28, 7, 4, 1, 'k')
    rect(m, W - 19, 5, 4, 3, 'k'); rect(m, W - 18, 6, 2, 1, 's')
    for d in range(4):
        px(m, W - 10 + d, 5 + d, 'k'); px(m, W - 7 - d, 5 + d, 'k')
    for y in range(N):
        for x in range(N):
            if icon[y][x] != '.':
                m[15 + y][6 + x] = icon[y][x]
    text(m, 45, 21, word, 'g', 2)
    text(m, 44, 20, word, accent, 2)
    rect(m, 44, 37, ww, 1, 'g'); rect(m, 44, 37, 1, 7, 'g')
    rect(m, 44, 43, ww, 1, 'w'); rect(m, 44 + ww - 1, 37, 1, 7, 'w')
    for bx in range(46, 46 + int((ww - 4) * 0.7) - 3, 5):
        rect(m, bx, 39, 4, 3, accent)
    return m

def write(icon, word, title, accent='B', here=None):
    here = here or os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, 'logo.svg'), 'w') as f:
        f.write(svg(icon, 8, title))
    with open(os.path.join(here, 'banner.svg'), 'w') as f:
        f.write(svg(banner(icon, word, title, accent), 6, title))

def power_bot():
    """A lit light bulb over a 24 h outage bar (green = power, dark = outage)."""
    m = blank()
    disc(m, 15, 11, 8, 'y')
    rect(m, 11, 16, 9, 5, 'y')
    rect(m, 18, 6, 3, 9, 'o'); rect(m, 17, 15, 3, 5, 'o')
    px(m, 10, 7, 'w'); px(m, 9, 8, 'w'); px(m, 9, 9, 'w'); px(m, 10, 6, 'w'); px(m, 11, 5, 'w')
    line(m, 13, 13, 15, 16, 'Y'); line(m, 17, 13, 15, 16, 'Y'); rect(m, 15, 16, 1, 4, 'Y')
    rect(m, 11, 21, 9, 5, 's')
    rect(m, 11, 22, 9, 1, 'g'); rect(m, 11, 24, 9, 1, 'g')
    rect(m, 13, 26, 5, 1, 'g')
    outline(m)
    for (x0, y0, x1, y1) in ((2, 11, 4, 11), (26, 11, 28, 11), (4, 3, 5, 4), (26, 3, 25, 4), (15, 0, 15, 1)):
        line(m, x0, y0, x1, y1, 'o')
    rect(m, 1, 28, 30, 4, 'k')
    for i, on in enumerate('11001110110011'):
        rect(m, 2 + i * 2, 29, 2, 2, 'l' if on == '1' else 'D')
    return m

if __name__ == '__main__':
    write(power_bot(), 'POWER BOT', 'LVIV POWER BOT', 'G')
