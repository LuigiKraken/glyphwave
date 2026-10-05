//! Raw terminal: alternate screen, hidden cursor, optional mouse-motion
//! reporting (so moving the mouse wakes the screensaver), non-blocking input.

use std::io::Write;
use std::os::fd::RawFd;

const STDIN: RawFd = 0;
const STDOUT: RawFd = 1;

pub struct Term {
    /// where it reads keys and writes frames: stdin and stdout, or the
    /// terminal of another screen
    pub fd_in: RawFd,
    pub fd: RawFd,
    saved: Option<libc::termios>,
    mouse: bool,
}

impl Term {
    pub fn enter(mouse: bool) -> Term {
        Term::on(STDIN, STDOUT, mouse)
    }

    pub fn on(fd_in: RawFd, fd: RawFd, mouse: bool) -> Term {
        let saved = unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd_in, &mut t) == 0 {
                let orig = t;
                t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::IEXTEN);
                if fd_in != libc::STDIN_FILENO {
                    // another screen's terminal: Ctrl+C or Ctrl+Z there is a key that wakes it
                    t.c_lflag &= !libc::ISIG;
                }
                t.c_iflag &= !(libc::IXON | libc::ICRNL);
                t.c_cc[libc::VMIN] = 0;
                t.c_cc[libc::VTIME] = 0;
                libc::tcsetattr(fd_in, libc::TCSANOW, &t);
                Some(orig)
            } else {
                None
            }
        };
        let mut s = String::from("\x1b[?1049h\x1b[?25l\x1b[?7l\x1b]11;rgb:00/00/00\x07\x1b[0m\x1b[2J");
        if mouse {
            s.push_str("\x1b[?1003h\x1b[?1006h");
        }
        write_to(fd, s.as_bytes());
        Term { fd_in, fd, saved, mouse }
    }

    pub fn write(&self, b: &[u8]) {
        write_to(self.fd, b);
    }

    pub fn restore(&mut self) {
        let mut s = String::new();
        if self.mouse {
            s.push_str("\x1b[?1006l\x1b[?1003l");
        }
        s.push_str("\x1b[0m\x1b]111\x07\x1b[?7h\x1b[?25h\x1b[?1049l");
        write_to(self.fd, s.as_bytes());
        if let Some(t) = self.saved.take() {
            unsafe { libc::tcsetattr(self.fd_in, libc::TCSANOW, &t) };
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
    size_of(STDOUT)
}

pub fn size_of(fd: RawFd) -> (usize, usize) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
            (ws.ws_col as usize, ws.ws_row as usize)
        } else {
            (80, 24)
        }
    }
}

/// Wait up to `timeout_ms` for input on any of `fds` (stdin, and the other
/// screens' terminals); whatever arrived goes into `buf`. With no fds (stdin
/// isn't a tty, e.g. /dev/null) this is just a sleep. True when one of the
/// other terminals went away: its window closed.
pub fn poll_input(timeout_ms: i32, fds: &[RawFd], buf: &mut Vec<u8>) -> bool {
    buf.clear();
    let mut p: Vec<libc::pollfd> = fds.iter().map(|&fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 }).collect();
    let r = unsafe { libc::poll(p.as_mut_ptr(), p.len() as libc::nfds_t, timeout_ms.max(0)) };
    let mut gone = false;
    for q in p.iter().filter(|_| r > 0) {
        if q.revents & libc::POLLIN != 0 {
            let mut tmp = [0u8; 256];
            let n = unsafe { libc::read(q.fd, tmp.as_mut_ptr() as *mut _, tmp.len()) };
            if n > 0 {
                buf.extend_from_slice(&tmp[..n as usize]);
                continue;
            }
        }
        gone |= q.fd != STDIN && q.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0;
    }
    gone
}

/// The keys that drive the player without waking the screensaver: - previous,
/// + next, Enter play/pause. Terminals send the numpad ones as the same bytes.
pub fn is_media(b: u8) -> bool {
    matches!(b, b'-' | b'+' | b'\r' | b'\n')
}

/// Whether screensaver input should wake the screen. Everything does except
/// the music keys and function keys: the media keys live on them, and with
/// Fn-lock the other way round a press arrives as F1–F12 instead of reaching
/// the desktop. Mouse motion wakes unless it's in the cell of a report that
/// came while `settling`, kept in `pointer`: Ptyxis reports one motion where
/// the pointer stands soon after its window goes fullscreen, with no movement.
pub fn wakes(mut b: &[u8], pointer: &mut Option<(u16, u16)>, settling: bool) -> bool {
    while !b.is_empty() {
        if is_media(b[0]) {
            b = &b[1..];
            continue;
        }
        if let Some((n, cell)) = motion(b) {
            if *pointer != Some(cell) && !(settling && pointer.is_none()) {
                return true;
            }
            *pointer = Some(cell);
            b = &b[n..];
            continue;
        }
        match fkey_len(b) {
            Some(n) => b = &b[n..],
            None => return true,
        }
    }
    false
}

