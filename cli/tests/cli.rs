//! The command-line workflow end to end on a short golden clip.

use std::path::{Path, PathBuf};
use std::process::Command;

use nlae_core::audio::wav::{write_wav, WavFormat};

fn nlae(args: &[&str], dir: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_nlae"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("runs");
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "nlae {args:?} failed:\n{text}");
    text
}

fn preview_id(text: &str) -> String {
    let at = text.find("pv_").expect("a preview id");
    text[at..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

fn workdir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("nlae-cli-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn record_preview_accept_replay_export() {
    let dir = workdir();
    let clip = nlae_core::golden::generate(&nlae_core::golden::spec_a_short());
    std::fs::write(dir.join("clip.wav"), write_wav(&clip.mix, WavFormat::F32)).unwrap();

    nlae(&["new", "clip.wav", "-o", "case"], &dir);
    assert!(nlae(&["inspect", "case"], &dir).contains("749."));
    let out = nlae(
        &[
            "preview",
            "case",
            "--op",
            "gain",
            "--params",
            "{\"gain_db\":40}",
        ],
        &dir,
    );
    nlae(
        &["reject", "case", &preview_id(&out), "--reason", "too loud"],
        &dir,
    );
    let out = nlae(
        &[
            "plan",
            "case",
            "--recipe",
            "builtin:spoken-word-cleanup",
            "--out",
            "p.wav",
            "--residual",
            "r.wav",
        ],
        &dir,
    );
    assert!(
        out.contains("Dry-run diff") && dir.join("p.wav").exists() && dir.join("r.wav").exists()
    );
    nlae(&["accept", "case", &preview_id(&out)], &dir);
    assert!(nlae(&["stack", "case"], &dir).contains("line_reduce"));
    nlae(&["render", "case", "-o", "clean.wav"], &dir);
    nlae(
        &["save-recipe", "case", "-o", "r.json", "--name", "mine"],
        &dir,
    );
    let out = nlae(
        &[
            "replay",
            "r.json",
            "clip.wav",
            "--into",
            "case2",
            "--dry-run",
        ],
        &dir,
    );
    assert!(out.contains("measured on this clip"));
    nlae(&["clone", "case", "-o", "copy"], &dir);
    nlae(&["pack", "copy", "-o", "copy.nlae"], &dir);
    assert!(nlae(&["verify", "copy.nlae"], &dir).contains("VERIFIED"));
    nlae(&["dataset", "case", "copy", "-o", "ds"], &dir);
    let steps = std::fs::read_to_string(dir.join("ds/steps.jsonl")).unwrap();
    assert!(steps.contains("\"rejected\""));
    nlae(
        &[
            "spectrogram",
            "case",
            "--what",
            "residual",
            "-o",
            "r.png",
            "--width",
            "200",
            "--height",
            "80",
        ],
        &dir,
    );
    assert!(std::fs::metadata(dir.join("r.png")).unwrap().len() > 100);
    let _ = std::fs::remove_dir_all(&dir);
}

fn nlae_fails(args: &[&str], dir: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_nlae"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("runs");
    assert!(!out.status.success(), "nlae {args:?} should have failed");
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn ask_in_plain_words_locally_and_through_a_model() {
    let dir = workdir().join("ask");
    std::fs::create_dir_all(&dir).unwrap();
    let clip = nlae_core::golden::generate(&nlae_core::golden::spec_a_short());
    std::fs::write(dir.join("clip.wav"), write_wav(&clip.mix, WavFormat::F32)).unwrap();
    nlae(&["new", "clip.wav", "-o", "case"], &dir);

    // Stated numbers are handled here, without a model.
    let out = nlae(&["ask", "case", "cut 3,100 to 3,200 Hz by 12 dB"], &dir);
    assert!(
        out.contains("Handled here: Cut 3100–3200 Hz by 12 dB"),
        "{out}"
    );
    nlae(&["accept", "case", &preview_id(&out)], &dir);

    // A model's answer (saved, so no network): validated, logged, previewed.
    let answer = serde_json::json!({ "choices": [{ "message": { "role": "assistant",
        "content": "The hum at 50 Hz and its harmonics stand out; reducing them.",
        "tool_calls": [{ "id": "call_1", "type": "function", "function": {
            "name": "line_reduce",
            "arguments": "{\"params\":{\"lines\":\"auto\"},\"scope\":{\"kind\":\"band\",\"f_lo\":40,\"f_hi\":400}}" } }] } }] });
    std::fs::write(dir.join("answer.json"), answer.to_string()).unwrap();
    let out = nlae(
        &[
            "ask",
            "case",
            "the hum is distracting",
            "--response-file",
            "answer.json",
        ],
        &dir,
    );
    assert!(out.contains("deepseek-chat: The hum at 50 Hz"), "{out}");
    nlae(&["accept", "case", &preview_id(&out)], &dir);
    let log = nlae(&["log", "case", "--json"], &dir);
    assert!(log.contains("assistant.exchange"), "{log}");
    let stack = nlae(&["stack", "case", "--json"], &dir);
    let steps: serde_json::Value = serde_json::from_str(&stack).unwrap();
    let last = &steps[1];
    assert_eq!(last["op"], "line_reduce");
    assert_eq!(last["actor"]["model"], "deepseek-chat");
    assert_eq!(last["actor"]["provider"], "deepseek");
    assert_eq!(last["intent"], "the hum is distracting");

    // An answer out of range is refused, never clamped (and still logged).
    let bad = serde_json::json!({ "choices": [{ "message": { "role": "assistant", "content": null,
        "tool_calls": [{ "id": "call_2", "type": "function", "function": {
            "name": "gain", "arguments": "{\"params\":{\"gain_db\":90},\"scope\":{\"kind\":\"clip\"}}" } }] } }] });
    std::fs::write(dir.join("bad.json"), bad.to_string()).unwrap();
    let err = nlae_fails(
        &[
            "ask",
            "case",
            "make it much louder",
            "--response-file",
            "bad.json",
        ],
        &dir,
    );
    assert!(err.contains("gain_db"), "{err}");

    // Undo in words; and the dataset carries the exchange as it happened.
    assert!(nlae(&["ask", "case", "undo"], &dir).contains("stays in the log"));
    nlae(&["dataset", "case", "-o", "ds"], &dir);
    let chat = std::fs::read_to_string(dir.join("ds/chat.jsonl")).unwrap();
    assert!(chat.contains("\"source\":\"exchange\""), "{chat}");
    assert!(chat.contains("\"source\":\"reconstructed\""), "{chat}");
    let _ = std::fs::remove_dir_all(&dir);
}
