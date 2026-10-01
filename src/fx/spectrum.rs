//! The bar floor under the banner (the floor theme): classic eighth-block
//! bars with peak caps, laid out like cava's stereo mirror — bass in the
//! middle, left channel to the left, right to the right.

use super::{BLOCKS, Ctx, Look};
use crate::canvas::Canvas;
use crate::color::WHITE;

pub struct Spectrum {
    pub bar_w: usize,
    pub gap: usize,
    heights: Vec<f32>,
    capv: Vec<f32>,
}

impl Spectrum {
    pub fn new() -> Spectrum {
        Spectrum {
            bar_w: 2,
            gap: 1,
            heights: Vec::new(),
            capv: Vec::new(),
        }
    }

    /// Bars per channel the DSP should compute for this width.
    pub fn bars_for(&mut self, w: usize) -> usize {
        self.bar_w = if w >= 150 { 2 } else { 1 };
        let d = (w + self.gap) / (self.bar_w + self.gap);
        (d / 2).max(2)
    }

    fn arrange(&mut self, cx: &Ctx) {
        let f = cx.f;
        let n = f.left.len();
        let d = n * 2;
        self.heights.resize(d, 0.0);
        self.capv.resize(d, 0.0);
        for j in 0..d {
            let (h, c) = if j < n { (f.left[n - 1 - j], f.caps[n - 1 - j]) } else { (f.right[j - n], f.caps[j - n]) };
            self.heights[j] = h;
            self.capv[j] = c;
        }
    }

    fn x0(&self, w: usize) -> i32 {
        let d = self.heights.len();
        let used = d * (self.bar_w + self.gap) - self.gap;
        (w as i32 - used as i32) / 2
    }

    /// Bars along the bottom `rows` rows of the screen (the floor theme).
    pub fn floor(&mut self, cv: &mut Canvas, cx: &Ctx, rows: usize, a: f32, look: &Look) {
        if cx.f.left.is_empty() {
            return;
        }
        self.arrange(cx);
        self.bars(cv, cx, cx.h, rows, a, look);
    }

    /// Bars growing up from row `base` (exclusive), `rows` tall.
    fn bars(&self, cv: &mut Canvas, cx: &Ctx, base: usize, rows: usize, a: f32, look: &Look) {
        let steps = rows * 8;
        let x0 = self.x0(cx.w);
        let boost = (0.8 + 0.2 * cx.f.kick_env) * cx.light * a;
        let row_col: Vec<_> = (0..rows)
            .map(|k| look.at(k as f32 / rows.max(2) as f32).scale(boost))
            .collect();
        let cap_col = look.at(1.0).mix(WHITE, 0.4).scale(a * cx.light);
        for (j, &h) in self.heights.iter().enumerate() {
            let hs = (h * steps as f32) as usize;
            let bx = x0 + (j * (self.bar_w + self.gap)) as i32;
            for k in 0..rows {
                let fill = hs.saturating_sub(k * 8).min(8);
                if fill == 0 {
                    break;
                }
                let y = base as i32 - 1 - k as i32;
                for dx in 0..self.bar_w as i32 {
                    cv.put(bx + dx, y, BLOCKS[fill], row_col[k]);
                }
            }
            let cp = (self.capv[j] * steps as f32) as usize;
            if cp > 16 {
                let y = base as i32 - 1 - cp.div_ceil(8) as i32;
                for dx in 0..self.bar_w as i32 {
                    cv.put_under(bx + dx, y, '▁', cap_col);
                }
            }
        }
    }
}