/// An SGR mouse-motion report at the start of `b` (ESC [ < button;col;row M
/// with the motion bit, 32, in the button): its length and cell. Clicks and
/// the wheel aren't motion.
fn motion(b: &[u8]) -> Option<(usize, (u16, u16))> {
    let rest = b.strip_prefix(b"\x1b[<")?;
    let end = rest.iter().position(|&c| !(c.is_ascii_digit() || c == b';'))?;
    let mut f = std::str::from_utf8(&rest[..end]).ok()?.split(';').map(|n| n.parse::<u16>().ok());
    let (btn, col, row) = (f.next()??, f.next()??, f.next()??);
    (rest[end] == b'M' && btn & 32 != 0 && btn & 64 == 0).then_some((end + 4, (col, row)))
}

/// Length of the function-key sequence at the start of `b`, if there is one:
/// ESC O P..S, ESC [ 1;m P..S (F1–F4), ESC [ n(;m) ~ for F1–F12 in the xterm
/// and rxvt numbering, ESC [ [ A..E (Linux console).
fn fkey_len(b: &[u8]) -> Option<usize> {
    match b {
        [0x1b, b'O', b'P'..=b'S', ..] => Some(3),
        [0x1b, b'[', b'[', b'A'..=b'E', ..] => Some(4),
        [0x1b, b'[', b'1', b';', b'0'..=b'9', b'P'..=b'S', ..] => Some(6),
        [0x1b, b'[', rest @ ..] => {
            let end = rest.iter().position(|&c| !(c.is_ascii_digit() || c == b';'))?;
            let n: u32 = std::str::from_utf8(&rest[..end]).ok()?.split(';').next()?.parse().ok()?;
            let fkey = matches!(n, 11..=15 | 17..=21 | 23 | 24);
            (rest[end] == b'~' && fkey).then_some(end + 3)
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
    write_to(STDOUT, b);
}

pub fn write_to(fd: RawFd, mut b: &[u8]) {
    if fd == STDOUT {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(b);
        let _ = out.flush();
        return;
    }
    while !b.is_empty() {
        let n = unsafe { libc::write(fd, b.as_ptr().cast(), b.len()) };
        if n <= 0 {
            return; // gone; the poll notices
        }
        b = &b[n as usize..];
    }
}

#[cfg(test)]
mod tests {
    use super::{Term, poll_input, wakes};

    #[test]
    fn another_screens_terminal() {
        let (mut m, mut sl) = (0, 0);
        let n = std::ptr::null_mut();
        assert_eq!(unsafe { libc::openpty(&mut m, &mut sl, n, std::ptr::null(), std::ptr::null()) }, 0);
        let t = Term::on(sl, sl, true); // raw, as glyphwave sets it
        let mut buf = Vec::new();
        // a key on it arrives like one on stdin
        unsafe { libc::write(m, b"x".as_ptr().cast(), 1) };
        assert!(!poll_input(100, &[sl], &mut buf));
        assert_eq!(buf, b"x");
        // nothing: just the wait
        assert!(!poll_input(10, &[sl], &mut buf));
        assert!(buf.is_empty());
        // its window closed
        unsafe { libc::close(m) };
        assert!(poll_input(100, &[sl], &mut buf));
        drop(t);
        unsafe { libc::close(sl) };
    }

    #[test]
    fn function_and_music_keys_do_not_wake() {
        for k in [&b"-"[..], b"+", b"\r", b"+\r-", b"\x1bOP", b"\x1bOS", b"\x1b[1;2Q", b"\x1b[15~", b"\x1b[24;5~", b"\x1b[11~", b"\x1b[[A", b"\x1bOP\x1b[17~"] {
            assert!(!wakes(k, &mut None, false), "{k:?}");
        }
    }

    #[test]
    fn everything_else_wakes() {
        let clicks = [&b"\x1b[<0;10;4M"[..], b"\x1b[<0;10;4m", b"\x1b[<64;10;4M", b"\x1b[<65;10;4M", b"\x1b[<35;10;4M\x1b[<0;10;4M"];
        for k in [&b"a"[..], b" ", b"\x1b", b"\x03", b"\x1b[A", b"\x1b[3~", b"\x1b[5~", b"\x1b[15~x", b"-a", b"="].into_iter().chain(clicks) {
            assert!(wakes(k, &mut None, false), "{k:?}");
        }
    }

    #[test]
    fn motion_wakes_once_the_pointer_moves() {
        // Ptyxis: one report where the pointer already stands, soon after it opens
        let mut p = None;
        assert!(!wakes(b"\x1b[<35;80;22M", &mut p, true));
        assert!(!wakes(b"\x1b[<35;80;22M", &mut p, false));
        assert!(wakes(b"\x1b[<35;81;22M", &mut p, false));
        // a move while it settles reports more than one cell
        assert!(wakes(b"\x1b[<35;10;4M\x1b[<35;11;4M", &mut None, true));
        let mut p = None;
        assert!(!wakes(b"\x1b[<35;10;4M", &mut p, true));
        assert!(wakes(b"\x1b[<35;10;5M", &mut p, true));
        // after that, one report is enough (Konsole sends one per pointer jump)
        assert!(wakes(b"\x1b[<35;10;4M", &mut None, false));
        // a motion and a key
        assert!(wakes(b"\x1b[<35;10;4Mx", &mut None, true));
    }
}
