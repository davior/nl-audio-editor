// Builds the fixtures with the command-line tool: a short golden clip, a
// project processed with the built-in recipe and packed as a bundle, and a
// copy of that bundle with one logged value changed.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { FIX, REPO } from "./paths";
import { patchStoredEntry } from "./zip";

export interface Fixtures {
  wav: string;
  sourceSha: string;
  cliBundle: string;
  cliProjectId: string;
  cliStackHash: string;
  cliSteps: string[];
  cliStepStackHashes: string[];
  cliOutputHash: string;
  tamperedBundle: string;
  tamperedLine: number;
  tamperedSeq: number;
}

function binary(): string {
  if (process.env.NLAE_BIN) return process.env.NLAE_BIN;
  // Cargo is quick when nothing changed; a stale binary would test old code.
  execFileSync("cargo", ["build", "--release", "-p", "nlae-cli", "--quiet"], { cwd: REPO, stdio: "inherit" });
  return join(REPO, "target", "release", process.platform === "win32" ? "nlae.exe" : "nlae");
}

type Json = null | boolean | number | string | Json[] | { [k: string]: Json };

/** Every object in `v` (depth first) that has all of `keys`. */
function objectsWith(v: Json, keys: string[], out: Record<string, Json>[] = []): Record<string, Json>[] {
  if (Array.isArray(v)) v.forEach((x) => objectsWith(x, keys, out));
  else if (v && typeof v === "object") {
    if (keys.every((k) => k in v)) out.push(v);
    Object.values(v).forEach((x) => objectsWith(x, keys, out));
  }
  return out;
}

export default function globalSetup() {
  rmSync(FIX, { recursive: true, force: true });
  mkdirSync(FIX, { recursive: true });
  const bin = binary();
  const nlae = (...args: string[]) => execFileSync(bin, args, { cwd: FIX, encoding: "utf8" });

  nlae("golden", "--short", ".");
  const wav = join(FIX, "golden_a_short.wav");
  nlae("new", "golden_a_short.wav", "-o", "cli-case");
  const plan = nlae("plan", "cli-case", "--recipe", "builtin:spoken-word-cleanup");
  const preview = /pv_[a-z0-9]+/.exec(plan)?.[0];
  if (!preview) throw new Error(`no preview id in:\n${plan}`);
  nlae("accept", "cli-case", preview, "--note", "accepted from the command line");
  nlae("pack", "cli-case", "-o", "cli-case.nlae");

  const manifest = JSON.parse(readFileSync(join(FIX, "cli-case", "manifest.json"), "utf8"));
  const events: Json[] = readFileSync(join(FIX, "cli-case", "events.jsonl"), "utf8")
    .trimEnd()
    .split("\n")
    .map((l) => JSON.parse(l));
  const accepted = events.filter((e) => (e as { type: string }).type === "plan.accepted");
  const steps = objectsWith(accepted, ["step_id", "op", "output_hash", "stack_hash"]);
  const last = steps[steps.length - 1];

  // Change the noise reduction recorded in the plan (12 dB → 24 dB) and fix the CRCs.
  let tamperedLine = 0;
  const tampered = patchStoredEntry(readFileSync(join(FIX, "cli-case.nlae")), "events.jsonl", (data) => {
    const lines = data.toString("utf8").split("\n");
    const i = lines.findIndex((l) => l.includes('"reduction_db":12'));
    if (i < 0) throw new Error("no reduction_db in the log");
    lines[i] = lines[i].replace('"reduction_db":12', '"reduction_db":24');
    tamperedLine = i + 1;
    return Buffer.from(lines.join("\n"), "utf8");
  });
  writeFileSync(join(FIX, "cli-case-tampered.nlae"), tampered);

  const fx: Fixtures = {
    wav,
    sourceSha: `sha256:${createHash("sha256").update(readFileSync(wav)).digest("hex")}`,
    cliBundle: join(FIX, "cli-case.nlae"),
    cliProjectId: manifest.project.id,
    cliStackHash: manifest.stack.stack_hash,
    cliSteps: steps.map((s) => s.op as string),
    cliStepStackHashes: steps.map((s) => s.stack_hash as string),
    cliOutputHash: last.output_hash as string,
    tamperedBundle: join(FIX, "cli-case-tampered.nlae"),
    tamperedLine,
    tamperedSeq: tamperedLine - 1,
  };
  writeFileSync(join(FIX, "fixtures.json"), JSON.stringify(fx, null, 2));
}
