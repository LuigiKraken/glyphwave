//! `glyphwave launch`: what the desktop's idle timer runs. Opens glyphwave
//! fullscreen in a terminal, once, and never under a lock screen.
//! `glyphwave idle-watch`: the idle timer GNOME doesn't have, kept tiny: one
//! Mutter idle watch per start time, then `launch` when it fires.

use crate::config::{self, Config};
use std::os::unix::process::CommandExt;
use std::process::Command;

pub const TERMINALS: [&str; 9] =
    ["kitty", "foot", "alacritty", "ghostty", "wezterm", "konsole", "ptyxis", "gnome-terminal", "xterm"];

/// Wayland/X lockers that aren't on the bus; KDE's and GNOME's glyphwave
/// watches itself.
const LOCKERS: [&str; 6] = ["hyprlock", "swaylock", "waylock", "gtklock", "i3lock", "xsecurelock"];

/// Plugged in, unless some supply reports `online` and none of them is.
/// A desktop with no supplies counts as plugged in.
pub fn on_battery() -> bool {
    let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") else { return false };
    let mut seen = false;
    for e in rd.flatten() {
        if let Ok(v) = std::fs::read_to_string(e.path().join("online")) {
            seen = true;
            if v.trim() == "1" {
                return false;
            }
        }
    }
    seen
}

/// (pid, argv) of every process we may look at.
fn processes() -> Vec<(i32, Vec<String>)> {
    let me = std::process::id() as i32;
    let Ok(rd) = std::fs::read_dir("/proc") else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let pid: i32 = e.file_name().to_str()?.parse().ok()?;
            let raw = std::fs::read(e.path().join("cmdline")).ok()?;
            let argv = raw.split(|&b| b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned());
            Some((pid, argv.collect::<Vec<_>>())).filter(|(p, a)| *p != me && !a.is_empty())
        })
        .collect()
}

/// glyphwave processes run with `first_arg` (`--screensaver`, `idle-watch`).
pub fn find(first_arg: &str) -> Vec<i32> {
    processes()
        .into_iter()
        .filter(|(_, a)| a[0].rsplit('/').next() == Some("glyphwave") && a.get(1).map(String::as_str) == Some(first_arg))
        .map(|(p, _)| p)
        .collect()
}

pub fn stop(first_arg: &str) {
    for pid in find(first_arg) {
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
}

pub fn locked() -> bool {
    std::fs::read_dir("/proc").into_iter().flatten().flatten().any(|e| {
        std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| LOCKERS.contains(&c.trim()))
    })
}

pub fn installed(cmd: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|d| {
        std::fs::metadata(d.join(cmd)).is_ok_and(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0)
    })
}

/// The terminal the launcher will use: the config's, or the first installed.
pub fn terminal(cfg: &Config) -> Option<String> {
    cfg.terminal.clone().or_else(|| TERMINALS.iter().find(|t| installed(t)).map(|t| t.to_string()))
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn command(term: &str, cfg: &Config, bin: &str, now: bool) -> Option<Vec<String>> {
    let run: &[&str] = if now { &[bin, "--screensaver", "--now"] } else { &[bin, "--screensaver"] };
    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).chain(run.iter().map(|s| s.to_string())).collect();
    Some(match term {
        // No scrollbar or padding, a black background, and the pointer hidden
        // where the terminal can do that without a key press.
        "kitty" => v(&[
            "kitty", "--class", "glyphwave", "--start-as=fullscreen", "-o", "background=#000000",
            "-o", "window_padding_width=0", "-o", "mouse_hide_wait=1",
        ]),
        "foot" => v(&["foot", "--app-id=glyphwave", "--fullscreen", "-o", "colors.background=000000", "-o", "pad=0x0"]),
        "alacritty" => v(&[
            "alacritty", "--class", "glyphwave", "-o", "window.startup_mode=\"Fullscreen\"",
            "-o", "colors.primary.background=\"#000000\"", "-o", "window.padding.x=0", "-o", "window.padding.y=0", "-e",
        ]),
        "ghostty" => v(&["ghostty", "--fullscreen=true", "--background=000000", "--window-padding-x=0", "--window-padding-y=0", "-e"]),
        "wezterm" => v(&[
            "wezterm", "--config", "enable_scroll_bar=false", "--config", "window_padding={left=0,right=0,top=0,bottom=0}",
            "start", "--class", "glyphwave", "--",
        ]), // fullscreen via a window rule
        "konsole" => {
            let mut c = vec!["konsole".to_string()];
            // No scrollbar or margin over any profile; black under the
            // margin and the part-cell leftover unless the user's profile
            // picks its own colours.
            c.extend(["-p", "ScrollBarPosition=2", "-p", "TerminalMargin=0"].map(String::from));
            match &cfg.konsole_profile {
                Some(p) => c.extend(["--profile".to_string(), p.clone()]),
                None => c.extend(["-p", "ColorScheme=WhiteOnBlack"].map(String::from)),
            }
            let rest = ["--fullscreen", "--hide-menubar", "--hide-tabbar", "--notransparency", "-e"];
            c.extend(rest.iter().chain(run).map(|s| s.to_string()));
            c
        }
        // standalone, so the process lives as long as the window and holds the lock
        "ptyxis" => vec!["ptyxis".into(), "-s".into(), "--fullscreen".into(), "-x".into(), format!("{} --screensaver{}", quote(bin), if now { " --now" } else { "" })],
        "gnome-terminal" => v(&["gnome-terminal", "--wait", "--full-screen", "--hide-menubar", "--"]),
        // a black pointer on the black background
        "xterm" => v(&[
            "xterm", "-class", "glyphwave", "-fullscreen", "-bg", "black", "+sb", "-b", "0",
            "-ms", "black", "-xrm", "*pointerColorBackground: black", "-e",
        ]),
        _ => return None,
    })
}

