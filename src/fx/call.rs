//! The incoming-call view, drawn where the banner was: the calling app's
//! icon in pixel art (half blocks, its own colours; shapes that read as the
//! app, not the logos), and under it a card with the caller and the app.
//! It moves with the ring: each note sends a wave out of the icon, buzzes it
//! and runs a light round the card's border. Nothing here allocates.

use super::Ctx;
use crate::calls::Call;
use crate::canvas::Canvas;
use crate::color::{Rgb, WHITE};
use crate::ringer::{NOTES, ROUND};

/// Pixel art, two pixels to a cell: a digit is that colour of the list
/// (1-based), `w` white, a space nothing.
struct Icon {
    rows: &'static [&'static str],
    colors: &'static [Rgb],
}

const DISCORD: Icon = Icon {
    rows: &[
        "        1111         1111       ",
        "      1111111      1111111      ",
        "      1111111      1111111      ",
        "      11111111111111111111      ",
        "     1111111111111111111111     ",
        "    111111111111111111111111    ",
        "   11111111111111111111111111   ",
        "   111111111111111111111111111  ",
        "  1111111111111111111111111111  ",
        "  11111111   111111   11111111  ",
        "  1111111     1111     1111111  ",
        "  1111111     1111     1111111  ",
        "  111111       11       111111  ",
        "  1111111      111      111111  ",
        "  1111111     1111     1111111  ",
        "  11111111   111111   11111111  ",
        "   11111111111111111111111111   ",
        "   111111111111111111111111111  ",
        "  1111111111111111111111111111  ",
        "  1111111111111111111111111111  ",
        "  111111111          111111111  ",
        "   1111111            1111111   ",
        "     1111              1111     ",
        "       11               1       ",
    ],
    colors: &[Rgb(88, 101, 242)],
};

const SLACK: Icon = Icon {
    rows: &[
        "        11111  22222        ",
        "        11111  22222        ",
        "        11111  22222        ",
        "        11111  22222        ",
        "        11111  22222        ",
        "               22222        ",
        "               22222        ",
        " 111111111111  22222  22222 ",
        " 111111111111  22222  22222 ",
        " 111111111111  22222  22222 ",
        " 111111111111  22222  22222 ",
        " 111111111111  22222  22222 ",
        "                            ",
        "                            ",
        " 44444  44444  333333333333 ",
        " 44444  44444  333333333333 ",
        " 44444  44444  333333333333 ",
        " 44444  44444  333333333333 ",
        " 44444  44444  333333333333 ",
        "        44444               ",
        "        44444               ",
        "        44444  33333        ",
        "        44444  33333        ",
        "        44444  33333        ",
        "        44444  33333        ",
        "        44444  33333        ",
    ],
    colors: &[Rgb(54, 197, 240), Rgb(46, 182, 125), Rgb(236, 178, 46), Rgb(224, 30, 90)],
};

const TEAMS: Icon = Icon {
    rows: &[
        "                       222      ",
        "                      22222     ",
        "  11111111111111111112222222    ",
        " 111111111111111111111222222    ",
        " 111111111111111111111222222    ",
        " 111111111111111111111222222    ",
        " 1111wwwwwwwwwwwww111122222     ",
        " 1111wwwwwwwwwwwww1111          ",
        " 1111wwwwwwwwwwwww1111222222    ",
        " 11111111wwwww1111111122222222  ",
        " 111111111wwww1111111122222222  ",
        " 111111111wwww11111111222222222 ",
        " 111111111wwww11111111222222222 ",
        " 111111111wwww11111111222222222 ",
        " 111111111wwww11111111222222222 ",
        " 111111111wwww11111111222222222 ",
        " 111111111wwww11111111222222222 ",
        " 111111111wwww11111111222222222 ",
        " 111111111wwww1111111122222222  ",
        " 11111111111111111111122222222  ",
        " 111111111111111111111222222    ",
        " 111111111111111111111          ",
        "  1111111111111111111           ",
        "                                ",
    ],
    colors: &[Rgb(80, 89, 201), Rgb(123, 131, 235)],
};

