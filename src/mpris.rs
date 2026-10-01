//! Session-bus watcher: the now-playing track from any MPRIS player (Spotify
//! preferred, then whatever is playing, then whichever played last), and — in
//! screensaver mode — whether a bus locker (KDE, GNOME) has taken over. Re-read only when
//! a player or the locker signals a change, so an idle bus costs no wakeups.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, MessageIterator, Proxy, fdo::DBusProxy, proxy::Builder};
use zbus::message::Type;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;
use zbus::MatchRule;

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Clone, Default, Debug)]
pub struct Track {
    pub player: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub length: f64,
    pub position: f64,
    pub status: String,
    pub at: Option<Instant>,
}

impl Track {
    pub fn playing(&self) -> bool {
        self.status == "Playing"
    }

    pub fn position_now(&self) -> f64 {
        let mut p = self.position;
        if self.playing() {
            if let Some(at) = self.at {
                p += at.elapsed().as_secs_f64();
            }
        }
        if self.length > 0.0 { p.min(self.length) } else { p }
    }
}

pub struct Watcher {
    pub track: Arc<Mutex<Track>>,
    pub locked: Arc<AtomicBool>,
    conn: Option<Connection>,
}

fn md_str(md: &HashMap<String, OwnedValue>, k: &str) -> String {
    md.get(k).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default()
}

fn md_strs(md: &HashMap<String, OwnedValue>, k: &str) -> String {
    let Some(v) = md.get(k) else { return String::new() };
    if let Ok(v) = v.try_clone() {
        if let Ok(list) = Vec::<String>::try_from(v) {
            return list.join(", ");
        }
    }
    md_str(md, k)
}

fn md_len(md: &HashMap<String, OwnedValue>) -> f64 {
    let Some(v) = md.get("mpris:length") else { return 0.0 };
    let us = v
        .downcast_ref::<i64>()
        .ok()
        .or_else(|| v.downcast_ref::<u64>().ok().map(|u| u as i64))
        .unwrap_or(0);
    us as f64 / 1e6
}

fn player_proxy<'a>(conn: &'a Connection, name: &'a str) -> Option<Proxy<'a>> {
    Builder::new(conn)
        .destination(name)
        .ok()?
        .path(PATH)
        .ok()?
        .interface(PLAYER)
        .ok()?
        .cache_properties(CacheProperties::No)
        .build()
        .ok()
}

/// `last` is the player most recently seen playing, so after a pause the
/// controls stay on it instead of jumping back to Spotify.
fn pick_player(conn: &Connection, last: &mut Option<String>) -> Option<String> {
    let names = DBusProxy::new(conn).ok()?.list_names().ok()?;
    let players: Vec<String> = names
        .iter()
        .map(|n| n.to_string())
        .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
        .collect();
    let status = |p: &str| -> String {
        player_proxy(conn, p)
            .and_then(|px| px.get_property::<String>("PlaybackStatus").ok())
            .unwrap_or_default()
    };
    let playing: Vec<&String> = players.iter().filter(|p| status(p) == "Playing").collect();
    if let Some(p) = playing.iter().find(|p| p.contains("spotify")).or(playing.first()) {
        *last = Some(p.to_string());
        return Some(p.to_string());
    }
    if let Some(p) = last.as_ref().filter(|l| players.contains(l)) {
        return Some(p.clone());
    }
    players.iter().find(|p| p.contains("spotify")).or(players.first()).cloned()
}

fn poll(conn: &Connection, track: &Mutex<Track>, last: &mut Option<String>) {
    let Some(name) = pick_player(conn, last) else {
        *track.lock().unwrap() = Track::default();
        return;
    };
    let Some(px) = player_proxy(conn, &name) else { return };

    let md: HashMap<String, OwnedValue> = px.get_property("Metadata").unwrap_or_default();
    let status: String = px.get_property("PlaybackStatus").unwrap_or_default();
    let pos: i64 = px.get_property("Position").unwrap_or(0);
    let mut t = track.lock().unwrap();
    t.player = name.clone();
    t.title = md_str(&md, "xesam:title");
    t.artist = md_strs(&md, "xesam:artist");
    t.album = md_str(&md, "xesam:album");
    t.length = md_len(&md);
    t.position = pos as f64 / 1e6;
    t.status = status;
    t.at = Some(Instant::now());
}

