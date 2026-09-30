//! The banner (tuxbook, from ~/.local/share/kde-screensaver/screensaver.txt).
//!
//! One cycle for both modes: a terminaltexteffects-style intro → a hold → an
//! outro → the next intro. While it's quiet the hold is a still banner with a
//! specular sweep. While music plays each cycle belongs to a theme
//! (`themes.rs`): a paired intro, the theme's palette, and a hold that
//! reacts to the music until the director calls time: a fade (outro, then
//! the next intro with no gap) or a cut (the next theme at once, under a
//! flash). Intros and outros run faster with faster, busier music.
//!
//! The cycle's colours live in one `Look` that the theme and the ribbon
//! read too, so everything on screen wears the same palette. While a theme
//! holds, the palette steps on every beat, harder the busier the music.

use super::effects::{Chars, Effect, INTROS, Kind, OUTROS, P};
use super::spectrum::Spectrum;
use super::themes::{Geom, Hold, Theme};
use super::{Ctx, Look, Rng};
use crate::canvas::Canvas;
use crate::color::{Rgb, VIVID, WHITE, saturate, vivid};
use crate::scene::{Cue, Director};

pub struct Ch {
    pub ch: char,
    pub dx: usize,
    pub dy: usize,
    pub home: P,
    pub pos: P,
    pub vel: P,
    pub fin: Rgb,
}

enum Phase {
    Gap(f32),
    Intro,
    Hold(f32),
    /// Leaving a themed hold: `react` eases to 0 so the outro starts clean.
    Leave(f32),
    Outro,
}

const LEAVE: f32 = 0.3;

pub struct Banner {
    lines: Vec<Vec<char>>,
    chars: Vec<Ch>,
    bw: usize,
    bh: usize,
    ox: i32,
    oy: i32,
    w: usize,
    h: usize,
    pub fits: bool,
    effect: Effect,
    phase: Phase,
    last_kinds: Vec<Kind>,
    pub current: Option<Kind>,
    /// Bumps on every new intro (main rerolls the idle ambience with it).
    pub cycles: u64,
    sweep: f32,
    /// The theme of the running cycle; None = an idle cycle.
    theme: Option<Theme>,
    hold: Hold,
    react: f32,
    rng: Rng,
    /// The running cycle's colours, shared with the theme and the ribbon.
    look: Look,
    last_vivid: usize,
}

impl Banner {
    pub fn load(path: &str) -> Banner {
        let text = std::fs::read_to_string(path).unwrap_or_else(|_| DEFAULT.to_string());
        Banner::from_text(&text)
    }

    pub fn from_text(text: &str) -> Banner {
        let mut lines: Vec<Vec<char>> = text.lines().map(|l| l.trim_end().chars().collect()).collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        while lines.first().is_some_and(|l| l.is_empty()) {
            lines.remove(0);
        }
        // drop the common left margin
        let margin = lines
            .iter()
            .filter(|l| !l.is_empty())
            .map(|l| l.iter().take_while(|c| **c == ' ').count())
            .min()
            .unwrap_or(0);
        for l in &mut lines {
            if l.len() >= margin {
                l.drain(..margin);
            }
        }
        let bw = lines.iter().map(|l| l.len()).max().unwrap_or(0);
        let bh = lines.len();
        Banner {
            lines,
            chars: Vec::new(),
            bw,
            bh,
            ox: 0,
            oy: 0,
            w: 0,
            h: 0,
            fits: false,
            effect: Effect::empty(),
            phase: Phase::Gap(0.3),
            last_kinds: Vec::new(),
            current: None,
            cycles: 0,
            sweep: -1.0,
            theme: None,
            hold: Hold::new(Theme::Pulse),
            react: 0.0,
            rng: Rng::seeded(),
            look: Look::new(&crate::color::fallback_palette()),
            last_vivid: usize::MAX,
        }
    }

    fn layout(&mut self, w: usize, h: usize) {
        if self.w == w && self.h == h {
            return;
        }
        self.w = w;
        self.h = h;
        self.fits = self.bw > 0 && self.bw + 2 <= w && self.bh + 2 <= h;
        self.ox = (w as i32 - self.bw as i32) / 2;
        self.oy = (h as i32 - self.bh as i32) / 2;
        let old: Vec<P> = self.chars.iter().map(|c| c.pos).collect();
        self.chars.clear();
        if !self.fits {
            return;
        }
        for (dy, l) in self.lines.iter().enumerate() {
            for (dx, &ch) in l.iter().enumerate() {
                if ch != ' ' {
                    let home = ((self.ox + dx as i32) as f32, (self.oy + dy as i32) as f32);
                    let pos = old.get(self.chars.len()).copied().unwrap_or(home);
                    self.chars.push(Ch { ch, dx, dy, home, pos, vel: (0.0, 0.0), fin: WHITE });
                }
            }
        }
        self.phase = Phase::Gap(0.2); // restart the cycle on a resize
    }

