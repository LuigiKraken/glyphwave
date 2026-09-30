//! Theme director for music mode: picks the theme for each banner cycle and
//! says when its hold should end. Policy (from the research brief, MilkDrop
//! practice):
//!
//! * a hold lasts 16 or 32 bars when the tempo is known, otherwise 25–60 s;
//! * a detected section change ends it early, landing on the next downbeat;
//! * never end one while a build-up's tension is rising;
//! * a drop plays out inside the current theme (each has its own burst) and
//!   makes the next pick a busy one;
//! * quieter music draws calmer themes, louder music busier ones.

use crate::dsp::Features;
use crate::fx::Rng;
use crate::fx::themes::{ALL, Theme};

pub struct Director {
    since: f32,
    until: f32,
    pending: bool,
    force: bool,
    busy_next: bool,
    locked: Option<Theme>,
    recent: Vec<Theme>,
    rng: Rng,
    /// Set when the running hold should end.
    pub cue: bool,
    pub name: String,
}

impl Director {
    pub fn new() -> Director {
        Director {
            since: 0.0,
            until: 30.0,
            pending: false,
            force: false,
            busy_next: false,
            locked: None,
            recent: Vec::new(),
            rng: Rng::seeded(),
            cue: false,
            name: "-".into(),
        }
    }

    /// Always use one theme (testing, taste).
    pub fn lock(&mut self, name: &str) -> Result<(), String> {
        let t = Theme::from_name(name)
            .ok_or_else(|| format!("unknown theme {name}; themes: {}", ALL.iter().map(|t| t.name()).collect::<Vec<_>>().join(" ")))?;
        self.locked = Some(t);
        Ok(())
    }

    /// `v`: end the current hold now.
    pub fn next(&mut self) {
        self.force = true;
    }

    /// A themed cycle is starting: pick its theme and reset the timers.
    pub fn start(&mut self, f: &Features) -> Theme {
        let t = self.locked.unwrap_or_else(|| self.pick(f));
        self.recent.push(t);
        if self.recent.len() > 3 {
            self.recent.remove(0);
        }
        self.since = 0.0;
        self.until = if f.beat_conf > 0.5 && f.bpm > 40.0 {
            let bars = if self.rng.chance(0.5) { 16.0 } else { 32.0 };
            (bars * 4.0 * 60.0 / f.bpm).clamp(20.0, 70.0)
        } else {
            self.rng.range(25.0, 60.0)
        };
        self.pending = false;
        self.force = false;
        self.cue = false;
        self.busy_next = false;
        self.name = t.name();
        t
    }

    fn pick(&mut self, f: &Features) -> Theme {
        let e = (0.6 * f.energy + 0.4 * (f.onset_rate / 6.0).min(1.0)).clamp(0.0, 1.0);
        let pool: Vec<Theme> = ALL.iter().copied().filter(|t| !self.recent.contains(t)).collect();
        if self.busy_next {
            return pool.iter().copied().max_by(|a, b| a.busy().total_cmp(&b.busy())).unwrap_or(Theme::Fire);
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
        if f.section && self.since > 12.0 {
            self.pending = true;
        }
        if f.drop {
            self.busy_next = true;
        }
        let building = f.tension > 0.4;
        let downbeat = f.beat_conf < 0.5 || (f.beat && f.bar_beat() == 0);
        self.cue = self.force
            || (!building && self.since > 12.0 && (self.pending || self.until <= 0.0) && downbeat)
            || self.until <= -8.0; // a lost beat or an endless build: don't wait forever
    }
}