const WHATSAPP: Icon = Icon {
    rows: &[
        "           1111111          ",
        "        111111111111        ",
        "      1111111111111111      ",
        "     111111111111111111     ",
        "    11111111111111111111    ",
        "   1111111111111111111111   ",
        "   11111wwww1111111111111   ",
        "  111111wwww11111111111111  ",
        "  111111wwww11111111111111  ",
        "  111111wwwww11111111111111 ",
        " 1111111wwww111111111111111 ",
        " 1111111www1111111111111111 ",
        " 1111111wwww111111111111111 ",
        " 11111111www111111111111111 ",
        " 11111111wwww11111111111111 ",
        " 111111111wwww11111ww111111 ",
        " 1111111111wwww11wwww111111 ",
        "  111111111wwwwwwwwww11111  ",
        "  11111111111wwwwwwww11111  ",
        "   11111111111wwwwww11111   ",
        "   1111111111111111111111   ",
        "   111111111111111111111    ",
        "  111111111111111111111     ",
        "  11111111111111111111      ",
        "  1111  111111111111        ",
        " 11       11111111          ",
    ],
    colors: &[Rgb(37, 211, 102)],
};

const PHONE: Icon = Icon {
    rows: &[
        "   11111111111111111111111111   ",
        "  1111111111111111111111111111  ",
        "  1111111111111111111111111111  ",
        "  1111111111111111111111111111  ",
        "  1111111111111111111111111111  ",
        "  1111111111111111111111111111  ",
        "  1111111              1111111  ",
        "  1111111              1111111  ",
        "   11111                11111   ",
        "          111111111111          ",
        "         11111111111111         ",
        "         11111    11111         ",
        "         1111      1111         ",
        "        1111        1111        ",
        "        1111   11   1111        ",
        "        1111   111  11111       ",
        "       11111    1   11111       ",
        "       111111      111111       ",
        "      11111111    11111111      ",
        "     1111111111111111111111     ",
        "     1111111111111111111111     ",
        "     1111111111111111111111     ",
        "      11111111111111111111      ",
        "                                ",
    ],
    colors: &[Rgb(200, 200, 200)],
};

fn icon(app: &str) -> &'static Icon {
    match app {
        "Discord" => &DISCORD,
        "Slack" => &SLACK,
        "Teams" => &TEAMS,
        "WhatsApp" => &WHATSAPP,
        _ => &PHONE,
    }
}

impl Icon {
    fn w(&self) -> usize {
        self.rows[0].len()
    }

    /// The pixel's colour index (white is 9, above the rest), 0 for none.
    fn px(&self, x: usize, y: usize) -> u8 {
        match self.rows.get(y).and_then(|r| r.as_bytes().get(x)) {
            Some(b'w') => 9,
            Some(&b @ b'1'..=b'8') => b - b'0',
            _ => 0,
        }
    }

    fn color(&self, i: u8) -> Rgb {
        if i == 9 { WHITE } else { self.colors[(i as usize - 1).min(self.colors.len() - 1)] }
    }

    /// The border's colours: the app's own, or one and a lighter one.
    fn edge(&self, u: f32) -> Rgb {
        let n = self.colors.len();
        if n == 1 {
            let c = self.colors[0];
            return c.mix(c.mix(WHITE, 0.5), 0.5 - 0.5 * (u * std::f32::consts::TAU).cos());
        }
        let p = u.rem_euclid(1.0) * n as f32;
        let i = p as usize % n;
        self.colors[i].mix(self.colors[(i + 1) % n], p.fract())
    }
}

const WAVES: usize = 6;
/// How long a wave takes to reach the screen's edge (s): longer than the
/// chime's quiet stretch, so one is always on its way out.
const LIFE: f32 = 2.4;

/// The ring's beat: when each recent wave left the icon. It follows the
/// built-in tone's notes, or else the onsets in what the sound card plays (a
/// ringtone file, the app's own ring); with nothing to follow it still beats
/// once a round.
pub struct Pulse {
    ages: [f32; WAVES],
    /// How hard each one rang: the first note of each pair, or a loud onset,
    /// more than the second.
    strength: [f32; WAVES],
    /// Waves sent so far: the newest is `sent - 1`, and it picks the colour.
    sent: usize,
    last_round: f32,
}

impl Pulse {
    pub fn new() -> Pulse {
        Pulse { ages: [f32::INFINITY; WAVES], strength: [0.0; WAVES], sent: 0, last_round: -1.0 }
    }

    /// A new ring: no waves yet, the first one at once.
    pub fn reset(&mut self) {
        *self = Pulse::new();
    }

