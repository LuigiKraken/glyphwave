#!/bin/sh
# Rebuild docs/demo.gif from one continuous take of `glyphwave --test`.
#
#   cargo build --release && sh docs/record-demo.sh [out.gif]
#
# W, H (terminal cells, 100x30) and FPS (20) can be set in the environment;
# KEEP=dir keeps the log and the PNGs there. About 7 MB for 53 s.
#
# Needs python3 with Pillow, ffmpeg, toilet (the banner) and the DejaVu /
# Noto mono fonts. glyphwave runs in a pseudo-terminal for about 90 s with
# keys sent on a schedule; its output is logged with timestamps, replayed
# into PNGs at $FPS and turned into a GIF. Nothing is spliced: the GIF is
# one stretch of the take that starts and ends on the same black frame (the
# gap between an outro and the next idle intro), so it loops without a jump.
# The synthetic track runs from launch, so the drop always lands at 61.9 s;
# the theme after it is the director's pick of the busiest three left (the
# warm-up plays fire and warp so it is glitch, shock or bounce, not fire).
set -eu
cd "$(dirname "$0")/.."
OUT=${1:-docs/demo.gif}
W=${W:-100} H=${H:-30} FPS=${FPS:-20}
BIN=$PWD/target/release/glyphwave
[ -x "$BIN" ] || { echo "build first: cargo build --release" >&2; exit 1; }
WORK=${KEEP:-$(mktemp -d)}; mkdir -p "$WORK"
[ -n "${KEEP:-}" ] || trap 'rm -rf "$WORK"' EXIT

# the README's banner; a config of its own so the user's (banner, fps,
# ringtone) stays out of it and the fake call rings silently
toilet -w 300 -f bigmono12 glyphwave | sed 's/ *$//' | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}' | sed '/./,$!d' >"$WORK/banner.txt"
mkdir -p "$WORK/cfg/glyphwave"
echo "ringtone = none" >"$WORK/cfg/glyphwave/config"

# time:key. o hides the debug overlay, i toggles idle, 1-0 pick the next
# theme, d fakes a Discord call (rings 5 s). The GIF runs from the black gap
# after the first i to the black gap after the last one: idle, levels, the
# breakdown and the drop (the director's picks), the call, back to idle.
PLAN="0.3:o 1:5 12:0 24:i 33:1 34:i 46:2 70:d 79:i 86:END"

XDG_CONFIG_HOME=$WORK/cfg python3 - "$BIN" "$WORK" "$W" "$H" "$FPS" "$PLAN" <<'EOF'
import os, pty, sys, time, select, struct, fcntl, termios, re, bisect
from PIL import Image, ImageDraw, ImageFont, ImageStat
bin_, work, W, H, fps, plan = sys.argv[1:]
W, H, fps = int(W), int(H), float(fps)
plan = [(float(t), k) for t, k in (s.split(':', 1) for s in plan.split())]
idle = [t for t, k in plan if k == 'i']; cut_from, cut_to = idle[0], idle[-1]

# --- capture: run it in a WxH pty, send the keys, log (time, bytes)
pid, fd = pty.fork()
if pid == 0:
    os.environ.update(TERM='xterm-256color', COLORTERM='truecolor')
    os.execv(bin_, ['glyphwave', '--test', '--banner', f'{work}/banner.txt'])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', H, W, 0, 0))
t0 = time.time(); stamps = []; data = bytearray(); i = 0
while True:
    now = time.time() - t0
    while i < len(plan) and plan[i][0] <= now:
        os.write(fd, b'q' if plan[i][1] == 'END' else plan[i][1].encode()); i += 1
    if select.select([fd], [], [], 0.005)[0]:
        try: d = os.read(fd, 65536)
        except OSError: break
        if not d: break
        data += d; stamps.append((len(data), now))
os.waitpid(pid, 0)

