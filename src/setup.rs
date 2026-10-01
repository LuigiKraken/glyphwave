//! `glyphwave setup`: asks five questions, writes ~/.config/glyphwave/config
//! and hooks glyphwave into the desktop's own idle timer: KDE's powerdevilrc,
//! GNOME's settings plus a tiny idle watcher, or (Hyprland, sway, X11) the
//! lines to paste; plus an app-menu entry and a key that start it now. Never
//! root: it only writes to the home folder, lists every change first, and
//! keeps the originals so `--remove` puts them back exactly.

use crate::config::{self, Config, Then, Times};
use crate::launch;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, PartialEq)]
pub enum Desktop {
    Kde,
    Gnome,
    Hyprland,
    Sway,
    X11,
}

const DESKTOPS: [(&str, Desktop, &str); 5] = [
    ("kde", Desktop::Kde, "KDE Plasma"),
    ("gnome", Desktop::Gnome, "GNOME"),
    ("hyprland", Desktop::Hyprland, "Hyprland"),
    ("sway", Desktop::Sway, "sway"),
    ("x11", Desktop::X11, "X11"),
];

fn parse_desktop(s: &str) -> Option<Desktop> {
    DESKTOPS.iter().find(|d| d.0 == s.to_ascii_lowercase()).map(|d| d.1)
}

fn desktop_name(d: Desktop) -> &'static str {
    DESKTOPS.iter().find(|x| x.1 == d).map_or("", |x| x.2)
}

pub fn detect() -> Option<Desktop> {
    let env = |k| std::env::var(k).unwrap_or_default();
    if let Some(d) = env("XDG_CURRENT_DESKTOP").split(':').find_map(parse_desktop) {
        return Some(d);
    }
    if env("WAYLAND_DISPLAY").is_empty() && !env("DISPLAY").is_empty() {
        return Some(Desktop::X11);
    }
    None
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

/// Ask until `parse` accepts the answer.
fn ask_for<T>(q: &str, def: &str, hint: &str, parse: impl Fn(&str) -> Option<T>) -> T {
    loop {
        if let Some(v) = parse(&ask(q, def).to_lowercase()) {
            return v;
        }
        println!("  {hint}");
    }
}

fn ask_yes(q: &str) -> bool {
    ask_for(q, "y/N", "yes or no", |a| match a {
        "y" | "yes" => Some(true),
        "n" | "no" | "y/n" => Some(false),
        _ => None,
    })
}

fn ask_times(t: Times, what: &str) -> Times {
    let mins = |a: &str| a.parse::<u32>().ok().filter(|&n| n > 0);
    let start = ask_for(&format!("Start after how many idle minutes{what}?"), &t.start.to_string(), "a number, 1 or more", mins);
    let then = ask_for("Then: lock, screen-off, sleep or none?", t.then.name(), "lock, screen-off, sleep or none", Then::parse);
    let after = match then {
        Then::Nothing => t.after,
        _ => ask_for("How many minutes after the screensaver starts?", &t.after.to_string(), "a number, 1 or more", mins),
    };
    Times { start, then, after }
}

fn summary(c: &Config) -> String {
    let t = |t: Times| format!("start after {} min, then {} {} min later", t.start, t.then.name(), t.after);
    let bat = match (c.on_battery, c.battery) {
        (false, _) => "only plugged in".to_string(),
        (true, None) => "on battery too".to_string(),
        (true, Some(b)) => format!("on battery {}", t(b)),
    };
    let lock = match c.lock_after {
        None => "waking it never locks".to_string(),
        Some(0) => "waking it always locks".to_string(),
        Some(m) => format!("waking it after {m} min locks"),
    };
    format!("{}; {lock}; {bat}; banner {}", t(c.ac), c.banner.as_deref().unwrap_or("(default)"))
}

fn questions(mut c: Config) -> Config {
    c.ac = ask_times(c.ac, "");
    let def = c.lock_after.map_or("none".to_string(), |m| m.to_string());
    c.lock_after = ask_for(
        "Waking it after how many minutes takes you to the lock screen? 0 always, none never",
        &def,
        "a number of minutes, 0, or none",
        |a| match a {
            "none" | "never" => Some(None),
            _ => a.parse().ok().map(Some),
        },
    );
    let def = match (c.on_battery, c.battery) {
        (false, _) => "no",
        (true, None) => "yes",
        _ => "own",
    };
    let a = ask_for("On battery too? yes, no, or own (other times)", def, "yes, no or own", |a| {
        ["yes", "no", "own"].contains(&a).then(|| a.to_string())
    });
    c.on_battery = a != "no";
    c.battery = None;
    if a == "own" {
        let b = ask_times(c.ac, " on battery");
        c.battery = Some(b).filter(|b| *b != c.ac);
    }
    c.banner = Some(ask("Banner: logo, name, or the path to a text file?", c.banner.as_deref().unwrap_or("logo")));
    c
}

// ------------------------------------------------------------------ backups

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn tilde(p: &Path) -> String {
    match p.strip_prefix(home()) {
        Ok(r) => format!("~/{}", r.display()),
        Err(_) => p.display().to_string(),
    }
}

/// What setup touched, in ~/.config/glyphwave/setup-backup/manifest. Only the
/// first change of each thing is recorded, so the original stays the
/// original across re-runs.
enum Entry {
    /// the file existed; its original is backup/<n>
    Saved(usize, PathBuf),
    Created(PathBuf),
    /// a GNOME setting and its original value; None = was at its default
    Setting(String, String, Option<String>),
}

struct Manifest {
    dir: PathBuf,
    entries: Vec<Entry>,
}

impl Manifest {
    fn load() -> Manifest {
        let dir = config::dir().join("glyphwave/setup-backup");
        let text = std::fs::read_to_string(dir.join("manifest")).unwrap_or_default();
        let entries = text
            .lines()
            .filter_map(|l| match l.split('\t').collect::<Vec<_>>().as_slice() {
                ["saved", n, p] => Some(Entry::Saved(n.parse().ok()?, p.into())),
                ["created", p] => Some(Entry::Created(p.into())),
                ["setting", s, k, v] => Some(Entry::Setting(s.to_string(), k.to_string(), v.strip_prefix('=').map(Into::into))),
                _ => None,
            })
            .collect();
        Manifest { dir, entries }
    }

    fn save(&self) -> std::io::Result<()> {
        let line = |e: &Entry| match e {
            Entry::Saved(n, p) => format!("saved\t{n}\t{}\n", p.display()),
            Entry::Created(p) => format!("created\t{}\n", p.display()),
            Entry::Setting(s, k, v) => format!("setting\t{s}\t{k}\t{}\n", v.as_ref().map_or("-".into(), |v| format!("={v}"))),
        };
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.dir.join("manifest"), self.entries.iter().map(line).collect::<String>())
    }

    fn has_file(&self, p: &Path) -> Option<&Entry> {
        self.entries.iter().find(|e| matches!(e, Entry::Saved(_, q) | Entry::Created(q) if q == p))
    }

    /// The file as it was before setup first touched it.
    fn original(&self, p: &Path) -> String {
        let b = match self.has_file(p) {
            Some(Entry::Saved(n, _)) => std::fs::read(self.dir.join(n.to_string())).ok(),
            Some(_) => None,
            None => std::fs::read(p).ok(),
        };
        String::from_utf8_lossy(&b.unwrap_or_default()).into_owned()
    }

    fn setting(&self, schema: &str, key: &str) -> Option<Option<String>> {
        self.entries.iter().find_map(|e| match e {
            Entry::Setting(s, k, v) if s == schema && k == key => Some(v.clone()),
            _ => None,
        })
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
        } else if let Some(v) = l.strip_prefix(key).and_then(|r| r.strip_prefix('=')).filter(|_| inside) {
            return Some(v.to_string());
        }
    }
    None
}