/// `glyphwave launch [--stop] [--now] [--on-ac | --on-battery]`; --now is
/// the start-now key and menu entry: a visualizer, which never locks when
/// woken and keeps the desktop from dimming, locking or sleeping meanwhile.
pub fn launch(args: &[String]) -> i32 {
    let has = |a: &str| args.iter().any(|x| x == a);
    if has("--stop") {
        stop("--screensaver");
        return 0;
    }
    let cfg = config::load().unwrap_or_default();
    let bat = on_battery();
    if (bat && !cfg.on_battery) || (bat && has("--on-ac")) || (!bat && has("--on-battery")) {
        return 0;
    }
    if locked() || !find("--screensaver").is_empty() {
        return 0;
    }
    let bin = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or("glyphwave".into());
    let Some(term) = terminal(&cfg) else {
        eprintln!("glyphwave: no terminal found; set one under [terminal] in {}", config::path().display());
        return 1;
    };
    let Some(cmd) = command(&term, &cfg, &bin, has("--now")) else {
        eprintln!("glyphwave: unknown terminal {term:?}; known: {}", TERMINALS.join(" "));
        return 1;
    };

    // The lock keeps two idle timers from opening two windows. It's inherited
    // by the terminal and released when it exits.
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or("/tmp".into());
    let lock = format!("{dir}/glyphwave-idle-{}.lock\0", unsafe { libc::getuid() }); // own even in /tmp
    unsafe {
        let fd = libc::open(lock.as_ptr().cast(), libc::O_RDWR | libc::O_CREAT, 0o600);
        if fd >= 0 && libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) != 0 {
            return 0;
        }
    }
    let err = Command::new(&cmd[0]).args(&cmd[1..]).exec();
    eprintln!("glyphwave: can't run {}: {err}", cmd[0]);
    1
}