# --- replay: a cell grid fed by the escape codes glyphwave writes, drawn
# at every synchronized-update end and sampled at fps
offs = [o for o, _ in stamps]
def tstamp(o): return stamps[min(bisect.bisect_left(offs, o), len(stamps) - 1)][1]
font = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf', 14)
fb = ImageFont.truetype('/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf', 14)
cw, chh = 8, 17
BRAILLE = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)]
def draw(grid):
    img = Image.new('RGB', (W * cw, H * chh)); d = ImageDraw.Draw(img)
    for yy, row in enumerate(grid):
        for xx, (c, col) in enumerate(row):
            if c == ' ': continue
            X, Y = xx * cw, yy * chh
            box = lambda y0, y1, col=col: d.rectangle([X, Y + y0, X + cw - 1, Y + y1 - 1], fill=col)
            if c == '█': box(0, chh)
            elif c in '░▒▓': a = {'░': .3, '▒': .55, '▓': .8}[c]; box(0, chh, tuple(int(v * a) for v in col))
            elif c == '▀': box(0, chh // 2)
            elif c == '▄': box(chh // 2, chh)
            elif '▁' <= c <= '▇': box(chh - chh * (ord(c) - ord('▁') + 1) // 8, chh)
            elif '\u2800' <= c <= '\u28ff':
                for b, (dx, dy) in enumerate(BRAILLE):
                    if (ord(c) - 0x2800) >> b & 1:
                        px, py = X + dx * cw // 2 + 1, Y + dy * chh // 4 + 1
                        d.rectangle([px, py, px + 1, py + 1], fill=col)
            else: d.text((X, Y), c, font=font if font.getmask(c).getbbox() else fb, fill=col)
    return img
tok = re.compile(r'\x1b\[(\d+);(\d+)H|\x1b\[([\d;]*)m|\x1b\[2J|\x1b\[[?\d;]*[A-Za-z]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|(.)', re.S)
grid = [[(' ', (0, 0, 0))] * W for _ in range(H)]
fg = (255, 255, 255); x = y = 0; prev = 0
frames = []  # (time, png path, brightness)
nxt = cut_from - 3
os.makedirs(f'{work}/f', exist_ok=True)
for m in re.finditer(rb'\x1b\[\?2026l', bytes(data)):
    e = m.end(); chunk = bytes(data[prev:e]).decode('utf-8', 'replace'); prev = e
    t = tstamp(e); img = None
    while nxt < t:  # what was on screen until this update
        img = img or draw(grid)
        path = f'{work}/f/{len(frames):05d}.png'; img.save(path)
        frames.append((nxt, path, sum(ImageStat.Stat(img).sum)))
        nxt += 1 / fps
    for m in tok.finditer(chunk):
        if m.group(1): y, x = int(m.group(1)) - 1, int(m.group(2)) - 1
        elif m.group(3) is not None:
            p = [int(v) for v in m.group(3).split(';') if v] or [0]; j = 0
            while j < len(p):
                if p[j] == 38 and p[j + 1:j + 2] == [2]: fg = tuple(p[j + 2:j + 5]); j += 5
                elif p[j] in (38, 48): j += 5 if p[j + 1:j + 2] == [2] else 3
                elif p[j] == 0: fg = (255, 255, 255); j += 1
                else: j += 1
        elif m.group(0) == '\x1b[2J': grid = [[(' ', (0, 0, 0))] * W for _ in range(H)]
        elif m.group(4) is not None and m.group(4) not in '\r\n':
            if 0 <= y < H and 0 <= x < W: grid[y][x] = (m.group(4), fg)
            x += 1

# --- the cut: from the end of the first black gap after cut_from to the
# first black frame after cut_to; both are the gap between a themed outro
# and an idle intro, so the last frame leads into the first
def first_black(t):
    for k, (ft, _, lum) in enumerate(frames):
        if ft >= t and lum == 0: return k
    sys.exit(f'no black frame after {t:.1f} s')
a, b = first_black(cut_from), first_black(cut_to)
while a + 1 < b and frames[a + 1][2] == 0: a += 1
a = max(first_black(cut_from), a - int(0.4 * fps))  # keep 0.4 s of the gap
print(f'clip {frames[a][0]:.2f}-{frames[b][0]:.2f} s, {b - a} frames', file=sys.stderr)
with open(f'{work}/list.txt', 'w') as f:
    for _, path, _ in frames[a:b]: f.write(f"file '{path}'\n")
EOF

ffmpeg -loglevel error -y -f concat -safe 0 -r "$FPS" -i "$WORK/list.txt" \
    -vf "split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" \
    -r "$FPS" "$OUT"
ls -l "$OUT"
