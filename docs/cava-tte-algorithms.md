# Algorithms reference: cava (spectrum) + TerminalTextEffects (text effects)

Studied sources (cloned into `scratchpad/src-study/`):

| Project | Repo / revision | Language | License |
|---|---|---|---|
| cava | github.com/karlstav/cava @ `2992198` (2026-09-21). `cavacore.c/.h` still exist and are still the engine (not replaced in 1.0). | C | **MIT**, (c) 2015 Karl Stavestrand |
| TerminalTextEffects (TTE) | github.com/ChrisBuilds/terminaltexteffects (v0.15.x, main) | Python | **MIT**, (c) 2023 ChrisBuilds |
| ttfx | github.com/omacom/ttfx (Rust port of TTE v0.15.0 commit `7a91dd9`, byte-identical frames) | Rust | **MIT**, (c) 2026 37signals / omacom-io + (c) 2023 ChrisBuilds |

Compliance: for either, keep the MIT copyright + permission notice in your LICENSE/NOTICE (e.g. a `THIRD_PARTY.md`/`NOTICE` listing "cava (c) 2015 Karl Stavestrand, MIT" and "TerminalTextEffects (c) 2023 ChrisBuilds, MIT"; add "ttfx (c) 2026 37signals/omacom-io, MIT" if you copy Rust code from ttfx). Reimplementing algorithms from a description does not strictly require it, but crediting is expected and trivial.

The TTE effect descriptions below are from the **Python originals** (clearer than the optimized ttfx SoA engine; ttfx reproduces them exactly, so either is authoritative).

---

## Part A — cava (cavacore.c + cava.c)

### A.1 API / data flow

```
cava_init(bars_per_channel, rate, channels(1|2), autosens(0|1), noise_reduction(0..1),
          low_cut_off(Hz), high_cut_off(Hz), scaling_mode(LINEAR|DECIBEL)) -> plan
cava_execute(in: *f64 interleaved samples (int16 range for LINEAR), new_samples, out: *f64, plan)
   out layout: out[0..N) = left (or mono), out[N..2N) = right.  N = bars per channel.
```
cava.c calls `cava_init(number_of_bars / output_channels, ...)` where output_channels = 2 in stereo mode.

Config defaults (config.c): `framerate=60`, `sensitivity=100` (%), `autosens=1`, `lower_cutoff_freq=50`, `higher_cutoff_freq=8000` (Windows build: 10000), `noise_reduction=77` (Windows: 70) → divided by 100 and clamped to [0,1] → **0.77**; `monstercat=0`, `waves=0`, `bar_width=2`, `bar_spacing=1`, `channels=stereo`, `mono_option=average`, `reverse=0`, `sample_rate=44100` (48000 for pipewire), `max_height=100%`, `sleep_timer=0`. `sens` config: `sens/100`, and if autosens then `sens=1` (manual sens multiplies after cavacore; up/down keys ×1.05 / ×0.95).

### A.2 FFT sizes (dual buffer)

```
fft = 512
if      8125 < rate <= 16250  : fft *= 2    (1024)
else if 16250 < rate <= 32500 : fft *= 4    (2048)
else if 32500 < rate <= 75000 : fft *= 8    (4096)   <- 44.1k / 48k
else if 75000 < rate <= 150000: fft *= 16
else if 150000< rate <= 300000: fft *= 32
else if rate > 300000         : fft *= 64
FFTbufferSize     = fft          (mid+treble, 4096 @44.1k)
FFTbassbufferSize = fft * 2      (bass, 8192 @44.1k)
input_buffer_size = FFTbassbufferSize * channels
max bars per channel = fft/2 + 1
bass_cut_off = 100 Hz  -> bars whose lower cutoff freq < 100 Hz read the bass FFT
```
Real-to-complex FFTs (FFTW `r2c_1d`), output bins 0..size/2. Magnitude per bin = `hypot(re, im)`.

### A.3 Input buffer & windowing

Per `cava_execute`:
```
new_samples = min(new_samples, input_buffer_size)
if new_samples > 0:
    framerate -= framerate/64
    framerate += (rate * frame_skip) / (new_samples / channels) / 64      // EMA of real fps, init 75
    frame_skip = 1
    shift input_buffer right by new_samples (input_buffer[n] = input_buffer[n-new_samples], n from end)
    for n in 0..new_samples: input_buffer[new_samples-n-1] = in[n]  (÷32768 if DECIBEL)   // newest first, REVERSED
    silence = all in[n]==0
else: frame_skip++
```
So the buffer is time-reversed (index 0 = newest sample). Because of the reversal, interleaved L,R becomes R,L:
```
stereo: bass_r[n]=buf[2n], bass_l[n]=buf[2n+1]   (n < FFTbass);  r[n]=buf[2n], l[n]=buf[2n+1] (n < FFT)
mono:   bass_l[n]=buf[n]; l[n]=buf[n]
```
(The treble FFT therefore uses the newest `FFTbufferSize` samples; the bass FFT uses twice as many.)
Hann window, precomputed: `w[i] = 0.5 * (1 - cos(2π i / (N-1)))` for each buffer size N.

### A.4 Cut-off frequencies (log distribution)

```
frequency_constant = log10(low / high) / (1/(bars+1) - 1)          // = log10(high/low) * (bars+1)/bars
min_bandwidth = rate / FFTbassbufferSize   (integer division in float var)
bass_cut_off_bar = 0
for n in 0..=bars:                                  // bars+1 edges
    coeff = -frequency_constant + (n+1)/(bars+1) * frequency_constant
    cut_off_frequency[n] = high * 10^coeff
    if n>0 && cut[n-1] >= cut[n]: cut[n] = cut[n-1] + min_bandwidth
    rel[n] = cut[n] / (rate/2)
    if cut[n] < 100 (bass):
        lower[n] = (int)(rel[n] * FFTbass/2); bass_cut_off_bar++; first_bar = (bass_cut_off_bar<=1)
        lower[n] = min(lower[n], FFTbass/2)
    else:
        lower[n] = ceil(rel[n] * FFT/2)
        if n == bass_cut_off_bar:        // first treble bar
            first_bar = 1
            if n>0: upper[n-1] = (int)(rel[n] * FFTbass/2) - 1     // last bass bar ends at this freq in the bass FFT
        else first_bar = 0
        lower[n] = min(lower[n], FFT/2)
    if n>0:
        if !first_bar:
            upper[n-1] = lower[n] - 1
            if lower[n] <= lower[n-1]:                       // clumped in bass: push spectrum up by 1 bin
                limit = (n < bass_cut_off_bar ? FFTbass/2 : FFT/2) + 1
                if lower[n-1] + 1 < limit: lower[n] = lower[n-1]+1; upper[n-1] = lower[n]-1
        else if upper[n-1] < lower[n-1]: upper[n-1] = lower[n-1] + 1
    // recompute the actual frequency from the bin
    rel[n] = lower[n] / ((n < bass_cut_off_bar ? FFTbass : FFT) / 2)
    cut_off_frequency[n] = rel[n] * rate/2
```
Note: with coeff at n: exponent goes from `-fc + fc/(bars+1)` (n=0 → `low` exactly, since `-fc*bars/(bars+1) = log10(low/high)`) to `+fc/(bars+1)` at n=bars (slightly above `high`). Bar n spans bins `[lower[n], upper[n]]`, using the bass FFT if `n < bass_cut_off_bar`.

### A.5 Per-bar EQ / normalisation (LINEAR mode)

```
for n in 0..bars:
    eq[n] = 1 / 2^28                                   // FFT magnitudes are huge (int16 input)
    eq[n] *= cut_off_frequency[n+1] ^ 0.85             // boost highs (pink-noise tilt)
    eq[n] /= log2(n < bass_cut_off_bar ? FFTbassbufferSize : FFTbufferSize)
    eq[n] /= (upper[n] - lower[n] + 1)                 // average over the band's bins
bar[n] = eq[n] * Σ_{i=lower[n]..=upper[n]} hypot(out[i])
```
DECIBEL mode (input normalised ÷32768): `bar = 20*log10(Σ mag) / 70` (max_db = 70), non-finite → 0; no eq.

### A.6 Sensitivity, smoothing (gravity fall-off + integral), autosens

```
if autosens: out[n] *= sens            (sens starts 1.0, sens_init = 1)
framerate_mod = 66 / framerate          // framerate = the EMA above -> frame-rate independence
gravity_mod   = framerate_mod^2.5 * 2 / noise_reduction
integral_mod  = framerate_mod^0.1
overshoot = false
for n in 0..bars*channels:
    // gravity fall-off
    if out[n] < prev[n] && noise_reduction > 0.1:
        out[n] = peak[n] * (1 - fall[n]^2 * gravity_mod)
        if out[n] < 0: out[n] = 0
        fall[n] += 0.028
    else:
        peak[n] = out[n]; fall[n] = 0
    prev[n] = out[n]
    // integral (IIR "memory")
    out[n] = mem[n] * noise_reduction / integral_mod + out[n]
    mem[n] = out[n]
    if autosens && out[n] > 1.0: overshoot = true; out[n] = 1.0
// autosens
if autosens:
    if overshoot: sens *= 1 - 0.02 * framerate_mod;  sens_init = 0          // fast decrease (-2%/frame @66fps)
    else if !silence:
        sens *= 1 + 0.001 * framerate_mod * autosens                          // slow increase (+0.1%/frame)
        if sens_init: sens *= 1 + 0.1 * framerate_mod                         // initial fast ramp (+10%) until first overshoot
```
Observations: the integral filter's steady-state gain is `1/(1 - nr/integral_mod)` (≈4.3× at nr=0.77), which is what autosens compensates. Output is 0..1 (clamped by autosens); cava.c then does `out *= manual_sens`, clamps to [0,1], multiplies by the bar dimension (rows, or rows*8 for the noncurses 1/8-block renderer, × max_height), halves for split orientations.

Frame pacing in cava.c: `samples_per_frame = rate / framerate`; if `samples_per_frame < input_buffer_size/channels` ("high_framerate") each frame consumes exactly `samples_per_frame*channels` samples from the audio ring (or fewer on underrun, or all-but-buffer on overflow); else all available. Sleep = `frame_time_ns - elapsed - wakeup_overshoot`. `sleep_timer`: after `framerate*sleep_timer` silent frames, sleeps 1 s per loop.

### A.7 Monstercat and waves filters (cava.c `monstercat_filter`, applied per channel after scaling to bar heights)

Operates on bar heights in **cells/pixels** (after the ×height scaling), in place, sequential (later bars see earlier updates):
```
height_normalizer = height > 1000 ? height / 912.76 : 1.0
if waves > 0:
    for z in 0..N:
        bars[z] /= 1.25
        for m in (0..z).rev():  d = z-m;  bars[m] = max(bars[z] - height_normalizer * d^2, bars[m])
        for m in z+1..N:        d = m-z;  bars[m] = max(bars[z] - height_normalizer * d^2, bars[m])
elif monstercat > 0:          // config value, e.g. 1.0 ; factor = monstercat*1.5
    for z in 0..N:
        for m in (0..z).rev():  d = z-m;  bars[m] = max(bars[z] / (monstercat*1.5)^d, bars[m])
        for m in z+1..N:        d = m-z;  bars[m] = max(bars[z] / (monstercat*1.5)^d, bars[m])
```
Note: only invoked `if (p.monstercat)` — so `waves` requires monstercat to be non-zero too (waves wins if both set). Waves = parabolic falloff (quadratic in distance, in cell units); monstercat = exponential falloff by factor 1.5·m per bar.

### A.8 Stereo layout (cava.c)

Left = `out[0..N)`, right = `out[N..2N)` (N = bars/2). Default (mirror, bass in the middle):
```
for n in 0..bars:
   if n < bars/2: raw[n] = reverse ? left[n]            : left[bars/2 - n - 1]
   else:          raw[n] = reverse ? right[bars - n - 1] : right[n - bars/2]
```
i.e. `[L_high .. L_low | R_low .. R_high]`. `reverse=1` gives lows on the outside. Split-orientation with `split_stereo`: `[L_low..L_high | R_low..R_high]` (reverse flips each half). Mono output from stereo input: `raw[n] = (L[n]+R[n])/2` (or L or R per `mono_option`), `reverse` → `raw[bars-n-1]`. Stereo requires an even bar count (≥2). Bar count: `bars = (width + spacing) / (bar_width + spacing)`; centering remainder `(width - bars*bar_width - bars*spacing + spacing)/2`.

### A.9 Terminal rendering (noncurses)

Bars are drawn with 1/8 block glyphs: heights in eighths; glyphs `' ' ▁▂▃▄▅▆▇█` (U+2581..U+2588; top-down variant uses ▔ U+2594 and ▀ U+2580 for flipped). It **diffs against previous frame**: per cell compares `current_cell = bar - line*8` vs `prev_cell` from the previous frame; skips if both <1, both >7, or equal; otherwise emits a cursor-move (`ESC[row;colH` / `ESC[nC`) and the glyph. Colors: `ESC[38;2;r;g;bm` truecolor, gradient per row (line) when `gradient=1`. Whole frame built into one buffer then written once.

### A.10 Porting notes (Rust)

