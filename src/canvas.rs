//! Cell canvas with a front/back buffer. `flush` emits only the cells that
//! changed since the last frame (most of a visualizer frame is black), with
//! cursor moves and SGR codes elided where the terminal state already matches.
//! That diff is what keeps the output a few KB per frame instead of ~200 KB.

use crate::color::{BLACK, Rgb};
use std::fmt::Write as _;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
}

pub const BLANK: Cell = Cell { ch: ' ', fg: BLACK, bg: BLACK };

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub cells: Vec<Cell>,
    front: Vec<Cell>,
    full: bool,
    out: String,
    /// Sub-cell braille dots, 2×4 per cell; bit layout per Unicode braille.
    dots: Vec<u8>,
    dot_col: Vec<Rgb>,
    /// Backdrop pixels, w × 2h (square), resolved into half blocks under glyphs.
    pub px: Vec<Rgb>,
}

/// A cell nothing has drawn a glyph into yet (blank, or a backdrop half block).
#[inline]
pub fn empty(ch: char) -> bool {
    ch == ' ' || ch == '▀'
}

#[inline]
fn avg(a: Rgb, b: Rgb) -> Rgb {
    Rgb(((a.0 as u16 + b.0 as u16) / 2) as u8, ((a.1 as u16 + b.1 as u16) / 2) as u8, ((a.2 as u16 + b.2 as u16) / 2) as u8)
}

/// Colour distance below which a cell counts as unchanged. Slow colour drift
/// would otherwise repaint every lit cell every frame for a 1/255 step. The
/// backdrop (dim, always moving) gets a coarser threshold than glyphs.
const TOLERANCE: i32 = 3;
const TOLERANCE_BACKDROP: i32 = 7;

fn close_by(a: Rgb, b: Rgb, t: i32) -> bool {
    (a.0 as i32 - b.0 as i32).abs() <= t
        && (a.1 as i32 - b.1 as i32).abs() <= t
        && (a.2 as i32 - b.2 as i32).abs() <= t
}

/// Quantise to steps of 4 so neighbouring backdrop cells share SGR codes.
#[inline]
fn quant(c: Rgb) -> Rgb {
    Rgb(c.0 & !3, c.1 & !3, c.2 & !3)
}

