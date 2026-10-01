//! ~/.config/glyphwave/config: plain `key = value` lines, `[section]`
//! headers, `#` comments. `glyphwave setup` writes it; glyphwave reads the
//! banner, fps and ringtone from it, the launcher the terminal and battery choice.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Then {
    Lock,
    ScreenOff,
    Sleep,
    Nothing,
}

impl Then {
    pub fn parse(s: &str) -> Option<Then> {
        Some(match s {
            "lock" => Then::Lock,
            "screen-off" => Then::ScreenOff,
            "sleep" => Then::Sleep,
            "none" => Then::Nothing,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Then::Lock => "lock",
            Then::ScreenOff => "screen-off",
            Then::Sleep => "sleep",
            Then::Nothing => "none",
        }
    }
}

/// Whether the desktop dims the screen while it runs (KDE).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dim {
    No,
    Yes,
    Battery,
}

impl Dim {
    pub fn parse(s: &str) -> Option<Dim> {
        Some(match s {
            "no" => Dim::No,
            "yes" => Dim::Yes,
            "battery" => Dim::Battery,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Dim::No => "no",
            Dim::Yes => "yes",
            Dim::Battery => "battery",
        }
    }
}

/// Minutes: start the screensaver after `start` idle, then act `after` later.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Times {
    pub start: u32,
    pub then: Then,
    pub after: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub ac: Times,
    pub on_battery: bool,
    /// [battery] overrides; None = same times as plugged in
    pub battery: Option<Times>,
    pub banner: Option<String>,
    pub fps: Option<f32>,
    /// default, none or a sound file; None = default
    pub ringtone: Option<String>,
    pub terminal: Option<String>,
    pub konsole_profile: Option<String>,
    pub hyprland_locker: String,
    pub sway_locker: String,
    pub x11_locker: String,
    /// the key that starts it now; None = no key
    pub shortcut: Option<String>,
    /// minutes into the screensaver after which dismissing it lands on the
    /// lock screen; Some(0) = always, None = never
    pub lock_after: Option<u32>,
    pub dim: Dim,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            ac: Times { start: 5, then: Then::Sleep, after: 10 },
            on_battery: true,
            battery: None,
            banner: None,
            fps: None,
            ringtone: None,
            terminal: None,
            konsole_profile: None,
            hyprland_locker: "hyprlock".into(),
            sway_locker: "swaylock -f".into(),
            x11_locker: "i3lock -c 000000".into(),
            shortcut: Some("Meta+Ctrl+L".into()),
            lock_after: None,
            dim: Dim::No,
        }
    }
}

pub fn dir() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"),
    }
}

pub fn path() -> PathBuf {
    dir().join("glyphwave/config")
}

/// The file if there is one; a broken line is reported and skipped.
pub fn load() -> Option<Config> {
    let p = path();
    let text = std::fs::read_to_string(&p).ok()?;
    let (cfg, errs) = parse(&text);
    for e in errs {
        eprintln!("glyphwave: {}: {e}", p.display());
    }
    Some(cfg)
}

fn minutes(v: &str) -> Result<u32, String> {
    match v.parse::<u32>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(format!("{v:?} isn't a number of minutes")),
    }
}

/// `lock_after`: minutes, 0 included, or none.
fn lock_minutes(v: &str) -> Result<Option<u32>, String> {
    match v {
        "none" => Ok(None),
        _ => v.parse::<u32>().map(Some).map_err(|_| format!("lock_after is a number of minutes (0 for always) or none, not {v:?}")),
    }
}

fn set(slot: &mut Option<String>, v: String) -> Result<(), String> {
    *slot = Some(v).filter(|v| !v.is_empty());
    Ok(())
}

fn set_str(slot: &mut String, v: String) -> Result<(), String> {
    *slot = v;
    Ok(())
}

fn yes_no(v: &str) -> Result<bool, String> {
    match v {
        "yes" | "true" | "on" => Ok(true),
        "no" | "false" | "off" => Ok(false),
        _ => Err(format!("{v:?} should be yes or no")),
    }
}

