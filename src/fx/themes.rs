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
//! * wave:    a mirrored waveform rolls in from both screen edges toward the
//!   banner, faster the busier the music; the letters stay put and light up
//!   as each swell arrives.
//! * fire:    character-cell fire (the classic ASCII ramp) rising off the letter tops,
//!   flame height per column from its band, surging on kicks.
//! * matrix:  katakana rain, speed from loudness, new streams on hi-hats;
//!   letters light up where the rain runs through them.
//! * glitch:  hi-hats scramble letters, snares tear rows sideways, kicks split
//!   the banner into magenta / cyan ghosts.
//! * springs: every letter on a damped spring; kicks push, the bass bends.
//! * floor:   the banner steady over a cava bar floor (only with --theme:
//!   it replaces the ribbon, and that swap doesn't transition well).
//! * bounce:  the whole banner flies around the screen, faster with the
//!   music, kicks swerving it, a rainbow trail behind it; every wall it hits
//!   jumps the palette.
//! * warp:    hyperspace: glyphs stream out of the banner's edges and
//!   accelerate away; density follows intensity, kicks burst, drops flood.
//!
//! Colours come from the shared `Look` (the letters' `fin` is refreshed from
//! it every frame), so the theme's accents and the ribbon always match.
//!
//! Before a switch, a theme winds its own layer down (no new rain, fire,
//! stars or rings; what's there fades) and brings moved letters home.
//!
//! Every theme blends toward the plain banner as `react` falls to 0, so the
//! intro hands over and the outro takes over without a jump.

use super::banner::Ch;
use super::effects::{CIPHER, Kind};
use super::spectrum::Spectrum;
use super::{Ctx, KATAKANA, Look, Rng, hash, pick, ramp, sample};
use crate::canvas::Canvas;
use crate::color::{Rgb, WHITE};

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
    Bounce,
    Warp,
}

/// The rotation. `floor` swaps the ribbon for its own bars, so it only runs
/// when asked for (`--theme floor`).
pub const ALL: [Theme; 10] = [
    Theme::Levels,
    Theme::Pulse,
    Theme::Shock,
    Theme::Wave,
    Theme::Fire,
    Theme::Matrix,
    Theme::Glitch,
    Theme::Springs,
    Theme::Bounce,
    Theme::Warp,
];

const FIRE: [Rgb; 6] = [
    Rgb::from_hex(0x1a0000),
    Rgb::from_hex(0x510100),
    Rgb::from_hex(0x8A003C),
    Rgb::from_hex(0xfe650d),
    Rgb::from_hex(0xfff75d),
    Rgb::from_hex(0xffffff),
];
const GREENS: [Rgb; 3] = [Rgb::from_hex(0x185318), Rgb::from_hex(0x3cb043), Rgb::from_hex(0x92be92)];
const VHS: [Rgb; 2] = [Rgb::from_hex(0xff3cfa), Rgb::from_hex(0x3cf0ff)];
/// The usual ASCII-fire ramp, cold → hot.
const FLAME: [char; 10] = ['.', ',', ':', '^', '*', 'x', 's', 'S', '#', '$'];
/// Beam trail, TTE's beams glyphs.
const BEAM_TRAIL: [char; 3] = ['▂', '▁', '_'];

impl Theme {
    pub fn name(self) -> String {
        format!("{self:?}").to_lowercase()
    }

