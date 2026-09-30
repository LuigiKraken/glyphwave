//! glyphwave — a terminal screensaver: the banner cycles through
//! terminaltexteffects-style animations; while music plays each cycle gets a
//! theme that makes the banner react to the sound, with a now-playing corner.

mod art;
mod audio;
mod canvas;
mod color;
mod dsp;
mod fx;
mod mpris;
mod scene;
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

  --screensaver     exit on any key or mouse motion, or when the KDE locker
                    takes over (for the idle launcher)
  --demo            play a built-in synthetic track instead of the sound card
  --idle            never use the music themes
  --fps N           frame rate (default 60)
  --banner FILE     banner text (default ~/.local/share/kde-screensaver/screensaver.txt)
  --frames N        exit after N frames (testing)
  --size WxH        render size when stdout isn't a terminal (testing)
  --stats           print frame timing / output size on exit
  --debug           feature overlay (bpm, onsets, theme)
  --trace           print beat/onset/drop events to stderr
  --theme NAME      always use one music theme (levels pulse shock wave fire
                    matrix glitch springs floor)

KEYS (interactive): q quit · space play/pause · n next · p previous ·
  v next theme/effect · i idle/music · d debug
";

struct Opts {
    screensaver: bool,
    demo: bool,
    idle: bool,
    fps: f32,
    banner: String,
    frames: Option<u64>,
    size: Option<(usize, usize)>,
    stats: bool,
    debug: bool,
    trace: bool,
    theme: Option<String>,
}

