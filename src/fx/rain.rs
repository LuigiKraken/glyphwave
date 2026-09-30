//! Matrix rain: columns of katakana whose fall speed follows loudness; hi-hats
//! light up a few heads. Drawn under everything else.

use super::{Ctx, KATAKANA, Rng};
use crate::canvas::Canvas;
use crate::color::WHITE;

struct Drop {
    y: f32,
    speed: f32,
    len: usize,
    glyphs: Vec<char>,
    flash: f32,
}

pub struct Rain {
    cols: Vec<Option<Drop>>,
    rng: Rng,
}

impl Rain {
    pub fn new() -> Rain {
        Rain { cols: Vec::new(), rng: Rng::seeded() }
    }

    pub fn draw(&mut self, cv: &mut Canvas, cx: &Ctx, a: f32) {
        if self.cols.len() != cx.w {
            self.cols = (0..cx.w).map(|_| None).collect();
        }
        let f = cx.f;
        let pace = if f.silent { 0.5 } else { 0.5 + 1.6 * f.loud };
        let density = 0.012 + 0.02 * f.energy;
        for x in 0..cx.w {
            let rng = &mut self.rng;
            let col = &mut self.cols[x];
            if col.is_none() && rng.chance(density * cx.dt * 60.0 * 0.3) {
                let len = 4 + rng.below(cx.h / 2 + 1);
                *col = Some(Drop {
                    y: -(rng.below(6) as f32),
                    speed: rng.range(6.0, 22.0),
                    len,
                    glyphs: (0..len).map(|_| rng.pick_char(KATAKANA)).collect(),
                    flash: 0.0,
                });
            }
            let Some(d) = col else { continue };
            d.y += d.speed * pace * cx.dt;
            d.flash = (d.flash - cx.dt * 4.0).max(0.0);
            if f.hat > 0.0 && rng.chance(0.08) {
                d.flash = 1.0;
            }
            if rng.chance(0.05) {
                let i = rng.below(d.glyphs.len());
                d.glyphs[i] = rng.pick_char(KATAKANA);
            }
            let head = d.y as i32;
            let body = cx.palette[0].mix(cx.palette[1], 0.4);
            for k in 0..d.len {
                let y = head - k as i32;
                if y < 0 {
                    break;
                }
                let fade = 1.0 - k as f32 / d.len as f32;
                let c = if k == 0 {
                    cx.palette[cx.palette.len() - 1].mix(WHITE, 0.5 + 0.5 * d.flash)
                } else {
                    body.scale(fade * 0.7)
                };
                cv.put_under(x as i32, y, d.glyphs[k % d.glyphs.len()], c.scale(a * cx.light * 0.8));
            }
            if head - d.len as i32 > cx.h as i32 {
                *col = None;
            }
        }
    }
}
