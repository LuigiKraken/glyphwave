//! `glyphwave setup`: asks four questions, writes ~/.config/glyphwave/config
//! and hooks glyphwave into the desktop's own idle timer: KDE's powerdevilrc,
//! GNOME's settings plus a tiny idle watcher, or (Hyprland, sway, X11) the
//! lines to paste. Never root: it only writes to the home folder, lists every
//! change first, and keeps the originals so `--remove` puts them back exactly.

use crate::config::{self, Config, Then, Times};
use crate::launch;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Desktop {
    Kde,
    Gnome,
    Hyprland,
    Sway,
    X11,
}

impl Desktop {
    fn parse(s: &str) -> Option<Desktop> {
        Some(match s.to_ascii_lowercase().as_str() {
            "kde" | "plasma" => Desktop::Kde,
            "gnome" => Desktop::Gnome,
            "hyprland" => Desktop::Hyprland,
            "sway" => Desktop::Sway,
            "x11" => Desktop::X11,
            _ => return None,
        })
    }

    fn detect() -> Option<Desktop> {
        let env = |k| std::env::var(k).unwrap_or_default();
        let cur = env("XDG_CURRENT_DESKTOP");
        for part in cur.split(':') {
            if let Some(d) = Desktop::parse(part) {
                return Some(d);
            }
        }
        if !env("HYPRLAND_INSTANCE_SIGNATURE").is_empty() {
            return Some(Desktop::Hyprland);
        }
        if !env("SWAYSOCK").is_empty() {
            return Some(Desktop::Sway);
        }
        if env("WAYLAND_DISPLAY").is_empty() && !env("DISPLAY").is_empty() {
            return Some(Desktop::X11);
        }
        None
    }

    fn name(self) -> &'static str {
        match self {
            Desktop::Kde => "KDE Plasma",
            Desktop::Gnome => "GNOME",
            Desktop::Hyprland => "Hyprland",
            Desktop::Sway => "sway",
            Desktop::X11 => "X11",
        }
    }
}

// ------------------------------------------------------------------ prompts

fn ask(q: &str, def: &str) -> String {
    print!("{q} [{def}] ");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    if std::io::stdin().lock().read_line(&mut s).unwrap_or(0) == 0 {
        println!();
    }
    let s = s.trim();
    if s.is_empty() { def.to_string() } else { s.to_string() }
}

fn ask_yes(q: &str, def: bool) -> bool {
    loop {
        match ask(q, if def { "Y/n" } else { "y/N" }).to_ascii_lowercase().as_str() {
            "y" | "yes" => return true,
            "n" | "no" => return false,
            "y/n" => return def,
            _ => println!("  yes or no"),
        }
    }
}

fn ask_minutes(q: &str, def: u32) -> u32 {
    loop {
        match ask(q, &def.to_string()).parse::<u32>() {
            Ok(n) if n > 0 => return n,
            _ => println!("  a number of minutes, 1 or more"),
        }
    }
}

fn ask_times(t: Times, what: &str) -> Times {
    let start = ask_minutes(&format!("Start the screensaver after how many idle minutes{what}?"), t.start);
    let then = loop {
        match Then::parse(&ask("Then what: lock, screen-off, sleep or none?", t.then.name())) {
            Some(x) => break x,
            None => println!("  one of lock, screen-off, sleep, none (no shutdown)"),
        }
    };
    let after = if then == Then::Nothing {
        t.after
    } else {
        ask_minutes(&format!("{} how many minutes after the screensaver starts?", then.name()), t.after)
    };
    Times { start, then, after }
}

fn summary(c: &Config) -> String {
    let t = |t: Times| match t.then {
        Then::Nothing => format!("start after {} min", t.start),
        x => format!("start after {} min, {} {} min later", t.start, x.name(), t.after),
    };
    let mut s = t(c.ac);
    match (c.on_battery, c.battery) {
        (false, _) => s += "; only when plugged in",
        (true, None) => s += "; on battery too",
        (true, Some(b)) => s += &format!("; on battery: {}", t(b)),
    }
    s + &format!("; banner {}", c.banner.as_deref().unwrap_or("(default)"))
}

fn questions(mut c: Config) -> Config {
    c.ac = ask_times(c.ac, "");
    loop {
        let def = match (c.on_battery, c.battery) {
            (false, _) => "no",
            (true, None) => "yes",
            (true, Some(_)) => "own",
        };
        match ask("On battery too? yes, no (only plugged in), or own (other times on battery)", def).as_str() {
            "yes" => (c.on_battery, c.battery) = (true, None),
            "no" => c.on_battery = false,
            "own" => {
                c.on_battery = true;
                let b = ask_times(c.bat(), " on battery");
                c.battery = Some(b).filter(|b| *b != c.ac);
            }
            _ => {
                println!("  yes, no or own");
                continue;
            }
        }
        break;
    }
    let b = ask("Banner: logo, name, or the path to a text file?", c.banner.as_deref().unwrap_or("logo"));
    c.banner = Some(b);
    c
}

// ------------------------------------------------------------------ paths

struct Dirs {
    home: PathBuf,
    config: PathBuf,
    data: PathBuf,
}

impl Dirs {
    fn get() -> Dirs {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let data = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
            Some(d) => PathBuf::from(d),
            None => home.join(".local/share"),
        };
        Dirs { config: config::dir(), data, home }
    }

    fn backup(&self) -> PathBuf {
        self.config.join("glyphwave/setup-backup")
    }

    fn show(&self, p: &Path) -> String {
        match p.strip_prefix(&self.home) {
            Ok(r) => format!("~/{}", r.display()),
            Err(_) => p.display().to_string(),
        }
    }
}

// ------------------------------------------------------------------ backups