fn opts() -> Opts {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut o = Opts {
        screensaver: false,
        demo: false,
        idle: false,
        fps: 60.0,
        banner: format!("{home}/.local/share/kde-screensaver/screensaver.txt"),
        frames: None,
        size: None,
        stats: false,
        debug: false,
        trace: false,
        theme: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screensaver" => o.screensaver = true,
            "--demo" => o.demo = true,
            "--idle" => o.idle = true,
            "--stats" => o.stats = true,
            "--debug" => o.debug = true,
            "--trace" => o.trace = true,
            "--theme" => o.theme = args.next(),
            "--fps" => o.fps = args.next().and_then(|v| v.parse().ok()).unwrap_or(60.0f32).clamp(5.0, 240.0),
            "--banner" => o.banner = args.next().unwrap_or_default(),
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
    let o = opts();
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
    let mut artl = art::ArtLoader::new();
    let mut cap = audio::Capture::new();
    if o.demo {
        audio::start_synth(cap.ring.clone());
    }
    let mut an = dsp::Analyzer::new();
    let mut spec = fx::spectrum::Spectrum::new();
    let mut aurora = fx::aurora::Aurora;
    let mut stars = fx::stars::Stars::new();
    let mut rain = fx::rain::Rain::new();
    let mut banner = fx::banner::Banner::load(&o.banner);
    let mut dir = scene::Director::new();
    if let Some(th) = &o.theme {
        if let Err(e) = dir.lock(th) {
            eprintln!("glyphwave: {e}");
            std::process::exit(2);
        }
    }

    let (mut w, mut h) = o.size.unwrap_or_else(term::size);
    let mut cv = Canvas::new(w, h);
    let mut palette = fallback_palette();
    let mut grad = Gradient::looping(&palette);
    let mut art_version = u64::MAX;

    let mut music = Fader::default();
    let mut label_f = Fader::default();
    let mut idle_layers = [Fader::default(); 3]; // aurora, stars, rain
    let mut idle_forced = o.idle;
    let mut debug = o.debug;
    let mut phase = 0.0f32;
    let mut not_playing = 0.0f32;
    let mut rng = fx::Rng::seeded();
    let mut idle_cycle = 0u64;

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
            term::poll_input(wait.as_millis() as i32, tty_in, &mut input);
            if !input.is_empty() {
                let t = start.elapsed().as_secs_f32();
                if o.screensaver {
                    if t > grace {
                        break 'main;
                    }
                    input.clear(); // swallow the launch keypress / a nudged mouse
                }
                for &b in &input {
                    match b {
                        b'q' | 3 | 27 => break 'main,
                        b' ' => watcher.control("PlayPause"),
                        b'n' => watcher.control("Next"),
                        b'p' => watcher.control("Previous"),
                        b'v' => {
                            if banner.theme().is_some() {
                                dir.next()
                            } else {
                                banner.skip_idle()
                            }
                        }
                        b'i' => idle_forced = !idle_forced,
                        b'd' => debug = !debug,
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
                cv = Canvas::new(w, h);
            }
            cv.force_full();
        }
        an.set_bars(spec.bars_for(w));

        // now playing + palette
        let track = watcher.snapshot();
        artl.request(track.version, &track.art_url);
        let a = artl.get();
        if a.version != art_version {
            art_version = a.version;
            palette = a.palette;
            grad = Gradient::looping(&palette);
        }

        // audio: capture while something plays (or the demo), stop after 10 s
        let playing = o.demo || track.playing();
        not_playing = if playing { 0.0 } else { not_playing + dt };
        if !o.demo {
            if playing && !idle_forced {
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
                "{t:7.2} bpm {:5.1} conf {:.2} ph {:.2} {}{}{}{}{}{} loud {:.2} en {:.2} ten {:.2} rms {:.1}",
                f.bpm, f.beat_conf, f.beat_phase,
                if f.beat { 'B' } else { ' ' },
                if f.kick > 0.0 { 'K' } else { ' ' },
                if f.snare > 0.0 { 'S' } else { ' ' },
                if f.hat > 0.0 { 'H' } else { ' ' },
                if f.drop { 'D' } else { ' ' },
                if f.section { '§' } else { ' ' },
                f.loud, f.energy, f.tension, f.rms_db
            );
        }

        let want_music = playing && !idle_forced && !(f.silent && f.silent_for > 4.0);
        music.target = if want_music { 1.0 } else { 0.0 };
        music.step(dt, if want_music { 1.2 } else { 2.0 });
        let themed = banner.theme().is_some();
        if banner.cycles != idle_cycle {
            // each new idle effect gets a fresh ambience: aurora, stars, both or none
            idle_cycle = banner.cycles;
            idle_layers[0].target = if !themed && rng.chance(0.45) { 1.0 } else { 0.0 };
            idle_layers[1].target = if !themed && rng.chance(0.3) { 1.0 } else { 0.0 };
        }
        label_f.target = if track.playing() && !idle_forced { 1.0 } else { 0.0 };
        label_f.step(dt, if label_f.target > 0.5 { 1.0 } else { 0.4 });

        phase = (phase + dt * 0.004) % 1.0;
        let light = 0.8 + 0.2 * f.loud * music.a() + 0.2 * (1.0 - music.a());
        let cx = Ctx { f, w, h, t, dt, palette: &palette, grad: &grad, phase, light };

        // ------------------------------------------------------ compose
        cv.clear();
        if idle_layers[0].on() {
            aurora.draw(&mut cv, &cx, idle_layers[0].a());
        }
        cv.resolve_pixels();
        for fl in &mut idle_layers {
            fl.step(dt, 2.0);
        }
        idle_layers[2].target = if banner.wants_rain() { 1.0 } else { 0.0 };
        if idle_layers[2].on() {
            rain.draw(&mut cv, &cx, idle_layers[2].a());
        }
        if idle_layers[1].on() {
            stars.draw(&mut cv, &cx, idle_layers[1].a());
        }
        cv.resolve_dots();
        banner.draw(&mut cv, &cx, want_music, &mut dir, &mut spec);
        fx::label::draw(&mut cv, &cx, &track, label_f.a());
        if debug {
            let s = format!(
                " {:>4.1}ms {:>6}B  {:>5.1}bpm conf {:.2}  loud {:.2} en {:.2} cen {:.2} flat {:.2}  {}{}{}  ten {:.2}  {} ",
                busy.as_secs_f32() * 1000.0 / frames.max(1) as f32,
                bytes / frames.max(1),
                f.bpm,
                f.beat_conf,
                f.loud,
                f.energy,
                f.centroid,
                f.flatness,
                if f.kick_env > 0.5 { 'K' } else { '·' },
                if f.snare_env > 0.5 { 'S' } else { '·' },
                if f.hat_env > 0.5 { 'H' } else { '·' },
                f.tension,
                if themed { dir.name.clone() } else { format!("idle:{:?}", banner.current) },
            );
            for x in 0..w as i32 {
                cv.dim(x, h as i32 - 1, 0.1);
            }
            cv.text(0, h as i32 - 1, &s, Rgb(200, 200, 200));
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
    cap.stop();
    term.restore();
    if o.stats {
        eprintln!(
            "glyphwave: {frames} frames, {:.2} ms/frame busy, {} bytes/frame, {:.1} s",
            busy.as_secs_f64() * 1000.0 / frames.max(1) as f64,
            bytes / frames.max(1),
            start.elapsed().as_secs_f32()
        );
    }
}
