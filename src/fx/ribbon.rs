//! The music ribbon along the bottom edge: on through every phase of a
//! themed cycle (intro, hold, outro and the gaps between), so the screen
//! never goes still while the music is going.
//!
//! A continuous stereo skyline, mirrored like cava (bass in the middle, left
//! channel to the left), with a dim reflection hanging under its baseline:
//! a water line that stays put while the music moves the bars.
//! It wears the banner's colours: each column takes the colour of the
//! letters actually drawn above it (`Banner::tint`), so it follows the
//! palette on the beat, a new cycle's palette, and an intro's own colours
//! (the green of the matrix rain) alike.
//! How much it does follows `intensity`: quiet music gets a low, dim, still
//! ribbon; busy music a tall one that brightens on the beat, flashes and
//! jumps on kicks and sends a pulse outward on each downbeat; wild music a
//! taller one still. Only the bars move: the water line never does.
//! Sparks are kept for when the music goes wild (the stretch after a drop,
//! or peak intensity under one of the explosive themes): then the skyline
//! steams on the hi-hats and bursts on the kicks, high and bright, the rate
//! ramping with the wildness so they thin out rather than stop.
//! Between holds it lifts a little, to carry the transition. Spikes run into
//! the headroom under the banner on a soft limit, so they never flat-top.

use super::{BLOCKS, Ctx, Rng, sample};
use crate::canvas::Canvas;
use crate::color::{Rgb, WHITE};

struct Spark {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    heat: f32,
}

pub struct Ribbon {
    beat: f32,
    pulse: f32,
    sparks: Vec<Spark>,
    rng: Rng,
}


impl Ribbon {
    pub fn new() -> Ribbon {
        Ribbon {
            beat: 0.0,
            pulse: -1.0,
            sparks: Vec::new(),
            rng: Rng::seeded(),
        }
    }

    /// Draw into the bottom `room` rows at opacity `a`; `lift` (0..1) is the
    /// extra height and life it gets while the banner is between holds.
    /// `wild` (0..1) is how hard the music is going off, and how many
    /// sparks it throws (none at 0). `tint` is the colour of the letters
    /// above each screen column.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, cv: &mut Canvas, cx: &Ctx, room: usize, a: f32, lift: f32, wild: f32, tint: &[Rgb]) {
        let f = cx.f;
        let (w, h) = (cx.w as i32, cx.h as i32);
        let dt = cx.dt;
        if room < 3 || f.left.is_empty() || w < 8 {
            return;
        }
        let int = (f.intensity + 0.2 * lift).min(1.0);
        if tint.len() != w as usize {
            return;
        }
        let colour = |x: f32| tint[(x as usize).min(tint.len() - 1)];

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
        // the headroom up to the banner that spikes may use. The baseline is
        // a fixed water line, sized for the tallest typical ribbon, so the
        // music only moves the bars and their reflection, never the line.
        // The height follows intensity on a curve: a low line for quiet
        // music, a tall skyline once it's going, and taller again (with a
        // kick on each kick) when it goes wild.
        let fit = |up: usize| up.min(room * 2 / 3).max(1);
        let hot = ((int - 0.5) / 0.35).clamp(0.0, 1.0);
        let up = fit((1.0 + 9.0 * int * int + 1.5 * lift + 2.0 * wild).round() as usize);
        let top = fit(8);
        let down = top.div_ceil(2).clamp(1, room - top);
        let max_up = room - down;
        let base = h - 1 - down as i32; // the baseline row, bars grow up from here
        let gain = (0.55 + 0.65 * int) * up as f32 * (1.0 + 0.5 * hot * f.kick_env.min(1.0));
        let bright = (0.4 + 0.6 * int + 0.25 * self.beat).min(1.0) * cx.light * a;
        let flash = f.kick_env.min(1.0) * ((int - 0.35) / 0.4).clamp(0.0, 1.0);

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
            let base_col = colour(x as f32);
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
                cv.put_lit(x, base - k as i32, BLOCKS[fill], col);
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
                cv.put_lit(x, base + 1 + k as i32, glyph, base_col.scale(bright * fade).mix(WHITE, a * 0.15 * pl));
            }
        }

        // sparks, only while the music is wild: steam on the hi-hats, a
        // burst off the taller columns on each kick
        let n = wild * (f.hat.min(1.0) * 10.0 + if f.kick > 0.0 { w as f32 / 6.0 } else { 0.0 });
        let n = n as usize + self.rng.chance(n.fract()) as usize;
        for _ in 0..n {
            let (x0, x1) = (self.rng.below(w as usize), self.rng.below(w as usize));
            let x = if tops[x0] >= tops[x1] { x0 } else { x1 };
            let top = tops[x].div_ceil(8) as f32;
            self.sparks.push(Spark {
                x: x as f32,
                y: base as f32 - top,
                vx: self.rng.range(-1.5, 1.5),
                vy: self.rng.range(6.0, 14.0),
                life: 1.0,
                heat: wild,
            });
        }
        // they rise, slow under gravity and drift a little
        for s in &mut self.sparks {
            s.x += s.vx * dt;
            s.y -= s.vy * dt;
            s.vy -= 8.0 * dt;
            s.life -= dt / 0.9;
        }
        let ceiling = (h - room as i32) as f32;
        self.sparks.retain(|s| s.life > 0.0 && s.y > ceiling);
        if self.sparks.len() > 300 {
            self.sparks.drain(..self.sparks.len() - 300);
        }
        for s in &self.sparks {
            let glyph = if s.life > 0.6 { '•' } else if s.life > 0.3 { '·' } else { '˙' };
            let col = colour(s.x).mix(WHITE, 0.4 + 0.3 * s.life);
            cv.put_under(s.x.round() as i32, s.y.round() as i32, glyph, col.scale(s.life.sqrt() * s.heat * a * cx.light));
        }
    }
}

