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

/// One ring of the built-in tone, built on the first call: a soft G5–E5
/// chime twice, then quiet; 2.5 s of mono float32le.
fn tone() -> &'static [u8] {
    static TONE: OnceLock<Vec<u8>> = OnceLock::new();
    TONE.get_or_init(|| {
        let sr = RATE as f32;
        let notes = [(0.0f32, 784.0f32), (0.22, 659.3), (0.6, 784.0), (0.82, 659.3)];
        let n = (2.5 * sr) as usize;
        let mut out = Vec::with_capacity(n * 4);
        for i in 0..n {
            let t = i as f32 / sr;
            let mut s = 0.0f32;
            for &(at, f) in &notes {
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

    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