/// The lockers that answer GetActive: KDE's, then GNOME's. Wayland lockers
/// like hyprlock and swaylock aren't on the bus; their launcher recipes stop
/// glyphwave before locking instead.
const LOCKERS: [(&str, &str); 2] = [
    ("org.freedesktop.ScreenSaver", "/ScreenSaver"),
    ("org.gnome.ScreenSaver", "/org/gnome/ScreenSaver"),
];

fn locker_active(conn: &Connection) -> bool {
    LOCKERS.iter().any(|&(name, path)| {
        conn.call_method(Some(name), path, Some(name), "GetActive", &())
            .ok()
            .and_then(|m| m.body().deserialize::<bool>().ok())
            .unwrap_or(false)
    })
}

/// Ask the bus for the signals that mean "re-read": a player's properties or
/// position changing, a player appearing or leaving, the locker toggling.
fn subscribe(conn: &Connection, watch_locker: bool) -> zbus::Result<()> {
    let bus = DBusProxy::new(conn)?;
    let sig = || MatchRule::builder().msg_type(Type::Signal);
    bus.add_match_rule(
        sig().interface("org.freedesktop.DBus.Properties")?.member("PropertiesChanged")?.path(PATH)?.arg(0, PLAYER)?.build(),
    )?;
    bus.add_match_rule(sig().interface(PLAYER)?.member("Seeked")?.path(PATH)?.build())?;
    bus.add_match_rule(
        sig()
            .sender("org.freedesktop.DBus")?
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .arg0ns("org.mpris.MediaPlayer2")?
            .build(),
    )?;
    if watch_locker {
        for (name, _) in LOCKERS {
            bus.add_match_rule(sig().interface(name)?.member("ActiveChanged")?.build())?;
        }
    }
    Ok(())
}

impl Watcher {
    pub fn start(watch_locker: bool) -> Watcher {
        let track = Arc::new(Mutex::new(Track::default()));
        let locked = Arc::new(AtomicBool::new(false));
        let conn = Connection::session().ok();
        if let Some(conn) = conn.clone() {
            // The reader only forwards signals and never calls the bus itself, so it
            // always drains the connection while the poller waits on replies.
            let (tx, rx) = mpsc::channel::<()>();
            let msgs = MessageIterator::from(&conn);
            let subscribed = subscribe(&conn, watch_locker).is_ok();
            let l = locked.clone();
            std::thread::spawn(move || {
                for m in msgs.flatten() {
                    if m.message_type() != Type::Signal {
                        continue;
                    }
                    if m.header().member().is_some_and(|n| n == "ActiveChanged") {
                        if m.body().deserialize::<bool>().unwrap_or(false) {
                            l.store(true, Ordering::Relaxed);
                        }
                    } else if tx.send(()).is_err() {
                        return;
                    }
                }
            });
            // Without signals fall back to polling each second; with them, a slow
            // recheck covers a player that forgets to announce something.
            let every = Duration::from_secs(if subscribed { 30 } else { 1 });
            let (t, l) = (track.clone(), locked.clone());
            let mut last = None;
            std::thread::spawn(move || loop {
                poll(&conn, &t, &mut last);
                if watch_locker && locker_active(&conn) {
                    l.store(true, Ordering::Relaxed);
                }
                if let Err(mpsc::RecvTimeoutError::Disconnected) = rx.recv_timeout(every) {
                    std::thread::sleep(every);
                }
                while rx.try_recv().is_ok() {} // one re-read covers a burst
            });
        }
        Watcher { track, locked, conn }
    }

    pub fn snapshot(&self) -> Track {
        self.track.lock().unwrap().clone()
    }

    /// PlayPause / Next / Previous on the current player (interactive mode keys).
    /// The player's PropertiesChanged signal brings the new state back.
    pub fn control(&self, method: &str) {
        let (Some(conn), name) = (&self.conn, self.snapshot().player) else { return };
        if name.is_empty() {
            return;
        }
        let _ = conn.call_method(Some(name.as_str()), PATH, Some(PLAYER), method, &());
    }

    /// Pause `name` later, from another thread (the fade before a call).
    /// `Pause` or `Play` for `name`, to call later from another thread.
    pub fn later(&self, name: String, method: &'static str) -> impl FnOnce() + Send + 'static {
        let conn = self.conn.clone();
        move || {
            if let Some(c) = conn {
                let _ = c.call_method(Some(name.as_str()), PATH, Some(PLAYER), method, &());
            }
        }
    }
}
