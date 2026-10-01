//! The incoming-call card, centred over whatever runs: the caller, the app,
//! and a border in the shared palette that brightens in a ring-ring rhythm.
//! Only colours move; the card keeps its size for the whole ring.

use super::Ctx;
use super::label::fit;
use crate::calls::Call;
use crate::canvas::{BLANK, Canvas};
use crate::color::Rgb;

/// Two quick rings, then a pause, every two seconds (0..1).
fn ring(t: f32) -> f32 {
    let p = t.rem_euclid(2.0);
    let bump = |c: f32| (-((p - c) * 9.0).powi(2)).exp();
    bump(0.15) + bump(0.55)
}

pub fn draw(cv: &mut Canvas, cx: &Ctx, call: &Call, a: f32) {
    if cx.w < 24 || cx.h < 8 || a < 0.01 {
        return;
    }
    let accent = cx.palette[cx.palette.len() - 1];
    let sub = format!("{} · incoming call", call.app);
    let caller = if call.caller.is_empty() { "—" } else { call.caller.as_str() };
    let text_w = caller.chars().count().max(sub.chars().count()) + 4; // "☎  " in front
    let inner = (text_w + 6).max(30).min(cx.w - 4);
    let (bw, bh) = (inner as i32 + 2, 6i32);
    let x0 = (cx.w as i32 - bw) / 2;
    let y0 = (cx.h as i32 - bh) / 2;
    for y in y0..y0 + bh {
        for x in x0..x0 + bw {
            if let Some(i) = cv.idx(x, y) {
                cv.cells[i] = BLANK;
            }
        }
    }
    // the border: the palette's gradient around the card, drifting slowly
    let k = a * (0.7 + 0.3 * ring(cx.t));
    let per = (2 * (bw + bh)) as f32;
    let edge = |i: i32| cx.grad.wrap(i as f32 / per + cx.t * 0.05).scale(k);
    for x in x0..x0 + bw {
        let (top, bot) = match x - x0 {
            0 => ('╭', '╰'),
            d if d == bw - 1 => ('╮', '╯'),
            _ => ('─', '─'),
        };
        cv.text(x, y0, &top.to_string(), edge(x - x0));
        cv.text(x, y0 + bh - 1, &bot.to_string(), edge(bw + bh + (bw - 1 - (x - x0))));
    }
    for y in y0 + 1..y0 + bh - 1 {
        cv.text(x0 + bw - 1, y, "│", edge(bw + (y - y0)));
        cv.text(x0, y, "│", edge(2 * bw + bh + (bh - 1 - (y - y0))));
    }
    let tx = x0 + 4;
    let room = inner.saturating_sub(7);
    cv.text(tx, y0 + 2, "☎", accent.mix(cx.grad.wrap(cx.t * 0.05), 0.5).scale(k));
    cv.text(tx + 3, y0 + 2, &fit(caller, room), accent.scale(a));
    cv.text(tx + 3, y0 + 3, &fit(&sub, room), Rgb(150, 150, 150).scale(a));
}
