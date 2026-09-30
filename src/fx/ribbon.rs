//! The music ribbon along the bottom edge: on through every phase of a
//! themed cycle (intro, hold, outro and the gaps between), so the screen
//! never goes still while the music is going.
//!
//! A continuous stereo skyline, mirrored like cava (bass in the middle, left
//! channel to the left), with a dim reflection hanging under its baseline.
//! It wears the banner's colours: each column takes the colour of the
//! letters above it, and crossfades when a new cycle brings a new palette.
//! How much it does follows `intensity`: quiet music gets a low, dim, still
//! ribbon; busy music a tall one that brightens on the beat, flashes on
//! kicks, throws hi-hat sparks and sends a pulse outward on each downbeat.
//! Between holds it lifts a little, to carry the transition. Spikes run into
//! the headroom under the banner on a soft limit, so they never flat-top.

use super::{BLOCKS, Ctx, Rng};
use crate::canvas::Canvas;
use crate::color::{Gradient, Rgb, WHITE};

struct Spark {
    x: i32,
    y: f32,
    vy: f32,
    life: f32,
}

pub struct Ribbon {
    beat: f32,
    pulse: f32,
    sparks: Vec<Spark>,
    rng: Rng,
    /// The banner's stops as last seen, their gradient, the one before it,
    /// and how far the crossfade between the two has got.
    stops: Vec<Rgb>,
    cur: Option<Gradient>,
    prev: Option<Gradient>,
    blend: f32,
}

/// A 0..1 series sampled at u in 0..1 with linear interpolation.
fn sample(v: &[f32], u: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let x = u.clamp(0.0, 1.0) * (v.len() - 1) as f32;
    let i = x as usize;
    let k = (i + 1).min(v.len() - 1);
    v[i] + (v[k] - v[i]) * (x - i as f32)
}

impl Ribbon {
    pub fn new() -> Ribbon {
        Ribbon {
            beat: 0.0,
            pulse: -1.0,
            sparks: Vec::new(),
            rng: Rng::seeded(),
            stops: Vec::new(),
            cur: None,
            prev: None,
            blend: 1.0,
        }
    }

    /// The banner's colour at letter-gradient position u, mid-crossfade.
    fn colour(&self, u: f32) -> Rgb {
        let Some(cur) = &self.cur else { return WHITE };
        let c = cur.at(u);
        match &self.prev {
            Some(p) if self.blend < 1.0 => p.at(u).mix(c, self.blend),
            _ => c,
        }
    }

