//! Colours: 8-bit RGB for the terminal, OKLab for mixing so gradients stay
//! perceptually even (no muddy midpoints between saturated stops).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub const BLACK: Rgb = Rgb(0, 0, 0);
pub const WHITE: Rgb = Rgb(255, 255, 255);

impl Rgb {
    /// 0xRRGGBB, for constant palettes.
    pub const fn from_hex(v: u32) -> Rgb {
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

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

    /// Nearest entry of the xterm 256-colour palette: the 6×6×6 cube or the
    /// 24-step grey ramp, whichever is closer. For terminals without truecolor.
    pub fn xterm256(self) -> u8 {
        const LEVELS: [i32; 6] = [0, 95, 135, 175, 215, 255];
        let near = |v: u8| {
            let v = v as i32;
            (0..6).min_by_key(|&i| (LEVELS[i] - v).abs()).unwrap()
        };
        let d = |a: (i32, i32, i32)| {
            let (r, g, b) = (self.0 as i32 - a.0, self.1 as i32 - a.1, self.2 as i32 - a.2);
            r * r + g * g + b * b
        };
        let (r, g, b) = (near(self.0), near(self.1), near(self.2));
        let cube = 16 + 36 * r + 6 * g + b;
        let avg = (self.0 as i32 + self.1 as i32 + self.2 as i32) / 3;
        let grey = ((avg - 3) / 10).clamp(0, 23);
        let gv = 8 + 10 * grey;
        if d((gv, gv, gv)) < d((LEVELS[r], LEVELS[g], LEVELS[b])) { 232 + grey as u8 } else { cube as u8 }
    }

    /// The colour a 256-colour terminal will actually show for this one.
    pub fn snap256(self) -> Rgb {
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        match self.xterm256() {
            n @ 232.. => {
                let v = 8 + 10 * (n - 232);
                Rgb(v, v, v)
            }
            n => {
                let n = n - 16;
                Rgb(LEVELS[(n / 36) as usize], LEVELS[(n / 6 % 6) as usize], LEVELS[(n % 6) as usize])
            }
        }
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

    #[allow(clippy::excessive_precision)] // the reference matrices, as published
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
    #[allow(clippy::excessive_precision)]
    fn linear(self) -> [f32; 3] {
        let l = self.l + 0.3963377774 * self.a + 0.2158037573 * self.b;
        let m = self.l - 0.1055613458 * self.a - 0.0638541728 * self.b;
        let s = self.l - 0.0894841775 * self.a - 1.2914855480 * self.b;
        let (l, m, s) = (l * l * l, m * m * m, s * s * s);
        let r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
        let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
        let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;
        [r, g, b]
    }

    fn in_gamut(self) -> bool {
        self.linear().iter().all(|c| (-1e-4..=1.0001).contains(c))
    }

    pub fn to_rgb(self) -> Rgb {
        let [r, g, b] = self.linear();
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
                // keep the stops' chroma through the blend, so hues far
                // apart on the wheel don't pass through grey
                let (p, q, k) = (labs[a], labs[a + 1], t - a as f32);
                let m = p.lerp(q, k);
                let (want, got) = (p.chroma() + (q.chroma() - p.chroma()) * k, m.chroma());
                if got > 1e-3 { Lab { l: m.l, a: m.a / got * want, b: m.b / got * want } } else { m }.to_rgb()
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


/// Hue offsets (degrees) of the colour schemes a music cycle can wear,
/// calm to busy: analogous, a pair with an accent, split complementary,
/// triad, square.
pub const SCHEMES: [&[f32]; 5] = [
    &[-35.0, 0.0, 35.0],
    &[0.0, 40.0, 180.0],
    &[0.0, 150.0, 210.0],
    &[0.0, 120.0, 240.0],
    &[0.0, 90.0, 180.0, 270.0],
];

/// A music cycle's stops: `SCHEMES[scheme]` turned round `hue` (degrees,
/// anywhere on the wheel), each at its hue's most vivid lightness that
/// still reads on black; full chroma when `hype` is 1, softer at 0.
pub fn harmony(hue: f32, scheme: usize, hype: f32) -> Vec<Rgb> {
    let share = 0.65 + 0.35 * hype.clamp(0.0, 1.0);
    SCHEMES[scheme % SCHEMES.len()].iter().map(|o| vivid(hue + o, share)).collect()
}

/// The hue `h` (degrees) at the lightness (0.62..0.9) where it holds the
/// most chroma in sRGB, at `share` (0..1) of that chroma.
pub fn vivid(h: f32, share: f32) -> Rgb {
    let (s, c) = h.to_radians().sin_cos();
    let at = |l: f32, ch: f32| Lab { l, a: c * ch, b: s * ch };
    let most = |l: f32| {
        let (mut lo, mut hi) = (0.0f32, 0.4f32);
        for _ in 0..12 {
            let m = (lo + hi) / 2.0;
            if at(l, m).in_gamut() { lo = m } else { hi = m }
        }
        lo
    };
    let (l, ch) = (0..=14)
        .map(|i| 0.62 + 0.02 * i as f32)
        .map(|l| (l, most(l)))
        .fold((0.62, 0.0), |b, x| if x.1 > b.1 { x } else { b });
    at(l, ch * share.clamp(0.0, 1.0)).to_rgb()
}

#[cfg(test)]
mod tests {
    use super::Rgb;

    #[test]
    fn xterm256_picks_cube_or_grey() {
        assert_eq!(Rgb(0, 0, 0).xterm256(), 16);
        assert_eq!(Rgb(255, 255, 255).xterm256(), 231);
        assert_eq!(Rgb(255, 0, 0).xterm256(), 196);
        assert_eq!(Rgb(0, 255, 255).xterm256(), 51);
        assert_eq!(Rgb(128, 128, 128).xterm256(), 244);
        for c in [Rgb(12, 200, 90), Rgb(90, 20, 160), Rgb(40, 40, 44), Rgb(255, 128, 0)] {
            assert_eq!(c.snap256().xterm256(), c.xterm256(), "{c:?} snaps to its own entry");
        }
    }
}
