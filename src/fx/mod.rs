//! Visual layers. Each one reads the shared frame context (audio features,
//! palette, time) and draws into the canvas at an opacity set by the scene
//! director. Audio mostly drives *forces and rates* (springs, speeds, zoom),
//! not positions, so motion stays continuous (the Magnetosphere lesson).

pub mod banner;
pub mod effects;
pub mod label;
pub mod stars;
pub mod rain;
pub mod ribbon;
pub mod spectrum;
pub mod themes;

use crate::color::{Gradient, Rgb, WHITE};
use crate::dsp::Features;

#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub f: &'a Features,
    pub w: usize,
    pub h: usize,
    pub t: f32,
    pub dt: f32,
    pub palette: &'a [Rgb],
    /// Looping OKLab gradient over the palette.
    pub grad: &'a Gradient,
    /// Colour-drift phase 0..1, added to gradient lookups.
    pub phase: f32,
    /// Overall brightness multiplier from loudness (0.75..1).
    pub light: f32,
}

/// The running cycle's colours, shared by the banner, its theme and the
/// ribbon so they always match. A looping gradient over the cycle's stops,
/// read at `u` (0..1 across the banner) plus `shift`, which the banner steps
/// on the beat; a new palette crossfades in, and `flash` whitens everything.
pub struct Look {
    pub stops: Vec<Rgb>,
    cur: Gradient,
    prev: Option<Gradient>,
    blend: f32,
    fade: f32,
    pub shift: f32,
    target: f32,
    pub flash: f32,
}

impl Look {
    pub fn new(stops: &[Rgb]) -> Look {
        Look {
            stops: stops.to_vec(),
            cur: Gradient::looping(stops),
            prev: None,
            blend: 1.0,
            fade: 1.0,
            shift: 0.0,
            target: 0.0,
            flash: 0.0,
        }
    }

    /// Switch to new stops, crossfading over `secs` (0 = at once).
    pub fn set(&mut self, stops: &[Rgb], secs: f32) {
        if stops == self.stops.as_slice() {
            return;
        }
        self.stops = stops.to_vec();
        let next = Gradient::looping(stops);
        self.prev = Some(std::mem::replace(&mut self.cur, next));
        self.blend = if secs <= 0.0 { 1.0 } else { 0.0 };
        self.fade = secs.max(0.01);
    }

    /// Step the colour position by `d` (eased in over ~80 ms).
    pub fn nudge(&mut self, d: f32) {
        self.target += d;
    }

    pub fn step(&mut self, dt: f32) {
        self.blend = (self.blend + dt / self.fade).min(1.0);
        self.shift += (self.target - self.shift) * (1.0 - (-dt / 0.08).exp());
        self.flash *= (-dt / 0.25).exp();
    }

    /// The colour at u (0..1 across the banner).
    pub fn at(&self, u: f32) -> Rgb {
        let t = u.clamp(0.0, 1.0) * 0.5 + self.shift;
        let c = self.cur.wrap(t);
        let c = match &self.prev {
            Some(p) if self.blend < 1.0 => p.wrap(t).mix(c, self.blend),
            _ => c,
        };
        c.mix(WHITE, 0.7 * self.flash)
    }
}

/// One layer's opacity, moved linearly toward a target and eased on read.
#[derive(Clone, Copy, Default)]
pub struct Fader {
    pub v: f32,
    pub target: f32,
}

impl Fader {
    pub fn step(&mut self, dt: f32, secs: f32) {
        let d = dt / secs.max(0.01);
        if self.v < self.target {
            self.v = (self.v + d).min(self.target);
        } else {
            self.v = (self.v - d).max(self.target);
        }
    }

    pub fn a(&self) -> f32 {
        self.v * self.v * (3.0 - 2.0 * self.v)
    }

    pub fn on(&self) -> bool {
        self.v > 0.004
    }
}

