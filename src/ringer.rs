//! What a ringing call does to the sound: the playing player's own stream
//! fades out (`pactl set-sink-input-volume`, never the sink, so the call
//! app's ring stays audible), the player pauses and gets its volume back for
//! the next play; and a ringtone loops through `pw-play` or `paplay` until the
//! ring ends. A ring that ends on its own (missed, declined elsewhere) plays
//! the music again and fades it back in, like a phone. Spotify's MPRIS Volume would do for the fade, Chromium's has
//! none, so the stream it is. Nothing runs while no call rings.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const FADE: Duration = Duration::from_millis(450);
const FADE_IN: Duration = Duration::from_millis(1500);
const STEPS: u32 = 12;
const RATE: u32 = crate::audio::RATE;

/// The player's sink inputs (id, raw volume per channel), matched by the bus
/// name's app part (`org.mpris.MediaPlayer2.brave.instance42` → brave)
/// against the stream's application name, binary or node name. Corked
/// (paused) ones only with `corked`.
fn streams(key: &str, corked_too: bool) -> Vec<(String, Vec<u32>)> {
    let Ok(out) = Command::new("pactl").args(["list", "sink-inputs"]).env("LC_ALL", "C").stderr(Stdio::null()).output() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut found = Vec::new();
    for block in text.split("Sink Input #").skip(1) {
        let id = block.lines().next().unwrap_or("").trim().to_string();
        let (mut vol, mut corked, mut ours) = (Vec::new(), false, false);
        for l in block.lines().map(str::trim) {
            if let Some(v) = l.strip_prefix("Volume:") {
                // "aux0: 65536 / 100% / 0.00 dB,   aux1: 65536 / …"
                vol = v.split(',').filter_map(|c| c.split_once(':')?.1.split_whitespace().next()?.parse().ok()).collect();
            }
            corked |= l == "Corked: yes";
            if let Some((k, v)) = l.split_once(" = ") {
                let named = matches!(k, "application.name" | "application.process.binary" | "node.name");
                ours |= named && v.trim_matches('"').to_lowercase().contains(key);
            }
        }
        if ours && (corked_too || !corked) && !vol.is_empty() {
            found.push((id, vol));
        }
    }
    found
}

fn set_volume(id: &str, vol: &[u32], k: f32) {
    let vols = vol.iter().map(|&v| ((v as f32 * k) as u32).to_string());
    let _ = Command::new("pactl").arg("set-sink-input-volume").arg(id).args(vols).stderr(Stdio::null()).status();
}

/// Fade `player`'s stream to silence, `pause` it, then put the volume back.
/// Joined on exit, so a key that ends glyphwave mid-fade can't leave it silent.
pub fn fade_and_pause(player: &str, pause: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    let key = app(player);
    std::thread::spawn(move || {
        let found = if key.is_empty() { Vec::new() } else { streams(&key, false) };
        ramp(&found, FADE, |k| 1.0 - k);
        pause();
        // the player still drains what it buffered; let that go out silent
        std::thread::sleep(Duration::from_millis(300));
        for (id, vol) in &found {
            set_volume(id, vol, 1.0);
        }
    })
}

/// Play `player` again (after `before`, the fade out, if it is still going)
/// and fade it in. Its stream is still there, corked, while it is paused, so
/// it is silenced first and starts from nothing; a player that dropped its
/// stream just starts.
pub fn play_and_fade_in(player: &str, before: Option<JoinHandle<()>>, play: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    let key = app(player);
    std::thread::spawn(move || {
        if let Some(b) = before {
            let _ = b.join();
        }
        let found = if key.is_empty() { Vec::new() } else { streams(&key, true) };
        for (id, vol) in &found {
            set_volume(id, vol, 0.0);
        }
        play();
        ramp(&found, FADE_IN, |k| k);
    })
}

/// The bus name's app part: `org.mpris.MediaPlayer2.brave.instance42` → brave.
fn app(player: &str) -> String {
    player.trim_start_matches("org.mpris.MediaPlayer2.").split('.').next().unwrap_or("").to_lowercase()
}

/// Step the streams' volume through `level(0..1]` over `len`.
fn ramp(found: &[(String, Vec<u32>)], len: Duration, level: impl Fn(f32) -> f32) {
    let start = Instant::now();
    for i in 1..=STEPS {
        for (id, vol) in found {
            set_volume(id, vol, level(i as f32 / STEPS as f32));
        }
        // on a clock, since each pactl takes a few ms itself
        if let Some(w) = (len * i / STEPS).checked_sub(start.elapsed()) {
            std::thread::sleep(w);
        }
    }
}

/// The built-in tone's notes (start in s, Hz) and the length of one round;
/// the call view pulses on them.
pub const NOTES: [(f32, f32); 4] = [(0.0, 784.0), (0.22, 659.3), (0.6, 784.0), (0.82, 659.3)];
pub const ROUND: f32 = 2.5;

/// One ring of the built-in tone, built on the first call: a soft G5–E5
/// chime twice, then quiet; 2.5 s of mono float32le (the demo plays it too).
pub fn tone() -> &'static [u8] {
    static TONE: OnceLock<Vec<u8>> = OnceLock::new();
    TONE.get_or_init(|| {
        let sr = RATE as f32;
        let n = (ROUND * sr) as usize;
        let mut out = Vec::with_capacity(n * 4);
        for i in 0..n {
            let t = i as f32 / sr;
            let mut s = 0.0f32;
            for &(at, f) in &NOTES {
                let d = t - at;
                if (0.0..0.8).contains(&d) {
                    let env = (d / 0.005).min(1.0) * (-d * 6.0).exp();
                    let w = std::f32::consts::TAU * f * d;
                    s += env * (w.sin() + 0.25 * (2.0 * w).sin());
                }
            }
            out.extend_from_slice(&(s * 0.12).to_le_bytes());
        }
        out
    })
}

