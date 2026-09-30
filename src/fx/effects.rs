//! Banner text effects for idle mode, after terminaltexteffects / ttfx.
//!
//! Same model as TTE's EffectCharacter, flattened: every character gets a
//! delay and a list of stages; a stage moves it from→to (optionally along a
//! quadratic Bezier) under an easing curve while playing a glyph sequence and
//! a colour ramp. Effects are just different ways of building those lists —
//! the phases, symbols, colours and easings follow the TTE originals, with
//! durations derived from TTE's per-tick speeds at 60 fps.

use super::{Ease, Rng, ease};
use crate::color::{Rgb, WHITE};

pub type P = (f32, f32);

#[derive(Clone)]
pub enum Glyph {
    Final,
    Fixed(char),
    /// Play the sequence `n` times across the stage.
    Seq(&'static str, u32),
    /// A new random symbol from the set every ~70 ms.
    Rand(&'static str),
}

#[derive(Clone)]
pub enum Col {
    Final,
    Solid(Rgb),
    /// Evenly across the stage.
    Grad(Vec<Rgb>),
    /// From a colour to the character's final colour.
    ToFinal(Rgb),
    /// Final colour at a brightness ramp a→b.
    Bright(f32, f32),
    /// Rainbow cycling, phase-shifted by an offset.
    Rainbow(f32),
}

#[derive(Clone)]
pub struct Stage {
    pub dur: f32,
    pub from: P,
    pub ctrl: Option<P>,
    pub to: P,
    pub ease: Ease,
    pub glyph: Glyph,
    pub col: Col,
    pub jitter: bool,
}

impl Stage {
    pub fn still(at: P, dur: f32, glyph: Glyph, col: Col) -> Stage {
        Stage { dur, from: at, ctrl: None, to: at, ease: Ease::Linear, glyph, col, jitter: false }
    }

    pub fn go(from: P, to: P, dur: f32, ease: Ease, glyph: Glyph, col: Col) -> Stage {
        Stage { dur, from, ctrl: None, to, ease, glyph, col, jitter: false }
    }
}

#[derive(Clone)]
pub struct Anim {
    pub delay: f32,
    /// Shown before the delay runs out (None = hidden).
    pub pre: Option<(P, Glyph, Col)>,
    pub stages: Vec<Stage>,
    /// Hidden once the last stage ends (outros).
    pub end_hidden: bool,
}

impl Anim {
    fn end(&self) -> f32 {
        self.delay + self.stages.iter().map(|s| s.dur).sum::<f32>()
    }
}

/// What the banner character looks like at a moment.
pub struct Look {
    pub pos: P,
    pub glyph: char,
    pub col: Rgb,
    pub vis: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Expand,
    Rain,
    Slide,
    Decrypt,
    Beams,
    Burn,
    Waves,
    Fireworks,
    Blackhole,
    Unstable,
    Spray,
    Bouncy,
    Print,
    Matrix,
    Vhs,
    Colorshift,
}

pub const INTROS: [Kind; 16] = [
    Kind::Expand,
    Kind::Rain,
    Kind::Slide,
    Kind::Decrypt,
    Kind::Beams,
    Kind::Burn,
    Kind::Waves,
    Kind::Fireworks,
    Kind::Blackhole,
    Kind::Unstable,
    Kind::Spray,
    Kind::Bouncy,
    Kind::Print,
    Kind::Matrix,
    Kind::Vhs,
    Kind::Colorshift,
];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Outro {
    Fade,
    Crumble,
    Implode,
    Scatter,
    Dissolve,
    Rise,
}

pub const OUTROS: [Outro; 6] = [Outro::Fade, Outro::Crumble, Outro::Implode, Outro::Scatter, Outro::Dissolve, Outro::Rise];

pub struct Effect {
    pub kind: Option<Kind>,
    pub anims: Vec<Anim>,
    pub t: f32,
    pub total: f32,
    /// VHS: per-row horizontal glitch offsets, recomputed as it runs.
    pub row_shift: Vec<i32>,
    rng: Rng,
}

fn hash(a: u32, b: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9E3779B1) ^ b.wrapping_mul(0x85EBCA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B3C6D);
    h ^= h >> 12;
    h
}

fn pick(set: &str, h: u32) -> char {
    let n = set.chars().count().max(1);
    set.chars().nth(h as usize % n).unwrap_or('?')
}

fn dist(a: P, b: P) -> f32 {
    let (dx, dy) = (b.0 - a.0, (b.1 - a.1) * 2.0); // rows count double, like TTE
    (dx * dx + dy * dy).sqrt()
}

/// Per-tick TTE speed (cells/frame at 60 fps) → a duration for this distance.
fn dur(a: P, b: P, tick_speed: f32) -> f32 {
    (dist(a, b) / (tick_speed * 60.0)).clamp(0.15, 4.0)
}

const RAINBOW: [&str; 7] = ["e81416", "ffa500", "faeb36", "79c314", "487de7", "4b369d", "70369d"];

fn rainbow(t: f32) -> Rgb {
    let n = RAINBOW.len() as f32;
    let x = t.rem_euclid(1.0) * n;
    let i = x as usize % RAINBOW.len();
    Rgb::hex(RAINBOW[i]).mix(Rgb::hex(RAINBOW[(i + 1) % RAINBOW.len()]), x.fract())
}

fn grad(stops: &[Rgb], u: f32) -> Rgb {
    if stops.len() == 1 {
        return stops[0];
    }
    let x = u.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let i = (x as usize).min(stops.len() - 2);
    stops[i].mix(stops[i + 1], x - i as f32)
}

pub struct Chars<'a> {
    pub home: &'a [P],
    pub row: &'a [usize],
    pub col: &'a [usize],
    pub rows: usize,
    pub cols: usize,
    pub w: f32,
    pub h: f32,
}