/// Parse the file; unknown keys and bad values come back as messages and
/// leave the default in place.
pub fn parse(text: &str) -> (Config, Vec<String>) {
    let mut c = Config::default();
    let (mut in_bat, mut bs, mut bt, mut ba) = (false, None, None, None);
    let mut errs = Vec::new();
    let mut section = String::new();
    for (n, raw) in text.lines().enumerate() {
        let line = match raw.find(" #").or(raw.find("\t#")) {
            Some(i) => &raw[..i],
            None => raw,
        }
        .trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.trim().to_string();
            in_bat |= section == "battery";
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            errs.push(format!("line {}: expected key = value", n + 1));
            continue;
        };
        let (k, v) = (k.trim(), v.trim().to_string());
        let then = |v: &str| Then::parse(v).ok_or(format!("then is lock, screen-off, sleep or none, not {v:?}"));
        let r: Result<(), String> = match (section.as_str(), k) {
            ("", "start_after") => minutes(&v).map(|m| c.ac.start = m),
            ("", "then") => then(&v).map(|t| c.ac.then = t),
            ("", "then_after") => minutes(&v).map(|m| c.ac.after = m),
            ("", "on_battery") => yes_no(&v).map(|y| c.on_battery = y),
            ("", "banner") => set(&mut c.banner, v),
            ("", "shortcut") => set(&mut c.shortcut, if v == "none" { String::new() } else { v }),
            ("", "fps") => v.parse::<f32>().map(|f| c.fps = Some(f)).map_err(|_| format!("fps {v:?} isn't a number")),
            ("", "ringtone") => set(&mut c.ringtone, v),
            ("", "lock_after") => lock_minutes(&v).map(|m| c.lock_after = m),
            ("", "dim") => Dim::parse(&v).map(|d| c.dim = d).ok_or(format!("dim is yes, no or battery, not {v:?}")),
            ("battery", "start_after") => minutes(&v).map(|m| bs = Some(m)),
            ("battery", "then") => then(&v).map(|t| bt = Some(t)),
            ("battery", "then_after") => minutes(&v).map(|m| ba = Some(m)),
            ("terminal", "terminal") => set(&mut c.terminal, v),
            ("terminal", "konsole_profile") => set(&mut c.konsole_profile, v),
            ("hyprland", "locker") => set_str(&mut c.hyprland_locker, v),
            ("sway", "locker") => set_str(&mut c.sway_locker, v),
            ("x11", "locker") => set_str(&mut c.x11_locker, v),
            (s, k) => Err(format!("unknown key {k}{}", if s.is_empty() { String::new() } else { format!(" in [{s}]") })),
        };
        if let Err(e) = r {
            errs.push(format!("line {}: {e}", n + 1));
        }
    }
    // [battery] keys not given fall back to the top-level values
    if in_bat {
        let b = Times { start: bs.unwrap_or(c.ac.start), then: bt.unwrap_or(c.ac.then), after: ba.unwrap_or(c.ac.after) };
        c.battery = Some(b).filter(|b| *b != c.ac);
    }
    (c, errs)
}

impl Config {
    /// The times that apply on battery.
    pub fn bat(&self) -> Times {
        self.battery.unwrap_or(self.ac)
    }

    /// Whether dismissing the screensaver after `secs` of it should lock:
    /// past lock_after, or at once when it was started by hand (`now`).
    pub fn locks(&self, now: bool, secs: f64) -> bool {
        self.lock_after.is_some_and(|m| now || secs >= m as f64 * 60.0)
    }

