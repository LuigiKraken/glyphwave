//! Audio capture from the default sink's monitor via `parec` (PipeWire's pulse
//! shim), or `pw-record` where the pulse tools aren't installed (Fedora ships
//! pipewire-utils, not always pulseaudio-utils). Float32 stereo at 48 kHz —
//! PipeWire's native rate, so no resampling. A reader thread keeps the newest samples in a ring; the DSP copies the
//! window it needs each frame (cava does the same with its input buffer).
//!
//! The monitor records the full signal even with the sink muted or at 0 %,
//! so `watch_sink` follows the default sink's mute and volume (`pactl
//! subscribe`, re-read on each sink event) and main treats silence by the
//! knob like a pause.

use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const RATE: u32 = 48_000;
pub const RING: usize = 16_384; // per channel, > the largest FFT (8192)

pub struct Ring {
    pub l: Vec<f32>,
    pub r: Vec<f32>,
    pub pos: usize,   // next write index
    pub total: u64,   // samples ever written (per channel)
}

impl Ring {
    fn new() -> Ring {
        Ring { l: vec![0.0; RING], r: vec![0.0; RING], pos: 0, total: 0 }
    }

    /// Copy the newest `n` samples (oldest first) into `l`, `r`.
    pub fn latest(&self, n: usize, l: &mut [f32], r: &mut [f32]) {
        let n = n.min(RING);
        let start = (self.pos + RING - n) % RING;
        for i in 0..n {
            let j = (start + i) % RING;
            l[i] = self.l[j];
            r[i] = self.r[j];
        }
    }
}

/// Capture tools in order of preference. Both write raw float32le stereo
/// frames to stdout, so one reader serves either.
const TOOLS: [&str; 2] = ["parec", "pw-record"];

fn command(tool: &str) -> Command {
    let mut c = Command::new(tool);
    if tool == "parec" {
        c.args([
            "-d",
            "@DEFAULT_MONITOR@",
            "--format=float32le",
            &format!("--rate={RATE}"),
            "--channels=2",
            "--latency-msec=15",
            "--raw",
            "--client-name=glyphwave",
        ]);
    } else {
        // capture.sink has the session manager link it to the default sink's
        // monitor, and follow the default when it changes
        c.args([
            "--format=f32",
            &format!("--rate={RATE}"),
            "--channels=2",
            "--latency=15ms",
            "--raw",
            "-P",
            "stream.capture.sink=true",
            "-P",
            "node.name=glyphwave",
            "-",
        ]);
    }
    c
}

pub struct Capture {
    child: Option<Child>,
    /// When a failed or dead capture tool may be tried again.
    retry: Option<Instant>,
    /// Index into TOOLS of the first one not known to be missing or broken.
    tool: usize,
    /// When the child started, and how many times in a row one died within
    /// a second of starting (no sound server, an old pw-record without
    /// --raw): two of those and the next tool is tried.
    since: Instant,
    quick_deaths: u32,
    /// No capture tool is installed or works; said once on exit.
    pub missing: bool,
    pub ring: Arc<Mutex<Ring>>,
}

impl Capture {
    pub fn new() -> Capture {
        Capture {
            child: None,
            retry: None,
            tool: 0,
            since: Instant::now(),
            quick_deaths: 0,
            missing: false,
            ring: Arc::new(Mutex::new(Ring::new())),
        }
    }

