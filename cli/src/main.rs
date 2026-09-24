//! `nlae` — the command-line front end. It drives the same core as the
//! interface: every command that changes a project goes through the stack
//! engine and is recorded in the project's event log.

mod env;
mod png;
mod show;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use nlae_core::audio::wav::{write_wav, WavFormat};
use nlae_core::audio::{decode, AudioBuffer};
use nlae_core::dataset::{self, AudioInclusion, ExportOptions, ProjectData};
use nlae_core::project::store::{DirStore, MemStore, Store};
use nlae_core::project::{
    bundle, open_bundle, AcceptOptions, CreateOptions, Origin, PreviewOptions, Project, Rating,
    StepDraft,
};
use nlae_core::provenance::{Actor, AppInfo};
use nlae_core::recipe::{plan_replay, Recipe, ReplayMode};
use nlae_core::scope::Scope;
use serde_json::{json, Value};

use env::SystemEnv;

type Res<T> = Result<T, String>;

#[derive(Parser)]
#[command(
    name = "nlae",
    version,
    about = "Natural-language audio editor: the action stack from the command line"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a project from a recording (stored byte-for-byte).
    New {
        audio: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
    /// Show a recording's or project's details and analysis.
    Inspect {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Preview one operation on a short window. Nothing is committed.
    Preview(PreviewArgs),
    /// Preview a recipe (built-in or file) as one plan, after a dry-run diff.
    Plan(PlanArgs),
    /// Replay a recipe on a recording or project (alias of `plan`, creating the project if needed).
    Replay {
        recipe: String,
        target: PathBuf,
        /// Project directory to create when the target is a recording.
        #[arg(long)]
        into: Option<PathBuf>,
        #[command(flatten)]
        common: PlanCommon,
    },
    /// Accept a preview: commit it to the stack.
    Accept {
        project: PathBuf,
        preview: String,
        #[arg(long)]
        note: Option<String>,
        /// Change a step's parameters before accepting: INDEX:JSON (index within the preview, from 1).
        #[arg(long = "set")]
        set: Vec<String>,
        /// Switch a plan step off before accepting (index from 1).
        #[arg(long = "skip")]
        skip: Vec<usize>,
    },
    /// Reject a preview.
    Reject {
        project: PathBuf,
        preview: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Remove the top step (history is kept).
    Undo { project: PathBuf },
    /// Show the stack.
    Stack {
        project: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Show the event log.
    Log {
        project: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Rate a step, a plan or the whole stack (1–5).
    Rate {
        project: PathBuf,
        #[arg(long)]
        overall: u8,
        /// Per-dimension scores, e.g. intelligibility=4.
        #[arg(long = "dim")]
        dims: Vec<String>,
        /// step:ID, plan:ID or stack (default).
        #[arg(long, default_value = "stack")]
        target: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Add a note or labels to a step.
    Annotate {
        project: PathBuf,
        step: String,
        #[arg(long)]
        note: Option<String>,
        #[arg(long = "label")]
        labels: Vec<String>,
    },
    /// Clone a project at a step (or its current state) to follow another stream of edits.
    Clone {
        project: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    /// Render the stack (with the final limiter) to a WAV file.
    Render {
        project: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value = "f32")]
        format: String,
    },
    /// Save part of the stack as a recipe.
    SaveRecipe {
        project: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long)]
        name: String,
        /// First step (from 1).
        #[arg(long)]
        from: Option<usize>,
        /// Last step (from 1).
        #[arg(long)]
        to: Option<usize>,
    },
    /// Export projects' stacks as a training dataset.
    Dataset {
        #[arg(required = true)]
        projects: Vec<PathBuf>,
        #[arg(short, long)]
        out: PathBuf,
        /// Include the source recordings (off by default).
        #[arg(long)]
        with_audio: bool,
    },
    /// Pack a project directory into a portable .nlae bundle.
    Pack {
        project: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Unpack a .nlae bundle into a project directory.
    Unpack {
        bundle: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Verify a project directory or bundle: source hash, hash chain, lineage, stack.
    Verify { path: PathBuf },
    /// Draw a spectrogram PNG of a recording or a project (source, stack or what the stack removed).
    Spectrogram {
        path: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        /// source | stack | residual (projects only).
        #[arg(long, default_value = "stack")]
        what: String,
        #[arg(long)]
        from: Option<f64>,
        #[arg(long)]
        to: Option<f64>,
        #[arg(long, default_value_t = 8000.0)]
        fmax: f64,
        #[arg(long)]
        log: bool,
        #[arg(long, default_value_t = 1200)]
        width: u32,
        #[arg(long, default_value_t = 400)]
        height: u32,
        /// Colour range floor (dB relative to the loudest cell drawn).
        #[arg(long, default_value_t = 90.0)]
        range: f64,
    },
    /// Write the golden clips (mixtures and components) and their hashes.
    Golden {
        out: PathBuf,
        /// Only the 12-second variant of clip A (mixture only), for quick tests.
        #[arg(long)]
        short: bool,
    },
}

#[derive(Args)]
struct PreviewArgs {
    project: PathBuf,
    #[arg(long)]
    op: String,
    /// Parameters as JSON, e.g. '{"mode":"tonal"}'.
    #[arg(long, default_value = "{}")]
    params: String,
    /// clip | time:T0,T1 | band:FLO,FHI | tf:T0,T1,FLO,FHI
    #[arg(long, default_value = "clip")]
    scope: String,
    /// Preview window T0:T1 in seconds (default: 10 s).
    #[arg(long)]
    window: Option<String>,
    /// Your words: what you want (recorded for training).
    #[arg(long)]
    intent: Option<String>,
    #[arg(long)]
    note: Option<String>,
    /// Write the processed window here.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Write what the step removes here.
    #[arg(long)]
    residual: Option<PathBuf>,
}

#[derive(Args, Clone)]
struct PlanCommon {
    #[arg(long, default_value = "adaptive")]
    mode: String,
    /// Include optional recipe steps (e.g. the compressor).
    #[arg(long)]
    with_optional: bool,
    /// Show the diff only; do not preview.
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    window: Option<String>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    residual: Option<PathBuf>,
}

#[derive(Args)]
struct PlanArgs {
    project: PathBuf,
    /// builtin:spoken-word-cleanup or a recipe file.
    #[arg(long)]
    recipe: String,
    #[command(flatten)]
    common: PlanCommon,
}

fn app() -> AppInfo {
    AppInfo::new("cli")
}

fn open_dir(dir: &Path) -> Res<Project<DirStore>> {
    if !dir.join("manifest.json").exists() {
        return Err(format!("{} is not a project directory", dir.display()));
    }
    let (mut p, report) = Project::open(DirStore::new(dir), app()).map_err(|e| e.to_string())?;
    p.cache_renders = true;
    if !report.ok() {
        eprintln!("warning: this project does not verify:");
        for pr in &report.problems {
            eprintln!("  {pr}");
        }
    }
    Ok(p)
}

fn parse_scope(s: &str) -> Res<Scope> {
    let nums = |rest: &str, n: usize| -> Res<Vec<f64>> {
        let v: Vec<f64> = rest
            .split(',')
            .map(|x| {
                x.trim()
                    .parse::<f64>()
                    .map_err(|e| format!("scope `{s}`: {e}"))
            })
            .collect::<Res<_>>()?;
        if v.len() != n {
            return Err(format!("scope `{s}`: expected {n} numbers"));
        }
        Ok(v)
    };
    match s.split_once(':') {
        None if s == "clip" => Ok(Scope::Clip),
        Some(("time", r)) => nums(r, 2).map(|v| Scope::TimeRange { t0: v[0], t1: v[1] }),
        Some(("band", r)) => nums(r, 2).map(|v| Scope::Band {
            f_lo: v[0],
            f_hi: v[1],
        }),
        Some(("tf", r)) => nums(r, 4).map(|v| Scope::TfPatch {
            t0: v[0],
            t1: v[1],
            f_lo: v[2],
            f_hi: v[3],
        }),
        _ => Err(format!(
            "unknown scope `{s}` (clip | time:T0,T1 | band:FLO,FHI | tf:T0,T1,FLO,FHI)"
        )),
    }
}

fn parse_window(w: &Option<String>) -> Res<Option<(f64, f64)>> {
    match w {
        None => Ok(None),
        Some(s) => {
            let (a, b) = s
                .split_once(':')
                .ok_or_else(|| format!("window `{s}`: use T0:T1"))?;
            let p = |x: &str| {
                x.trim()
                    .parse::<f64>()
                    .map_err(|e| format!("window `{s}`: {e}"))
            };
            Ok(Some((p(a)?, p(b)?)))
        }
    }
}

fn write_audio(path: &Path, audio: &AudioBuffer, format: WavFormat) -> Res<()> {
    std::fs::write(path, write_wav(audio, format)).map_err(|e| format!("{}: {e}", path.display()))
}

fn load_recipe(spec: &str) -> Res<Recipe> {
    if let Some(name) = spec.strip_prefix("builtin:") {
        return Recipe::builtin(name).ok_or_else(|| {
            format!("no built-in recipe `{name}` (try builtin:spoken-word-cleanup)")
        });
    }
    let text = std::fs::read_to_string(spec).map_err(|e| format!("{spec}: {e}"))?;
    Recipe::parse(&text).map_err(|e| e.to_string())
}

fn print_preview<S: Store>(
    p: &Project<S>,
    pv: &nlae_core::project::Preview,
    out: &Option<PathBuf>,
    residual: &Option<PathBuf>,
) -> Res<()> {
    let r = &pv.record;
    println!(
        "Preview {} ({}) — window {:.2}–{:.2} s, on top of {} accepted step(s)",
        r.preview_id,
        if r.kind == "plan" {
            format!("plan of {} steps", r.steps.len())
        } else {
            "one step".into()
        },
        r.window[0],
        r.window[1],
        p.state().steps.len()
    );
    for (i, s) in r.steps.iter().enumerate() {
        println!("{}", show::step_line(i, s));
        let m = show::measurements(s);
        if !m.is_empty() {
            println!("      window: {m}");
        }
    }
    if let Some(o) = out {
        write_audio(o, &pv.output, WavFormat::F32)?;
        println!("Processed window → {}", o.display());
    }
    if let Some(o) = residual {
        write_audio(o, &pv.residual, WavFormat::F32)?;
        println!("What it removes  → {}", o.display());
    }
    println!(
        "Accept: nlae accept <project> {}   Reject: nlae reject <project> {}",
        r.preview_id, r.preview_id
    );
    Ok(())
}

fn run_plan<S: Store>(p: &mut Project<S>, recipe: &Recipe, common: &PlanCommon) -> Res<()> {
    let mode = match common.mode.as_str() {
        "adaptive" => ReplayMode::Adaptive,
        "exact" => ReplayMode::Exact,
        m => return Err(format!("unknown mode `{m}` (adaptive | exact)")),
    };
    let mut e = SystemEnv;
    let plan = plan_replay(p, recipe, mode, common.with_optional, Actor::user())
        .map_err(|e| e.to_string())?;
    println!(
        "Recipe {} ({:?}), {} step(s) on top of {} accepted:",
        recipe.name,
        mode,
        plan.drafts.len(),
        p.state().steps.len()
    );
    for (k, (d, i)) in plan.drafts.iter().zip(&plan.step_indices).enumerate() {
        let title = recipe.steps[*i].title.clone().unwrap_or_default();
        println!(
            "  {}. {:<20} {}  [{}]",
            k + 1,
            d.op,
            title,
            show::scope(&d.scope)
        );
    }
    if !plan.skipped.is_empty() {
        println!(
            "  skipped (off or optional): {:?}",
            plan.skipped.iter().map(|i| i + 1).collect::<Vec<_>>()
        );
    }
    println!("Dry-run diff:");
    show::diff(&plan.diff);
    p.record(
        &mut e,
        "recipe.replayed",
        None,
        json!({ "recipe": plan.recipe, "hash": plan.recipe_hash, "mode": plan.mode, "dry_run": common.dry_run, "diff": plan.diff }),
    )
    .map_err(|e| e.to_string())?;
    if common.dry_run {
        return Ok(());
    }
    let opts = PreviewOptions {
        window: parse_window(&common.window)?,
        plan: true,
        recipe: Some(json!({ "name": plan.recipe, "hash": plan.recipe_hash, "mode": plan.mode })),
    };
    let pv = p
        .preview(&mut e, plan.drafts.clone(), opts)
        .map_err(|e| e.to_string())?;
    print_preview(p, &pv, &common.out, &common.residual)
}

fn data_of(p: &mut Project<DirStore>, with_audio: bool) -> Res<ProjectData> {
    let src = p.source.clone();
    Ok(ProjectData {
        manifest: p.manifest.clone(),
        events: p.log.events().to_vec(),
        source_bytes: if with_audio {
            Some(p.source_bytes().map_err(|e| e.to_string())?)
        } else {
            None
        },
        source_features: Some(p.features_for(&src)),
    })
}

fn run(cli: Cli) -> Res<()> {
    let mut e = SystemEnv;
    match cli.cmd {
        Cmd::New { audio, out, name } => {
            let bytes = std::fs::read(&audio).map_err(|e| format!("{}: {e}", audio.display()))?;
            let filename = audio
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("unusable file name")?
                .to_string();
            if out.exists()
                && std::fs::read_dir(&out)
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false)
            {
                return Err(format!("{} already exists and is not empty", out.display()));
            }
            let last_modified = std::fs::metadata(&audio)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| nlae_core::provenance::env::format_rfc3339_ms(d.as_millis() as u64));
            let opts = CreateOptions {
                name,
                last_modified,
                ..Default::default()
            };
            let mut p =
                Project::create(DirStore::new(&out), &mut e, app(), &bytes, &filename, opts)
                    .map_err(|e| e.to_string())?;
            p.analyse(&mut e).map_err(|e| e.to_string())?;
            println!(
                "Created project {} ({}) in {}",
                p.manifest.project.name,
                p.id(),
                out.display()
            );
            println!(
                "Source {} stored byte-for-byte as source/{}",
                p.source_sha256(),
                filename
            );
            let info = &p.manifest.source_info;
            println!(
                "{} Hz, {} ch, {:.2} s ({} / {})",
                info.sample_rate, info.channels, info.duration_s, info.container, info.codec
            );
        }
        Cmd::Inspect { path, json } => {
            let (audio, info, title) = if path.is_dir() {
                let mut p = open_dir(&path)?;
                let a = p.current_render().map_err(|e| e.to_string())?;
                (
                    a,
                    p.manifest.source_info.clone(),
                    format!(
                        "project {} — current stack ({} steps)",
                        p.manifest.project.name,
                        p.state().steps.len()
                    ),
                )
            } else {
                let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let ext = path.extension().and_then(|x| x.to_str());
                let (a, i) = decode(&bytes, ext).map_err(|e| e.to_string())?;
                println!("sha256 {}", nlae_core::hash::sha256(&bytes));
                (a, i, format!("recording {}", path.display()))
            };
            let f = nlae_core::analysis::features(&audio, None);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({ "source": info, "features": f }))
                        .unwrap()
                );
                return Ok(());
            }
            println!("{title}");
            println!(
                "{} Hz, {} ch, {:.2} s, {} / {} ({})",
                info.sample_rate,
                info.channels,
                info.duration_s,
                info.container,
                info.codec,
                info.decoder
            );
            println!(
                "peak {} dBFS (true {}), rms {} dBFS, loudness {}, crest {} dB",
                f.peak_dbfs,
                f.true_peak_dbfs,
                f.rms_dbfs,
                f.loudness_lufs
                    .map(|l| format!("{l} LUFS"))
                    .unwrap_or("—".into()),
                f.crest_db
            );
            println!(
                "DC offset {:?}, clipped samples {}, noise floor {} dBFS, speech activity {}",
                f.dc_offset, f.clipped_samples, f.noise_floor_dbfs, f.speech_activity_ratio
            );
            if let Some(q) = &f.quietest_region {
                println!(
                    "quietest steady region {:.2}–{:.2} s at {} dBFS (stable: {})",
                    q.t0, q.t1, q.level_dbfs, q.stable
                );
            }
            if let Some(h) = &f.hum {
                println!(
                    "hum {} Hz, {} harmonic(s), up to {} dB above its surroundings",
                    h.fundamental_hz, h.harmonics_found, h.strength_db
                );
            }
            println!(
                "bandwidth {} Hz{}",
                f.bandwidth_hz,
                if f.bandwidth_limited {
                    " (band-limited: earlier codec or channel?)"
                } else {
                    ""
                }
            );
            println!("tonal lines ({}):", f.tonal_lines.len());
            for l in &f.tonal_lines {
                println!(
                    "  {:>9.2} Hz  +{:>5.2} dB  width {:>5.2} Hz  present {:>3.0}%",
                    l.freq_hz,
                    l.prominence_db,
                    l.width_hz,
                    l.persistence * 100.0
                );
            }
        }
        Cmd::Preview(a) => {
            let mut p = open_dir(&a.project)?;
            let params: Value =
                serde_json::from_str(&a.params).map_err(|e| format!("--params: {e}"))?;
            let mut d = StepDraft::new(
                &a.op,
                params,
                parse_scope(&a.scope)?,
                Actor::user(),
                Origin::Cli,
            );
            d.intent = a.intent;
            d.note = a.note;
            let opts = PreviewOptions {
                window: parse_window(&a.window)?,
                ..Default::default()
            };
            let pv = p
                .preview(&mut e, vec![d], opts)
                .map_err(|e| e.to_string())?;
            print_preview(&p, &pv, &a.out, &a.residual)?;
        }
        Cmd::Plan(a) => {
            let mut p = open_dir(&a.project)?;
            let recipe = load_recipe(&a.recipe)?;
            run_plan(&mut p, &recipe, &a.common)?;
        }
        Cmd::Replay {
            recipe,
            target,
            into,
            common,
        } => {
            let recipe = load_recipe(&recipe)?;
            let dir = if target.is_dir() {
                target
            } else {
                let dir = into.unwrap_or_else(|| target.with_extension("nlae.d"));
                let bytes =
                    std::fs::read(&target).map_err(|e| format!("{}: {e}", target.display()))?;
                let filename = target
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or("unusable file name")?
                    .to_string();
                let mut created = Project::create(
                    DirStore::new(&dir),
                    &mut e,
                    app(),
                    &bytes,
                    &filename,
                    CreateOptions::default(),
                )
                .map_err(|e| e.to_string())?;
                created.analyse(&mut e).map_err(|e| e.to_string())?;
                println!("Created project in {}", dir.display());
                dir
            };
            let mut p = open_dir(&dir)?;
            run_plan(&mut p, &recipe, &common)?;
        }
        Cmd::Accept {
            project,
            preview,
            note,
            set,
            skip,
        } => {
            let mut p = open_dir(&project)?;
            let mut overrides = BTreeMap::new();
            for s in set {
                let (i, js) = s
                    .split_once(':')
                    .ok_or_else(|| format!("--set `{s}`: use INDEX:JSON"))?;
                let i: usize = i.parse().map_err(|_| format!("--set `{s}`: bad index"))?;
                let v: Value = serde_json::from_str(js).map_err(|e| format!("--set `{s}`: {e}"))?;
                overrides.insert(i.checked_sub(1).ok_or("indices start at 1")?, v);
            }
            let disabled = skip
                .iter()
                .map(|i| i.checked_sub(1).ok_or("indices start at 1"))
                .collect::<Result<Vec<_>, _>>()?;
            let steps = p
                .accept(
                    &mut e,
                    &preview,
                    AcceptOptions {
                        note,
                        overrides,
                        disabled,
                        actor: None,
                    },
                )
                .map_err(|e| e.to_string())?;
            println!(
                "Accepted {} step(s). Stack now has {}:",
                steps.len(),
                p.state().steps.len()
            );
            for (i, s) in p.state().steps.iter().enumerate() {
                println!("{}", show::step_line(i, s));
            }
            for s in &steps {
                let m = show::measurements(s);
                if !m.is_empty() {
                    println!("  {} (whole scope): {m}", s.op);
                }
                if !s.bindings.is_empty() {
                    let b: Vec<String> = s
                        .bindings
                        .iter()
                        .map(|(k, b)| {
                            format!(
                                "{k} → {}",
                                if b.note.is_empty() {
                                    &b.feature
                                } else {
                                    &b.note
                                }
                            )
                        })
                        .collect();
                    println!("  reusable as: {}", b.join("; "));
                }
            }
        }
        Cmd::Reject {
            project,
            preview,
            reason,
        } => {
            let mut p = open_dir(&project)?;
            p.reject(&mut e, &preview, reason, None)
                .map_err(|e| e.to_string())?;
            println!("Rejected {preview} (recorded).");
        }
        Cmd::Undo { project } => {
            let mut p = open_dir(&project)?;
            let s = p.remove_top(&mut e, None).map_err(|e| e.to_string())?;
            println!("Removed {} ({}); it stays in the log.", s.step_id, s.op);
        }
        Cmd::Stack { project, json } => {
            let p = open_dir(&project)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&p.state().steps).unwrap()
                );
                return Ok(());
            }
            println!(
                "{} — {} step(s), stack {}",
                p.manifest.project.name,
                p.state().steps.len(),
                p.state().stack_hash
            );
            if let Some(l) = &p.manifest.lineage {
                println!(
                    "clone of {} ({}) at {}",
                    l.parent_name,
                    l.parent_project,
                    l.forked_at_step.as_deref().unwrap_or("the start")
                );
            }
            for (i, s) in p.state().steps.iter().enumerate() {
                println!("{}", show::step_line(i, s));
            }
            let open: Vec<&str> = p
                .state()
                .open_previews()
                .map(|r| r.preview_id.as_str())
                .collect();
            if !open.is_empty() {
                println!("open previews: {}", open.join(", "));
            }
        }
        Cmd::Log { project, json } => {
            let p = open_dir(&project)?;
            for ev in p.log.events() {
                if json {
                    println!("{}", serde_json::to_string(ev).unwrap());
                } else {
                    println!(
                        "{:>4} {} {:<20} {}",
                        ev["seq"],
                        ev["ts"].as_str().unwrap_or(""),
                        ev["type"].as_str().unwrap_or(""),
                        &ev["hash"].as_str().unwrap_or("")[..19]
                    );
                }
            }
        }
        Cmd::Rate {
            project,
            overall,
            dims,
            target,
            note,
        } => {
            let mut p = open_dir(&project)?;
            let (kind, id) = match target.split_once(':') {
                Some((k, id)) => (k.to_string(), id.to_string()),
                None if target == "stack" => ("stack".to_string(), p.state().stack_hash.clone()),
                None => return Err("--target is step:ID, plan:ID or stack".into()),
            };
            let mut d = BTreeMap::new();
            for s in dims {
                let (k, v) = s
                    .split_once('=')
                    .ok_or_else(|| format!("--dim `{s}`: use NAME=N"))?;
                d.insert(
                    k.to_string(),
                    v.parse::<u8>().map_err(|_| format!("--dim `{s}`: 1–5"))?,
                );
            }
            p.rate(
                &mut e,
                Rating {
                    target_kind: kind,
                    target: id,
                    overall,
                    dims: d,
                    note,
                },
                None,
            )
            .map_err(|e| e.to_string())?;
            println!("Rating recorded.");
        }
        Cmd::Annotate {
            project,
            step,
            note,
            labels,
        } => {
            let mut p = open_dir(&project)?;
            p.annotate(&mut e, &step, note, labels, None)
                .map_err(|e| e.to_string())?;
            println!("Annotation recorded.");
        }
        Cmd::Clone {
            project,
            out,
            at,
            name,
        } => {
            let mut p = open_dir(&project)?;
            if out.exists()
                && std::fs::read_dir(&out)
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false)
            {
                return Err(format!("{} already exists and is not empty", out.display()));
            }
            let c = p
                .clone_into(
                    &mut e,
                    DirStore::new(&out),
                    at.as_deref(),
                    name.as_deref(),
                    None,
                )
                .map_err(|e| e.to_string())?;
            println!(
                "Cloned into {} ({}) with {} inherited step(s).",
                out.display(),
                c.id(),
                c.state().steps.len()
            );
        }
        Cmd::Render {
            project,
            out,
            format,
        } => {
            let mut p = open_dir(&project)?;
            let fmt = WavFormat::parse(&format)
                .ok_or_else(|| format!("unknown format `{format}` (f32 | pcm16 | pcm24)"))?;
            let fin = p.render_final().map_err(|e| e.to_string())?;
            write_audio(&out, &fin.audio, fmt)?;
            let file = out
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            p.record(
                &mut e,
                "render.exported",
                None,
                json!({ "file": file, "format": format, "stack_hash": fin.stack_hash, "output_hash": fin.output_hash, "limiter": fin.limiter }),
            )
            .map_err(|e| e.to_string())?;
            println!(
                "Rendered {} step(s) + limiter → {} ({})",
                p.state().steps.len(),
                out.display(),
                fin.output_hash
            );
            println!("limiter: {}", serde_json::to_string(&fin.limiter).unwrap());
        }
        Cmd::SaveRecipe {
            project,
            out,
            name,
            from,
            to,
        } => {
            let mut p = open_dir(&project)?;
            let n = p.state().steps.len();
            if n == 0 {
                return Err("the stack is empty".into());
            }
            let from = from.unwrap_or(1).max(1) - 1;
            let to = to.unwrap_or(n).min(n) - 1;
            let r = Recipe::from_project(&p, &name, from, to).map_err(|e| e.to_string())?;
            std::fs::write(&out, serde_json::to_vec_pretty(&r).unwrap())
                .map_err(|e| format!("{}: {e}", out.display()))?;
            p.record(
                &mut e,
                "recipe.saved",
                None,
                json!({ "name": name, "hash": r.hash(), "from": from + 1, "to": to + 1 }),
            )
            .map_err(|e| e.to_string())?;
            println!(
                "Saved steps {}–{} as recipe `{name}` → {} ({})",
                from + 1,
                to + 1,
                out.display(),
                r.hash()
            );
            for (i, s) in r.steps.iter().enumerate() {
                let keys: Vec<&String> = s.bindings.keys().collect();
                println!(
                    "  {}. {:<20} reusable values: {}",
                    i + 1,
                    s.op,
                    if keys.is_empty() {
                        "auto / as given".into()
                    } else {
                        format!("{keys:?}")
                    }
                );
            }
        }
        Cmd::Dataset {
            projects,
            out,
            with_audio,
        } => {
            let mut opened = Vec::new();
            for dir in &projects {
                opened.push(open_dir(dir)?);
            }
            let data = opened
                .iter_mut()
                .map(|p| data_of(p, with_audio))
                .collect::<Res<Vec<_>>>()?;
            let created = nlae_core::provenance::Env::now(&mut e);
            let opts = ExportOptions {
                audio: if with_audio {
                    AudioInclusion::Full
                } else {
                    AudioInclusion::None
                },
            };
            let ex = dataset::export(&data, &opts, &created, &app());
            for (name, bytes) in &ex.files {
                let path = out.join(name);
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
                }
                std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            }
            let manifest_hash = nlae_core::hash::sha256(&ex.files["manifest.json"]);
            for p in opened.iter_mut() {
                p.record(&mut e, "dataset.exported", None, json!({ "manifest_hash": manifest_hash, "with_audio": with_audio, "counts": ex.counts }))
                    .map_err(|e| e.to_string())?;
            }
            println!("Exported {} project(s) → {}", projects.len(), out.display());
            println!("{}", serde_json::to_string(&ex.counts).unwrap());
        }
        Cmd::Pack { project, out } => {
            let mut p = open_dir(&project)?;
            let bytes = p.export_bundle(&mut e, None).map_err(|e| e.to_string())?;
            std::fs::write(&out, &bytes).map_err(|e| format!("{}: {e}", out.display()))?;
            println!(
                "Packed → {} ({} bytes, {})",
                out.display(),
                bytes.len(),
                nlae_core::hash::sha256(&bytes)
            );
        }
        Cmd::Unpack { bundle: file, out } => {
            let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let mem: MemStore = bundle::unpack(&bytes).map_err(|e| e.to_string())?;
            let mut dir = DirStore::new(&out);
            for (path, data) in &mem.files {
                dir.write_new(path, data).map_err(|e| e.to_string())?;
            }
            let (_, report) =
                Project::open(DirStore::new(&out), app()).map_err(|e| e.to_string())?;
            show::report(&report);
        }
        Cmd::Verify { path } => {
            let report = if path.is_dir() {
                Project::open(DirStore::new(&path), app())
                    .map_err(|e| e.to_string())?
                    .1
            } else {
                let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                open_bundle(&bytes, app()).map_err(|e| e.to_string())?.1
            };
            show::report(&report);
            if !report.ok() {
                return Err("verification failed".into());
            }
        }
        Cmd::Spectrogram {
            path,
            out,
            what,
            from,
            to,
            fmax,
            log,
            width,
            height,
            range,
        } => {
            let audio = if path.is_dir() {
                let mut p = open_dir(&path)?;
                match what.as_str() {
                    "source" => p.source.clone(),
                    "stack" => p.current_render().map_err(|e| e.to_string())?,
                    "residual" => p.residual_render().map_err(|e| e.to_string())?,
                    w => return Err(format!("--what `{w}`: source | stack | residual")),
                }
            } else {
                let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                decode(&bytes, path.extension().and_then(|x| x.to_str()))
                    .map_err(|e| e.to_string())?
                    .0
            };
            let t0 = from.unwrap_or(0.0);
            let t1 = to.unwrap_or(audio.duration_s());
            png::spectrogram_png(&audio, &out, t0, t1, fmax, log, width, height, range)?;
            println!(
                "Spectrogram {:.2}–{:.2} s, 0–{fmax} Hz → {}",
                t0,
                t1,
                out.display()
            );
        }
        Cmd::Golden { out, short } => {
            std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
            let mut hashes = serde_json::Map::new();
            let specs = if short {
                vec![nlae_core::golden::spec_a_short()]
            } else {
                vec![nlae_core::golden::spec_a(), nlae_core::golden::spec_b()]
            };
            for spec in specs {
                let clip = nlae_core::golden::generate(&spec);
                let mut files = vec![(format!("{}.wav", spec.name), clip.mix.clone())];
                if !short {
                    for (name, c) in &clip.components {
                        files.push((format!("{}.{name}.wav", spec.name), c.clone()));
                    }
                }
                for (name, audio) in files {
                    let bytes = write_wav(&audio, WavFormat::F32);
                    hashes.insert(name.clone(), json!(nlae_core::hash::sha256(&bytes)));
                    std::fs::write(out.join(&name), bytes).map_err(|e| format!("{name}: {e}"))?;
                }
                std::fs::write(
                    out.join(format!("{}.json", spec.name)),
                    serde_json::to_vec_pretty(&clip.meta()).unwrap(),
                )
                .map_err(|e| e.to_string())?;
            }
            std::fs::write(
                out.join("expected.json"),
                serde_json::to_vec_pretty(&hashes).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            println!("Wrote {} golden files to {}", hashes.len(), out.display());
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
