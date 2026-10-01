//! Audio analysis. Two paths, both from the same sample ring:
//!
//! * Display spectrum, once per rendered frame: three Hann-windowed FFTs
//!   (4096 bass < 250 Hz, 2048 mids < 2.5 kHz, 1024 treble — a cheap
//!   multi-resolution stack), band power over log-spaced bars in dB (no
//!   tilt: band-summed power already shows pink noise flat), slow
//!   asymmetric auto-gain, cava-style gravity.
//! * Onsets and tempo on a fixed 100 Hz hop: SuperFlux (Böck & Widmer 2013,
//!   max-filtered log-band flux) per kick/snare/hat band, z-score peak picking
//!   (Böck et al. 2012 online parameters), Ellis 2007 weighted autocorrelation
//!   for tempo with the duple fold, a PLL for beat phase that fires beats
//!   ~30 ms early to cancel the analysis delay.
//!
//! Plus the cheap high-level features that map well to visuals: momentary and
//! short-term loudness, log spectral centroid, flatness, MilkDrop-style
//! band-energy ratios, build-up/drop heuristic, two-window novelty (Foote-lite)
//! for section changes. Every time constant is scaled by dt.

use crate::audio::{RATE, Ring};
use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

const HOP: usize = 480; // 10 ms at 48 kHz
const ODF_N: usize = 2048;
const HOP_RATE: f32 = RATE as f32 / HOP as f32;
const TEMPO_LEN: usize = 800; // 8 s of ODF
pub const WAVE: usize = 2048;
/// dB/octave lift around 1 kHz. Band-summed power already shows pink noise
/// flat; 0 keeps dense guitar/cymbal mixes from standing taller than the bass.
const TILT: f32 = 0.0;
/// Visible dynamic range below the auto-gain reference, and the contrast curve.
const RANGE_DB: f32 = 36.0;
const GAMMA: f32 = 2.2;

fn ema(y: &mut f32, x: f32, dt: f32, tau: f32) {
    *y += (x - *y) * (1.0 - (-dt / tau).exp());
}

struct Fft {
    n: usize,
    plan: Arc<dyn RealToComplex<f32>>,
    win: Vec<f32>,
    inp: Vec<f32>,
    out: Vec<realfft::num_complex::Complex<f32>>,
    scratch: Vec<realfft::num_complex::Complex<f32>>,
    /// |X|² normalised so a full-scale sine is 1.0 (0 dB).
    pow: Vec<f32>,
}

impl Fft {
    fn new(planner: &mut RealFftPlanner<f32>, n: usize) -> Fft {
        let plan = planner.plan_fft_forward(n);
        let win = (0..n)
            .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos()))
            .collect();
        Fft {
            n,
            inp: plan.make_input_vec(),
            out: plan.make_output_vec(),
            scratch: plan.make_scratch_vec(),
            pow: vec![0.0; n / 2 + 1],
            plan,
            win,
        }
    }

    /// FFT of the last `n` samples of `x`.
    fn run(&mut self, x: &[f32]) {
        let s = &x[x.len() - self.n..];
        for i in 0..self.n {
            self.inp[i] = s[i] * self.win[i];
        }
        let _ = self.plan.process_with_scratch(&mut self.inp, &mut self.out, &mut self.scratch);
        let norm = 1.0 / (self.n as f32 / 4.0).powi(2);
        for (p, c) in self.pow.iter_mut().zip(&self.out) {
            *p = (c.re * c.re + c.im * c.im) * norm;
        }
    }

    fn bin_hz(&self) -> f32 {
        RATE as f32 / self.n as f32
    }
}

/// Which FFT serves a bar, and how to read its power.
#[derive(Clone)]
struct Band {
    fc: f32,
    fft: usize,
    k0: usize,
    k1: usize, // exclusive; k0 == k1 → interpolate at fc
    width_bins: f32,
}

#[derive(Default, Clone)]
struct Onset {
    mean: f32,
    var: f32,
    hist: [f32; 3],
    since: f32,
    warm: f32,
}

