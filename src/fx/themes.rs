//! Music themes: what the banner does between a themed intro and its outro.
//!
//! The idle cycle (terminaltexteffects intro → hold → outro) keeps running
//! while music plays; only the hold changes. Each theme is one idea with its
//! own palette, glyph set and paired intros, drawn in plain character cells —
//! no half-block backdrops, no stacked layers — so it stays sharp:
//!
//! * levels:  the letters are the equaliser; each column pair fills from the
//!   bottom to its band's level, unlit parts stay dim.
//! * pulse:   brightness on the kick, colour steps on every beat, a specular
//!   sweep on the downbeat, hi-hats sparkle single letters.
//! * shock:   kicks send rings out from the centre that jolt the letters as
//!   they pass; snares fire a beam along one row.
//! * wave:    the waveform runs across the screen as a line, and through the
//!   banner by lifting its columns.
//! * fire:    character-cell fire (aafire's ramp) rising off the letter tops,
//!   flame height per column from its band, surging on kicks.
//! * matrix:  katakana rain, speed from loudness, new streams on hi-hats;
//!   letters light up where the rain runs through them.
//! * glitch:  hi-hats scramble letters, snares tear rows sideways, kicks split
//!   the banner into magenta / cyan ghosts.
//! * springs: every letter on a damped spring; kicks push, the bass bends.
//! * floor:   the banner steady over a cava bar floor.
//!
//! Every theme blends toward the plain banner as `react` falls to 0, so the
//! intro hands over and the outro takes over without a jump.

use super::banner::Ch;
use super::effects::{CIPHER, Kind};
use super::spectrum::Spectrum;
use super::{Ctx, KATAKANA, Rng};
use crate::canvas::Canvas;
use crate::color::{Gradient, Rgb, WHITE};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    Levels,
    Pulse,
    Shock,
    Wave,
    Fire,
    Matrix,
    Glitch,
    Springs,
    Floor,
}

pub const ALL: [Theme; 9] = [
    Theme::Levels,
    Theme::Pulse,
    Theme::Shock,
    Theme::Wave,
    Theme::Fire,
    Theme::Matrix,
    Theme::Glitch,
    Theme::Springs,
    Theme::Floor,
];

const FIRE: [&str; 6] = ["1a0000", "510100", "8A003C", "fe650d", "fff75d", "ffffff"];
const BEAMS: [&str; 3] = ["8A008A", "00D1FF", "ffffff"];
const WAVES: [&str; 3] = ["31a0d4", "f0ff65", "ffb102"];
const GREENS: [&str; 3] = ["185318", "3cb043", "92be92"];
const VHS: [&str; 2] = ["ff3cfa", "3cf0ff"];
/// aafire's ramp, cold → hot.
const FLAME: [char; 10] = ['.', ',', ':', '^', '*', 'x', 's', 'S', '#', '$'];
/// Beam trail, TTE's beams glyphs.
const BEAM_TRAIL: [char; 3] = ['▂', '▁', '_'];

impl Theme {
    pub fn name(self) -> String {
        format!("{self:?}").to_lowercase()
    }

    pub fn from_name(s: &str) -> Option<Theme> {
        ALL.iter().copied().find(|t| t.name().eq_ignore_ascii_case(s))
    }

    /// Intros that set the theme up.
    pub fn intros(self) -> &'static [Kind] {
        match self {
            Theme::Levels => &[Kind::Rain, Kind::Waves],
            Theme::Pulse => &[Kind::Expand, Kind::Colorshift],
            Theme::Shock => &[Kind::Beams, Kind::Blackhole],
            Theme::Wave => &[Kind::Waves, Kind::Slide],
            Theme::Fire => &[Kind::Burn],
            Theme::Matrix => &[Kind::Matrix, Kind::Decrypt],
            Theme::Glitch => &[Kind::Vhs, Kind::Decrypt],
            Theme::Springs => &[Kind::Bouncy, Kind::Unstable],
            Theme::Floor => &[Kind::Print, Kind::Spray, Kind::Fireworks],
        }
    }

    /// The letters' final colours; None = the album palette.
    pub fn palette(self) -> Option<Gradient> {
        let g = |s: &[&str]| Some(Gradient::new(&s.iter().map(|h| Rgb::hex(h)).collect::<Vec<_>>()));
        match self {
            Theme::Shock => g(&BEAMS[..2]),
            Theme::Wave => g(&WAVES),
            Theme::Fire => g(&FIRE[2..5]),
            Theme::Matrix => g(&GREENS),
            Theme::Glitch => g(&VHS),
            _ => None,
        }
    }

    /// 0 calm .. 1 busy, matched against the music's energy.
    pub fn busy(self) -> f32 {
        match self {
            Theme::Pulse => 0.15,
            Theme::Wave => 0.25,
            Theme::Levels => 0.4,
            Theme::Floor => 0.45,
            Theme::Matrix => 0.55,
            Theme::Springs => 0.65,
            Theme::Shock => 0.75,
            Theme::Glitch => 0.8,
            Theme::Fire => 0.9,
        }
    }
}

