//! Incoming calls, read passively off the session bus: a second connection
//! turns itself into a bus monitor (BecomeMonitor, which any session client
//! may do) for the desktop's call notifications: `Notify` on
//! org.freedesktop.Notifications, and `AddNotification` on org.gtk.Notifications,
//! where GNOME's portal sends Flatpak apps'. Nothing is answered or declined;
//! main draws the caller and lets any key end the screensaver. The monitor
//! only wakes on notification traffic.
//!
//! A call is a notification with the category `call` / `call.incoming`, or
//! one from a known call app whose text reads like a ring ("Incoming call",
//! "… is calling you", "… invited you to a huddle"). It rings until the app
//! closes or replaces the notification, or for `RING` at most.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;
use zbus::zvariant::OwnedValue;

const RING: Duration = Duration::from_secs(45);
const FDO: &str = "org.freedesktop.Notifications";
const GTK: &str = "org.gtk.Notifications";

#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub app: String,
    pub caller: String,
    until: Instant,
    /// The notification it came in: its org.freedesktop id (0 until the
    /// server's reply names it), or the org.gtk app id and id.
    id: u32,
    gtk: Option<(String, String)>,
}

impl Call {
    pub fn ringing(&self, now: Instant) -> bool {
        now < self.until
    }
}

pub struct Calls {
    ring: Arc<Slot>,
}

/// Lowercase key in an app name or desktop entry → the name shown.
const APPS: [(&str, &str); 9] = [
    ("slack", "Slack"),
    ("discord", "Discord"),
    ("teams", "Teams"),
    ("whatsapp", "WhatsApp"),
    ("zapzap", "WhatsApp"),
    ("zoom", "Zoom"),
    ("signal", "Signal"),
    ("telegram", "Telegram"),
    ("skype", "Skype"),
];

/// Phrases that mark a notification from a call app as a ring.
const RINGS: [&str; 8] = [
    "incoming call",
    "incoming video",
    "incoming voice",
    "incoming audio",
    "is calling",
    "calling you",
    "to a huddle",
    "you to huddle",
];

/// Wrapped around the caller's name in those phrases; cut to leave the name.
const AROUND: [&str; 9] = [
    "incoming video call from ",
    "incoming voice call from ",
    "incoming call from ",
    " is calling you",
    " is calling",
    " is inviting you to a huddle",
    " invited you to a huddle",
    " invited you to huddle",
    " calling you",
];

/// Notification text is a markup subset: drop the tags and the entities.
fn plain(s: &str) -> String {
    let mut o = String::new();
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => o.push(c),
            _ => {}
        }
    }
    let o = o.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&");
    o.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

fn cut_around(s: &str) -> String {
    let mut s = s.to_string();
    for p in AROUND {
        if let Some(i) = s.to_lowercase().find(p) {
            // the phrases are ASCII, so lowercasing kept the byte offsets
            s.replace_range(i..i + p.len(), "");
        }
    }
    s.trim().trim_end_matches(['.', '!', ':']).trim().to_string()
}

/// Whether a notification is a ringing call, and if so the app and caller
/// to show. `app` is the sender's app name (or GTK app id), `entry` its
/// desktop-entry hint.
pub fn detect(app: &str, entry: &str, category: &str, summary: &str, body: &str) -> Option<(String, String)> {
    let (summary, body) = (plain(summary), plain(body));
    let text = format!("{summary}\n{body}").to_lowercase();
    let cat = category.to_lowercase();
    let ids = format!("{app} {entry}").to_lowercase();
    let known = APPS.iter().find(|(k, _)| ids.contains(k)).map(|&(_, n)| n);
    let rings = if cat.starts_with("call") {
        // call.ended, call.unanswered, call.ongoing: not ringing (any more)
        cat == "call" || cat == "call.incoming"
    } else {
        known.is_some() && !text.contains("missed") && RINGS.iter().any(|p| text.contains(p))
    };
    if !rings {
        return None;
    }
    let name = known.map(str::to_string).unwrap_or_else(|| {
        let a = app.rsplit('.').next().unwrap_or(app).trim();
        if a.is_empty() { "Call".into() } else { a.to_string() }
    });
    // "Incoming call" over the name, or the name over "Incoming call", or
    // "Name is calling you"
    let s = cut_around(&summary);
    let generic = s.is_empty() || ["incoming", "call", "huddle"].iter().any(|w| s.to_lowercase().contains(w));
    let caller = if generic && !body.is_empty() { cut_around(&body) } else { s };
    Some((name, caller))
}

