//! More than one screen. One glyphwave process does the work for all of
//! them: it records and analyses the audio, watches the bus, rings and locks,
//! once. Each extra screen gets a terminal of its own running `glyphwave
//! --screensaver --screen LABEL`, a stub that only keeps the window open; the
//! main process opens the stub's terminal, draws into it and reads its keys.
//! The stub waits on a lock the main process holds and exits when it lets go,
//! so no window outlives it, not even after a kill -9.
//!
//! Which window goes where is the compositor's to say: KWin takes a script
//! over D-Bus, sway a swaymsg by pid, Hyprland exec rules. GNOME and the other
//! X11 window managers have no way to ask, so there it's one window, on the
//! screen the desktop picks.

use crate::config::Config;
use crate::launch;
use crate::setup::Desktop;
use std::os::fd::RawFd;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const KWIN_SCRIPT: &str = "glyphwave-screens";

/// Connectors with a screen plugged in, and whether the desktop lights it.
fn connectors() -> Vec<(String, bool)> {
    let Ok(rd) = std::fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut v: Vec<(String, bool)> = rd
        .flatten()
        .filter_map(|e| {
            let st = std::fs::read_to_string(e.path().join("status")).ok()?;
            let on = std::fs::read_to_string(e.path().join("enabled")).is_ok_and(|s| s.trim() == "enabled");
            let name = e.file_name().to_str()?.split_once('-')?.1.to_string(); // card1-DP-1
            (st.trim() == "connected").then_some((name, on))
        })
        .collect();
    v.sort();
    v
}

/// Screens in use; at least one.
pub fn count() -> usize {
    connectors().iter().filter(|c| c.1).count().max(1)
}

/// Why glyphwave can't put windows on the other screens here, or None when
/// it can.
pub fn unplaceable(d: Option<Desktop>, term: Option<&str>) -> Option<&'static str> {
    match d {
        None => Some("glyphwave can't tell which desktop this is"),
        Some(Desktop::Gnome) => Some("GNOME doesn't let an app choose the screen a window opens on"),
        Some(Desktop::X11) => Some("on X11 only KDE lets an app choose the screen a window opens on"),
        // its windows all come from one server process, so their pids can't tell them apart
        _ if term == Some("gnome-terminal") => Some("GNOME Terminal opens every window from one process; any other terminal works"),
        _ => None,
    }
}

/// sway's or Hyprland's screens, the focused one first.
fn outputs(d: Desktop) -> Vec<String> {
    let (cmd, args): (&str, &[&str]) = if d == Desktop::Sway { ("swaymsg", &["-t", "get_outputs"]) } else { ("hyprctl", &["monitors"]) };
    let Some(out) = Command::new(cmd).args(args).stderr(Stdio::null()).output().ok().filter(|o| o.status.success()) else {
        return Vec::new();
    };
    let mut v: Vec<(bool, String)> = Vec::new();
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        // sway: `Output DP-1 'Make Model' (focused)`; Hyprland: `Monitor DP-1 (ID 0):` then `focused: yes`
        if let Some(r) = l.strip_prefix("Output ").or(l.strip_prefix("Monitor ")) {
            if !l.contains("(disabled)") && !l.contains("(inactive)") {
                v.push((l.ends_with("(focused)"), r.split_whitespace().next().unwrap_or("").to_string()));
            }
        } else if l.trim() == "focused: yes" {
            if let Some(m) = v.last_mut() {
                m.0 = true;
            }
        }
    }
    v.sort_by_key(|m| !m.0);
    v.into_iter().map(|m| m.1).collect()
}

fn lock_path() -> String {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or("/tmp".into());
    format!("{dir}/glyphwave-screens-{}.lock\0", unsafe { libc::getuid() })
}