impl Onset {
    /// Returns strength 0..1 when this hop is an onset.
    fn step(&mut self, o: f32, delta: f32, refractory: f32, gate: bool) -> f32 {
        let dt = 1.0 / HOP_RATE;
        self.since += dt;
        self.warm += dt;
        let sd = (self.var + 1e-6).sqrt();
        let z = (o - self.mean) / sd;
        let is_max = self.hist.iter().all(|&h| o >= h);
        self.hist = [self.hist[1], self.hist[2], o];
        ema(&mut self.mean, o, dt, 3.0);
        let d = o - self.mean;
        ema(&mut self.var, d * d, dt, 3.0);
        if gate && self.warm > 1.0 && z > delta && is_max && self.since > refractory && o > 0.05 {
            self.since = 0.0;
            ((z - delta) / 4.0 + 0.35).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

#[derive(Default, Clone)]
pub struct Features {
    /// Display bar heights 0..1 (smoothed, gravity), left / right / mono.
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub mono: Vec<f32>,
    /// Peak caps for `mono`, 0..1.
    pub caps: Vec<f32>,
    /// Newest WAVE raw samples per channel, for scopes.
    pub wave_l: Vec<f32>,
    pub wave_r: Vec<f32>,
    pub rms_db: f32,
    /// Momentary loudness relative to the recent loud level, 0..1.
    pub loud: f32,
    /// Short-term (3 s) energy, 0..1.
    pub energy: f32,
    /// Log spectral centroid 0 (100 Hz) .. 1 (8 kHz), smoothed.
    pub centroid: f32,
    pub flatness: f32,
    /// MilkDrop-style band energy relative to its 8 s average (1 = normal).
    pub bass: f32,
    pub mid: f32,
    pub treb: f32,
    pub bass_att: f32,
    pub mid_att: f32,
    pub treb_att: f32,
    /// Onset strength this frame (0 = none) and decaying envelopes.
    pub kick: f32,
    pub snare: f32,
    pub hat: f32,
    pub kick_env: f32,
    pub snare_env: f32,
    pub hat_env: f32,
    /// Onsets per second over ~2 s.
    pub onset_rate: f32,
    /// How much is going on, 0 (quiet, sparse) .. 1 (loud, dense, punchy):
    /// onset density, loudness (relative and absolute), how much of the
    /// spectrum is lit, and kick/snare punch. Rises in ~0.4 s, falls in ~2 s.
    pub intensity: f32,
    /// Predicted beat this frame (fired slightly early).
    pub beat: bool,
    pub beat_phase: f32,
    pub beat_count: u64,
    pub bpm: f32,
    pub beat_conf: f32,
    pub drop: bool,
    pub tension: f32,
    pub section: bool,
    pub silent: bool,
    pub silent_for: f32,
}

impl Features {
    /// Beat in the bar (0 = downbeat), assuming 4/4.
    pub fn bar_beat(&self) -> u64 {
        self.beat_count % 4
    }
}

pub struct Analyzer {
    ffts: Vec<Fft>, // bass 4096, mid 2048, treble 1024
    odf_fft: Fft,
    bands: Vec<Band>,
    nbars: usize,
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    mono: Vec<f32>,
    last_total: u64,
    // display state
    lref: f32,
    started: f32,
    peak: [Vec<f32>; 2],
    fall_t: [Vec<f32>; 2],
    cap_hold: Vec<f32>,
    cap_vel: Vec<f32>,
    raw_h: Vec<f32>,
    // odf
    odf_bands: Vec<(usize, usize, f32)>,
    prev_log: Vec<f32>,
    cur_log: Vec<f32>,
    on_all: Onset,
    on_kick: Onset,
    on_snare: Onset,
    on_hat: Onset,
    odf_hist: Vec<f32>,
    odf_pos: usize,
    tempo_timer: f32,
    period: f32,
    period_score: f32,
    tempo_cand: f32,
    tempo_cand_n: u32,
    phase: f32,
    fired: bool,
    matched: bool,
    checked: bool,
    onset_times: Vec<f32>,
    clock: f32,
    // features
    ms_mom: f32,
    ms_short: f32,
    loud_ref: f32,
    e_bands: [f32; 3],
    e_long: [f32; 3],
    bass_short: f32,
    bass_fast: f32,
    bass_long: f32,
    low_time: f32,
    drop_ref: f32,
    armed: bool,
    cent_s: f32,
    cent_l: f32,
    nov_timer: f32,
    nov_feat: Vec<Vec<f32>>,
    nov_d: Vec<f32>,
    nov_since: f32,
    gate_open: bool,
    below: f32,
    punch: f32,
    pub f: Features,
}

impl Analyzer {
    pub fn new() -> Analyzer {
        let mut planner = RealFftPlanner::<f32>::new();
        let ffts = vec![
            Fft::new(&mut planner, 4096),
            Fft::new(&mut planner, 2048),
            Fft::new(&mut planner, 1024),
        ];
        let odf_fft = Fft::new(&mut planner, ODF_N);
        // ODF filterbank: 6 bands/octave, 30 Hz..16 kHz, each at least one bin.
        let bin = RATE as f32 / ODF_N as f32;
        let mut odf_bands = Vec::new();
        let mut lo = 30.0f32;
        let mut last_k1 = 0usize;
        while lo < 16_000.0 {
            let hi = lo * 2f32.powf(1.0 / 6.0);
            let k0 = ((lo / bin).round() as usize).max(last_k1).max(1);
            let k1 = ((hi / bin).round() as usize).max(k0 + 1);
            if k0 >= last_k1 {
                odf_bands.push((k0, k1, (lo * hi).sqrt()));
                last_k1 = k1;
            }
            lo = hi;
        }
        let nb = odf_bands.len();
        Analyzer {
            ffts,
            odf_fft,
            bands: Vec::new(),
            nbars: 0,
            buf_l: vec![0.0; 16_384],
            buf_r: vec![0.0; 16_384],
            mono: vec![0.0; 16_384],
            last_total: 0,
            lref: -20.0,
            started: 0.0,
            peak: [Vec::new(), Vec::new()],
            fall_t: [Vec::new(), Vec::new()],
            cap_hold: Vec::new(),
            cap_vel: Vec::new(),
            raw_h: Vec::new(),
            odf_bands,
            prev_log: vec![0.0; nb],
            cur_log: vec![0.0; nb],
            on_all: Onset::default(),
            on_kick: Onset::default(),
            on_snare: Onset::default(),
            on_hat: Onset::default(),
            odf_hist: vec![0.0; TEMPO_LEN],
            odf_pos: 0,
            tempo_timer: 0.0,
            period: 0.5,
            period_score: 0.0,
            tempo_cand: 0.0,
            tempo_cand_n: 0,
            phase: 0.0,
            fired: false,
            matched: false,
            checked: false,
            onset_times: Vec::new(),
            clock: 0.0,
            ms_mom: 0.0,
            ms_short: 0.0,
            loud_ref: -30.0,
            e_bands: [0.0; 3],
            e_long: [1e-6; 3],
            bass_short: 0.0,
            bass_fast: 0.0,
            bass_long: 1e-6,
            low_time: 0.0,
            drop_ref: 0.0,
            armed: false,
            cent_s: 0.5,
            cent_l: 0.5,
            nov_timer: 0.0,
            nov_feat: Vec::new(),
            nov_d: Vec::new(),
            nov_since: 0.0,
            gate_open: false,
            below: 0.0,
            punch: 0.0,
            f: Features { bpm: 120.0, silent: true, ..Default::default() },
        }
    }

    /// Lay out `n` log-spaced bars from 40 Hz to 16 kHz (per channel).
    pub fn set_bars(&mut self, n: usize) {
        if n == self.nbars {
            return;
        }
        self.nbars = n;
        let (lo, hi) = (40.0f32, 16_000.0f32);
        let edge = |k: usize| lo * (hi / lo).powf(k as f32 / n as f32);
        self.bands = (0..n)
            .map(|k| {
                let (a, b) = (edge(k), edge(k + 1));
                let fc = (a * b).sqrt();
                let fft = if fc < 250.0 { 0 } else if fc < 2500.0 { 1 } else { 2 };
                let bin = self.ffts[fft].bin_hz();
                let k0 = (a / bin).ceil() as usize;
                let k1 = ((b / bin).ceil() as usize).min(self.ffts[fft].n / 2);
                let (k0, k1) = if k1 > k0 { (k0, k1) } else { (0, 0) };
                Band { fc, fft, k0, k1, width_bins: (b - a) / bin }
            })
            .collect();
        for c in 0..2 {
            self.peak[c] = vec![0.0; n];
            self.fall_t[c] = vec![0.0; n];
        }
        self.f.left = vec![0.0; n];
        self.f.right = vec![0.0; n];
        self.f.mono = vec![0.0; n];
        self.f.caps = vec![0.0; n];
        self.cap_hold = vec![0.0; n];
        self.cap_vel = vec![0.0; n];
        self.raw_h = vec![0.0; n];
    }

    fn band_power(&self, b: &Band) -> f32 {
        let p = &self.ffts[b.fft].pow;
        if b.k1 > b.k0 {
            p[b.k0..b.k1].iter().sum()
        } else {
            // narrower than a bin: interpolate the density at fc, scale by width
            let x = b.fc / self.ffts[b.fft].bin_hz();
            let i = (x as usize).min(p.len() - 2);
            let f = x - i as f32;
            (p[i] * (1.0 - f) + p[i + 1] * f) * b.width_bins
        }
    }

    pub fn update(&mut self, ring: &Ring, dt: f32) {
        let dt = dt.clamp(1e-4, 0.1);
        self.clock += dt;
        let f = &mut self.f;
        f.kick = 0.0;
        f.snare = 0.0;
        f.hat = 0.0;
        f.beat = false;
        f.drop = false;
        f.section = false;

        // how many new 10 ms hops arrived; copy enough history for all of them
        let new = ring.total.saturating_sub(self.last_total);
        let hops = ((new / HOP as u64) as usize).min(20);
        self.last_total += (hops * HOP) as u64;
        if new > (20 * HOP) as u64 {
            self.last_total = ring.total; // fell behind (or first call): resync
        }
        let backlog = (ring.total - self.last_total) as usize;
        let need = 4096 + hops * HOP + backlog;
        let need = need.min(self.buf_l.len());
        let (bl, br) = (&mut self.buf_l[..need], &mut self.buf_r[..need]);
        ring.latest(need, bl, br);
        for i in 0..need {
            self.mono[i] = 0.5 * (self.buf_l[i] + self.buf_r[i]);
        }

        // loudness + gate on the newest frame's worth of samples
        let n_now = ((RATE as f32 * dt) as usize).clamp(256, 4096);
        let ms = self.mono[need - n_now..need].iter().map(|x| x * x).sum::<f32>() / n_now as f32;
        let f = &mut self.f;
        f.rms_db = 10.0 * (ms + 1e-12).log10();
        if f.rms_db > -55.0 {
            self.gate_open = true;
            self.below = 0.0;
        } else if f.rms_db < -65.0 {
            self.below += dt;
            if self.below > 0.3 {
                self.gate_open = false;
            }
        }
        f.silent = !self.gate_open;
        f.silent_for = if f.silent { f.silent_for + dt } else { 0.0 };
        ema(&mut self.ms_mom, ms, dt, 0.4);
        ema(&mut self.ms_short, ms, dt, 3.0);
        let mom_db = 10.0 * (self.ms_mom + 1e-12).log10();
        if self.gate_open {
            let tau = if mom_db > self.loud_ref { 1.0 } else { 20.0 };
            ema(&mut self.loud_ref, mom_db, dt, tau);
        }
        f.loud = (1.0 + (mom_db - self.loud_ref) / 24.0).clamp(0.0, 1.0);
        let short_db = 10.0 * (self.ms_short + 1e-12).log10();
        f.energy = (1.0 + (short_db - self.loud_ref) / 24.0).clamp(0.0, 1.0);

        // onsets / tempo on each complete hop
        for h in 0..hops {
            let end = need - backlog - (hops - 1 - h) * HOP;
            self.odf_hop(end);
        }
        self.beat_track(dt);

        // display spectrum, both channels
        let mut max_l = -120.0f32;
        let mut levels = [vec![0.0f32; self.nbars], vec![0.0f32; self.nbars]];
        let mut powers = vec![0.0f32; self.nbars];
        for ch in 0..2 {
            for k in 0..3 {
                let src = if ch == 0 { &self.buf_l[..need] } else { &self.buf_r[..need] };
                self.ffts[k].run(src);
            }
            for (i, b) in self.bands.iter().enumerate() {
                let p = self.band_power(b);
                powers[i] += 0.5 * p;
                let l = 10.0 * (p + 1e-12).log10() + TILT * (b.fc / 1000.0).log2();
                levels[ch][i] = l;
                max_l = max_l.max(l);
            }
        }
        if self.gate_open {
            self.started += dt;
            let tau = if max_l > self.lref {
                0.25
            } else if self.started < 2.0 {
                0.6
            } else {
                5.0
            };
            ema(&mut self.lref, max_l, dt, tau);
            self.lref = self.lref.clamp(-70.0, 12.0);
        }
        let range = RANGE_DB;
        let floor = self.lref - range;
        for ch in 0..2 {
            for i in 0..self.nbars {
                let h = ((levels[ch][i] - floor) / range).clamp(0.0, 1.0).powf(GAMMA);
                let h = if self.gate_open { h } else { 0.0 };
                let out = if ch == 0 { &mut self.f.left } else { &mut self.f.right };
                let y = &mut out[i];
                let pk = &mut self.peak[ch][i];
                let t = &mut self.fall_t[ch][i];
                let grav = *pk * (1.0 - (*t / 0.42).powi(2)).max(0.0);
                if h >= grav {
                    *pk = h;
                    *t = 0.0;
                    *y += (h - *y) * (1.0 - (-dt / 0.012f32).exp());
                } else {
                    *t += dt;
                    *y = grav.max(h);
                }
            }
        }
        let f = &mut self.f;
        for i in 0..self.nbars {
            let m = 0.5 * (f.left[i] + f.right[i]);
            f.mono[i] = m;
            // caps: hold 0.4 s, then fall with constant acceleration
            if m >= f.caps[i] {
                f.caps[i] = m;
                self.cap_hold[i] = 0.25;
                self.cap_vel[i] = 0.0;
            } else if self.cap_hold[i] > 0.0 {
                self.cap_hold[i] -= dt;
            } else {
                self.cap_vel[i] += 6.0 * dt;
                f.caps[i] = (f.caps[i] - self.cap_vel[i] * dt).max(m);
            }
            self.raw_h[i] = ((0.5 * (levels[0][i] + levels[1][i]) - floor) / range).clamp(0.0, 1.0);
        }

        self.spectral_features(&powers, dt);

        let f = &mut self.f;
        f.wave_l.resize(WAVE, 0.0);
        f.wave_r.resize(WAVE, 0.0);
        f.wave_l.copy_from_slice(&self.buf_l[need - WAVE..need]);
        f.wave_r.copy_from_slice(&self.buf_r[need - WAVE..need]);
        let decay = |e: &mut f32, x: f32, tau: f32| *e = (*e * (-dt / tau).exp()).max(x);
        decay(&mut f.kick_env, f.kick, 0.18);
        decay(&mut f.snare_env, f.snare, 0.14);
        decay(&mut f.hat_env, f.hat, 0.08);

        // intensity: a blend of everything that reads as "busy", absolute
        // enough that a quiet track stays low even once the auto-gain settles
        ema(&mut self.punch, f.kick_env.max(f.snare_env).min(1.0), dt, 1.0);
        let dens = ((f.onset_rate - 0.5) / 5.0).clamp(0.0, 1.0);
        let abs = ((10.0 * (self.ms_short + 1e-12).log10() + 42.0) / 26.0).clamp(0.0, 1.0);
        let lit = if f.mono.is_empty() { 0.0 } else { f.mono.iter().sum::<f32>() / f.mono.len() as f32 };
        let lit = ((lit - 0.1) / 0.45).clamp(0.0, 1.0);
        let raw = 0.35 * dens + 0.15 * f.energy.powi(2) + 0.15 * abs + 0.15 * lit + 0.2 * (self.punch * 1.6).min(1.0);
        let raw = if f.silent { 0.0 } else { raw };
        let tau = if raw > f.intensity { 0.4 } else { 2.0 };
        ema(&mut f.intensity, raw, dt, tau);
    }

    fn spectral_features(&mut self, powers: &[f32], dt: f32) {
        let f = &mut self.f;
        // centroid + flatness over the bars
        let (mut num, mut den, mut lsum, mut n) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let mut e = [0.0f32; 3];
        for (b, &p) in self.bands.iter().zip(powers) {
            num += b.fc * p;
            den += p;
            if p > 1e-12 {
                lsum += p.ln();
                n += 1.0;
            }
            let g = if b.fc < 250.0 { 0 } else if b.fc < 2000.0 { 1 } else { 2 };
            e[g] += p;
        }
        if self.gate_open && den > 1e-10 {
            let c = ((num / den / 100.0).log2() / 80f32.log2()).clamp(0.0, 1.0);
            ema(&mut f.centroid, c, dt, 0.15);
            let flat = if n > 0.0 { (lsum / n).exp() / (den / powers.len() as f32) } else { 0.0 };
            ema(&mut f.flatness, flat.clamp(0.0, 1.0), dt, 0.3);
            for g in 0..3 {
                ema(&mut self.e_bands[g], e[g], dt, 0.05);
                ema(&mut self.e_long[g], e[g], dt, 8.0);
            }
        }
        let rel = |g: usize| (self.e_bands[g] / self.e_long[g].max(1e-9)).clamp(0.0, 4.0);
        f.bass = rel(0);
        f.mid = rel(1);
        f.treb = rel(2);
        ema(&mut f.bass_att, f.bass, dt, 1.0);
        ema(&mut f.mid_att, f.mid, dt, 1.0);
        ema(&mut f.treb_att, f.treb, dt, 1.0);

        // build-up / drop: a stretch of missing bass, then bass back with a kick
        if self.gate_open {
            ema(&mut self.bass_short, e[0], dt, 0.5);
            ema(&mut self.bass_fast, e[0], dt, 0.06);
            ema(&mut self.bass_long, e[0], dt, 8.0);
            ema(&mut self.cent_s, f.centroid, dt, 1.0);
            ema(&mut self.cent_l, f.centroid, dt, 8.0);
        }
        if self.bass_short < 0.5 * self.bass_long {
            if self.low_time == 0.0 {
                self.drop_ref = self.bass_long;
            }
            self.low_time += dt;
            if self.low_time > 2.0 {
                self.armed = true;
            }
        } else if self.bass_short > 0.8 * self.bass_long {
            self.low_time = 0.0;
        }
        // the bass coming back is the drop; don't wait for the onset picker
        if self.armed && self.bass_fast > 0.7 * self.drop_ref {
            f.drop = true;
            self.armed = false;
            self.low_time = 0.0;
        }
        if self.armed && self.low_time > 20.0 {
            self.armed = false; // a quiet section, not a build
        }
        let low = (self.low_time / 8.0).clamp(0.0, 1.0);
        let rise = ((self.cent_s - self.cent_l) * 6.0).clamp(0.0, 1.0);
        let target = if self.armed { (0.6 * low + 0.4 * rise).clamp(0.0, 1.0) } else { 0.0 };
        let tau = if target > f.tension { 1.0 } else { 0.3 };
        ema(&mut f.tension, target, dt, tau);

        // novelty: 4 s vs the 8 s before it, 4 Hz feature frames
        self.nov_timer += dt;
        self.nov_since += dt;
        if self.nov_timer >= 0.25 && self.gate_open {
            self.nov_timer = 0.0;
            let k = 16;
            let per = (self.raw_h.len() / k).max(1);
            let mut v: Vec<f32> = (0..k)
                .map(|i| {
                    let s = &self.raw_h[(i * per).min(self.raw_h.len())..((i + 1) * per).min(self.raw_h.len())];
                    if s.is_empty() { 0.0 } else { s.iter().sum::<f32>() / s.len() as f32 }
                })
                .collect();
            v.push(f.centroid);
            v.push(f.flatness * 2.0);
            self.nov_feat.push(v);
            if self.nov_feat.len() > 64 {
                self.nov_feat.remove(0);
            }
            if self.nov_feat.len() >= 48 {
                let l = self.nov_feat.len();
                let mean = |r: std::ops::Range<usize>| {
                    let mut m = vec![0.0f32; k + 2];
                    for fv in &self.nov_feat[r.clone()] {
                        for (a, b) in m.iter_mut().zip(fv) {
                            *a += b;
                        }
                    }
                    m
                };
                let (a, b) = (mean(l - 16..l), mean(l - 48..l - 16));
                let dot: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
                let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
                let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
                let d = 1.0 - dot / (na * nb + 1e-9);
                let hist = &self.nov_d;
                if hist.len() >= 40 {
                    let m = hist.iter().sum::<f32>() / hist.len() as f32;
                    let sd = (hist.iter().map(|x| (x - m).powi(2)).sum::<f32>() / hist.len() as f32).sqrt();
                    if d > m + 1.5 * sd && d > 0.01 && self.nov_since > 10.0 {
                        f.section = true;
                        self.nov_since = 0.0;
                    }
                }
                self.nov_d.push(d);
                if self.nov_d.len() > 240 {
                    self.nov_d.remove(0);
                }
            }
        }
    }

    /// One 10 ms hop of SuperFlux ending at buffer index `end`.
    fn odf_hop(&mut self, end: usize) {
        if end < ODF_N {
            return;
        }
        self.odf_fft.run(&self.mono[..end]);
        std::mem::swap(&mut self.prev_log, &mut self.cur_log);
        let scale = (ODF_N as f32 / 4.0).powi(2); // back to raw |X|² scale
        for (j, &(k0, k1, _)) in self.odf_bands.iter().enumerate() {
            let k1 = k1.min(self.odf_fft.pow.len());
            let mag: f32 = self.odf_fft.pow[k0..k1].iter().map(|p| (p * scale).sqrt()).sum();
            self.cur_log[j] = (1.0 + mag).log10();
        }
        let nb = self.odf_bands.len();
        let (mut all, mut kick, mut snare, mut hat) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for j in 0..nb {
            let lo = j.saturating_sub(1);
            let hi = (j + 1).min(nb - 1);
            let prev_max = self.prev_log[lo..=hi].iter().cloned().fold(0.0f32, f32::max);
            let d = (self.cur_log[j] - prev_max).max(0.0);
            let fc = self.odf_bands[j].2;
            all += d;
            if (30.0..150.0).contains(&fc) {
                kick += d;
            } else if (150.0..500.0).contains(&fc) || (1500.0..5000.0).contains(&fc) {
                snare += d;
            } else if (6000.0..16_000.0).contains(&fc) {
                hat += d;
            }
        }
        let g = self.gate_open;
        let a = self.on_all.step(all, 1.3, 0.03, g);
        let k = self.on_kick.step(kick, 1.6, 0.12, g);
        let s = self.on_snare.step(snare, 1.6, 0.10, g);
        let h = self.on_hat.step(hat, 1.4, 0.05, g);
        self.f.kick = self.f.kick.max(k);
        self.f.snare = self.f.snare.max(s);
        self.f.hat = self.f.hat.max(h);
        if a > 0.0 || k > 0.0 {
            self.onset_times.push(self.clock);
            self.pll_onset(if k > 0.0 { 1.0 } else { 0.5 });
        }
        let clock = self.clock;
        self.onset_times.retain(|&t| clock - t < 2.0);
        self.f.onset_rate = self.onset_times.len() as f32 / 2.0;
        self.odf_hist[self.odf_pos] = if g { all } else { 0.0 };
        self.odf_pos = (self.odf_pos + 1) % TEMPO_LEN;
    }

    fn pll_onset(&mut self, weight: f32) {
        let e = if self.phase < 0.5 { self.phase } else { self.phase - 1.0 };
        if e.abs() < 0.15 {
            self.phase -= 0.15 * e * weight;
            self.period *= 1.0 + 0.03 * e * weight;
            self.period = self.period.clamp(0.3, 1.0);
            self.matched = true;
        } else if self.f.beat_conf < 0.3 && weight >= 1.0 {
            self.phase = 0.0; // not locked: snap to a strong kick
            self.fired = true;
        }
    }

    fn beat_track(&mut self, dt: f32) {
        // tempo re-estimate once a second (Ellis 2007)
        self.tempo_timer += dt;
        if self.tempo_timer >= 1.0 {
            self.tempo_timer = 0.0;
            self.estimate_tempo();
        }
        let prev = self.phase;
        self.phase += dt / self.period;
        let lead = (0.03 / self.period).min(0.2);
        let f = &mut self.f;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.fired = false;
        }
        if !self.fired && self.phase >= 1.0 - lead && prev < 1.0 - lead {
            self.fired = true;
            f.beat = !f.silent;
            if f.beat {
                f.beat_count += 1;
            }
        }
        // confidence: did an onset land near the last predicted beat?
        if self.phase >= 0.5 && !self.checked {
            self.checked = true;
            let hit = if self.matched { 1.0 } else { 0.0 };
            f.beat_conf = f.beat_conf * 0.85 + 0.15 * hit;
            self.matched = false;
        }
        if self.phase < 0.5 {
            self.checked = false;
        }
        if f.silent {
            f.beat_conf *= (-dt / 2.0).exp();
        }
        f.beat_phase = self.phase;
        f.bpm = 60.0 / self.period;
        if f.drop || f.section {
            // re-anchor the bar: the next beat is a downbeat
            f.beat_count = f.beat_count - f.beat_count % 4 + 3;
        }
    }

    fn estimate_tempo(&mut self) {
        let n = TEMPO_LEN;
        let o: Vec<f32> = (0..n).map(|i| self.odf_hist[(self.odf_pos + i) % n]).collect();
        let mean = o.iter().sum::<f32>() / n as f32;
        if mean < 1e-4 {
            return;
        }
        let o: Vec<f32> = o.iter().map(|x| x - mean).collect();
        let maxlag = 210;
        let mut tps = vec![0.0f32; maxlag + 2];
        for lag in 20..=maxlag + 1 {
            let ac: f32 = (lag..n).map(|t| o[t] * o[t - lag]).sum::<f32>() / (n - lag) as f32;
            let tau = lag as f32 / HOP_RATE;
            let w = (-0.5 * ((tau / 0.5).log2() / 1.4).powi(2)).exp();
            tps[lag] = w * ac;
        }
        let mut best = (0usize, f32::MIN);
        let score = |lag: usize| {
            tps[lag] + 0.5 * tps[2 * lag] + 0.25 * (tps[2 * lag - 1] + tps[2 * lag + 1])
        };
        for lag in 33..=100 {
            let s = score(lag);
            if s > best.1 {
                best = (lag, s);
            }
        }
        if best.1 <= 0.0 {
            return;
        }
        // parabolic refinement
        let (l, s0) = best;
        let (sm, sp) = (score(l - 1), score(l + 1));
        let den = sm - 2.0 * s0 + sp;
        let off = if den.abs() > 1e-12 { (0.5 * (sm - sp) / den).clamp(-0.5, 0.5) } else { 0.0 };
        let cand = (l as f32 + off) / HOP_RATE;
        let rel = (cand - self.period).abs() / self.period;
        let cur_lag = (self.period * HOP_RATE).round() as usize;
        let cur_score = if (33..=100).contains(&cur_lag) { score(cur_lag) } else { 0.0 };
        if rel < 0.04 {
            self.period += (cand - self.period) * 0.3;
            self.tempo_cand_n = 0;
        } else if s0 > 1.2 * cur_score.max(0.0) || self.period_score == 0.0 {
            if (cand - self.tempo_cand).abs() / cand < 0.04 {
                self.tempo_cand_n += 1;
            } else {
                self.tempo_cand = cand;
                self.tempo_cand_n = 1;
            }
            if self.tempo_cand_n >= 2 || self.period_score == 0.0 {
                self.period = cand;
                self.tempo_cand_n = 0;
            }
        }
        self.period_score = s0;
    }
}