/// xorshift32 — plenty for visuals, no dependency.
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Rng {
        Rng(seed.max(1))
    }

    pub fn seeded() -> Rng {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
            .unwrap_or(7);
        Rng::new(t ^ std::process::id().wrapping_mul(2654435761))
    }

    pub fn u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// 0..1
    pub fn f(&mut self) -> f32 {
        (self.u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { self.u32() as usize % n }
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.f() < p
    }

    pub fn pick<T: Copy>(&mut self, s: &[T]) -> T {
        s[self.below(s.len())]
    }

    pub fn pick_char(&mut self, s: &str) -> char {
        let n = s.chars().count();
        s.chars().nth(self.below(n)).unwrap_or(' ')
    }
}

/// Easing curves (terminaltexteffects' set; not all used yet).
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub enum Ease {
    Linear,
    InQuad,
    OutQuad,
    InOutQuad,
    InCubic,
    OutCubic,
    InOutCubic,
    InQuart,
    InOutQuart,
    OutQuint,
    InExpo,
    OutExpo,
    OutCirc,
    InOutCirc,
    OutBack,
    OutBounce,
    InOutSine,
    OutSine,
}

pub fn ease(e: Ease, t: f32) -> f32 {
    use std::f32::consts::PI;
    let t = t.clamp(0.0, 1.0);
    match e {
        Ease::Linear => t,
        Ease::InQuad => t * t,
        Ease::OutQuad => 1.0 - (1.0 - t) * (1.0 - t),
        Ease::InOutQuad => {
            if t < 0.5 { 2.0 * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(2) / 2.0 }
        }
        Ease::InCubic => t * t * t,
        Ease::OutCubic => 1.0 - (1.0 - t).powi(3),
        Ease::InOutCubic => {
            if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
        }
        Ease::InQuart => t.powi(4),
        Ease::InOutQuart => {
            if t < 0.5 { 8.0 * t.powi(4) } else { 1.0 - (-2.0 * t + 2.0).powi(4) / 2.0 }
        }
        Ease::OutQuint => 1.0 - (1.0 - t).powi(5),
        Ease::InExpo => {
            if t == 0.0 { 0.0 } else { 2f32.powf(10.0 * t - 10.0) }
        }
        Ease::OutExpo => {
            if t == 1.0 { 1.0 } else { 1.0 - 2f32.powf(-10.0 * t) }
        }
        Ease::OutCirc => (1.0 - (t - 1.0).powi(2)).sqrt(),
        Ease::InOutCirc => {
            if t < 0.5 {
                (1.0 - (1.0 - (2.0 * t).powi(2)).sqrt()) / 2.0
            } else {
                ((1.0 - (-2.0 * t + 2.0).powi(2)).sqrt() + 1.0) / 2.0
            }
        }
        Ease::OutBack => {
            let (c1, c3) = (1.70158, 2.70158);
            1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
        }
        Ease::OutBounce => {
            let (n1, d1) = (7.5625, 2.75);
            if t < 1.0 / d1 {
                n1 * t * t
            } else if t < 2.0 / d1 {
                let t = t - 1.5 / d1;
                n1 * t * t + 0.75
            } else if t < 2.5 / d1 {
                let t = t - 2.25 / d1;
                n1 * t * t + 0.9375
            } else {
                let t = t - 2.625 / d1;
                n1 * t * t + 0.984375
            }
        }
        Ease::InOutSine => -((PI * t).cos() - 1.0) / 2.0,
        Ease::OutSine => (t * PI / 2.0).sin(),
    }
}

/// Quadratic Bezier.
pub fn bez(p0: (f32, f32), c: (f32, f32), p1: (f32, f32), t: f32) -> (f32, f32) {
    let u = 1.0 - t;
    (
        u * u * p0.0 + 2.0 * u * t * c.0 + t * t * p1.0,
        u * u * p0.1 + 2.0 * u * t * c.1 + t * t * p1.1,
    )
}

pub const KATAKANA: &str = "ｦｱｳｴｵｶｷｹｺｻｼｽｾｿﾀﾂﾃﾅﾆﾇﾈﾊﾋﾎﾏﾐﾑﾒﾓﾔﾕﾗﾘﾜ";
pub const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