impl Effect {
    pub fn empty() -> Effect {
        Effect { kind: None, anims: Vec::new(), t: 0.0, total: 0.0, row_shift: Vec::new(), rng: Rng::seeded() }
    }

    fn finish(mut self) -> Effect {
        self.total = self.anims.iter().map(|a| a.end()).fold(0.0, f32::max);
        self
    }

    pub fn done(&self) -> bool {
        self.t >= self.total
    }

    pub fn intro(kind: Kind, c: &Chars, rng: &mut Rng) -> Effect {
        let n = c.home.len();
        let centre = (c.w / 2.0, c.h / 2.0);
        let mut e = Effect { kind: Some(kind), row_shift: vec![0; c.rows], ..Effect::empty() };
        let hx = |h: u32| h as f32 / u32::MAX as f32;
        let mut anims = Vec::with_capacity(n);
        match kind {
            Kind::Expand => {
                for &h in c.home {
                    anims.push(Anim {
                        delay: 0.0,
                        pre: None,
                        stages: vec![Stage::go(centre, h, dur(centre, h, 0.35), Ease::InOutQuart, Glyph::Final, Col::ToFinal(Rgb::hex("8A008A")))],
                        end_hidden: false,
                    });
                }
            }
            Kind::Rain => {
                let blues = ["00315C", "004C8F", "0075DB", "3F91D9", "78B9F2", "9AC8F5", "B8D8F8", "E3EFFC"];
                // rows land bottom-first, ~1.5 characters per frame
                let mut t0 = 0.0;
                for r in (0..c.rows).rev() {
                    let idx: Vec<usize> = (0..n).filter(|&i| c.row[i] == r).collect();
                    let span = idx.len() as f32 / 90.0;
                    for &i in &idx {
                        let h = c.home[i];
                        let from = (h.0, -1.0);
                        let blue = Rgb::hex(blues[rng.below(blues.len())]);
                        anims.push(Anim {
                            delay: t0 + rng.f() * span,
                            pre: None,
                            stages: vec![
                                Stage::go(from, h, dur(from, h, rng.range(0.33, 0.57)), Ease::InQuart, Glyph::Fixed(rng.pick_char("o.,*|")), Col::Solid(blue)),
                                Stage::still(h, 0.35, Glyph::Final, Col::ToFinal(blue)),
                            ],
                            end_hidden: false,
                        });
                    }
                    t0 += span * 0.6;
                }
                sort_like(&mut anims, c, |a| a);
            }
            Kind::Slide => {
                for i in 0..n {
                    let (r, h) = (c.row[i], c.home[i]);
                    let left = r % 2 == 0;
                    let from = (if left { -2.0 } else { c.w + 1.0 }, h.1);
                    let order = if left { c.col[i] } else { c.cols - c.col[i] };
                    anims.push(Anim {
                        delay: r as f32 * 0.05 + order as f32 / 60.0,
                        pre: None,
                        stages: vec![Stage::go(from, h, dur(from, h, 0.8), Ease::InOutQuad, Glyph::Final, Col::ToFinal(Rgb::hex("833ab4")))],
                        end_hidden: false,
                    });
                }
            }
            Kind::Decrypt => {
                let greens = [Rgb::hex("008000"), Rgb::hex("00cb00"), Rgb::hex("00ff00")];
                let mut order: Vec<usize> = (0..n).collect();
                shuffle(&mut order, rng);
                let typing = n as f32 / 240.0; // ~4 per frame
                for i in 0..n {
                    let h = c.home[i];
                    let d = order[i] as f32 / 240.0;
                    let g = greens[rng.below(3)];
                    anims.push(Anim {
                        delay: d,
                        pre: None,
                        stages: vec![
                            Stage::still(h, 0.1, Glyph::Seq("▉▓▒░", 1), Col::Solid(g)),
                            Stage::still(h, typing - d + 1.2 + rng.f() * 1.6, Glyph::Rand(CIPHER), Col::Solid(g.scale(rng.range(0.5, 1.0)))),
                            Stage::still(h, 0.6, Glyph::Final, Col::Grad(vec![WHITE, Rgb::hex("eda000")])),
                            Stage::still(h, 0.4, Glyph::Final, Col::ToFinal(Rgb::hex("eda000"))),
                        ],
                        end_hidden: false,
                    });
                }
            }
            Kind::Beams => {
                let beam = vec![WHITE, Rgb::hex("00D1FF"), Rgb::hex("8A008A")];
                let starts: Vec<(f32, bool, f32)> = (0..c.rows).map(|_| (rng.range(0.0, 2.2), rng.chance(0.5), rng.range(90.0, 300.0))).collect();
                let mut last = 0.0f32;
                let mut beam_at = vec![0.0f32; n];
                for i in 0..n {
                    let (st, rev, sp) = starts[c.row[i]];
                    let along = if rev { c.w - c.home[i].0 } else { c.home[i].0 };
                    beam_at[i] = st + along / sp;
                    last = last.max(beam_at[i] + 0.15);
                }
                for i in 0..n {
                    let h = c.home[i];
                    let wipe = last + 0.3 + (c.col[i] as f32 + c.row[i] as f32 * 2.0) * 0.006;
                    anims.push(Anim {
                        delay: beam_at[i],
                        pre: None,
                        stages: vec![
                            Stage::still(h, 0.15, Glyph::Seq("▂▁_", 1), Col::Grad(beam.clone())),
                            Stage::still(h, (wipe - beam_at[i] - 0.15).max(0.0), Glyph::Final, Col::Bright(0.3, 0.3)),
                            Stage::still(h, 0.35, Glyph::Final, Col::Bright(0.3, 1.0)),
                        ],
                        end_hidden: false,
                    });
                }
            }
            Kind::Burn => {
                let level = bfs_levels(c, rng);
                let fire = vec![WHITE, Rgb::hex("fff75d"), Rgb::hex("fe650d"), Rgb::hex("8A003C"), Rgb::hex("510100")];
                for i in 0..n {
                    let h = c.home[i];
                    anims.push(Anim {
                        delay: level[i] as f32 * 0.035 + rng.f() * 0.03,
                        pre: Some((h, Glyph::Final, Col::Solid(Rgb::hex("837373").scale(0.6)))),
                        stages: vec![
                            Stage::still(h, 0.6, Glyph::Seq("'.▖▙█▜▀▝.", 1), Col::Grad(fire.clone())),
                            Stage::still(h, 0.7, Glyph::Final, Col::ToFinal(Rgb::hex("510100"))),
                        ],
                        end_hidden: false,
                    });
                }
            }
            Kind::Waves => {
                let wave = vec![Rgb::hex("f0ff65"), Rgb::hex("ffb102"), Rgb::hex("31a0d4"), Rgb::hex("f0ff65")];
                for &h in c.home {
                    anims.push(Anim {
                        delay: dist(h, centre) * 0.012,
                        pre: None,
                        stages: vec![
                            Stage::still(h, 1.8, Glyph::Seq("▁▂▃▄▅▆▇█▇▆▅▄▃▂▁", 3), Col::Grad(wave.clone())),
                            Stage::still(h, 0.8, Glyph::Final, Col::ToFinal(Rgb::hex("31a0d4"))),
                        ],
                        end_hidden: false,
                    });
                }
            }
            Kind::Fireworks => {
                let mut order: Vec<usize> = (0..n).collect();
                shuffle(&mut order, rng);
                let shell = (n / 20).max(4);
                let top = c.home.iter().map(|h| h.1).fold(f32::MAX, f32::min);
                let colours = ["88F7E2", "44D492", "F5EB67", "FFA15C", "FA233E"];
                let mut t0 = 0.0;
                for group in order.chunks(shell) {
                    let apex = (rng.range(c.w * 0.2, c.w * 0.8), (top - rng.range(0.0, 4.0)).max(1.0));
                    let launch = (apex.0, c.h);
                    let col = Rgb::hex(colours[rng.below(colours.len())]);
                    let r = (c.w * 0.2).min(15.0);
                    for &i in group {
                        let a = rng.range(0.0, std::f32::consts::TAU);
                        let rr = r * rng.f().sqrt();
                        let burst = (apex.0 + a.cos() * rr, apex.1 + a.sin() * rr * 0.5);
                        let h = c.home[i];
                        let mut down = Stage::go(burst, h, 1.1, Ease::InOutQuart, Glyph::Final, Col::ToFinal(col.mix(WHITE, 0.5)));
                        down.ctrl = Some((burst.0, burst.1 + 7.0));
                        anims.push(Anim {
                            delay: t0,
                            pre: None,
                            stages: vec![
                                Stage::go(launch, apex, 0.7, Ease::OutExpo, Glyph::Rand("oO0°"), Col::Solid(col)),
                                Stage::go(apex, burst, 0.45, Ease::OutCirc, Glyph::Rand("*+·"), Col::Grad(vec![col, WHITE, col])),
                                down,
                            ],
                            end_hidden: false,
                        });
                    }
                    t0 += 0.75 * rng.range(0.5, 1.5) * 0.6;
                }
            }
            Kind::Blackhole => {
                let stars = [Rgb::hex("ffcc0d"), Rgb::hex("ff7326"), Rgb::hex("ff194d"), Rgb::hex("bf2669"), Rgb::hex("702a8c"), Rgb::hex("049dbf")];
                let collapse = 3.4;
                for &h in c.home {
                    let start = (rng.range(0.0, c.w), rng.range(0.0, c.h));
                    let d = rng.range(0.0, 1.6);
                    let fall = rng.range(1.0, 1.7);
                    let sc = stars[rng.below(stars.len())];
                    let a = rng.range(0.0, std::f32::consts::TAU);
                    let far = (centre.0 + a.cos() * c.w * rng.range(0.15, 0.5), centre.1 + a.sin() * c.h * rng.range(0.15, 0.5));
                    anims.push(Anim {
                        delay: d,
                        pre: Some((start, Glyph::Rand("*'`¤•°·"), Col::Solid(Rgb::hex("4a4a4d")))),
                        stages: vec![
                            Stage::go(start, centre, fall, Ease::InExpo, Glyph::Rand("*'`¤•°·"), Col::Grad(vec![Rgb::hex("4a4a4d"), WHITE])),
                            Stage::still(centre, (collapse - d - fall).max(0.0), Glyph::Seq("◦◎◉●", 2), Col::Solid(WHITE)),
                            Stage::go(centre, far, 0.7, Ease::OutExpo, Glyph::Rand("*'`¤•°·"), Col::Solid(sc)),
                            Stage::go(far, h, 1.0, Ease::InCubic, Glyph::Final, Col::ToFinal(sc)),
                        ],
                        end_hidden: false,
                    });
                }
            }
            Kind::Unstable => {
                let mut perm: Vec<usize> = (0..n).collect();
                shuffle(&mut perm, rng);
                let orange = Rgb::hex("ff9200");
                for i in 0..n {
                    let h = c.home[i];
                    let swap = c.home[perm[i]];
                    let edge = match rng.below(4) {
                        0 => (rng.range(0.0, c.w), 0.0),
                        1 => (rng.range(0.0, c.w), c.h - 1.0),
                        2 => (0.0, rng.range(0.0, c.h)),
                        _ => (c.w - 1.0, rng.range(0.0, c.h)),
                    };
                    let mut shake = Stage::still(swap, 2.4, Glyph::Final, Col::ToFinal(orange));
                    shake.col = Col::Grad(vec![WHITE.scale(0.8), orange]);
                    shake.jitter = true;
                    anims.push(Anim {
                        delay: 0.0,
                        pre: None,
                        stages: vec![
                            shake,
                            Stage::go(swap, edge, 0.6, Ease::OutExpo, Glyph::Final, Col::Solid(orange)),
                            Stage::still(edge, 0.5, Glyph::Final, Col::Solid(orange)),
                            Stage::go(edge, h, 0.9, Ease::OutExpo, Glyph::Final, Col::ToFinal(orange)),
                        ],
                        end_hidden: false,
                    });
                }
            }
            Kind::Spray => {
                let origin = match rng.below(3) {
                    0 => (c.w - 1.0, c.h / 2.0),
                    1 => (0.0, c.h / 2.0),
                    _ => (c.w / 2.0, c.h - 1.0),
                };
                for &h in c.home {
                    let col = rainbow(rng.f());
                    anims.push(Anim {
                        delay: rng.range(0.0, 2.4),
                        pre: None,
                        stages: vec![Stage::go(origin, h, dur(origin, h, rng.range(0.6, 1.4)).max(0.5), Ease::OutExpo, Glyph::Final, Col::ToFinal(col))],
                        end_hidden: false,
                    });
                }
            }
            Kind::Bouncy => {
                let greens = [Rgb::hex("d1f4a5"), Rgb::hex("96e2a4"), Rgb::hex("5acda9")];
                for i in 0..n {
                    let h = c.home[i];
                    let from = (h.0, -rng.range(1.0, c.h * 0.5));
                    let g = greens[rng.below(3)];
                    anims.push(Anim {
                        delay: (c.rows - 1 - c.row[i]) as f32 * 0.22 + rng.f() * 0.4,
                        pre: None,
                        stages: vec![
                            Stage::go(from, h, 1.3, Ease::OutBounce, Glyph::Fixed(rng.pick_char("*oO0.")), Col::Solid(g)),
                            Stage::still(h, 0.4, Glyph::Final, Col::ToFinal(g)),
                        ],
                        end_hidden: false,
                    });
                }
            }
            Kind::Print => {
                let mut t0 = 0.0;
                for r in 0..c.rows {
                    let mut last = 0usize;
                    for i in (0..n).filter(|&i| c.row[i] == r) {
                        let h = c.home[i];
                        last = last.max(c.col[i]);
                        anims.push(Anim {
                            delay: t0 + c.col[i] as f32 / 150.0,
                            pre: None,
                            stages: vec![
                                Stage::still(h, 0.08, Glyph::Seq("█▓▒░", 1), Col::Solid(WHITE)),
                                Stage::still(h, 0.3, Glyph::Final, Col::ToFinal(WHITE)),
                            ],
                            end_hidden: false,
                        });
                    }
                    t0 += last as f32 / 150.0 + 0.12;
                }
            }
            Kind::Matrix => {
                let coldelay: Vec<f32> = (0..c.cols + 1).map(|_| rng.range(0.0, 2.5)).collect();
                for i in 0..n {
                    let h = c.home[i];
                    let d = coldelay[c.col[i]] + c.row[i] as f32 * 0.06;
                    let resolve = 3.8 + rng.f() * 2.5;
                    anims.push(Anim {
                        delay: d,
                        pre: None,
                        stages: vec![
                            Stage::still(h, (resolve - d).max(0.2), Glyph::Rand(super::KATAKANA), Col::Solid(Rgb::hex("92be92").mix(Rgb::hex("185318"), rng.f()))),
                            Stage::still(h, 0.25, Glyph::Final, Col::Solid(Rgb::hex("dbffdb"))),
                            Stage::still(h, 0.5, Glyph::Final, Col::ToFinal(Rgb::hex("dbffdb"))),
                        ],
                        end_hidden: false,
                    });
                }
            }
            Kind::Vhs => {
                for i in 0..n {
                    let h = c.home[i];
                    let redraw = 2.8 + c.row[i] as f32 * 0.06;
                    anims.push(Anim {
                        delay: 0.0,
                        pre: None,
                        stages: vec![
                            Stage::still(h, redraw, Glyph::Final, Col::Grad(vec![Rgb::hex("ff3cfa"), Rgb::hex("3cf0ff"), Rgb::hex("ffffff").scale(0.7)])),
                            Stage::still(h, 0.07, Glyph::Fixed('█'), Col::Solid(WHITE)),
                            Stage::still(h, 0.25, Glyph::Final, Col::ToFinal(WHITE)),
                        ],
                        end_hidden: false,
                    });
                }
            }
            Kind::Colorshift => {
                for &h in c.home {
                    let off = dist(h, centre) / c.w.max(1.0);
                    anims.push(Anim {
                        delay: 0.0,
                        pre: None,
                        stages: vec![
                            Stage::still(h, 3.6, Glyph::Final, Col::Rainbow(off)),
                            Stage::still(h, 0.9, Glyph::Final, Col::ToFinal(rainbow(off + 3.6 * 0.6))),
                        ],
                        end_hidden: false,
                    });
                }
            }
        }
        let _ = hx;
        e.anims = anims;
        e.finish()
    }

