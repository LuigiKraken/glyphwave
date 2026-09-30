//! Album art: fetched once per track (file:// or http via curl, cached), reduced
//! to a palette by k-means in OKLab.

use crate::color::{Lab, Rgb, fallback_palette};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Covers are shrunk to THUMB×THUMB before clustering.
const THUMB: usize = 48;

#[derive(Clone)]
pub struct Art {
    /// Three stops, dark → bright, saturated enough to read on black.
    pub palette: Vec<Rgb>,
    pub version: u64,
}

impl Default for Art {
    fn default() -> Art {
        Art { palette: fallback_palette(), version: 0 }
    }
}

fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"));
    base.join("glyphwave")
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn fetch(url: &str) -> Option<PathBuf> {
    if let Some(p) = url.strip_prefix("file://") {
        let p = PathBuf::from(percent_decode(p));
        return p.exists().then_some(p);
    }
    if !url.starts_with("http") {
        return None;
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{:016x}.img", fnv(url)));
    if !path.exists() {
        let tmp = path.with_extension("part");
        let ok = std::process::Command::new("curl")
            .args(["-sfL", "--max-time", "8", "-o"])
            .arg(&tmp)
            .arg(url)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let _ = std::fs::remove_file(&tmp);
            return None;
        }
        std::fs::rename(&tmp, &path).ok()?;
    }
    Some(path)
}

/// k-means (k=6) over the thumbnail in OKLab, then pick three vivid clusters.
fn palette(px: &[Rgb]) -> Vec<Rgb> {
    let labs: Vec<Lab> = px.iter().map(|c| c.to_lab()).collect();
    let k = 6;
    let mut cent: Vec<Lab> = (0..k).map(|i| labs[i * labs.len() / k]).collect();
    let mut assign = vec![0usize; labs.len()];
    for _ in 0..8 {
        for (i, p) in labs.iter().enumerate() {
            let d = |c: &Lab| (c.l - p.l).powi(2) + (c.a - p.a).powi(2) + (c.b - p.b).powi(2);
            assign[i] = (0..k).min_by(|&a, &b| d(&cent[a]).total_cmp(&d(&cent[b]))).unwrap();
        }
        let mut sum = vec![(0.0f32, 0.0f32, 0.0f32, 0usize); k];
        for (i, p) in labs.iter().enumerate() {
            let s = &mut sum[assign[i]];
            s.0 += p.l;
            s.1 += p.a;
            s.2 += p.b;
            s.3 += 1;
        }
        for c in 0..k {
            if sum[c].3 > 0 {
                let n = sum[c].3 as f32;
                cent[c] = Lab { l: sum[c].0 / n, a: sum[c].1 / n, b: sum[c].2 / n };
            }
        }
    }
    let mut count = vec![0usize; k];
    for &a in &assign {
        count[a] += 1;
    }
    // Score: share of the cover × how colourful. Greys only win if nothing else exists.
    let mut idx: Vec<usize> = (0..k).filter(|&c| count[c] > 0).collect();
    idx.sort_by(|&a, &b| {
        let s = |c: usize| (count[c] as f32).sqrt() * (0.02 + cent[c].chroma());
        s(b).total_cmp(&s(a))
    });
    let mut picks: Vec<Lab> = Vec::new();
    for &c in &idx {
        // skip near-duplicates of an already picked hue
        if picks.iter().all(|p| {
            let (da, db) = (p.a - cent[c].a, p.b - cent[c].b);
            (da * da + db * db).sqrt() > 0.06
        }) {
            picks.push(cent[c]);
        }
        if picks.len() == 3 {
            break;
        }
    }
    if picks.iter().all(|p| p.chroma() < 0.03) {
        return fallback_palette(); // black-and-white cover
    }
    while picks.len() < 3 {
        let base = picks[0];
        picks.push(Lab { l: (base.l + 0.3).min(0.95), ..base });
    }
    picks.sort_by(|a, b| a.l.total_cmp(&b.l));
    // Normalise: dark / mid / bright lightness, chroma pushed up so it glows on black.
    let target = [0.45, 0.65, 0.88];
    picks
        .iter()
        .zip(target)
        .map(|(p, l)| {
            let ch = p.chroma().max(1e-4);
            let want = if ch < 0.03 { ch } else { ch.max(0.13) * if l > 0.8 { 0.6 } else { 1.0 } };
            Lab { l, a: p.a / ch * want, b: p.b / ch * want }.to_rgb()
        })
        .collect()
}

fn load(url: &str) -> Option<Vec<Rgb>> {
    let path = fetch(url)?;
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?.to_rgb8();
    let small = image::imageops::resize(&img, THUMB as u32, THUMB as u32, image::imageops::FilterType::Triangle);
    let px: Vec<Rgb> = small.pixels().map(|p| Rgb(p[0], p[1], p[2])).collect();
    Some(palette(&px))
}

/// Loads art on a worker thread whenever the track version changes.
pub struct ArtLoader {
    pub art: Arc<Mutex<Art>>,
    want: u64,
}

impl ArtLoader {
    pub fn new() -> ArtLoader {
        ArtLoader { art: Arc::new(Mutex::new(Art::default())), want: u64::MAX }
    }

    pub fn request(&mut self, version: u64, url: &str) {
        if version == self.want {
            return;
        }
        self.want = version;
        let (art, url) = (self.art.clone(), url.to_string());
        std::thread::spawn(move || {
            let palette = load(&url).unwrap_or_else(fallback_palette);
            let mut a = art.lock().unwrap();
            if version >= a.version {
                *a = Art { palette, version };
            }
        });
    }

    pub fn get(&self) -> Art {
        self.art.lock().unwrap().clone()
    }
}
