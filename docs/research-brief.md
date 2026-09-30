# Terminal Music Visualizer: Research Brief

Target: 44.1 kHz stereo s16 from a PulseAudio/PipeWire monitor, about 240x62 truecolor cells, 30-60 fps, Rust.
Notation: `fs` = 44100, `dt` = frame time in seconds. A one-pole smoother with time constant τ uses `a = 1 - exp(-dt/τ)` and `y += a·(x - y)`, which keeps it frame-rate independent.

---

## 1. Frequency analysis

**Latency vs resolution.** A Hann-windowed FFT of length N has a bin spacing of fs/N. Its effective delay is about N/2, because the energy centre of a symmetric window sits in the middle. The options at 44.1 kHz:

| N | bin spacing | window length | ~delay (N/2) |
|---|---|---|---|
| 1024 | 43 Hz | 23 ms | 12 ms |
| 2048 | 21.5 Hz | 46 ms | 23 ms |
| 4096 | 10.8 Hz | 93 ms | 46 ms |
| 8192 | 5.4 Hz | 186 ms | 93 ms |

- The hop size sets the update rate, not the resolution. Use a hop of 1/4 to 1/8 of N (75-87.5% overlap) so each rendered frame sees fresh data. A hop of 441 samples gives 100 analysis frames/s, and 735 samples matches 60 fps.
- A single FFT cannot resolve bass notes (E1 is 41 Hz, and 40-80 Hz is only about 2-4 bins at N=2048) and also keep the hi-hats sharp in time.
- cava's fix is several FFTs. Its buffer size scales with the sample rate (2048 at 44.1 kHz). The bass FFT is twice that length and covers bars below 100 Hz, while mids and treble use the base size. Every bar is forced to be at least `fs/N_bass` wide.
- The principled version of this is a constant-Q or variable-Q transform (Brown 1991; Schörkhuber et al. "CQT toolbox", variable-Q with a bandwidth offset γ ≈ ERB). For a 60 fps display, three FFTs in a multi-resolution stack get most of the benefit for very little CPU.

**Frequency scale for the bar layout.** Formulas:
- Mel: `2595·log10(1 + f/700)`
- Bark (Traunmüller): `26.81f/(1960+f) - 0.53`
- ERB-number (Glasberg & Moore 1990): `21.4·log10(1 + 0.00437f)`
- Log/octave: `log2(f/f0)`

Share of the 30 Hz-16 kHz axis that falls between 30 and 150 Hz (kick and bass):
- log: about 26%
- ERB: about 9%
- mel: about 4%

What that means for layout:
- **Mel and Bark squash the bass**, which is where most of the visual energy in modern music sits.
- **Pure log** matches musical pitch: an octave is a constant width. cava uses log between about 50 Hz and 10 kHz. The downside is that it spends a lot of bars on the region where FFT resolution is poorest, so neighbouring low bars end up showing the same bin.
- **ERB** follows the ear's own filter bandwidths and sits between the two.

**Magnitude to dB.**
- Sum *power* (|X|²) over the bins in each band, using triangular weights so energy is shared across band edges.
- Then compute `L = 10·log10(P + ε)`.
- Normalise the window: for a Hann window, a full-scale sine has |X| = N/4.
- s16 audio has about 96 dB of range, but bar displays look best with only the top **40-60 dB** shown.

**Weighting.**
- Do not apply A-weighting to the bars. It is -30 dB at 50 Hz and would wipe out the bass, and it is a noise-exposure curve, not a loudness model.
- ISO 226 equal-loudness contours depend on level, and nobody knows the playback SPL, so they are not usable here either.
- Use a simple **spectral tilt** instead.
  - Commercial pop records fall by about 5 dB/oct between 100 Hz and 4 kHz (Pestana et al., AES 2013).
  - Voxengo SPAN and FabFilter analysers default to a +4.5 dB/oct display slope around 1 kHz.