/// Where the banner sits.
#[derive(Clone, Copy)]
pub struct Geom {
    pub ox: i32,
    pub oy: i32,
    pub bw: usize,
    pub bh: usize,
}

struct Drop {
    x: i32,
    y: f32,
    speed: f32,
    len: usize,
    seed: u32,
}

/// Per-theme state for one hold; rebuilt when a theme starts.
pub struct Hold {
    pub theme: Theme,
    rng: Rng,
    t: f32,
    flash: f32,
    sweep: f32,
    off: f32,
    beats: u64,
    since_kick: f32,
    sparks: Vec<(usize, f32)>,
    rings: Vec<(f32, f32)>,
    beams: Vec<(i32, f32, bool)>,
    peak: f32,
    heat: Vec<f32>,
    hw: usize,
    fire_acc: f32,
    surge: f32,
    drops: Vec<Drop>,
    wet: Vec<f32>,
    tears: Vec<(usize, i32, f32)>,
    scramble: f32,
    stiff: f32,
}

fn hex(h: &str) -> Rgb {
    Rgb::hex(h)
}

fn ramp(stops: &[&str], u: f32) -> Rgb {
    let x = u.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let i = (x as usize).min(stops.len() - 2);
    hex(stops[i]).mix(hex(stops[i + 1]), x - i as f32)
}

fn hash(a: u32, b: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9E3779B1) ^ b.wrapping_mul(0x85EBCA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B3C6D);
    h ^ (h >> 12)
}

