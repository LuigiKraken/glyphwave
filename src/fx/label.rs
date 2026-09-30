//! The now-playing label in the top-left corner, as nowplaying-vis.py drew
//! it: ▶ title, artist · album, and a progress bar between the elapsed time
//! and the length. Only while something is playing.

use super::Ctx;
use crate::canvas::{BLANK, Canvas};
use crate::color::Rgb;
use crate::mpris::Track;

fn fit(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        let mut o: String = s.chars().take(w.saturating_sub(1)).collect();
        o.push('…');
        o
    }
}

fn mmss(t: f64) -> String {
    let t = t.max(0.0) as u64;
    format!("{}:{:02}", t / 60, t % 60)
}

pub fn draw(cv: &mut Canvas, cx: &Ctx, tr: &Track, a: f32) {
    if cx.w < 24 || cx.h < 6 || a < 0.01 {
        return;
    }
    let accent = cx.palette[cx.palette.len() - 1];
    let w = 40.min(cx.w - 4);
    let (x0, y0) = (2i32, 1i32);
    // a clean box: the themes never draw through the text
    for y in y0..y0 + 3 {
        for x in x0 - 1..x0 + w as i32 + 1 {
            if let Some(i) = cv.idx(x, y) {
                cv.cells[i] = BLANK;
            }
        }
    }
    let title = if tr.title.is_empty() { "—" } else { &tr.title };
    cv.text(x0, y0, &fit(&format!("▶ {title}"), w), accent.scale(a));
    let sub: Vec<&str> = [tr.artist.as_str(), tr.album.as_str()].into_iter().filter(|s| !s.is_empty() && *s != tr.title).collect();
    cv.text(x0 + 2, y0 + 1, &fit(&sub.join(" · "), w - 2), Rgb(150, 150, 150).scale(a));
    let pos = tr.position_now();
    let (l, r) = (format!("{} ", mmss(pos)), if tr.length > 0.0 { format!(" {}", mmss(tr.length)) } else { String::new() });
    let bw = (w - 2).saturating_sub(l.len() + r.len());
    let done = if tr.length > 0.0 { ((bw as f64) * (pos / tr.length).clamp(0.0, 1.0)) as usize } else { 0 };
    let dim = Rgb(120, 120, 120).scale(a);
    let mut x = cv.text(x0 + 2, y0 + 2, &l, dim);
    for i in 0..bw {
        let (ch, c) = if i < done { ('━', accent.scale(a)) } else { ('─', Rgb(70, 70, 70).scale(a)) };
        cv.put(x, y0 + 2, ch, c);
        x += 1;
    }
    cv.text(x, y0 + 2, &r, dim);
}
