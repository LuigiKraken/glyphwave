//! Theme director for music mode: picks the theme for each banner cycle and
//! says when, and how, its hold should end. The music decides, not a clock:
//!
//! * a **lull** (the level falls well under its recent average: a breakdown,
//!   a breath between sections) ends the hold with a fade: the outro, then
//!   straight into the next intro;
//! * a **high** (a drop, or a surge of level with a kick) cuts: the next
//!   theme takes over at once, under a flash, and it is a busy one;
//! * a detected **section change** cuts when the music is busy, and otherwise
//!   waits for the next lull or downbeat;
//! * the timer (16 or 32 bars, or 30–60 s) only marks a hold as due; a due
//!   hold still waits for a lull or a downbeat, and gives up waiting at 10 s;
//! * quieter music draws calmer themes, louder music busier ones; a theme
//!   picked before the analysis had heard the music (it was already playing
//!   hard when the screensaver came up) is cut for a busy one within seconds.

use crate::dsp::Features;
use crate::fx::Rng;
use crate::fx::themes::{ALL, Theme};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cue {
    None,
    /// Straight to the next theme, no outro or intro.
    Cut,
    /// Outro, then the next theme's intro.
    Fade,
}

/// The shortest hold before a lull, a section or the timer may end it, and
/// before a high may cut it.
const MIN_FADE: f32 = 8.0;
const MIN_CUT: f32 = 4.0;
/// How long into a hold a theme picked too calm may still be cut.
const OUTRUN: f32 = 6.0;

pub struct Director {
    since: f32,
    until: f32,
    pending: bool,
    pending_for: f32,
    force: bool,
    busy_next: bool,
    calm_next: bool,
    locked: Option<Theme>,
    /// The theme a `--test` number key asked for next.
    want: Option<Theme>,
    recent: Vec<Theme>,
    rng: Rng,
    /// Fast and slow level in dB, fast and slow intensity, and whether a
    /// lull / surge may fire again.
    fast: f32,
    slow: f32,
    int_fast: f32,
    int_slow: f32,
    low_for: f32,
    lull_armed: bool,
    surge_armed: bool,
    /// The theme picked last, and the intensity it was picked on.
    theme: Option<Theme>,
    picked_on: f32,
    /// Set when the running hold should end, and how.
    pub cue: Cue,
    pub name: String,
}

impl Director {
    pub fn new() -> Director {
        Director {
            since: 0.0,
            until: 30.0,
            pending: false,
            pending_for: 0.0,
            force: false,
            busy_next: false,
            calm_next: false,
            locked: None,
            want: None,
            recent: Vec::new(),
            rng: Rng::seeded(),
            fast: -60.0,
            slow: -60.0,
            int_fast: 0.0,
            int_slow: 0.0,
            low_for: 0.0,
            lull_armed: true,
            surge_armed: true,
            theme: None,
            picked_on: 0.0,
            cue: Cue::None,
            name: "-".into(),
        }
    }

    /// Always use one theme (testing, taste).
    pub fn lock(&mut self, name: &str) -> Result<(), String> {
        let t = Theme::from_name(name)
            .ok_or_else(|| format!("unknown theme {name}; themes: {}", ALL.iter().map(|t| t.name()).collect::<Vec<_>>().join(" ") + " floor"))?;
        self.locked = Some(t);
        Ok(())
    }

    /// `v`: end the current hold now.
    pub fn next(&mut self) {
        self.force = true;
    }

    /// `--test` 1–0: fade out the current hold and into `t`.
    pub fn jump(&mut self, t: Theme) {
        self.want = Some(t);
        self.force = true;
    }

    /// A themed cycle is starting: pick its theme and reset the timers.
    pub fn start(&mut self, f: &Features) -> Theme {
        let chosen = self.want.is_some() || self.locked.is_some();
        let t = self.want.take().or(self.locked).unwrap_or_else(|| self.pick(f));
        self.recent.push(t);
        if self.recent.len() > 4 {
            self.recent.remove(0);
        }
        self.since = 0.0;
        self.until = if f.beat_conf > 0.5 && f.bpm > 40.0 {
            let bars = if self.rng.chance(0.5) { 16.0 } else { 32.0 };
            (bars * 4.0 * 60.0 / f.bpm).clamp(20.0, 70.0)
        } else {
            self.rng.range(30.0, 60.0)
        };
        self.pending = false;
        self.pending_for = 0.0;
        self.force = false;
        self.cue = Cue::None;
        self.busy_next = false;
        self.calm_next = false;
        self.name = t.name();
        self.theme = Some(t);
        // a theme asked for is never outrun
        self.picked_on = if chosen { 1.0 } else { f.intensity };
        t
    }

