// Builds the fixtures with the command-line tool: a short golden clip, a
// project processed with the built-in recipe and packed as a bundle (and
// rendered to a WAV file), and a copy of that bundle with one logged value
// changed. Starts the stand-in model provider and the stand-in recogniser
// for the console tests.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { startMockDeepgram } from "./mock-deepgram";
import { startMockModel } from "./mock-model";
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
  /** The command line's 32-bit float WAV render of that stack, as it logged it (`render.exported`). */
  cliRender: { stack_hash: string; output_hash: string; file_sha256: string; limiter: Record<string, number> };
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

export default async function globalSetup() {
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
  // After packing: the render is logged, and the bundle stays as it was.
  nlae("render", "cli-case", "-o", "cli-case.wav", "--format", "f32");

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

  const renderEvent = readFileSync(join(FIX, "cli-case", "events.jsonl"), "utf8")
    .trimEnd()
    .split("\n")
    .map((l) => JSON.parse(l))
    .find((e) => e.type === "render.exported");
  const fileSha = `sha256:${createHash("sha256")
    .update(readFileSync(join(FIX, "cli-case.wav")))
    .digest("hex")}`;
  if (renderEvent?.data.file_sha256 !== fileSha) throw new Error("the command line's render log does not match its file");

  const fx: Fixtures = {
    wav,
    sourceSha: `sha256:${createHash("sha256").update(readFileSync(wav)).digest("hex")}`,
    cliBundle: join(FIX, "cli-case.nlae"),
    cliProjectId: manifest.project.id,
    cliStackHash: manifest.stack.stack_hash,
    cliSteps: steps.map((s) => s.op as string),
    cliStepStackHashes: steps.map((s) => s.stack_hash as string),
    cliOutputHash: last.output_hash as string,
    cliRender: renderEvent.data as Fixtures["cliRender"],
    tamperedBundle: join(FIX, "cli-case-tampered.nlae"),
    tamperedLine,
    tamperedSeq: tamperedLine - 1,
  };
  writeFileSync(join(FIX, "fixtures.json"), JSON.stringify(fx, null, 2));

  const servers = [await startMockModel(), await startMockDeepgram()];
  return () => Promise.all(servers.map((s) => new Promise<void>((resolve) => s.close(() => resolve()))));
}
