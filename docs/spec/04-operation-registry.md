# 04 — Operation registry

The registry is the keystone. Validation, panels, tool schemas, logging, replay and the
training corpus are all derived from it. Adding an operation means adding one descriptor and
its implementation — nothing else.

## Descriptor

One file per operation and version: `schemas/ops/<id>.v<N>.json`, validated against
`schemas/op-descriptor.schema.json`.

| Field | Meaning |
|---|---|
| `id`, `version` | stable identity; a behaviour change is a new version |
| `title`, `summary` | plain English, shown in panels and sent to the model |
| `category` | `gain_level`, `filters_eq`, `noise_restoration`, `separation`, `time_pitch`, `stereo`, `spectral`, `creative`, `analysis`, `utility`, `safety`, `editing` |
| `class` | `analysis` (read-only), `corrective`, `attenuative` (can only reduce level), `reconstruction` (synthesises; labelled processed, not factual), `creative`, `safety`, `edit` (changes the timeline) |
| `tier` | `core`: the model always has the full tool schema; `on_demand` (the default): it sees a one-line index entry and asks for the schema (`06-reasoning-layer.md`) |
| `scopes` | which scope kinds it accepts |
| `params` | each with `id`, `type`, `title`, `help`, `default`, and type-specific limits |
| `identity` | parameter sets that must render bit-identical output (null tests) |
| `residual` | `exact`, `approximate` or `n/a` |
| `measurements` | what the DSP reports, with units |
| `binding_hints` | which analysis features the parameters naturally bind to |
| `system` | `true` for operations the application adds itself (the final limiter) |

### Parameter types

| Type | Limits | Notes |
|---|---|---|
| `number` | `min`, `max` (hard), `unit`, `step` | `auto_allowed` lets the value be `"auto"`, resolved on the clip |
| `integer` | `min`, `max` | |
| `enum` | `values` | |
| `bool` | | |
| `time_range` | | `"auto"` or `{t0, t1}` in seconds |
| `lines` | `max_items` | `"auto"` or a list of `{freq_hz, width_hz, depth_db}` |

## Validation

Every step is validated against its descriptor in Rust **before** any DSP runs:

- unknown parameters are rejected;
- missing parameters are filled with their defaults, so the recorded step always has every
  parameter written out;
- values outside the hard limits are **rejected** with a typed error naming the parameter and
  the limit — never silently clamped;
- `auto` is accepted only where the descriptor allows it.

Each operation's typed Rust parameter struct must accept the descriptor's defaults and
nothing else (tested), so the descriptor and the code cannot drift apart. A parity test fails
the build if a descriptor has no implementation or an implementation has no descriptor. When
panels are generated (M2), the parity test extends to panels and tool schemas.

## Tool schemas and panels (M1–M2)

- **Tool schema** for the model: generated from the descriptor — name, summary, JSON Schema of
  the parameters with limits and units. Tool exposure is tiered by the descriptor's `tier`:
  the `core` operations' schemas are always sent; each `on_demand` operation is one index line
  (id, title, summary) in the instructions, and its schema is sent when the model asks for it
  with `describe_operations`. Adding an `on_demand` operation costs the model one line.
- **Panel** for the user: generated from the same descriptor — a control per parameter with its
  limits, units, default and help text, and an "auto" toggle where allowed.

## Current operations

| Operation | Class | Tier | Summary |
|---|---|---|---|
| `gain` v1 | corrective | core | Raise or lower the level by a fixed amount |
| `dc_remove` v1 | corrective | core | Remove a constant (or slowly drifting) offset |
| `normalise` v1 | corrective | core | Bring the peak to a target level |
| `compressor` v1 | corrective | core | Broadband speech compressor |
| `limiter` v1 | safety (system) | — | Always last; bit-exact below the ceiling |
| `noise_reduce` v1 | attenuative | core | Spectral gate against a noise profile |
| `line_reduce` v1, v2 | attenuative | core | Find and flatten persistent tonal lines |
| `band_cut` v1 | attenuative | core | Manual narrow cut between two frequencies |
| `spectral_compressor` v1 | attenuative | core | Push down high points inside a time × frequency area |
| `remove_time` v1 | edit | core | Remove a stretch from the output, which becomes shorter |
| `insert_silence` v1 | edit | core | Insert digital silence into the output at a point |
| `high_pass` v1 | attenuative | on demand | Cut everything below a frequency |
| `low_pass` v1 | attenuative | on demand | Cut everything above a frequency |
| `bell` v1 | corrective | on demand | Raise or lower a band around a centre frequency |
| `shelf` v1 | corrective | on demand | Raise or lower everything below or above a corner |
| `tilt` v1 | corrective | on demand | Tilt the spectrum around a pivot: brighter or darker |
| `gate` v1 | attenuative | on demand | Lower the level in the pauses, by up to a set range |
| `loudness_normalise` v1 | corrective | on demand | Bring the integrated loudness (LUFS) to a target |
| `hum_reduce` v1 | attenuative | on demand | Cut the mains hum and its harmonics |

Full parameter tables are in `05-operation-catalogue.md` and the descriptor files.