enum Tone {
    Builtin,
    Off,
    File(String),
}

pub struct Ringer {
    tone: Tone,
    child: Option<Child>,
    since: Instant,
    gave_up: bool,
}

impl Ringer {
    /// `cfg` is the config's `ringtone`: default, none or a sound file.
    pub fn new(cfg: Option<&str>) -> Ringer {
        let tone = match cfg {
            None | Some("default") => Tone::Builtin,
            Some("none") => Tone::Off,
            Some(p) => Tone::File(p.to_string()),
        };
        Ringer { tone, child: None, since: Instant::now(), gave_up: false }
    }

    fn spawn(&self) -> Option<Child> {
        for tool in ["pw-play", "paplay"] {
            let mut c = Command::new(tool);
            match (&self.tone, tool) {
                (Tone::File(p), "pw-play") => c.args(["--media-role=Notification", p]),
                (Tone::File(p), _) => c.args(["--property=media.role=event", p]),
                (_, "pw-play") => c.args(["-a", "--format=f32", "--rate=48000", "--channels=1", "--media-role=Notification", "-"]),
                _ => c.args(["--raw", "--format=float32le", "--rate=48000", "--channels=1", "--property=media.role=event"]),
            };
            c.stdin(if matches!(self.tone, Tone::Builtin) { Stdio::piped() } else { Stdio::null() });
            c.stdout(Stdio::null()).stderr(Stdio::null());
            match crate::audio::die_with_us(&mut c).spawn() {
                Ok(mut child) => {
                    if let Some(mut stdin) = child.stdin.take() {
                        // the pipe paces the writes; closing it ends the player
                        std::thread::spawn(move || { let _ = stdin.write_all(tone()); });
                    }
                    return Some(child);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return None,
            }
        }
        None
    }

    /// Each frame: keep the ringtone going while `ringing` (one more round
    /// when the last one ends), stop it the moment the ring ends.
    pub fn ring(&mut self, ringing: bool) {
        if !ringing {
            self.stop();
            self.gave_up = false;
            return;
        }
        if self.gave_up || matches!(self.tone, Tone::Off) {
            return;
        }
        if let Some(c) = &mut self.child {
            if !matches!(c.try_wait(), Ok(Some(_))) {
                return;
            }
            // a file that won't play ends at once; don't respawn it each frame
            self.gave_up = self.since.elapsed() < Duration::from_millis(500);
            self.child = None;
            if self.gave_up {
                return;
            }
        }
        self.child = self.spawn();
        self.since = Instant::now();
        self.gave_up = self.child.is_none();
    }

    /// How far into its round the built-in tone is, when it's what plays
    /// (the player takes a moment to start sounding).
    pub fn round(&self) -> Option<f32> {
        let on = matches!(self.tone, Tone::Builtin) && self.child.is_some();
        on.then(|| (self.since.elapsed().as_secs_f32() - 0.05).max(0.0))
    }

    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NOTES, ROUND, tone};
    use crate::audio::{RATE, RING, Ring};
    use crate::dsp::Analyzer;

    /// Where the analyser hears onsets in `rounds` of `round` (mono samples,
    /// one round long), fed to it frame by frame like the capture does.
    fn heard(round: &[f32], rounds: usize) -> Vec<f32> {
        let mut ring = Ring { l: vec![0.0; RING], r: vec![0.0; RING], pos: 0, total: 0 };
        let mut an = Analyzer::new();
        an.set_bars(40);
        let per = RATE as usize / 30;
        let n = round.len() * rounds;
        let mut at = Vec::new();
        for f in 0..n / per {
            for i in f * per..(f + 1) * per {
                let s = round[i % round.len()];
                (ring.l[ring.pos], ring.r[ring.pos]) = (s, s);
                ring.pos = (ring.pos + 1) % RING;
                ring.total += 1;
            }
            an.update(&ring, 1.0 / 30.0);
            if an.f.onset > 0.0 {
                at.push(((f + 1) * per) as f32 / RATE as f32);
            }
        }
        at
    }

    fn near(at: &[f32], want: f32) -> bool {
        at.iter().any(|&t| (t - want).abs() < 0.08)
    }

    #[test]
    fn the_chime_is_heard_on_its_notes() {
        let round: Vec<f32> = tone().chunks(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let at = heard(&round, 3);
        // after the first round (the analyser warms up), every note
        for k in 1..3 {
            for &(n, _) in &NOTES {
                let want = k as f32 * ROUND + n;
                assert!(near(&at, want), "no onset near {want:.2} s in {at:?}");
            }
        }
    }

    #[test]
    fn another_ringtone_is_heard_too() {
        // a marimba-ish triplet at another pitch and pace, 1.8 s a round
        let sr = RATE as f32;
        let notes = [(0.0, 523.3), (0.3, 659.3), (0.6, 880.0)];
        let round: Vec<f32> = (0..(1.8 * sr) as usize)
            .map(|i| {
                let t = i as f32 / sr;
                notes.iter().filter(|&&(at, _)| t >= at).map(|&(at, f)| {
                    let d = t - at;
                    (-d * 9.0).exp() * (std::f32::consts::TAU * f * d).sin() * 0.2
                }).sum()
            })
            .collect();
        let at = heard(&round, 3);
        for k in 1..3 {
            for &(n, _) in &notes {
                let want = k as f32 * 1.8 + n;
                assert!(near(&at, want), "no onset near {want:.2} s in {at:?}");
            }
        }
    }
}