/// Set (Some) or remove (None) a key, leaving every other byte alone. A new
/// key goes after the group's last line, a new group at the end; a group
/// left empty goes, with the blank line before it.
fn ini_set(text: &str, group: &str, key: &str, val: Option<&str>) -> String {
    let head = format!("[{group}]");
    let mut out: Vec<String> = text.split_inclusive('\n').map(String::from).collect();
    let Some(start) = out.iter().position(|l| l.trim_end() == head) else {
        let Some(v) = val else { return text.to_string() };
        let mut s = text.to_string();
        if !s.is_empty() {
            s += if s.ends_with("\n\n") { "" } else if s.ends_with('\n') { "\n" } else { "\n\n" };
        }
        return s + &format!("{head}\n{key}={v}\n");
    };
    let end = out[start + 1..].iter().position(|l| l.starts_with('[')).map_or(out.len(), |i| start + 1 + i);
    let found = (start + 1..end).find(|&i| out[i].strip_prefix(key).is_some_and(|r| r.starts_with('=')));
    match (found, val) {
        (Some(i), Some(v)) => out[i] = format!("{key}={v}\n"),
        (Some(i), None) => {
            out.remove(i);
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
    File(PathBuf, Vec<u8>),
    /// a GNOME setting: schema, key, value (None = reset to default)
    Setting(String, String, Option<String>),
}

#[derive(Default)]
struct Plan {
    changes: Vec<Change>,
    warnings: Vec<String>,
    notes: Vec<String>,
}

impl Plan {
    fn file(&mut self, path: PathBuf, new: impl Into<Vec<u8>>) {
        let new = new.into();
        if std::fs::read(&path).ok().as_ref() != Some(&new) {
            self.changes.push(Change::File(path, new));
        }
    }

    fn warn_early(&mut self, what: &str, secs: u32, start: u32, fix: &str) {
        if secs <= start * 60 {
            let t = if secs % 60 == 0 { format!("{} min", secs / 60) } else { format!("{secs} s") };
            self.warnings.push(format!("{what} after {t}, before the screensaver starts ({fix})."));
        }
    }
}

/// A command line for KDE (QProcess::splitCommand) or a .desktop Exec line.
fn cmdline(bin: &Path, args: &str) -> String {
    let b = bin.display().to_string();
    if b.contains(' ') { format!("\"{b}\" {args}") } else { format!("{b} {args}") }
}

/// The modifiers of `Meta+Ctrl+L` as another desktop names them (`names`
/// for super, ctrl, alt, shift, in that order), and its key, a letter in
/// lower case.
fn spell<'a>(k: &str, names: [&'a str; 4]) -> (Vec<&'a str>, String) {
    let (mods, key) = k.rsplit_once('+').unwrap_or(("", k));
    let aka: [&[&str]; 4] = [&["meta", "super", "win"], &["ctrl", "control"], &["alt"], &["shift"]];
    let on = (0..4).filter(|&i| mods.split('+').any(|m| aka[i].contains(&m.trim().to_lowercase().as_str()))).map(|i| names[i]);
    let key = key.trim();
    (on.collect(), if key.len() == 1 { key.to_lowercase() } else { key.to_string() })
}

fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map_or_else(|| home().join(".local/share"), PathBuf::from)
}

/// The app-menu entry, for every desktop; on KDE it also carries the key,
/// which kglobalacceld picks up when the menu database is rebuilt.
fn menu_entry(p: &mut Plan, bin: &Path, kde_key: Option<&str>) {
    let key = kde_key.map_or(String::new(), |k| format!("X-KDE-Shortcuts={k}\n"));
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=glyphwave\nComment=Start the screensaver now\nExec={}\n\
         Icon=preferences-desktop-screensaver\nTerminal=false\nCategories=AudioVideo;\n{key}",
        cmdline(bin, "launch --now")
    );
    p.file(data_dir().join("applications/glyphwave.desktop"), entry);
}

/// The key for KDE, unless kglobalshortcutsrc already gives it to something
/// else: KDE would then quietly leave glyphwave without one.
fn kde_key<'a>(p: &mut Plan, key: &'a str) -> Option<&'a str> {
    let same = |k: &str| spell(k, ["s", "c", "a", "h"]) == spell(key, ["s", "c", "a", "h"]);
    let rc = std::fs::read_to_string(config::dir().join("kglobalshortcutsrc")).unwrap_or_default();
    let mut group = "";
    for l in rc.lines() {
        if l.starts_with('[') {
            group = l;
        } else if let Some((_, v)) = l.split_once('=').filter(|_| group != "[services][glyphwave.desktop]") {
            // `now,default,name`, or just `now` for an app; keys are tab-separated
            if v.split(',').next().unwrap_or("").split('\t').any(same) {
                p.warnings.push(format!(
                    "{key} is already taken in KDE ({group}), so setup binds no key. Free it in \
                     System Settings > Keyboard > Shortcuts and run setup again, or pick another shortcut in the config."
                ));
                return None;
            }
        }
    }
    Some(key)
}

/// The keys setup owns in each powerdevilrc profile, and Plasma 6's
/// defaults for the ones it warns about (AC, battery), used when a key is
/// missing: (group, key that switches it off, its off value, timeout key, AC, battery, what).
const KDE_TIMERS: [(&str, &str, &str, &str, u32, u32, &str); 3] = [
    ("Display", "DimDisplayWhenIdle", "false", "DimDisplayIdleTimeoutSec", 300, 120, "KDE dims the screen"),
    ("Display", "TurnOffDisplayWhenIdle", "false", "TurnOffDisplayIdleTimeoutSec", 600, 300, "KDE turns the screen off"),
    ("SuspendAndShutdown", "AutoSuspendAction", "0", "AutoSuspendIdleTimeoutSec", 900, 600, "KDE sleeps"),
];

/// Edit a KConfig file: our keys back to their pre-setup values first, so a
/// re-run with other answers leaves nothing behind, then `f` sets the new ones.
fn kconfig(p: &mut Plan, m: &Manifest, path: PathBuf, owned: &[(String, &str)], f: impl FnOnce(String) -> String) -> String {
    let orig = m.original(&path);
    let cur = std::fs::read_to_string(&path).unwrap_or_default();
    let t = f(owned.iter().fold(cur.clone(), |t, (g, k)| ini_set(&t, g, k, ini_get(&orig, g, k).as_deref())));
    if t != cur {
        p.file(path, t.clone());
    }
    t
}