    /// `round` is how far the built-in tone is into its round, `heard` an
    /// onset in the sound (0 = none); waves only leave while it rings.
    pub fn step(&mut self, dt: f32, ringing: bool, round: Option<f32>, heard: f32) {
        for a in &mut self.ages {
            *a += dt;
        }
        let newest = if self.sent == 0 { f32::INFINITY } else { self.ages[(self.sent - 1) % WAVES] };
        let fire = match round {
            Some(r) => {
                let (r, prev) = (r % ROUND, self.last_round);
                self.last_round = r;
                // a note since last frame; r below prev is the next round
                let note = NOTES.iter().position(|&(at, _)| if r >= prev { prev < at && at <= r } else { at <= r });
                note.map(|i| if i % 2 == 0 { 1.0 } else { 0.7 })
            }
            None if heard > 0.0 && newest > 0.12 => Some((0.6 + 0.6 * heard).min(1.0)),
            None => (newest > ROUND).then_some(1.0),
        };
        if let Some(k) = fire.filter(|_| ringing) {
            self.ages[self.sent % WAVES] = 0.0;
            self.strength[self.sent % WAVES] = k;
            self.sent += 1;
        }
    }

    /// The newest wave's kick, its strength as it leaves, gone in ~0.3 s.
    fn hit(&self) -> f32 {
        let n = self.sent.wrapping_sub(1) % WAVES;
        if self.sent == 0 { 0.0 } else { self.strength[n] * (-self.ages[n] * 8.0).exp() }
    }
}

/// `s` cut to `room` cells (with … when it doesn't fit), centred on `mid`.
fn centred(cv: &mut Canvas, mid: i32, y: i32, parts: &[(&str, Rgb)], room: usize) {
    let n: usize = parts.iter().map(|(s, _)| s.chars().count()).sum();
    let mut x = mid - n.min(room) as i32 / 2;
    let mut left = room;
    for (s, c) in parts {
        for ch in s.chars() {
            if left == 0 {
                return;
            }
            left -= 1;
            cv.put_top(x, y, if left == 0 && n > room { '…' } else { ch }, *c);
            x += 1;
        }
    }
}

const SUB: &str = "incoming call · ";
const CARD_H: i32 = 7;

/// Where the view goes on a `w`×`h` screen: the icon at full size, at half
/// size on a small terminal, or not at all (its scale and size in cells),
/// the gap under it and the top row.
fn layout(ic: &Icon, w: usize, h: usize) -> (Option<usize>, i32, i32, i32, i32) {
    let ih = ic.rows.len() / 2;
    let fits = |s: usize| ic.w() / s + 4 <= w && ih / s + 2 + CARD_H as usize + 2 <= h;
    let scale = [1, 2].into_iter().find(|&s| fits(s));
    let (iw, ih) = scale.map_or((0, 0), |s| ((ic.w() / s) as i32, (ih / s) as i32));
    let gap = if scale.is_some() { 2 } else { 0 };
    (scale, iw, ih, gap, (h as i32 - ih - gap - CARD_H) / 2)
}

/// The first row under the card: the ribbon keeps below it.
pub fn bottom(call: &Call, w: usize, h: usize) -> i32 {
    let (_, _, ih, gap, top) = layout(icon(&call.app), w, h);
    top + ih + gap + CARD_H
}

/// The app's colours along 0..1, drifting like the card's border; the
/// ribbon wears them while the call is up.
pub fn hue(call: &Call, u: f32, t: f32) -> Rgb {
    icon(&call.app).edge(u - t * 0.06)
}