/// Lock the session for a screensaver dismissed past lock_after, and wait
/// (up to 2 s) until the lock screen is up, so the window closes onto it
/// rather than onto the desktop. KDE and GNOME lock through logind, which
/// they answer on the bus (`bus_locked`); Hyprland, sway and X11 run the
/// config's locker, which shows up as a process.
pub fn lock_session(cfg: &Config, bus_locked: &std::sync::atomic::AtomicBool) {
    use crate::setup::Desktop;
    let cmd = match crate::setup::detect() {
        Some(Desktop::Hyprland) => &cfg.hyprland_locker,
        Some(Desktop::Sway) => &cfg.sway_locker,
        Some(Desktop::X11) => &cfg.x11_locker,
        _ => "loginctl lock-session",
    };
    let null = std::process::Stdio::null;
    let mut c = Command::new("sh");
    c.args(["-c", cmd]).stdin(null()).stdout(null()).stderr(null());
    // its own session, so the locker outlives the terminal closing
    unsafe { c.pre_exec(|| { libc::setsid(); Ok(()) }) };
    let Ok(mut ch) = c.spawn() else { return };
    std::thread::spawn(move || ch.wait()); // reap a locker that forks
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < until && !bus_locked.load(std::sync::atomic::Ordering::Relaxed) && !locked() {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Holds the desktop's idle timers off for a screensaver started by hand:
/// org.freedesktop.ScreenSaver (KDE, hypridle), else GNOME's session
/// manager. Let go on drop, or by the bus when the process dies.
pub struct Awake(Option<(zbus::blocking::Connection, &'static Inhibitor, u32)>);

/// (service and interface, path, release method)
type Inhibitor = (&'static str, &'static str, &'static str);
const INHIBITORS: [Inhibitor; 2] = [
    ("org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver", "UnInhibit"),
    ("org.gnome.SessionManager", "/org/gnome/SessionManager", "Uninhibit"),
];

/// Best effort: with no inhibitor on the bus nothing changes.
pub fn keep_awake() -> Awake {
    if crate::setup::detect() == Some(crate::setup::Desktop::Sway) {
        sway_inhibit();
    }
    const WHY: &str = "music visualizer started by hand";
    let held = || {
        let conn = zbus::blocking::Connection::session().ok()?;
        let call = |i: &'static Inhibitor| {
            let (n, p, _) = *i;
            let r = if n == INHIBITORS[0].0 {
                conn.call_method(Some(n), p, Some(n), "Inhibit", &("glyphwave", WHY))
            } else {
                // no window id; 4 | 8: suspend and idle (blank, lock)
                conn.call_method(Some(n), p, Some(n), "Inhibit", &("glyphwave", 0u32, WHY, 12u32))
            };
            r.ok()?.body().deserialize::<u32>().ok().map(|c| (i, c))
        };
        let (i, c) = INHIBITORS.iter().find_map(call)?;
        Some((conn, i, c))
    };
    Awake(held())
}

impl Drop for Awake {
    fn drop(&mut self) {
        if let Some((conn, (n, p, un), c)) = &self.0 {
            let _ = conn.call_method(Some(*n), *p, Some(*n), *un, c);
        }
    }
}

/// swayidle doesn't hear the bus, only Wayland idle inhibitors: sway makes
/// the window one, gone with it. Retried until the terminal's window is up.
fn sway_inhibit() {
    std::thread::spawn(|| {
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let set = |c: &str| {
                let q = format!("[{c}=\"glyphwave\"] inhibit_idle open");
                Command::new("swaymsg").arg(q).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
            };
            if set("app_id") | set("class") {
                return;
            }
        }
    });
}

/// `glyphwave idle-watch`, started from ~/.config/autostart on GNOME.
pub fn idle_watch() -> i32 {
    use zbus::blocking::{Connection, MessageIterator};
    use zbus::message::Type;
    const NAME: &str = "org.gnome.Mutter.IdleMonitor";
    const PATH: &str = "/org/gnome/Mutter/IdleMonitor/Core";

    let cfg = config::load().unwrap_or_default();
    let run = || -> zbus::Result<()> {
        let conn = Connection::session()?;
        let rule = zbus::MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(NAME)?
            .member("WatchFired")?
            .path(PATH)?
            .build();
        let msgs = MessageIterator::for_match_rule(rule, &conn, None)?;
        let add = |min: u32| -> zbus::Result<u32> {
            let ms = min as u64 * 60_000;
            conn.call_method(Some(NAME), PATH, Some(NAME), "AddIdleWatch", &ms)?.body().deserialize()
        };
        // One watch fires again each time the idle time is reached, until removed.
        let ac = add(cfg.ac.start)?;
        let bat = if cfg.on_battery && cfg.bat().start != cfg.ac.start { Some(add(cfg.bat().start)?) } else { None };
        let me = std::env::current_exe().map_err(|e| zbus::Error::Failure(e.to_string()))?;
        for m in msgs {
            let id: u32 = m?.body().deserialize()?;
            let when = match bat {
                None if id == ac => None,
                Some(b) if id == ac || id == b => Some(if id == ac { "--on-ac" } else { "--on-battery" }),
                _ => continue,
            };
            let mut c = Command::new(&me);
            c.arg("launch").args(when);
            if let Ok(mut ch) = c.spawn() {
                std::thread::spawn(move || ch.wait()); // reap it without blocking the watch
            }
        }
        Ok(())
    };
    match run() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("glyphwave: idle-watch needs GNOME's Mutter on the session bus: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn konsole_black_only_without_a_profile() {
        let c = command("konsole", &Config::default(), "gw", false).unwrap().join(" ");
        assert!(c.starts_with("konsole -p ScrollBarPosition=2 -p TerminalMargin=0 -p ColorScheme=WhiteOnBlack --fullscreen"), "{c}");
        let cfg = Config { konsole_profile: Some("Mine".into()), ..Config::default() };
        let c = command("konsole", &cfg, "gw", false).unwrap().join(" ");
        assert!(c.contains("-p TerminalMargin=0 --profile Mine --fullscreen") && !c.contains("ColorScheme"), "{c}");
    }
}