fn plan_kde(p: &mut Plan, m: &Manifest, cfg: &Config, bin: &Path) {
    let mut profiles = vec![("AC", cfg.ac)];
    if cfg.on_battery {
        profiles.push(("Battery", cfg.bat()));
    }
    let mut owned = Vec::new();
    for prof in ["AC", "Battery"] {
        owned.push((format!("{prof}][RunScript"), "IdleTimeoutCommand"));
        owned.push((format!("{prof}][RunScript"), "RunScriptIdleTimeoutSec"));
        owned.extend(KDE_TIMERS[1..].iter().flat_map(|r| [(format!("{prof}][{}", r.0), r.1), (format!("{prof}][{}", r.0), r.3)]));
    }
    let t = kconfig(p, m, config::dir().join("powerdevilrc"), &owned, |mut t| {
        for (prof, tm) in &profiles {
            let later = ((tm.start + tm.after) * 60).to_string();
            let mut set = |g: &str, k: &str, v: &str| t = ini_set(&t, &format!("{prof}][{g}"), k, Some(v));
            set("RunScript", "IdleTimeoutCommand", &cmdline(bin, "launch"));
            set("RunScript", "RunScriptIdleTimeoutSec", &(tm.start * 60).to_string());
            match tm.then {
                Then::ScreenOff => {
                    set("Display", "TurnOffDisplayWhenIdle", "true");
                    set("Display", "TurnOffDisplayIdleTimeoutSec", &later);
                }
                Then::Sleep => {
                    set("SuspendAndShutdown", "AutoSuspendAction", "1"); // PowerButtonAction::Sleep
                    set("SuspendAndShutdown", "AutoSuspendIdleTimeoutSec", &later);
                }
                _ => {}
            }
        }
        t
    });
    for (prof, tm) in &profiles {
        for (g, sw, off, key, ac, bat, what) in KDE_TIMERS {
            let g = format!("{prof}][{g}");
            if ini_get(&t, &g, sw).as_deref() != Some(off) {
                let secs = ini_get(&t, &g, key).and_then(|v| v.parse().ok()).unwrap_or(if *prof == "AC" { ac } else { bat });
                p.warn_early(&format!("{what} ({prof})"), secs, tm.start, "System Settings > Power Management");
            }
        }
    }
    // the lock timer is kscreenlocker's, in minutes, one for both power states
    let lock = (cfg.ac.then == Then::Lock).then_some(cfg.ac.start + cfg.ac.after);
    let owned = [("Daemon".to_string(), "Autolock"), ("Daemon".to_string(), "Timeout")];
    let t = kconfig(p, m, config::dir().join("kscreenlockerrc"), &owned, |t| match lock {
        Some(at) => ini_set(&ini_set(&t, "Daemon", "Autolock", Some("true")), "Daemon", "Timeout", Some(&at.to_string())),
        None => t,
    });
    if ini_get(&t, "Daemon", "Autolock").as_deref() != Some("false") {
        let mins: u32 = ini_get(&t, "Daemon", "Timeout").and_then(|v| v.parse().ok()).unwrap_or(5);
        let first = profiles.iter().map(|(_, t)| t.start).min().unwrap_or(5);
        p.warn_early("KDE locks the screen, which ends it,", mins * 60, first, "System Settings > Screen Locking, or then = lock");
    }
}

// GNOME keys setup may change, with the schema defaults that apply while unset
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

/// The value the user set; None while it's at its default (no dconf entry).
/// Debian and Ubuntu don't install the dconf command (dconf-cli), so there
/// it's the effective value, and --remove sets it back rather than resetting.
fn gs_user(schema: &str, key: &str) -> Option<String> {
    if !launch::installed("dconf") {
        return run_out("gsettings", &["get", schema, key]);
    }
    // a relocatable schema says its path: `schema:/its/path/`
    let dir = schema.split_once(':').map_or_else(|| format!("/{}/", schema.replace('.', "/")), |(_, p)| p.to_string());
    run_out("dconf", &["read", &format!("{dir}{key}")]).filter(|v| !v.is_empty())
}

fn gs_num(v: &str) -> u32 {
    v.rsplit(' ').next().and_then(|n| n.parse().ok()).unwrap_or(0)
}

const GS_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const GS_KEY_PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/glyphwave/";

/// A custom keybinding: our path in the list, and its name, command and key.
fn plan_gnome_key(p: &mut Plan, m: &Manifest, cfg: &Config, bin: &Path) {
    let one = format!("{GS_KEYS}.custom-keybinding:{GS_KEY_PATH}");
    let orig = |s: &str, k: &str| m.setting(s, k).unwrap_or_else(|| gs_user(s, k));
    let list = orig(GS_KEYS, "custom-keybindings");
    let mut want = [(GS_KEYS, "custom-keybindings"), (&one, "name"), (&one, "command"), (&one, "binding")].map(|(s, k)| (s, k, orig(s, k)));
    if let Some(key) = &cfg.shortcut {
        let (mods, k) = spell(key, ["<Super>", "<Control>", "<Alt>", "<Shift>"]);
        let list = match list.as_deref() {
            None | Some("@as []") | Some("[]") => format!("['{GS_KEY_PATH}']"),
            Some(l) if l.contains(GS_KEY_PATH) => l.to_string(),
            Some(l) => format!("{}, '{GS_KEY_PATH}']", l.trim_end_matches(']')),
        };
        let vals = [list, "'glyphwave'".into(), format!("'{}'", cmdline(bin, "launch --now")), format!("'{}{k}'", mods.concat())];
        want.iter_mut().zip(vals).for_each(|(w, v)| w.2 = Some(v));
    }
    for (s, k, v) in want {
        if v != gs_user(s, k) {
            p.changes.push(Change::Setting(s.to_string(), k.to_string(), v));
        }
    }
}