type Hints = HashMap<String, OwnedValue>;

fn hint(h: &Hints, k: &str) -> String {
    h.get(k).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default()
}

/// The ring, and a counter bumped on every change so main only has to
/// look at an atomic each frame.
#[derive(Default)]
struct Slot {
    call: Mutex<Option<Call>>,
    changes: AtomicU32,
}

fn ring(slot: &Slot, (app, caller): (String, String), id: u32, gtk: Option<(String, String)>, len: Duration) {
    *slot.call.lock().unwrap() = Some(Call { app, caller, until: Instant::now() + len, id, gtk });
    slot.changes.fetch_add(1, Ordering::Release);
}

/// End the ring if `is` says the message is about its notification.
fn end_if(slot: &Slot, is: impl Fn(&Call) -> bool) {
    let mut g = slot.call.lock().unwrap();
    if g.as_ref().is_some_and(is) {
        *g = None;
        slot.changes.fetch_add(1, Ordering::Release);
    }
}

fn watch(conn: Connection, slot: Arc<Slot>) {
    // the Notify call that rang, until the server's reply gives its id
    let mut pending: Option<(String, u32)> = None;
    for m in MessageIterator::from(&conn).flatten() {
        let h = m.header();
        let member = h.member().map(|n| n.as_str()).unwrap_or("");
        let iface = h.interface().map(|n| n.as_str()).unwrap_or("");
        let body = m.body();
        match (m.message_type(), iface, member) {
            (Type::MethodCall, FDO, "Notify") => {
                type Notify = (String, u32, String, String, String, Vec<String>, Hints, i32);
                let Ok((app, replaces, _, summary, text, _, hints, _)) = body.deserialize::<Notify>() else { continue };
                match detect(&app, &hint(&hints, "desktop-entry"), &hint(&hints, "category"), &summary, &text) {
                    Some(c) => {
                        ring(&slot, c, replaces, None, RING);
                        pending = h.sender().map(|s| (s.to_string(), h.primary().serial_num().get()));
                    }
                    // the ring's notification turned into "missed call" or the like
                    None => end_if(&slot, |c| replaces != 0 && c.id == replaces),
                }
            }
            (Type::MethodReturn, ..) => {
                let to = h.destination().map(|d| d.to_string()).unwrap_or_default();
                let serial = h.reply_serial().map(|s| s.get()).unwrap_or(0);
                if pending.as_ref().is_some_and(|(s, n)| *s == to && *n == serial) {
                    pending = None;
                    if let (Ok(id), Some(c)) = (body.deserialize::<u32>(), slot.call.lock().unwrap().as_mut()) {
                        c.id = id;
                    }
                }
            }
            (Type::MethodCall, FDO, "CloseNotification") | (Type::Signal, FDO, "NotificationClosed") => {
                let id = body.deserialize::<u32>().or_else(|_| body.deserialize::<(u32, u32)>().map(|t| t.0)).unwrap_or(0);
                end_if(&slot, |c| id != 0 && c.id == id);
            }
            (Type::MethodCall, GTK, "AddNotification") => {
                let Ok((app, id, n)) = body.deserialize::<(String, String, Hints)>() else { continue };
                let key = Some((app.clone(), id));
                match detect(&app, &app, &hint(&n, "category"), &hint(&n, "title"), &hint(&n, "body")) {
                    Some(c) => ring(&slot, c, 0, key, RING),
                    None => end_if(&slot, |c| c.gtk == key),
                }
            }
            (Type::MethodCall, GTK, "RemoveNotification") => {
                let Ok(key) = body.deserialize::<(String, String)>() else { continue };
                end_if(&slot, |c| c.gtk.as_ref() == Some(&key));
            }
            _ => {}
        }
    }
}