    pub fn running(&mut self) -> bool {
        match &mut self.child {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    pub fn start(&mut self) {
        if self.missing || self.running() {
            return;
        }
        // called every frame while playing, so a death is seen at once
        if let Some(mut c) = self.child.take() {
            let _ = c.wait();
            self.quick_deaths = if self.since.elapsed() < Duration::from_secs(1) { self.quick_deaths + 1 } else { 0 };
            if self.quick_deaths >= 2 {
                self.tool += 1;
                self.quick_deaths = 0;
            }
        }
        if self.retry.is_some_and(|r| Instant::now() < r) {
            return;
        }
        self.stop();
        // without a capture tool (or a sound server) don't respawn it every frame
        self.retry = Some(Instant::now() + Duration::from_secs(5));
        let mut child = loop {
            let Some(tool) = TOOLS.get(self.tool) else {
                self.missing = true;
                return;
            };
            match command(tool).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
                Ok(c) => break c,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.tool += 1,
                Err(_) => return,
            }
        };
        let mut out = child.stdout.take().unwrap();
        let ring = self.ring.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 4096];
            let mut carry: Vec<u8> = Vec::with_capacity(8);
            loop {
                let n = match out.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                carry.extend_from_slice(&buf[..n]);
                let frames = carry.len() / 8;
                let mut g = ring.lock().unwrap();
                for f in 0..frames {
                    let b = &carry[f * 8..f * 8 + 8];
                    let l = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                    let r = f32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                    let p = g.pos;
                    g.l[p] = l;
                    g.r[p] = r;
                    g.pos = (p + 1) % RING;
                }
                g.total += frames as u64;
                drop(g);
                carry.drain(..frames * 8);
            }
        });
        self.child = Some(child);
        self.since = Instant::now();
    }

    pub fn stop(&mut self) {
        self.retry = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let mut g = self.ring.lock().unwrap();
        g.l.fill(0.0);
        g.r.fill(0.0);
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Have the child go when glyphwave does, even if it dies without stopping
/// it. The signal follows the thread that spawns, so that thread must live
/// as long as the process.
pub fn die_with_us(c: &mut Command) -> &mut Command {
    // pre_exec: only prctl, which is async-signal-safe
    unsafe { c.pre_exec(|| { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM); Ok(()) }) }
}

/// `pactl` with untranslated output.
fn pactl(args: &[&str]) -> Command {
    let mut c = Command::new("pactl");
    c.args(args).env("LC_ALL", "C").stdin(Stdio::null()).stderr(Stdio::null());
    c
}

/// `pactl get-sink-mute` / `get-sink-volume` output → whether nothing can be
/// heard: muted, or every channel at 0 %.
fn hushed(mute: &str, volume: &str) -> bool {
    let muted = mute.trim() == "Mute: yes";
    let mut pcts = volume.split_whitespace().filter_map(|w| w.strip_suffix('%')?.parse::<u32>().ok()).peekable();
    muted || (pcts.peek().is_some() && pcts.all(|p| p == 0))
}