fn plan_gnome(p: &mut Plan, m: &Manifest, cfg: &Config, bin: &Path) {
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=glyphwave idle watcher\nExec={}\n\
         OnlyShowIn=GNOME;\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
        cmdline(bin, "idle-watch")
    );
    p.file(config::dir().join("autostart/glyphwave.desktop"), entry);

    // start from the originals, then set what the answers need; blank and
    // lock are one timer for both power states, so they follow plugged-in
    let mut want: Vec<(&str, &str, Option<String>)> =
        GNOME_OWNED.iter().map(|&(s, k, _)| (s, k, m.setting(s, k).unwrap_or_else(|| gs_user(s, k)))).collect();
    let mut set = |k: &str, v: String| want.iter_mut().filter(|w| w.1 == k).for_each(|w| w.2 = Some(v.clone()));
    let later = |t: Times| (t.start + t.after) * 60;
    if matches!(cfg.ac.then, Then::Lock | Then::ScreenOff) {
        set("idle-delay", format!("uint32 {}", later(cfg.ac)));
    }
    if cfg.ac.then == Then::Lock {
        set("lock-enabled", "true".into());
        set("lock-delay", "uint32 0".into());
    }
    let mut powers = vec![("ac", cfg.ac)];
    if cfg.on_battery {
        powers.push(("battery", cfg.bat()));
    }
    for (pw, t) in powers.iter().filter(|(_, t)| t.then == Then::Sleep) {
        set(&format!("sleep-inactive-{pw}-timeout"), later(*t).to_string());
        set(&format!("sleep-inactive-{pw}-type"), "'suspend'".into());
    }
    let eff = |k: &str| {
        let w = want.iter().position(|w| w.1 == k).unwrap();
        want[w].2.clone().unwrap_or(GNOME_OWNED[w].2.to_string())
    };
    for (i, (s, k, v)) in want.iter().enumerate() {
        // nothing to write when the default already says the same
        if *v != gs_user(s, k) && !(gs_user(s, k).is_none() && v.as_deref() == Some(GNOME_OWNED[i].2)) {
            p.changes.push(Change::Setting(s.to_string(), k.to_string(), v.clone()));
        }
    }
    let first = powers.iter().map(|(_, t)| t.start).min().unwrap_or(5);
    if gs_num(&eff("idle-delay")) > 0 {
        p.warn_early("GNOME blanks the screen, which ends it,", gs_num(&eff("idle-delay")), first, "Settings > Power > Screen Blank, or then = lock");
    }
    for (pw, t) in &powers {
        let secs = gs_num(&eff(&format!("sleep-inactive-{pw}-timeout")));
        if eff(&format!("sleep-inactive-{pw}-type")) != "'nothing'" && secs > 0 {
            p.warn_early(&format!("GNOME suspends ({pw})"), secs, t.start, "Settings > Power > Automatic Suspend");
        }
    }
}

/// Hyprland, sway and X11 keep their idle setup in files people write by
/// hand; setup prints the lines (plugged-in times; glyphwave itself skips
/// battery when on_battery = no).
fn paste(d: Desktop, cfg: &Config, bin: &Path) -> (&'static str, String) {
    let (b, t) = (bin.display(), cfg.ac);
    let (start, later) = (t.start * 60, (t.start + t.after) * 60);
    let stop_lock = |l: &str| format!("{b} launch --stop; {l}");
    match d {
        Desktop::Hyprland => {
            let l = &cfg.hyprland_locker;
            let l0 = l.split_whitespace().next().unwrap_or(l);
            let then = match t.then {
                Then::Lock => "loginctl lock-session",
                Then::ScreenOff => "hyprctl dispatch dpms off\n    on-resume = hyprctl dispatch dpms on",
                Then::Sleep => "systemctl suspend",
                Then::Nothing => "",
            };
            let mut s = format!(
                "general {{\n    lock_cmd = {b} launch --stop; pidof {l0} || {l}\n    before_sleep_cmd = loginctl lock-session\n}}\n\n\
                 listener {{\n    timeout = {start}\n    on-timeout = {b} launch\n}}\n"
            );
            if !then.is_empty() {
                s += &format!("\nlistener {{\n    timeout = {later}\n    on-timeout = {then}\n}}\n");
            }
            s += "\n# hyprland.conf, for wezterm: windowrulev2 = fullscreen, class:^(glyphwave)$\n";
            if let Some((mods, k)) = cfg.shortcut.as_deref().map(|k| spell(k, ["SUPER", "CTRL", "ALT", "SHIFT"])) {
                s += &format!("# hyprland.conf, to start it now: bind = {}, {}, exec, {b} launch --now\n", mods.join(" "), k.to_uppercase());
            }
            ("~/.config/hypr/hypridle.conf", s)
        }
        Desktop::Sway => {
            let then = match t.then {
                Then::Lock => format!("    timeout {later} '{}' \\\n", stop_lock(&cfg.sway_locker)),
                Then::ScreenOff => format!("    timeout {later} 'swaymsg \"output * power off\"' resume 'swaymsg \"output * power on\"' \\\n"),
                Then::Sleep => format!("    timeout {later} 'systemctl suspend' \\\n"),
                Then::Nothing => String::new(),
            };
            let mut s = format!(
                "exec swayidle -w \\\n    timeout {start} '{b} launch' \\\n{then}    before-sleep '{}'\n\n\
                 for_window [app_id=\"glyphwave\"] fullscreen enable\nfor_window [class=\"glyphwave\"] fullscreen enable\n",
                stop_lock(&cfg.sway_locker)
            );
            if let Some((mods, k)) = cfg.shortcut.as_deref().map(|k| spell(k, ["Mod4", "Ctrl", "Mod1", "Shift"])) {
                s += &format!("bindsym {}+{k} exec {b} launch --now\n", mods.join("+"));
            }
            ("~/.config/sway/config", s)
        }
        _ => {
            // xidlehook's timers count from the one before
            let then = match t.then {
                Then::Lock => stop_lock(&cfg.x11_locker),
                Then::ScreenOff => "xset dpms force off".into(),
                Then::Sleep => "systemctl suspend".into(),
                Then::Nothing => String::new(),
            };
            let mut s = format!("xidlehook --timer {start} '{b} launch' ''");
            if !then.is_empty() {
                s += &format!(" \\\n    --timer {} '{then}' ''", later - start);
            }
            s += "\n";
            if let Some((mods, k)) = cfg.shortcut.as_deref().map(|k| spell(k, ["super", "ctrl", "alt", "shift"])) {
                s += &format!("\n# to start it now, bind `{b} launch --now` to a key in your WM, or in sxhkdrc:\n# {} + {k}\n#     {b} launch --now\n", mods.join(" + "));
            }
            ("your session autostart (e.g. ~/.xinitrc)", s)
        }
    }
}

