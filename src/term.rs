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

pub fn stdin_is_tty() -> bool {
    unsafe { libc::isatty(STDIN) == 1 }
}

pub fn write_all(b: &[u8]) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(b);
    let _ = out.flush();
}