fn read_hushed() -> bool {
    let out = |a: &str| {
        pactl(&[a, "@DEFAULT_SINK@"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    hushed(&out("get-sink-mute"), &out("get-sink-volume"))
}

/// Follow the default sink's mute and volume. One `pactl subscribe` sleeps
/// on its pipe; a sink or server event (the default changing) re-reads the
/// state, once per burst. Without pactl it just stays false. The thread
/// lives until the subscription ends, so the child dies with glyphwave.
pub fn watch_sink() -> Arc<AtomicBool> {
    let hushed = Arc::new(AtomicBool::new(false));
    let h = hushed.clone();
    std::thread::spawn(move || {
        let Ok(mut child) = die_with_us(&mut pactl(&["subscribe"])).stdout(Stdio::piped()).spawn() else { return };
        h.store(read_hushed(), Ordering::Relaxed);
        let mut rd = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        let mut dirty = false;
        while rd.read_line(&mut line).is_ok_and(|n| n > 0) {
            // "Event 'change' on sink #56", not sink-input (every stream's own)
            dirty |= line.contains(" on sink #") || line.contains(" on server");
            line.clear();
            if dirty && rd.buffer().is_empty() {
                dirty = false;
                h.store(read_hushed(), Ordering::Relaxed);
            }
        }
        h.store(false, Ordering::Relaxed);
        let _ = child.wait();
    });
    hushed
}

/// How busy the demo track is, 0 (just the pad) to 6; 4 is the full mix
/// (`[` / `]` in `--test`).
pub static DEMO_LEVEL: AtomicU32 = AtomicU32::new(4);

/// A synthetic 124 BPM track (kick, off-beat hats, snare on 2/4, bass, pad,
/// with a breakdown and a drop every 32 bars) written into the ring in real
/// time. For `--demo` and for testing without a player.
pub fn start_synth(ring: Arc<Mutex<Ring>>) {
    std::thread::spawn(move || {
        let sr = RATE as f32;
        let bpm = 124.0f32;
        let beat = 60.0 / bpm;
        let mut t = 0.0f64;
        let mut seed = 0x1234_5678u32;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 * 2.0 - 1.0
        };
        let chunk = 480;
        let start = std::time::Instant::now();
        let mut written = 0u64;
        let notes = [55.0f32, 55.0, 65.41, 49.0]; // A1 A1 C2 G1, one per bar
        let mut lp = 0.0f32;
        loop {
            // drums and bass fade out below the full mix, push past it above
            let lv = DEMO_LEVEL.load(Ordering::Relaxed) as f32 / 4.0;
            let (drums, bass) = (lv, lv.min(1.0));
            let mut g = ring.lock().unwrap();
            for _ in 0..chunk {
                let tt = t as f32;
                let b = tt / beat; // beats elapsed
                let bar = (b / 4.0) as usize;
                let section = bar % 32;
                let breakdown = (24..32).contains(&section);
                let pb = b.fract() * beat; // seconds since beat
                let mut s = 0.0f32;
                if !breakdown {
                    let f = 45.0 + 120.0 * (-pb * 30.0).exp();
                    s += drums * 0.9 * (std::f32::consts::TAU * f * pb).sin() * (-pb * 7.0).exp();
                    let root = notes[bar % 4];
                    let ph = (tt * root).fract();
                    s += bass * 0.25 * (ph * 2.0 - 1.0) * (0.6 + 0.4 * (-pb * 4.0).exp());
                }
                let off = ((b + 0.5).fract()) * beat;
                s += drums * 0.18 * noise() * (-off * 40.0).exp() * if breakdown { 0.4 } else { 1.0 };
                let beat_in_bar = (b as usize) % 4;
                if !breakdown && (beat_in_bar == 1 || beat_in_bar == 3) {
                    s += drums * 0.35 * noise() * (-pb * 18.0).exp();
                    s += drums * 0.3 * (std::f32::consts::TAU * 190.0 * pb).sin() * (-pb * 25.0).exp();
                }
                // pad: detuned saws through a lowpass that opens in the breakdown
                let pad = [220.0f32, 261.6, 329.6]
                    .iter()
                    .map(|f| ((tt * f).fract() + (tt * f * 1.003).fract()) - 1.0)
                    .sum::<f32>();
                let cut = if breakdown { 0.02 + 0.2 * ((section - 24) as f32 / 8.0 + b.fract() / 32.0) } else { 0.03 };
                lp += (pad - lp) * cut;
                s += 0.12 * lp;
                if breakdown && section >= 30 {
                    s += 0.15 * noise() * ((section - 30) as f32 * 4.0 + b.fract() * 4.0) / 8.0;
                }
                let s = (s * 0.6).tanh() * (0.25 + 0.25 * lv);
                let p = g.pos;
                g.l[p] = s;
                g.r[p] = s * 0.9 + 0.05 * noise();
                g.pos = (p + 1) % RING;
                t += 1.0 / sr as f64;
            }
            g.total += chunk as u64;
            drop(g);
            written += chunk as u64;
            let due = std::time::Duration::from_secs_f64(written as f64 / sr as f64);
            if let Some(w) = due.checked_sub(start.elapsed()) {
                std::thread::sleep(w);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::hushed;

    const VOL: &str = "Volume: front-left: 42598 /  65% / -11.23 dB,   front-right: 42598 /  65% / -11.23 dB\n        balance 0.00\n";
    const ZERO: &str = "Volume: front-left: 0 /   0% / -inf dB,   front-right: 0 /   0% / -inf dB\n        balance 0.00\n";

    #[test]
    fn muted_or_at_zero_is_hushed() {
        assert!(!hushed("Mute: no\n", VOL));
        assert!(hushed("Mute: yes\n", VOL));
        assert!(hushed("Mute: no\n", ZERO));
        assert!(!hushed("Mute: no\n", "Volume: front-left: 0 /   0% / -inf dB,   front-right: 655 /   1% / -120.00 dB"));
        // no sink, or pactl failed: not hushed
        assert!(!hushed("", ""));
    }
}