/// What setup touched, in ~/.config/glyphwave/setup-backup/manifest, so
/// `--remove` can undo it. Only the first change of each thing is recorded:
/// the original stays the original across re-runs.
#[derive(Debug, PartialEq)]
enum Entry {
    /// the file existed; its original is backup/<n>
    Saved(usize, PathBuf),
    /// setup created the file
    Created(PathBuf),
    /// a GNOME setting and its original value; None = was at its default
    Setting(String, String, Option<String>),
}

struct Manifest {
    dir: PathBuf,
    entries: Vec<Entry>,
}

impl Manifest {
    fn load(dir: PathBuf) -> Manifest {
        let text = std::fs::read_to_string(dir.join("manifest")).unwrap_or_default();
        let entries = text
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                Some(match f.as_slice() {
                    ["saved", n, p] => Entry::Saved(n.parse().ok()?, p.into()),
                    ["created", p] => Entry::Created(p.into()),
                    ["setting", s, k, v] => Entry::Setting(s.to_string(), k.to_string(), v.strip_prefix('=').map(str::to_string)),
                    _ => return None,
                })
            })
            .collect();
        Manifest { dir, entries }
    }

    fn save(&self) -> std::io::Result<()> {
        let mut s = String::new();
        for e in &self.entries {
            s += &match e {
                Entry::Saved(n, p) => format!("saved\t{n}\t{}\n", p.display()),
                Entry::Created(p) => format!("created\t{}\n", p.display()),
                Entry::Setting(sc, k, v) => match v {
                    Some(v) => format!("setting\t{sc}\t{k}\t={v}\n"),
                    None => format!("setting\t{sc}\t{k}\t-\n"),
                },
            };
        }
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.dir.join("manifest"), s)
    }

    fn file(&self, p: &Path) -> Option<&Entry> {
        self.entries.iter().find(|e| matches!(e, Entry::Saved(_, q) | Entry::Created(q) if q == p))
    }

    /// The file as it was before setup first touched it.
    fn original(&self, p: &Path) -> Option<Vec<u8>> {
        match self.file(p) {
            Some(Entry::Saved(n, _)) => std::fs::read(self.dir.join(n.to_string())).ok(),
            Some(Entry::Created(_)) => None,
            _ => std::fs::read(p).ok(),
        }
    }

    fn setting(&self, schema: &str, key: &str) -> Option<&Option<String>> {
        self.entries.iter().find_map(|e| match e {
            Entry::Setting(s, k, v) if s == schema && k == key => Some(v),
            _ => None,
        })
    }

    /// Record a file before its first change.
    fn keep_file(&mut self, p: &Path) -> std::io::Result<()> {
        if self.file(p).is_some() {
            return Ok(());
        }
        if p.exists() {
            let n = self.entries.len();
            std::fs::create_dir_all(&self.dir)?;
            std::fs::copy(p, self.dir.join(n.to_string()))?;
            self.entries.push(Entry::Saved(n, p.into()));
        } else {
            self.entries.push(Entry::Created(p.into()));
        }
        self.save()
    }
}

// ------------------------------------------------------------------ KConfig files

