# glyphwave

A terminal screensaver for the Tuxedo, in one small Rust binary. It replaces the
ttfx + `nowplaying-vis.py` + cava trio that `kde-screensaver-run` used to switch
between.

- **When nothing is playing**, the tuxbook banner cycles through
  terminaltexteffects-style animations: 16 intros (expand, rain, slide, decrypt,
  beams, burn, waves, fireworks, blackhole, unstable, spray, bouncy balls, print,
  matrix, VHS tape, colour shift). Each is followed by a hold with a specular
  sweep and one of 6 outros. A faint starfield sometimes drifts behind.
- **When music plays** (any MPRIS player, Spotify preferred), the same banner
  cycle keeps going, but each cycle gets a theme: a paired intro, its own
  palette, and a hold in which the banner reacts to the music.
  - `levels`: the letters are the equaliser. Each column pair fills from the
    bottom to its band's level.
  - `pulse`: brightness follows the kick, the colour steps on every beat, the
    downbeat runs a sweep, and hi-hats sparkle single letters.
  - `shock`: kicks send rings out from the centre that jolt the letters;
    snares fire a beam along a row.
  - `wave`: a mirrored waveform rolls in from both screen edges toward the
    banner, faster the busier the music. The letters stay put and light up
    as each swell arrives.
  - `fire`: ASCII fire rises off the letters. Flame height follows each
    column's band, and kicks make it surge.
  - `matrix`: katakana rain whose speed follows loudness, with new streams on
    hi-hats. The letters light up where the rain passes.
  - `glitch`: hi-hats scramble letters, snares tear rows sideways, and kicks
    split the banner into magenta and cyan ghosts.
  - `springs`: every letter hangs on a spring. Kicks push them and the bass
    bends the banner.
  - `floor`: the banner holds still over a cava bar floor.
  - `bounce`: the whole banner flies around the screen, faster with the
    music, with kicks swerving it and a dim trail behind. Each wall it hits
    jumps the palette.
  - `warp`: hyperspace. Glyphs stream out from the banner's edges and speed
    up as they go. How many follows intensity; kicks burst and drops flood.

  - Under all of it runs a **ribbon** along the bottom edge: a mirrored stereo
    skyline with a dim reflection, on through intros, outros and the gaps
    between them, so the screen never goes still while the music plays. How
    much it does follows the music's intensity. Quiet music gets a low, dim,
    still ribbon. Busy music gets a tall one that brightens on the beat,
    flashes on kicks, throws hi-hat sparks and pulses outward on the downbeat.
    It wears the banner's colours (each column takes the colour of the letters
    above it). Spikes run into the headroom under the banner on a soft limit
    instead of flat-topping. Once the music is really going, a mirror image
    grows down from the top edge too, flipped left for right. It lifts a
    little between holds to carry the transition, and it gives way to the
    `floor` theme's own bars.
  - **Colours** are shared: the banner, the theme's accents and the ribbon
    all read one palette, so they always match. Each cycle gets a neon
    palette (more likely the busier the music) or the album's, saturated;
    `fire`, `matrix` and `glitch` keep their own. While a theme holds, the
    palette steps on every beat: a little when it's calm, big jumps (and a
    bigger one on the downbeat) when it's busy.
  - **Switching** follows the music rather than a clock. A lull (a
    breakdown, a breath between sections) fades out: the outro, then the
    next intro straight away, with a calmer pick. A high (a drop, or a surge
    in level on a kick) cuts: the next, busier theme takes over at once
    under a flash, with no outro or intro. A section change cuts when the
    music is busy and otherwise waits for a lull. The 16/32-bar timer only
    marks a hold as due.

  Everything is drawn in plain character cells: no half-block backdrops, no
  stacked layers. Themes without a fixed palette take their colours from the
  album art or a neon set. Calm music draws calm themes and loud music busy ones. While a
  track plays, the top-left corner shows its title, artist · album, and
  progress with the length. When playback pauses or stops, the corner stays
  empty.
- Pausing, or 4 s of silence, ends the themed cycle with its outro, and the idle
  cycle takes over.

## Run

```bash
cargo build --release
./target/release/glyphwave                # interactive
./target/release/glyphwave --demo         # built-in 124 BPM test track, no player needed
./target/release/glyphwave --screensaver  # what the idle launcher runs
```