- The right tilt depends on how the bands are built:
  - With **per-bin density** (mean or max |X|² per Hz), pink noise falls 3 dB/oct, so add ≥ +3 dB/oct.
  - With **band-summed power in log-spaced bands**, pink noise already comes out flat, because the band width doubles each octave. Only about +1.5 to 2 dB/oct is left to make typical music look flat.
- cava's per-bar EQ is `eq ∝ f^0.85 / bandwidth`, which is the same idea in another form.

**Recommended for implementation:**
- Run three Hann-windowed real FFTs, all on the same hop (about 512 samples):
  - **Bass:** N=4096 for 20-250 Hz.
  - **Mid:** N=2048 for 250 Hz-2.5 kHz.
  - **Treble:** N=1024 above 2.5 kHz.
- 8192 for bass is optional as a "smooth" mode, but it adds about 90 ms of lag.
- Use a mono downmix, `(L+R)/2`. Keep L and R separately only if you want a stereo or mirrored layout.
- Bar edges: `f_k = f_lo·(f_hi/f_lo)^(k/B)` with f_lo = 40 Hz, f_hi = 16 kHz (or about 12 kHz), B = 64-120 bars. Offer an ERB-spaced option as well.
- Enforce a minimum bandwidth of one bin of the FFT that serves the band. Where bands would be narrower, interpolate between bin centres (quadratic or linear on the log-frequency axis) instead of letting several bars show the same bin.
- For each band: `L_k = 10·log10(Σ w_i|X_i|² / norm + 1e-12) + T·log2(f_c/1000)`, with tilt **T = +2 dB/oct** for band-summed power. Make T tunable from 0 to 4.5.
- Display: `h_k = clamp((L_k - (L_ref - R)) / R, 0, 1)` with range **R = 50 dB**, and `L_ref` set by the auto-gain in §2.
- Optionally apply `h^γ` with γ ≈ 1.3-1.5 to add contrast.

---

## 2. Temporal smoothing, gravity, auto-gain, gate

**Asymmetric attack/release.** Two time constants: `τ = τ_att` when x > y, else `τ_rel`.
- Attack should be fast, 5-15 ms, which is effectively instant at 60 fps.
- Release should be 150-400 ms.
- Smooth in the **dB domain**, so that decay reads as a steady number of dB per second, the way meters do. IEC 60268-10 PPM return rates are 20 dB in 1.5-1.7 s (Type I), or about 8.6 dB/s for the BBC Type IIb. A VU meter integrates over about 300 ms.

**cava as a reference.** cava works at a 66 fps baseline with `framerate_mod = 66/fps`.
- Gravity: `out = peak·(1 - fall²·gravity_mod)`, where `fall += 0.028` per frame and `gravity_mod = framerate_mod^2.5 · 2 / noise_reduction`. This is a quadratic, accelerating fall that starts from the last peak.
- Integral smoothing (a leaky integrator): `out = mem·noise_reduction/framerate_mod^0.1 + out`.
- Autosens:
  - On any overshoot above 1.0: `sens *= 1 - 0.02·framerate_mod`. That is τ ≈ 0.75 s, about -11 dB/s.
  - Otherwise: `sens *= 1 + 0.001·framerate_mod`, about +0.6 dB/s.
  - Start-up mode: `×1.1` per frame, about +40 dB/s, until the first overshoot.

**Peak caps.** Hold the cap for 0.3-0.6 s, then let it fall with constant acceleration: `v += g·dt`, `cap -= v·dt`. Use g ≈ 3-6 bar-heights/s².

**Auto-gain.** It should be slow and asymmetric, so that quiet passages do not get pumped up to look like loud ones.
- Track `L_ref` as a slow envelope of `max_k L_k`, or better, its 95th percentile across bands.
- Rising level: τ = 0.3-0.5 s, so clipping is avoided quickly.
- Falling level: τ = 5-10 s.
- Clamp the gain to a window of about ±30 dB around the calibrated default.
- MilkDrop does something similar. Its `bass`, `mid` and `treb` variables are *relative to a running average*: "1 is normal, below ~0.7 quiet, above ~1.3 loud", and `*_att` are time-damped versions. Keep the same ratio features for driving motion.

