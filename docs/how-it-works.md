# How glyphwave works

The README covers what glyphwave is and how to install it. This page goes
into the detail: what each theme does, how the music is analysed, how setup
hooks in, and the research it builds on.

## The rules

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

## The themes and the rest, in detail

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
  apps, the screen crossfades in a third of a second to the app's icon and a
  card saying who's calling, read from the desktop's call notification. The
  music fades out in half a second and pauses, and a soft chime rings until
  the call stops (`ringtone = none` or a sound file of your own in the
  config). The view moves with the ring: every note sends a ring out of the
  icon to the screen's edge, pops and buzzes it, and runs a light round the
  card. With the built-in chime it
  follows the notes; with your own ringtone, or the app's own ring, it
  follows the onsets it hears. When the call ends the scene fades back in
  where it left off. You answer
  in the app itself: while a call rings, every key (the music keys too) and
  the mouse end the screensaver, and the music stays paused. `c` ignores
  the call: the ring stops, the screensaver stays, and the music fades back
  in, as it does when a call stops ringing on its own (missed, or declined
  elsewhere), like a phone.
- **The banner** is your system's logo (whatever fastfetch or neofetch
  shows, in glyphwave's colours), your machine's name in big block letters,
  your own words in the same letters (`text:WORDS`), or your own art in
  `~/.config/glyphwave/banner.txt`. It uses your own art if it's there, else
  the logo, else the name; `--banner logo|name|text:WORDS|FILE` picks one, and `l` cycles through them. Art too big for the screen steps
  down to the name.
- **A test mode.** `glyphwave --test` runs the demo track in the current
  terminal with the debug line. `1`–`0` jump to a theme (through the normal
  fade), `d`/`t`/`s`/`w` fake a 5 s Discord, Teams, Slack or WhatsApp call, `m`
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
  installed) recording the monitor of the output the player's stream plays
  to, else the default output's. It switches when the stream moves, as it
  does when headphones or HDMI come and go. That works on PipeWire and
  PulseAudio alike.
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
  terminal. It passes each terminal's own options for a black background
  with no scrollbar or padding (Konsole: `-p ScrollBarPosition=2
  -p TerminalMargin=0 -p ColorScheme=WhiteOnBlack`, the colour scheme only
  without a `konsole_profile`), so a terminal nobody customised looks clean
  to the edge. kitty and xterm also hide the mouse pointer (kitty after a
  second, xterm by turning it black); in the others, Konsole among them, it
  stays where it was left. `contrib/` has the same recipes
  by hand for KDE, GNOME, Hyprland, sway and X11.
- **One file.** Releases are a static binary (musl, x86_64 and ARM64), so it
  needs no Rust and no extra libraries.

## Setup and the config file

Setup uses your desktop's own idle settings (KDE, GNOME, Hyprland, sway, X11)
rather than running its own timer. On KDE and GNOME it changes them for you;
on Hyprland, sway and X11 it prints the lines to paste. GNOME has no hook for
this, so setup adds a small autostart entry that runs `glyphwave idle-watch`.
Music keeps the computer awake the way it always does: players hold off
sleep while they play, and the desktop's sleep timer takes over once the
music stops.

It never needs root and never installs packages. It only writes to your home
folder, keeps the originals, and `glyphwave setup --remove` puts everything
back.

The first question is the banner (`logo`, `name` or `text`; `text` takes
your words or the path to your own art). With more than one screen, and a
desktop and terminal that can place windows (see below), the next is
`screens`: all of them, or just the main one. The rest follow the order things
happen: idle minutes before it starts (`start_after`), how long it runs
(`then_after`, or `forever` for `then = none`), whether coming back while it
runs shows the lock screen (`lock_after`), what comes after the run (`then`,
sleep by default), battery (the same times, off, or different times asked
again), and whether the screen dims while it runs (`dim`, KDE; only on battery
by default).
`b` goes back a question. Before the list of changes it prints the answers as
a timeline in which each step counts from the one before. Sleep shows
whether the computer comes back locked: on KDE that's Lock after waking from
sleep (`LockOnResume`), on GNOME the Screen Lock switch, and the Hyprland,
sway and X11 lines setup prints lock before suspending.

