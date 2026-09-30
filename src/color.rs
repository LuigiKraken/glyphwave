//! Colours: 8-bit RGB for the terminal, OKLab for mixing so gradients stay
//! perceptually even (no muddy midpoints between saturated stops).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub const BLACK: Rgb = Rgb(0, 0, 0);
pub const WHITE: Rgb = Rgb(255, 255, 255);

impl Rgb {
    pub fn hex(s: &str) -> Rgb {
        let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0xffffff);
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// Multiply towards black (a layer's opacity over the black background).
    pub fn scale(self, a: f32) -> Rgb {
        let a = a.clamp(0.0, 1.0);
        Rgb(
            (self.0 as f32 * a) as u8,
            (self.1 as f32 * a) as u8,
            (self.2 as f32 * a) as u8,
        )
    }

    /// Straight RGB lerp: cheap, fine for short hops like fading to white.
    pub fn mix(self, o: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
        Rgb(l(self.0, o.0), l(self.1, o.1), l(self.2, o.2))
    }

    /// Intensify by k (0..1): brighter, and towards white near the top.
    pub fn boost(self, k: f32) -> Rgb {
        let k = k.clamp(0.0, 1.0);
        let l = |a: u8| (a as f32 * (1.0 + 1.2 * k)).min(255.0) as u8;
        Rgb(l(self.0), l(self.1), l(self.2)).mix(WHITE, 0.3 * k)
    }

    pub fn max(self, o: Rgb) -> Rgb {
        Rgb(self.0.max(o.0), self.1.max(o.1), self.2.max(o.2))
    }

    pub fn is_black(self) -> bool {
        self.0 < 3 && self.1 < 3 && self.2 < 3
    }

    pub fn to_lab(self) -> Lab {
        let f = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let (r, g, b) = (f(self.0), f(self.1), f(self.2));
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
        Lab {
            l: 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
            a: 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
            b: 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Lab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
}

impl Lab {
    pub fn to_rgb(self) -> Rgb {
        let l = self.l + 0.3963377774 * self.a + 0.2158037573 * self.b;
        let m = self.l - 0.1055613458 * self.a - 0.0638541728 * self.b;
        let s = self.l - 0.0894841775 * self.a - 1.2914855480 * self.b;
        let (l, m, s) = (l * l * l, m * m * m, s * s * s);
        let r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
        let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
        let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;
        let f = |c: f32| {
            let c = c.clamp(0.0, 1.0);
            let c = if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
            (c * 255.0 + 0.5) as u8
        };
        Rgb(f(r), f(g), f(b))
    }

    pub fn chroma(self) -> f32 {
        (self.a * self.a + self.b * self.b).sqrt()
    }

    pub fn lerp(self, o: Lab, t: f32) -> Lab {
        Lab {
            l: self.l + (o.l - self.l) * t,
            a: self.a + (o.a - self.a) * t,
            b: self.b + (o.b - self.b) * t,
        }
    }
}

/// A precomputed looping gradient (OKLab-interpolated), sampled by 0..1 phase.
#[derive(Clone)]
pub struct Gradient {
    lut: Vec<Rgb>,
}

impl Gradient {
    pub const N: usize = 512;

    /// `stops` evenly spaced; the lut runs stops[0]→…→stops[n-1] once.
    pub fn new(stops: &[Rgb]) -> Gradient {
        let labs: Vec<Lab> = stops.iter().map(|c| c.to_lab()).collect();
        let n = Self::N;
        let lut = (0..n)
            .map(|i| {
                if labs.len() == 1 {
                    return stops[0];
                }
                let t = i as f32 / (n - 1) as f32 * (labs.len() - 1) as f32;
                let a = (t as usize).min(labs.len() - 2);
                labs[a].lerp(labs[a + 1], t - a as f32).to_rgb()
            })
            .collect();
        Gradient { lut }
    }

    /// Stops mirrored back to the start, so sampling can wrap without a seam.
    pub fn looping(stops: &[Rgb]) -> Gradient {
        let mut s = stops.to_vec();
        s.extend(stops.iter().rev().skip(1));
        Gradient::new(&s)
    }

    /// Wrapping sample.
    pub fn wrap(&self, t: f32) -> Rgb {
        let i = (t.rem_euclid(1.0) * (Self::N - 1) as f32) as usize;
        self.lut[i]
    }
}

/// ttfx's default: purple → cyan → white.
pub fn fallback_palette() -> Vec<Rgb> {
    vec![Rgb::hex("8A008A"), Rgb::hex("00D1FF"), Rgb::hex("FFFFFF")]
}


/// Neon palettes for music cycles, picked more often the busier it gets.
pub const VIVID: [&[&str]; 9] = [
    &["ff00c8", "7a00ff", "00e5ff"],
    &["ff0055", "ff8800", "ffee00", "00ff99"],
    &["00ff87", "00c3ff", "b400ff"],
    &["3a00ff", "ff00e6", "ff9a00"],
    &["00ffea", "e0ffff", "ff00aa"],
    &["c6ff00", "00ff6a", "00d0ff"],
    &["ff1744", "d500f9", "2979ff", "00e5ff"],
    &["ff6a00", "ff0080", "8000ff"],
    &["ffe600", "ff2d95", "00f0ff"],
];

pub fn vivid(i: usize) -> Vec<Rgb> {
    VIVID[i % VIVID.len()].iter().map(|h| Rgb::hex(h)).collect()
}

/// Push a palette's chroma and lightness up so it glows on black
/// (album covers are often muted).
pub fn saturate(stops: &[Rgb]) -> Vec<Rgb> {
    stops
        .iter()
        .map(|c| {
            let p = c.to_lab();
            let ch = p.chroma();
            if ch < 0.03 {
                return *c; // a grey stays grey
            }
            let want = (ch * 1.35).max(0.16);
            Lab { l: p.l.max(0.6), a: p.a / ch * want, b: p.b / ch * want }.to_rgb()
        })
        .collect()
}