**Noise gate.**
- Compute RMS in dBFS.
- The gate opens at -55 dBFS and closes at -65 dBFS (hysteresis) after 300 ms below threshold.
- While the gate is closed: freeze auto-gain (never boost noise), let bars fall to zero, and after about 2-5 s cross-fade to an idle animation.

**Recommended for implementation:**
- Per bar, in dB: attack τ = 10 ms, release τ = 250 ms.
- Or use gravity: after a peak, `v += 1.5·R/s²·dt` and cap the fall at about 40 dB/s.
- Peak caps: hold 400 ms, g = 4 heights/s².
- Auto-gain `L_ref`: up τ 0.4 s, down τ 8 s. Add a fast start-up mode for the first 2 s.
- Gate as described above.
- Scale every time constant by `dt` so results do not depend on frame rate.
- Optional cava-style "monstercat" neighbour smoothing: `b[m] = max(b[m], b[z]/k^|z-m|)` with k ≈ 1.5-2.
- Optional "waves" smoothing: `b[m] = max(b[m], b[z] - c·(z-m)²)`.

---

## 3. Onset and beat detection

**Spectral flux, following the Bello et al. (2005) tutorial.**
- `SF(n) = Σ_k H(|X_n(k)| - |X_{n-1}(k)|)`, where H(x) = max(x, 0) is half-wave rectification.
- Log compression (`log(1+λ|X|)`) and a perceptual filterbank improve it a lot.
- Bello's adaptive threshold: `δ̃(n) = δ + λ·median(|d(n-M..n+M)|)`.

**LogFiltSpecFlux, online (Böck, Krebs & Schedl, ISMIR 2012).**
- 2048-sample frames at 100 fps.
- Semitone triangular filterbank. The filters are not normalised, which emphasises the highs.
- `log10(X_filt + 1)` compression.
- Online peak-picking: frame n is an onset if all three hold:
  1. `ODF(n) = max(ODF(n-w1..n))`
  2. `ODF(n) ≥ mean(ODF(n-w3..n)) + δ`
  3. `n - n_last > w5`
- Online optima: **w1 = 3, w3 = 4-12, w5 = 3 frames** (10 ms frames), with w2 = w4 = 0. Onsets tend to show up about one frame *early*.

**SuperFlux (Böck & Widmer, DAFx 2013).**
- Hann window N=2048, frame rate **200 fps** (hop about 220).
- 138 quarter-tone triangular filters from 27.5 Hz to 16 kHz.
- `X_log = log10(|X|·F + 1)`.
- A **maximum filter across ±1 band** runs on the *previous* frame, and the difference is taken over **μ = 2 frames**: `SF*(n) = Σ_m H(X_log(n,m) - maxfilt(X_log)(n-μ, m))`.
- This suppresses vibrato and tremolo and cuts false positives by up to 60%.
- Peak-picking: pre_max 30 ms, pre_avg 100 ms, combine 30 ms. Online, post_max = post_avg = 0.

**Band-wise onsets.** Run the same SuperFlux with the sum restricted to groups of bands:
- **kick:** 30-150 Hz
- **snare/body:** 150-500 Hz, plus 1.5-5 kHz for snap
- **hats/cymbals:** 6-16 kHz

