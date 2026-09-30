//! Session-bus watcher: the now-playing track from any MPRIS player (Spotify
//! preferred, then whatever is playing), and — in screensaver mode — whether the
//! KDE locker has taken over. Polled once a second on its own thread.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy, fdo::DBusProxy, proxy::Builder};
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Clone, Default, Debug)]
pub struct Track {
    pub player: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art_url: String,
    pub length: f64,
    pub position: f64,
    pub status: String,
    pub at: Option<Instant>,
    /// Bumps whenever the track (player, title or art) changes.
    pub version: u64,
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

fn pick_player(conn: &Connection) -> Option<String> {
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
    if let Some(p) = playing.iter().find(|p| p.contains("spotify")) {
        return Some(p.to_string());
    }
    if let Some(p) = playing.first() {
        return Some(p.to_string());
    }
    players.iter().find(|p| p.contains("spotify")).or(players.first()).cloned()
}

fn poll(conn: &Connection, track: &Mutex<Track>) {
    let Some(name) = pick_player(conn) else {
        let mut t = track.lock().unwrap();
        if !t.player.is_empty() {
            let v = t.version + 1;
            *t = Track { version: v, ..Default::default() };
        }
        return;
    };
    let Some(px) = player_proxy(conn, &name) else { return };

    let md: HashMap<String, OwnedValue> = px.get_property("Metadata").unwrap_or_default();
    let status: String = px.get_property("PlaybackStatus").unwrap_or_default();
    let pos: i64 = px.get_property("Position").unwrap_or(0);
    let title = md_str(&md, "xesam:title");
    let art = md_str(&md, "mpris:artUrl");
    let mut t = track.lock().unwrap();
    if (name.as_str(), title.as_str(), art.as_str())
        != (t.player.as_str(), t.title.as_str(), t.art_url.as_str())
    {
        t.version += 1;
    }
    t.player = name.clone();
    t.title = title;
    t.art_url = art;
    t.artist = md_strs(&md, "xesam:artist");
    t.album = md_str(&md, "xesam:album");
    t.length = md_len(&md);
    t.position = pos as f64 / 1e6;
    t.status = status;
    t.at = Some(Instant::now());
}

fn locker_active(conn: &Connection) -> bool {
    conn.call_method(
        Some("org.freedesktop.ScreenSaver"),
        "/ScreenSaver",
        Some("org.freedesktop.ScreenSaver"),
        "GetActive",
        &(),
    )
    .ok()
    .and_then(|m| m.body().deserialize::<bool>().ok())
    .unwrap_or(false)
}

impl Watcher {
    pub fn start(watch_locker: bool) -> Watcher {
        let track = Arc::new(Mutex::new(Track::default()));
        let locked = Arc::new(AtomicBool::new(false));
        let conn = Connection::session().ok();
        if let Some(conn) = conn.clone() {
            let (t, l) = (track.clone(), locked.clone());
            std::thread::spawn(move || loop {
                poll(&conn, &t);
                if watch_locker && locker_active(&conn) {
                    l.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(1000));
            });
        }
        Watcher { track, locked, conn }
    }

    pub fn snapshot(&self) -> Track {
        self.track.lock().unwrap().clone()
    }

    /// PlayPause / Next / Previous on the current player (interactive mode keys).
    pub fn control(&self, method: &str) {
        let (Some(conn), name) = (&self.conn, self.snapshot().player) else { return };
        if name.is_empty() {
            return;
        }
        let _ = conn.call_method(Some(name.as_str()), PATH, Some(PLAYER), method, &());
        if let Some(conn) = self.conn.clone() {
            let t = self.track.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                poll(&conn, &t);
            });
        }
    }
}