    /// Draw into the bottom `room` rows at opacity `a`; `lift` (0..1) is the
    /// extra height and life it gets while the banner is between holds.
    /// `stops` and `span` (left edge, width) are the banner's colours and
    /// columns, so the ribbon under each letter matches it.
    pub fn draw(&mut self, cv: &mut Canvas, cx: &Ctx, room: usize, a: f32, lift: f32, stops: &[Rgb], span: (i32, usize)) {
        let f = cx.f;
        let (w, h) = (cx.w as i32, cx.h as i32);
        let dt = cx.dt;
        if !stops.is_empty() && stops != self.stops.as_slice() {
            self.stops = stops.to_vec();
            self.prev = self.cur.take();
            self.cur = Some(Gradient::new(stops));
            self.blend = if self.prev.is_some() { 0.0 } else { 1.0 };
        }
        self.blend = (self.blend + dt / 1.2).min(1.0);
        if room < 3 || f.left.is_empty() || w < 8 || self.cur.is_none() {
            return;
        }
        let int = (f.intensity + 0.2 * lift).min(1.0);

        // a brightness bump on every beat, a pulse on the downbeat
        if f.beat {
            self.beat = int;
            if f.bar_beat() == 0 && int > 0.45 {
                self.pulse = 0.0;
            }
        }
        self.beat *= (-dt / 0.15).exp();
        if self.pulse >= 0.0 {
            self.pulse += dt / 0.7;
            if self.pulse > 1.3 {
                self.pulse = -1.0;
            }
        }

        // typical height above the baseline, the reflection below it, and
        // the headroom up to the banner that spikes may use
        let up = (1.5 + 5.0 * int + 1.5 * lift).round() as usize;
        let up = up.min(room * 2 / 3).max(1);
        let down = up.div_ceil(2).clamp(1, room - up);
        let max_up = room - down;
        let base = h - 1 - down as i32; // the baseline row, bars grow up from here
        let gain = (0.55 + 0.65 * int) * up as f32;
        let bright = (0.4 + 0.6 * int + 0.25 * self.beat).min(1.0) * cx.light * a;
        let flash = f.kick_env.min(1.0) * ((int - 0.35) / 0.4).clamp(0.0, 1.0);

        let (ox, bw) = (span.0 as f32, span.1.max(1) as f32);
        let mid = (w - 1) as f32 / 2.0;
        let lim = max_up as f32;
        let mut tops = Vec::with_capacity(w as usize);
        for x in 0..w {
            let d = (x as f32 - mid) / mid.max(1.0); // -1 left .. 1 right
            let u = d.abs();
            let ch = if d < 0.0 { &f.left } else { &f.right };
            // soft limit: linear while low, easing into the headroom
            let cells = lim * (sample(ch, u) * gain / lim).tanh();
            let hs = (cells * 8.0) as usize;
            tops.push(hs);
            // downbeat pulse: a bright band running out from the middle
            let pd = if self.pulse >= 0.0 { u - self.pulse } else { 9.0 };
            let pl = (-(pd * pd) * 90.0).exp() * int;
            // the colour of the banner's bottom row above this column
            let base_col = self.colour(((x as f32 - ox) / bw * 0.8 + 0.2).clamp(0.0, 1.0));
            for k in 0..max_up {
                let fill = hs.saturating_sub(k * 8).min(8);
                if fill == 0 {
                    break;
                }
                let tip = fill < 8 || hs <= (k + 1) * 8;
                let rise = (k as f32 / up.max(2) as f32).min(1.0);
                let col = base_col
                    .scale(bright * (0.7 + 0.3 * rise))
                    .mix(WHITE, a * (0.35 * flash * if tip { 1.0 } else { 0.3 } + 0.6 * pl));
                cv.put(x, base - k as i32, BLOCKS[fill], col);
            }
            // reflection: half-cell steps, dim, fading with depth
            let rh = hs.div_ceil(8); // bar height in cells
            for k in 0..down {
                let cells = (rh as f32 / 2.0) - k as f32; // mirrored at half scale
                if cells <= 0.0 {
                    break;
                }
                let glyph = if cells >= 1.0 { '█' } else if cells >= 0.5 { '▀' } else { '▔' };
                let fade = 0.28 * (1.0 - k as f32 / down as f32 * 0.6);
                cv.put(x, base + 1 + k as i32, glyph, base_col.scale(bright * fade).mix(WHITE, a * 0.15 * pl));
            }
        }

        // hi-hat sparks off the skyline once the music is busy
        if f.hat > 0.15 && int > 0.35 {
            for _ in 0..(f.hat * 5.0 * int) as usize + 1 {
                let x = self.rng.below(w as usize);
                let top = tops[x].div_ceil(8) as f32;
                self.sparks.push(Spark { x: x as i32, y: base as f32 - top, vy: self.rng.range(3.0, 7.0), life: 1.0 });
            }
        }
        for s in &mut self.sparks {
            s.y -= s.vy * dt;
            s.life -= dt / 0.6;
        }
        let ceiling = (h - room as i32) as f32;
        self.sparks.retain(|s| s.life > 0.0 && s.y > ceiling);
        if self.sparks.len() > 200 {
            self.sparks.drain(..self.sparks.len() - 200);
        }
        for s in &self.sparks {
            let glyph = if s.life > 0.6 { '•' } else if s.life > 0.3 { '·' } else { '˙' };
            let col = self.colour(((s.x as f32 - ox) / bw * 0.8 + 0.2).clamp(0.0, 1.0));
            cv.put_under(s.x, s.y.round() as i32, glyph, col.mix(WHITE, 0.5).scale(s.life * a * cx.light));
        }
    }
}