Give each band its own threshold and refractory period (kick 120 ms, snare 100 ms, hats 50 ms). Making the ODF more percussive with a causal median filter (Stark's percussive beat tracking, 2013) helps with busy mixes.

**Tempo.**
- Scheirer (1998) used about 6 sub-band envelopes feeding a bank of comb-filter resonators: `y_τ(n) = α_τ·y_τ(n-τ) + (1-α_τ)·x(n)`, with α_τ set so the half-energy time is the same for every lag (≈1.5-2 s). The tempo is the lag with the most output energy, and the phase is read from the resonator's delay line.
- Ellis (2007):
  - Onset envelope: 40 mel bands with a 4 ms hop, converted to dB, first-order difference, half-wave rectified, summed, mean removed, smoothed with a Gaussian about 20 ms wide.
  - Tempo: take the autocorrelation `TPS(τ) = W(τ)·Σ O(t)O(t-τ)` weighted by the log-Gaussian `W(τ) = exp(-½(log2(τ/τ0)/σ)²)`, with **τ0 = 0.5 s (120 BPM) and σ = 1.4 octaves**. Folding in the duple level, `TPS2(τ) = TPS(τ) + 0.5·TPS(2τ) + 0.25·TPS(2τ±1)`, resolves octave errors.

**Real-time beat tracking (BTrack; Stark, Davies & Plumbley, DAFx 2009).**
- Hop 512, and an ODF buffer of about 6 s.
- Cumulative score: `C(n) = (1-α)·O(n) + α·max_{v∈[-2τ, -τ/2]} W(v)·C(n+v)`, with `W(v) = exp(-(η·ln(-v/τ))²/2)`, **α = 0.9, η (tightness) = 5**.
- To predict the next beat, run C one beat into the future with O = 0 and take the argmax, weighted by a Gaussian centred on the last beat + τ.
- The tempo is re-estimated from the ODF autocorrelation (with a comb-filter matrix) about every 1 s.

**Recommended for implementation:**
- A dedicated ODF path:
  - N=1024 or 2048 Hann, hop 441 (100 fps).
  - About 80 semitone or 24-per-octave log bands from 30 Hz to 16 kHz.
  - `log10(1 + 1·X)`.
  - SuperFlux max filter ±1 band, with μ = 1 at 100 fps (μ = 2 at 200 fps).
  - Half-wave rectified sum.
- Online peak pick: pre_max 30 ms, pre_avg 100-150 ms, refractory 30 ms (global) or per band as above. δ is a z-score threshold, so normalise the ODF by a running mean and standard deviation (τ = 3 s) and start at δ ≈ 1.0-1.5.
- Tempo:
  - Keep a 6-8 s ODF ring buffer.
  - Once a second, compute the autocorrelation (via FFT) over lags 0.33-1.0 s (60-180 BPM).
  - Apply Ellis's weighting (τ0 = 0.5 s, σ = 1.4) and the TPS2 fold.
  - Smooth the tempo with hysteresis: switch only if the new peak is more than 20% stronger for 2 consecutive estimates.
- Beat phase: use a PLL. Phase φ advances by `dt/T`.
  - On an onset within ±0.15T of the predicted beat, with error e (in periods): `φ -= 0.15·e` and `T *= 1 + 0.03·e`.
  - With no onset, keep free-running.
  - Confidence: an EMA of the fraction of predicted beats that had a matching onset. Animate on beats only when confidence > 0.5, and fall back to raw onsets otherwise.
- **Fire beat visuals from the prediction, 20-40 ms *early***. This cancels the analysis delay (see §5).
- Downbeat and phrase: count beats mod 4. Re-anchor bar 1 at the strongest kick onset after a section change.

---

## 4. Higher-level features

All of these are O(N) per frame or less, so they are cheap.

- **RMS / loudness.** `RMS = sqrt(mean(x²))` over a 50-100 ms window, then convert to dBFS. For a perceptual measure use ITU-R BS.1770 / EBU R128:
  - K-weighting: a high-shelf of +4 dB above about 1.5 kHz, plus a 2nd-order high-pass at about 38 Hz.
  - LUFS = -0.691 + 10·log10(mean-square).
  - **Momentary** loudness uses 400 ms, and **short-term** uses 3 s.
  - Use momentary loudness for "intensity" and short-term for the global energy level.
- **Spectral centroid ("brightness").** `C = Σ f_k P_k / Σ P_k`. Work on band powers and map it logarithmically: `c = log2(C/100)/log2(8000/100)`, clamped to 0..1. Smooth with τ ≈ 150 ms.
- **Spread.** `sqrt(Σ (f_k-C)² P_k / Σ P_k)`. Wide spread means full-band or noisy; narrow means sparse.
- **Flatness (Wiener entropy).** `exp(mean(ln P_k)) / mean(P_k)`, in 0..1, or in dB. Close to 1 means noise-like (risers, crashes, white-noise sweeps); close to 0 means tonal.
- **Rolloff.** The frequency below which 85% of the energy lies. **Band energies:** sub (20-60), bass (60-250), low-mid, mid, presence (2-6k), air (6k+), each also expressed as a ratio to its own 8-10 s average (the MilkDrop style).
- **Onset density.** Onsets per second over 2 s. A good proxy for "busy-ness" and for driving motion speed.
- **Build-up and drop detection (heuristic, cheap).** Signs of a *build*: over 4-16 s, the centroid rises, flatness rises (riser noise), snare or hat onset density climbs (snare rolls), and bass energy *falls* (the filter sweep removes it).
  - A *drop* is `E_bass(0.5 s) / E_bass(8 s) > 2` (+3 dB) together with `E_bass` below 0.5× its long average for the previous 2 s or more, followed by a strong kick onset.
  - Track this "tension" in 0..1 and ramp visuals with it.
- **Section changes (Foote 2000).**
  - Build a self-similarity matrix of feature vectors, for example 12-bin chroma plus 13 MFCC or 20 log-mel bands, averaged per beat or at 2-4 Hz, with cosine similarity.
  - Correlate along the diagonal with a Gaussian-tapered checkerboard kernel. Peaks in the result are boundaries.
  - A causal version only reports a boundary after half the kernel width has passed.
  - A cheap equivalent is the two-window distance `D = 1 - cos(mean(F[t-4s..t]), mean(F[t-12s..t-4s]))`, peak-picked with a 10 s refractory period and an adaptive threshold (mean + 1.5σ over 60 s).

**Recommended for implementation:**
- Compute RMS, momentary and short-term LUFS, log-centroid, flatness, 6 band energies and their ratios, onset density, tension, and novelty D.
- Update at the analysis rate. Novelty can run at 4 Hz.
- Hand every feature to the renderer twice, as `raw` and `att` (τ ≈ 0.1 s and 1 s).

---

## 5. Perception: mappings and latency

**Congruent mappings** that people match reliably across cultures:
- **Pitch → vertical position.** High pitch = up, and also = small and bright (Spence 2011 review; Walker 1987; Evans & Treisman 2010, which showed it is automatic and holds even when irrelevant to the task).
- **Loudness → size, brightness and line thickness.** Marks 1987; Lipscomb & Kim 2004. Küssner et al. (2014) had people draw in real time to music: pitch mapped to height, loudness to line thickness and size, and tempo to horizontal speed, and trained musicians were more consistent.
- **Timbral brightness (centroid) → lightness, saturation and sharpness.** Bright, noisy timbres go with light colours and pointy shapes; dark timbres with dark colours and round shapes (the bouba/kiki family).
- **Tempo and mode → colour.** Palmer et al. (PNAS 2013), US and Mexican participants: fast, major-key music picks **lighter, more saturated, yellower** colours; slow, minor music picks darker, desaturated, bluer ones. The link is mediated by emotion.
- **Tempo and onset density → speed of motion. Energy → amount of motion and number of particles.**

**Latency tolerance.** ITU-R BT.1359 gives detectability thresholds of **-45 ms (audio leads) to +125 ms (audio lags)**, and acceptability thresholds of **-90 to +185 ms**. The asymmetry exists because in nature light arrives before sound (Vroomen & Keetels 2010 review the temporal integration window).
- **A visualizer is always the "audio leads" case**: visuals come after the audio. That puts us on the *tight* side of the window: about 45 ms before people notice, and about 90 ms before it becomes unacceptable.
- The budget is spent on: FFT centre delay, the render frame (8-16 ms at 60 fps), the terminal's own present time (1-2 frames), and audio capture buffering.
- On the other side, the PulseAudio/PipeWire monitor delivers samples *before* they reach the DAC. The sink latency (typically 20-60 ms, and 150-250 ms over Bluetooth) is headroom in our favour. With Bluetooth the visuals can even run *early*.

**Recommended for implementation:**
- Default mappings:
  - frequency → x position (or angle)
  - band level → height
  - loudness → overall brightness and scale
  - centroid → hue/lightness shift (dark to light), in OKLCH
  - flatness → saturation down plus grain/noise
  - onset density and tempo → speed of motion
  - kick → radial pulse and scale
  - hats → small sparkles
- Keep the analysis latency (capture + FFT centre + frame) **≤ 40 ms**.
- Use capture fragments of 256-512 frames (`fragsize` in pa_buffer_attr / PipeWire `node.latency=512/44100`).
- Query the sink latency. Expose a user A/V offset from -200 to +200 ms and apply it as a small ring-buffer delay on the visuals. Beat *prediction* lets you remove the processing delay entirely for beat-locked events.

---

## 6. Design lessons from great visualizers

- **MilkDrop / projectM.** Most of the look comes from the *feedback loop*: each frame warps, zooms and rotates the previous one and multiplies it by `decay` (0.98 recommended, where 0.9 is a strong fade). This gives trails and motion continuity for free, so audio only needs to *nudge* the parameters (zoom, rot, warp, colour).
  - Audio variables are relative to their running average, with smoothed `*_att` versions.
  - Presets auto-switch after tens of seconds with a soft **blend** of a few seconds, plus optional beat-driven **hard cuts** on loud beats.
  - Randomising the preset order gives variety over hours.
- **Winamp AVS.** Layered effects (render, trans/movement, blur, "fadeout") combined with blend modes, plus beat detection that toggles effects on and off. The lesson is to layer a slow background, a mid-speed structure, and fast beat accents.
- **iTunes Magnetosphere (Robert Hodgin).** Audio drives *forces* on particles (frequency-driven charges), not particle *positions*. The physics integrates away jitter, so motion always stays continuous. This is the single best trick for avoiding twitchiness.
- **Magic Music Visuals.** Modular graphs that map features to parameters, with explicit smoothing and envelope modules. This is the "mapping matrix" approach: make mappings data-driven and randomisable.
- **cava / Monstercat.** Bars fall off smoothly into their neighbours (monstercat filter), which reads as a continuous, organic silhouette instead of spiky noise. The palette is restrained and the gradient vertical.
- **Strobing and photosensitivity.** WCAG 2.3.1 / ITU-R BT.1702 say **no more than 3 flashes per second** over large areas, and saturated red flashes are the worst. A full-screen flash on every hi-hat violates this, and so can 1/8 notes at 128 BPM (4.3 Hz).

**Recommended for implementation:**
- Build every scene around persistent state: a feedback buffer with decay 0.9-0.97 per frame at 60 fps, adjusted by `decay^(dt·60)`, or a particle or force field. Audio modulates the rates and forces of that state rather than its absolute values.
- Use three time scales:
  - **beat:** pulses with a 60-250 ms exponential decay
  - **bar/phrase:** palette rotation, camera drift
  - **section:** scene or preset change
- Scene policy:
  - On a novelty peak, or after 45-120 s at the latest, schedule a switch **on the next downbeat** (or on a phrase boundary, 4 or 8 bars, when the beat is confident).
  - Cross-fade over 1-2 bars (about 2-4 s).
  - **Hard cut exactly on a detected drop.**
  - Never switch while tension is rising.
- Keep slow generative drift (0.01-0.1 Hz noise on hue and camera) so quiet passages still move.
- Limit large-area luminance jumps above 10% to **3 or fewer per second**. Keep hat and 16th-note accents local, covering less than about 10% of the screen. Avoid flashing saturated red.
- Pick each scene's parameters at random within designer-set ranges, to get variety without ugliness.

---

## 7. Character-cell rendering

- **Cells are about 1:2 (width:height).**
- **Eighth blocks** `▁▂▃▄▅▆▇█` (U+2581-2588) give 8 sub-levels per cell vertically, so 62 rows × 8 = 496 levels. Horizontal eighths `▏▎▍▌▋▊▉` (U+258F-2589) do the same sideways. The partial glyph at the top of a bar acts as its anti-aliasing. To colour a partial cell, set its foreground to the bar colour and its background to whatever is behind.
- **Half blocks** `▀`/`▄` with foreground + background truecolor give **240×124 square pixels, each with its own colour**. This is the best mode for images, feedback effects and fluid effects.
- **Quadrants** (▖▗▘▝ etc., 2×2) and **sextants** (U+1FB00, 2×3, Legacy Computing) add resolution, but only 2 colours per cell. **Octants** (U+1CD00, 2×4, Unicode 16) have weak font support.
- **Braille** U+2800-28FF gives 2×4 dots per cell, so 480×248, but only one foreground colour per cell. It is ideal for thin lines, oscilloscope and Lissajous (L vs R) traces, and particles. Fonts render dots of uneven size and spacing, and empty dot positions look gappy.
- **Dithering.** Use ordered dithering (a 4×4 or 8×8 Bayer matrix) to pick the braille dot or sub-glyph for an intensity. Bayer on colour before quantising removes banding. Avoid temporal dithering, which shows up as flicker.
- **Colour.** Build gradients in OKLab/OKLCH (Ottosson 2020) so steps look even. Blend and decay in linear light and convert to sRGB at the end.
- **Throughput.** A full truecolor redraw costs about 20 bytes per SGR per colour. With 14,880 cells that is up to about 0.6 MB/frame, about 36 MB/s at 60 fps, which is too much. To cut it:
  - Diff against the previous frame.
  - Emit SGR only when the colour changes, and use cursor-move for skipped runs.
  - Quantise colours slightly (for example to 6-7 bits per channel) to lengthen runs.
  - Write one buffered `write()` per frame.
  - Wrap each frame in **synchronized output** (DEC mode 2026: `CSI ?2026h` … `CSI ?2026l`) to prevent tearing.
  - Use the alternate screen and hide the cursor.

**Recommended for implementation:**
- Keep one frame buffer as a 240×124 RGB half-block canvas, with an optional braille overlay layer at 480×248 for lines and particles.
- Bars use eighth blocks, with a colour per cell row taken from an OKLCH gradient.
- Bayer 8×8 dithering for braille intensity.
- Diffed output with run-length SGR, colours quantised to 7 bits per channel, and mode 2026.
- Cap at 60 fps and fall back to 30 fps if the write time goes over 8 ms.

---

## References

- cava source (cavacore.c: dual FFT, log bar distribution, EQ, gravity, integral, autosens, monstercat): https://github.com/karlstav/cava , https://github.com/karlstav/cava/blob/master/cavacore.c
- Bello et al. 2005, "A Tutorial on Onset Detection in Music Signals", IEEE TSAP: https://ieeexplore.ieee.org/document/1495485
- Dixon 2006, "Onset Detection Revisited", DAFx: https://www.dafx.de/paper-archive/2006/papers/p_133.pdf
- Böck, Krebs, Schedl 2012, "Evaluating the Online Capabilities of Onset Detection Methods", ISMIR: https://www.cp.jku.at/research/papers/Boeck_etal_ISMIR_2012.pdf
- Böck & Widmer 2013, "Maximum Filter Vibrato Suppression for Onset Detection" (SuperFlux), DAFx: https://www.dafx.de/paper-archive/2013/papers/09.dafx2013_submission_12.pdf ; code: https://github.com/CPJKU/SuperFlux
- Scheirer 1998, "Tempo and beat analysis of acoustic musical signals", JASA 103(1): https://doi.org/10.1121/1.421129
- Ellis 2007, "Beat Tracking by Dynamic Programming", JNMR: https://www.ee.columbia.edu/~dpwe/pubs/Ellis07-beattrack.pdf
- Stark, Davies, Plumbley 2009, "Real-time beat-synchronous analysis of musical audio", DAFx; BTrack: https://github.com/adamstark/BTrack
- Stark 2011, "Real-time visual beat tracking using a comb filter matrix", ICMC: https://www.adamstark.co.uk/pdf/papers/comb-filter-matrix-ICMC-2011.pdf
- Foote 2000, "Automatic audio segmentation using a measure of audio novelty", ICME: https://doi.org/10.1109/ICME.2000.869637
- Müller, FMP notebooks (onset, novelty, peak picking): https://www.audiolabs-erlangen.de/resources/MIR/FMP/C6/C6S1_OnsetDetection.html
- Glasberg & Moore 1990, "Derivation of auditory filter shapes from notched-noise data" (ERB): https://doi.org/10.1016/0378-5955(90)90170-T
- Schörkhuber, Klapuri, Holighaus, Dörfler 2014, "A Matlab toolbox for efficient perfect reconstruction time-frequency transforms with log-frequency resolution" (variable-Q), AES
- Pestana et al. 2013, "Spectral characteristics of popular commercial recordings 1950-2010", AES 135: https://aes.org/publications/elibrary-page/?id=17010
- ITU-R BS.1770 (loudness, K-weighting): https://www.itu.int/rec/R-REC-BS.1770 ; EBU R128: https://tech.ebu.ch/publications/r128
- ITU-R BT.1359 (A/V sync thresholds): https://www.itu.int/dms_pubrec/itu-r/rec/bt/R-REC-BT.1359-0-199802-S!!PDF-E.pdf
- Vroomen & Keetels 2010, "Perception of intersensory synchrony: A tutorial review", APP 72: https://link.springer.com/article/10.3758/APP.72.4.871
- Spence 2011, "Crossmodal correspondences: A tutorial review", APP 73: https://link.springer.com/article/10.3758/s13414-010-0073-7
- Evans & Treisman 2010, "Natural cross-modal mappings between visual and auditory features", J. Vision 10(1): https://doi.org/10.1167/10.1.6
- Walker 1987, "The effects of culture, environment, age, and musical training on choices of visual metaphors for sound", Perception & Psychophysics 42
- Küssner, Tidhar, Prior, Leech 2014, "Musicians are more consistent: gestural cross-modal mappings of pitch, loudness and tempo in real-time", Frontiers in Psychology: https://www.frontiersin.org/journals/psychology/articles/10.3389/fpsyg.2014.00789/full
- Palmer, Schloss, Xu, Prado-León 2013, "Music-color associations are mediated by emotion", PNAS: https://www.pnas.org/doi/10.1073/pnas.1212562110
- MilkDrop preset authoring guide (bass/mid/treb semantics, decay): https://www.geisswerks.com/milkdrop/milkdrop_preset_authoring.html ; MilkDrop docs: https://www.geisswerks.com/milkdrop/milkdrop.html ; projectM: https://github.com/projectM-visualizer/projectm
- WCAG 2.x SC 2.3.1 Three Flashes or Below Threshold: https://www.w3.org/WAI/WCAG21/Understanding/three-flashes-or-below-threshold.html
- Ottosson 2020, "A perceptual color space for image processing" (OKLab): https://bottosson.github.io/posts/oklab/
- Synchronized output (DEC mode 2026) spec: https://gist.github.com/christianparpart/d8a62cc1ab659194337d73e399004036