Keys (interactive): `q` quit, `space` play/pause, `n`/`p` next/previous,
`v` next theme (or next text effect), `i` toggle idle/music, `d` debug overlay.

Options: `--fps N` (default 60), `--banner FILE`, `--idle`,
`--theme fire` (always use one theme; `--help` lists them),
`--debug`, `--trace` (beat/onset/drop events to stderr), and for testing
`--frames N`, `--size WxH` and `--stats`.

## How it works

| File | What |
|---|---|
| `audio.rs` | `parec` from `@DEFAULT_MONITOR@`, float32 stereo at 48 kHz, into a ring buffer; plus the synthetic demo track |
| `dsp.rs` | spectrum, onsets, tempo, beat phase, loudness, drop and section detection |
| `mpris.rs` | now playing (zbus), plus the KDE locker check in screensaver mode |
| `art.rs` | cover fetch and cache, OKLab k-means palette |
| `canvas.rs` | cell buffer with half-block pixels and braille dots; the diffed output |
| `fx/*` | banner cycle, text effects, music themes, bar floor, music ribbon, idle stars / rain, player label |
| `scene.rs` | picks each cycle's theme and says when its hold ends |

### DSP choices

These follow `docs/research-brief.md`, a literature review with references. `docs/cava-tte-algorithms.md` holds the constants and effect designs taken from the cava and TTE sources.

- **Bars.** Three Hann-windowed FFTs (4096 below 250 Hz, 2048 up to 2.5 kHz,
  1024 above), log-spaced bars from 40 Hz to 16 kHz, band-summed power in dB.
  - Auto-gain is slow and asymmetric: up τ 0.25 s, down 5 s.
  - 36 dB of visible range with a contrast curve.
  - cava's quadratic gravity fall.
- **Onsets.** SuperFlux (Böck & Widmer 2013) on a fixed 100 Hz hop, split into
  kick, snare and hat bands. Peaks are picked with a z-score threshold, using the
  online parameters from Böck et al. 2012.
- **Tempo.** Ellis 2007 weighted autocorrelation: τ0 = 0.5 s, σ = 1.4 octaves,
  with the duple fold. The estimate only changes after two agreeing estimates.
- **Beat phase.** A PLL that nudges phase and period on matching onsets. Beats
  fire about 30 ms early to cancel the analysis delay; audio-first sync
  tolerance is only about 45 ms (ITU-R BT.1359).
- **Features.** Momentary and short-term loudness, log centroid, flatness, and
  MilkDrop-style band ratios.
  - **Intensity:** onset density, loudness (relative and absolute), how much
    of the spectrum is lit, and kick/snare punch. It rises in 0.4 s and falls
    in 2 s. It drives the ribbon and the calm/busy theme choice. The relative
    loudness has a small weight because it reads near-full for any steady
    track once the auto-gain settles.
  - **Drop:** a stretch of more than 2 s with the bass below half its running
    average, then the bass coming back.
  - **Section change:** the last 4 s of features against the 8 s before
    (Foote-lite).
- **Themes.** A hold lasts 16 or 32 bars (25–60 s without a tempo), or ends
  early on a section change, landing on the next downbeat. Nothing switches
  during a build-up. A drop plays out inside the current theme (rings, a
  flame surge, a full scramble, the letters blown apart) and makes the next
  theme a busy one. Themed intros start on a beat and run a little faster
  with faster music.

### Cost

Measured at 160×45 and 30 fps, on this machine, over a 150 s demo run
through four themes:
- **CPU:** 0.3 ms per frame.
- **Output:** 10–12 KB per frame on average (6 KB without the ribbon). Only changed cells are sent, and cells
  whose colour barely changed are skipped.
- **Audio:** `parec` only runs while a player reports Playing. It stops 10 s
  after playback stops.

If Konsole itself gets busy, `--fps 30` halves everything.

## Credits

The spectrum pipeline follows [cava](https://github.com/karlstav/cava). The text
effects follow [terminaltexteffects](https://github.com/ChrisBuilds/terminaltexteffects)
and its Rust port [ttfx](https://github.com/omacom/ttfx). All three are MIT
licensed; see `NOTICE`. The algorithms were re-implemented here, and no source
code was copied.