impl Calls {
    pub fn start() -> Calls {
        let ring = Arc::new(Slot::default());
        // its own connection: a monitor may not send anything any more
        if let Ok(conn) = Connection::session() {
            let rules = [
                "type='method_call',interface='org.freedesktop.Notifications'",
                "type='signal',interface='org.freedesktop.Notifications',member='NotificationClosed'",
                "type='method_return',sender='org.freedesktop.Notifications'",
                "type='method_call',interface='org.gtk.Notifications'",
            ];
            let ok = conn
                .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus.Monitoring"), "BecomeMonitor", &(&rules[..], 0u32))
                .is_ok();
            if ok {
                let slot = ring.clone();
                std::thread::spawn(move || watch(conn, slot));
            }
        }
        Calls { ring }
    }

    /// Bring `call` up to date: a new ring is copied in (the only time this
    /// allocates); an ended one keeps its text for the fade-out but stops
    /// ringing. `seen` is the change counter last looked at.
    pub fn sync(&self, seen: &mut u32, call: &mut Option<Call>) {
        let changes = self.ring.changes.load(Ordering::Acquire);
        if changes == *seen {
            return;
        }
        *seen = changes;
        match self.ring.call.lock().unwrap().as_ref() {
            Some(c) => *call = Some(c.clone()),
            None => {
                if let Some(c) = call {
                    c.until = c.until.min(Instant::now());
                }
            }
        }
    }

    /// `--test`: ring as if `app` had sent this notification, for 5 s, then
    /// stop as a missed call does.
    pub fn fake(&self, app: &str, summary: &str, body: &str) {
        if let Some(c) = detect(app, "", "", summary, body) {
            ring(&self.ring, c, 0, None, Duration::from_secs(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::detect;

    fn d(app: &str, cat: &str, s: &str, b: &str) -> Option<(String, String)> {
        detect(app, "", cat, s, b)
    }

    #[test]
    fn rings() {
        let c = |a: &str, n: &str| Some((a.to_string(), n.to_string()));
        assert_eq!(d("Slack", "", "Sam Rivera invited you to a huddle", ""), c("Slack", "Sam Rivera"));
        assert_eq!(d("discord", "", "pixelfox", "Incoming Call"), c("Discord", "pixelfox"));
        assert_eq!(d("Microsoft Teams", "", "Morgan Lee is calling you", ""), c("Teams", "Morgan Lee"));
        assert_eq!(d("ZapZap", "", "Incoming voice call", "+49 151 2345 6789"), c("WhatsApp", "+49 151 2345 6789"));
        assert_eq!(d("Phone", "call.incoming", "<b>Incoming call from Ana &amp; Bo</b>", ""), c("Phone", "Ana & Bo"));
        assert_eq!(d("com.example.Dialer", "call", "Ana", "ringing"), c("Dialer", "Ana"));
    }

    #[test]
    fn does_not_ring() {
        assert_eq!(d("Slack", "", "Sam Rivera", "can you call me later?"), None);
        assert_eq!(d("Slack", "", "Missed call", "Sam Rivera is calling you"), None);
        assert_eq!(d("Thunderbird", "", "Incoming call", "from a newsletter"), None);
        assert_eq!(d("Phone", "call.ended", "Ana", "Call ended"), None);
        assert_eq!(d("Phone", "call.unanswered", "Ana", "Incoming call"), None);
    }
}
