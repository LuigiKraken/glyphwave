//! Where the banner comes from: the system logo (what fastfetch or
//! neofetch shows), the machine's name in big block letters, or the user's
//! own words in the same letters, or own art. `--banner` and the config
//! file's `banner =` take the same forms, `logo`, `name`, `text:<words>` or
//! a path, and both go through `resolve`.
//!
//! Every source comes back as plain text: colours are stripped, since the
//! banner wears glyphwave's own palette.

use std::fmt;
use std::process::{Command, Stdio};

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Logo,
    Name,
    /// own words, in the name's big letters
    Text(String),
    File(String),
    /// `--test`'s stand-in for own art when there's no banner.txt.
    Sample,
}

/// The sample own art `l` shows in `--test` when there's no banner.txt.
const SAMPLE: &str = "\
+--------------------------------------+
|                                      |
|    ~~~   your own art goes here  ~~~ |
|                                      |
|    ~/.config/glyphwave/banner.txt    |
|                                      |
+--------------------------------------+";

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Source::Logo => f.write_str("logo"),
            Source::Name => f.write_str("name"),
            Source::Text(t) => write!(f, "text:{t}"),
            Source::File(p) => f.write_str(p),
            Source::Sample => f.write_str("sample"),
        }
    }
}

impl Source {
    /// `logo`, `name`, `text:<words>`, or a path (a leading `~/` is the home
    /// directory).
    pub fn parse(spec: &str) -> Source {
        match spec.trim() {
            "logo" => Source::Logo,
            "name" => Source::Name,
            t if t.starts_with("text:") => Source::Text(t[5..].trim().to_string()),
            p => match p.strip_prefix("~/") {
                Some(rest) => Source::File(format!("{}/{rest}", home())),
                None => Source::File(p.to_string()),
            },
        }
    }

    /// The art, or None when there's nothing to show (no fetch tool, a
    /// missing file, only blanks). The logo runs fastfetch or neofetch.
    pub fn text(&self) -> Option<String> {
        let t = match self {
            Source::Logo => run("fastfetch", &["-s", "none", "--pipe", "false"]).or_else(|| run("neofetch", &["-L"])),
            Source::Name => Some(big(&hostname())),
            Source::Text(t) => Some(big(t)),
            Source::File(p) => std::fs::read_to_string(p).ok().map(|s| strip_ansi(&s)),
            Source::Sample => Some(SAMPLE.to_string()),
        };
        t.filter(|t| t.chars().any(|c| !c.is_whitespace()))
    }
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// The user's own art, used without any flag when it exists.
pub fn own_banner() -> String {
    crate::config::dir().join("glyphwave/banner.txt").to_string_lossy().into_owned()
}

/// A banner spec (`logo`, `name` or a path; None for the default) to the
/// source that gets used and its text. The default, and the fallback when
/// the spec gives nothing: the own banner.txt, else the logo, else the name.
pub fn resolve(spec: Option<&str>) -> (Source, String) {
    let mut tried = Vec::new();
    for s in [spec.map(Source::parse), Some(Source::File(own_banner())), Some(Source::Logo), Some(Source::Name)] {
        let Some(s) = s else { continue };
        if tried.contains(&s) {
            continue;
        }
        if let Some(t) = s.text() {
            return (s, t);
        }
        tried.push(s);
    }
    (Source::Name, big("glyphwave"))
}

/// For `l`: the next source after `cur` (logo → name → own file) that has
/// something to show, and its text. `own` is the file `l` offers; with
/// `sample` (`--test`) a missing one shows the sample art instead.
pub fn next(cur: &Source, own: &str, sample: bool) -> Option<(Source, String)> {
    let file = Source::File(own.to_string());
    let third = if sample && file.text().is_none() { Source::Sample } else { file };
    let all = [Source::Logo, Source::Name, third];
    let i = all.iter().position(|s| s == cur).unwrap_or(2);
    (1..=3).map(|k| all[(i + k) % 3].clone()).find_map(|s| s.text().map(|t| (s, t)))
}

/// `text` and its stand-ins for terminals too small for it: the name in big
/// letters, then the bare name. The banner shows the first that fits.
pub fn variants(text: String) -> Vec<String> {
    let host = hostname();
    let mut v = vec![text];
    for s in [big(&host), host] {
        if !v.contains(&s) {
            v.push(s);
        }
    }
    v
}

/// The short host name ("glyphwave" if there's none).
pub fn hostname() -> String {
    let raw = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .unwrap_or_default();
    match raw.trim().split('.').next() {
        Some(h) if !h.is_empty() => h.to_string(),
        _ => "glyphwave".to_string(),
    }
}

/// A fetch tool's stdout, colours stripped; None if it isn't installed,
/// fails, or prints only blanks.
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let t = strip_ansi(&String::from_utf8_lossy(&out.stdout));
    (out.status.success() && t.chars().any(|c| !c.is_whitespace())).then_some(t)
}

