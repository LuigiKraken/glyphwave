//! glyphwave — a terminal screensaver: the banner cycles through
//! terminaltexteffects-style animations; while music plays each cycle gets a
//! theme that makes the banner react to the sound, with a now-playing corner.

mod art;
mod audio;
mod calls;
mod canvas;
mod color;
mod config;
mod dsp;
mod fx;
mod launch;
mod mpris;
mod ringer;
mod scene;
mod setup;
mod term;

use canvas::Canvas;
use color::{Gradient, Rgb, fallback_palette};
use fx::{Ctx, Fader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const HELP: &str = "\
glyphwave — terminal screensaver + music visualizer

USAGE: glyphwave [options]
       glyphwave setup [--remove] [--dry-run] [--desktop NAME]
       glyphwave launch [--stop]

  setup             ask when to start and what comes after, then hook glyphwave
                    into the desktop's idle timer (KDE, GNOME, Hyprland, sway,
                    X11); lists every file first. --remove puts it all back
  launch            open the screensaver fullscreen in a terminal, once; what
                    the idle timer runs. --stop closes it (before a locker)

  --screensaver     exit on mouse motion, on any key but F1–F12 and the
                    music keys, or when the KDE/GNOME locker takes over
  --demo            play a built-in synthetic track instead of the sound card
  --test            try things out in this terminal: --demo and --debug,
                    plus the TEST KEYS below
  --idle            never use the music themes
  --fps N           frame rate (default 30)
  --colors MODE     truecolor or 256 (default: from COLORTERM / TERM)
  --banner SRC      logo (what fastfetch or neofetch shows), name (the host
                    name in big letters) or a text file (default: your own
                    ~/.config/glyphwave/banner.txt, else logo, else name)
  --frames N        exit after N frames (testing)
  --size WxH        render size when stdout isn't a terminal (testing)
  --stats           print frame timing / output size on exit
  --debug           feature overlay (bpm, onsets, theme)
  --trace           print beat/onset/drop events to stderr
  --theme NAME      always use one music theme (levels pulse shock wave fire
                    matrix glitch springs bounce warp; floor, the bar floor
                    in place of the ribbon, runs only when asked for)

banner and fps also come from ~/.config/glyphwave/config (setup writes it);
options given here win. ringtone = default, none or a sound file (wav, ogg,
flac) there picks what plays while a call rings.

KEYS (interactive): q quit · space play/pause · n next · p previous ·
  v next theme/effect · i idle/music · d debug · l next banner (logo, name,
  your file)
MUSIC KEYS (also in --screensaver, without waking it): - previous ·
  + next · Enter play/pause
