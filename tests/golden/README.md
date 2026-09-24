# Golden set

The golden clips are synthesised deterministically by `nlae golden` (see
`core/src/golden.rs`) from separate, known components, so every test can measure
exactly what an operation did to each component: the voices, the noise, each
tonal line, the whine and the bangs.

- **Clip A** — 60 s, 48 kHz, very quiet (peaks ≈ −30 dBFS), DC offset, broadband
  noise and rumble, steady lines at 750 Hz and 3,150 Hz, weak 50 Hz hum, a
  foreground speech-like voice, a background voice at −24 dB, an intermittent
  whine, bangs and a clipped section. Shaped like the material in
  `docs/spec/reference-workflows/audacity-spoken-word-cleanup.md`.
- **Clip B** — the same kinds of problem with different voices and levels, lines
  at 620 Hz and 2,450 Hz and 60 Hz hum. Used to prove adaptive replay.

`expected.json` holds the SHA-256 of every generated file; the audio itself is
not committed. Regenerate with `cargo run -p nlae-cli -- golden tests/golden/out`.

Real clips can be placed in `tests/golden/real/` (git-ignored) for local checks.
