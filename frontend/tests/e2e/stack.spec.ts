// The stack end to end: requests applied at once; a step removed from the
// middle keeps its place and comes back; a step measured on audio that has
// changed is measured again; values changed in place; one step heard on its
// own; undo and redo, kept across a reload; and the export afterwards is the
// command line's render of the same project, byte for byte.
import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { importClip, logged, nlae, say, start, type Event } from "./helpers";
import { FIX, REPO } from "./paths";

const sha = (b: Buffer) => `sha256:${createHash("sha256").update(b).digest("hex")}`;

/** The command-line tool the global setup built. */
const cli = (...args: string[]) =>
  execFileSync(process.env.NLAE_BIN ?? join(REPO, "target", "release", "nlae"), args, { cwd: FIX, encoding: "utf8" });

/** The operations in the stack panel, in order: active ones plain, removed ones marked. */
const listed = (page: Page) =>
  page
    .getByTestId("stack")
    .locator("ol.steps > li")
    .evaluateAll((els) => els.map((e) => `${e.getAttribute("data-op")}${e.classList.contains("removed") ? " (removed)" : ""}`));

/** A step as the latest event that recorded it has it. */
function latest(events: Event[], id: string): Record<string, unknown> {
  for (const e of [...events].reverse()) {
    const d = e.data as { chain?: Record<string, unknown>[]; steps?: Record<string, unknown>[] };
    const found = [...(d.chain ?? []), ...(d.steps ?? [])].find((s) => s.step_id === id);
    if (found) return found;
  }
  throw new Error(`no version of ${id}`);
}

test("a step is removed from the middle and restored, measured again, changed and heard on its own; undo and redo; the export is the command line's render", async ({
  page,
}) => {
  await start(page);
  await importClip(page);
  for (const words of ["high-pass at 80 Hz", "turn it down by 6 dB", "normalise to -1 dB"]) {
    await expect(await say(page, words)).toHaveAttribute("data-status", "applied");
  }
  await expect(page.getByTestId("stack-step")).toHaveCount(3);
  expect(await listed(page)).toEqual(["high_pass", "gain", "normalise"]);

  // Remove the gain: it keeps its place; the normalise above keeps its values
  // but was measured with the gain in place.
  await page.getByTestId("stack-step").nth(1).getByTestId("remove-step").click();
  await page.getByTestId("remove-confirm").click();
  const [excluded] = await logged(page, "step.excluded");
  expect((excluded.data.chain as { op: string }[]).map((s) => s.op)).toEqual(["normalise"]);
  expect(await listed(page)).toEqual(["high_pass", "gain (removed)", "normalise"]);
  await expect(page.getByTestId("approval-note")).toContainText("2 steps; 1 removed");

  // The drift badge opens the step, with what measuring again would change.
  await page.getByTestId("drifted").click();
  const editor = page.getByTestId("step-editor");
  await expect(editor.getByTestId("remeasure-diff")).toContainText("gain_db");
  await editor.getByTestId("remeasure").click();
  const [remeasured] = await logged(page, "step.edited");
  expect(remeasured.data.remeasured).toBe(true);
  await expect(page.getByTestId("drifted")).toHaveCount(0);

  // Change the high-pass in place: it is measured again; the steps above keep their values.
  const hp = page.getByTestId("stack-step").nth(0);
  await hp.getByTestId("select-step").click();
  await expect(page.getByTestId("step-editor")).toHaveAttribute("data-step", (await hp.getAttribute("data-step"))!);
  await page.getByTestId("param-cutoff_hz").fill("120");
  await page.getByTestId("apply-edit").click();
  const edits = await logged(page, "step.edited", 2);
  expect((edits[1].data.to_params as Record<string, unknown>).cutoff_hz).toBe(120);
  await expect(hp).toContainText("edited");

  // Heard on its own: after it is exactly what the log records as its output.
  const hpId = (await hp.getAttribute("data-step"))!;
  await expect(page.getByTestId("monitor-step-after")).toBeVisible();
  await page.getByTestId("monitor-step-after").click();
  await expect.poll(async () => (await nlae(page)).view.monitor).toBe(`step:${hpId}:after`);
  await page.getByTestId("compute-render-hash").click();
  const events = await logged(page, "step.edited", 2);
  const recorded = latest(events, hpId).output_hash as string;
  await expect(page.getByTestId("render-hash")).toHaveAttribute("title", recorded);
  await page.getByTestId("monitor-step-removed").click();
  await expect.poll(async () => (await nlae(page)).view.monitor).toBe(`step:${hpId}:removed`);

  // Undo takes the change back; redo repeats it.
  await expect(page.getByTestId("undo")).toHaveAttribute("title", /changing high_pass/);
  await page.getByTestId("undo").click();
  const undone = await logged(page, "step.edited", 3);
  expect(typeof undone[2].data.undoes).toBe("string");
  expect((undone[2].data.to_params as Record<string, unknown>).cutoff_hz).toBe(80);
  await page.getByTestId("redo").click();
  const redone = await logged(page, "step.edited", 4);
  expect(typeof redone[3].data.redoes).toBe("string");

  // Restore the gain to its place: the normalise was measured without it.
  await page.getByTestId("restore-step").click();
  await logged(page, "step.restored");
  expect(await listed(page)).toEqual(["high_pass", "gain", "normalise"]);
  await expect(page.getByTestId("drifted")).toHaveCount(1);
  const hash = (await nlae(page)).project.stackHash;

  // A reload keeps the stack and what Undo would take back.
  await page.reload();
  await page.waitForFunction(() => window.__nlae?.ready === true);
  await expect.poll(async () => (await nlae(page)).project.stackHash).toBe(hash);
  await expect(page.getByTestId("undo")).toHaveAttribute("title", /restoring gain/);
  await expect(page.getByTestId("redo")).toBeDisabled();

  // Export, and the bundle: the command line renders the same file from it.
  const [wav] = await Promise.all([page.waitForEvent("download"), page.getByTestId("export-wav").click()]);
  const exported = readFileSync(await wav.path());
  const [bundle] = await Promise.all([page.waitForEvent("download"), page.getByTestId("save-bundle").click()]);
  const dir = join(FIX, `stack-${Date.now()}`);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "case.nlae"), readFileSync(await bundle.path()));
  expect(cli("verify", join(dir, "case.nlae"))).toContain("VERIFIED");
  cli("unpack", join(dir, "case.nlae"), "-o", join(dir, "case"));
  cli("render", join(dir, "case"), "-o", join(dir, "cli.wav"), "--format", "f32");
  expect(sha(readFileSync(join(dir, "cli.wav")))).toBe(sha(exported));
});