/// A `Key=value` in a KConfig group such as `[AC][RunScript]`.
fn ini_get(text: &str, group: &str, key: &str) -> Option<String> {
    let head = format!("[{group}]");
    let mut inside = false;
    for l in text.lines() {
        if l.starts_with('[') {
            inside = l.trim_end() == head;
        } else if inside {
            if let Some(v) = l.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Set (Some) or remove (None) a key, leaving every other byte alone. A new
/// key goes after the group's last line; a new group at the end.
fn ini_set(text: &str, group: &str, key: &str, val: Option<&str>) -> String {
    let head = format!("[{group}]");
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let start = lines.iter().position(|l| l.trim_end() == head);
    let Some(start) = start else {
        let Some(v) = val else { return text.to_string() };
        let mut s = text.to_string();
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        if !s.is_empty() && !s.ends_with("\n\n") {
            s.push('\n');
        }
        return s + &format!("{head}\n{key}={v}\n");
    };
    let end = lines[start + 1..].iter().position(|l| l.starts_with('[')).map_or(lines.len(), |i| start + 1 + i);
    let found = (start + 1..end).find(|&i| lines[i].strip_prefix(key).is_some_and(|r| r.starts_with('=')));
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    match (found, val) {
        (Some(i), Some(v)) => out[i] = format!("{key}={v}\n"),
        (Some(i), None) => {
            out.remove(i);
            // a group left empty goes too, with the blank line before it
            if out[start + 1..end - 1].iter().all(|l| l.trim().is_empty()) {
                let from = if start > 0 && out[start - 1].trim().is_empty() { start - 1 } else { start };
                out.drain(from..end - 1);
            }
        }
        (None, Some(v)) => {
            let mut at = end;
            while at > start + 1 && out[at - 1].trim().is_empty() {
                at -= 1;
            }
            if !out[at - 1].ends_with('\n') {
                out[at - 1].push('\n');
            }
            out.insert(at, format!("{key}={v}\n"));
        }
        (None, None) => {}
    }
    out.concat()
}

// ------------------------------------------------------------------ the plan

enum Change {
    File { path: PathBuf, new: Option<Vec<u8>>, mode: u32 },
    /// a GNOME setting: schema, key, value (None = reset to default)
    Setting(String, String, Option<String>),
}

#[derive(Default)]
struct Plan {
    changes: Vec<Change>,
    warnings: Vec<String>,
    notes: Vec<String>,
    paste: Option<(String, String)>,
}

impl Plan {
    fn file(&mut self, path: PathBuf, new: Vec<u8>, mode: u32) {
        if std::fs::read(&path).ok().as_ref() != Some(&new) {
            self.changes.push(Change::File { path, new: Some(new), mode });
        }
    }
}

fn exe_path(dirs: &Dirs) -> (PathBuf, PathBuf) {
    let exe = std::env::current_exe().unwrap_or_default();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let in_path = exe.parent().is_some_and(|d| std::env::split_paths(&path).any(|p| p == d));
    let bin = if in_path { exe.clone() } else { dirs.home.join(".local/bin/glyphwave") };
    (exe, bin)
}

/// A path in a command line read by KDE (QProcess::splitCommand) or a
/// .desktop Exec line: double quotes when it needs them.
fn cmdline(bin: &Path, args: &str) -> String {
    let b = bin.display().to_string();
    if b.contains([' ', '"', '\'', '\\']) {
        format!("\"{}\" {args}", b.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("{b} {args}")
    }
}

const KDE_OWNED: [(&str, &str); 6] = [
    ("RunScript", "IdleTimeoutCommand"),
    ("RunScript", "RunScriptIdleTimeoutSec"),
    ("Display", "TurnOffDisplayWhenIdle"),
    ("Display", "TurnOffDisplayIdleTimeoutSec"),
    ("SuspendAndShutdown", "AutoSuspendAction"),
    ("SuspendAndShutdown", "AutoSuspendIdleTimeoutSec"),
];

/// powerdevilrc's AutoSuspendAction for sleep (PowerButtonAction::Sleep)
const KDE_SLEEP: &str = "1";

fn text_of(b: Option<Vec<u8>>) -> String {
    b.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

/// Our keys back to their pre-setup values, so a re-run with other answers
/// leaves nothing behind.
fn revert(cur: &str, orig: &str, keys: &[(String, String)]) -> String {
    keys.iter().fold(cur.to_string(), |t, (g, k)| ini_set(&t, g, k, ini_get(orig, g, k).as_deref()))
}

fn plan_kde(p: &mut Plan, m: &Manifest, dirs: &Dirs, cfg: &mut Config, bin: &Path, ask_profile: bool) {
    let rc = dirs.config.join("powerdevilrc");
    let orig = text_of(m.original(&rc));
    let cur = text_of(std::fs::read(&rc).ok());
    let owned: Vec<(String, String)> = ["AC", "Battery"]
        .iter()
        .flat_map(|prof| KDE_OWNED.iter().map(move |(g, k)| (format!("{prof}][{g}"), k.to_string())))
        .collect();
    let mut t = revert(&cur, &orig, &owned);
    let mut profiles = vec![("AC", cfg.ac)];
    if cfg.on_battery {
        profiles.push(("Battery", cfg.bat()));
    }
    let launch = cmdline(bin, "launch");
    let mut lock_at = None::<u32>;
    for &(prof, tm) in &profiles {
        let g = |s: &str| format!("{prof}][{s}");
        let later = ((tm.start + tm.after) * 60).to_string();
        t = ini_set(&t, &g("RunScript"), "IdleTimeoutCommand", Some(&launch));
        t = ini_set(&t, &g("RunScript"), "RunScriptIdleTimeoutSec", Some(&(tm.start * 60).to_string()));
        match tm.then {
            Then::ScreenOff => {
                t = ini_set(&t, &g("Display"), "TurnOffDisplayWhenIdle", Some("true"));
                t = ini_set(&t, &g("Display"), "TurnOffDisplayIdleTimeoutSec", Some(&later));
            }
            Then::Sleep => {
                t = ini_set(&t, &g("SuspendAndShutdown"), "AutoSuspendAction", Some(KDE_SLEEP));
                t = ini_set(&t, &g("SuspendAndShutdown"), "AutoSuspendIdleTimeoutSec", Some(&later));
            }
            Then::Lock => {
                let at = tm.start + tm.after;
                if lock_at.is_some_and(|l| l != at) {
                    p.notes.push("KDE has one lock timer for both power states; the plugged-in time is used.".into());
                }
                lock_at.get_or_insert(at);
            }
            Then::Nothing => {}
        }
        // Timers that would fire before (or with) the screensaver. Plasma 6's
        // defaults for a laptop/desktop apply when a key isn't in the file.
        let get = |grp: &str, k: &str| ini_get(&t, &g(grp), k);
        let on = |grp: &str, k: &str| get(grp, k).is_none_or(|v| v == "true");
        let secs = |grp: &str, k: &str, ac: u32, bat: u32| {
            get(grp, k).and_then(|v| v.parse().ok()).unwrap_or(if prof == "AC" { ac } else { bat })
        };
        let early = |s: u32| s <= tm.start * 60;
        let power = if prof == "AC" { "plugged in" } else { "on battery" };
        let dim = secs("Display", "DimDisplayIdleTimeoutSec", 300, 120);
        if on("Display", "DimDisplayWhenIdle") && early(dim) {
            p.warnings.push(format!(
                "KDE dims the screen after {} {power}, before the screensaver starts \
                 (System Settings > Power Management > Dim automatically).",
                min_s(dim)
            ));
        }
        let off = secs("Display", "TurnOffDisplayIdleTimeoutSec", 600, 300);
        if on("Display", "TurnOffDisplayWhenIdle") && early(off) {
            p.warnings.push(format!(
                "KDE turns the screen off after {} {power}, before the screensaver starts \
                 (Power Management > Turn off screen), or choose then = screen-off.",
                min_s(off)
            ));
        }
        let sus = secs("SuspendAndShutdown", "AutoSuspendIdleTimeoutSec", 900, 600);
        if get("SuspendAndShutdown", "AutoSuspendAction").is_none_or(|v| v != "0") && early(sus) {
            p.warnings.push(format!(
                "KDE sleeps after {} {power}, before the screensaver starts \
                 (Power Management > When inactive), or choose then = sleep.",
                min_s(sus)
            ));
        }
    }
    if cur.as_bytes() != t.as_bytes() {
        let mode = if rc.exists() { file_mode(&rc) } else { 0o600 };
        p.changes.push(Change::File { path: rc, new: Some(t.into_bytes()), mode });
    }

    // the lock timer is kscreenlocker's, in minutes, for both power states
    let lrc = dirs.config.join("kscreenlockerrc");
    let orig = text_of(m.original(&lrc));
    let cur = text_of(std::fs::read(&lrc).ok());
    let owned = [("Daemon".to_string(), "Autolock".to_string()), ("Daemon".to_string(), "Timeout".to_string())];
    let mut t = revert(&cur, &orig, &owned);
    if let Some(at) = lock_at {
        t = ini_set(&t, "Daemon", "Autolock", Some("true"));
        t = ini_set(&t, "Daemon", "Timeout", Some(&at.to_string()));
    } else {
        let first = profiles.iter().map(|(_, t)| t.start).min().unwrap_or(cfg.ac.start);
        let auto = ini_get(&t, "Daemon", "Autolock").is_none_or(|v| v == "true");
        let after: u32 = ini_get(&t, "Daemon", "Timeout").and_then(|v| v.parse().ok()).unwrap_or(5);
        if auto && after <= first {
            p.warnings.push(format!(
                "KDE locks the screen after {after} min, before the screensaver starts, and that ends it \
                 (System Settings > Screen Locking), or choose then = lock."
            ));
        }
    }
    if cur.as_bytes() != t.as_bytes() {
        let mode = if lrc.exists() { file_mode(&lrc) } else { 0o600 };
        p.changes.push(Change::File { path: lrc, new: Some(t.into_bytes()), mode });
    }

    // a black, borderless Konsole profile, when Konsole is the terminal
    let k = dirs.data.join("konsole");
    let files = [(k.join("Glyphwave.profile"), KONSOLE_PROFILE), (k.join("GlyphwaveBlack.colorscheme"), KONSOLE_COLORS)];
    let konsole = launch::terminal(cfg).as_deref() == Some("konsole");
    if konsole && ask_profile {
        cfg.kde_black_profile = ask_yes("Konsole opens the screensaver. Add a black, borderless Konsole profile for it?", cfg.kde_black_profile);
    }
    if konsole && cfg.kde_black_profile {
        for (f, text) in files {
            p.file(f, text.as_bytes().to_vec(), 0o644);
        }
        cfg.konsole_profile.get_or_insert("Glyphwave".into());
    } else {
        // answered no this time: take back the profile an earlier run added
        for (f, _) in files {
            if matches!(m.file(&f), Some(Entry::Created(_))) && f.exists() {
                p.changes.push(Change::File { path: f, new: None, mode: 0 });
            }
        }
        if cfg.konsole_profile.as_deref() == Some("Glyphwave") {
            cfg.konsole_profile = None;
        }
    }
}

fn min_s(s: u32) -> String {
    if s % 60 == 0 { format!("{} min", s / 60) } else { format!("{s} s") }
}

fn file_mode(p: &Path) -> u32 {
    std::fs::metadata(p).map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o7777).unwrap_or(0o644)
}

const KONSOLE_PROFILE: &str = "\
[Appearance]
ColorScheme=GlyphwaveBlack

[General]
Name=Glyphwave
Parent=FALLBACK/
ShowTerminalSizeHint=false
TerminalMargin=0

[Scrolling]
HistoryMode=0
ScrollBarPosition=2
";

const KONSOLE_COLORS: &str = "\
[General]
Description=Glyphwave Black
Opacity=1
Blur=false

[Background]
Color=0,0,0

[BackgroundIntense]
Color=0,0,0

[BackgroundFaint]
Color=0,0,0

[Foreground]
Color=252,252,252

[ForegroundIntense]
Color=255,255,255

[ForegroundFaint]
Color=239,240,241

[Color0]
Color=35,38,39

[Color0Intense]
Color=127,140,141

[Color0Faint]
Color=49,54,59

[Color1]
Color=237,21,21

[Color1Intense]
Color=192,57,43

[Color1Faint]
Color=120,50,40

[Color2]
Color=17,209,22

[Color2Intense]
Color=28,220,154

[Color2Faint]
Color=23,162,98

[Color3]
Color=246,116,0

[Color3Intense]
Color=253,188,75

[Color3Faint]
Color=182,86,25

[Color4]
Color=29,153,243

[Color4Intense]
Color=61,174,233

[Color4Faint]
Color=27,102,143

[Color5]
Color=155,89,182

[Color5Intense]
Color=142,68,173

[Color5Faint]
Color=97,74,115

[Color6]
Color=26,188,156

[Color6Intense]
Color=22,160,133

[Color6Faint]
Color=24,108,96

[Color7]
Color=252,252,252

[Color7Intense]
Color=255,255,255

[Color7Faint]
Color=99,104,109
";

// GNOME keys setup may change, with the schema defaults (gsettings-desktop-schemas,
// gnome-settings-daemon) that apply while a key is unset
const GS_SESSION: &str = "org.gnome.desktop.session";
const GS_LOCK: &str = "org.gnome.desktop.screensaver";
const GS_POWER: &str = "org.gnome.settings-daemon.plugins.power";
const GNOME_OWNED: [(&str, &str, &str); 7] = [
    (GS_SESSION, "idle-delay", "uint32 300"),
    (GS_LOCK, "lock-enabled", "true"),
    (GS_LOCK, "lock-delay", "uint32 0"),
    (GS_POWER, "sleep-inactive-ac-timeout", "900"),
    (GS_POWER, "sleep-inactive-ac-type", "'suspend'"),
    (GS_POWER, "sleep-inactive-battery-timeout", "900"),
    (GS_POWER, "sleep-inactive-battery-type", "'suspend'"),
];

fn run_out(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// The value the user set, None while it's at its default (dconf has no entry).
fn gs_user(schema: &str, key: &str) -> Option<String> {
    let path = format!("/{}/{key}", schema.replace('.', "/"));
    match run_out("dconf", &["read", &path]) {
        Some(v) if v.is_empty() => None,
        Some(v) => Some(v),
        None => gs_get(schema, key), // no dconf tool: treat the value as set
    }
}

/// The value in effect, default or not.
fn gs_get(schema: &str, key: &str) -> Option<String> {
    run_out("gsettings", &["get", schema, key])
}

fn gs_num(v: &str) -> u32 {
    v.rsplit(' ').next().and_then(|n| n.parse().ok()).unwrap_or(0)
}

fn plan_gnome(p: &mut Plan, m: &Manifest, dirs: &Dirs, cfg: &Config, bin: &Path) {
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=glyphwave idle watcher\n\
         Comment=Opens the glyphwave screensaver when the session is idle\n\
         Exec={}\nOnlyShowIn=GNOME;\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
        cmdline(bin, "idle-watch")
    );
    p.file(dirs.config.join("autostart/glyphwave.desktop"), entry.into_bytes(), 0o644);

    // start from the originals, then set what the answers need
    let mut want: Vec<(&str, &str, Option<String>)> = GNOME_OWNED
        .iter()
        .map(|&(s, k, _)| (s, k, m.setting(s, k).cloned().unwrap_or_else(|| gs_user(s, k))))
        .collect();
    let mut set = |s: &str, k: &str, v: String| {
        if let Some(w) = want.iter_mut().find(|w| w.0 == s && w.1 == k) {
            w.2 = Some(v);
        }
    };
    let mut blank_at = None::<u32>;
    let mut powers = vec![("ac", cfg.ac)];
    if cfg.on_battery {
        powers.push(("battery", cfg.bat()));
    }
    for &(pw, tm) in &powers {
        let later = (tm.start + tm.after) * 60;
        match tm.then {
            Then::Lock | Then::ScreenOff => {
                if blank_at.is_some_and(|b| b != later) {
                    p.notes.push("GNOME has one blank/lock timer for both power states; the plugged-in time is used.".into());
                }
                let b = *blank_at.get_or_insert(later);
                set(GS_SESSION, "idle-delay", format!("uint32 {b}"));
                if tm.then == Then::Lock {
                    set(GS_LOCK, "lock-enabled", "true".into());
                    set(GS_LOCK, "lock-delay", "uint32 0".into());
                }
            }
            Then::Sleep => {
                set(GS_POWER, &format!("sleep-inactive-{pw}-timeout"), later.to_string());
                set(GS_POWER, &format!("sleep-inactive-{pw}-type"), "'suspend'".into());
            }
            Then::Nothing => {}
        }
    }
    if cfg.ac.then == Then::ScreenOff {
        p.notes.push("GNOME also locks when it blanks the screen, if Screen Lock is on in Settings > Privacy.".into());
    }
    for (s, k, v) in &want {
        let user = gs_user(s, k);
        // nothing to write when the default already says the same
        let same = user.is_none() && v.is_some() && *v == gs_get(s, k);
        if *v != user && !same {
            p.changes.push(Change::Setting(s.to_string(), k.to_string(), v.clone()));
        }
    }
    // what will be in effect afterwards
    let eff = |s: &str, k: &str| {
        let w = want.iter().find(|w| w.0 == s && w.1 == k)?;
        w.2.clone().or_else(|| GNOME_OWNED.iter().find(|o| o.0 == s && o.1 == k).map(|o| o.2.to_string()))
    };
    let first = powers.iter().map(|(_, t)| t.start * 60).min().unwrap_or(300);
    if let Some(d) = eff(GS_SESSION, "idle-delay").map(|v| gs_num(&v)).filter(|&d| d > 0 && d <= first) {
        p.warnings.push(format!(
            "GNOME blanks the screen after {}, before the screensaver starts, and that ends it \
             (Settings > Power > Screen Blank), or choose then = lock or screen-off.",
            min_s(d)
        ));
    }
    for &(pw, tm) in &powers {
        let ty = eff(GS_POWER, &format!("sleep-inactive-{pw}-type")).unwrap_or_default();
        let t = eff(GS_POWER, &format!("sleep-inactive-{pw}-timeout")).map_or(0, |v| gs_num(&v));
        if ty != "'nothing'" && t > 0 && t <= tm.start * 60 {
            p.warnings.push(format!(
                "GNOME suspends after {} {}, before the screensaver starts \
                 (Settings > Power > Automatic Suspend).",
                min_s(t),
                if pw == "ac" { "plugged in" } else { "on battery" }
            ));
        }
    }
    p.notes.push("The idle watcher starts now and at each login (~/.config/autostart).".into());
}

/// Hyprland, sway and X11 read their idle setup from files people write by
/// hand; setup prints the lines rather than editing them.
fn plan_paste(p: &mut Plan, d: Desktop, cfg: &Config, bin: &Path) {
    let b = bin.display();
    let two = cfg.on_battery && cfg.battery.is_some_and(|x| x.start != cfg.ac.start);
    let starts: Vec<(u32, String)> = if two {
        vec![(cfg.ac.start, format!("{b} launch --on-ac")), (cfg.bat().start, format!("{b} launch --on-battery"))]
    } else {
        vec![(cfg.ac.start, format!("{b} launch"))]
    };
    if cfg.on_battery && cfg.battery.is_some_and(|x| x.then != cfg.ac.then || x.after != cfg.ac.after) {
        p.notes.push(format!("{} can't tell power states apart, so the later step uses the plugged-in time.", d.name()));
    }
    let tm = cfg.ac;
    let later = tm.start + tm.after;
    let (file, text) = match d {
        Desktop::Hyprland => {
            let l = &cfg.hyprland_locker;
            let l0 = l.split_whitespace().next().unwrap_or("hyprlock");
            let mut s = format!(
                "general {{\n    # close the screensaver first: {l0} isn't on the bus\n    \
                 lock_cmd = {b} launch --stop; pidof {l0} || {l}\n    before_sleep_cmd = loginctl lock-session\n}}\n"
            );
            for (m, c) in &starts {
                s += &format!("\nlistener {{\n    timeout = {}\n    on-timeout = {c}\n}}\n", m * 60);
            }
            let then = match tm.then {
                Then::Lock => Some(("loginctl lock-session", "")),
                Then::ScreenOff => Some(("hyprctl dispatch dpms off", "hyprctl dispatch dpms on")),
                Then::Sleep => Some(("systemctl suspend", "")),
                Then::Nothing => None,
            };
            if let Some((on, back)) = then {
                s += &format!("\nlistener {{\n    timeout = {}\n    on-timeout = {on}\n", later * 60);
                if !back.is_empty() {
                    s += &format!("    on-resume = {back}\n");
                }
                s += "}\n";
            }
            s += "\n# and in hyprland.conf, for terminals that can't start fullscreen (wezterm):\n\
                  # windowrulev2 = fullscreen, class:^(glyphwave)$\n";
            ("~/.config/hypr/hypridle.conf", s)
        }
        Desktop::Sway => {
            let l = &cfg.sway_locker;
            let mut s = "exec swayidle -w \\\n".to_string();
            for (m, c) in &starts {
                s += &format!("    timeout {} '{c}' \\\n", m * 60);
            }
            match tm.then {
                Then::Lock => s += &format!("    timeout {} '{b} launch --stop; {l}' \\\n", later * 60),
                Then::ScreenOff => {
                    s += &format!("    timeout {} 'swaymsg \"output * power off\"' resume 'swaymsg \"output * power on\"' \\\n", later * 60)
                }
                Then::Sleep => s += &format!("    timeout {} 'systemctl suspend' \\\n", later * 60),
                Then::Nothing => {}
            }
            s += &format!("    before-sleep '{b} launch --stop; {l}'\n\n");
            s += "for_window [app_id=\"glyphwave\"] fullscreen enable\nfor_window [class=\"glyphwave\"] fullscreen enable\n";
            ("~/.config/sway/config", s)
        }
        _ => {
            // xidlehook's timers count from the one before
            let mut starts = starts;
            starts.sort();
            let mut s = "xidlehook \\\n".to_string();
            let mut prev = 0;
            for (m, c) in &starts {
                s += &format!("    --timer {} '{c}' '' \\\n", (m - prev).max(1) * 60);
                prev = *m;
            }
            let last = starts.iter().map(|(m, _)| *m).max().unwrap_or(tm.start);
            let rest = later.saturating_sub(last).max(1) * 60;
            match tm.then {
                Then::Lock => s += &format!("    --timer {rest} '{b} launch --stop; {}' ''\n", cfg.x11_locker),
                Then::ScreenOff => s += &format!("    --timer {rest} 'xset dpms force off' ''\n"),
                Then::Sleep => s += &format!("    --timer {rest} 'systemctl suspend' ''\n"),
                Then::Nothing => s = s.trim_end_matches(" \\\n").to_string() + "\n",
            }
            ("your session autostart (e.g. ~/.xinitrc or your window manager's config)", s)
        }
    };
    p.paste = Some((file.to_string(), text));
}

// ------------------------------------------------------------------ show, apply, undo

fn describe(c: &Change, dirs: &Dirs) -> String {
    match c {
        Change::File { path, new, .. } => {
            let old = std::fs::read(path).ok();
            let verb = match (&old, new) {
                (None, _) => "create",
                (Some(_), None) => "delete",
                (Some(_), Some(_)) => "change",
            };
            let mut s = format!("  {verb} {}", dirs.show(path));
            let (Some(new), Some(old)) = (new, &old) else {
                if let Some(n) = new.as_ref().filter(|n| std::str::from_utf8(n).is_err()) {
                    s += &format!("  (glyphwave itself, {} KB)", n.len() / 1024);
                }
                return s;
            };
            let (o, n) = (String::from_utf8_lossy(old), String::from_utf8_lossy(new));
            if std::str::from_utf8(new).is_err() {
                return s + "  (glyphwave itself, a newer copy)";
            }
            // the settings lines, each with its [group], that differ
            let keyed = |t: &str| {
                let mut group = String::new();
                let mut v = Vec::new();
                for l in t.lines().map(str::trim_end) {
                    if l.starts_with('[') {
                        group = format!("{l} ");
                    } else if !l.is_empty() && !l.starts_with('#') {
                        v.push(format!("{group}{l}"));
                    }
                }
                v
            };
            let (ol, nl) = (keyed(&o), keyed(&n));
            for l in nl.iter().filter(|l| !ol.contains(l)) {
                s += &format!("\n      + {l}");
            }
            for l in ol.iter().filter(|l| !nl.contains(l)) {
                s += &format!("\n      - {l}");
            }
            s
        }
        Change::Setting(sch, k, v) => match v {
            Some(v) => format!("  set GNOME setting {sch} {k} to {v} (was {})", gs_get(sch, k).unwrap_or("?".into())),
            None => format!("  reset GNOME setting {sch} {k} to its default"),
        },
    }
}

fn apply(c: &Change, m: &mut Manifest) -> std::io::Result<()> {
    match c {
        Change::File { path, new, mode } => {
            m.keep_file(path)?;
            match new {
                Some(b) => write_file(path, b, *mode),
                None => remove_file(path),
            }
        }
        Change::Setting(s, k, v) => {
            if m.setting(s, k).is_none() {
                m.entries.push(Entry::Setting(s.clone(), k.clone(), gs_user(s, k)));
                m.save()?;
            }
            gsettings(s, k, v.as_deref())
        }
    }
}

fn write_file(p: &Path, b: &[u8], mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    // via a temporary file, so the running binary or a half-written rc file never shows
    let tmp = p.with_extension("glyphwave-tmp");
    std::fs::write(&tmp, b)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(&tmp, p)
}

fn remove_file(p: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(p) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn gsettings(s: &str, k: &str, v: Option<&str>) -> std::io::Result<()> {
    let st = match v {
        Some(v) => Command::new("gsettings").args(["set", s, k, v]).status()?,
        None => Command::new("gsettings").args(["reset", s, k]).status()?,
    };
    if st.success() { Ok(()) } else { Err(std::io::Error::other(format!("gsettings failed for {s} {k}"))) }
}

fn kde_reload() {
    let Ok(c) = zbus::blocking::Connection::session() else {
        println!("KDE didn't answer; the settings apply at the next login.");
        return;
    };
    let pm = "org.kde.Solid.PowerManagement";
    let ok = c.call_method(Some(pm), "/org/kde/Solid/PowerManagement", Some(pm), "reparseConfiguration", &()).is_ok()
        && c.call_method(Some(pm), "/org/kde/Solid/PowerManagement", Some(pm), "refreshStatus", &()).is_ok();
    let ok2 = c
        .call_method(Some("org.freedesktop.ScreenSaver"), "/ScreenSaver", Some("org.kde.screensaver"), "configure", &())
        .is_ok();
    if ok && ok2 {
        println!("KDE has reloaded its power and lock settings.");
    } else {
        println!("KDE didn't answer; the settings apply at the next login.");
    }
}

fn start_watcher(bin: &Path) {
    use std::os::unix::process::CommandExt;
    launch::stop("idle-watch"); // a re-run picks up the new times
    let _ = Command::new(bin)
        .arg("idle-watch")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
}

/// `glyphwave setup [--remove] [--dry-run] [--desktop NAME]`
pub fn run(args: &[String]) -> i32 {
    let mut remove = false;
    let mut dry = false;
    let mut desktop = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--remove" => remove = true,
            "--dry-run" => dry = true,
            "--desktop" => match it.next().and_then(|d| Desktop::parse(d)) {
                Some(d) => desktop = Some(d),
                None => {
                    eprintln!("glyphwave: --desktop takes kde, gnome, hyprland, sway or x11");
                    return 2;
                }
            },
            other => {
                eprintln!("glyphwave: setup doesn't know {other}; it takes --remove, --dry-run, --desktop NAME");
                return 2;
            }
        }
    }
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("glyphwave: run setup as yourself, not root; it only changes your home folder.");
        return 1;
    }
    let dirs = Dirs::get();
    if dirs.home.as_os_str().is_empty() {
        eprintln!("glyphwave: HOME isn't set");
        return 1;
    }
    if remove { undo(&dirs, dry) } else { setup(&dirs, desktop, dry) }
}

fn setup(dirs: &Dirs, desktop: Option<Desktop>, dry: bool) -> i32 {
    let Some(d) = desktop.or_else(Desktop::detect) else {
        eprintln!("glyphwave: can't tell which desktop this is; say it with --desktop kde|gnome|hyprland|sway|x11");
        return 1;
    };
    println!("Desktop: {}", d.name());
    let mut m = Manifest::load(dirs.backup());
    let cfg_path = config::path();
    let (mut cfg, asked) = match config::load() {
        Some(c) => {
            println!("Settings from {}: {}", dirs.show(&cfg_path), summary(&c));
            if ask_yes("Use these?", true) { (c, false) } else { (questions(c), true) }
        }
        None => (questions(Config::default()), true),
    };

    let mut p = Plan::default();
    let (exe, bin) = exe_path(dirs);
    if exe != bin {
        match std::fs::read(&exe) {
            Ok(b) => p.file(bin.clone(), b, 0o755),
            Err(e) => {
                eprintln!("glyphwave: can't read {}: {e}", exe.display());
                return 1;
            }
        }
    }
    match d {
        Desktop::Kde => plan_kde(&mut p, &m, dirs, &mut cfg, &bin, asked),
        Desktop::Gnome => plan_gnome(&mut p, &m, dirs, &cfg, &bin),
        _ => plan_paste(&mut p, d, &cfg, &bin),
    }
    if [Some(cfg.ac), cfg.on_battery.then(|| cfg.bat())].iter().flatten().any(|t| t.then == Then::Sleep) {
        p.notes.push(
            "While music plays, most players and browsers hold off sleep, so the computer \
             sleeps once the music has stopped."
                .into(),
        );
    }
    // the config goes first in the list
    let rendered = cfg.render().into_bytes();
    if std::fs::read(&cfg_path).ok().as_ref() != Some(&rendered) {
        p.changes.insert(0, Change::File { path: cfg_path, new: Some(rendered), mode: 0o644 });
    }

    println!();
    for w in &p.warnings {
        println!("Warning: {w}");
    }
    if !p.warnings.is_empty() {
        println!();
    }
    if p.changes.is_empty() {
        println!("Nothing to change.");
    } else {
        println!("Setup will:");
        for c in &p.changes {
            println!("{}", describe(c, dirs));
        }
        println!("Originals are kept in {} for `glyphwave setup --remove`.", dirs.show(&dirs.backup()));
        if dry {
            println!("\n(dry run: nothing written)");
        } else if !ask_yes("Go ahead?", false) {
            println!("Nothing written.");
            return 1;
        }
    }
    if !dry {
        for c in &p.changes {
            if let Err(e) = apply(c, &mut m) {
                eprintln!("glyphwave: {e}; `glyphwave setup --remove` undoes what was done so far");
                return 1;
            }
        }
        match d {
            Desktop::Kde if !p.changes.is_empty() => kde_reload(),
            Desktop::Gnome => start_watcher(&bin),
            _ => {}
        }
    }
    if let Some((file, text)) = &p.paste {
        println!("\n{} needs these lines in {file}:\n\n{text}", d.name());
    }
    for n in &p.notes {
        println!("{n}");
    }
    if !launch::installed("parec") {
        println!("For the music visuals, install parec (pulseaudio-utils, or libpulse on Arch).");
    }
    if !launch::installed("fastfetch") && !launch::installed("neofetch") && cfg.banner.as_deref() == Some("logo") {
        println!("Optional: with fastfetch installed, the banner shows your system's logo.");
    }
    0
}

fn undo(dirs: &Dirs, dry: bool) -> i32 {
    let m = Manifest::load(dirs.backup());
    if m.entries.is_empty() {
        println!("Nothing to remove: no setup backup in {}.", dirs.show(&dirs.backup()));
        return 0;
    }
    println!("Remove will:");
    for e in &m.entries {
        match e {
            Entry::Saved(_, p) => println!("  restore {}", dirs.show(p)),
            Entry::Created(p) => println!("  delete {}", dirs.show(p)),
            Entry::Setting(s, k, Some(v)) => println!("  set GNOME setting {s} {k} back to {v}"),
            Entry::Setting(s, k, None) => println!("  reset GNOME setting {s} {k} to its default"),
        }
    }
    println!("  delete {}", dirs.show(&dirs.backup()));
    if dry {
        println!("\n(dry run: nothing changed)");
        return 0;
    }
    if !ask_yes("Go ahead?", false) {
        println!("Nothing changed.");
        return 1;
    }
    let mut failed = false;
    for e in m.entries.iter().rev() {
        let r = match e {
            Entry::Saved(n, p) => std::fs::copy(m.dir.join(n.to_string()), p).map(|_| ()),
            Entry::Created(p) => {
                if p.ends_with("autostart/glyphwave.desktop") {
                    launch::stop("idle-watch");
                }
                remove_file(p)
            }
            Entry::Setting(s, k, v) => gsettings(s, k, v.as_deref()),
        };
        if let Err(e) = r {
            eprintln!("glyphwave: {e}");
            failed = true;
        }
    }
    if failed {
        eprintln!("glyphwave: kept {} so you can retry", dirs.show(&dirs.backup()));
        return 1;
    }
    let _ = std::fs::remove_dir_all(&m.dir);
    let _ = std::fs::remove_dir(dirs.config.join("glyphwave")); // only if empty
    if m.entries.iter().any(|e| matches!(e, Entry::Saved(_, p) | Entry::Created(p) if p.ends_with("powerdevilrc") || p.ends_with("kscreenlockerrc"))) {
        kde_reload();
    }
    println!("Done.");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const RC: &str = "[AC][Display]\nDimDisplayWhenIdle=false\n\n[AC][RunScript]\nIdleTimeoutCommand=/old\n\
                      RunScriptIdleTimeoutSec=300\n\n[Battery][Display]\nDimDisplayIdleTimeoutSec=300\n";

    #[test]
    fn ini_get_set() {
        assert_eq!(ini_get(RC, "AC][RunScript", "IdleTimeoutCommand").as_deref(), Some("/old"));
        assert_eq!(ini_get(RC, "Battery][RunScript", "IdleTimeoutCommand"), None);
        let t = ini_set(RC, "AC][RunScript", "IdleTimeoutCommand", Some("/new launch"));
        assert_eq!(t, RC.replace("=/old", "=/new launch"));
        // a new key lands at the group's end, before the blank line
        let t = ini_set(RC, "AC][Display", "TurnOffDisplayWhenIdle", Some("true"));
        assert!(t.starts_with("[AC][Display]\nDimDisplayWhenIdle=false\nTurnOffDisplayWhenIdle=true\n\n[AC][RunScript]"));
        // a new group at the file's end
        let t = ini_set(RC, "Battery][RunScript", "RunScriptIdleTimeoutSec", Some("120"));
        assert_eq!(t, format!("{RC}\n[Battery][RunScript]\nRunScriptIdleTimeoutSec=120\n"));
        // set then remove gives back the original bytes
        let back = ini_set(&ini_set(RC, "AC][Display", "X", Some("1")), "AC][Display", "X", None);
        assert_eq!(back, RC);
        assert_eq!(ini_set("", "Daemon", "Timeout", Some("15")), "[Daemon]\nTimeout=15\n");
        // a key that only shares a prefix is a different key
        assert_eq!(ini_get("[A]\nTimeoutSec=1\n", "A", "Timeout"), None);
    }

    #[test]
    fn revert_restores_original_keys() {
        let ours = ini_set(RC, "AC][RunScript", "IdleTimeoutCommand", Some("/glyphwave launch"));
        let ours = ini_set(&ours, "Battery][RunScript", "IdleTimeoutCommand", Some("/glyphwave launch"));
        let keys = [
            ("AC][RunScript".to_string(), "IdleTimeoutCommand".to_string()),
            ("Battery][RunScript".to_string(), "IdleTimeoutCommand".to_string()),
        ];
        let back = revert(&ours, RC, &keys);
        assert_eq!(back, RC);
    }

    #[test]
    fn cmdline_quotes_only_when_needed() {
        assert_eq!(cmdline(Path::new("/home/a/.local/bin/glyphwave"), "launch"), "/home/a/.local/bin/glyphwave launch");
        assert_eq!(cmdline(Path::new("/home/a b/glyphwave"), "launch"), "\"/home/a b/glyphwave\" launch");
    }
}