fn nth(set: &str, h: u32) -> char {
    let n = set.chars().count().max(1);
    set.chars().nth(h as usize % n).unwrap_or('?')
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

impl Hold {
    pub fn new(theme: Theme) -> Hold {
        Hold {
            theme,
            rng: Rng::seeded(),
            t: 0.0,
            flash: 0.0,
            sweep: -1.0,
            off: 0.0,
            beats: 0,
            since_kick: 1.0,
            sparks: Vec::new(),
            rings: Vec::new(),
            beams: Vec::new(),
            peak: 0.05,
            heat: Vec::new(),
            hw: 0,
            fire_acc: 0.0,
            surge: 0.0,
            drops: Vec::new(),
            wet: Vec::new(),
            tears: Vec::new(),
            scramble: 0.0,
            stiff: 60.0,
        }
    }

    pub fn draw(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &mut [Ch], g: Geom, spec: &mut Spectrum, react: f32) {
        let f = cx.f;
        self.t += cx.dt;
        if f.drop {
            self.flash = 1.0;
        }
        self.flash *= (-cx.dt / 0.3).exp();
        match self.theme {
            Theme::Levels => self.levels(cv, cx, chars, g, react),
            Theme::Pulse => self.pulse(cv, cx, chars, g, react),
            Theme::Shock => self.shock(cv, cx, chars, g, react),
            Theme::Wave => self.wave(cv, cx, chars, g, react),
            Theme::Fire => self.fire(cv, cx, chars, g, react),
            Theme::Matrix => self.matrix(cv, cx, chars, react),
            Theme::Glitch => self.glitch(cv, cx, chars, g, react),
            Theme::Springs => self.springs(cv, cx, chars, g, react),
            Theme::Floor => self.floor(cv, cx, chars, g, spec, react),
        }
    }

    /// Shared finish: drop flash, then the letter itself.
    fn letter(&self, cv: &mut Canvas, x: i32, y: i32, ch: char, col: Rgb) {
        cv.put(x, y, ch, col.mix(WHITE, 0.6 * self.flash));
    }

    /// Downbeat specular sweep, a diagonal band running across the banner.
    fn step_sweep(&mut self, cx: &Ctx) {
        if cx.f.beat && cx.f.bar_beat() == 0 {
            self.sweep = 0.0;
        }
        if self.sweep >= 0.0 {
            self.sweep += cx.dt;
            if self.sweep > 0.9 {
                self.sweep = -1.0;
            }
        }
    }

    fn sweep_at(&self, c: &Ch, g: Geom) -> f32 {
        if self.sweep < 0.0 {
            return 0.0;
        }
        let d = (c.dx as f32 / g.bw as f32 + c.dy as f32 / g.bh as f32 * 0.35) - (self.sweep / 0.9 * 1.6 - 0.3);
        (-(d * d) * 50.0).exp()
    }

    // ------------------------------------------------------------ levels

    fn levels(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], g: Geom, react: f32) {
        let f = cx.f;
        let bh = g.bh as f32;
        // two banner columns per bar reads as bars, not noise
        let bars = g.bw.div_ceil(2).max(1) as f32;
        for c in chars {
            let u = ((c.dx / 2) as f32 + 0.5) / bars;
            let lit = sample(&f.mono, u) * bh * 1.12;
            let cap = (sample(&f.caps, u) * bh * 1.12) as usize;
            let lit = bh * 1.2 + (lit - bh * 1.2) * react; // react 0: everything lit
            let up = (g.bh - 1 - c.dy) as f32; // rows from the bottom
            let col = if up < lit {
                let edge = lit - up < 1.0;
                c.fin.mix(WHITE, if edge { 0.55 * react } else { 0.0 })
            } else if up as usize == cap && cap > 0 {
                c.fin.scale(1.0 - 0.3 * react)
            } else {
                c.fin.scale(1.0 - 0.87 * react)
            };
            self.letter(cv, c.home.0 as i32, c.home.1 as i32, c.ch, col);
        }
    }

    // ------------------------------------------------------------- pulse

    fn pulse(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], g: Geom, react: f32) {
        let f = cx.f;
        if f.beat {
            self.beats += 1;
        }
        // colour steps a quarter turn per beat, eased over ~60 ms
        let target = self.beats as f32 * 0.25;
        self.off += (target - self.off) * (1.0 - (-cx.dt / 0.06).exp());
        self.step_sweep(cx);
        if f.hat > 0.2 && !chars.is_empty() {
            for _ in 0..(2.0 + f.hat * 6.0) as usize {
                self.sparks.push((self.rng.below(chars.len()), 0.12));
            }
        }
        self.sparks.iter_mut().for_each(|s| s.1 -= cx.dt);
        self.sparks.retain(|s| s.1 > 0.0);
        let bright = 0.45 + 0.55 * f.kick_env;
        let mut spark = vec![false; chars.len()];
        for s in &self.sparks {
            spark[s.0] = true;
        }
        for (i, c) in chars.iter().enumerate() {
            let u = c.dx as f32 / g.bw as f32;
            let mut col = cx.grad.wrap(u * 0.5 + self.off).scale(bright).mix(WHITE, 0.8 * self.sweep_at(c, g));
            if spark[i] {
                col = WHITE;
            }
            self.letter(cv, c.home.0 as i32, c.home.1 as i32, c.ch, c.fin.mix(col, react));
        }
    }

    // ------------------------------------------------------------- shock

    fn shock(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], g: Geom, react: f32) {
        let f = cx.f;
        let centre = (g.ox as f32 + g.bw as f32 / 2.0, g.oy as f32 + g.bh as f32 / 2.0);
        let maxr = (cx.w as f32).hypot(cx.h as f32 * 2.0) * 0.55;
        self.since_kick += cx.dt;
        if f.kick > 0.25 && self.since_kick > 0.18 && react > 0.5 {
            self.rings.push((0.0, f.kick.min(1.5)));
            self.since_kick = 0.0;
        }
        if f.drop {
            self.rings.push((0.0, 2.5));
            self.rings.push((-0.12, 2.0));
        }
        if f.snare > 0.4 && self.beams.len() < 3 && react > 0.5 {
            let row = g.oy + self.rng.below(g.bh) as i32;
            self.beams.push((row, 0.0, self.rng.chance(0.5)));
        }
        let speed = cx.w as f32 * 0.75;
        self.rings.iter_mut().for_each(|r| r.0 += cx.dt);
        self.rings.retain(|r| r.0 * speed < maxr);
        self.beams.iter_mut().for_each(|b| b.1 += cx.dt / 0.4);
        self.beams.retain(|b| b.1 < 1.3);

        // rings: one glyph per cell along the circle (rows count double)
        for &(age, s) in &self.rings {
            if age < 0.0 {
                continue;
            }
            let r = age * speed;
            let life = 1.0 - r / maxr;
            let ch = if life > 0.75 { '•' } else if life > 0.4 { '∙' } else { '·' };
            let col = ramp(&BEAMS, 0.35 + 0.65 * life).scale((0.3 + 0.7 * life) * s.min(1.0) * react);
            let steps = (r * 7.0).max(12.0) as usize;
            for k in 0..steps {
                let a = k as f32 / steps as f32 * std::f32::consts::TAU;
                cv.put((centre.0 + a.cos() * r).round() as i32, (centre.1 + a.sin() * r * 0.5).round() as i32, ch, col);
            }
        }
        // beams: a head with a short trail running the full width
        let mut lit_rows: Vec<(i32, f32, bool)> = Vec::new();
        for &(row, p, rev) in &self.beams {
            let head = p * (cx.w as f32 + 12.0);
            let hx = if rev { cx.w as f32 - head } else { head };
            for k in 0..12 {
                let x = if rev { hx + k as f32 } else { hx - k as f32 };
                let ch = BEAM_TRAIL[(k / 4).min(2)];
                cv.put(x as i32, row, ch, ramp(&BEAMS, 1.0 - k as f32 / 12.0).scale(react));
            }
            lit_rows.push((row, hx, rev));
        }
        for c in chars {
            let (dx, dy) = (c.home.0 - centre.0, (c.home.1 - centre.1) * 2.0);
            let d = dx.hypot(dy);
            let mut hl = 0.0f32;
            for &(age, s) in &self.rings {
                let r = age * speed;
                if age >= 0.0 && (d - r).abs() < 1.5 {
                    hl = hl.max((s * (1.0 - r / maxr)).min(1.0));
                }
            }
            for &(row, hx, rev) in &lit_rows {
                if row == c.home.1 as i32 {
                    let behind = if rev { c.home.0 - hx } else { hx - c.home.0 };
                    if behind >= 0.0 {
                        hl = hl.max((1.0 - behind / 30.0).max(0.0));
                    }
                }
            }
            hl *= react;
            let push = (hl * 1.2).round();
            let x = c.home.0 + if dx >= 0.0 { push } else { -push };
            let col = c.fin.scale(1.0 - 0.5 * react + 0.5 * hl).mix(WHITE, 0.6 * hl);
            self.letter(cv, x as i32, c.home.1 as i32, c.ch, col);
        }
    }

    // -------------------------------------------------------------- wave

    fn wave(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], g: Geom, react: f32) {
        let f = cx.f;
        let s = &f.wave_l;
        let (w, h) = (cx.w, cx.h);
        let span = s.len() / 2;
        if span < 64 || w == 0 {
            return self.plain(cv, chars, react);
        }
        // trigger on a rising zero crossing so the picture holds still
        let mut trig = 0;
        let mut lp = 0.0f32;
        for (i, &v) in s[..span].iter().enumerate() {
            let prev = lp;
            lp += (v - lp) * 0.1;
            if i > 8 && prev <= 0.0 && lp > 0.0 {
                trig = i;
                break;
            }
        }
        // box-filter each column's slice of samples (a cheap low-pass)
        let per = (span / w).max(1);
        let mut ys: Vec<f32> = (0..w)
            .map(|x| {
                let a = trig + x * span / w;
                let b = (a + per).min(s.len());
                s[a..b].iter().sum::<f32>() / (b - a).max(1) as f32
            })
            .collect();
        let pk = ys.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        self.peak = pk.max(self.peak * (-cx.dt / 1.5).exp()).max(1e-4);
        let amp = ((h as f32 - g.bh as f32) / 2.0 - 1.0).clamp(1.0, 8.0) * react * (0.6 + 0.4 * f.loud);
        for y in &mut ys {
            *y = *y / self.peak * amp;
        }
        let cy = g.oy as f32 + g.bh as f32 / 2.0;
        // the line, outside the banner: ▔ ─ ▁ give three heights per row
        let mut prev: Option<i32> = None;
        for (x, &v) in ys.iter().enumerate() {
            let xi = x as i32;
            if xi >= g.ox - 1 && xi <= g.ox + g.bw as i32 {
                prev = None;
                continue;
            }
            let yc = cy - v;
            let row = yc.floor();
            let fr = yc - row;
            let ch = if fr < 0.34 { '▔' } else if fr < 0.67 { '─' } else { '▁' };
            let col = ramp(&WAVES, 0.5 + v / (2.0 * amp.max(1.0))).scale(react);
            let r = row as i32;
            if let Some(p) = prev {
                // join steep steps so the line never breaks
                for y in (p.min(r) + 1)..p.max(r) {
                    cv.put(xi, y, '│', col.scale(0.7));
                }
            }
            cv.put(xi, r, ch, col);
            prev = Some(r);
        }
        for c in chars {
            let x = c.home.0 as usize;
            let v = ys.get(x).copied().unwrap_or(0.0);
            let col = ramp(&WAVES, 0.5 + v / (2.0 * amp.max(1.0)));
            self.letter(cv, c.home.0 as i32, (c.home.1 - v).round() as i32, c.ch, c.fin.mix(col, react * 0.7));
        }
    }

    // -------------------------------------------------------------- fire

    fn fire(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], g: Geom, react: f32) {
        let f = cx.f;
        let (w, h) = (cx.w, cx.h);
        if self.hw != w || self.heat.len() != w * h {
            self.hw = w;
            self.heat = vec![0.0; w * h];
        }
        if f.drop {
            self.surge = 1.0;
        }
        self.surge *= (-cx.dt / 1.2).exp();
        // the top letter of each banner column is a burner
        let mut top = vec![usize::MAX; g.bw + 1];
        for (i, c) in chars.iter().enumerate() {
            if top[c.dx] == usize::MAX || chars[top[c.dx]].dy > c.dy {
                top[c.dx] = i;
            }
        }
        // fixed 30 Hz so the flame height doesn't depend on the frame rate
        self.fire_acc += cx.dt;
        while self.fire_acc >= 1.0 / 30.0 {
            self.fire_acc -= 1.0 / 30.0;
            for &i in top.iter().filter(|&&i| i != usize::MAX) {
                let c = &chars[i];
                let u = c.dx as f32 / g.bw as f32;
                let band = sample(&f.mono, (u - 0.5).abs() * 2.0); // bass in the middle
                let src = ((0.55 + 0.9 * band) * (0.75 + 0.5 * f.kick_env) * (1.0 + 1.5 * self.surge) * react).min(1.8);
                let (x, y) = (c.home.0 as i32, c.home.1 as i32 - 1);
                if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                    let p = &mut self.heat[y as usize * w + x as usize];
                    *p = p.max(src * self.rng.range(0.7, 1.0));
                }
            }
            // heat rises one row, wanders a column, and cools
            for y in 0..h - 1 {
                for x in 0..w {
                    let sx = (x as i32 + self.rng.below(3) as i32 - 1).clamp(0, w as i32 - 1) as usize;
                    let below = self.heat[(y + 1) * w + sx];
                    let here = &mut self.heat[y * w + x];
                    *here = (below.max(*here * 0.3) - self.rng.f() * 0.1).max(0.0);
                }
            }
            for x in 0..w {
                self.heat[(h - 1) * w + x] = 0.0;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let v = self.heat[y * w + x];
                if v < 0.06 {
                    continue;
                }
                let k = ((v.min(1.0) * FLAME.len() as f32) as usize).min(FLAME.len() - 1);
                cv.put(x as i32, y as i32, FLAME[k], ramp(&FIRE, 0.15 + v.min(1.0) * 0.85));
            }
        }
        for c in chars {
            let glow = ramp(&FIRE, 0.6 + 0.35 * f.kick_env);
            self.letter(cv, c.home.0 as i32, c.home.1 as i32, c.ch, c.fin.mix(glow, react * 0.6));
        }
    }

    // ------------------------------------------------------------ matrix

    fn matrix(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], react: f32) {
        let f = cx.f;
        let (w, h) = (cx.w, cx.h);
        let cap = w * 3 / 4;
        let mut spawn = (cx.dt * (2.0 + 22.0 * f.energy)) as usize + usize::from(self.rng.chance((cx.dt * (2.0 + 22.0 * f.energy)).fract()));
        if f.hat > 0.15 {
            spawn += (f.hat * 5.0) as usize;
        }
        if f.drop {
            spawn += w / 3;
        }
        if react < 0.5 {
            spawn = 0;
        }
        for _ in 0..spawn {
            if self.drops.len() >= cap {
                break;
            }
            self.drops.push(Drop {
                x: self.rng.below(w) as i32,
                y: -1.0,
                speed: self.rng.range(10.0, 28.0),
                len: 6 + self.rng.below(16),
                seed: self.rng.u32(),
            });
        }
        let mult = 0.55 + 0.8 * f.loud;
        for d in &mut self.drops {
            d.y += d.speed * mult * cx.dt;
        }
        self.drops.retain(|d| (d.y as i32 - d.len as i32) < h as i32);
        self.wet.clear();
        self.wet.resize(w * h, 0.0);
        let frame = (self.t * 12.0) as u32;
        for d in &self.drops {
            let head = d.y as i32;
            for k in 0..d.len as i32 {
                let y = head - k;
                if y < 0 || y >= h as i32 || d.x as usize >= w {
                    continue;
                }
                let fade = 1.0 - k as f32 / d.len as f32;
                let ch = nth(KATAKANA, hash(d.seed ^ y as u32, if k == 0 { frame } else { frame / 6 }));
                let col = if k == 0 { hex("dbffdb") } else { ramp(&GREENS, fade) .scale(0.35 + 0.65 * fade) };
                cv.put(d.x, y, ch, col.scale(react));
                let i = y as usize * w + d.x as usize;
                self.wet[i] = self.wet[i].max(if k == 0 { 1.0 } else { fade * 0.8 });
            }
        }
        for c in chars {
            let (x, y) = (c.home.0 as i32, c.home.1 as i32);
            let wet = cv.idx(x, y).map(|i| self.wet[i]).unwrap_or(0.0);
            let b = 0.3 + 0.7 * wet + 0.2 * f.kick_env;
            let col = c.fin.scale(b.min(1.0)).mix(WHITE, 0.5 * (wet - 0.9).max(0.0) * 10.0);
            self.letter(cv, x, y, c.ch, c.fin.mix(col, react));
        }
    }

    // ------------------------------------------------------------ glitch

    fn glitch(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], g: Geom, react: f32) {
        let f = cx.f;
        if f.drop {
            self.scramble = 0.6;
        }
        self.scramble -= cx.dt;
        if f.snare > 0.4 && self.tears.len() < 3 && react > 0.5 {
            for _ in 0..1 + self.rng.below(3) {
                let s = (2 + self.rng.below(7)) as i32 * if self.rng.chance(0.5) { 1 } else { -1 };
                self.tears.push((self.rng.below(g.bh), s, self.rng.range(0.06, 0.12)));
            }
        }
        self.tears.iter_mut().for_each(|t| t.2 -= cx.dt);
        self.tears.retain(|t| t.2 > 0.0);
        let mut shift = vec![0i32; g.bh];
        for &(r, s, _) in &self.tears {
            shift[r] = s;
        }
        let p = if self.scramble > 0.0 { 1.0 } else { (0.01 + 0.4 * f.hat_env + 0.15 * (f.treb_att - 1.0).max(0.0)).min(0.6) } * react;
        let split = (f.kick_env * 2.4 * react).round() as i32;
        let frame = (self.t * 15.0) as u32;
        let (mag, cyan) = (hex(VHS[0]), hex(VHS[1]));
        if split > 0 {
            for c in chars {
                let (x, y) = (c.home.0 as i32 + shift[c.dy], c.home.1 as i32);
                cv.put(x - split, y, c.ch, mag.scale(0.75));
                cv.put(x + split, y, c.ch, cyan.scale(0.75));
            }
        }
        for (i, c) in chars.iter().enumerate() {
            let h = hash(i as u32, frame);
            let scrambled = (h % 1000) as f32 / 1000.0 < p;
            let ch = if scrambled { nth(CIPHER, h >> 10) } else { c.ch };
            let col = if scrambled {
                if h & 1 == 0 { mag } else { cyan }
            } else if shift[c.dy] != 0 {
                c.fin.mix(cyan, 0.5)
            } else {
                c.fin.mix(WHITE, 0.3 * react)
            };
            self.letter(cv, c.home.0 as i32 + shift[c.dy], c.home.1 as i32, ch, col);
        }
    }

    // ----------------------------------------------------------- springs

    fn springs(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &mut [Ch], g: Geom, react: f32) {
        let f = cx.f;
        let dt = cx.dt;
        let centre = (g.ox as f32 + g.bw as f32 / 2.0, g.oy as f32 + g.bh as f32 / 2.0);
        if f.drop {
            for c in chars.iter_mut() {
                let ang = self.rng.range(0.0, std::f32::consts::TAU);
                let sp = self.rng.range(30.0, 110.0);
                let (dx, dy) = (c.home.0 - centre.0, c.home.1 - centre.1);
                let l = (dx * dx + dy * dy * 4.0).sqrt().max(1.0);
                c.vel = (dx / l * sp * 0.7 + ang.cos() * sp * 0.5, dy / l * sp * 0.35 + ang.sin() * sp * 0.3);
            }
            self.stiff = 3.0;
        }
        self.stiff += (60.0 - self.stiff) * (1.0 - (-dt / 1.4).exp());
        let k = self.stiff;
        let damp = 2.0 * k.sqrt() * 0.55;
        let wob = f.bass_att.min(2.5) * 0.45;
        self.step_sweep(cx);
        let pulse = f.kick_env;
        for c in chars.iter_mut() {
            if f.kick > 0.2 {
                let (dx, dy) = (c.home.0 - centre.0, c.home.1 - centre.1);
                let l = (dx * dx + dy * dy * 4.0).sqrt().max(1.0);
                c.vel.0 += dx / l * f.kick * 3.0;
                c.vel.1 += dy / l * f.kick * 1.2;
            }
            let target = (c.home.0, c.home.1 + wob * ((c.dx as f32 * 0.16) - cx.t * 5.0).sin());
            let ax = k * (target.0 - c.pos.0) - damp * c.vel.0;
            let ay = k * (target.1 - c.pos.1) - damp * c.vel.1;
            c.vel.0 += ax * dt;
            c.vel.1 += ay * dt;
            c.pos.0 += c.vel.0 * dt;
            c.pos.1 += c.vel.1 * dt;
        }
        for c in chars.iter() {
            let x = c.home.0 + (c.pos.0 - c.home.0) * react;
            let y = c.home.1 + (c.pos.1 - c.home.1) * react;
            let col = c.fin.scale(0.7 + 0.3 * pulse).mix(WHITE, 0.25 * pulse + 0.8 * self.sweep_at(c, g));
            self.letter(cv, x.round() as i32, y.round() as i32, c.ch, c.fin.mix(col, react));
        }
    }

    // ------------------------------------------------------------- floor

    fn floor(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], g: Geom, spec: &mut Spectrum, react: f32) {
        let f = cx.f;
        let rows = cx.h as i32 - (g.oy + g.bh as i32) - 2;
        if rows >= 3 {
            spec.floor(cv, cx, rows as usize, react);
        }
        self.step_sweep(cx);
        for c in chars {
            let col = c.fin.scale(0.75 + 0.25 * f.kick_env).mix(WHITE, 0.8 * self.sweep_at(c, g));
            self.letter(cv, c.home.0 as i32, c.home.1 as i32, c.ch, c.fin.mix(col, react));
        }
    }

    fn plain(&self, cv: &mut Canvas, chars: &[Ch], _react: f32) {
        for c in chars {
            self.letter(cv, c.home.0 as i32, c.home.1 as i32, c.ch, c.fin);
        }
    }
}