    pub fn outro(kind: Outro, c: &Chars, rng: &mut Rng) -> Effect {
        let centre = (c.w / 2.0, c.h / 2.0);
        let mut anims = Vec::new();
        for i in 0..c.home.len() {
            let h = c.home[i];
            let pre = Some((h, Glyph::Final, Col::Final));
            let stages = match kind {
                Outro::Fade => {
                    let d = (c.col[i] + c.row[i] * 2) as f32 * 0.008;
                    vec![Stage::still(h, d, Glyph::Final, Col::Final), Stage::still(h, 1.0, Glyph::Final, Col::Bright(1.0, 0.0))]
                }
                Outro::Crumble => {
                    let floor = (h.0, c.h - 1.0);
                    vec![
                        Stage::still(h, rng.range(0.0, 2.0), Glyph::Final, Col::Bright(1.0, 0.65)),
                        Stage::still(h, 0.3, Glyph::Final, Col::Bright(0.65, 0.4)),
                        Stage::go(h, floor, 1.0, Ease::OutBounce, Glyph::Rand("*.,"), Col::Bright(0.4, 0.3)),
                        Stage::still(floor, 0.6, Glyph::Fixed('.'), Col::Bright(0.3, 0.0)),
                    ]
                }
                Outro::Implode => vec![
                    Stage::still(h, rng.range(0.0, 0.5), Glyph::Final, Col::Final),
                    Stage::go(h, centre, rng.range(0.7, 1.2), Ease::InExpo, Glyph::Final, Col::Grad(vec![Rgb::hex("ffffff").scale(0.9), WHITE])),
                ],
                Outro::Scatter => {
                    let a = rng.range(0.0, std::f32::consts::TAU);
                    let far = (centre.0 + a.cos() * c.w, centre.1 + a.sin() * c.h);
                    vec![Stage::go(h, far, rng.range(1.0, 1.6), Ease::InCubic, Glyph::Final, Col::Bright(1.0, 0.0))]
                }
                Outro::Dissolve => vec![
                    Stage::still(h, rng.range(0.0, 1.6), Glyph::Final, Col::Final),
                    Stage::still(h, 0.5, Glyph::Seq("▓▒░·", 1), Col::Bright(0.9, 0.0)),
                ],
                Outro::Rise => {
                    let up = (h.0 + rng.range(-2.0, 2.0), -2.0);
                    vec![
                        Stage::still(h, (c.rows - c.row[i]) as f32 * 0.08 + rng.f() * 0.3, Glyph::Final, Col::Final),
                        Stage::go(h, up, 1.0, Ease::InQuad, Glyph::Final, Col::Bright(1.0, 0.2)),
                    ]
                }
            };
            anims.push(Anim { delay: 0.0, pre: pre.clone(), stages, end_hidden: true });
        }
        Effect { anims, ..Effect::empty() }.finish()
    }