/// Plain text from coloured terminal output: escape sequences go (cursor
/// forward becomes spaces, as fastfetch pads with it), tabs become spaces,
/// other control characters go.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0;
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            '\x1b' => match it.next() {
                Some('[') => {
                    let mut arg = String::new();
                    for f in it.by_ref() {
                        if ('@'..='~').contains(&f) {
                            if f == 'C' {
                                let n = arg.parse().unwrap_or(1).clamp(1, 256);
                                out.extend(std::iter::repeat_n(' ', n));
                                col += n;
                            }
                            break;
                        }
                        arg.push(f);
                    }
                }
                // OSC, DCS, APC, PM, SOS: up to BEL or ESC \ (images, titles)
                Some(']' | 'P' | '_' | '^' | 'X') => {
                    while let Some(f) = it.next() {
                        if f == '\x07' {
                            break;
                        }
                        if f == '\x1b' {
                            it.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' => {
                out.push('\n');
                col = 0;
            }
            '\t' => {
                let n = 8 - col % 8;
                out.extend(std::iter::repeat_n(' ', n));
                col += n;
            }
            c if c.is_control() => {}
            c => {
                out.push(c);
                col += 1;
            }
        }
    }
    out
}

/// `name` in big block letters, like `toilet -f bigmono12`. Letters are
/// case-blind; a space is half a letter wide; other characters the font
/// lacks are skipped.
pub fn big(name: &str) -> String {
    let idx: Vec<Option<usize>> =
        name.chars().filter_map(|c| if c == ' ' { Some(None) } else { ORDER.find(c.to_ascii_lowercase()).map(Some) }).collect();
    let mut s = String::new();
    for r in 0..ROWS {
        for &k in &idx {
            match k {
                Some(k) => s.extend(SHEET[k / 8 * ROWS + r].chars().skip(k % 8 * W).take(W)),
                None => s.extend(std::iter::repeat_n(' ', W / 2)),
            }
        }
        s.push('\n');
    }
    s
}

/// The glyphs `big` knows, in the order of the sheet below.
const ORDER: &str = "abcdefghijklmnopqrstuvwxyz0123456789-";
const ROWS: usize = 16;
const W: usize = 10;