    /// The file setup writes: the general settings, then the optional
    /// sections, commented out unless they hold something.
    pub fn render(&self) -> String {
        let mut s = String::from(
            "# glyphwave — edit, then run `glyphwave setup` again to apply.\n\
             # Times are minutes. then_after and lock_after count from the screensaver's start.\n\n",
        );
        s += &format!("start_after = {}\n", self.ac.start);
        s += &format!("then = {}                # lock, screen-off, sleep or none\n", self.ac.then.name());
        s += &format!("then_after = {}\n", self.ac.after);
        let lock = self.lock_after.map_or("none".to_string(), |m| m.to_string());
        s += &format!("lock_after = {lock}          # after this many minutes, waking it lands on the lock screen; 0 always, none never\n");
        s += &format!("on_battery = {}            # no: only when plugged in\n", if self.on_battery { "yes" } else { "no" });
        s += &format!("dim = {}                   # dim the screen while it runs: yes, no or battery (KDE)\n", self.dim.name());
        match &self.banner {
            Some(b) => s += &format!("banner = {b}\n"),
            None => s += "# banner = logo         # logo, name, text:<your words>, or a path to a text file\n",
        }
        s += &format!("fps = {}\n", self.fps.unwrap_or(30.0));
        match &self.ringtone {
            Some(r) => s += &format!("ringtone = {r}\n"),
            None => s += "# ringtone = default     # while a call rings: default, none, or a wav/ogg/flac file\n",
        }
        s += &format!("shortcut = {}      # starts it now; none for no key\n", self.shortcut.as_deref().unwrap_or("none"));

        let opt = |on: bool| if on { "" } else { "# " };
        s += "\n# Battery times, when they should differ from the ones above. KDE and GNOME\n\
              # have one lock / blank timer, which follows the plugged-in times.\n";
        let b = self.bat();
        let on = self.battery.is_some();
        s += &format!("{}[battery]\n", opt(on));
        s += &format!("{}start_after = {}\n", opt(on), b.start);
        s += &format!("{}then = {}\n", opt(on), b.then.name());
        s += &format!("{}then_after = {}\n", opt(on), b.after);

        s += "\n# Which terminal opens the screensaver; empty picks the first installed of\n\
              # kitty foot alacritty ghostty wezterm konsole ptyxis gnome-terminal xterm.\n\
              # konsole_profile names a Konsole profile of your own (e.g. no scrollbar).\n";
        let on = self.terminal.is_some() || self.konsole_profile.is_some();
        s += &format!("{}[terminal]\n", opt(on));
        s += &format!("{}terminal = {}\n", opt(self.terminal.is_some()), self.terminal.as_deref().unwrap_or("konsole"));
        s += &format!(
            "{}konsole_profile = {}\n",
            opt(self.konsole_profile.is_some()),
            self.konsole_profile.as_deref().unwrap_or("MyProfile")
        );

        s += "\n# KDE and GNOME have no settings here: setup edits their own timers\n\
              # (powerdevilrc, kscreenlockerrc; gsettings plus an idle watcher in\n\
              # ~/.config/autostart).\n";

        for (name, val, def) in [
            ("hyprland", &self.hyprland_locker, "hyprlock"),
            ("sway", &self.sway_locker, "swaylock -f"),
            ("x11", &self.x11_locker, "i3lock -c 000000"),
        ] {
            let on = val != def;
            s += &format!("\n# {name}: the locker in the lines setup prints\n{}[{name}]\n{}locker = {val}\n", opt(on), opt(on));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let (c, e) = parse("");
        assert!(e.is_empty());
        assert_eq!(c, Config::default());
    }

    #[test]
    fn top_level_and_comments() {
        let (c, e) = parse(
            "# hi\nstart_after = 7\nthen = sleep   # comment\nthen_after=3\non_battery = no\n\
             banner = /home/me/my art.txt\nfps = 24\nringtone = /home/me/ring.ogg\n",
        );
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(c.ac, Times { start: 7, then: Then::Sleep, after: 3 });
        assert!(!c.on_battery);
        assert_eq!(c.banner.as_deref(), Some("/home/me/my art.txt"));
        assert_eq!(c.fps, Some(24.0));
        assert_eq!(c.ringtone.as_deref(), Some("/home/me/ring.ogg"));
        assert_eq!(c.battery, None);
    }

    #[test]
    fn battery_overrides_fall_back_to_top() {
        let (c, e) = parse("start_after = 5\nthen = lock\nthen_after = 10\n[battery]\nstart_after = 2\n");
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(c.bat(), Times { start: 2, then: Then::Lock, after: 10 });
        // a [battery] section equal to the top is no override at all
        let (c, _) = parse("start_after = 5\n[battery]\nstart_after = 5\n");
        assert_eq!(c.battery, None);
    }

    #[test]
    fn commented_sections_are_ignored() {
        let (c, e) = parse("# [battery]\n# start_after = 1\n# [terminal]\n# terminal = xterm\n");
        assert!(e.is_empty());
        assert_eq!(c.battery, None);
        assert_eq!(c.terminal, None);
    }

    #[test]
    fn sections() {
        let (c, e) = parse(
            "[terminal]\nterminal = konsole\nkonsole_profile = Black\n[sway]\nlocker = waylock\n",
        );
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(c.terminal.as_deref(), Some("konsole"));
        assert_eq!(c.konsole_profile.as_deref(), Some("Black"));
        assert_eq!(c.sway_locker, "waylock");
    }

    #[test]
    fn errors_keep_defaults() {
        let (c, e) = parse("start_after = soon\nthen = shutdown\nwhat = 1\nnonsense\n[gnome]\nx = 1\nfps = fast\n");
        assert_eq!(e.len(), 6, "{e:?}");
        assert!(e[0].starts_with("line 1"));
        assert_eq!(c.ac, Config::default().ac);
        assert_eq!(c.fps, None);
    }

    #[test]
    fn render_round_trips() {
        let c = Config {
            ac: Times { start: 3, then: Then::ScreenOff, after: 4 },
            on_battery: false,
            battery: Some(Times { start: 1, then: Then::Sleep, after: 2 }),
            banner: Some("name".into()),
            fps: Some(20.0),
            ringtone: Some("none".into()),
            terminal: Some("konsole".into()),
            konsole_profile: Some("Glyphwave".into()),
            x11_locker: "slock".into(),
            shortcut: None,
            lock_after: Some(0),
            dim: Dim::Battery,
            ..Config::default()
        };
        let (back, e) = parse(&c.render());
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(back, c);
        let d = Config { fps: Some(30.0), ..Config::default() };
        assert_eq!(parse(&d.render()).0, d);
        let d = Config { lock_after: Some(15), ..d };
        assert_eq!(parse(&d.render()).0, d);
    }

    #[test]
    fn lock_after() {
        let (c, e) = parse("lock_after = 0\n");
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(c.lock_after, Some(0));
        assert_eq!(parse("lock_after = 20\n").0.lock_after, Some(20));
        assert_eq!(parse("lock_after = 5\nlock_after = none\n").0.lock_after, None);
        let (c, e) = parse("lock_after = soon\n[battery]\nlock_after = 1\n");
        assert_eq!(e.len(), 2, "{e:?}");
        assert_eq!(c.lock_after, None);
    }

    #[test]
    fn locks_on_dismiss() {
        let at = |m| Config { lock_after: m, ..Config::default() };
        assert!(!at(None).locks(false, 1e6));
        assert!(!at(None).locks(true, 0.0)); // started by hand, but no locking asked for
        assert!(at(Some(0)).locks(false, 0.0));
        assert!(!at(Some(5)).locks(false, 299.0));
        assert!(at(Some(5)).locks(false, 300.0));
        assert!(at(Some(5)).locks(true, 1.0));
    }
}