    pub fn step(&mut self, dt: f32) {
        self.t += dt;
        if self.kind == Some(Kind::Vhs) && self.t < 2.8 {
            let rows = self.row_shift.len();
            for r in 0..rows {
                if self.row_shift[r] != 0 && self.rng.chance(0.15) {
                    self.row_shift[r] = 0;
                } else if self.rng.chance(0.02) {
                    let s = self.rng.below(22) as i32 + 4;
                    self.row_shift[r] = if self.rng.chance(0.5) { s } else { -s };
                }
            }
            // the 3-row drifting glitch band
            let band = ((self.t * 2.3).sin() * 0.5 + 0.5) * rows as f32;
            let b = band as usize;
            for (k, s) in [(0usize, 8), (1, 14), (2, 8)] {
                if b + k < rows && self.rng.chance(0.5) {
                    self.row_shift[b + k] = s;
                }
            }
        } else {
            self.row_shift.iter_mut().for_each(|s| *s = 0);
        }
    }

    /// Where and how character `i` (final glyph `ch`, colour `fin`) looks now.
    pub fn look(&self, i: usize, ch: char, fin: Rgb, home: P) -> Look {
        let a = &self.anims[i];
        let t = self.t;
        let frame = (t / 0.07) as u32;
        let gl = |g: &Glyph, u: f32| -> char {
            match g {
                Glyph::Final => ch,
                Glyph::Fixed(c) => *c,
                Glyph::Seq(s, n) => {
                    let len = s.chars().count().max(1);
                    let k = ((u * (len as u32 * n) as f32) as usize).min(len * *n as usize - 1) % len;
                    s.chars().nth(k).unwrap_or(ch)
                }
                Glyph::Rand(s) => pick(s, hash(i as u32, frame)),
            }
        };
        let co = |c: &Col, u: f32| -> Rgb {
            match c {
                Col::Final => fin,
                Col::Solid(c) => *c,
                Col::Grad(v) => grad(v, u),
                Col::ToFinal(c) => c.mix(fin, u),
                Col::Bright(a, b) => fin.scale(a + (b - a) * u),
                Col::Rainbow(off) => rainbow(off + t * 0.6),
            }
        };
        if t < a.delay {
            return match &a.pre {
                Some((p, g, c)) => Look { pos: *p, glyph: gl(g, 0.0), col: co(c, 0.0), vis: true },
                None => Look { pos: home, glyph: ch, col: fin, vis: false },
            };
        }
        let mut s0 = a.delay;
        for st in &a.stages {
            if t < s0 + st.dur {
                let u = ((t - s0) / st.dur.max(1e-4)).clamp(0.0, 1.0);
                let e = ease(st.ease, u);
                let mut pos = match st.ctrl {
                    Some(cp) => super::bez(st.from, cp, st.to, e),
                    None => (st.from.0 + (st.to.0 - st.from.0) * e, st.from.1 + (st.to.1 - st.from.1) * e),
                };
                if st.jitter {
                    let h = hash(i as u32 ^ 0xabcd, (t / 0.05) as u32);
                    if (h % 1000) as f32 / 1000.0 < 0.08 + 0.7 * u {
                        pos.0 += ((h >> 10) % 3) as f32 - 1.0;
                        pos.1 += ((h >> 12) % 3) as f32 - 1.0;
                    }
                }
                return Look { pos, glyph: gl(&st.glyph, u), col: co(&st.col, u), vis: true };
            }
            s0 += st.dur;
        }
        if a.end_hidden {
            let last = a.stages.last().map(|s| s.to).unwrap_or(home);
            return Look { pos: last, glyph: ch, col: fin, vis: false };
        }
        Look { pos: home, glyph: ch, col: fin, vis: true }
    }
}

