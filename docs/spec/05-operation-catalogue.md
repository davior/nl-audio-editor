# 05 — Operation catalogue

Ordered by the product's targets. **Built** means implemented, with tests, in this
repository; the rest is the planned catalogue with a milestone.

## Shared engine for spectral operations

`noise_reduce`, `line_reduce`, `band_cut` and `spectral_compressor` share one STFT-mask
engine:

1. STFT with a periodic sqrt-Hann window at 75% overlap on a frame grid anchored to the start
   of the clip (never to the selection), so results never depend on where a preview starts.
2. Each operation computes a gain `g ≤ 1` per time–frequency cell (static or dynamic).
3. All smoothing has finite support; each operation declares its reach.
4. Reductions smaller than 0.001 dB snap to exactly zero.
5. **Subtractive render**: `residual = ISTFT((1 − g)·X)`, `output = input − residual`.

Consequences, all tested: samples outside the scope (plus one window) are bit-identical;
identity settings are bit-identical; the residual is exactly what was removed; a preview is
bit-identical to the same span of the final render. Zero-phase: no phase shift or timing
smear, which matters for evidential material.

## Gain and level

| Operation | Status | Notes |
|---|---|---|
| `gain` | **built** | `gain_db` −60…+60. Scoped gains fade in and out over 5 ms at the edges. |
| `dc_remove` | **built** | `mode` `mean` (subtract the mean per channel, as Audacity does) or `drift` (subtract a centred moving average, `window_ms` 50–5000). |
| `normalise` | **built** | `target_peak_dbfs` −60…0, default −1. Resolves to a fixed gain. |
| `compressor` | **built** | Broadband, for speech: `threshold_dbfs` −80…0 (−24), `ratio` 1–20 (3), `knee_db` 0–24 (6), `attack_ms` 0–500 (10), `release_ms` 1–5000 (150), `makeup_db` `auto` or −24…24 (`auto` restores the input peak). |
| `limiter` | **built** (system) | Always last. `ceiling_dbfs` −20…0 (−1), `lookahead_ms` 0.5–20 (5), `release_ms` 1–1000 (50). Inactive and bit-exact when nothing exceeds the ceiling. |
| Gating, level riding, loudness normalisation (LUFS target) | M2 | |

## Noise reduction and restoration

| Operation | Status | Notes |
|---|---|---|
| `noise_reduce` | **built** | Spectral gate against a per-bin noise profile. `profile` `auto` (quietest steady region) or `{t0,t1}`; `reduction_db` 0–48 (12); `sensitivity_db` 0–24 (6); `frequency_smoothing_bands` 0–12 (3); `attack_ms` 0–500 (20); `release_ms` 0–2000 (100). Parameter names follow Audacity's so they feel familiar; results are not bit-identical to Audacity's. |
| De-click, de-crackle | M2 | |
| De-clip | M2 | reconstruction class |
| De-reverb | M3 | reconstruction class |

## Filters and EQ

| Operation | Status | Notes |
|---|---|---|
| `line_reduce` | **built** (v2) | Static, zero-phase. `lines` `auto` or explicit `[{freq_hz, width_hz, depth_db}]`; `min_prominence_db` 1–40 (6); `min_persistence` 0–1 (0.5); `max_lines` 1–64 (16); `require_in_pauses` (on); `target_excess_db` 0–24 (0 = match the surroundings); `max_depth_db` 0–60 (40); `width_factor` 0.5–4 (1). Detection band from a `band` or `tf_patch` scope. Cuts each line by its measured prominence, re-measures, and cuts again until nothing stands out; records the largest remaining prominence. **Version 2:** a detected line must also stand out by ≥ 3 dB in the speech pauses (frames in which at most 10% of the 20 ms activity frames are active), because hum and whines do not stop when people stop talking while a voice harmonic at a recurring pitch does; with less than 1 s of pause in the span it must instead be present in ≥ 90% of segments. Version 1 (no pause rule) is kept so recipes recorded with it replay exactly. |
| `band_cut` | **built** | Manual cut: `f_lo`, `f_hi` (Hz), `depth_db` 0–60 (12), soft edges. Resolution chosen so the band spans several bins. |
| High-pass, low-pass, shelf, bell, tilt, voice-band EQ | M2 | zero-phase on the same engine |
| Hum removal (50/60 Hz + harmonics, automatic) | M2 | `line_reduce` already removes hum lines it finds |
| Dynamic EQ | M3 | |

## Spectral-domain surgical

### `spectral_compressor` — **built**

Pushes down high points inside a selected time × frequency area. Only ever reduces.

Scope: `tf_patch`, `time_range`, `band` or `clip`.

**Detection** — a cell is a high point if its level rises above a reference level by more
than `threshold_db`:

| `mode` | Reference level | Catches |
|---|---|---|
| `tonal` | median of neighbouring frequencies at the same moment (±`tonal_width_hz`) | whines, tones, feedback |
| `transient` | upper quartile of the same frequency over neighbouring moments (±`transient_width_ms`/2) — a brief burst fills well under a quarter of its neighbourhood, speech fills most of it | bangs, knocks, handling noise |
| `auto` (default) | the lower of the two | either |
| `level` | the band's background: the median cell per frame, then the median of that over ±`level_context_s` | a loud foreground voice, so quieter voices come up relative to it |