To change your answers, edit `~/.config/glyphwave/config` or run
`glyphwave setup` again. To remove glyphwave, run `glyphwave setup --remove`
and delete `~/.local/bin/glyphwave`.

**The config file** has the general settings at the top (`start_after`,
`then`, `then_after`, `lock_after`, `on_battery`, `dim`, `screens`, `banner`, `fps`) and optional sections
below, commented out: shorter times on battery, which terminal to open (and a
Konsole profile of your own, which then keeps its colours), and the locker for Hyprland, sway and X11.

**Locking when you come back.** `lock_after` is a grace period, in minutes
from the screensaver's start. A key or the mouse within it closes the
screensaver onto the desktop; after it, glyphwave locks the session first and
closes once the lock screen is up (it waits at most 2 s), so you land on the
lock screen. `0` always locks; `none`, the default, never does. Only waking
it counts: when the desktop closes it (`launch --stop`, its own lock screen)
nothing extra happens. A call is no exception: the key you press to answer locks too, past
the grace period. It's one value for both power states and counts time
asleep. On KDE and GNOME it locks through logind (`loginctl lock-session`),
which both lock screens answer; on Hyprland, sway and X11 it runs the
`locker` from the config. `then = lock` still works as before: at
`then_after` the desktop locks and the screensaver ends. To keep the
screensaver up for longer and still have it lock, pair `lock_after` with
`then = screen-off` or `none` (and on KDE turn off its own lock timer, which
setup warns about).

**Starting it by hand.** Started with the start-now key or the app-menu
entry (`glyphwave launch --now`, which passes `--now` on), it's a music
visualizer: waking it never locks, whatever `lock_after` says, and while it
runs it holds off the desktop's own timers with
`org.freedesktop.ScreenSaver.Inhibit` on the session bus, or, where nothing
answers that (GNOME), `org.gnome.SessionManager.Inhibit` for idle and suspend. The
inhibit is let go when it exits, and by the bus if it crashes. On KDE that
stops the lock timer and PowerDevil's dimming, screen-off and sleep; hypridle
honours it unless `ignore_dbus_inhibit = true`. swayidle doesn't listen to the
bus, so on sway glyphwave asks sway to make its window an idle inhibitor
(`inhibit_idle open`), gone with the window. xidlehook on X11 has neither, so
its timers still fire during a run started by hand. It keeps the screen on
until you stop it, which costs battery. Started by the idle timer it holds
nothing off, so the timers after it work as before. Tested on KDE; GNOME and
sway are untested.

## More than one screen

`screens = all` (the default) runs the screensaver fullscreen on every
screen; `main` runs it on the main one and turns the others black. Black
rather than left as they were, so the desktop isn't on show while you're
away, and so a key or the mouse on any screen still finds a glyphwave window
and ends it. A black screen costs nothing after it's painted: glyphwave
sends it no frames.

It stays one process. Each extra screen gets a terminal of its own running
`glyphwave --screensaver --screen N`, a stub that only keeps the window
open. The main process opens the stub's terminal, draws into it and reads
its keys and mouse, like its own. So the audio is recorded and analysed
once, the bus is watched once, the ringtone rings once, and the lock
(`lock_after`, or the locker taking over) happens once; each screen only
adds its own drawing, about 0.3 ms a frame, and the terminal's redraw. Every
screen gets its own banner cycle and theme picks, fed by the same analysis,
so they react to the same beats and drops but don't show the same theme at
the same moment.

- **Ending it.** Any key or the mouse on any screen ends it on all of them,
  and so does closing one of the windows. Plugging a screen in or out while
  it runs ends it as well (it checks the connectors in
  `/sys/class/drm` every 2 s): someone is at the machine, and the desktop
  may have moved a window onto another screen. All of these count as waking
  it, so `lock_after` applies.
- **No strays.** A stub waits on a lock the main process holds, so when the
  main process ends, however it ends (`launch --stop`, a crash, `kill -9`),
  every stub exits and its window closes. `launch --stop` stops the stubs
  too. A terminal whose stub doesn't show up within 10 s is closed.