/// `glyphwave --screensaver --screen LABEL`: hold an extra screen's window
/// open until the main process exits.
pub fn stub() -> i32 {
    let p = lock_path();
    unsafe {
        let fd = libc::open(p.as_ptr().cast(), libc::O_RDONLY | libc::O_CLOEXEC);
        if fd >= 0 {
            libc::flock(fd, libc::LOCK_SH);
        }
    }
    0
}

/// The first ancestor of this process that is the terminal `term`: the
/// window glyphwave itself runs in.
fn own_terminal(term: &str) -> Option<i32> {
    let mut pid = std::process::id() as i32;
    for _ in 0..4 {
        let st = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        pid = st.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()?;
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        let comm = comm.trim();
        if comm == term || comm == format!("{term}-gui") {
            return Some(pid);
        }
    }
    None
}

/// The extra screens' terminals, until they've all shown up.
pub struct Extra {
    /// label → the terminal's pid (when glyphwave started it itself), while
    /// its stub hasn't shown up
    want: Vec<(String, Option<i32>)>,
    since: Instant,
    scanned: Instant,
    kwin: bool,
    plugged: Vec<String>,
    checked: Instant,
}

impl Extra {
    /// Open a terminal on each other screen, when there are any and this
    /// desktop and terminal can place them.
    pub fn start(cfg: &Config) -> Option<Extra> {
        let d = crate::setup::detect();
        let term = launch::terminal(cfg)?;
        if count() < 2 || unplaceable(d, Some(&term)).is_some() {
            return None;
        }
        let d = d?;
        // held (not inherited) until glyphwave exits; the stubs wait on it
        let p = lock_path();
        unsafe {
            let fd = libc::open(p.as_ptr().cast(), libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC, 0o600);
            if fd < 0 || libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) != 0 {
                return None;
            }
        }
        let bin = std::env::current_exe().ok()?.to_string_lossy().into_owned();
        let labels: Vec<String> = match d {
            Desktop::Kde => (1..count()).map(|i| i.to_string()).collect(),
            _ => outputs(d).into_iter().skip(1).collect(),
        };
        let mut want = Vec::new();
        for l in labels {
            let cmd = launch::command(&term, cfg, &[&bin, "--screensaver", "--screen", &l])?;
            let mut c = if d == Desktop::Hyprland {
                let line = cmd.iter().map(|a| launch::quote(a)).collect::<Vec<_>>().join(" ");
                let mut c = Command::new("hyprctl");
                c.args(["dispatch", "exec", &format!("[monitor {l}; fullscreen] {line}")]);
                c
            } else {
                let mut c = Command::new(&cmd[0]);
                c.args(&cmd[1..]);
                c
            };
            c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            let Ok(mut ch) = c.spawn() else { continue };
            let pid = (d != Desktop::Hyprland).then_some(ch.id() as i32);
            std::thread::spawn(move || ch.wait());
            if let Some(p) = pid.filter(|_| d == Desktop::Sway) {
                let out = l.clone();
                std::thread::spawn(move || sway_place(p, &out));
            }
            want.push((l, pid));
        }
        let kwin = d == Desktop::Kde;
        if kwin {
            let mut map: Vec<String> = want.iter().filter_map(|(l, p)| Some(format!("{}: {l}", (*p)?))).collect();
            map.extend(own_terminal(&term).map(|p| format!("{p}: 0")));
            if let Err(e) = kwin_place(&map.join(", ")) {
                eprintln!("glyphwave: KWin didn't take the window placement: {e}");
            }
        }
        let now = Instant::now();
        let plugged = connectors().into_iter().map(|c| c.0).collect();
        Some(Extra { want, since: now, scanned: now, kwin, plugged, checked: now })
    }

    /// Stubs that showed up since the last call: their label and terminal,
    /// opened for reading and writing.
    pub fn attach(&mut self) -> Vec<(String, RawFd)> {
        let mut got = Vec::new();
        if self.want.is_empty() || self.scanned.elapsed() < Duration::from_millis(250) {
            return got;
        }
        self.scanned = Instant::now();
        for (pid, a) in launch::processes() {
            let is_stub = a.len() == 4 && a[0].rsplit('/').next() == Some("glyphwave") && a[1] == "--screensaver" && a[2] == "--screen";
            let Some(i) = self.want.iter().position(|w| is_stub && w.0 == a[3]) else { continue };
            let Ok(tty) = std::fs::read_link(format!("/proc/{pid}/fd/0")) else { continue };
            let Ok(path) = std::ffi::CString::new(tty.as_os_str().as_encoded_bytes()) else { continue };
            let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
            if fd >= 0 && unsafe { libc::isatty(fd) } == 1 {
                got.push((self.want.remove(i).0, fd));
            }
        }
        // a terminal whose stub never came: close it rather than leave it
        if self.since.elapsed() > Duration::from_secs(10) {
            for (_, pid) in self.want.drain(..) {
                if let Some(p) = pid {
                    unsafe { libc::kill(p, libc::SIGTERM) };
                }
            }
        }
        got
    }

    /// Whether a screen was plugged in or out (checked every 2 s): someone is
    /// at the machine, and a window may have moved onto another screen.
    pub fn replugged(&mut self) -> bool {
        if self.checked.elapsed() < Duration::from_secs(2) {
            return false;
        }
        self.checked = Instant::now();
        let now: Vec<String> = connectors().into_iter().map(|c| c.0).collect();
        now != self.plugged
    }

    pub fn end(&self) {
        if self.kwin {
            if let Ok(c) = zbus::blocking::Connection::session() {
                let _ = c.call_method(Some("org.kde.KWin"), "/Scripting", Some("org.kde.kwin.Scripting"), "unloadScript", &(KWIN_SCRIPT));
            }
            let _ = std::fs::remove_file(script_path());
        }
    }
}