/// The held bars' resting height (0..1): low, but still a line.
const REST: f32 = 0.2;

/// What the ribbon is fed. While it's live, the analysis as it is; when it
/// stops (a mute, a pause, the music gone quiet, the floor theme) the last
/// bars are held and settle to a low, still line, the beat, kicks and hats
/// let go, and only then does the ribbon fade. Going live again they rise
/// from there, so it never jumps either way.
pub struct Calm {
    f: crate::dsp::Features,
    /// How much of the analysis gets through, 0 (held) .. 1 (live).
    w: f32,
    /// Seconds since it stopped being live.
    pub since: f32,
}

impl Calm {
    pub fn new() -> Calm {
        Calm { f: Default::default(), w: 0.0, since: 0.0 }
    }

    /// Each frame, drawn or not. `floor` keeps the bars at least at the low
    /// line (a ring: the chime's quiet stretches mustn't empty the ribbon).
    pub fn feed(&mut self, src: &crate::dsp::Features, dt: f32, live: bool, floor: bool) {
        // lets go at once (a pause empties the bars faster than the player
        // says so), takes the music back over a moment
        self.w = if live { (self.w + dt / 0.6).min(1.0) } else { (self.w - dt / 0.15).max(0.0) };
        self.since = if live { 0.0 } else { self.since + dt };
        // the analysis' share each frame, the same at any frame rate
        let k = 1.0 - (1.0 - self.w).powf(dt * 30.0);
        let settle = 1.0 - (-dt / 0.3).exp();
        let f = &mut self.f;
        for (out, inp) in [(&mut f.left, &src.left), (&mut f.right, &src.right)] {
            if out.len() != inp.len() {
                out.clone_from(inp);
            }
            for (o, &i) in out.iter_mut().zip(inp) {
                let held = *o - (*o - o.min(REST)) * settle;
                *o = held + (i - held) * k;
                if floor {
                    *o = o.max(REST);
                }
            }
        }
        let held = f.intensity * (1.0 - settle);
        f.intensity = held + (src.intensity - held) * k;
        f.beat = src.beat && self.w > 0.9;
        f.beat_count = src.beat_count;
        f.kick_env = src.kick_env * self.w;
        f.hat = src.hat * self.w;
    }

    pub fn fed(&self) -> &crate::dsp::Features {
        &self.f
    }
}
