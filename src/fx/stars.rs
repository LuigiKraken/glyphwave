//! A 3D starfield behind idle effects, in braille dots; it streaks when fast
//! and its speed follows tempo and loudness.

use super::{Ctx, Rng};
use crate::canvas::Canvas;
use crate::color::WHITE;

struct Star {
    x: f32,
    y: f32,
    z: f32,
    px: Option<(i32, i32)>,
}

pub struct Stars {
    stars: Vec<Star>,
    rng: Rng,
    speed: f32,
}

impl Stars {
    pub fn new() -> Stars {
        Stars { stars: Vec::new(), rng: Rng::seeded(), speed: 0.0 }
    }

    /// Starfield in braille-dot space; streaks when fast.
    pub fn draw(&mut self, cv: &mut Canvas, cx: &Ctx, a: f32) {
        let (dw, dh) = ((cx.w * 2) as f32, (cx.h * 4) as f32);
        let want = ((cx.w * cx.h) / 18).clamp(80, 900);
        while self.stars.len() < want {
            let z = self.rng.range(0.1, 1.0);
            self.stars.push(Star { x: self.rng.range(-1.0, 1.0), y: self.rng.range(-1.0, 1.0), z, px: None });
        }
        self.stars.truncate(want);
        let f = cx.f;
        let target = if f.silent { 0.05 } else { 0.08 + 0.25 * f.energy + 0.6 * f.kick_env + 0.1 * (f.bpm / 120.0 - 1.0) };
        self.speed += (target - self.speed) * (1.0 - (-cx.dt / 0.25).exp());
        let (mx, my) = (dw / 2.0, dh / 2.0);
        let scale = dw.max(dh) * 0.5;
        for s in &mut self.stars {
            s.z -= self.speed * cx.dt;
            let sx = mx + s.x / s.z * scale;
            let sy = my + s.y / s.z * scale;
            if s.z <= 0.02 || sx < 0.0 || sy < 0.0 || sx >= dw || sy >= dh {
                s.x = self.rng.range(-1.0, 1.0);
                s.y = self.rng.range(-1.0, 1.0);
                s.z = 1.0;
                s.px = None;
                continue;
            }
            let b = ((1.0 - s.z) * 1.2).clamp(0.05, 1.0);
            let c = cx.grad.wrap(s.z * 0.3 + cx.phase).mix(WHITE, 0.5 * b).scale(b * a * cx.light);
            let p = (sx as i32, sy as i32);
            match s.px {
                Some(q) if self.speed > 0.25 => cv.dot_line(q.0, q.1, p.0, p.1, c),
                _ => cv.dot(p.0, p.1, c),
            }
            s.px = Some(p);
        }
    }
}