    /// Each letter's colour from the look: a diagonal gradient, like TTE's.
    fn finals(&mut self) {
        let bw = self.bw.max(1) as f32;
        let bh = self.bh.max(1) as f32;
        for c in &mut self.chars {
            let u = (c.dx as f32 / bw * 0.8 + c.dy as f32 / bh * 0.2).clamp(0.0, 1.0);
            c.fin = self.look.at(u);
        }
    }

    /// A music cycle's stops: the theme's own, a neon set (likelier the
    /// busier it gets), or the album's, saturated.
    fn pick_stops(&mut self, cx: &Ctx, theme: Option<Theme>) -> Vec<Rgb> {
        let Some(t) = theme else { return cx.palette.to_vec() };
        if let Some(p) = t.palette() {
            return p;
        }
        if self.rng.chance(0.35 + 0.55 * cx.f.intensity) {
            let mut i = self.rng.below(VIVID.len());
            if i == self.last_vivid {
                i = (i + 1) % VIVID.len();
            }
            self.last_vivid = i;
            vivid(i)
        } else {
            saturate(cx.palette)
        }
    }

    fn chars_view(&self) -> (Vec<P>, Vec<usize>, Vec<usize>) {
        (
            self.chars.iter().map(|c| c.home).collect(),
            self.chars.iter().map(|c| c.dy).collect(),
            self.chars.iter().map(|c| c.dx).collect(),
        )
    }

    fn next_intro(&mut self) {
        let from: &[Kind] = self.theme.map(|t| t.intros()).unwrap_or(&INTROS);
        let mut pool: Vec<Kind> = from.iter().copied().filter(|k| !self.last_kinds.contains(k)).collect();
        if pool.is_empty() {
            pool = from.to_vec();
        }
        let k = self.rng.pick(&pool);
        self.last_kinds.push(k);
        if self.last_kinds.len() > 6 {
            self.last_kinds.remove(0);
        }
        let (home, row, col) = self.chars_view();
        let c = Chars { home: &home, row: &row, col: &col, rows: self.bh, cols: self.bw, w: self.w as f32, h: self.h as f32 };
        self.effect = Effect::intro(k, &c, &mut self.rng);
        self.current = Some(k);
        self.cycles += 1;
    }

    fn start_outro(&mut self) {
        let o = self.rng.pick(&OUTROS);
        let (home, row, col) = self.chars_view();
        let c = Chars { home: &home, row: &row, col: &col, rows: self.bh, cols: self.bw, w: self.w as f32, h: self.h as f32 };
        self.effect = Effect::outro(o, &c, &mut self.rng);
    }

    /// The theme of the running cycle (None while idle).
    pub fn theme(&self) -> Option<Theme> {
        self.theme
    }

    /// End the current idle hold early (`v` while idle).
    pub fn skip_idle(&mut self) {
        if let Phase::Hold(t) = &mut self.phase {
            *t = 0.0;
        } else if matches!(self.phase, Phase::Gap(_)) {
            self.phase = Phase::Gap(0.0);
        }
    }

    /// The theme whose hold is running now (None in intros, outros, gaps).
    pub fn holding(&self) -> Option<Theme> {
        self.theme.filter(|_| matches!(self.phase, Phase::Hold(_) | Phase::Leave(_)))
    }

    /// First screen row below the banner.
    pub fn bottom(&self) -> i32 {
        self.oy + self.bh as i32
    }

    /// The running cycle's colours and the banner's columns on screen (left
    /// edge, width), so other layers can colour to match the letters.
    pub fn look(&self) -> (&Look, i32, usize) {
        (&self.look, self.ox, self.bw)
    }

    /// First screen row of the banner.
    pub fn top(&self) -> i32 {
        self.oy
    }

    /// True while the matrix intro runs (the rain layer joins in).
    pub fn wants_rain(&self) -> bool {
        matches!(self.phase, Phase::Intro) && self.current == Some(Kind::Matrix)
    }

    fn start_cycle(&mut self, cx: &Ctx, music: bool, dir: &mut Director) {
        self.theme = music.then(|| dir.start(cx.f));
        let stops = self.pick_stops(cx, self.theme);
        self.look.set(&stops, 0.0);
        self.finals();
        for c in &mut self.chars {
            c.pos = c.home;
            c.vel = (0.0, 0.0);
        }
        if let Some(t) = self.theme {
            self.hold = Hold::new(t);
        }
        self.next_intro();
        self.phase = Phase::Intro;
    }

    /// A cut: the next theme takes over the hold at once, under a flash.
    fn cut(&mut self, cx: &Ctx, dir: &mut Director) {
        let t = dir.start(cx.f);
        self.theme = Some(t);
        let stops = self.pick_stops(cx, Some(t));
        self.look.set(&stops, 0.35);
        self.look.flash = 1.0;
        self.hold = Hold::new(t);
        self.react = self.react.min(0.5);
        for c in &mut self.chars {
            c.pos = c.home;
            c.vel = (0.0, 0.0);
        }
    }