/// From toilet's bigmono12 font (caca2tlf of Monospace Bold 12, by Sam
/// Hocevar, WTFPL): eight glyphs to a block, ten columns each, 16 rows.
#[rustfmt::skip]
const SHEET: [&str; 5 * ROWS] = [
    // a         b         c         d         e         f         g         h
    "                                                                                ",
    "           ██                        ██              ▒████             ██       ",
    "           ██                        ██              █████             ██       ",
    "           ██                        ██              ██                ██       ",
    "  ▒████▓   ██░███▒     ▓████▒   ▒███░██   ░████▒   ███████    ▒███▒██  ██░████  ",
    "  ██████▓  ███████▒   ███████  ▒███████  ░██████▒  ███████   ░███████  ███████▓ ",
    "  █▒  ▒██  ███  ███  ▓██▒  ▒█  ███  ███  ██▒  ▒██    ██      ███  ███  ███  ▒██ ",
    "   ▒█████  ██░  ░██  ██░       ██░  ░██  ████████    ██      ██░  ░██  ██    ██ ",
    " ░███████  ██    ██  ██        ██    ██  ████████    ██      ██    ██  ██    ██ ",
    " ██▓░  ██  ██░  ░██  ██░       ██░  ░██  ██          ██      ██░  ░██  ██    ██ ",
    " ██▒  ███  ███  ███  ▓██▒  ░█  ███  ███  ███░  ▒█    ██      ███  ███  ██    ██ ",
    " ████████  ███████▒   ███████  ▒███████  ░███████    ██      ░███████  ██    ██ ",
    "  ▓███░██  ██░███▒     ▓████▒   ▒███░██   ░█████▒    ██       ▒███▒██  ██    ██ ",
    "                                                              █░  ▒██           ",
    "                                                              ██████▓           ",
    "                                                              ▒████▒            ",
    // i         j         k         l         m         n         o         p
    "    ██        ██                                                                ",
    "    ██        ██     ██        ████                                             ",
    "    ██        ██     ██        ████                                             ",
    "                     ██          ██                                             ",
    "  ████      ████     ██  ▓██▒    ██      ██▓█▒██▒  ██░████    ░████░   ██░███▒  ",
    "  ████      ████     ██ ▓██▒     ██      ████████  ███████▓  ░██████░  ███████▒ ",
    "    ██        ██     ██▒██▒      ██      ██░██░██  ███  ▒██  ███  ███  ███  ███ ",
    "    ██        ██     ████▓       ██      ██ ██ ██  ██    ██  ██░  ░██  ██░  ░██ ",
    "    ██        ██     █████       ██      ██ ██ ██  ██    ██  ██    ██  ██    ██ ",
    "    ██        ██     ██░███      ██      ██ ██ ██  ██    ██  ██░  ░██  ██░  ░██ ",
    "    ██        ██     ██  ██▒     ██▒     ██ ██ ██  ██    ██  ███  ███  ███  ███ ",
    " ████████     ██     ██  ▒██     █████   ██ ██ ██  ██    ██  ░██████░  ███████▒ ",
    " ████████     ██     ██   ███    ░████   ██ ██ ██  ██    ██   ░████░   ██░███▒  ",
    "             ▒██                                                       ██       ",
    "           █████                                                       ██       ",
    "           ████░                                                       ██       ",
    // q         r         s         t         u         v         w         x
    "                                                                                ",
    "                                                                                ",
    "                                 ██                                             ",
    "                                 ██                                             ",
    "  ▒███░██   ██░████   ▒█████░  ███████   ██    ██  ██▒  ▒██ ██      ██ ███  ███ ",
    " ▒███████   ███████  ████████  ███████   ██    ██  ▓██  ██▓ ██░    ░██  ██▒▒██  ",
    " ███  ███   ███░     ██▒  ░▒█    ██      ██    ██  ▒██  ██▒ ▓█▒ ██ ▒█▓  ▒████▒  ",
    " ██░  ░██   ██       █████▓░     ██      ██    ██   ██░░██  ▒█▒░██░▒█▒   ████   ",
    " ██    ██   ██       ░██████▒    ██      ██    ██   ██▒▒██   █▓▒██▒██    ▒██▒   ",
    " ██░  ░██   ██          ░▒▓██    ██      ██    ██   ▒████▒   ██▓██▓██    ████   ",
    " ███  ███   ██       █▒░  ▒██    ██░     ██▒  ███    ████    ███▒▒██▓   ▒████▒  ",
    " ▒███████   ██       ████████    █████   ▓███████    ████    ▒██░░██▒   ██▒▒██  ",
    "  ▒███░██   ██       ░▓████▓     ░████    ▓███░██    ▒██▒    ░██  ██   ███  ███ ",
    "       ██                                                                       ",
    "       ██                                                                       ",
    "       ██                                                                       ",
    // y         z         0         1         2         3         4         5
    "                                                                                ",
    "                      ░████░    ░███     ░▓████▒   ░▓████▒       ███   ███████  ",
    "                      ██████    ████     ███████▒  ███████▒     ▒███   ███████  ",
    "                     ▒██  ██▒   █▒██     █▒░  ▓██  █▒░  ▓██    ░████   ██       ",
    " ██▓  ▓██  ████████  ██▒  ▒██     ██           ██        ██    ██░██   ██       ",
    " ▒██  ██▓  ████████  ██    ██     ██          ▒█▓       ▓██   ▒█▒ ██   █████▓░  ",
    "  ██▒ ██░      ▒██▒  ██ ██ ██     ██          ██     █████   ░██  ██   ███████░ ",
    "  ███▒██      ▒██▒   ██ ██ ██     ██        ░██▒     █████░  ██   ██   █▒  ░███ ",
    "  ░██▓█▓     ▒██▒    ██    ██     ██       ░██▒         ▓██  ████████        ██ ",
    "   ████░    ▒██▒     ██▒  ▒██     ██      ▒██▒           ██  ████████        ██ ",
    "   ▒███    ▒██▒      ▒██  ██▒     ██     ▒██▒      █▒   ▓██       ██   █▒  ░███ ",
    "    ██▓    ████████   ██████   ████████  ████████  ███████▒       ██   ███████░ ",
    "    ██░    ████████   ░████░   ████████  ████████  ▒█████▒        ██   ▒████▓░  ",
    "   ▒██                                                                          ",
    "  ███▒                                                                          ",
    "  ███                                                                           ",
    // 6         7         8         9         -
    "                                                  ",
    "   ▓███▒   ████████   ▒████▒    ▒████░            ",
    "  ██████   ████████  ▒██████▒  ▒██████            ",
    " ▒██░ ░█        ▓█▓  ██▓  ▓██  ██▓  ▓█▓           ",
    " ██▒            ██░  ██    ██  ██    ██           ",
    " ██▒███▒       ▓██   ██▓  ▓██  ██    ██           ",
    " ███████▒      ██░    ██████   ██▓  ▓██           ",
    " ██▓  ▓██     ▒██    ░██████░  ▒███████           ",
    " ██    ██     ██▒    ██▓  ▓██   ▒███▒██   █████   ",
    " ██    ██    ▒██     ██    ██       ▒██   █████   ",
    " ▓█▓  ▓██    ██▒     ██▓  ▓██   █░ ░██▒           ",
    "  ██████▒   ▒██      ▒██████▒   ██████            ",
    "  ░████▒    ██▒       ▒████▒    ▒███▓             ",
    "                                                  ",
    "                                                  ",
    "                                                  ",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colours_and_cursor_moves() {
        // fastfetch -s none --pipe false
        assert_eq!(strip_ansi("\x1b[m\x1b[1m\x1b[37m ##+\x1b[m\n\x1b[1m\x1b[37m.%@\x1b[m\n\x1b[m"), " ##+\n.%@\n");
        // neofetch -L: cursor hiding, wrap off, and a jump back up at the end
        assert_eq!(strip_ansi("\x1b[?25l\x1b[?7l\x1b[37m\x1b[0m\x1b[1m  #\x1b[0m\n\x1b[16A\x1b[9999999D\n\n\x1b[?25h\x1b[?7h"), "  #\n\n\n");
        assert_eq!(strip_ansi("a\x1b[3Cb\tc\r\n"), "a   b   c\n");
        assert_eq!(strip_ansi("\x1b]0;title\x07x\x1b_Gf=100;AAAA\x1b\\y\x1bPq#0\x1b\\z"), "xyz");
        assert_eq!(strip_ansi("█▓▒░ é"), "█▓▒░ é");
    }

    #[test]
    fn block_font() {
        for (i, row) in SHEET.iter().enumerate() {
            let glyphs = if i / ROWS == 4 { ORDER.len() - 32 } else { 8 };
            assert_eq!(row.chars().count(), glyphs * W, "sheet row {i}");
        }
        let t = big("tuxbook");
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), ROWS);
        assert!(lines.iter().all(|l| l.chars().count() == 7 * W));
        assert_eq!(lines[4].trim_end(), " ███████   ██    ██  ███  ███  ██░███▒    ░████░    ░████░   ██  ▓██▒");
        assert_eq!(big("TuxBook"), t);
        assert_eq!(big("tux.book!"), t);
        assert!(big("").trim().is_empty());
    }

    #[test]
    fn specs() {
        assert_eq!(Source::parse("logo"), Source::Logo);
        assert_eq!(Source::parse(" name\n"), Source::Name);
        assert_eq!(Source::parse("/a/b.txt"), Source::File("/a/b.txt".into()));
        assert_eq!(Source::parse("~/x.txt"), Source::File(format!("{}/x.txt", home())));
        assert_eq!(resolve(Some("name")).0, Source::Name);
    }

    #[test]
    fn text_banner() {
        let s = Source::parse("text:Hi there");
        assert_eq!(s, Source::Text("Hi there".into()));
        assert_eq!(Source::parse(&s.to_string()), s);
        // a space is half a letter: 2 + 5 letters at 10 columns, plus 5
        assert_eq!(s.text().unwrap().lines().map(|l| l.chars().count()).max(), Some(75));
    }
}