* Use `realfft`/`rustfft`: plan two R2C FFTs (bass 2N, treble N) per channel.
* Keep state arrays `fall, peak, prev, mem` of length `bars*channels`, `sens`, `sens_init`, `framerate` EMA (init 75), `frame_skip`.
* Feed int16-range f64 samples (i.e. don't normalise to ±1 in LINEAR mode, or multiply `eq` by 32768 if you do).
* If you call `execute` at a fixed render rate with whatever samples arrived, the EMA framerate makes gravity/integral/autosens frame-rate independent (`66/fps` normalisation).

---

## Part B — TerminalTextEffects engine architecture

Source: `terminaltexteffects/engine/{base_character,motion,animation,base_effect,terminal,canvas}.py`, `utils/{easing,geometry,graphics}.py`. ttfx mirrors this 1:1 in `src/engine/*` (reference port) and `src/fx/*` (optimized struct-of-arrays engine, same output).

### B.1 Coordinates & canvas

* `Coord{column, row}`, integers, **1-based, (1,1) = bottom-left; row grows upward**. Output rows are reversed when printing (`'\n'.join(rows[::-1])`).
* `TERMINAL_ROW_SCALE = 2`: every distance/length computation multiplies the row delta by 2 (cells are ~2:1). Circles use `row_radius = radius // 2` → ellipses that look round.
* Canvas: `top,right,bottom=1,left=1`; `text_{left,right,top,bottom}` bounds of the input text; `text_center`; `random_coord(outside_scope=..)` (random point just outside canvas, used for "fly in from off-screen"). Canvas size defaults to text size (`-1`), `0` = terminal size. Anchors (`sw` default) for canvas in terminal and text in canvas.
* Characters: `EffectCharacter{id, input_symbol, input_coord, is_visible, layer, motion, animation, event_handler, is_fill_character, links, neighbors}`. Fill characters (spaces inside/outside text) can be requested; `add_character(symbol, coord)` creates extra "added" characters (e.g. beams' beam heads, fireworks shells are just re-used input chars).

### B.2 Character selection helpers (Terminal)

* `get_characters(sort=...)`: default order is `(-row, column)` i.e. top-to-bottom, left-to-right. Sorts: `RANDOM, TOP_TO_BOTTOM_LEFT_TO_RIGHT, TOP_TO_BOTTOM_RIGHT_TO_LEFT, BOTTOM_TO_TOP_LEFT_TO_RIGHT, BOTTOM_TO_TOP_RIGHT_TO_LEFT, OUTSIDE_ROW_TO_MIDDLE, MIDDLE_ROW_TO_OUTSIDE, SPIRAL_{CLOCKWISE,COUNTER_CLOCKWISE}{,_DOUBLE,_QUAD}`.
* `get_characters_grouped(grouping)`: returns `Vec<Vec<Char>>`:
  * `COLUMN_LEFT_TO_RIGHT/RIGHT_TO_LEFT` (key column), `ROW_BOTTOM_TO_TOP/TOP_TO_BOTTOM` (key row),
  * `DIAGONAL_BOTTOM_LEFT_TO_TOP_RIGHT` / reverse: key `row + column`; `DIAGONAL_TOP_LEFT_TO_BOTTOM_RIGHT` / reverse: key `column - row`,
  * `DIAMONDS_CENTER_TO_OUTSIDE` / reverse: key `|col-cx| + |row-cy|` (Manhattan from text_center),
  * `CIRCLE_CENTER_TO_OUTSIDE` / reverse: key `ceil(hypot(col-cx, (row-cy)*2))`.
  Within a group, chars sorted by `(row, column)`.
* `neighbors` map: `north/south/east/west/(diagonals)` → used by spanning-tree effects (burn, laseretch, pour, binarypath). Spanning trees in `utils/spanningtree/` (PrimsSimple, PrimsWeighted, RecursiveBacktracker, BreadthFirst).

### B.3 Motion: Waypoint / Segment / Path / Motion

```
Waypoint { id, coord, bezier_control: Option<Vec<Coord>> }   // control points of the segment ENDING here
Segment  { start: Waypoint, end: Waypoint, distance: f64, enter_fired, exit_fired }
Path     { id, speed: f64 (>0, cells per tick), ease: Option<EasingFn>, layer: Option<i32>,
           hold_time: u32 (ticks), loop: bool, waypoints, segments, total_distance,
           current_step, max_steps, hold_time_remaining, last_distance_reached, origin_segment }
```
* Adding waypoint k (k≥1): segment distance = `bezier_length(prev, controls, coord)` or `hypot(dc, dr*2)`; `total_distance += d`; `max_steps = round(total_distance / speed)` (Python round = banker's).
* Bezier: arbitrary-order De Casteljau `find_coord_on_bezier_curve(start, controls, end, t)` → rounded Coord. Length: recursive subdivision, `(polygon + chord)/2` when `polygon - chord <= 1e-4` or depth 12, in row-scaled space.
* `activate_path(p)`: inserts/replaces an **origin segment** from the character's *current* coord to waypoint[0] (so paths always start where the char is); recomputes `total_distance`, `max_steps`; resets step/hold, clears segment event flags; sets `layer` if path has one; fires `PATH_ACTIVATED`.
* `Path.step()` each tick:
  ```
  if max_steps==0 or current_step>=max_steps or total_distance==0: return last waypoint coord
  current_step += 1
  f = ease ? ease(current_step/max_steps) : current_step/max_steps
  dist = f * total_distance ; last_distance_reached = dist
  walk segments subtracting distances, firing SEGMENT_ENTERED(end wp)/SEGMENT_EXITED(end wp) once each
  t = dist / seg.distance   (clamped to ≤1 only when no ease; eases like out_back may overshoot)
  coord = seg.end.bezier ? bezier(seg.start, ctrl, seg.end, t) : lerp(start,end,t) (rounded)
  ```
* `Motion.move()` per tick: `previous_coord = current`; step; when `current_step == max_steps`: fire final SEGMENT_EXITED; then hold: first tick fires `PATH_HOLDING`, count down `hold_time`; then if `loop` re-activate the same path, else deactivate and fire `PATH_COMPLETE`.
* `chain_paths([p1,p2,..], loop)`: registers PATH_COMPLETE(p_i) → ACTIVATE_PATH(p_{i+1}).
* Speed is in **row-scaled cells per tick**; a vertical move of 1 row costs distance 2.

### B.4 Animation: CharacterVisual / Frame / Scene / Animation

```
CharacterVisual { symbol, bold, dim, italic, underline, blink, reverse, hidden, strike,
                  colors: ColorPair{fg: Option<Color>, bg: Option<Color>}, formatted_symbol (precomputed SGR + sym + reset) }
Frame  { visual, duration: u32 ticks (>=1), ticks_elapsed }
Scene  { id, is_looping, sync: Option<DISTANCE|STEP>, ease: Option<EasingFn>, frames, played_frames,
         easing_total_steps = Σ durations, easing_current_step, frame_end_steps (prefix sums) }
Animation { scenes, active_scene, current_character_visual, existing_color_handling }
```
* `scene.add_frame(symbol, duration, colors, bold..)`.
* `scene.apply_gradient_to_symbols(symbols, duration, fg_gradient, bg_gradient)`: one frame per gradient colour (or per symbol, whichever list is longer), distributing the shorter list cyclically/evenly across the longer (`cyclic_distribution`: each element of the smaller seq repeated `len_big // len_small` times, with the remainder spread one extra per element early). This is the standard way effects make "fade from colour A to B over N frames".
* Stepping (`step_animation`, called after `motion.move()` in `character.tick()`):
  * **plain**: show current frame; `ticks_elapsed++`; when == duration advance; at end: loop → index 0, else scene complete.
  * **sync=STEP/DISTANCE**: frame index = `round((len-1) * progress)`, progress = `current_step/max_steps` or `last_distance_reached/total_distance` of the active path. If no active path → last frame and complete. (Colour follows motion.)
  * **ease**: `ratio = easing_current_step / max(total_steps-1, 1)`; `idx = round(ease(ratio) * (total_steps-1))`; frame = first frame whose prefix-sum end > idx (`bisect_right`); `easing_current_step++`, done at total.
  * On completion: `reset_scene()`; if not looping `active_scene = None` and fire `SCENE_COMPLETE`.
* `set_appearance(symbol, colors)` sets a static visual (no scene).
* `adjust_color_brightness(color, factor)`: RGB→HSL, `L = clamp(L*factor, 0, 1)`, HSL→RGB. Used for dimming (e.g. rain, spotlights, synthgrid, vhstape).
* `existing_color_handling` (`ignore` default | `always` | `dynamic`): whether input ANSI colours from the parsed text override scene colours.

### B.5 Events

`Event ∈ {SEGMENT_ENTERED(waypoint), SEGMENT_EXITED(waypoint), PATH_ACTIVATED(path), PATH_COMPLETE(path), PATH_HOLDING(path), SCENE_ACTIVATED(scene), SCENE_COMPLETE(scene)}`;
`Action ∈ {ACTIVATE_PATH, ACTIVATE_SCENE, DEACTIVATE_PATH, DEACTIVATE_SCENE, RESET_APPEARANCE, SET_LAYER(i), SET_COORDINATE(c), CALLBACK(fn, args)}`.
`register_event(event, caller, action, target)`; handlers run synchronously inside `move()`/`step_animation()` (motion code re-checks "path generation" to abort a step if a handler switched paths). This is what builds multi-phase choreography without per-effect state machines, e.g. `PATH_COMPLETE(fall) → ACTIVATE_SCENE(land)`, `SEGMENT_ENTERED(wp_mid) → ACTIVATE_SCENE(flash)`.

In Rust: store events as `HashMap<(EventKind, CallerId), Vec<Action>>` per character, or better an enum-indexed small Vec; effects needing complex logic can use a `Callback(fn(&mut Char, &mut EffectState))`.

### B.6 Effect iterator & frame loop

```
struct EffectIterator { config, terminal, pending: Vec<Char>, active_characters: Set<Char>, phase... }
fn next() -> Option<String>:
    effect-specific: move chars from pending to active (per-tick counts / delays / group releases),
                     set_character_visibility(c, true), activate paths/scenes
    update(): for c in active: c.tick()  (motion.move(); animation.step_animation())
              remove c where !is_active  (is_active = active_scene not complete || active_path.is_some())
    if pending.empty && active.empty (and phase==done): return None
    return terminal.get_formatted_output_string()
```
Global config everywhere: `final_gradient_stops`, `final_gradient_steps`, `final_gradient_direction` — the colour a character ends on is `final_gradient.build_coordinate_color_mapping(text bounds, direction)[input_coord]`.

### B.7 Colour & gradients (`utils/graphics.py`)

* `Color(hex | xterm-256 int)`; stored as rgb hex; xterm→hex via table; with `--xterm-colors` hex→nearest xterm.
* `Gradient(*stops, steps: int | tuple, loop=false)`:
  ```
  if 1 stop: spectrum=[stop]
  stops' = loop ? stops + [stops[0]] : stops
  spectrum = [stops'[0]]
  for each consecutive pair (a,b) with step count s (tuple broadcast: last value repeats):
      for i in 1..s: spectrum.push(round(a + (b-a) * i/s))  (per channel)
      spectrum.push(b)
  // len = 1 + Σ s
  get_color_at_fraction(f) = spectrum[round(f * (len-1))]
  ```
* `build_coordinate_color_mapping(min_row,max_row,min_col,max_col, direction)`:
  * VERTICAL: `f = (row-min_row)/row_span` (bottom = first stop!)
  * HORIZONTAL: `f = (col-min_col)/col_span`
  * RADIAL: `f = hypot(col-cx, (row-cy)*2) / hypot(w/2, h/2*2)` (centre = first stop)
  * DIAGONAL: `f = ((row-min_row)*2 + (col-min_col)) / (row_span*2 + col_span)`
* `shift_color_towards(c, target, factor)` = rgb lerp; `random_color()`.
* Typical TTE defaults: `final_gradient_steps = 12`, `final_gradient_direction = VERTICAL` or DIAGONAL, many effects use stops like `8A008A, 00D1FF, FFFFFF` (see per-effect sections).

### B.8 Easing functions (`utils/easing.py`) — input t∈[0,1]

| name | formula |
|---|---|
| linear | t |
| in_sine | 1 - cos(tπ/2) |
| out_sine | sin(tπ/2) |
| in_out_sine | -(cos(πt) - 1)/2 |
| in_quad / out_quad | t² / 1-(1-t)² |
| in_out_quad | t<.5 ? 2t² : 1-(-2t+2)²/2 |
| in_cubic / out_cubic | t³ / 1-(1-t)³ |
| in_out_cubic | t<.5 ? 4t³ : 1-(-2t+2)³/2 |
| in_quart / out_quart | t⁴ / 1-(1-t)⁴ |
| in_out_quart | t<.5 ? 8t⁴ : 1-(-2t+2)⁴/2 |
| in_quint / out_quint | t⁵ / 1-(1-t)⁵ |
| in_out_quint | t<.5 ? 16t⁵ : 1-(-2t+2)⁵/2 |
| in_expo | t==0 ? 0 : 2^(10t-10) |
| out_expo | t==1 ? 1 : 1-2^(-10t) |
| in_out_expo | 0/1 at ends; t<.5 ? 2^(20t-10)/2 : (2-2^(-20t+10))/2 |
| in_circ | 1-√(1-t²) |
| out_circ | √(1-(t-1)²) |
| in_out_circ | t<.5 ? (1-√(1-(2t)²))/2 : (√(1-(-2t+2)²)+1)/2 |
| in_back | c3 t³ - c1 t², c1=1.70158, c3=c1+1 |
| out_back | 1 + c3(t-1)³ + c1(t-1)² |
| in_out_back | c2=c1·1.525; t<.5 ? ((2t)²((c2+1)2t - c2))/2 : ((2t-2)²((c2+1)(2t-2)+c2)+2)/2 |
| in_elastic | c4=2π/3; 0/1 at ends; -2^(10t-10)·sin((10t-10.75)c4) |
| out_elastic | 2^(-10t)·sin((10t-0.75)c4) + 1 |
| in_out_elastic | c5=2π/4.5; t<.5 ? -(2^(20t-10) sin((20t-11.125)c5))/2 : (2^(-20t+10) sin((20t-11.125)c5))/2 + 1 |
| in_bounce | 1 - out_bounce(1-t) |
| out_bounce | n1=7.5625,d1=2.75: t<1/d1: n1t²; t<2/d1: n1(t-1.5/d1)²+.75; t<2.5/d1: n1(t-2.25/d1)²+.9375; else n1(t-2.625/d1)²+.984375 |
| in_out_bounce | t<.5 ? (1-out_bounce(1-2t))/2 : (1+out_bounce(2t-1))/2 |
| make_easing(x1,y1,x2,y2) | CSS cubic-bezier: solve x(t)=p by Newton + bisection (tol 1e-12), return y(t) |

Helpers: `EasingTracker{fn, total_steps=100, clamp}` → `step()` returns eased value of `current_step/total_steps`, plus `step_delta`; `SequenceEaser{sequence, fn, total_steps}` → each step reveals `sequence[..int(eased*len)]`, returns newly `added` (or `removed`) items — used for "ease the release of groups" (highlight, colorshift, etc.).

### B.8b Output to the terminal

* `prep_canvas()`: hide cursor (`ESC[?25l`), print `visible_top` blank lines (reserve area), `ESC 7` (DEC save cursor).
* Every frame: `ESC 8` (restore) + `ESC 7` + `ESC[{visible_top}A` (cursor up to top of area), then write the **entire frame** (all rows joined by `\n`, each cell = pre-formatted `SGR+symbol+ESC[0m`, blanks as spaces). **No diffing in Python TTE; full redraw every frame.** ttfx builds the same bytes but maintains the grid incrementally (dirty-row patching of its internal buffer, one `writev` iovec per row) — the terminal still receives a full frame each tick.
* Painter order: visible characters sorted by `(layer, character_id)`; higher layer wins a cell. Only `is_visible` chars render; chars outside canvas are clipped.
* Frame rate: `frame_rate=60` default; `enforce_framerate` sleeps `1/fps - elapsed`. Each frame = one engine tick (all speeds/durations are in ticks, i.e. effects are frame-locked, not time-based).
* End: show cursor, newline.

Recommendation for your Rust screensaver: keep the TTE tick model (ticks, not seconds) for effect logic, but render into a cell grid and diff against the previous frame (cava-style) or use crossterm/ratatui's buffer diff; wrap frames in synchronized-output mode (`ESC[?2026h … ESC[?2026l`) to avoid tearing.

---

## Part C — Effects (from Python originals)

Conventions: "speed" = cells/tick in row-scaled distance (see B.3); durations are ticks (frames @60 fps). `final_gradient` = mapping of input coord → colour using the effect's final gradient stops/steps/direction. Unless noted, every effect ends with each character at its input coord showing its final-gradient colour.
### TTE effects, part 1: beams, blackhole, burn, crumble, decrypt, expand, highlight

Source: terminaltexteffects (Python, MIT, ChrisBuilds), current `main` at clone time (the ttfx Rust port copies v0.15.0 exactly).

Conventions used below (engine semantics these effects depend on):
- Coordinates are `Coord{column,row}` with row 1 = **bottom** (TTE counts rows bottom-up). `canvas.top` is the highest row number.
- `Gradient(stops..., steps=N | (n1,n2,..))` makes a spectrum: for each consecutive pair of stops, `n` interpolated colours (steps can be a tuple, one per pair, and the last entry repeats). `gradient.spectrum[-1]` is the last colour.
- `scene.apply_gradient_to_symbols(symbols, duration, fg_gradient)` makes frames that walk the symbols and the gradient spectrum together, so the frame count is max(len(symbols), len(spectrum)) and each frame lasts `duration` ticks.
- `Scene.sync = DISTANCE`: the frame shown is picked by how far the character has gone along its active path (`index = round((n_frames-1) * progress)`), not by ticks.
- `path.speed` is cells moved per frame along the path's total length. `ease` remaps progress t in [0,1]. A waypoint may have a `bezier_control`, which makes the segment a Bezier (one control point = quadratic, two = cubic; arbitrary order via De Casteljau).
- Events are `(PATH_ACTIVATED | PATH_COMPLETE | SCENE_COMPLETE, target) -> (ACTIVATE_PATH | ACTIVATE_SCENE | SET_LAYER n | CALLBACK)`.
- `adjust_color_brightness(c, f)`: RGB to HSL, then `L = min(L*f, 1)` (clamped), then back to RGB.
- `TERMINAL_ROW_SCALE = 2`. `find_coords_on_circle(origin, r, limit)` makes an ellipse with x radius `r` and y radius `r//2`, over `limit` evenly spaced angles (default limit = round(2πr)), with duplicates removed.
- The final gradient is mapped over the text bounding box: `build_coordinate_color_mapping(bottom, top, left, right, direction)`.
- The effect ends when `active_characters` is empty and the phase is complete. A character leaves `active_characters` once it has no active path and no active scene. A scene that ends does not reset the look: the last frame stays as the character's appearance.

---

### beams

**Defaults.** beam_row_symbols `▂ ▁ _`. beam_column_symbols `▌ ▍ ▎ ▏`. beam_delay 6 frames. beam_row_speed_range (15,60) and beam_column_speed_range (9,15), both in tenths of a cell per frame. beam_gradient stops `#ffffff #00D1FF #8A008A`, steps (2,6), frames 2. final_gradient stops `#8A008A #00D1FF #ffffff`, steps 12, frames 4, direction VERTICAL. final_wipe_speed 3 (diagonal groups per frame).

**Setup.**
- Build one Group for each row and each column, fill characters included, so every cell of the canvas is covered.
- Each group gets `speed = randint(lo,hi)*0.1`.
- Sort a row group by column and a column group by row. Reverse the order with probability 0.5.
- Shuffle all groups into a `pending` deque.
- Each character gets three scenes:
  - `beam_row`: `apply_gradient_to_symbols(row_symbols, 2, beam_gradient)`, then a fade tail of `apply_gradient_to_symbols(input_symbol, 2, Gradient(final, brightness(final,0.3), steps=10))`.
  - `beam_column`: the same, with the column symbols.
  - `brighten`: `Gradient(brightness(final,0.3) -> final, steps=10)`, 4 ticks per frame.
- `final` is the gradient-mapped colour, or `#000000` for fill characters. Characters start invisible.

**Phase `beams`.**
- When `delay` reaches 0, move `randint(1,5)` groups from pending to active and set delay = 6. Otherwise `delay -= 1`.
- For each active group, add `speed` to its counter. Then, `int(counter)` times: `counter -= 1` and take the next character in the group. If that character already has a scene running, reset it (restart the beam on it). Otherwise make it visible and add it to the active set. Either way, activate `beam_<dir>`.
- Drop completed groups.
- When nothing is pending, no group is active and no character is active, switch to `final_wipe`.

The beam's look comes from this: a character plays the bright head of the gradient first and decays through the purple into a dim version of its final colour. Characters activated one after another along a line therefore form a comet with a tail.

**Phase `final_wipe`.** Groups are diagonals from top-left to bottom-right. Each frame pops 3 diagonal groups and activates `brighten` on them (dim to full colour over 10 steps × 4 ticks). Done when the groups run out.

```
groups = rows∪cols (with fill) each {chars sorted, maybe reversed, speed, ctr, idx}; shuffle
loop phase beams: if delay==0 {activate rand(1..=5) groups; delay=6} else delay-=1
  for g in active { g.ctr+=g.speed; for _ in 0..g.ctr as int { g.ctr-=1; c=g.next(); restart_or_show(c,"beam_"+dir) } }
phase wipe: 3 diagonals/frame -> scene brighten
```

---

### blackhole

**Defaults.** blackhole_color `#ffffff`. star_colors `#ffcc0d #ff7326 #ff194d #bf2669 #702a8c #049dbf`. final_gradient `#8A008A #00D1FF #ffffff`, steps 9, direction DIAGONAL.

**Sizes.**
- `radius = 2 * max(min(round(w*0.3), round(h*0.2)), 3)` in column units.
- `ring_count = radius*3 // 2`.
- The radius is clamped to `min(radius, (w-1)//2, ((h-1)//2)*2)`. If it had to shrink, the layout is "compact".
- Ring positions are `find_coords_on_circle(center, radius, ring_count)`, sampled evenly down to ring_count. If the radius is below 2, the ring is the rectangle perimeter instead.

**Setup.**
- Pick ring characters at random from the input characters. If the input is too small, add helper `*` characters at the centre.
- Each ring character gets:
  - path `blackhole`: speed 0.7, `in_out_sine`, one waypoint at its ring slot. Activating it sets layer 1.
  - scene `blackhole`: frame `*` in blackhole_color.
  - path `blackhole_rotation`: speed 0.45, `loop=True`. Its waypoints are all ring positions rotated so they start at its own slot, so the whole ring spins.
- Every character becomes visible as a star:
  - symbol from `* ' ` ¤ • ° ·`
  - colour from `Gradient(#4a4a4d, #ffffff, steps=6).spectrum`
- Characters not on the ring are placed at `canvas.random_coord()`. Each gets:
  - path `singularity`: speed `uniform(0.17,0.3)`, `in_expo`, to the centre.
  - consumed scene: `Gradient(star_color -> #000000, steps=10)`, 1 tick per frame, then `' '`, with `sync=DISTANCE` (fades out as it nears the centre). Path activation sets layer 2 and starts this scene.
- Shuffle the characters waiting to be consumed.

**Phases.**
1. `forming`: `formation_delay = max(100 // n_ring, 6)`. Every `formation_delay` frames, pop one ring character: activate path `blackhole` and scene `blackhole`, make it visible. Once all are placed and the active set is empty, activate `blackhole_rotation` on every ring character, then go to `consuming`.
2. `consuming`: activate `singularity` on all waiting stars in one frame. Their random speeds and in_expo stagger the arrivals. Wait until the active set is a subset of the ring characters (only the looping rotation is left).
3. `collapsing`, run once:
   - Target ring: in normal layout, `find_coords_on_circle(center, radius + 6, n_ring)`; in compact layout, the same ring.
   - Each ring character gets `expand_path` (speed 0.2, in_expo, to its target), then on complete `collapse_path` (speed 0.3, in_expo, to the centre).
   - The first ring character also gets a point scene: `◦ ◎ ◉ ● ◉ ◎ ◦` played 3 times, 3 ticks per frame, each frame a random star colour. It starts when that character's collapse completes, and sets layer 3.
4. `exploding`: once every ring character has no active path and no active scene:
   - Hide the helper characters.
   - For every character, set `nearby = find_coords_on_circle(input_coord, 6, 5)[rand 0..5]`.
   - Path 1: speed `randint(3,4)/10`, out_expo, to `nearby`. Path 2: speed `randint(4,6)/100`, in_cubic, back to `input_coord`.
   - `explode_scn` is the input symbol in a random star colour. When path 1 completes, `cooling_scn` starts: `Gradient(star_color -> final, steps=10)`, 20 ticks per frame.
   - Set phase = complete and run until the active set drains.

---

### burn

**Defaults.** starting_color `#837373`. burn_colors `#ffffff #fff75d #fe650d #8A003C #510100`. smoke_chance 0.5. final_gradient `#00c3ff #ffff1c`, steps 12, direction VERTICAL. Smoke pool max 2000 particles; symbols are drawn at random from `. , ' ` # *`.

**Order: random spanning tree, simple Prim's.**
- Start at a random character inside the text boundary.
- Keep an `edge` list, starting with that character.
- Each step: pop a random edge character. If it has unvisited 4-neighbours inside the text boundary, link one of them at random and append it to `link_order`. Push the current character back if it has more unvisited neighbours. Push the new one if it has any.
- Stop when `edge` is empty. Run to completion at build time.

This gives an organic, spreading "fire front" order.

**Scenes.**
- All characters start visible with the input symbol in `starting_color`.
- `burn`: `apply_gradient_to_symbols(["'", '.', '▖', '▙', '█', '▜', '▀', '▝', '.'], 4, fire_gradient)`, where fire_gradient is burn_colors with steps=10 (about 40 colours, so about 40 frames × 4 ticks).
- On burn SCENE_COMPLETE:
  - Start final_color_scn: `Gradient(fire.spectrum[-1] (#510100) -> final, steps=8)`, 4 ticks per frame.
  - Callback: emit smoke with probability 0.5. The smoke particle is placed at `input_coord`, with path speed 0.5 to `(col + randint(-4,4), canvas.top+1)`. Its smoke scene is `Gradient(#504F4F -> #C7C7C7, steps=9)`, 10 ticks per frame, and the particle goes back to the pool when the scene completes. Newer particles get lower layers.

**Loop.** Each frame, pop `randint(2,4)` characters from `link_order`. Skip non-burnable ones (spaces without colour). Activate `burn` on the rest.

---

### crumble

**Defaults.** final_gradient `#5CE1FF #FF8C00`, steps 12, direction DIAGONAL.

**Per character (non-dynamic colour mode).** Let `final` be the mapped colour.
- `weak = brightness(final, 0.65)` and `dust = brightness(final, 0.55)`.
- Initial appearance: the input symbol in `weak`, visible.
- `weaken` scene: `Gradient(weak -> dust, 9)`, 4 ticks per frame.
- `fall_path`: speed 0.65, **out_bounce**, to `(col, canvas.bottom)`.
- `dust` scene, `sync=DISTANCE`: 5 frames of random `* . ,` in `dust`.
- On weaken complete: start fall_path, set layer 1, start the dust scene.
- `top` path: speed 1, **out_quint**, waypoint `(col, canvas.top)` with **bezier_control = canvas center**. This is the vacuum swoop.
- `input` path: speed 1, linear, back to `input_coord`.
- When the input path completes, start the flash scene `Gradient(final -> #ffffff, 6)`, 4 ticks per frame, then the strengthen scene `Gradient(#ffffff -> final, 9)`, 4 ticks per frame.
- Shuffle all characters into `pending`.

**Phase `falling`.**
- Start with `fall_delay=12`, `min=9`, `max=12`, `group_max=1`.
- When `fall_delay` reaches 0: activate `weaken` on `randint(1, group_max)` characters and set `fall_delay = randint(min,max)`.
- With probability 0.6 (`randint(1,10)>4`), accelerate: `group_max += 1`, `min = max(0, min-1)`, `max = max(0, max-1)`.
- Otherwise `fall_delay -= 1`.
- Go to `vacuuming` when nothing is pending and nothing is active.

**Phase `vacuuming`.** Each frame, activate path `top` on `randint(3,10)` shuffled characters. Wait until the active set is empty.

**Phase `resetting`.** Activate path `input` on all characters at once. Wait until the active set is empty, then done.

---

### decrypt

**Defaults.** typing_speed 2. ciphertext_colors `#008000 #00cb00 #00ff00`. final_gradient `#eda000` (single stop), steps 12, direction VERTICAL.

**Cipher alphabet.** chr 33..126, chr 9608..9631 (blocks), chr 9472..9598 (box drawing), chr 174..451 (Latin extended).

**Scenes per character.**
- `typing`: frames `▉ ▓ ▒ ░`, each 2 ticks in a random cipher colour, then 1 random cipher symbol for 1 tick.
- `fast_decrypt`: one colour chosen per character. 80 frames of random cipher symbols, 2 ticks each (160 ticks).
- `slow_decrypt`: `randint(1,15)` frames of random symbols in the same colour. Each frame lasts `randrange(35,60)` ticks with probability about 30%, otherwise `randrange(3,6)`.
- `discovered`: `Gradient(#ffffff -> final, 10)`, 5 ticks per frame.
- Chain them with events: fast then slow then discovered.

**Phase `typing`.**
- In each frame, with probability about 75% (`randint(0,100) <= 75`), reveal the next `typing_speed` (2) characters in reading order (top-left first) and play `typing` on them.
- When nothing is left to type and the active set is empty, set active = all characters and activate `fast_decrypt` on each.

**Phase `decrypting`.** Run until everything is done. The random slow-frame durations make characters "lock in" at scattered times.

---

### expand

**Defaults.** movement_speed 0.35. expand_easing **in_out_quart**. final_gradient `#8A008A #00D1FF #FFFFFF`, steps 12, direction VERTICAL.

**Build, all characters at once.**
- Set the position to `canvas.center` and make the character visible.
- Path: speed 0.35, in_out_quart, to `input_coord`. It sets layer 1 when it starts and layer 0 when it finishes.
- Scene with `sync=DISTANCE`: `Gradient(final.spectrum[0] (#8A008A) -> final_color, 10)`, 5 ticks per frame. The colour is tied to how far the character has travelled.
- Activate the path and scene immediately.

**Loop.** Update until the active set is empty. The speed is per character along its own path, so far characters take longer: they all leave together but arrive at different times.

```
for c in chars { c.pos=center; c.path=[input] speed .35 in_out_quart; c.scene=grad(first->final,10) sync=distance }
```

---

### highlight

**Defaults.** highlight_brightness 1.75. highlight_direction `DIAGONAL_BOTTOM_LEFT_TO_TOP_RIGHT` (a CharacterOrder: diagonal groups). reverse False. highlight_width 8. final_gradient `#8A008A #00D1FF #FFFFFF`, steps 12, direction VERTICAL.

**Setup.**
- All characters start visible in their final colour `base`.
- `hl = brightness(base, 1.75)`.
- Scene `highlight`: `Gradient(base, hl, hl, base, steps=(3, width=8, 3))`, one frame per colour, 2 ticks each. That is about 14 colours: a ramp up, a plateau at the highlight colour, and a ramp down, which gives the specular band.

**Timing: SequenceEaser.**
- `groups` = the characters grouped by diagonal.
- An EasingTracker with `total_steps=100` and **in_out_circ**. Each frame: `step++`, `v = ease(step/100)`, `len = int(v * n_groups)` (forced to n_groups when complete).
- The groups in `[prev_len, len)` are "added" and get `highlight` activated.

The sweep therefore always lasts about 100 frames plus the scene length, whatever the text size, and it accelerates and decelerates through the middle.

```
easer=SequenceEaser(groups, in_out_circ, 100)
frame: for g in easer.step_added() { for c in g { c.play("highlight") } }; done when easer complete && active empty
```
### Effects part 2: laseretch, matrix, rain, slide, spotlights, swarm, synthgrid

Source: terminaltexteffects (Python, upstream HEAD, v0.15-era), `terminaltexteffects/effects/effect_*.py`.

Conventions used below, all from TTE:
- Coordinates are `Coord(column, row)`, 1-based. **Row 1 is the BOTTOM** row and `canvas.top` is the top row, so "row - 1" moves down.
- Path `speed` is in cells per frame along the path, using the curve's length. Easing maps the path progress `t` to a position on the curve.
- "Distance" in geometry helpers is terminal-adjusted, with the row difference scaled by `TERMINAL_ROW_SCALE = 2`.
- `Gradient(a, b, ..., steps=N)` gives N colours per segment between stops. The `spectrum` is that list, and `gradient[-3:]` means its last 3 colours.
- A scene frame is `(symbol, duration_frames, ColorPair)`. `apply_gradient_to_symbols(sym, dur, fg_gradient)` adds one frame per gradient colour.
- The "final colour" of each character comes from `final_gradient.build_coordinate_color_mapping(text_bottom, text_top, text_left, text_right, direction)`, or from the input ANSI colours when `existing_color_handling == 'dynamic'`. The dynamic-mode branches are ignored below unless relevant.
- An effect is finished when there is no pending work and `active_characters` is empty. A character leaves the active set once its active path and scene have both completed.

---

### laseretch

**Concept.** A diagonal "laser beam" runs from the current etch point up and to the right, off the top of the canvas. Each etched character appears as `^`, then cools from yellow through orange to its final colour. Sparks fall from the etch point along Bezier arcs and cool as they fall.

**Config defaults**
- `etch_pattern='algorithm'`: a recursive-backtracker spanning-tree walk (maze order). It can also be any CharacterOrder/Group/Sort, taken serpentine.
- `reverse_etch_pattern=False`
- `etch_speed=1`: characters etched per etch tick.
- `etch_delay=1`: frames waited between etch ticks, so a tick lands every 2 frames.
- `cool_gradient_stops=(#ffe680, #ff7b00)`
- `laser_gradient_stops=(#ffffff, #376cff)`
- `spark_gradient_stops=(#ffffff, #ffe680, #ff7b00, #1a0900)`
- `spark_cooling_frames=7`
- Final gradient: `(#8A008A, #00D1FF, #ffffff)`, steps 8, frames 4, direction VERTICAL.

**Etch order ('algorithm').**
- Start at a random character inside the text boundary.
- Run a randomized DFS over 4-neighbours limited to the text boundary: at each step, pick a random unvisited neighbour, link it and push it; if there is none, pop.
- `char_link_order`, the visit order, becomes `pending_chars`. It is reversed if `reverse_etch_pattern` is set.

**Per-character spawn scene** (activated at build time and played once the character becomes visible)
- Frame `'^'`, 3 frames, colour `#ffe680`.
- Then `Gradient(#ffe680, #ff7b00, final_color, steps=8)`, one frame per colour, 3 frames each, showing the input symbol. This gives about 17 colours, roughly 51 frames of cooling.

**Laser**
- The beam is a list of added characters built at start at `(col=r, row=r)` for `r = 0..=canvas.top`.
  - The first character is `'*'`, the rest are `'/'`. All are on layer 2 and visible.
  - Each has a looping scene of `Gradient(#ffffff, #376cff, steps=6, loop=True)` colours at 3 frames per colour.
  - The gradient deque is rotated by -1 per beam character, so the colour appears to travel along the beam.
- `reposition(target)`:
  - For each beam character i, `set_coordinate(target.col + i, target.row + i)`, running diagonally up and right from the target.
  - Then emit 1 spark.
- `disable()`: deactivate the scenes and hide the beam characters. This happens once `pending_chars` is empty.

**Sparks**
- A particle pool of 2000 characters with symbols `. , *`, on layer 2.
- Spark scene: `Gradient(spark stops, steps=(3,8))`, 7 frames per colour. The steps tuple is per segment, and the last value is reused for later segments.
- Emission:
  - Set the spark's coordinate to the laser position.
  - `target = (randint(pos.col-20, pos.col+20), canvas.bottom)`.
  - Path: `speed=0.3`, ease `out_sine`, one waypoint at `target` with `bezier_control = (target.col, pos.row + randint(-10, 20))`.
  - Activate the path and the 'spark' scene.
- The particle is reclaimed into the pool when its spark scene completes.

**Loop**
```
each frame:
  if char_delay == 0:
     repeat etch_speed: pop next pending char, skipping spaces with no input colours;
        make it visible and active; laser.reposition(char.input_coord)
     char_delay = etch_delay
  else char_delay -= 1
  if pending: active += beam_chars else laser.disable()
  update()
```

---

### matrix

**Concept.** Digital-rain columns fall for `rain_time` seconds of wall-clock time (not frames). In the "fill" phase every column fills completely. In the "resolve" phase random characters of each full column turn into the real text, fading from the highlight colour to their final colour.

**Config defaults**
- `highlight_color=#dbffdb`
- `rain_color_gradient=(#92be92, #185318)`, built into `rain_colors = Gradient(...steps=6)`.
- `rain_symbols`, which is COMMON plus KATA:
  - COMMON: `2 5 9 8 Z * ) : . " = + - ¦ | _`
  - KATA: half-width katakana `ｦｱｳｴｵｶｷｹｺｻｼｽｾｿﾀﾂﾃﾅﾆﾇﾈﾊﾋﾎﾏﾐﾑﾒﾓﾔﾕﾗﾘﾜ`
- `rain_fall_delay_range=(2,15)`: frames between one column advance and the next.
- `rain_column_delay_range=(3,9)`: frames between column launches.
- `rain_time=15` seconds.
- `symbol_swap_chance=0.005` and `color_swap_chance=0.001`, both per visible character per frame.
- `resolve_delay=3`
- Final gradient: `(#92be92, #336b33)`, steps 12, frames 3, direction RADIAL.

**Columns**
- Characters are grouped by column, left to right, including outer and inner fill characters so the whole canvas is covered.
- Each column list is reversed so it runs top to bottom, and the column list is shuffled.

**RainColumn state**, set up by `setup_column(phase)`:
- Hide all characters and reset them to their input coordinates. `pending = all chars` (top to bottom). `visible = []`. `drop_chance = 0.08`.
- `base_fall_delay`:
  - rain phase: `randint(2,15)`
  - fill phase: `randint(max(2//3,1), max(15//3,1))`, which is `randint(1,5)`.
- `length`:
  - rain phase: `randint(max(1, int(0.1*n)), n)`
  - fill phase: `n`
- `hold_time = randint(20,45)` if `length == n`, otherwise 0.

**Column tick**
```
if fall_delay==0:
   if pending: c=pending.pop_front(); c.symbol=random(rain_symbols), fg=highlight
               previous head (visible.last) -> fg=random(rain_colors); show c; visible.push(c)
   elif visible:
      if head still highlight -> recolour to random rain colour
      if hold_time: hold_time-=1
      elif phase=='rain':
          if rand<drop_chance: drop_column()   # every visible char moves row-1 (down); hide those below canvas.bottom
          trim_column()                        # hide visible[0] (the top/tail); if >1 left, fade new tail
   if len(visible)>length: trim_column()
   fall_delay = base_fall_delay
else fall_delay-=1
for each visible char: with p=0.005 swap symbol; with p=0.001 recolour to random rain colour
fade tail = set visible[0].fg = adjust_brightness(random(rain_colors[-3:]), 0.65)
```

**Driver (rain phase)**
- When `column_delay == 0`: activate `randint(1,3)` pending columns, then set `column_delay = randint(3,9)`.
- Tick all active columns.
- A column with no pending characters and no visible characters is set up again and pushed back to pending, so it recycles.
- Drop active columns that have no visible characters.
- After `rain_time` seconds:
  - Switch to phase `fill`.
  - For active columns, set `hold_time = 0` and `drop_chance = 1`.
  - Run `setup_column('fill')` on all pending columns.

**Fill phase**
- All pending columns are activated at once, with `column_delay = 1`.
- A column that finishes its pending characters in fill phase is marked full and added to `full_columns`.
- Once nothing is pending and every active column is a full fill column, go to `resolve`.

**Resolve phase**
- Keep ticking the full columns, which keeps the glyph and colour swaps going.
- Every `resolve_delay + 1` frames (the counter is shared globally), each full column pops `randint(1,4)` random visible characters.
  - A non-space character, or a character with input colours, activates its resolve scene: `Gradient(#dbffdb, final, steps=8)`, 3 frames each.
  - Otherwise the character is hidden.
- The effect ends when every full column is empty and every active character has finished. One extra final frame is emitted.

**Scene** `resolve`: 8 colours × 3 frames, showing the input symbol.

---

### rain

**Concept.** Every character starts at the top row of its column as a random raindrop glyph in a random blue. It falls straight down to its input position with `in_quart` easing, then fades from the raindrop colour to its final colour.

**Config defaults**
- `rain_colors = #00315C #004C8F #0075DB #3F91D9 #78B9F2 #9AC8F5 #B8D8F8 #E3EFFC`
- `movement_speed = (0.33, 0.57)`, uniform random per character.
- `rain_symbols = o . , * |`
- `movement_easing = in_quart`
- Final gradient: `(#488bff, #b2e7de, #57eaf7)`, steps 12, direction DIAGONAL.

**Build, per character**
- `drop = random(rain_colors)`.
- Rain scene: one frame of `random(rain_symbols)` in colour `drop`.
- Fade scene: `Gradient(drop, final, steps=7)` applied to the input symbol, 3 frames each.
- Start at `(input.col, canvas.top)`.
- Path: one waypoint at the input coordinate, speed `U(0.33, 0.57)`, ease `in_quart`.
- On PATH_COMPLETE, activate the fade scene.

**Ordering**
- Characters are grouped by row. The row with the **lowest** row number, the bottom row, is released first, so the text fills from the bottom up.
- Each frame:
  - If the current row's pending list is empty, load the next-lowest row.
  - Pop `randint(1,2)` random characters from it, make them visible and activate them.

```
groups = chars grouped by input row, ascending
each frame:
  if pending.empty && groups: pending = groups.pop_min()
  repeat rand(1..=2): if pending: c=pending.swap_remove(rand); show+activate c
  update()
```

---

### slide

**Concept.** Rows (or columns, or diagonals) slide in from off-canvas one character at a time. Groups are staggered by `gap` frames. There is an optional "merge" mode in which alternate groups come from opposite sides.

**Config defaults**
- `movement_speed = 0.8`
- `grouping = 'row'`; options are row, column and diagonal.
- `gap = 2`
- `reverse_direction = False`
- `merge = False`
- `movement_easing = in_out_quad`
- Final gradient: `(#833ab4, #fd1d1d, #fcb045)`, steps 12, frames 6, direction VERTICAL.

**Build**
- Groups are formed as follows:
  - row: ROW_TOP_TO_BOTTOM
  - column: COLUMN_LEFT_TO_RIGHT
  - diagonal: DIAGONAL_TOP_LEFT_TO_BOTTOM_RIGHT
- Each character gets path `input_path`: speed 0.8, `in_out_quad`, one waypoint at its input coordinate.
- **Row grouping**
  - Default: the group is reversed (rightmost character launches first) and every character starts at `(canvas.left-1, row)`. The whole row stacks at the left edge and fans out rightwards.
  - `merge` with an even group index: the group is not reversed and starts at `canvas.right+1`.
  - `reverse_direction` without merge: the group is reversed again and starts at `canvas.right+1`.
- **Column grouping**
  - The start is `canvas.bottom-1` (even groups when merging, or when reversed) or `canvas.top+1` (the default, with the group reversed).
- **Diagonal grouping**
  - Let `d = last.row - (canvas.bottom-1)`. The start is `(last.col - d, last.row - d)`, that is, off the bottom-left along the diagonal.
  - Merge on even groups, or reverse: reverse the group, then start at `(first.col + d', first.row + d')` with `d' = canvas.top+1 - first.row`, which is off the top-right.
- Gradient scene, activated immediately so it plays while the character moves: `Gradient(final_stops[0], final_color, steps=10)`, 6 frames per colour. Every character therefore starts purple (`#833ab4`) and shifts to its final colour.

**Loop**
```
each frame:
  if current_gap == gap && pending_groups: active_groups.push(pending_groups.pop_front()); current_gap=0
  elif pending_groups: current_gap += 1
  for g in active_groups: c=g.pop_front(); show; activate 'input_path'; active+=c
  drop empty groups; update()
```
With the default gap of 2, a new group starts every 3 frames. Each active group launches one character per frame.

---

### spotlights

**Concept.** The text starts dimmed to 20% brightness. Invisible "spotlight" points wander on looping Bezier paths and light up the characters inside a circle around them, with a soft falloff at the edge. After the search period all the spotlights converge on the centre and merge into one, and its radius grows until the whole text is lit.

**Config defaults**
- `beam_width_ratio = 2.0`
- `beam_falloff = 0.3`
- `search_duration = 550` frames
- `search_speed_range = (0.35, 0.75)`
- `spotlight_count = 3`
- Final gradient: `(#ab48ff, #e7b2b2, #fffebd)`, steps 12, direction VERTICAL.

**Build**
- Each character has two colour pairs, both taken from its final colour:
  - `bright = final`
  - `dark = adjust_brightness(final, 0.2)`
- Every character is made visible immediately, with `dark` set as a static appearance. No scenes are used; `set_appearance` is called directly each frame.
- `illuminate_range = max(int(min(S // 2.0, S)), 1)` where `S = min(canvas.right, canvas.top)`.
- **Spotlights**
  - Each spotlight is an added character `'O'`, never made visible, starting at a random coordinate outside the canvas.
  - Its targets are 11 coordinates: the first is random inside the canvas; each of the next 10 is a random coordinate at least `canvas.right // 4` away from the previous one (terminal-adjusted distance).
  - Each target is its own path: speed `U(0.35, 0.75)`, ease `in_out_quad`, one waypoint with `bezier_control = random coordinate outside the canvas`, which produces wide swooping arcs.
  - The paths are chained with `loop=True`.
  - Extra path `center`: speed 0.5, `in_out_sine`, waypoint at `canvas.center`.
  - At start, activate path '0'.

**Illuminate(range), each frame**
- Collect the coordinates within the circle of radius `range` around every spotlight, using `find_coords_in_circle`, which accounts for the aspect ratio.
- Take the characters at those input coordinates, skipping spaces that have no input colour.
- Characters that are no longer in range are set back to `dark`.
- For each character in range:
  - `d = min distance to any spotlight` (terminal-adjusted).
  - If `d > range*(1-falloff)`:
    - `bf = max(1 - (d - range*(1-falloff)) / (range*falloff), 0.2)`
    - colour = `adjust_brightness(bright, bf)`
  - Otherwise colour = `bright`.

**Loop**
```
illuminate(range)
if searching: search_duration -= 1; if 0: all spotlights activate 'center'; searching=false
if no spotlight has an active path:      # all arrived at centre
    keep only spotlights[0]; expanding=true; range += 1
    if range > max(canvas.right, canvas.top) / 1.5: complete
update()
```

`adjust_brightness` converts to HLS, sets `l = max(0, min(l*factor, 1))`, and converts back. That is TTE's `Animation.adjust_color_brightness`.

---

### swarm

**Concept.**
- The characters are split into swarms of about 10% of the text each.
- Each swarm spawns at one point off-canvas and flies through 2 to 4 "swarm areas", which are random circular clouds. The characters flash yellow while they travel.
- Individual characters wander between random points inside each area. When one character moves on to the next area, the others follow it with probability `coordination`.
- Finally every character flies to its input position and fades from the flash colour to its final colour.
- The next swarm launches as soon as the current one starts landing.

**Config defaults**
- `base_color = (#31a0d4,)`
- `flash_color = #f2ea79`
- `swarm_size = 0.1`, a fraction of the total character count.
- `swarm_coordination = 0.8`
- `swarm_area_count_range = (2, 4)`
- Final gradient: `(#31b900, #f0ff65)`, steps 12, direction HORIZONTAL.

**Making swarms**
- `size = max(round(N*0.1), 1)`.
- Sort the characters BOTTOM_TO_TOP_RIGHT_TO_LEFT and repeatedly `pop()` from the end into chunks of `size`.
- If the last chunk has fewer than `size//2` characters, merge it into the previous chunk.
- The iterator later takes `swarms.pop()`, the last chunk, first.

**Per swarm**
- Flash gradient: `g = Gradient(random(base_color), flash, steps=7)`. The mirror list is `g + [flash]*10 + reverse(g)`.
- `spawn = random coordinate outside the canvas`.
- `n_areas = randint(2,4)`.
- `R = 2 * max(min(right,top)//2, 1)`.
- **Finding the area centres**
  - Starting from `last = spawn`, for each area choose `next` = a random point on the circle of radius R around `last` that lies inside the canvas. The circle is sampled at `round(2πR/2)` points. If none of those points is inside the canvas, use a random canvas coordinate.
  - Record `area_coords[last] = find_coords_in_circle(last, max(min(right,top)//6, 1) * 2)`, then set `last = next`.
  - **Quirk:** the recorded cloud centres are `spawn` plus the first `n-1` chosen centres. The first cloud is therefore centred on the off-canvas spawn point.

**Per character in the swarm**
- Starts at `spawn`.
- Flash scene with `sync=DISTANCE`, meaning the frame index follows the fraction of path distance travelled: one frame per mirror colour, duration 1.
- For each area k:
  - Path `"{k}_swarm_area"`: speed 0.4, `out_sine`, waypoint = a random coordinate in cloud k.
    - On PATH_ACTIVATED: activate the flash scene and set layer 1.
    - On PATH_COMPLETE: deactivate the scene.
  - Then 2 inner paths: speed 0.18, `in_out_sine`, each to another random coordinate in the same cloud.
- Final `input_path`: speed 0.45, `in_out_quad`, to the input coordinate.
  - On PATH_ACTIVATED: activate the flash scene.
  - On PATH_COMPLETE: activate the input scene, which is `Gradient(flash, final, steps=10)` at 3 frames each, and set layer 0.
- All paths are chained in insertion order. There is no loop.

**Loop**
```
if swarms && call_next: call_next=false; current=swarms.pop(); active_area='0_swarm_area';
    for c in current: activate '0_swarm_area', show, active+=c
if active.len() < current.len(): call_next = true     # some chars finished -> launch next swarm
for c in current:   # coordination: first char seen on a higher-numbered swarm_area path leads
   if c.active_path.id contains 'swarm_area' and index(c.path) > index(active_area):
       active_area = c.path.id
       for other != c: with p=0.8 other.activate_path(other.paths[active_area])
       break
update()
```

---

### synthgrid

**Concept.**
- A synthwave grid of `─` and `│` lines grows across the whole canvas, split into balanced, roughly square cells.
- Text cells then "generate": each character cycles through the shade glyphs `░▒▓` in random text-gradient colours before settling on its real glyph. At most 10% of the cells are active at once.
- Finally the grid lines retract.

**Config defaults**
- `grid_gradient_stops = (#CC00CC, #ffffff)`, steps 12, direction DIAGONAL, mapped over the whole canvas rather than just the text.
- `text_gradient_stops = (#8A008A, #00D1FF, #FFFFFF)`, steps 12, direction VERTICAL, mapped over the text bounds.
- `grid_row_symbol = '─'` and `grid_column_symbol = '│'`
- `text_generation_symbols = ░ ▒ ▓`
- `max_active_blocks = 0.1`

**Grid layout**: `find_balanced_grid(left, bottom, right, top, target_cells=5, min_cell_width=4, min_cell_height=2)`.
- `visual_h = height*2`.
- `target = max(4, 2*2, min(max(w, visual_h)/5, w, visual_h))`.
- Candidate counts for each axis are the integers from `floor(ideal)-1` to `ceil(ideal)+1`, where `ideal = visual_len/target`, clamped to `[1, len // min]`.
- Score each pair as `(|ln(cw/ch)| + (|ln(cw/target)| + |ln(ch/target)|)/4, |max(cols,rows) - 5|, cols*rows)` and pick the minimum.
- Boundaries are `left + i*w//cols` for `i` in `0..=cols`, and likewise `bottom + i*h//rows`.

**Grid lines**
- If the layout is a single cell (2 boundaries on each axis), no lines are drawn.
- Horizontal lines are drawn at rows `{rows[0], rows[-1]-1, interior boundaries}`, if `canvas.height >= 3` and the minimum cell height is at least 2.
- Vertical lines are drawn at columns `{cols[0], cols[-1]-1, interior boundaries}`, if `canvas.width >= 3` and the minimum cell width is at least 2.
- Each line is a set of added characters spanning the full canvas. Coordinates already used by another line are skipped, so crossings are drawn once.
- Grid characters are on layer 2, each with a static frame coloured by the grid gradient at its coordinate. All start hidden, in the `collapsed` list.
- `extend()`: reveal the next 3 characters of a horizontal line or 1 of a vertical line, in order from the left or bottom.
- `collapse()`: once a line is fully extended, reverse the extended list, then hide 3 (or 1) characters per frame.

**Text cells**
- Characters are grouped by grid cell with `get_characters_grouped_by_grid`, which returns non-empty cells bottom to top, then left to right. The groups are shuffled.
- Dissolve scene, per character:
  - `randint(15,30)` frames of a random `░▒▓` glyph in a random colour from `text_gradient.spectrum`, 2 frames each.
  - Then the input symbol in its final colour, 1 frame.
  - Spaces get an empty ColorPair.
- On SCENE_COMPLETE, a callback decrements `group_tracker[group]`.

**Phases**
```
grid_expand: every line not extended -> extend(); when all extended -> add_chars
add_chars:   if pending && active_groups < total_groups*0.1: pop one group, show+activate all, tracker[g]+=len
             if !pending && no active chars && active_groups==0 -> collapse
collapse:    every line not collapsed -> collapse(); when all collapsed -> complete
after update: active_groups = count(tracker values != 0)
```
Only one new cell can start per frame, and only while fewer than 10% of cells are active.
### Effects part 3: unstable, vhstape, waves, wipe, print, fireworks, bouncyballs

Source: TTE Python `terminaltexteffects/effects/effect_*.py` (ttfx reproduces these frame for frame).

Conventions used by every effect below:
- Coordinates: `Coord(column, row)`, 1-based, **row 1 is the bottom** (rows increase upward). `canvas.top` = number of rows, `canvas.right` = number of columns. `text_*` is the bounding box of the input text.
- `Gradient(stops..., steps=n)`: the spectrum has `(len(stops)-1)*n + 1` colours, or 1 colour when there is only one stop. For example, 2 stops at steps 5 give 6 colours, 3 stops at steps 5 give 11, 5 stops at steps 6 give 25, and 3 stops at steps 12 give 25.
- `final_gradient_mapping = final_gradient.build_coordinate_color_mapping(text_bottom, text_top, text_left, text_right, direction)` maps each input coordinate to its resting colour. "final colour" below means this mapping.
- `scene.apply_gradient_to_symbols(symbols, dur, fg_gradient)` adds one frame (lasting `dur` ticks) per element of the longer of the two sequences (symbols or gradient colours). The shorter sequence is spread cyclically: each element repeats `len_long/len_short` times, and the leftover count goes to the earliest elements as one extra repeat each. For example, a 1-symbol list with a 13-colour gradient gives 13 frames of the same symbol.
- A `Scene` with `sync=STEP` does not advance on its own clock. Its frame index is `round((n_frames-1) * path.current_step / path.max_steps)` of the currently active motion path. A scene with `ease` spreads its frames over `total_duration` ticks, and the eased progress picks the frame.
- `self.update()` calls `tick()` on each active character (motion step plus animation step), then drops characters for which `is_active` is false (no active path and no active scene).
- `existing_color_handling == 'dynamic'` (keep the input's ANSI colours) is an alternative branch in every effect. Skip it for a first implementation. When it is used, the start colour for characters that have no input colour is the neutral grey `#808080`.

---

### unstable
Characters appear scrambled onto one another's positions, shake in bursts that get more frequent, fly to the canvas edges, then fly back to their correct positions.

**Defaults:** `unstable_color #ff9200`, `explosion_ease out_expo`, `explosion_speed 1`, `reassembly_ease out_expo`, `reassembly_speed 1`. The final gradient is `#8A008A → #00D1FF → #FFFFFF`, steps 12, **VERTICAL**. Internal constants: `max_rumble_steps = 150`, `rumble_mod_delay = 18` (the initial jolt interval), `explosion_hold_time = 30`.

**Build (per character):**
- `jumbled` = pop a random coordinate from the list of all input coordinates. This is a permutation, so each character starts on another character's cell. Call `set_coordinate(jumbled)`.
- The explosion target is a random edge cell. Pick `randint(0,3)`: 0 is the left column at a random row, 1 is the right column at a random row, 2 is the bottom row at a random column, 3 is the top row at a random column.
- Path `explosion`: speed 1, ease out_expo, one waypoint at the edge target.
- Path `reassembly`: speed 1, ease out_expo, one waypoint at `input_coord`.
- Scene `rumble`: `apply_gradient_to_symbols(sym, 10, Gradient(final, unstable_color, steps=12))`. That gives 13 frames × 10 ticks = 130 ticks, fading from the final colour to orange.
- Scene `final`: `apply_gradient_to_symbols(sym, 3, Gradient(unstable_color, final, steps=12))`. That gives 13 frames × 3 ticks.
- Activate `rumble` and make every character visible immediately.

**Frame loop:**
```
phase rumble: while step < 150:
   if step > 30 and step % mod_delay == 0:
       (dc,dr) = random choice of {-1,0,1} each      # one offset shared by ALL chars
       for c: c.set_coord(c.coord + (dc,dr)); c.step_animation()
       emit frame
       for c: c.set_coord(jumbled[c])               # snap back
       mod_delay = max(mod_delay-1, 1)              # jolts accelerate
   else: for c: c.step_animation(); emit frame
   step += 1
 then: every char activates 'explosion'; active = all
phase explosion: tick all active; keep those whose coord != edge target; emit
   when none are left: tick for 30 hold frames (nothing moves; this is a pause)
   then: activate the 'final' scene and the 'reassembly' path on all chars
phase reassembly: tick; keep chars that have not reached input_coord or whose final scene is unfinished
```
The animation is stepped only; motion is never ticked during the rumble. The shake is one whole-screen offset shared by all characters, lasting 1 frame. The colour drift to orange takes 130 ticks and then holds on the last frame.

---

### vhstape
The whole text is visible from the start. For about 600 frames, individual lines shift sideways with an RGB colour sweep, a 3-line "glitch wave" band rolls vertically, and occasional full-screen snow appears. Then there is a final snow pass, and the lines are redrawn one at a time from the top down with a white block flash.

**Defaults:**
- `glitch_line_colors` = `glitch_wave_colors` = `#ffffff #ff0000 #00ff00 #0000ff #ffffff`
- `noise_colors #1e1e1f #3c3b3d #6d6c70 #a2a1a6 #cbc9cf #ffffff`
- `glitch_line_chance 0.05`, `noise_chance 0.004`, `total_glitch_time 600`
- Final gradient `#ab48ff → #e7b2b2 → #fffebd`, steps 12, VERTICAL
- Snow chars `# * . :`

**Lines:** group the characters by row, bottom to top, so `lines[0]` is the bottom row. Each line gets `offset = randint(4,25)`, `direction = ±1` and `hold = randint(1,50)`, shared by the characters in that line. Per character, add these paths. All have speed 2 and a single waypoint:
- `glitch`: goes to `(col + offset*dir, row)` and holds there for `hold` ticks.
- `restore`: goes to `input_coord`.
- `glitch_wave_mid`: goes to `(col+8, row)`.
- `glitch_wave_end`: goes to `(col+14, row)`.

Scenes (every frame lasts 1 tick unless noted):
- `base`: the symbol in its stable (final gradient) colour.
- `rgb_glitch_fwd` (sync STEP): one frame per glitch-line colour.
- `rgb_glitch_bwd` (sync STEP): the same colours in reverse order.
- `rgb_glitch_wave` (sync STEP): one frame per wave colour.
- `snow`: 25 frames of a random snow char in a random noise colour, 2 ticks each, then 1 frame of the symbol in its stable colour.
- `final_snow`: 30 such frames, 2 ticks each.
- `final_redraw`: `'█'` in `#ffffff` for 6 ticks, then the symbol in its final colour.

Events:
- `glitch` complete → activate the `restore` path.
- `glitch` activated → play the `fwd` scene.
- `restore` activated → play the `bwd` scene.
- Either wave path activated → play the `wave` scene.
- `bwd` complete → play `base`.

Because the colour scenes are STEP-synced, the RGB sweep follows the character's travel.

**Line operations:**
- `glitch(final)`: glitch speed = `40/randint(20,40)` and restore speed = `40/randint(20,40)`, both in [1,2]. When `final` is set, the hold is 0. Then activate `glitch`.
- `restore()`: set a new random restore speed and activate `restore`.
- `snow()`: activate the `snow` scene.

**Glitch wave:**
- If no wave is running and `text_height >= 3`, set `wave_top = text_bottom + randint(max(3, round(h*0.5)), h)`.
- Each time all wave lines have finished moving:
  - With probability 0.3, `wave_top` changes by ±1: +1 with probability 0.3, −1 with probability 0.7. Otherwise it stays. Clamp to `[2, text_top]`.
  - The new band is the rows `wave_top-2 .. wave_top`.
  - Lines that have left the band get `restore()`.
  - If `wave_top < text_bottom+2`, restore all band lines and end the wave.
  - Otherwise the 3 band lines take the paths (`mid`, `end`, `mid`), so the middle row shifts 14 columns and the outer rows shift 8.

**Loop:**
```
glitching (600 frames):
  if the wave is idle or finished moving: glitch_wave()
  drop finished lines from active_glitch_lines
  if rand < 0.05 and len(active_glitch_lines) < 3:
      pick a random line; if it is not in the wave and not already glitching:
          hold = randint(20,75); line.glitch()
  if rand < 0.004: snow() on every line (a full-screen noise burst, 51 ticks)
  once 600 steps have elapsed: restore() every wave line and glitch line → phase noise
noise:  once active is empty: activate final_snow on all chars → phase redraw
redraw: once active is empty (and on every frame after that): pop the LAST line (the top row)
        and activate final_redraw on its chars. That is one line per frame, top to bottom.
        When no lines are left → complete (the effect ends once active empties)
update(); emit
```

---

### waves
Characters appear in groups ordered from the centre outward in circular bands. Each character first plays a block-height "wave" of `▁▂▃…█…▂▁` symbols cycling through a colour gradient 7 times (on one eased timeline), then fades into its final colour.

**Defaults:**
- `wave_symbols ▁▂▃▄▅▆▇█▇▆▅▄▃▂▁` (15 symbols)
- `wave_gradient_stops #f0ff65 #ffb102 #31a0d4 #ffb102 #f0ff65`, steps (6,), giving 25 colours
- `wave_count 7`, `wave_length 2` (ticks per frame)
- `wave_direction CIRCLE_CENTER_TO_OUTSIDE`: one-column-wide radial bands around the text midpoint, with row distance scaled ×2 for cell aspect
- `reverse false`, `travel_speed 1` (groups released per frame)
- `wave_easing in_out_sine`
- Final gradient `#ffb102 #31a0d4 #f0ff65`, steps 12, **DIAGONAL**

**Build per character:**
```
wave_scn.ease = in_out_sine
repeat 7x: wave_scn.apply_gradient_to_symbols(15 symbols, 2, fg=wave_gradient)
     # 25 colours > 15 symbols → 25 frames: each symbol ×1, and the first 10 symbols get one extra
     # repeat → 7*25 = 175 frames = 350 ticks. Ease is applied across the WHOLE scene
     # (the sine accelerates/decelerates the 7 passes as one span)
final_scn: for colour in Gradient(wave_gradient.last (#f0ff65), final, steps=12): add_frame(sym, 10, colour)
     # 13 frames × 10 ticks = 130 ticks
on wave_scn complete → activate final_scn; activate wave_scn
groups = get_characters_grouped(order=wave_direction, reverse)
```
**Loop:** each frame, pop `travel_speed` groups from the front, make them visible and add them to active, then run `update()`. There is no motion; this is a colour/symbol ripple only. The outward spread comes from the staggered start times, so the effect lasts about `n_groups + 480` frames.

---

### wipe
Groups (diagonal by default) are revealed according to an eased count over 100 easer steps. Each character fades from the gradient's first colour to its final colour.

**Defaults:**
- `wipe_direction DIAGONAL_TOP_LEFT_TO_BOTTOM_RIGHT`, `reverse false`
- `wipe_delay 0`: extra frames between easer steps
- `wipe_ease in_out_circ`
- Final gradient `#833ab4 → #fd1d1d → #fcb045`, steps 12, `final_gradient_frames 3`, VERTICAL

**Build:** scene `wipe` = `apply_gradient_to_symbols(sym, 3, Gradient(final_gradient.spectrum[0] (#833ab4), final, steps=12))`. That is 13 frames × 3 ticks.

**SequenceEaser(groups, ease, total_steps=100):** on each `step()`:
- `t = ease(step/100)`, clamped.
- `length = int(t * len(groups))`. On the final step, set `length = len` if `t≈1`.
- `added = groups[prev:length]` when `length` grows. `removed = groups[length:prev]` when it shrinks (possible with overshooting eases like `in_out_back`/`elastic`).

**Loop:**
```
if active or not easer.complete:
  if delay == 0:
     easer.step()
     for g in added:   for c in g: activate 'wipe', make visible, add to active
     for g in removed: for c in g: deactivate the scene, reset 'wipe', make INVISIBLE
     delay = wipe_delay
  else: delay -= 1
  update(); emit
```
With the defaults, the reveal takes 100 frames, and the last characters then need 39 more ticks to reach their colour.

---

### print
Text is "typed" row by row at the bottom row of the screen, like a line printer. Each completed row scrolls up by one. A white `█` print head travels back (eased carriage return) to the start of the next line.

**Defaults:**
- `print_head_return_speed 1.5`
- `print_speed 2`: characters typed per frame
- `print_head_easing in_out_quad`
- Final gradient `#02b8bd → #c1f0e3 → #00ffa0`, steps 12, DIAGONAL
- Typing head colour `#ffffff`
- Fill characters (spaces inside and around the text) are included. Coordinates with no mapped colour default to `#ffffff`.

**Row build** (rows from `ROW_TOP_TO_BOTTOM`, fill characters included):
- If a row is all spaces, keep only its first character, so a blank line still costs one step.
- Otherwise trim trailing fill after the rightmost non-fill column.
- Every character starts at `(col, 1)`, which is the bottom screen row.
- Typed scene: `apply_gradient_to_symbols(('█','▓','▒','░',sym), 3, Gradient(#ffffff, final, steps=5))`. That gives 6 frames: `█ █ ▓ ▒ ░ sym`, 3 ticks each, fading from white to the final colour.

**State:** the head is an extra character `'█'` added at `(1,1)`. `current_row = rows.pop(0)`. `last_column` is tracked.

**Loop:**
```
while active or typing:
  if head has an active path: pass (typing is paused during the carriage return)
  elif current_row has untyped chars:
      type up to print_speed chars (left to right): make visible, add to active; last_column = its column
  else:
      processed.append(current_row)
      if rows pending:
          every typed char of every processed row: row += 1   (scroll up one line)
          current_row = pending.pop(0)
          if neither the previous row nor the new row is all fill:
              trim the new row's leading fill (left_extent = min non-fill column)
          head.set_coord((last_column, 1)); make it visible; clear its paths
          path 'carriage_return' (speed 1.5, in_out_quad) → (first untyped column, 1)
          on path complete → callback that hides the head; add the head to active
      else: typing = false
  update(); emit
```
Note that the scroll works by incrementing the row. The text ends in its original layout only because each row is moved up once for every later row. The first row is moved up `n_rows - 1` times, so it lands at row `n_rows`, which is the top.

---

### fireworks
Characters are split into "shells" of about 5% of all characters each. The shells launch from the bottom one at a time at random intervals and rise, easing out, to an apex. There they burst outward to random points in a circle, droop in a Bezier arc, then swoop down and back up into their input positions.

**Defaults:**
- `explode_anywhere false`
- `firework_colors #88F7E2 #44D492 #F5EB67 #FFA15C #FA233E`
- `firework_symbol 'o'`
- `firework_volume 0.05`: fraction of characters per shell
- `launch_delay 45`: frames, with jitter ×U(0.5,1.5)
- `explode_distance 0.2`: fraction of canvas width
- Final gradient `#8A008A → #00D1FF → #FFFFFF`, steps 12, **HORIZONTAL**

Derived values:
- `volume = max(1, round(0.05 * n_chars))`
- `R = explode_distance = min(15, max(1, round(canvas.right * 0.2)))`

**Waypoints** (iterate the characters in input order, which is also how they are split into shells):
```
at the start of each shell:
   origin_x = randrange(left, right+1)
   min_row  = explode_anywhere ? canvas.bottom : char.input_row   (row of the shell's first char)
   origin_y = randrange(min_row, top+1)                          # apex at or above that text row
   circle   = find_coords_in_circle(origin, R)   # ellipse: x radius R, y radius R/2
                                                 # (TERMINAL_ROW_SCALE = 2); all integer cells inside
per char:
   start at (origin_x, canvas.bottom)
   apex_pth: speed 0.35, out_expo, layer 2 → origin
   explode_path: speed U(0.2,0.4), out_circ, layer 2:
       wp1 = random choice from circle
       ctrl = extrapolate_along_ray(origin, wp1, R//2, terminal_adjusted)
              # t = 1 + off/len, where len uses row_diff*2; point = lerp(origin, wp1, t), rounded
       wp2 = (ctrl.col, max(1, ctrl.row - 7)), with quadratic bezier control = ctrl   # droops 7 rows
   input_pth: speed 0.6, in_out_quart, layer 2 → input_coord, with bezier control (wp2.col, 1)
              # swoops down to the bottom row and curves up into place
   events: apex complete → explode; explode complete → input; input complete → set layer 0
   activate apex_pth
```
**Scenes** (shell colour = a random choice from `firework_colors`, one per shell):
- `launch` (looping): `'o'` in the shell colour for 2 ticks, then `'o'` in white for 1 tick. This twinkles during the ascent.
- `bloom` (sync STEP to the explode path): `Gradient(shell, #FFFFFF, shell, steps=5)` gives 11 frames of the input symbol, 2 ticks each. It flashes white at mid-burst.
- `fall`: `apply_gradient_to_symbols(sym, 10, Gradient(shell, final, steps=15))` gives 16 frames × 10 ticks.
- Events: `apex_pth` complete → play bloom. `input_pth` activated → play fall.

**Loop:**
```
while shells or active:
  if shells and launch_delay <= 0:
      g = shells.pop()   # pops from the END, so the last text characters launch first
      make visible; add to active
      launch_delay = int(45 * U(0.5,1.5))
  launch_delay -= 1; update(); emit
```

---

### bouncyballs
Each character starts above the screen as a coloured "ball" symbol. The balls drop row by row, bottom text row first, in random bursts of 2–6, bounce into place (out_bounce), then fade to the text symbol in its final colour.

**Defaults:**
- `ball_colors #d1f4a5 #96e2a4 #5acda9`
- `ball_symbols * o O 0 .`
- `ball_delay 4`: frames between bursts
- `movement_speed 0.45`, `movement_easing out_bounce`
- Final gradient `#f8ffae → #43c6ac`, steps 12, DIAGONAL

**Build per character:**
- Pick a random colour and a random ball symbol.
- `ball_scene` = 1 frame of the ball symbol in that colour. It is non-looping, so the symbol persists until it is replaced.
- `final_scene` = `apply_gradient_to_symbols(sym, 6, Gradient(ball_colour, final, steps=10))`, giving 11 frames × 6 ticks.
- Start at `(input_col, int(canvas.top * U(1.0, 1.5)))`, which is above the visible area.
- One path, speed 0.45 with out_bounce, to `input_coord`. When the path completes, play `final_scene`.
- Activate the path and `ball_scene`.
- Group the characters by `input_row` in ascending order, so row 1 (the bottom) comes first.

**Loop:**
```
while rows or active or pending:
  if pending is empty and rows remain: pending = rows.pop(min row)   # lowest row first
  if pending:
     if delay == 0:
        repeat randint(2,6): pop a RANDOM char from pending → visible, add to active
        delay = 4
     else: delay -= 1
  update(); emit
```
The bottom rows fill first and the text stacks upward. Each row is released as random 2–6 character bursts every 5 frames. The bounce comes entirely from the `out_bounce` easing of a single straight-line path.
### TTE effects, part 4: colorshift, rings, orbittingvolley, spray, errorcorrect, smoke

Source: terminaltexteffects (Python, HEAD) `terminaltexteffects/effects/effect_*.py`. MIT, (c) 2023 ChrisBuilds.

Shared engine facts these effects rely on:
- **Gradient(stops…, steps, loop)**: `steps` is an int, or a tuple with one value per adjacent stop pair (the last value repeats). The spectrum has `sum(steps)+1` colours, and shared stops appear once. `loop=true` adds a closing transition from the last stop back to the first, and the first stop becomes the final colour.
- **build_coordinate_color_mapping(bottom, top, left, right, direction)**: maps every coordinate in the text box to a spectrum colour by fraction. Horizontal uses column, vertical uses row, and radial uses the normalised distance from the centre with rows scaled ×2. Diagonal uses (row+col) normalised.
- **Scene.apply_gradient_to_symbols(symbols, duration, fg_gradient)**: builds `max(len(symbols), len(spectrum))` frames, each `duration` ticks long, and pairs the shorter sequence cyclically across the longer one. For example, 5 symbols over 16 colours gives 16 frames, with each symbol held for about 3 frames.
- **Scene sync=DISTANCE**: the frame index is `round((n-1) * path.distance_travelled / path.total_distance)`, so the colour follows travel progress instead of ticks.
- **TERMINAL_ROW_SCALE = 2**: rows count double in distances and circles, because terminal cells are about 2:1.
- **Paths**: `speed` is the distance per tick along the waypoint chain. `ease` remaps progress, and the total step count is `ceil(total_distance / speed)`. `layer` sets the draw order, and higher layers draw on top.
- **"final colour"**: each character's colour from the final gradient mapping. The default final direction is VERTICAL unless stated otherwise.
- Every effect has a `dynamic` existing-colour mode that uses the input's ANSI colours instead. It is ignored below.

---

### colorshift

This effect has no motion. Every character stays visible at its input coordinate the whole time and cycles through a looping gradient. With travel on, each character's gradient is phase-shifted by its position, so the colours look like a moving wave.

**Defaults**
- `gradient_stops` = rainbow `#e81416 #ffa500 #faeb36 #79c314 #487de7 #4b369d #70369d`, `gradient_steps=12`, `loop=!no_loop` (looping by default). That gives 7 transitions × 12 + 1 = 85 colours.
- `gradient_frames=2` (ticks per colour), `cycles=3` (0 means infinite), `travel_direction=RADIAL`, `reverse_travel_direction=false`, `no_travel=false`, `skip_final_gradient=false`.
- The final gradient uses the same rainbow, steps 12, VERTICAL.

**Build, per character**
- `idx`:
  - HORIZONTAL: `col / canvas.right`
  - VERTICAL: `row / canvas.top`
  - DIAGONAL: `(row+col)/(right+top)`
  - RADIAL: `normalized_distance_from_center(text box)`
- `shift = int(len(spectrum) * idx)`, and it is negated if the direction is reversed.
- `colors = spectrum[shift..] ++ spectrum[..shift]`, a rotation.
- Scene `gradient`: one frame per colour, each `gradient_frames` long, with the input symbol.
- Scene `final_gradient`: `Gradient(colors.last, final_colour, steps=8)` gives 9 frames, each `gradient_frames` long.
- On SCENE_COMPLETE(`gradient`), a callback increments the loop count. If `cycles==0 || count<cycles`, the effect re-activates `gradient`. Otherwise it activates `final_gradient` unless that is skipped.

All characters activate on frame 0. The effect ends when no character is active.

**Length**: about `cycles × 85 × 2` ticks, which is 510 ticks, plus 18 ticks for the final scene.

```
for ch in chars:
  idx = direction_index(ch); s = (spectrum.len() as f64 * idx) as isize * (rev?-1:1)
  colors = rotate(spectrum, s)
  ch.scene("gradient", colors.map(|c| Frame(sym, gradient_frames, fg=c)))
  ch.scene("final", Gradient(colors.last, final_map[ch.coord], 8).map(|c| Frame(sym, gf, c)))
  on_scene_complete("gradient", |ch| { ch.loops+=1; if cycles==0||ch.loops<cycles {activate("gradient")} else {activate("final")} })
  activate("gradient"); visible
loop { tick all; if none active break }
```

---

### rings

Characters are assigned to concentric elliptical rings around the canvas centre. The effect alternates between a *disperse* phase, where characters jitter randomly near their ring slot, and a *spin* phase, where the rings rotate with alternating directions and random speeds. Finally every character flies home.

**Defaults**
- `ring_colors=#ab48ff #e7b2b2 #fffebd`, with ring i taking colour `i % 3`.
- `ring_gap=0.1`, as a fraction of `min(canvas.top, canvas.right)`. `ring_gap_px = max(round(min(top,right)*0.1), 1)`.
- `spin_duration=200` ticks, `spin_speed=(0.25, 1.0)` (random per ring), `disperse_duration=200` ticks, `spin_disperse_cycles=3`.
- The final gradient is `#ab48ff #e7b2b2 #fffebd`, steps 12, VERTICAL.

**Ring construction**
- For `radius_scale in (1..max(right, top)).step_by(ring_gap_px)`:
  - `radius = radius_scale * 2`.
  - `coords = find_coords_on_circle(center, radius, limit = 7*radius_scale, unique)`.
- `find_coords_on_circle` samples angles `2πi/limit` with `x = cx + r·cos`, `y = cy + (r/2)·sin` (integer `r//2`), rounds the points, and drops duplicates.
- Stop adding rings once fewer than 25% of a ring's coords lie inside the canvas.
- Characters are shuffled, then dealt out ring by ring, one per ring coordinate, until they run out.
- Ring index parity sets the direction: odd rings are clockwise, which uses the reversed coord list.

**Characters on a ring**
- A character's slot index `k` is its order of addition to the ring.
- It gets one single-waypoint path per ring coord, in rotated order `coords[k..] ++ coords[..k]`. All paths use the ring's `rotation_speed` and are chained with `loop=true`, so the character hops coord to coord around the ring forever.
- Scene `gradient` (spin): `Gradient(final_colour, ring_colour, 8)`, 9 frames × 3 ticks.
- Scene `disperse`: `Gradient(ring_colour, final_colour, 8)`, 9 frames × 10 ticks, not looping.
- Characters left over (not on any ring) get an `external` path to `canvas.random_coord(outside_scope=true)`, with speed 0.8 and `out_sine`. When it completes, the character becomes invisible.
- Every character gets a `home` path to its input coord, with speed 0.8 and `out_quad`. Its start scene is its final colour.

**Disperse path**
- `make_disperse(origin)` picks 5 random coords from `find_coords_in_rect(origin, ring_gap_px)`, a square of side `2·gap+1`.
- The path has speed 0.14 and `loop=true`, so the character wanders slowly between those 5 points.

**Phases (frame counters)**
1. `start`: 100 ticks with the text shown statically in its final colours.
2. First `disperse` entry:
   - Each ring character builds a disperse path around its ring slot 0 coord.
   - An `initial` path (speed 0.3, `out_cubic`) takes it to the first disperse waypoint. When that path completes, the disperse path activates.
   - Scene `disperse` plays.
   - Non-ring characters activate `external`.
   - The phase then counts down `disperse_duration`.
3. `spin`:
   - Decrement `cycles`, and reset the spin timer to 200.
   - Per ring character, a `condense` path (speed 0.1) goes to the first waypoint of its remembered ring path. When it completes, that ring path activates and the loop chain resumes.
   - Scene `gradient` plays.
4. When the spin timer runs out:
   - If cycles remain, `ring.disperse()` runs. It remembers each character's current ring path and activates a new disperse path around the character's *current* coord with scene `disperse`, then goes back to phase 2's countdown.
   - If no cycles remain, the `final` phase starts: all characters become visible and activate `home`. Characters without an `external` path also play `disperse` (ring colour to final colour).
5. The effect is complete when no characters are active.

```
gap = max(round(min(top,right)*0.1),1)
for (ri, rs) in (1..max(right,top)).step_by(gap).enumerate():
  r = rs*2; coords = circle(center, r, 7*rs); if in_canvas_frac(coords)<0.25 {break}
  ring = Ring{coords, cw: ri%2==1, speed: rand(0.25,1.0), color: ring_colors[ri%3]}
deal shuffled chars into ring slots
per ring char: chain_loop(paths over rotated coords at ring.speed); scenes gradient(3t)/disperse(10t)
state machine: start(100) -> disperse(200) -> spin(200) -> [disperse -> spin]*(cycles-1) -> home
```

---

### orbittingvolley

This is the HEAD behaviour, and it differs from older TTE releases. Four launcher blocks (`█`) orbit the canvas edges, and each launcher fires characters from its magazine toward their home positions. Characters are loaded ring by ring from the centre outward, so the text fills in from the centre.

**Defaults**
- All launcher symbols are `█`.
- `launcher_movement_speed=0.8`, `character_movement_speed=1.5`, `character_easing=out_sine`.
- `volley_size=0.03` (fraction of all characters per volley, split across 4 launchers), `launch_delay=1` tick between volleys.
- The final gradient is `#FFA15C #44D492`, steps 12, **RADIAL**.
- There are two colour maps:
  - character colours are mapped over the text box;
  - launcher colours are mapped over the full canvas `(bottom, top, left, right)`.

**Launchers**
- Four launchers are added as extra characters on layer 2, at TL `(left, top)`, TR `(right, top)`, BR `(right, bottom)` and BL `(left, bottom)`.
- Only the **main launcher** (TL) has a real path, `perimeter`: waypoints `(left,top)` then `(right,top)`, speed 0.8. It travels along the top edge.
- Whenever its path finishes, it teleports back to waypoint 0 and restarts.
- The other three launchers are placed every frame from the main launcher's progress `p = main.col / right`:
  - TR: `(right, max(1, top - int(top·p)))`, moving down the right edge.
  - BR: `(max(1, right - int(right·p)), bottom)`, moving left along the bottom.
  - BL: `(left, min(top, bottom + int(top·p)))`, moving up the left edge.
- So all four sweep the perimeter in sync, like a rotating square. Each launcher's colour is `launcher_map[current_coord]`, re-set every frame.

**Magazines**
- `get_characters_grouped(CIRCLE_CENTER_TO_OUTSIDE)` buckets characters by ring, where `radius = ceil(hypot(col-cx, (row-cy)*2))` measured from the text-box centre. Buckets are sorted ascending.
- For each ring, each character goes to the magazine of its *nearest side*. The distances are:
  - top: `(top-row)*2`
  - right: `right-col`
  - bottom: `(row-bottom)*2`
  - left: `col-left`
- Ties break on the smaller magazine, then the lower side index. The magazine order is side 0 = top/TL launcher, 1 = right/TR, 2 = bottom/BR, 3 = left/BL.
- A pending ring loads into the magazines only when all magazines are empty **and** fewer than 2 rings are in flight. This limits the effect to at most 2 rings in the air at once.

**Launching**
- When `delay==0`, each launcher fires `max(int(0.03 · N_input / 4), 1)` characters.
- A fired character is teleported to the launcher's current coord, activates `input_path` (speed 1.5, `out_sine`, layer 1), and becomes visible.
- When its path completes, it switches to layer 0.
- The character's appearance is fixed at its final colour from the start, so there is no colour animation in flight.
- After firing, `delay` is reset to `launch_delay`.
- In-flight ring sets are intersected with the active set each frame and dropped when empty.

**End**: once the magazines, the pending rings and the in-flight rings are all empty, one more frame hides the launchers, then the effect stops.

```
main.perimeter = [(L,T),(R,T)] @0.8 ; others positioned from p=main.col/R each frame
rings = group_by(ceil(hypot(dx, dy*2))) ascending; each ring -> 4 magazines by nearest side
loop:
  if pending && all mags empty && inflight<2 { load next ring }
  if main idle { main.set(L,T); main.activate(perimeter) }
  place/color launchers
  if delay==0 { for l in launchers { for _ in 0..max(0.03*N/4,1) { fire(l) } } delay=1 } else {delay-=1}
  tick; prune inflight
```

---

### spray

All characters start stacked at one origin point and are released in random-sized bursts. Each one flies on a straight path with ease `out_expo` (fast launch, long slow settle) to its home position, while its colour fades from a random spectrum colour to its final colour.

**Defaults**
- `spray_position='e'`. The options are n, ne, e, se, s, sw, w, nw and center. The origin coords are:
  - E: `(right-1, center_row)`
  - NE: `(right-1, top)`
  - SE: `(right-1, bottom)`
  - N: `(center_col, top)`
  - S: `(center_col, bottom)`
  - W, NW, SW: use `left` on the matching row
  - center: `canvas.center`
- `spray_volume=0.005`, `movement_speed_range=(0.6, 1.4)` (uniform per character), `movement_easing=out_expo`.
- The final gradient is `#8A008A #00D1FF #FFFFFF`, steps 12, VERTICAL.

**Build, per character**
- `set_coordinate(origin)`.
- Path to `input_coord` at a random speed with `out_expo`. PATH_ACTIVATED sets layer 1, and PATH_COMPLETE sets layer 0.
- Scene: `Gradient(random_choice(final_spectrum), final_colour, steps=7)`, 8 frames × 20 ticks, so 160 ticks of colour fade. The scene is not synced to motion.
- The scene and path are activated at build time, but the character only starts ticking once it is released.
- The pending list is shuffled. `volume = max(int(N · 0.005), 1)`.

**Frame**: while characters are pending, release `randint(1, volume)` of them, popping from the end. Each released character becomes visible and active. Then tick. The effect ends when nothing is pending or active.

```
origin = pos_map[spray_position]
for ch in chars { ch.pos=origin; ch.path([ch.home], speed=rand(0.6,1.4), ease=out_expo, layer 1->0)
  ch.scene(Gradient(rand(final_spec), final[ch], 7) x20t) }
shuffle; vol=max(N*0.005,1)
loop { for _ in 0..rand(1..=vol) { release(pop) } tick; until empty }
```

---

### errorcorrect

The text starts fully visible in its final colours, except that random pairs of characters are swapped. Pairs are fixed one at a time: both characters flash an error state, wipe up into a red block, fly back to their correct positions while turning from red to green, wipe down, then fade to their final colour.

**Defaults**
- `error_pairs=0.1`, as a fraction of characters: `pair_count = min(N/2, int(0.1·N/2 + 0.5))`, with at least 1 if N ≥ 2.
- `swap_delay=6` ticks between pair activations.
- `error_color=#e74c3c`, `correct_color=#45bf55`, `movement_speed=0.9`.
- The final gradient is `#8A008A #00D1FF #FFFFFF`, steps 12, VERTICAL.

**Build**
- Every character gets a spawn scene with 1 frame in its final colour and is visible.
- For each pair, pick two random characters c1 and c2 without replacement.
  - Swap their positions: `c1.set_coord(c2.home)` and `c2.set_coord(c1.home)`.
  - Each gets path `input_coord` to its own home at speed 0.9, with no easing.
- Scenes for each swapped character:
  - `initial`: its symbol in `error_color`, 1 frame. It is active immediately, so swapped characters show red at their wrong positions from frame 0.
  - `error`: 10 × [`▓` in error colour for 3 ticks, then the symbol in `#ffffff` for 3 ticks], which is 60 ticks of flashing.
  - `first_block_wipe`: `▁▂▃▄▅▆▇█` in error colour, 3 ticks each (24 ticks).
  - `correcting`: `█` over `Gradient(error, correct, 10)`, 11 frames, with **sync=DISTANCE**, so the block turns green in step with travel.
  - `last_block_wipe`: `▇▆▅▄▃▂▁` in correct colour, 3 ticks each.
  - `final`: its symbol over `Gradient(correct, final_colour, 10)`, 11 frames × 3 ticks.
- Event chain:
  1. `error` done → activate `first_block_wipe`.
  2. `first_block_wipe` done → activate `correcting` and path `input_coord`. The path activation sets layer 1.
  3. Path done → layer 0 and activate `last_block_wipe`.
  4. `last_block_wipe` done → activate `final`.

**Frame**
- If pairs remain and `delay==0`, pop the next pair, activate `error` on both, and add them to the active set. Then `delay = 6`.
- Otherwise, if `delay > 0`, decrement it.
- Tick if any character is active.
- The effect ends when no pairs, no active characters and no delay remain. At least one frame is always rendered.

```
for ch: scene(final_color) visible
for _ in 0..pairs { (a,b)=pop2_random(); a.pos=b.home; b.pos=a.home;
  for c in [a,b] { c.path("home",[c.home],0.9); scenes as above; chain events } }
loop { if pairs && delay==0 {activate_error(pop_front); delay=6} else if delay>0 {delay-=1}
       tick_active; until nothing left }
```

---

### smoke

The whole text box starts visible in a flat grey. A random spanning tree over the grid, built with weighted Prim's, is flood-filled breadth-first from a random start cell. Each newly reached cell plays a "smoke" puff (`░▒▓▒░` over a dark to white to final-colours gradient), then a "paint" fade into its final colour. The result is an organic smoke front that spreads through the text.

**Defaults**
- `starting_color=#7A7A7A`, `smoke_symbols=('░','▒','▓','▒','░')`, `smoke_gradient_stops=(#242424, #FFFFFF)`.
- `use_whole_canvas=false`, so the effect is limited to the text boundary.
- The final gradient is `#8A008A #00D1FF #FFFFFF`, steps `(12,)`, VERTICAL.

**Build**
- Characters include inner and outer **fill characters** (the spaces in the box), so blank cells animate too.
- The final colour is `final_map.get(coord)` or `#000000`. The base appearance is the input symbol in `#7A7A7A`.
- `smoke_gradient = Gradient(#242424, #FFFFFF, *reversed(final_stops), steps=(3,4))`.
  - The stops are `242424, FFFFFF, FFFFFF, 00D1FF, 8A008A`, with effective steps `(3,4,4,4)`, giving 16 colours.
  - Scene `smoke` = `apply_gradient_to_symbols(smoke_symbols, 3, smoke_gradient)`: 16 frames × 3 ticks = 48 ticks, with each of the 5 symbols spread over about 3 frames.
- Scene `paint` = `apply_gradient_to_symbols([sym], 5, Gradient(#8A008A, #00D1FF, #FFFFFF, final_colour, steps=5))`: 16 frames × 5 ticks = 80 ticks.
- `smoke` done → activate `paint`.
- **Maze generation (PrimsWeighted)** runs to completion in `build`:
  - Each eligible cell gets a random weight 0..99.
  - Start from a random in-text cell and repeatedly take the pending link with the lowest weight, picking randomly within that weight.
  - The chosen link joins `char_a` to the unlinked neighbour `char_b`.
  - Then push `char_b`'s 4-neighbours (those still unlinked and eligible) into the pending buckets, keyed by the neighbour's weight.
  - The result is a random spanning tree stored as `ch.links`.
- **Fill (BreadthFirst)** starts from a different random in-text cell:
  - Each `step()` expands the whole frontier by one level along tree links.
  - `explored_last_step` holds the newly reached cells.
- The start cell activates `smoke` immediately.

**Frame**: if the fill is not complete, run `fill.step()` once and activate `smoke` on every cell in `explored_last_step`. Then tick. Because BFS runs along a random tree, the front moves one tree-edge per frame and forms winding tendrils rather than a circle. The effect ends when the fill is complete and nothing is active.

```
weights = rand 0..99 per cell; tree = prim_weighted(start=rand_cell)   // links (4-neigh)
bfs = BFS(tree, start=rand_cell2); activate_smoke(start)
loop { if !bfs.done { for c in bfs.step() { activate_smoke(c) } } tick; until bfs.done && none active }
activate_smoke(c): scene smoke(16f x3t: ░▒▓▒░ over 242424→FFF→FFF→00D1FF→8A008A) then paint(16f x5t → final)
```

Rust port hint: store `links: Vec<Vec<usize>>` over a grid index. For Prim's, use a `BTreeMap<u8, Vec<(a,b)>>` and pop a random element from the lowest bucket. BFS is a frontier `Vec` with a `visited` bitset.

---

## Part D — Other TTE effects not covered above

TTE also ships: binarypath, bubbles, middleout, overflow, pour, random_sequence, scattered, slice, sweep, thunderstorm (ttfx ports all 37). They use the same engine primitives; see `src-study/terminaltexteffects/terminaltexteffects/effects/effect_<name>.py`. `scratchpad/strip.py <file>` prints a source file without docstrings/help text for quick reading.