    pub fn from_name(s: &str) -> Option<Theme> {
        ALL.iter().chain(&[Theme::Floor]).copied().find(|t| t.name().eq_ignore_ascii_case(s))
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
            Theme::Bounce => &[Kind::Slide, Kind::Spray, Kind::Bouncy],
            Theme::Warp => &[Kind::Blackhole, Kind::Expand, Kind::Fireworks],
        }
    }

    /// The letters' final colour stops; None = one built round a random hue.
    pub fn palette(self) -> Option<Vec<Rgb>> {
        match self {
            Theme::Fire => Some(FIRE[2..5].to_vec()),
            Theme::Matrix => Some(GREENS.to_vec()),
            Theme::Glitch => Some(VHS.to_vec()),
            _ => None,
        }
    }

    /// 0 calm .. 1 busy, matched against the music's energy.
    pub fn busy(self) -> f32 {
        match self {
            Theme::Pulse => 0.15,
            Theme::Wave => 0.3,
            Theme::Levels => 0.4,
            Theme::Floor => 0.45,
            Theme::Matrix => 0.55,
            Theme::Springs => 0.6,
            Theme::Bounce => 0.65,
            Theme::Shock => 0.75,
            Theme::Glitch => 0.8,
            Theme::Warp => 0.85,
            Theme::Fire => 0.9,
        }
    }

    /// Extra palette step per beat, for themes whose idea is the colour.
    pub fn beat_step(self) -> f32 {
        match self {
            Theme::Pulse => 0.25,
            _ => 0.0,
        }
    }
}

/// How long a theme's own layer takes to wind down before a switch.
const WIND_DOWN: f32 = 0.9;

/// Where the banner sits.
#[derive(Clone, Copy)]
pub struct Geom {
    pub ox: i32,
    pub oy: i32,
    pub bw: usize,
    pub bh: usize,
}

struct Star {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    u: f32,
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
    /// A switch is waiting: stop moving the letters and bring them home,
    /// and wind the theme's own layer (rain, fire, stars…) down to nothing.
    pub homing: bool,
    wind: f32,
    rng: Rng,
    t: f32,
    flash: f32,
    sweep: f32,
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
    /// wave: per-column level history, outer edge first (left, right).
    hist: Vec<(f32, f32)>,
    tick: f32,
    /// bounce: offset from home, unit heading, trail of past offsets.
    off: (f32, f32),
    head: (f32, f32),
    trail: Vec<(f32, f32)>,
    stars: Vec<Star>,
    spawn_acc: f32,
}


