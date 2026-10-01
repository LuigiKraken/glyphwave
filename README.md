# glyphwave

A terminal screensaver that listens to the music.

## The idea

A laptop screensaver should be something nice to look at, not just a black
screen. glyphwave puts a text banner in the middle of a fullscreen terminal
and animates it. When nothing is playing it runs through text effects: the
letters rain in, burn, decrypt, get pulled into a black hole. When music
plays, the same banner becomes a visualizer. It follows the beat, the kicks
and hi-hats, the drops and the quiet sections, and switches between themes
when the music changes rather than on a timer.

The rules it's built on:

- **Legible first.** It can get loud and colourful when the music does, but
  the banner always stays readable.
- **Nothing jumps.** Letters that were moved return home, and theme layers
  fade out before the next theme starts.
- **One palette.** The banner, the effects and the ribbon along the bottom
  always share their colours.
- **Cheap.** One small binary, well under a millisecond of CPU per frame, and
  no audio capture unless something is playing. A screensaver shouldn't cost
  battery.
- **Plain character cells.** Everything is text in a terminal, with no
  graphics protocol and no GPU.

It replaced a stack of three separate tools on one KDE laptop:
[ttfx](https://github.com/omacom/ttfx) for the idle text effects, a Python
now-playing script, and [cava](https://github.com/karlstav/cava) for the
bars, with a shell script switching between them. Doing it in one program
means the text and the music share one screen, one palette and one clock.

## What it does

- **When nothing is playing**, the banner cycles through
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
  - `floor`: the banner holds still over a cava bar floor. It replaces the
    ribbon, so it isn't in the rotation; `--theme floor` runs it.
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
    instead of flat-topping. It lifts a little between holds to carry the
    transition, and it gives way to the `floor` theme's own bars.
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
    under a flash, with no outro or intro. A theme that moves the letters
    (`bounce`, `springs`, `glitch`) first brings them back home, and one
    with its own layer (`matrix` rain, `fire`, `warp` stars, `shock` rings,
    `wave`) stops adding to it and fades it out, so neither kind of switch
    jumps. A section change cuts when the
    music is busy and otherwise waits for a lull. The 16/32-bar timer only
    marks a hold as due.

  Everything is drawn in plain character cells: no half-block backdrops, no
  stacked layers. Themes without a fixed palette take a neon set or the
  base purple → cyan → white, saturated. Calm music draws calm themes and loud music busy ones. While a
  track plays, the top-left corner shows its title, artist · album, and
  progress with the length. When playback pauses or stops, the corner stays
  empty.
- Pausing, muting, turning the volume to 0 %, or 4 s of silence ends the
  themed cycle with its outro, and the idle cycle takes over. (The capture
  records the sound before the system volume is applied, so glyphwave watches
  the output's mute and volume itself.)
- **Calls.** When someone calls on Slack, Discord, Teams, WhatsApp and similar
  apps, the screen fades to a small icon for the app and a card saying who's
  calling, read from the desktop's call notification. The music fades out in
  half a second and pauses, and a soft chime rings until the call stops
  (`ringtone = none` or a sound file of your own in the config). You answer
  in the app itself: while a call rings, every key (the music keys too) and
  the mouse end the screensaver, and the music stays paused. A call that
  stops ringing on its own (missed, or declined elsewhere) brings the music
  back, fading in, like a phone.
- **The banner** is your system's logo (whatever fastfetch or neofetch
  shows, in glyphwave's colours), your machine's name in big block letters,
  or your own art in `~/.config/glyphwave/banner.txt`. It uses your own art
  if it's there, else the logo, else the name; `--banner logo|name|FILE`
  picks one, and `l` cycles through them. Art too big for the screen steps
  down to the name.
- **A test mode.** `glyphwave --test` runs the demo track in the current
  terminal with the debug line. `1`–`0` jump to a theme (through the normal
  fade), `d`/`t`/`s`/`w` fake a Discord, Teams, Slack or WhatsApp call, `m`
  fakes mute, space fakes pause, and `[`/`]` make the track calmer or louder.

## The stack

- **Rust**, with four crates:
  - [`realfft`](https://crates.io/crates/realfft) for the FFTs
  - [`zbus`](https://crates.io/crates/zbus) for D-Bus: now playing, player
    controls, lock-screen detection
  - [`signal-hook`](https://crates.io/crates/signal-hook) for clean exits
    and resizes
  - [`libc`](https://crates.io/crates/libc) for the raw terminal
- **No TUI framework.** glyphwave writes the escape codes itself: a
  double-buffered cell grid that sends only the cells that changed, inside
  synchronized-output markers so frames don't tear. 24-bit colour, with a
  256-colour fallback for terminals without it. Colours are mixed in OKLab
  so gradients stay even.
- **Audio** comes from `parec` (or `pw-record` where `parec` isn't
  installed) recording the default output's monitor, so it sees whatever the
  machine plays. That works on PipeWire and PulseAudio alike.
- **Now playing** comes from MPRIS on the session bus, which Spotify,
  browsers and most Linux players speak. Nothing is polled: the bus signals
  when something changes.
- **Calls** come from watching the desktop's notifications on the session
  bus. glyphwave only listens; it never answers or dismisses anything.
- **The banner** is plain text. Every non-space character becomes one letter
  the effects can move and colour. The logo comes from running fastfetch or
  neofetch once, with their colours stripped; the big letters are a font
  built into the binary.
- **Starting it** is left to the desktop's own idle timer. `glyphwave setup`
  hooks into it, and `glyphwave launch` opens the screensaver fullscreen in a
  terminal. `contrib/` has the same recipes by hand for KDE, GNOME,
  Hyprland, sway and X11.
- **One file.** Releases are a static binary (musl, x86_64 and ARM64), so it
  needs no Rust and no extra libraries.

## Status

It runs daily as the screensaver on the laptop it was written for. Not
released yet. Left before the first release:

- **Real calls.** Call detection is tested with test notifications only.
  Whether Slack, Discord, Teams and WhatsApp send a call notification glyphwave
  recognises still needs a real call on each.
- **A recording at the top of this README,** so you can see it move before
  reading anything.
- **A first look on GNOME and Fedora.** The GNOME idle watcher, the Ptyxis and
  GNOME Terminal launch, and the ARM64 binary have been built but not yet run
  on a real system.

## Getting it running

1. **Download.** Paste one line into a terminal. It puts a single file in
   `~/.local/bin`, with no compiling and no password:

   ```sh
   curl -fsSL https://github.com/LuigiKraken/glyphwave/releases/latest/download/install.sh | sh
   ```

2. **Try it.** Type `glyphwave`. It plays right there in the terminal,
   dancing if music is on. `q` quits. `glyphwave --test` gives you keys to
   try every theme and a fake call.
3. **Set it up.** Type `glyphwave setup`. It works out your desktop and asks
   four questions, each with a default:
   - after how many idle minutes the screensaver starts;
   - what happens after it has run a while: lock, screen off, sleep, or keep
     running;
   - whether it runs only when plugged in, or on battery too;
   - which banner to use.
4. **Confirm.** Setup lists exactly which files it will write or change, and
   does nothing until you say yes.
5. **Done.** From then on it fades in when you're away and goes away when
   you move the mouse or press a key. The music keys skip songs without
   waking it, calls show who's calling, and your computer locks or sleeps
   when you chose.

Setup uses your desktop's own idle settings (KDE, GNOME, Hyprland, sway, X11)
rather than running its own timer. On KDE and GNOME it changes them for you;
on Hyprland, sway and X11 it prints the lines to paste. GNOME has no hook for
this, so setup adds a small autostart entry that runs `glyphwave idle-watch`.
Music keeps the computer awake the way it always does: players hold off
sleep while they play, and the desktop's sleep timer takes over once the
music stops.

It never needs root and never installs packages. It only writes to your home
folder, keeps the originals, and `glyphwave setup --remove` puts everything
back. To change your answers, edit `~/.config/glyphwave/config` or run
`glyphwave setup` again. To remove glyphwave, run `glyphwave setup --remove`
and delete `~/.local/bin/glyphwave`.

**The config file** has the general settings at the top (`start_after`,
`then`, `then_after`, `on_battery`, `banner`, `fps`) and optional sections
below, commented out: shorter times on battery, which terminal to open (and a
Konsole profile, if you made one), and the locker for Hyprland, sway and X11.

**What a system needs:**
- a terminal; `glyphwave launch` finds kitty, foot, Alacritty, Ghostty,
  WezTerm, Konsole, Ptyxis, GNOME Terminal or xterm;
- `parec` or `pw-record`, for the music. PipeWire systems have `pw-record`
  (Fedora: `pipewire-utils`, installed by default); `parec` is in
  `pulseaudio-utils`;
- a font with block characters, which every common terminal font has. On the
  bare Linux console it swaps in the few glyphs its fonts have;
- optionally fastfetch or neofetch, for the logo banner.

Fedora Workstation (GNOME) and the KDE spin should need nothing extra; that
is still to be tried on a real Fedora install. The binary in `~/.local/bin`
also suits Silverblue and the other Atomic variants, where `/usr` is
read-only.

## How it works

| File | What |
|---|---|
| `audio.rs` | `parec` (or `pw-record`) from the default output's monitor, float32 stereo at 48 kHz, into a ring buffer; the output's mute and volume; plus the synthetic demo track |
| `dsp.rs` | spectrum, onsets, tempo, beat phase, loudness, drop and section detection |
| `mpris.rs` | now playing (zbus), plus the KDE/GNOME locker check in screensaver mode; re-read only on bus signals |
| `canvas.rs` | cell buffer with half-block pixels and braille dots; the diffed output |
| `calls.rs` | incoming calls, read passively from the desktop's notifications |
| `art.rs` | the banner sources: fetch-tool logo, built-in block font, own file |
| `config.rs` | `~/.config/glyphwave/config` |
| `setup.rs` | `glyphwave setup` and `--remove`, per desktop |
| `launch.rs` | `glyphwave launch` (the terminal) and `idle-watch` (GNOME) |
| `fx/*` | banner cycle, text effects, music themes, bar floor, music ribbon, call view, idle stars / rain, player label |
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

- **Bus:** the player and locker are re-read only when they signal a change
  (plus a 30 s recheck), not polled.

30 fps is the default; Konsole's own redraw cost scales with it.

## Inspiration and credits

- **[cava](https://github.com/karlstav/cava)**: the spectrum pipeline (multiple
  FFT sizes, log-spaced bars, auto-gain, gravity fall) follows it.
- **[terminaltexteffects](https://github.com/ChrisBuilds/terminaltexteffects)**
  and its Rust port **[ttfx](https://github.com/omacom/ttfx)**: the text
  effects, the idea of letters as particles with paths, and the easing
  curves come from them.
- **[MilkDrop](https://www.geisswerks.com/milkdrop/) / [projectM](https://github.com/projectM-visualizer/projectm)**:
  bass, mid and treble measured against their own running average, so a
  quiet track moves as much as a loud one.
- **Music research**, collected in `docs/research-brief.md`: SuperFlux onset
  detection (Böck & Widmer 2013), online peak picking (Böck et al. 2012),
  tempo from weighted autocorrelation (Ellis 2007), novelty-based section
  changes (Foote 2000), and the audio/video sync limits of ITU-R BT.1359.
- **[OKLab](https://bottosson.github.io/posts/oklab/)** (Ottosson 2020) for
  colour mixing.

cava, terminaltexteffects and ttfx are MIT licensed; see `NOTICE`. The
algorithms were re-implemented here, and no source code was copied.
`docs/cava-tte-algorithms.md` holds the constants and effect designs taken
from their sources.