    /// While a theme holds: the palette steps on the beat, a little when
    /// it's calm, by big jumps when it's busy; the downbeat jumps further.
    fn step_look(&mut self, cx: &Ctx) {
        let f = cx.f;
        let hype = ((f.intensity - 0.35) / 0.5).clamp(0.0, 1.0);
        if f.beat {
            let mut d = 0.015 + 0.13 * hype + self.hold.theme.beat_step();
            if f.bar_beat() == 0 && hype > 0.5 {
                d += 0.12;
            }
            self.look.nudge(d);
        }
        self.look.nudge(cx.dt * 0.01);
        if f.drop {
            self.look.flash = 1.0;
        }
        self.finals();
    }
    /// Advance the cycle and draw. `music` says whether the next cycle should
    /// be themed and whether a themed hold may continue.
    pub fn draw(&mut self, cv: &mut Canvas, cx: &Ctx, music: bool, dir: &mut Director, spec: &mut Spectrum) {
        self.layout(cx.w, cx.h);
        if !self.fits {
            return;
        }
        let f = cx.f;
        let dt = cx.dt;
        self.look.step(dt);
        // faster, busier music, faster effects (TTE durations assume ~120 bpm)
        let speed = if self.theme.is_some() {
            let tempo = if f.beat_conf > 0.5 { (f.bpm / 120.0).clamp(0.85, 1.4) } else { 1.0 };
            tempo * (1.0 + 0.5 * f.intensity)
        } else {
            1.0
        };
        match &mut self.phase {
            Phase::Gap(t) => {
                *t -= dt;
                if *t > 0.0 {
                    return;
                }
                self.start_cycle(cx, music, dir);
            }
            Phase::Intro => {
                self.effect.step(dt * speed);
                if self.effect.done() {
                    self.phase = Phase::Hold(self.rng.range(4.0, 7.0));
                    self.sweep = 0.0;
                    self.react = 0.0;
                }
            }
            Phase::Hold(t) => match self.theme {
                None => {
                    *t -= dt;
                    if music {
                        *t = t.min(0.4); // music started: move on to a themed cycle
                    }
                    if *t <= 0.0 {
                        self.start_outro();
                        self.phase = Phase::Outro;
                    }
                }
                Some(_) => {
                    self.react = (self.react + dt / 0.8).min(1.0);
                    dir.update(f, dt);
                    if !music || dir.cue == Cue::Fade {
                        self.phase = Phase::Leave(LEAVE);
                    } else if dir.cue == Cue::Cut {
                        self.cut(cx, dir);
                    }
                }
            },
            Phase::Leave(t) => {
                *t -= dt;
                self.react = self.react.min((*t / LEAVE).max(0.0));
                if *t <= 0.0 {
                    self.start_outro();
                    self.phase = Phase::Outro;
                }
            }
            Phase::Outro => {
                self.effect.step(dt * speed);
                if self.effect.done() {
                    // while music plays, the next intro follows at once
                    self.phase = Phase::Gap(if music { 0.0 } else { self.rng.range(0.5, 1.2) });
                    return;
                }
            }
        }
        let holding = matches!(self.phase, Phase::Hold(_) | Phase::Leave(_));
        if holding && self.theme.is_some() {
            if matches!(self.phase, Phase::Hold(_)) {
                self.step_look(cx);
            }
            let g = Geom { ox: self.ox, oy: self.oy, bw: self.bw, bh: self.bh };
            self.hold.draw(cv, cx, &mut self.chars, g, spec, self.react, &mut self.look);
            return;
        }
        if holding {
            self.sweep += dt;
            if self.sweep > 3.2 {
                self.sweep = 0.0;
            }
        }
        let shift = self.effect.row_shift.clone();
        let sweep = self.sweep;
        let (bw, bh) = (self.bw as f32, self.bh as f32);
        for (i, c) in self.chars.iter_mut().enumerate() {
            let (pos, glyph, col, vis) = if holding {
                (c.home, c.ch, c.fin, true)
            } else if i < self.effect.anims.len() {
                let l = self.effect.look(i, c.ch, c.fin, c.home);
                (l.pos, l.glyph, l.col, l.vis)
            } else {
                (c.home, c.ch, c.fin, false)
            };
            c.pos = pos;
            c.vel = (0.0, 0.0);
            if !vis {
                continue;
            }
            let mut col = col;
            if holding {
                // specular sweep: a diagonal band brightening towards white
                let d = (c.dx as f32 / bw + c.dy as f32 / bh * 0.35) - (sweep / 1.4 * 1.6 - 0.3);
                let hl = (-(d * d) * 60.0).exp();
                col = col.mix(WHITE, 0.75 * hl);
            }
            let sx = shift.get(c.dy).copied().unwrap_or(0);
            cv.put(pos.0.round() as i32 + sx, pos.1.round() as i32, glyph, col);
        }
    }
}

const DEFAULT: &str = "\
      _             _
 __ _| |_  _ _ __ | |_ __ ____ ___ _____
/ _` | | || | '_ \\| ' \\\\ V  V / _` \\ V / -_)
\\__, |_|\\_, | .__/|_||_|\\_/\\_/\\__,_|\\_/\\___|
|___/   |__/|_|
";