**Parameters** (hard limits enforced in Rust):

| Parameter | Range | Default |
|---|---|---|
| `mode` | `auto`, `tonal`, `transient`, `level` | `auto` |
| `threshold_db` | 0–48 | `auto`: 20 in `level` mode, 6 otherwise |
| `ratio` | 1–50 | 4 |
| `knee_db` | 0–24 | 6 |
| `max_reduction_db` | 0–60 | 20 |
| `attack_ms` | 0–500 | 10 (look-ahead: the reduction is in place when the peak arrives) |
| `release_ms` | 0–5000 | 150 |
| `resolution` | `auto`, `fine_time`, `balanced`, `fine_frequency` | `auto` (FFT 1024 / 2048 / 4096 at 48 kHz, scaled with the rate; `auto` picks by mode) |
| `tonal_width_hz` | 20–2000 | 150 |
| `transient_width_ms` | 20–2000 | 400 |
| `level_context_s` | 0.5–30 | 3 |
| `link_channels` | bool | true (one mask for all channels) |

**Measurements**: share of cells reduced, largest and mean reduction, energy removed relative
to the scope, band peak before and after.

**When to use it rather than `line_reduce`**: when the peak comes and goes, or should only be
reduced inside a selected area. For a line running through the whole clip, `line_reduce`.

### Planned

Two-dimensional time–frequency patch attenuation (M2), spectral repair / interpolation (M3,
reconstruction class).

## Separation and remix (M3+)

Voice isolation, speaker separation, stem blending, ducking. Reconstruction class: labelled
processed, not factual. `OPEN:` which local models, under what licences (brief question 4).

## Time edits — **built**

These operations change the output's length. The processing never sees them:
- every other step still works on the whole original timeline, so scopes never shift and
  residuals stay exact;
- the edits are applied to the processed result, just before the final limiter;
- every position is in seconds of the original recording, so their order does not matter
  (silences inserted at one point keep the order they were made in).

Class `edit`, category `editing`, residual `n/a`.

| Operation | Status | Notes |
|---|---|---|
| `remove_time` | **built** | Scope `time_range`: that stretch is left out of the output. `fade_ms` 0–50 (5): the audio fades out before the join and back in after it, so the join doesn't click; 0 joins the samples directly. |
| `insert_silence` | **built** | `at_s` (seconds of the original; 0 is the start, the recording's length is the end) and `duration_s` 0.001–3600: digital silence, nothing invented. `fade_ms` 0–50 (5) on the audio either side. |

- **What resolving records:** the exact samples (`s0`/`s1`, or `at_sample`/`samples`) and the
  fade in samples.
- **Overlaps and nesting:**
  - removals that overlap or touch merge, fading by the largest of their fades;
  - a silence inserted inside a removed stretch goes at its join.
- **Kept audio is untouched:** it is copied exactly, except within the fades.
- **Preview:** an edit is previewed on its own, with 3 s of audio either side.
  - *Before* is the original timeline, with the stretch marked.
  - *Processed* is the joined result.
  - *Residual* is the removed audio, where it was.
- **Exports:** a cue marker at each edit, labelled in the original's time, e.g. `removed
  12.000-15.500 s of the original (3.500 s)`.
- **Recipes:** edits are not saved in them, because they belong to one recording.

## Time and pitch (M3+)

Time stretch, pitch shift, formant shift, gap compression, alignment, crossfade. Residual
kind `n/a`.

## Stereo and spatial (M2)

Downmix, mid/side, width, balance, channel alignment, polarity correction.

## Creative and gated (later)

Saturation, reverb, delays, lo-fi; identity-altering operations separately gated.
`OPEN:` how much of this is exposed at all (brief question 11) — a policy question with
evidentiary consequences.

## Analysis (read-only)

Features v1 (computed for the clip and for each step's scope, recorded before and after every
step — see `10-learning-and-automation.md`) — **built**:

- levels: peak, true peak (4× oversampled), RMS, integrated loudness (BS.1770), crest factor,
  DC offset per channel, clipped samples (flat-topped runs);
- noise floor, and energy and floor per octave band;
- spectral centroid and tilt;
- **tonal lines**: frequency, width, prominence, persistence, level;
- **quietest steady region**: the shortest window, among 0.25–2 s, whose two halves agree
  within 1 dB (median over bands), in the quietest part of the clip; digital silence is
  skipped;
- hum fundamental (50 or 60 Hz) and how many harmonics are present;
- speech-activity ratio (energy-based, v1);
- estimated bandwidth (hints at earlier lossy encoding or band-limited channels).

Signals and authenticity (M2–M3), all reporting **indicators, not proof**, with the method
and its limits stated alongside every result:

- ENF trace: the mains-hum frequency tracked over time; discontinuities flagged;
- noise-floor, DC-offset and bandwidth discontinuity scans;
- clipping map; codec-history hints; metadata report.

## Selection, scope and utility

Scopes (`clip`, `time_range`, `band`, `tf_patch`) — **built**. Render and export (WAV float,
PCM 16/24, with cue markers at time edits) — **built** in the command-line tool and the
browser. Capability discovery (M1).