    /// True when the music runs well hotter than the theme was picked for:
    /// it was already going when the screensaver came up, before the
    /// analysis had heard it, or it took off during the intro.
    fn outrun(&self, f: &Features) -> bool {
        let Some(t) = self.theme else {
            return false;
        };
        f.intensity > 0.6 && f.intensity - t.busy() > 0.25 && f.intensity - self.picked_on > 0.25
    }

    fn pick(&mut self, f: &Features) -> Theme {
        // intensity lags a lull by a second or two: aim lower after one
        let e = if self.calm_next { f.intensity * 0.5 } else { f.intensity };
        let pool: Vec<Theme> = ALL.iter().copied().filter(|t| !self.recent.contains(t)).collect();
        if self.busy_next {
            // one of the three busiest, not always the same one
            let mut busy = pool.clone();
            busy.sort_by(|a, b| b.busy().total_cmp(&a.busy()));
            busy.truncate(3);
            return self.rng.pick(&busy);
        }
        // weight by how well the theme's busyness matches the music
        let wts: Vec<f32> = pool.iter().map(|t| (1.0 - (t.busy() - e).abs() * 1.4).max(0.08)).collect();
        let mut x = self.rng.f() * wts.iter().sum::<f32>();
        for (t, w) in pool.iter().zip(&wts) {
            if x < *w {
                return *t;
            }
            x -= w;
        }
        pool[0]
    }

    /// Advance during a themed hold; sets `cue` when it should end.
    pub fn update(&mut self, f: &Features, dt: f32) {
        self.since += dt;
        self.until -= dt;
        let ema = |v: &mut f32, x: f32, tau: f32| *v += (x - *v) * (1.0 - (-dt / tau).exp());
        // raw level, not the auto-gained `loud`, which hides a breakdown
        let db = f.rms_db.max(-70.0);
        ema(&mut self.fast, db, 0.25);
        ema(&mut self.slow, db, 5.0);
        ema(&mut self.int_fast, f.intensity, 0.3);
        ema(&mut self.int_slow, f.intensity, 5.0);
        let under = self.slow - self.fast; // dB under the recent level

        // a lull: well under the recent level (or busyness) for a moment
        let low = self.slow > -55.0 && (under > 7.0 || self.int_fast < 0.55 * self.int_slow);
        let lull = if low {
            self.low_for += dt;
            self.lull_armed && self.low_for > 0.35
        } else {
            self.low_for = 0.0;
            false
        };
        if under < 3.0 && self.int_fast > 0.8 * self.int_slow {
            self.lull_armed = true;
        }
        // a high: a jump in level landing with a kick
        let surge = self.surge_armed && (under < -6.0 || self.int_fast > 1.5 * self.int_slow.max(0.15)) && f.kick > 0.3;
        if under > -2.0 && self.int_fast < 1.15 * self.int_slow.max(0.15) {
            self.surge_armed = true;
        }
        if f.drop {
            self.busy_next = true;
        }
        if f.section && self.since > MIN_FADE {
            self.pending = true;
        }
        if self.until <= 0.0 {
            self.pending = true;
        }
        if self.pending {
            self.pending_for += dt;
        }
        let downbeat = f.beat_conf < 0.5 || (f.beat && f.bar_beat() == 0);
        let busy = f.intensity > 0.55;

        self.cue = if self.force {
            if busy && self.want.is_none() { Cue::Cut } else { Cue::Fade }
        } else if self.since < OUTRUN && self.outrun(f) {
            // early in the hold, the music outran the theme: cut to a busy
            // one now instead of holding a calm one until the next drop
            self.busy_next = f.intensity > 0.7;
            Cue::Cut
        } else if (f.drop || surge) && self.since > MIN_CUT {
            self.busy_next = true;
            Cue::Cut
        } else if lull && self.since > MIN_FADE {
            self.calm_next = true;
            Cue::Fade
        } else if self.pending && self.since > MIN_FADE && downbeat && (busy || self.pending_for > 10.0) {
            if busy { Cue::Cut } else { Cue::Fade }
        } else {
            Cue::None
        };
        if lull {
            self.lull_armed = false;
        }
        if surge {
            self.surge_armed = false;
        }
    }
}