- **Placing the windows** is the compositor's job, and each has its own way:

  | desktop | how | main screen |
  |---|---|---|
  | KDE, Wayland and X11 | a KWin script, loaded over D-Bus, puts each window by its pid fullscreen on its screen, now and as it opens; it's unloaded when glyphwave ends | the primary screen (System Settings > Display) |
  | sway | `swaymsg '[pid=…] move container to output …'` once the window is there | the focused screen (sway has no primary) |
  | Hyprland | the terminal is started through `hyprctl dispatch exec '[monitor …; fullscreen] …'` | the focused screen |
  | GNOME, other X11 window managers | can't be done: there's no way for an app to choose the screen | one window, on the screen the desktop picks; the others stay as they are |

  Placing by pid needs one process per window, so the launcher starts
  Konsole with `--separate`, WezTerm with `--always-new-process` and Ghostty
  with `--gtk-single-instance=false`. GNOME Terminal opens every window from
  one server process, so with it glyphwave also stays on one screen. Setup
  says so in both cases, rather than half-working.

## What a system needs

- a terminal; `glyphwave launch` finds kitty, foot, Alacritty, Ghostty,
  WezTerm, Konsole, Ptyxis, GNOME Terminal or xterm;
- `parec`, for the music, and `pactl`, for the output's mute and volume;
  both come with `pulseaudio-utils` (`libpulse` on Arch), which every
  desktop with PipeWire or PulseAudio normally has. `pw-record` stands in
  for `parec` on PipeWire 1.4 or newer;
- a font with block characters, which every common terminal font has. On the
  bare Linux console it swaps in the few glyphs its fonts have;
- optionally fastfetch or neofetch, for the logo banner.

See [contrib/gnome](../contrib/gnome/README.md) for Fedora.

## Source layout

| File | What |
|---|---|
| `audio.rs` | `parec` (or `pw-record`) from the player's output's monitor (else the default's), float32 stereo at 48 kHz, into a ring buffer; that output's mute and volume; plus the synthetic demo track |
| `dsp.rs` | spectrum, onsets, tempo, beat phase, loudness, drop and section detection |
| `mpris.rs` | now playing (zbus), plus the KDE/GNOME locker check in screensaver mode; re-read only on bus signals |
| `canvas.rs` | cell buffer with half-block pixels and braille dots; the diffed output |
| `calls.rs` | incoming calls, read passively from the desktop's notifications |
| `art.rs` | the banner sources: fetch-tool logo, built-in block font, own file |
| `config.rs` | `~/.config/glyphwave/config` |
| `setup.rs` | `glyphwave setup` and `--remove`, per desktop |
| `launch.rs` | `glyphwave launch` (the terminal) and `idle-watch` (GNOME) |
| `screens.rs` | more than one screen: the extra terminals, their stubs, placing them per desktop, plugging in and out |
| `fx/*` | banner cycle, text effects, music themes, bar floor, music ribbon, call view, idle stars / rain, player label |
| `scene.rs` | picks each cycle's theme and says when its hold ends |

## DSP choices

These follow [research-brief.md](research-brief.md), a literature review with references. [cava-tte-algorithms.md](cava-tte-algorithms.md) holds the constants and effect designs taken from the cava and TTE sources.

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

## Cost

Measured at 160×45 and 30 fps, on this machine, over a 150 s demo run
through four themes:
- **CPU:** 0.3 ms per frame.
- **Output:** about 9–12 KB per frame on average (6 KB without the ribbon).
  Only changed cells are sent, and cells whose colour barely changed are
  skipped.
- **Audio:** `parec` only runs while a player reports Playing. It stops 10 s
  after playback stops.
- **Bus:** the player and locker are re-read only when they signal a change
  (plus a 30 s recheck), not polled.

30 fps is the default; Konsole's own redraw cost scales with it.

## References

- **[MilkDrop](https://www.geisswerks.com/milkdrop/) / [projectM](https://github.com/projectM-visualizer/projectm)**:
  bass, mid and treble measured against their own running average, so a
  quiet track moves as much as a loud one.
- **Music research**, collected in [research-brief.md](research-brief.md): SuperFlux onset
  detection (Böck & Widmer 2013), online peak picking (Böck et al. 2012),
  tempo from weighted autocorrelation (Ellis 2007), novelty-based section
  changes (Foote 2000), and the audio/video sync limits of ITU-R BT.1359.
- **[OKLab](https://bottosson.github.io/posts/oklab/)** (Ottosson 2020) for
  colour mixing.