impl Hold {
    pub fn new(theme: Theme) -> Hold {
        Hold {
            theme,
            homing: false,
            wind: 1.0,
            rng: Rng::seeded(),
            t: 0.0,
            flash: 0.0,
            sweep: -1.0,
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
            hist: Vec::new(),
            tick: 0.0,
            off: (0.0, 0.0),
            head: (0.0, 0.0),
            trail: Vec::new(),
            stars: Vec::new(),
            spawn_acc: 0.0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &mut [Ch], g: Geom, spec: &mut Spectrum, react: f32, look: &mut Look) {
        let f = cx.f;
        self.t += cx.dt;
        if self.homing {
            self.wind = (self.wind - cx.dt / WIND_DOWN).max(0.0);
        }
        if f.drop && !self.homing {
            self.flash = 1.0;
        }
        self.flash *= (-cx.dt / 0.3).exp();
        match self.theme {
            Theme::Levels => self.levels(cv, cx, chars, g, react),
            Theme::Pulse => self.pulse(cv, cx, chars, g, react),
            Theme::Shock => self.shock(cv, cx, chars, g, react, look),
            Theme::Wave => self.wave(cv, cx, chars, g, react, look),
            Theme::Fire => self.fire(cv, cx, chars, g, react),
            Theme::Matrix => self.matrix(cv, cx, chars, react),
            Theme::Glitch => self.glitch(cv, cx, chars, g, react),
            Theme::Springs => self.springs(cv, cx, chars, g, react),
            Theme::Floor => self.floor(cv, cx, chars, g, spec, react, look),
            Theme::Bounce => self.bounce(cv, cx, chars, g, react, look),
            Theme::Warp => self.warp(cv, cx, chars, g, react, look),
        }
    }

    /// True once every letter is back at its home cell, so a switch won't
    /// make the banner jump.
    pub fn at_home(&self, chars: &[Ch]) -> bool {
        let layered = matches!(self.theme, Theme::Shock | Theme::Wave | Theme::Fire | Theme::Matrix | Theme::Warp | Theme::Floor);
        if layered && self.wind > 0.0 {
            return false;
        }
        match self.theme {
            Theme::Bounce => self.off == (0.0, 0.0) && self.trail.is_empty(),
            Theme::Springs => chars.iter().all(|c| {
                (c.pos.0 - c.home.0).abs() < 0.4 && (c.pos.1 - c.home.1).abs() < 0.4 && c.vel.0.hypot(c.vel.1) < 2.0
            }),
            Theme::Glitch => self.tears.is_empty(),
            _ => true,
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
        // the colour steps a quarter turn per beat (Theme::beat_step)
        let f = cx.f;
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
            let mut col = c.fin.scale(bright).mix(WHITE, 0.8 * self.sweep_at(c, g));
            if spark[i] {
                col = WHITE;
            }
            self.letter(cv, c.home.0 as i32, c.home.1 as i32, c.ch, c.fin.mix(col, react));
        }
    }

    // ------------------------------------------------------------- shock

    fn shock(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], g: Geom, react: f32, look: &Look) {
        let f = cx.f;
        let centre = (g.ox as f32 + g.bw as f32 / 2.0, g.oy as f32 + g.bh as f32 / 2.0);
        let maxr = (cx.w as f32).hypot(cx.h as f32 * 2.0) * 0.55;
        let layer = react * self.wind;
        self.since_kick += cx.dt;
        if f.kick > 0.25 && self.since_kick > 0.18 && layer > 0.5 {
            self.rings.push((0.0, f.kick.min(1.5)));
            self.since_kick = 0.0;
        }
        if f.drop && !self.homing {
            self.rings.push((0.0, 2.5));
            self.rings.push((-0.12, 2.0));
        }
        if f.snare > 0.4 && self.beams.len() < 3 && layer > 0.5 {
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
            let col = look.at(0.35 + 0.65 * life).mix(WHITE, 0.3 * life).scale((0.3 + 0.7 * life) * s.min(1.0) * layer);
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
                let fade = 1.0 - k as f32 / 12.0;
                cv.put(x as i32, row, ch, look.at(fade).mix(WHITE, 0.5 * fade).scale(layer));
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

    fn wave(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], g: Geom, react: f32, look: &Look) {
        let f = cx.f;
        let (w, h) = (cx.w as i32, cx.h as i32);
        // the fields either side of the banner, one column of air between;
        // each wave runs on to the middle, fading out behind the letters
        let left_end = g.ox - 2; // last column of the left field
        let right_start = g.ox + g.bw as i32 + 1;
        let reach = (left_end + 1).max(w - right_start).max(1) as f32;
        let n = (w as usize).div_ceil(2).max(1);
        if self.hist.len() != n {
            self.hist = vec![(0.0, 0.0); n];
        }
        // the newest level enters at the screen edge and rolls inward
        let peak = |s: &[f32]| s[s.len().saturating_sub(800)..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let (pl, pr) = (peak(&f.wave_l), peak(&f.wave_r));
        self.peak = pl.max(pr).max(self.peak * (-cx.dt / 3.0).exp()).max(1e-4);
        let lv = |p: f32| ((p / self.peak).powf(1.4) * (0.7 + 0.5 * f.kick_env)).min(1.0);
        let (vl, vr) = (lv(pl), lv(pr));
        let tempo = if f.beat_conf > 0.5 { (f.bpm / 120.0).clamp(0.8, 1.5) } else { 1.0 };
        let rate = (22.0 + 70.0 * f.intensity) * tempo;
        self.tick += cx.dt * rate;
        while self.tick >= 1.0 {
            self.tick -= 1.0;
            self.hist.rotate_right(1);
            self.hist[0] = (vl, vr);
        }

        let cy = g.oy as f32 + g.bh as f32 / 2.0;
        let amp = (g.bh as f32 * 0.5 + 1.5 + 3.5 * f.intensity).min(cy - 1.0).min(h as f32 - cy - 1.0).max(1.0) * react * self.wind;
        for k in 0..n {
            let (xl, xr) = (k as i32, w - 1 - k as i32);
            for (x, v, depth) in [(xl, self.hist[k].0, xl - left_end), (xr, self.hist[k].1, right_start - xr)] {
                if x < 0 || x >= w || (k > 0 && xl >= xr) {
                    continue;
                }
                // shrink and fade from a few cells before the letters on, so
                // the wave slips behind them instead of hitting a wall
                let fade = (-((depth + 3).max(0) as f32) / 4.0).exp();
                if fade < 0.04 {
                    continue;
                }
                // mirrored about the banner's middle row, in half cells
                let half = v * amp * (0.3 + 0.7 * fade);
                if half < 0.25 {
                    continue;
                }
                let near = (k as f32 / reach).min(1.0); // 0 at the edge .. 1 at the banner
                let base = look.at(near).scale((0.45 + 0.55 * v) * fade);
                let top = cy - half;
                let bot = cy + half;
                for y in top.floor() as i32..=bot.ceil() as i32 - 1 {
                    let (y0, y1) = (y as f32, y as f32 + 1.0);
                    let cover_top = (y1 - top).clamp(0.0, 1.0);
                    let cover_bot = (bot - y0).clamp(0.0, 1.0);
                    let glyph = if cover_top < 0.5 {
                        '▄'
                    } else if cover_bot < 0.5 {
                        '▀'
                    } else {
                        '█'
                    };
                    // brightest on the axis, falling off toward the tips
                    let d = ((y as f32 + 0.5 - cy).abs() / half.max(0.5)).min(1.0);
                    let col = base.scale(1.0 - 0.6 * d).mix(WHITE, 0.35 * (1.0 - d) * v);
                    cv.put(x, y, glyph, col);
                }
            }
        }
        // the letters stay put; each swell lights them as it arrives
        let at = |k: i32| self.hist.get(k.max(0) as usize).copied().unwrap_or((0.0, 0.0));
        let arrive = at(left_end).0.max(at(w - 1 - right_start).1);
        for c in chars {
            let col = c.fin.scale(0.65 + 0.35 * arrive).mix(WHITE, 0.35 * arrive * arrive);
            self.letter(cv, c.home.0 as i32, c.home.1 as i32, c.ch, c.fin.mix(col, react));
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
        if f.drop && !self.homing {
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
                let src = ((0.55 + 0.9 * band) * (0.75 + 0.5 * f.kick_env) * (1.0 + 1.5 * self.surge) * react * self.wind).min(1.8);
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
                cv.put(x as i32, y as i32, FLAME[k], ramp(&FIRE, 0.15 + v.min(1.0) * 0.85).scale(self.wind.sqrt()));
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
        let layer = react * self.wind;
        if layer < 0.5 || self.homing {
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
                let ch = pick(KATAKANA, hash(d.seed ^ y as u32, if k == 0 { frame } else { frame / 6 }));
                let col = if k == 0 { Rgb::from_hex(0xdbffdb) } else { ramp(&GREENS, fade) .scale(0.35 + 0.65 * fade) };
                cv.put(d.x, y, ch, col.scale(layer));
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
        if f.snare > 0.4 && self.tears.len() < 3 && react > 0.5 && !self.homing {
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
        let split = if self.homing { 0 } else { (f.kick_env * 2.4 * react).round() as i32 };
        let frame = (self.t * 15.0) as u32;
        let [mag, cyan] = VHS;
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
            let ch = if scrambled { pick(CIPHER, h >> 10) } else { c.ch };
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
        if f.drop && !self.homing {
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
        let wob = if self.homing { 0.0 } else { f.bass_att.min(2.5) * 0.45 };
        self.step_sweep(cx);
        let pulse = f.kick_env;
        for c in chars.iter_mut() {
            if f.kick > 0.2 && !self.homing {
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

    #[allow(clippy::too_many_arguments)]
    fn floor(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], g: Geom, spec: &mut Spectrum, react: f32, look: &Look) {
        let f = cx.f;
        let rows = cx.h as i32 - (g.oy + g.bh as i32) - 2;
        if rows >= 3 {
            spec.floor(cv, cx, rows as usize, react * self.wind, look);
        }
        self.step_sweep(cx);
        for c in chars {
            let col = c.fin.scale(0.75 + 0.25 * f.kick_env).mix(WHITE, 0.8 * self.sweep_at(c, g));
            self.letter(cv, c.home.0 as i32, c.home.1 as i32, c.ch, c.fin.mix(col, react));
        }
    }

    // ------------------------------------------------------------ bounce

    fn bounce(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], g: Geom, react: f32, look: &mut Look) {
        let f = cx.f;
        if self.head == (0.0, 0.0) {
            let a = self.rng.range(0.3, 1.2) * if self.rng.chance(0.5) { 1.0 } else { -1.0 };
            self.head = (a.cos() * if self.rng.chance(0.5) { 1.0 } else { -1.0 }, a.sin());
        }
        if self.homing {
            // fly home at flying speed, easing only over the last few cells,
            // trail and all; once there, the trail catches up
            let d = self.off.0.hypot(self.off.1 * 2.0);
            if d > 0.05 {
                let speed = self.flight_speed(f).max(12.0).min(d * 5.0 + 3.0);
                let step = (speed * cx.dt).min(d);
                self.off.0 -= self.off.0 / d * step;
                self.off.1 -= self.off.1 * 2.0 / d * step * 0.5;
                self.record_trail();
            } else {
                self.off = (0.0, 0.0);
                self.tick += cx.dt;
                if self.tick > 0.08 {
                    self.tick = 0.0;
                    self.trail.pop();
                }
            }
        } else {
            self.fly(cx, g, react, look);
        }
        let place = |o: (f32, f32), c: &Ch| ((c.home.0 + o.0 * react).round() as i32, (c.home.1 + o.1 * react).round() as i32);
        for c in chars {
            let (x, y) = place(self.off, c);
            // over the ribbon too: a flying banner must stay readable
            let col = c.fin.mix(WHITE, 0.35 * f.kick_env.min(1.0));
            cv.put_top(x, y, c.ch, c.fin.mix(col, react).mix(WHITE, 0.6 * self.flash));
        }
        // the trail: older copies behind, each a step further round the palette
        let ghosts = ((0.5 + 3.5 * f.intensity) * react) as usize;
        for (k, &o) in self.trail.iter().enumerate().take(ghosts) {
            let fade = 0.3 - 0.07 * k as f32;
            for c in chars {
                let (x, y) = place(o, c);
                let u = (c.dx as f32 / g.bw as f32 * 0.8 + 0.12 * k as f32).fract();
                cv.put_under(x, y, c.ch, look.at(u).scale(fade * cx.light));
            }
        }
    }

    /// Bounce's flight: speed from the music, kicks swerve, walls reflect.
    fn fly(&mut self, cx: &Ctx, g: Geom, react: f32, look: &mut Look) {
        let f = cx.f;
        let (w, h) = (cx.w as i32, cx.h as i32);
        // where the banner may go: the whole width, from the top down to a
        // third of the way into the ribbon's room
        let (x0, x1) = ((-g.ox + 1) as f32, (w - g.ox - g.bw as i32 - 1) as f32);
        let (y0, y1) = ((-g.oy + 1) as f32, ((h - g.oy - g.bh as i32) / 3) as f32);
        // kicks swerve it once the music's going
        if f.kick > 0.35 && f.intensity > 0.45 {
            let r = self.rng.range(-0.5, 0.5);
            let (c, s) = (r.cos(), r.sin());
            self.head = (self.head.0 * c - self.head.1 * s, self.head.0 * s + self.head.1 * c);
        }
        let speed = self.flight_speed(f) * react;
        self.off.0 += self.head.0 * speed * cx.dt;
        self.off.1 += self.head.1 * speed * 0.5 * cx.dt; // rows are twice as tall
        let mut hit = false;
        if x1 > x0 {
            if self.off.0 < x0 || self.off.0 > x1 {
                self.head.0 = -self.head.0;
                self.off.0 = self.off.0.clamp(x0, x1);
                hit = true;
            }
        } else {
            self.off.0 = 0.0;
        }
        if y1 > y0 && (self.off.1 < y0 || self.off.1 > y1) {
            self.head.1 = -self.head.1;
            self.off.1 = self.off.1.clamp(y0, y1);
            hit = true;
        }
        if hit {
            look.nudge(0.22);
            look.flash = look.flash.max(0.5 * f.intensity);
        }
        self.record_trail();
    }

    fn flight_speed(&self, f: &crate::dsp::Features) -> f32 {
        (5.0 + 30.0 * f.intensity) * (1.0 + 1.5 * f.kick_env.min(1.0))
    }

    /// Copies far enough apart to read as separate banners, not a smear.
    fn record_trail(&mut self) {
        let last = self.trail.first().copied().unwrap_or((f32::MAX, 0.0));
        if (self.off.0 - last.0).hypot((self.off.1 - last.1) * 2.0) > 5.0 {
            self.trail.insert(0, self.off);
            self.trail.truncate(4);
        }
    }

    // -------------------------------------------------------------- warp

    fn warp(&mut self, cv: &mut Canvas, cx: &Ctx, chars: &[Ch], g: Geom, react: f32, look: &Look) {
        let f = cx.f;
        let (w, h) = (cx.w as f32, cx.h as f32);
        let centre = (g.ox as f32 + g.bw as f32 / 2.0, g.oy as f32 + g.bh as f32 / 2.0);
        let (rx, ry) = (g.bw as f32 / 2.0 + 1.0, g.bh as f32 / 2.0 + 1.0);
        let maxr = (w / 2.0).hypot(h);
        // stars leave from the banner's edges
        let layer = react * self.wind;
        self.spawn_acc += cx.dt * (8.0 + 320.0 * f.intensity.powf(1.5)) * (1.0 + 3.0 * f.kick_env.min(1.0)) * layer;
        if f.drop && !self.homing {
            self.spawn_acc += 500.0;
        }
        let spawn = self.spawn_acc as usize;
        self.spawn_acc -= spawn as f32;
        for _ in 0..spawn {
            if self.stars.len() >= 1800 {
                break;
            }
            let a = self.rng.range(0.0, std::f32::consts::TAU);
            let (dx, dy) = (a.cos(), a.sin() * 0.5);
            let t = (rx / dx.abs().max(1e-3)).min(ry / dy.abs().max(1e-3)) * self.rng.range(1.0, 1.5);
            let sp = self.rng.range(3.0, 9.0);
            self.stars.push(Star { x: centre.0 + dx * t, y: centre.1 + dy * t, vx: dx * sp, vy: dy * sp, u: a / std::f32::consts::TAU });
        }
        let accel = (cx.dt * (0.9 + 2.6 * f.intensity + 2.0 * f.kick_env.min(1.0))).exp();
        for s in &mut self.stars {
            s.vx *= accel;
            s.vy *= accel;
            s.x += s.vx * cx.dt;
            s.y += s.vy * cx.dt;
        }
        self.stars.retain(|s| s.x >= -1.0 && s.x <= w && s.y >= -1.0 && s.y <= h);

        self.step_sweep(cx);
        for c in chars {
            let col = c.fin.scale(0.75 + 0.25 * f.kick_env.min(1.0)).mix(WHITE, 0.8 * self.sweep_at(c, g));
            self.letter(cv, c.home.0 as i32, c.home.1 as i32, c.ch, c.fin.mix(col, react));
        }
        for s in &self.stars {
            let (dx, dy) = (s.x - centre.0, (s.y - centre.1) * 2.0);
            let dist = dx.hypot(dy);
            let sp = s.vx.hypot(s.vy * 2.0);
            // slow ones are dots, fast ones streaks along their heading
            let glyph = if sp < 12.0 {
                '·'
            } else if sp < 24.0 {
                '•'
            } else {
                let a = dy.atan2(dx).abs().to_degrees();
                let a = if a > 90.0 { 180.0 - a } else { a };
                if a < 22.5 {
                    '─'
                } else if a > 67.5 {
                    '│'
                } else if (dx > 0.0) == (dy > 0.0) {
                    '╲'
                } else {
                    '╱'
                }
            };
            let near = (dist / maxr).min(1.0);
            let col = look.at(s.u).scale((0.25 + 0.95 * near).min(1.0) * layer).mix(WHITE, 0.4 * near * near);
            cv.put_under(s.x.round() as i32, s.y.round() as i32, glyph, col);
        }
    }
}
