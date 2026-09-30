//! Slow drifting aurora behind idle effects, in the half-block pixel layer:
//! summed sines, no audio needed.

use super::Ctx;
use crate::canvas::Canvas;

pub struct Aurora;

impl Aurora {
    pub fn draw(&mut self, cv: &mut Canvas, cx: &Ctx, a: f32) {
        let (w, h) = (cx.w as f32, (cx.h * 2) as f32);
        let t = cx.t * 0.08;
        for py in 0..(cx.h * 2) {
            let v = py as f32 / h;
            for x in (0..cx.w).step_by(1) {
                let u = x as f32 / w;
                let n = (u * 3.1 + t * 1.3).sin() + (u * 5.7 - t * 0.9 + v * 2.0).sin() * 0.6
                    + (v * 4.0 + t * 0.7 + u * 1.5).sin() * 0.5;
                let band = (-(v - 0.35 - 0.12 * n).powi(2) * 30.0).exp();
                if band < 0.02 {
                    continue;
                }
                let c = cx.grad.wrap(u * 0.4 + t * 0.2 + cx.phase).scale(a * band * 0.22);
                cv.px_add(x as i32, py as i32, c);
            }
        }
    }
}
