# Renders docs/screenshot.png by running etop in a pseudo-terminal, capturing
# the screen with pyte and rasterising it with Pillow. macOS only (uses Menlo).
#   python3 -m venv .venv && .venv/bin/pip install pyte pillow
#   .venv/bin/python docs/screenshot.py target/release/etop docs/screenshot.png 22
import os, pty, sys, time, select, fcntl, termios, struct, signal, pyte
from PIL import Image, ImageDraw, ImageFont

bin_, out = sys.argv[1], sys.argv[2]
cols, rows = 132, 38
screen = pyte.Screen(cols, rows)
stream = pyte.ByteStream(screen)
pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.environ["COLORTERM"] = "truecolor"
    os.execv(bin_, [bin_, "-r", "250"])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
os.kill(pid, signal.SIGWINCH)
end = time.time() + float(sys.argv[3]) if len(sys.argv) > 3 else time.time() + 7
while time.time() < end:
    r, _, _ = select.select([fd], [], [], 0.05)
    if r:
        try: stream.feed(os.read(fd, 65536))
        except OSError: break
os.write(fd, b"q"); time.sleep(0.3)
try: os.kill(pid, signal.SIGKILL)
except ProcessLookupError: pass

NAMED = {
    "black": (20, 21, 31), "red": (240, 80, 80), "green": (80, 220, 120), "yellow": (240, 200, 60),
    "blue": (100, 140, 255), "magenta": (220, 120, 240), "cyan": (80, 200, 240), "white": (220, 224, 232),
    "brightblack": (90, 95, 110), "brightred": (255, 110, 110), "brightgreen": (120, 240, 150),
    "brightyellow": (255, 220, 100), "brightblue": (140, 170, 255), "brightmagenta": (240, 150, 255),
    "brightcyan": (120, 220, 255), "brightwhite": (245, 246, 250),
}
BG, FG = (20, 21, 31), (208, 212, 224)
def rgb(c, default):
    if c == "default": return default
    if c in NAMED: return NAMED[c]
    try: return tuple(int(c[i:i+2], 16) for i in (0, 2, 4))
    except Exception: return default

size = 26
regular = ImageFont.truetype("/System/Library/Fonts/Menlo.ttc", size, index=0)
bold = ImageFont.truetype("/System/Library/Fonts/Menlo.ttc", size, index=1)
braille = ImageFont.truetype("/System/Library/Fonts/Apple Braille.ttf", size)
symbols = ImageFont.truetype("/System/Library/Fonts/Apple Symbols.ttf", size)
cw = round(regular.getlength("M"))
asc, desc = regular.getmetrics()
ch = asc + desc
pad = 28
img = Image.new("RGB", (cols * cw + 2 * pad, rows * ch + 2 * pad), BG)
d = ImageDraw.Draw(img)
BOX = {0x2500, 0x2501, 0x2502, 0x256D, 0x256E, 0x256F, 0x2570}
def draw_special(c, x0, y0, fg):
    o = ord(c)
    if 0x2800 <= o <= 0x28FF:
        bits = o - 0x2800
        dots = [(0, 0, 0x01), (0, 1, 0x02), (0, 2, 0x04), (1, 0, 0x08),
                (1, 1, 0x10), (1, 2, 0x20), (0, 3, 0x40), (1, 3, 0x80)]
        dw, dh = cw / 2, ch / 4
        for col, row, bit in dots:
            if bits & bit:
                x, y = x0 + col * dw, y0 + row * dh
                d.rectangle([x + 1, y + 1, x + dw - 1, y + dh - 1], fill=fg)
        return True
    if o in BOX:
        mx, my = x0 + cw // 2, y0 + ch // 2
        w = 3 if o == 0x2501 else 2
        if o in (0x2500, 0x2501):
            d.rectangle([x0, my - w // 2, x0 + cw, my + w // 2], fill=fg)
        elif o == 0x2502:
            d.rectangle([mx - w // 2, y0, mx + w // 2, y0 + ch], fill=fg)
        else:
            # Rounded corners: draw the two half-segments meeting at the centre.
            right = o in (0x256D, 0x2570)   # ╭ ╰ open to the right
            down = o in (0x256D, 0x256E)    # ╭ ╮ open downwards
            hx0, hx1 = (mx, x0 + cw) if right else (x0, mx)
            vy0, vy1 = (my, y0 + ch) if down else (y0, my)
            d.rectangle([hx0, my - w // 2, hx1, my + w // 2], fill=fg)
            d.rectangle([mx - w // 2, vy0, mx + w // 2, vy1], fill=fg)
        return True
    return False

def pick(font, c):
    o = ord(c)
    if 0x2800 <= o <= 0x28FF:
        return braille
    if o in (0x2423,):  # open box (␣) is missing from Menlo
        return symbols
    return font
for y in range(rows):
    line = screen.buffer[y]
    for x in range(cols):
        c = line[x]
        fg, bg = rgb(c.fg, FG), rgb(c.bg, BG)
        if c.reverse: fg, bg = bg, fg
        x0, y0 = pad + x * cw, pad + y * ch
        if bg != BG:
            d.rectangle([x0, y0, x0 + cw - 1, y0 + ch - 1], fill=bg)
        if c.data and c.data != " ":
            if draw_special(c.data, x0, y0, fg):
                continue
            f = pick(bold if c.bold else regular, c.data)
            if f is regular or f is bold:
                d.text((x0, y0), c.data, font=f, fill=fg)
            else:
                # Centre glyphs from fallback fonts in the cell.
                gl, gt, gr, gb = f.getbbox(c.data)
                d.text((x0 + (cw - (gr - gl)) // 2 - gl, y0 + (ch - (gb - gt)) // 2 - gt), c.data, font=f, fill=fg)
img.save(out, optimize=True)
print(out, img.size)