const BRAILLE_BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas {
            w,
            h,
            cells: vec![BLANK; w * h],
            front: vec![BLANK; w * h],
            full: true,
            out: String::with_capacity(1 << 16),
            dots: vec![0; w * h],
            dot_col: vec![BLACK; w * h],
            px: vec![BLACK; w * h * 2],
        }
    }

    pub fn clear(&mut self) {
        self.cells.fill(BLANK);
        self.dots.fill(0);
        self.dot_col.fill(BLACK);
        self.px.fill(BLACK);
    }

    #[inline]
    pub fn px_add(&mut self, x: i32, py: i32, c: Rgb) {
        if x >= 0 && py >= 0 && (x as usize) < self.w && (py as usize) < self.h * 2 {
            let i = py as usize * self.w + x as usize;
            self.px[i] = self.px[i].add(c);
        }
    }

    /// Turn the pixel layer into half-block cells. Call before any glyph layer.
    pub fn resolve_pixels(&mut self) {
        for y in 0..self.h {
            for x in 0..self.w {
                let (t, b) = (quant(self.px[2 * y * self.w + x]), quant(self.px[(2 * y + 1) * self.w + x]));
                if t.is_black() && b.is_black() {
                    continue;
                }
                let c = &mut self.cells[y * self.w + x];
                if close_by(t, b, 4) {
                    c.bg = avg(t, b); // one colour: a plain background cell is cheaper
                } else {
                    c.ch = '▀';
                    c.fg = t;
                    c.bg = b;
                }
            }
        }
    }

    #[inline]
    pub fn idx(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h)
            .then(|| y as usize * self.w + x as usize)
    }

    /// Set a glyph and its colour, keeping the background.
    #[inline]
    pub fn put(&mut self, x: i32, y: i32, ch: char, fg: Rgb) {
        if let Some(i) = self.idx(x, y) {
            let c = &mut self.cells[i];
            if c.ch == '▀' {
                c.bg = avg(c.fg, c.bg);
            }
            c.ch = ch;
            c.fg = fg;
        }
    }

    /// Put only where the cell is still empty (under-layer semantics).
    #[inline]
    pub fn put_under(&mut self, x: i32, y: i32, ch: char, fg: Rgb) {
        if let Some(i) = self.idx(x, y) {
            if empty(self.cells[i].ch) {
                self.put(x, y, ch, fg);
            }
        }
    }

    /// Dim everything already drawn in a cell (used to seat a label or banner).
    pub fn dim(&mut self, x: i32, y: i32, a: f32) {
        if let Some(i) = self.idx(x, y) {
            let c = &mut self.cells[i];
            c.fg = c.fg.scale(a);
            c.bg = c.bg.scale(a);
        }
    }

    pub fn text(&mut self, x: i32, y: i32, s: &str, fg: Rgb) -> i32 {
        let mut cx = x;
        for ch in s.chars() {
            self.put(cx, y, ch, fg);
            cx += 1;
        }
        cx
    }

    /// Braille dot at sub-cell resolution (2 wide × 4 tall per cell). Colours
    /// accumulate per cell by max so overlapping lines stay bright.
    #[inline]
    pub fn dot(&mut self, dx: i32, dy: i32, c: Rgb) {
        if dx < 0 || dy < 0 {
            return;
        }
        let (x, y) = (dx / 2, dy / 4);
        if let Some(i) = self.idx(x, y) {
            self.dots[i] |= BRAILLE_BITS[(dx % 2) as usize][(dy % 4) as usize];
            self.dot_col[i] = self.dot_col[i].max(c);
        }
    }

    /// Bresenham line in braille-dot space.
    pub fn dot_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb) {
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let (mut x, mut y, mut err) = (x0, y0, dx + dy);
        for _ in 0..4096 {
            self.dot(x, y, c);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Move accumulated braille dots into the cells: only where no glyph sits.
    pub fn resolve_dots(&mut self) {
        for i in 0..self.cells.len() {
            let d = self.dots[i];
            if d != 0 && empty(self.cells[i].ch) {
                let c = &mut self.cells[i];
                if c.ch == '▀' {
                    c.bg = avg(c.fg, c.bg);
                }
                c.ch = char::from_u32(0x2800 + d as u32).unwrap_or(' ');
                c.fg = self.dot_col[i];
            }
            self.dots[i] = 0;
            self.dot_col[i] = BLACK;
        }
    }

    pub fn force_full(&mut self) {
        self.full = true;
    }

    /// Diff against what's on screen and build the escape stream.
    pub fn flush(&mut self) -> &str {
        let out = &mut self.out;
        out.clear();
        out.push_str("\x1b[?2026h");
        if self.full {
            out.push_str("\x1b[0m\x1b[2J");
        }
        let (mut cur_fg, mut cur_bg): (Option<Rgb>, Option<Rgb>) = (None, None);
        let mut cursor: Option<(usize, usize)> = None;
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                let mut c = self.cells[i];
                if c.fg.is_black() && c.ch != ' ' && c.bg.is_black() {
                    c.ch = ' '; // an invisible glyph is a blank; saves bytes
                }
                let f = self.front[i];
                let tol = if c.ch == '▀' || c.ch == ' ' { TOLERANCE_BACKDROP } else { TOLERANCE };
                let same = c.ch == f.ch
                    && close_by(c.bg, f.bg, tol)
                    && (c.ch == ' ' || close_by(c.fg, f.fg, tol));
                if same && !self.full {
                    continue;
                }
                if self.full && c == BLANK {
                    self.front[i] = c;
                    continue;
                }
                if cursor != Some((x, y)) {
                    let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
                }
                // one SGR sequence carrying whichever of fg / bg changed
                let need_bg = cur_bg != Some(c.bg);
                let need_fg = c.ch != ' ' && cur_fg != Some(c.fg);
                if need_bg || need_fg {
                    out.push_str("\x1b[");
                    if need_fg {
                        let _ = write!(out, "38;2;{};{};{}", c.fg.0, c.fg.1, c.fg.2);
                        cur_fg = Some(c.fg);
                    }
                    if need_bg {
                        if need_fg {
                            out.push(';');
                        }
                        if c.bg.is_black() {
                            out.push_str("49");
                        } else {
                            let _ = write!(out, "48;2;{};{};{}", c.bg.0, c.bg.1, c.bg.2);
                        }
                        cur_bg = Some(c.bg);
                    }
                    out.push('m');
                }
                out.push(c.ch);
                cursor = Some((x + 1, y));
                self.front[i] = c;
            }
        }
        self.full = false;
        out.push_str("\x1b[?2026l");
        out
    }
}