pub const CIPHER: &str = "!#$%&()*+-/<=>?@[]^_{|}~0123456789ABCDEFabcdef▖▗▘▙▚▛▜▝▞▟░▒▓";

fn shuffle<T>(v: &mut [T], rng: &mut Rng) {
    for i in (1..v.len()).rev() {
        let j = rng.below(i + 1);
        v.swap(i, j);
    }
}

/// Anim lists are indexed like the characters; rain builds them per row, so
/// put them back in character order.
fn sort_like(anims: &mut Vec<Anim>, c: &Chars, _f: fn(usize) -> usize) {
    let n = c.home.len();
    let mut order = Vec::with_capacity(n);
    for r in (0..c.rows).rev() {
        order.extend((0..n).filter(|&i| c.row[i] == r));
    }
    let mut out: Vec<Option<Anim>> = vec![None; n];
    for (k, a) in anims.drain(..).enumerate() {
        out[order[k]] = Some(a);
    }
    *anims = out.into_iter().map(|a| a.unwrap()).collect();
}

/// Breadth-first levels over 4-neighbour adjacency from a random character —
/// the burn front (TTE uses a spanning tree; BFS gives the same wavefront feel).
fn bfs_levels(c: &Chars, rng: &mut Rng) -> Vec<usize> {
    let n = c.home.len();
    let mut grid = vec![usize::MAX; (c.rows + 1) * (c.cols + 1)];
    for i in 0..n {
        grid[c.row[i] * (c.cols + 1) + c.col[i]] = i;
    }
    let mut level = vec![usize::MAX; n];
    let mut q = std::collections::VecDeque::new();
    let mut seeds = vec![rng.below(n.max(1))];
    let mut next_seed = 0;
    let mut maxl = 0;
    loop {
        for &s in &seeds {
            if s < n && level[s] == usize::MAX {
                level[s] = maxl;
                q.push_back(s);
            }
        }
        while let Some(i) = q.pop_front() {
            let (r, cc) = (c.row[i] as i32, c.col[i] as i32);
            for (dr, dc) in [(0, 1), (0, -1), (1, 0), (-1, 0), (1, 1), (-1, -1)] {
                let (nr, nc) = (r + dr, cc + dc);
                if nr < 0 || nc < 0 || nr as usize > c.rows || nc as usize > c.cols {
                    continue;
                }
                let j = grid[nr as usize * (c.cols + 1) + nc as usize];
                if j != usize::MAX && level[j] == usize::MAX {
                    level[j] = level[i] + 1;
                    maxl = maxl.max(level[j]);
                    q.push_back(j);
                }
            }
        }
        // disconnected pieces: continue from the next unreached character
        while next_seed < n && level[next_seed] != usize::MAX {
            next_seed += 1;
        }
        if next_seed >= n {
            break;
        }
        seeds = vec![next_seed];
    }
    level
}