TEST KEYS (--test): 1–0 the themes in the order above · d t s w fake a
  Discord / Teams / Slack / WhatsApp call, ringing 5 s or until c (a real
  player that's playing fades out and back in) · m mute the demo · space
  pause the demo · [ ] calmer / louder demo · o debug (in place of d) ·
  l also shows sample art when there's no banner.txt

A muted sink, or one at 0 %, counts as paused. While a call rings (read from
the desktop's call notification) the banner makes way for the app's icon and
the caller, the music fades out and pauses, and a ringtone plays. Any key or
the mouse ends glyphwave, the music keys too, and the music stays paused; c
ignores the call, and that or a ring that stops on its own fades the music
back in.
";

struct Opts {
    screensaver: bool,
    demo: bool,
    test: bool,
    idle: bool,
    fps: f32,
    banner: Option<String>,
    ringtone: Option<String>,
    frames: Option<u64>,
    size: Option<(usize, usize)>,
    stats: bool,
    debug: bool,
    trace: bool,
    theme: Option<String>,
    truecolor: bool,
    console: bool,
}

fn opts() -> Opts {
    let cfg = config::load().unwrap_or_default();
    let mut o = Opts {
        screensaver: false,
        demo: false,
        test: false,
        idle: false,
        fps: cfg.fps.unwrap_or(30.0).clamp(5.0, 240.0),
        banner: cfg.banner,
        ringtone: cfg.ringtone,
        frames: None,
        size: None,
        stats: false,
        debug: false,
        trace: false,
        theme: None,
        truecolor: term::truecolor(),
        console: std::env::var("TERM").is_ok_and(|t| t == "linux"),
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screensaver" => o.screensaver = true,
            "--demo" => o.demo = true,
            "--test" => (o.test, o.demo, o.debug) = (true, true, true),
            "--idle" => o.idle = true,
            "--stats" => o.stats = true,
            "--debug" => o.debug = true,
            "--trace" => o.trace = true,
            "--theme" => o.theme = args.next(),
            "--colors" => match args.next().as_deref() {
                Some("truecolor" | "24bit") => o.truecolor = true,
                Some("256") => o.truecolor = false,
                v => {
                    eprintln!("glyphwave: --colors takes truecolor or 256, not {}", v.unwrap_or("nothing"));
                    std::process::exit(2);
                }
            },
            "--fps" => o.fps = args.next().and_then(|v| v.parse().ok()).unwrap_or(30.0f32).clamp(5.0, 240.0),
            "--banner" => match args.next() {
                Some(v) if !v.is_empty() => o.banner = Some(v),
                _ => {
                    eprintln!("glyphwave: --banner takes logo, name or a file");
                    std::process::exit(2);
                }
            },
            "--frames" => o.frames = args.next().and_then(|v| v.parse().ok()),
            "--size" => {
                o.size = args.next().and_then(|v| {
                    let (w, h) = v.split_once('x')?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                })
            }
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            other => {
                eprintln!("glyphwave: unknown option {other}\n\n{HELP}");
                std::process::exit(2);
            }
        }
    }
    o
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("setup") => std::process::exit(setup::run(&args[1..])),
        Some("launch") => std::process::exit(launch::launch(&args[1..])),
        Some("idle-watch") => std::process::exit(launch::idle_watch()),
        _ => {}
    }
    let o = opts();
    // before the screen switches, so a warning stays readable
    let (mut source, text) = art::resolve(o.banner.as_deref());
    if let Some(b) = &o.banner {
        if art::Source::parse(b) != source {
            eprintln!("glyphwave: nothing to show from --banner {b}, using {source}");
        }
    }
    // the file `l` offers: the one asked for, else the own banner.txt
    let own = match o.banner.as_deref().map(art::Source::parse) {
        Some(art::Source::File(p)) => p,
        _ => art::own_banner(),
    };
    std::panic::set_hook(Box::new(|info| {
        term::emergency_restore();
        eprintln!("glyphwave: {info}");
    }));

    let quit = Arc::new(AtomicBool::new(false));
    let resized = Arc::new(AtomicBool::new(true));
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT, signal_hook::consts::SIGHUP] {
        let _ = signal_hook::flag::register(sig, quit.clone());
    }
    let _ = signal_hook::flag::register(signal_hook::consts::SIGWINCH, resized.clone());

    let tty_in = term::stdin_is_tty();
    let mut term = term::Term::enter(o.screensaver);
    let watcher = mpris::Watcher::start(o.screensaver);
    let mut cap = audio::Capture::new();
    if o.demo {
        audio::start_synth(cap.ring.clone());
    }
    let calls = calls::Calls::start();
    let mut ringer = ringer::Ringer::new(o.ringtone.as_deref());
    let (mut fade, mut was_ringing) = (None::<std::thread::JoinHandle<()>>, false);
    // the player a ring paused, to play again if the ring ends on its own
    let mut paused_for_call: Option<String> = None;
    // the demo isn't on the sink; --test fakes a mute with m instead
    let hushed = if o.demo { Arc::new(AtomicBool::new(false)) } else { audio::watch_sink() };
    let (mut fake_pause, mut fake_mute) = (false, false);
    let mut an = dsp::Analyzer::new();
    let mut spec = fx::spectrum::Spectrum::new();
    let mut stars = fx::stars::Stars::new();
    let mut rain = fx::rain::Rain::new();
    let mut ribbon = fx::ribbon::Ribbon::new();
    let mut calm = fx::ribbon::Calm::new();
    let mut call_tint: Vec<Rgb> = Vec::new();
    let mut banner = fx::banner::Banner::new(&art::variants(text));
    let mut dir = scene::Director::new();
    if let Some(th) = &o.theme {
        if let Err(e) = dir.lock(th) {
            eprintln!("glyphwave: {e}");
            std::process::exit(2);
        }
    }

    let (mut w, mut h) = o.size.unwrap_or_else(term::size);
    let mut cv = Canvas::new(w, h, o.truecolor, o.console);
    let palette = fallback_palette();
    let grad = Gradient::looping(&palette);

    let mut music = Fader::default();
    let mut label_f = Fader::default();
    let mut ribbon_f = Fader::default();
    let mut lift = Fader::default();
    // how wild the music is right now, for the ribbon's sparks, and the
    // seconds left of the rush after a drop
    let (mut wild, mut rush) = (Fader::default(), 0.0f32);
    let mut call_f = Fader::default();
    let mut pulse = fx::call::Pulse::new();
    let mut scene_f = Fader { v: 1.0, target: 1.0 };
    // the last call seen, kept after it ends for the fade-out
    let (mut call, mut call_seen) = (None::<calls::Call>, 0u32);
    let mut idle_layers = [Fader::default(); 2]; // stars, rain
    let mut idle_forced = o.idle;
    let mut debug = o.debug;
    let mut phase = 0.0f32;
    let mut not_playing = 0.0f32;
    let mut rng = fx::Rng::seeded();
    let mut idle_cycle = 0u64;
    let mut traced = String::new();

    let frame_dt = Duration::from_secs_f32(1.0 / o.fps);
    let start = Instant::now();
    let mut last = Instant::now();
    let mut next_frame = Instant::now();
    let mut input = Vec::new();
    let grace = if o.screensaver { 0.5 } else { 0.0 };
    let (mut frames, mut bytes, mut busy) = (0u64, 0u64, Duration::ZERO);

    'main: loop {
        if quit.load(Ordering::Relaxed) || watcher.locked.load(Ordering::Relaxed) {
            break;
        }
        // input until the next frame is due
        loop {
            let wait = next_frame.saturating_duration_since(Instant::now());
            // rounded up: poll(0) on the last fraction of a ms would spin
            term::poll_input(wait.as_millis() as i32 + 1, tty_in, &mut input);
            if !input.is_empty() {
                let t = start.elapsed().as_secs_f32();
                // while a call rings every key ends glyphwave, music keys too
                calls.sync(&mut call_seen, &mut call);
                let ringing = call.as_ref().is_some_and(|c| c.ringing(Instant::now()));
                // c ignores it: the ring stops, the music comes back, glyphwave stays
                if ringing && input.contains(&b'c') && !(o.screensaver && t <= grace) {
                    calls.ignore();
                    input.clear();
                    continue;
                }
                if o.screensaver {
                    if t <= grace {
                        input.clear(); // swallow the launch keypress
                    } else if ringing || term::wakes(&input) {
                        break 'main;
                    }
                    input.retain(|&b| term::is_media(b)); // drop F-key sequences
                } else if ringing {
                    break 'main;
                }
                for &b in &input {
                    match b {
                        b'q' | 3 | 27 => break 'main,
                        b'0'..=b'9' if o.test => dir.jump(fx::themes::ALL[(b - b'0' + 9) as usize % 10]),
                        b'd' if o.test => calls.fake("discord", "pixelfox", "Incoming call"),
                        b't' if o.test => calls.fake("Microsoft Teams", "Morgan Lee is calling you", ""),
                        b's' if o.test => calls.fake("Slack", "Sam Rivera invited you to a huddle", ""),
                        b'w' if o.test => calls.fake("WhatsApp", "Incoming voice call", "+49 151 2345 6789"),
                        b'm' if o.test => fake_mute = !fake_mute,
                        b' ' if o.test => fake_pause = !fake_pause,
                        b'[' | b']' if o.test => {
                            let lv = audio::DEMO_LEVEL.load(Ordering::Relaxed);
                            let lv = if b == b'[' { lv.saturating_sub(1) } else { (lv + 1).min(6) };
                            audio::DEMO_LEVEL.store(lv, Ordering::Relaxed);
                        }
                        b'o' if o.test => debug = !debug,
                        b' ' | b'\r' | b'\n' => watcher.control("PlayPause"),
                        b'n' | b'+' => watcher.control("Next"),
                        b'p' | b'-' => watcher.control("Previous"),
                        b'v' => {
                            if banner.theme().is_some() {
                                dir.next()
                            } else {
                                banner.skip_idle()
                            }
                        }
                        b'i' => idle_forced = !idle_forced,
                        b'd' => debug = !debug,
                        b'l' => {
                            if let Some((s, t)) = art::next(&source, &own, o.test) {
                                source = s;
                                banner.swap(&art::variants(t));
                            }
                        }
                        _ => {}
                    }
                }
            }
            if Instant::now() >= next_frame || quit.load(Ordering::Relaxed) {
                break;
            }
        }
        next_frame += frame_dt;
        let now = Instant::now();
        if next_frame < now {
            next_frame = now + frame_dt; // fell behind; don't try to catch up
        }
        let t0 = Instant::now();
        let dt = now.duration_since(last).as_secs_f32().min(0.1);
        last = now;
        let t = start.elapsed().as_secs_f32();

        if resized.swap(false, Ordering::Relaxed) && o.size.is_none() {
            let (nw, nh) = term::size();
            if (nw, nh) != (w, h) || frames == 0 {
                (w, h) = (nw, nh);
                cv = Canvas::new(w, h, o.truecolor, o.console);
            }
            cv.force_full();
        }
        an.set_bars(spec.bars_for(w));

        let track = watcher.snapshot();

        // audio: capture while something plays (or the demo), stop after 10 s;
        // a muted sink, or one at 0 %, counts as paused. While a call rings
        // it listens too: the music has paused, so what plays is the ring,
        // and the call view pulses on it
        let hush = fake_mute || hushed.load(Ordering::Relaxed);
        let playing = !hush && ((o.demo && !fake_pause) || track.playing());
        not_playing = if playing { 0.0 } else { not_playing + dt };
        let listen = was_ringing && !o.demo;
        if !o.demo {
            if (playing && !idle_forced) || listen {
                cap.start();
            } else if (not_playing > 10.0 || idle_forced) && cap.running() {
                cap.stop();
            }
        }
        {
            let ring = cap.ring.lock().unwrap();
            an.update(&ring, dt);
        }
        let f = &an.f;
        if o.trace && (f.beat || f.kick > 0.0 || f.drop || f.section || frames % 30 == 0) {
            eprintln!(
                "{t:7.2} bpm {:5.1} conf {:.2} ph {:.2} {}{}{}{}{}{} loud {:.2} en {:.2} int {:.2} ten {:.2} rms {:.1}",
                f.bpm, f.beat_conf, f.beat_phase,
                if f.beat { 'B' } else { ' ' },
                if f.kick > 0.0 { 'K' } else { ' ' },
                if f.snare > 0.0 { 'S' } else { ' ' },
                if f.hat > 0.0 { 'H' } else { ' ' },
                if f.drop { 'D' } else { ' ' },
                if f.section { '§' } else { ' ' },
                f.loud, f.energy, f.intensity, f.tension, f.rms_db
            );
        }

        let want_music = playing && !idle_forced && !(f.silent && f.silent_for > 4.0);
        music.target = if want_music { 1.0 } else { 0.0 };
        music.step(dt, if want_music { 1.2 } else { 2.0 });
        let themed = banner.theme().is_some();
        if banner.cycles != idle_cycle {
            // each new idle effect gets a fresh ambience: stars or none
            idle_cycle = banner.cycles;
            idle_layers[0].target = if !themed && rng.chance(0.3) { 1.0 } else { 0.0 };
        }
        label_f.target = if track.playing() && !hush && !idle_forced { 1.0 } else { 0.0 };
        label_f.step(dt, if label_f.target > 0.5 { 1.0 } else { 0.4 });

        phase = (phase + dt * 0.004) % 1.0;
        let light = 0.8 + 0.2 * f.loud * music.a() + 0.2 * (1.0 - music.a());
        let cx = Ctx { f, w, h, t, dt, palette: &palette, grad: &grad, phase, light };

        // a ringing call: the scene (banner, theme, ribbon, ambience) and the
        // call view crossfade, quickly so the call is up at once; when it
        // ends the scene comes back where it was, more slowly
        calls.sync(&mut call_seen, &mut call);
        let ringing = call.as_ref().is_some_and(|c| c.ringing(now));
        scene_f.target = if ringing { 0.0 } else { 1.0 };
        scene_f.step(dt, if ringing { 0.35 } else { 0.9 });
        if ringing && !was_ringing {
            pulse.reset();
        }
        // the player fades out and pauses once (--demo leaves the real one
        // alone, --test's fake calls don't, to try it); the ringtone loops while it rings. A ring that ends without
        // a key (missed, or declined elsewhere) brings the music back, like a
        // phone; a key ends glyphwave first, so answering keeps it paused
        if ringing && !was_ringing && (!o.demo || o.test) && track.playing() {
            fade = Some(ringer::fade_and_pause(&track.player, watcher.later(track.player.clone(), "Pause")));
            paused_for_call = Some(track.player.clone());
        }
        if !ringing && was_ringing {
            if let Some(p) = paused_for_call.take() {
                fade = Some(ringer::play_and_fade_in(&p, fade.take(), watcher.later(p.clone(), "Play")));
            }
        }
        was_ringing = ringing;
        ringer.ring(ringing);
        audio::DEMO_RINGING.store(ringing && o.demo, Ordering::Relaxed);
        call_f.target = if ringing { 1.0 } else { 0.0 };
        call_f.step(dt, if ringing { 0.3 } else { 0.5 });

        // ------------------------------------------------------ compose
        cv.clear();
        for fl in &mut idle_layers {
            fl.step(dt, 2.0);
        }
        idle_layers[1].target = if banner.wants_rain() { 1.0 } else { 0.0 };
        // the music ribbon runs under every phase, and through a ring, which
        // it hears (the ringtone); the floor theme has its own bars. Going, it
        // settles first and then fades
        let holding = banner.holding();
        let live = ringing || (want_music && holding != Some(fx::themes::Theme::Floor));
        calm.feed(f, dt, live, ringing);
        ribbon_f.target = if live { 1.0 } else if calm.since > 0.35 { 0.0 } else { ribbon_f.target };
        ribbon_f.step(dt, if ribbon_f.target > 0.5 { 0.8 } else { 0.5 });
        lift.target = if holding.is_none() { 1.0 } else { 0.0 };
        lift.step(dt, 0.6);
        // wild: the stretch after a drop, or peak intensity under one of the
        // explosive themes (shock, glitch, warp, fire)
        rush = if f.drop { 8.0 } else { (rush - dt).max(0.0) };
        let peak = holding.is_some_and(|t| t.busy() >= 0.75) && f.intensity > 0.75;
        wild.target = if rush > 0.0 || peak { 1.0 } else { 0.0 };
        wild.step(dt, if wild.target > 0.5 { 1.0 } else { 2.5 });
        if scene_f.on() {
            if idle_layers[1].on() {
                rain.draw(&mut cv, &cx, idle_layers[1].a());
            }
            if idle_layers[0].on() {
                stars.draw(&mut cv, &cx, idle_layers[0].a());
            }
            cv.resolve_dots();
        }
        if ribbon_f.on() && banner.fits {
            let mut room = h as i32 - banner.bottom() - 1;
            let mut tint = banner.tint();
            // under the call view it keeps below the card, in the app's colours
            if let Some(c) = call.as_ref().filter(|_| call_f.on()) {
                let k = call_f.a();
                room += ((h as i32 - fx::call::bottom(c, w, h) - 1 - room) as f32 * k).round() as i32;
                call_tint.clear();
                for (x, b) in tint.iter().enumerate() {
                    let u = (x as f32 / w as f32 - 0.5).abs();
                    call_tint.push(b.mix(fx::call::hue(c, u, t), k));
                }
                tint = &call_tint;
            }
            ribbon.draw(&mut cv, &Ctx { f: calm.fed(), ..cx }, room.max(0) as usize, ribbon_f.a(), lift.a(), wild.a(), tint);
        }
        if scene_f.on() {
            banner.draw(&mut cv, &cx, want_music, &mut dir, &mut spec);
            banner.sample(&cv, dt);
            // the ribbon (the lit cells) stays
            if scene_f.v < 1.0 {
                let k = scene_f.a();
                for y in 0..h as i32 {
                    for x in 0..w as i32 {
                        if cv.idx(x, y).is_some_and(|i| !cv.is_lit(i)) {
                            cv.dim(x, y, k);
                        }
                    }
                }
            }
        }
        fx::label::draw(&mut cv, &cx, &track, label_f.a());
        if let Some(c) = call.as_ref().filter(|_| call_f.on()) {
            pulse.step(dt, ringing, ringer.round(), if listen { f.onset } else { 0.0 });
            fx::call::draw(&mut cv, &cx, c, &pulse, call_f.a());
        }
        if debug {
            let s = format!(
                " {:>4.1}ms {:>6}B  {:>5.1}bpm conf {:.2}  loud {:.2} en {:.2} int {:.2} cen {:.2} flat {:.2}  {}{}{}  ten {:.2}  {} ",
                busy.as_secs_f32() * 1000.0 / frames.max(1) as f32,
                bytes / frames.max(1),
                f.bpm,
                f.beat_conf,
                f.loud,
                f.energy,
                f.intensity,
                f.centroid,
                f.flatness,
                if f.kick_env > 0.5 { 'K' } else { '·' },
                if f.snare_env > 0.5 { 'S' } else { '·' },
                if f.hat_env > 0.5 { 'H' } else { '·' },
                f.tension,
                if themed { dir.name.clone() } else { format!("idle:{:?}", banner.current) },
            ) + if hush { "muted " } else { "" }
                + if fake_pause { "paused " } else { "" }
                + &if o.test { format!("demo {}/6 ", audio::DEMO_LEVEL.load(Ordering::Relaxed)) } else { String::new() };
            for x in 0..w as i32 {
                cv.dim(x, h as i32 - 1, 0.1);
            }
            cv.text(0, h as i32 - 1, &s, Rgb(200, 200, 200));
        }

        if o.trace && dir.name != traced {
            traced = dir.name.clone();
            eprintln!("{t:7.2} theme {traced} (int {:.2})", f.intensity);
        }
        let out = cv.flush();
        bytes += out.len() as u64;
        term::write_all(out.as_bytes());
        busy += t0.elapsed();
        frames += 1;
        if o.frames.is_some_and(|n| frames >= n) {
            break;
        }
    }
    ringer.stop();
    cap.stop();
    term.restore();
    // a fade cut short by the exit still pauses and restores the volume
    if let Some(f) = fade {
        let _ = f.join();
    }
    if cap.missing {
        eprintln!(
            "glyphwave: no music visuals: parec and pw-record are missing or didn't run. parec comes \
             with pulseaudio-utils (Debian, Ubuntu, Fedora) or libpulse (Arch); pw-record needs \
             PipeWire 1.4 or newer."
        );
    }
    if o.stats {
        eprintln!(
            "glyphwave: {frames} frames, {:.2} ms/frame busy, {} bytes/frame, {:.1} s",
            busy.as_secs_f64() * 1000.0 / frames.max(1) as f64,
            bytes / frames.max(1),
            start.elapsed().as_secs_f32()
        );
    }
}