/// Move the window of `pid` to output `out` once it's there, fullscreen.
fn sway_place(pid: i32, out: &str) {
    let cmd = format!("[pid={pid}] fullscreen disable, move container to output \"{out}\", fullscreen enable");
    for _ in 0..40 {
        let ok = Command::new("swaymsg").arg(&cmd).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        if ok {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn script_path() -> String {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or("/tmp".into());
    format!("{dir}/{KWIN_SCRIPT}-{}.js", unsafe { libc::getuid() })
}

/// A KWin script that puts each window, by its pid, fullscreen on its screen
/// in KWin's order (0 = the primary one), now and as they open. It stays
/// loaded until glyphwave ends.
fn kwin_place(map: &str) -> zbus::Result<()> {
    let js = format!(
        "var want = {{{map}}};\n\
         function place(w) {{\n\
         \x20   var i = want[w.pid], o = workspace.screenOrder[i];\n\
         \x20   if (i === undefined || !o || !w.normalWindow) return;\n\
         \x20   if (w.output.name !== o.name) {{ w.fullScreen = false; workspace.sendClientToScreen(w, o); }}\n\
         \x20   w.fullScreen = true;\n\
         }}\n\
         workspace.windowList().forEach(place);\n\
         workspace.windowAdded.connect(place);\n"
    );
    // KWin reads it in the background, so it stays until glyphwave ends
    let path = script_path();
    std::fs::write(&path, js).map_err(|e| zbus::Error::Failure(e.to_string()))?;
    let c = zbus::blocking::Connection::session()?;
    let (kwin, s) = (Some("org.kde.KWin"), Some("org.kde.kwin.Scripting"));
    let _ = c.call_method(kwin, "/Scripting", s, "unloadScript", &(KWIN_SCRIPT));
    let id: i32 = c.call_method(kwin, "/Scripting", s, "loadScript", &(path.as_str(), KWIN_SCRIPT))?.body().deserialize()?;
    c.call_method(kwin, format!("/Scripting/Script{id}").as_str(), Some("org.kde.kwin.Script"), "run", &())?;
    Ok(())
}
