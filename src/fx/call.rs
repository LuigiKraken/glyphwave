//! The incoming-call view, drawn where the banner was once it has made way:
//! a small block-art icon for the calling app in its own colours (not the
//! logos, just shapes that read as them), and under it a bordered card with
//! the caller and the app. Only colours move, and nothing here allocates.

use super::Ctx;
use crate::calls::Call;
use crate::canvas::{BLANK, Canvas};
use crate::color::{Rgb, WHITE};

/// Block art: `█ ▀ ▄` in the first colour, digits in that colour of the
/// list (1-based), `w` a white block.
struct Icon {
    rows: &'static [&'static str],
    colors: &'static [Rgb],
}

const W: usize = 16;

const DISCORD: Icon = Icon {
    rows: &[
        "   ▄▄      ▄▄   ",
        " ▄████████████▄ ",
        " ██████████████ ",
        "████  ████  ████",
        "████  ████  ████",
        "████████████████",
        " ▀████▀▀▀▀████▀ ",
        "   ▀▀      ▀▀   ",
    ],
    colors: &[Rgb(88, 101, 242)],
};

const SLACK: Icon = Icon {
    rows: &[
        "    11    22    ",
        "    11    22    ",
        " 33311333322333 ",
        "    11    22    ",
        "    11    22    ",
        " 44411444422444 ",
        "    11    22    ",
        "    11    22    ",
    ],
    colors: &[Rgb(54, 197, 240), Rgb(46, 182, 125), Rgb(236, 178, 46), Rgb(224, 30, 90)],
};

const TEAMS: Icon = Icon {
    rows: &[
        "            22  ",
        " ▄█████████▄2222",
        " █wwwwwwwww█2222",
        " ████www████2222",
        " ████www████2222",
        " ████www████ 22 ",
        " ▀█████████▀    ",
    ],
    colors: &[Rgb(80, 89, 201), Rgb(123, 131, 235)],
};

const WHATSAPP: Icon = Icon {
    rows: &[
        "    ▄██████▄    ",
        "  ▄██████████▄  ",
        " ███ww█████████ ",
        " ███www████████ ",
        " ████www███████ ",
        " █████wwwwww███ ",
        "  ███████████▀  ",
        " ▄██▀▀▀▀▀▀▀▀    ",
    ],
    colors: &[Rgb(37, 211, 102)],
};

const PHONE: Icon = Icon {
    rows: &[
        " ▄████████████▄ ",
        "████▀▀▀▀▀▀▀▀████",
        "███▀        ▀███",
        "     ▄████▄     ",
        "   ▄████████▄   ",
        "  ████████████  ",
        "  ████████████  ",
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

/// Two quick rings, then a pause, every two seconds (0..1).
fn ring(t: f32) -> f32 {
    let p = t.rem_euclid(2.0);
    let bump = |c: f32| (-((p - c) * 9.0).powi(2)).exp();
    bump(0.15) + bump(0.55)
}

/// `s` cut to `room` cells (with … when it doesn't fit); returns the next x.
fn text_fit(cv: &mut Canvas, mut x: i32, y: i32, s: &str, room: usize, c: Rgb) -> i32 {
    let n = s.chars().count();
    for (i, ch) in s.chars().enumerate() {
        if i + 1 == room && n > room {
            cv.put_top(x, y, '…', c);
            return x + 1;
        }
        if i >= room {
            break;
        }
        cv.put_top(x, y, ch, c);
        x += 1;
    }
    x
}

const SUB: &str = " · incoming call";

pub fn draw(cv: &mut Canvas, cx: &Ctx, call: &Call, a: f32) {
    if cx.w < 24 || cx.h < 8 || a < 0.01 {
        return;
    }
    let ic = icon(&call.app);
    let caller = if call.caller.is_empty() { "—" } else { call.caller.as_str() };
    let text_w = caller.chars().count().max(call.app.chars().count() + SUB.chars().count());
    let inner = (text_w + 6).max(W + 8).min(cx.w - 4);
    let (bw, bh) = (inner as i32 + 2, 6i32);
    // the icon goes above the card when there's room for both
    let with_icon = cx.h >= ic.rows.len() + bh as usize + 3;
    let ih = if with_icon { ic.rows.len() as i32 + 1 } else { 0 };
    let top = (cx.h as i32 - ih - bh) / 2;
    let k = 0.85 + 0.15 * ring(cx.t);
    if with_icon {
        let x0 = (cx.w as i32 - W as i32) / 2;
        for (dy, row) in ic.rows.iter().enumerate() {
            for (dx, ch) in row.chars().enumerate() {
                let (g, c) = match ch {
                    ' ' => continue,
                    'w' => ('█', WHITE),
                    '1'..='9' => ('█', ic.colors[(ch as usize - '1' as usize).min(ic.colors.len() - 1)]),
                    _ => (ch, ic.colors[0]),
                };
                cv.put_top(x0 + dx as i32, top + dy as i32, g, c.scale(a * k));
            }
        }
    }
    let (x0, y0) = ((cx.w as i32 - bw) / 2, top + ih);
    for y in y0..y0 + bh {
        for x in x0..x0 + bw {
            if let Some(i) = cv.idx(x, y) {
                cv.cells[i] = BLANK;
            }
        }
    }
    // the border: the shared palette's gradient around the card, drifting
    let per = (2 * (bw + bh)) as f32;
    let edge = |i: i32| cx.grad.wrap(i as f32 / per + cx.t * 0.05).scale(a * (0.7 + 0.3 * ring(cx.t)));
    for x in x0..x0 + bw {
        let (t, b) = match x - x0 {
            0 => ('╭', '╰'),
            d if d == bw - 1 => ('╮', '╯'),
            _ => ('─', '─'),
        };
        cv.put_top(x, y0, t, edge(x - x0));
        cv.put_top(x, y0 + bh - 1, b, edge(bw + bh + (bw - 1 - (x - x0))));
    }
    for y in y0 + 1..y0 + bh - 1 {
        cv.put_top(x0 + bw - 1, y, '│', edge(bw + (y - y0)));
        cv.put_top(x0, y, '│', edge(2 * bw + bh + (bh - 1 - (y - y0))));
    }
    let accent = cx.palette[cx.palette.len() - 1];
    let room = inner.saturating_sub(6);
    let tx = x0 + 4;
    text_fit(cv, tx, y0 + 2, caller, room, accent.scale(a));
    let grey = Rgb(150, 150, 150).scale(a);
    let x = text_fit(cv, tx, y0 + 3, &call.app, room, ic.colors[0].max(Rgb(150, 150, 150)).scale(a));
    text_fit(cv, x, y0 + 3, SUB, room.saturating_sub((x - tx) as usize), grey);
}