pub fn draw(cv: &mut Canvas, cx: &Ctx, call: &Call, pulse: &Pulse, a: f32) {
    if cx.w < 24 || cx.h < CARD_H as usize + 2 || a < 0.01 {
        return;
    }
    let ic = icon(&call.app);
    let caller = if call.caller.is_empty() { "—" } else { call.caller.as_str() };
    let (scale, iw, ih, gap, top) = layout(ic, cx.w, cx.h);
    let mid = cx.w as i32 / 2;
    let hit = pulse.hit();

    // the waves: rings out of the icon to the screen's edge, like the shock
    // theme's, a bright head and a fading band behind it, in the app's
    // colours (rows count double, so they're round)
    let (ox, oy) = (mid as f32, top as f32 + ih as f32 / 2.0);
    let r0 = iw as f32 / 2.0 + 2.0;
    let maxr = (cx.w as f32).hypot(cx.h as f32 * 2.0) * 0.55;
    for k in 0..WAVES.min(pulse.sent) {
        let n = pulse.sent - 1 - k;
        let p = pulse.ages[n % WAVES] / LIFE;
        if p >= 1.0 {
            continue;
        }
        let life = 1.0 - p;
        let r = r0 + (maxr - r0) * (1.0 - life * life);
        let c = ic.colors[n % ic.colors.len()];
        let k = a * pulse.strength[n % WAVES] * (0.3 + 0.7 * life);
        // the head starts near white and takes the colour as it runs out
        for (j, (ch, white, kj)) in [('●', 0.7, 1.0), ('•', 0.3, 0.7), ('·', 0.0, 0.4)].into_iter().enumerate() {
            let r = r - j as f32 * 1.1;
            let col = c.mix(WHITE, white * life * life).scale(k * kj);
            let steps = (r * 7.0).max(12.0) as usize;
            for i in 0..steps {
                let th = i as f32 / steps as f32 * std::f32::consts::TAU;
                cv.put((ox + th.cos() * r).round() as i32, (oy + th.sin() * r * 0.5).round() as i32, ch, col);
            }
        }
    }

    // the icon: on each note it pops a little bigger, flashes and buzzes a
    // cell either way, then settles
    if let Some(s) = scale {
        let buzz = if hit > 0.3 { if (cx.t * 30.0) as i32 % 2 == 0 { 1 } else { -1 } } else { 0 };
        let z = 1.0 + 0.12 * hit;
        let (zw, zh) = ((iw as f32 * z).round() as i32, (ih as f32 * z).round() as i32);
        let (x0, y0) = (mid - zw / 2 + buzz, (oy - zh as f32 / 2.0).round() as i32);
        let k = a * (0.85 + 0.15 * hit);
        // output cell → source pixel, nearest
        let sx = |u: i32| ((u as f32 + 0.5) / zw as f32 * (iw as usize * s) as f32) as usize;
        let sy = |v: f32| (v / (zh * 2) as f32 * (ih as usize * 2 * s) as f32) as usize;
        for cy in 0..zh {
            for cx_ in 0..zw {
                let px = sx(cx_);
                let (t, b) = (ic.px(px, sy(cy as f32 * 2.0 + 0.5)), ic.px(px, sy(cy as f32 * 2.0 + 1.5)));
                let (g, i) = match (t, b) {
                    (0, 0) => continue,
                    (_, 0) => ('▀', t),
                    (0, _) => ('▄', b),
                    // one colour, or two (the cell has no background): the
                    // detail, the higher index, takes the cell
                    _ => ('█', t.max(b)),
                };
                cv.put_top(x0 + cx_, y0 + cy, g, ic.color(i).mix(WHITE, 0.3 * hit).scale(k));
            }
        }
    }

    // the card: the scene under it is gone by the time it's half in
    let text_w = caller.chars().count().max(SUB.chars().count() + call.app.chars().count());
    let bw = ((text_w + 12).max(iw as usize + 8)).min(cx.w - 2) as i32;
    let (x0, y0) = (mid - bw / 2, top + ih + gap);
    for y in y0..y0 + CARD_H {
        for x in x0..x0 + bw {
            cv.dim(x, y, (1.0 - 2.0 * a).max(0.0));
        }
    }
    // the border in the app's colours, drifting; each note runs a light from
    // the top middle down both sides to meet at the bottom
    let per = (2 * (bw + CARD_H - 2)) as f32;
    let light = |i: i32| {
        let d = (i as f32 - bw as f32 / 2.0).rem_euclid(per);
        let d = d.min(per - d) / (per / 2.0); // 0 top middle .. 1 bottom middle
        (0..WAVES.min(pulse.sent))
            .map(|k| {
                let age = pulse.ages[(pulse.sent - 1 - k) % WAVES];
                (-((d - age / 0.5) * 7.0).powi(2)).exp() * (-age * 2.5).exp()
            })
            .fold(0.0f32, f32::max)
    };
    let edge = |i: i32| {
        let c = ic.edge(i as f32 / per - cx.t * 0.06);
        c.mix(WHITE, 0.6 * light(i)).scale(a * (0.75 + 0.25 * light(i).max(hit * 0.5)))
    };
    // perimeter index clockwise from the top left
    for d in 0..bw {
        let (t, b) = match d {
            0 => ('╭', '╰'),
            _ if d == bw - 1 => ('╮', '╯'),
            _ => ('─', '─'),
        };
        cv.put_top(x0 + d, y0, t, edge(d));
        cv.put_top(x0 + d, y0 + CARD_H - 1, b, edge(bw + CARD_H - 2 + (bw - 1 - d)));
    }
    for d in 1..CARD_H - 1 {
        cv.put_top(x0 + bw - 1, y0 + d, '│', edge(bw - 1 + d));
        cv.put_top(x0, y0 + d, '│', edge(2 * bw + CARD_H - 3 + (CARD_H - 1 - d)));
    }
    let room = (bw - 6).max(1) as usize;
    centred(cv, mid, y0 + 2, &[(caller, WHITE.scale(a))], room);
    let app_c = ic.colors[0].max(Rgb(110, 110, 110)).scale(a);
    centred(cv, mid, y0 + 4, &[(SUB, Rgb(140, 140, 140).scale(a)), (&call.app, app_c)], room);
}
