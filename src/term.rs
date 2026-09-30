//! Raw terminal: alternate screen, hidden cursor, optional mouse-motion
//! reporting (so moving the mouse wakes the screensaver), non-blocking input.

use std::io::Write;
use std::os::fd::RawFd;

const STDIN: RawFd = 0;
const STDOUT: RawFd = 1;

pub struct Term {
    saved: Option<libc::termios>,
    mouse: bool,
}

impl Term {
    pub fn enter(mouse: bool) -> Term {
        let saved = unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(STDIN, &mut t) == 0 {
                let orig = t;
                t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::IEXTEN);
                t.c_iflag &= !(libc::IXON | libc::ICRNL);
                t.c_cc[libc::VMIN] = 0;
                t.c_cc[libc::VTIME] = 0;
                libc::tcsetattr(STDIN, libc::TCSANOW, &t);
                Some(orig)
            } else {
                None
            }
        };
        let mut s = String::from("\x1b[?1049h\x1b[?25l\x1b[?7l\x1b]11;rgb:00/00/00\x07\x1b[0m\x1b[2J");
        if mouse {
            s.push_str("\x1b[?1003h\x1b[?1006h");
        }
        write_all(s.as_bytes());
        Term { saved, mouse }
    }

    pub fn restore(&mut self) {
        let mut s = String::new();
        if self.mouse {
            s.push_str("\x1b[?1006l\x1b[?1003l");
        }
        s.push_str("\x1b[0m\x1b]111\x07\x1b[?7h\x1b[?25h\x1b[?1049l");
        write_all(s.as_bytes());
        if let Some(t) = self.saved.take() {
            unsafe { libc::tcsetattr(STDIN, libc::TCSANOW, &t) };
        }
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Restore the terminal from a panic hook, where the Term value can't be reached.
pub fn emergency_restore() {
    write_all(b"\x1b[?1006l\x1b[?1003l\x1b[0m\x1b]111\x07\x1b[?7h\x1b[?25h\x1b[?1049l");
}

pub fn size() -> (usize, usize) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(STDOUT, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
            (ws.ws_col as usize, ws.ws_row as usize)
        } else {
            (80, 24)
        }
    }
}

/// Wait up to `timeout_ms` for input; returns whatever bytes arrived.
/// With `stdin` false (not a tty, e.g. /dev/null) this is just a sleep.
pub fn poll_input(timeout_ms: i32, stdin: bool, buf: &mut Vec<u8>) {
    buf.clear();
    let mut fds = libc::pollfd { fd: STDIN, events: libc::POLLIN, revents: 0 };
    let r = unsafe { libc::poll(&mut fds, stdin as libc::nfds_t, timeout_ms.max(0)) };
    if r > 0 && fds.revents & libc::POLLIN != 0 {
        let mut tmp = [0u8; 256];
        let n = unsafe { libc::read(STDIN, tmp.as_mut_ptr() as *mut _, tmp.len()) };
        if n > 0 {
            buf.extend_from_slice(&tmp[..n as usize]);
        }
    }
}

/// The keys that drive the player without waking the screensaver: - previous,
/// + next, Enter play/pause. Terminals send the numpad ones as the same bytes.
pub fn is_media(b: u8) -> bool {
    matches!(b, b'-' | b'+' | b'\r' | b'\n')
}

/// Whether screensaver input should wake the screen. Everything does except
/// the music keys and function keys: the media keys live on them, and with
/// Fn-lock the other way round a press arrives as F1–F12 instead of reaching
/// the desktop.
pub fn wakes(mut b: &[u8]) -> bool {
    while !b.is_empty() {
        if is_media(b[0]) {
            b = &b[1..];
            continue;
        }
        match fkey_len(b) {
            Some(n) => b = &b[n..],
            None => return true,
        }
    }
    false
}

/// Length of the function-key sequence at the start of `b`, if there is one:
/// ESC O P..S, ESC [ 1;m P..S (F1–F4), ESC [ n(;m) ~ for F1–F12 in the xterm
/// and rxvt numbering, ESC [ [ A..E (Linux console).
fn fkey_len(b: &[u8]) -> Option<usize> {
    match b {
        [0x1b, b'O', b'P'..=b'S', ..] => return Some(3),
        [0x1b, b'[', b'[', b'A'..=b'E', ..] => return Some(4),
        [0x1b, b'[', b'1', b';', b'0'..=b'9', b'P'..=b'S', ..] => return Some(6),
        [0x1b, b'[', rest @ ..] => {
            let end = rest.iter().position(|&c| !(c.is_ascii_digit() || c == b';'))?;
            let n: u32 = std::str::from_utf8(&rest[..end]).ok()?.split(';').next()?.parse().ok()?;
            let fkey = matches!(n, 11..=15 | 17..=21 | 23 | 24);
            return (rest[end] == b'~' && fkey).then_some(end + 3);
        }
        _ => None,
    }
}

/// Whether the terminal takes 24-bit colour. Most say so in COLORTERM; a few
/// only in TERM. Anything else (tmux without it passed through, urxvt, the
/// Linux console) gets the 256-colour palette.
pub fn truecolor() -> bool {
    let ct = std::env::var("COLORTERM").unwrap_or_default();
    if matches!(ct.as_str(), "truecolor" | "24bit") {
        return true;
    }
    let t = std::env::var("TERM").unwrap_or_default();
    t.ends_with("-direct")
        || ["xterm-kitty", "alacritty", "foot", "wezterm", "xterm-ghostty", "contour"].iter().any(|p| t.starts_with(p))
}

pub fn stdin_is_tty() -> bool {
    unsafe { libc::isatty(STDIN) == 1 }
}

pub fn write_all(b: &[u8]) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(b);
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::wakes;

    #[test]
    fn function_and_music_keys_do_not_wake() {
        for k in [&b"-"[..], b"+", b"\r", b"+\r-", b"\x1bOP", b"\x1bOS", b"\x1b[1;2Q", b"\x1b[15~", b"\x1b[24;5~", b"\x1b[11~", b"\x1b[[A", b"\x1bOP\x1b[17~"] {
            assert!(!wakes(k), "{k:?}");
        }
    }

    #[test]
    fn everything_else_wakes() {
        for k in [&b"a"[..], b" ", b"\x1b", b"\x03", b"\x1b[A", b"\x1b[3~", b"\x1b[5~", b"\x1b[<35;10;4M", b"\x1b[15~x", b"-a", b"="] {
            assert!(wakes(k), "{k:?}");
        }
    }
}