// ------------------------------------------------------------------ show, apply, undo

fn describe(c: &Change) -> String {
    match c {
        Change::File(path, new) => {
            let old = std::fs::read(path).ok();
            let mut s = format!("  {} {}", if old.is_some() { "change" } else { "create" }, tilde(path));
            let (Some(old), Ok(new)) = (old, std::str::from_utf8(new)) else {
                return s;
            };
            // the settings lines, each with its [group], that differ
            let keyed = |t: &str| {
                let mut group = "";
                let mut v = Vec::new();
                for l in t.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
                    if l.starts_with('[') { group = l } else { v.push(format!("{group} {l}").trim().to_string()) }
                }
                v
            };
            let (o, n) = (keyed(&String::from_utf8_lossy(&old)), keyed(new));
            n.iter().filter(|l| !o.contains(l)).for_each(|l| s += &format!("\n      + {l}"));
            o.iter().filter(|l| !n.contains(l)).for_each(|l| s += &format!("\n      - {l}"));
            s
        }
        Change::Setting(sch, k, Some(v)) => format!("  set GNOME setting {sch} {k} to {v}"),
        Change::Setting(sch, k, None) => format!("  reset GNOME setting {sch} {k} to its default"),
    }
}

fn apply(c: &Change, m: &mut Manifest) -> std::io::Result<()> {
    match c {
        Change::File(p, new) => {
            if m.has_file(p).is_none() {
                if p.exists() {
                    std::fs::create_dir_all(&m.dir)?;
                    std::fs::copy(p, m.dir.join(m.entries.len().to_string()))?;
                    m.entries.push(Entry::Saved(m.entries.len(), p.clone()));
                } else {
                    m.entries.push(Entry::Created(p.clone()));
                }
                m.save()?;
            }
            std::fs::create_dir_all(p.parent().unwrap_or(Path::new("/")))?;
            if p.file_name().is_some_and(|n| n == "glyphwave") {
                // the binary: beside it, then renamed over, as it may be running
                use std::os::unix::fs::PermissionsExt;
                let tmp = p.with_extension("new");
                std::fs::write(&tmp, new)?;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
                std::fs::rename(&tmp, p)
            } else {
                std::fs::write(p, new) // in place, so the file keeps its permissions
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

fn gsettings(s: &str, k: &str, v: Option<&str>) -> std::io::Result<()> {
    let mut c = Command::new("gsettings");
    match v {
        Some(v) => c.args(["set", s, k, v]),
        None => c.args(["reset", s, k]),
    };
    if c.status()?.success() { Ok(()) } else { Err(std::io::Error::other(format!("gsettings failed for {s} {k}"))) }
}

fn kde_reload() {
    let pm = "org.kde.Solid.PowerManagement";
    let ok = zbus::blocking::Connection::session().is_ok_and(|c| {
        let call = |dest, path, iface, m| c.call_method(Some(dest), path, Some(iface), m, &()).is_ok();
        call(pm, "/org/kde/Solid/PowerManagement", pm, "reparseConfiguration")
            && call(pm, "/org/kde/Solid/PowerManagement", pm, "refreshStatus")
            && call("org.freedesktop.ScreenSaver", "/ScreenSaver", "org.kde.screensaver", "configure")
    });
    println!("{}", if ok { "KDE has reloaded its settings." } else { "KDE didn't answer; the settings apply at the next login." });
}

/// Rebuild KDE's menu database; kglobalacceld then picks up the key at once.
fn sycoca() {
    let null = std::process::Stdio::null;
    let _ = Command::new("kbuildsycoca6").stdout(null()).stderr(null()).status();
}

/// `glyphwave setup [--remove] [--dry-run] [--desktop NAME]`
pub fn run(args: &[String]) -> i32 {
    let has = |a: &str| args.iter().any(|x| x == a);
    let desktop = args.iter().position(|a| a == "--desktop").map(|i| args.get(i + 1).and_then(|d| parse_desktop(d)));
    if desktop == Some(None) || args.iter().any(|a| a.starts_with("--") && !["--remove", "--dry-run", "--desktop"].contains(&a.as_str())) {
        eprintln!("glyphwave: setup takes --remove, --dry-run, --desktop kde|gnome|hyprland|sway|x11");
        return 2;
    }
    if unsafe { libc::geteuid() } == 0 || home().as_os_str().is_empty() {
        eprintln!("glyphwave: run setup as yourself, with HOME set; it only changes your home folder.");
        return 1;
    }
    if has("--remove") { undo(has("--dry-run")) } else { setup(desktop.flatten(), has("--dry-run")) }
}

fn setup(desktop: Option<Desktop>, dry: bool) -> i32 {
    let Some(d) = desktop.or_else(detect) else {
        eprintln!("glyphwave: can't tell which desktop this is; say it with --desktop kde|gnome|hyprland|sway|x11");
        return 1;
    };
    println!("Desktop: {}", desktop_name(d));
    let mut m = Manifest::load();
    let cfg = match config::load() {
        Some(c) => {
            println!("Settings in {}: {}", tilde(&config::path()), summary(&c));
            if ask("Use these? (n asks again)", "Y/n").to_lowercase() == "n" { questions(c) } else { c }
        }
        None => questions(Config::default()),
    };

    let mut p = Plan::default();
    p.file(config::path(), cfg.render());
    // the idle timer needs a lasting path: copy a downloaded binary to ~/.local/bin
    let exe = std::env::current_exe().unwrap_or_default();
    let on_path = exe.parent().is_some_and(|d| std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).any(|p| p == d));
    let bin = if on_path { exe.clone() } else { home().join(".local/bin/glyphwave") };
    if bin != exe {
        p.file(bin.clone(), std::fs::read(&exe).unwrap_or_default());
    }
    let mut pasted = None;
    let key = cfg.shortcut.as_deref().filter(|_| d == Desktop::Kde).and_then(|k| kde_key(&mut p, k));
    menu_entry(&mut p, &bin, key);
    let bound = if d == Desktop::Gnome { cfg.shortcut.as_deref() } else { key };
    match d {
        Desktop::Kde => plan_kde(&mut p, &m, &cfg, &bin),
        Desktop::Gnome => {
            plan_gnome(&mut p, &m, &cfg, &bin);
            plan_gnome_key(&mut p, &m, &cfg, &bin);
        }
        _ => pasted = Some(paste(d, &cfg, &bin)),
    }
    if let Some(m) = cfg.lock_after {
        let wake = if m == 0 { "Waking it locks the screen".to_string() } else { format!("Waking it after {m} min locks the screen") };
        let key = if m > 0 && cfg.shortcut.is_some() { " (at once when you started it with the key)" } else { "" };
        let then = [cfg.ac, cfg.bat()].into_iter().filter(|t| t.then == Then::Lock).map(|t| t.after).min();
        p.notes.push(match then {
            Some(a) if a <= m => format!("then = lock comes first: at {a} min the lock screen replaces the screensaver, so lock_after = {m} never comes into play."),
            Some(a) => format!("{wake}{key}; at {a} min the lock screen replaces the screensaver (then = lock)."),
            None => format!("{wake}{key}."),
        });
    }
    if [cfg.ac, cfg.bat()].iter().any(|t| t.then == Then::Sleep) {
        p.notes.push(match d {
            Desktop::Kde => "Sleep: while music plays, players hold off sleep. When the music stops, KDE restarts the idle count, so sleep comes the full time after the music.",
            Desktop::Gnome => "Sleep: while music plays, players hold off sleep. When the music stops, GNOME counts from your last input, so if that was long enough ago it sleeps right away.",
            _ => "Sleep: while music plays, players hold off sleep; the idle daemon decides what happens when it stops.",
        }.into());
    }

    println!();
    for w in &p.warnings {
        println!("Warning: {w}");
    }
    if p.changes.is_empty() {
        println!("Nothing to change.");
    } else {
        println!("Setup will:");
        p.changes.iter().for_each(|c| println!("{}", describe(c)));
        if let Some(k) = bound {
            println!("  bind {k} to start it now (and add it to the app menu)");
        }
        println!("Originals are kept in {} for `glyphwave setup --remove`.", tilde(&m.dir));
        if dry {
            println!("(dry run: nothing written)");
        } else if !ask_yes("Go ahead?") {
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
        if d == Desktop::Kde && !p.changes.is_empty() {
            kde_reload();
            sycoca();
        }
        if d == Desktop::Gnome {
            use std::os::unix::process::CommandExt;
            launch::stop("idle-watch"); // a re-run picks up the new times
            let null = std::process::Stdio::null;
            let _ = Command::new(&bin).arg("idle-watch").stdin(null()).stdout(null()).stderr(null()).process_group(0).spawn();
            println!("The idle watcher runs now and at each login.");
        }
    }
    if let Some((file, text)) = pasted {
        println!("\nAdd these lines to {file}:\n\n{text}");
    }
    p.notes.iter().for_each(|n| println!("{n}"));
    if !launch::installed("parec") {
        println!("For the music visuals, install parec (pulseaudio-utils, or libpulse on Arch).");
    }
    if cfg.banner.as_deref() == Some("logo") && !launch::installed("fastfetch") && !launch::installed("neofetch") {
        println!("Optional: with fastfetch installed, the banner shows your system's logo.");
    }
    0
}

fn undo(dry: bool) -> i32 {
    let m = Manifest::load();
    if m.entries.is_empty() {
        println!("Nothing to remove: setup hasn't changed anything.");
        return 0;
    }
    println!("Remove will:");
    for e in &m.entries {
        match e {
            Entry::Saved(_, p) => println!("  restore {}", tilde(p)),
            Entry::Created(p) => println!("  delete {}", tilde(p)),
            Entry::Setting(s, k, Some(v)) => println!("  set GNOME setting {s} {k} back to {v}"),
            Entry::Setting(s, k, None) => println!("  reset GNOME setting {s} {k} to its default"),
        }
    }
    println!("  delete {}", tilde(&m.dir));
    if dry || !ask_yes("Go ahead?") {
        println!("Nothing changed.");
        return i32::from(!dry);
    }
    let menu = std::fs::read_to_string(data_dir().join("applications/glyphwave.desktop")).unwrap_or_default();
    let mut ok = true;
    for e in m.entries.iter().rev() {
        let r = match e {
            Entry::Saved(n, p) => std::fs::copy(m.dir.join(n.to_string()), p).map(drop),
            Entry::Created(p) => {
                if p.ends_with("autostart/glyphwave.desktop") {
                    launch::stop("idle-watch");
                }
                std::fs::remove_file(p).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
            }
            Entry::Setting(s, k, v) => gsettings(s, k, v.as_deref()),
        };
        if let Err(e) = r {
            eprintln!("glyphwave: {e}");
            ok = false;
        }
    }
    if !ok {
        eprintln!("glyphwave: kept {} so you can retry", tilde(&m.dir));
        return 1;
    }
    let _ = std::fs::remove_dir_all(&m.dir);
    let _ = std::fs::remove_dir(config::dir().join("glyphwave")); // only if empty
    if m.entries.iter().any(|e| matches!(e, Entry::Saved(_, p) | Entry::Created(p) if p.ends_with("powerdevilrc") || p.ends_with("kscreenlockerrc"))) {
        kde_reload();
    }
    if menu.contains("X-KDE-Shortcuts=") {
        sycoca(); // KDE lets go of the key
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
        // set then remove gives back the original bytes, also for a new group
        assert_eq!(ini_set(&ini_set(RC, "AC][Display", "X", Some("1")), "AC][Display", "X", None), RC);
        assert_eq!(ini_set(&t, "Battery][RunScript", "RunScriptIdleTimeoutSec", None), RC);
        assert_eq!(ini_set("", "Daemon", "Timeout", Some("15")), "[Daemon]\nTimeout=15\n");
        // a key that only shares a prefix is a different key
        assert_eq!(ini_get("[A]\nTimeoutSec=1\n", "A", "Timeout"), None);
    }

    #[test]
    fn spell_shortcuts() {
        let gnome = ["<Super>", "<Control>", "<Alt>", "<Shift>"];
        assert_eq!(spell("Meta+Shift+V", gnome), (vec!["<Super>", "<Shift>"], "v".to_string()));
        assert_eq!(spell("shift+Super+v", gnome), spell("Meta+Shift+V", gnome));
        assert_ne!(spell("Meta+V", gnome), spell("Meta+Shift+V", gnome));
        assert_eq!(spell("Ctrl+Alt+F12", gnome), (vec!["<Control>", "<Alt>"], "F12".to_string()));
    }

    #[test]
    fn cmdline_quotes_only_when_needed() {
        assert_eq!(cmdline(Path::new("/home/a/.local/bin/glyphwave"), "launch"), "/home/a/.local/bin/glyphwave launch");
        assert_eq!(cmdline(Path::new("/home/a b/glyphwave"), "launch"), "\"/home/a b/glyphwave\" launch");
    }
}
