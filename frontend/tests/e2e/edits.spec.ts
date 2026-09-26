// Time edits end to end: remove a selected stretch and insert silence at the
// playhead; playback skips what was removed; the export is shorter, with cue
// markers where the edits are; undo restores the original length.
import { expect, test, type Page } from "@playwright/test";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { drag, importClip, logged, nlae, start } from "./helpers";

type EditMap = { edited: boolean; duration_s: number; marks: { kind: string; from_s?: number; to_s?: number; at_s?: number }[] };

const editMap = async (page: Page) => (await nlae(page)) as unknown as { editMap: EditMap };

/** The chunks of a RIFF/WAVE file. */
function chunks(b: Buffer): Record<string, Buffer> {
  const out: Record<string, Buffer> = {};
  let i = 12;
  while (i + 8 <= b.length) {
    const id = b.toString("latin1", i, i + 4);
    const n = b.readUInt32LE(i + 4);
    out[id] = b.subarray(i + 8, i + 8 + n);
    i += 8 + n + (n % 2);
  }
  return out;
}

test("a selected stretch is removed and silence inserted; playback skips the removal; the export is shorter and marked", async ({
  page,
}) => {
  await start(page);
  await importClip(page);
  // 12 s clip: select 4–6 s on the waveform.
  await drag(page, "waveform", [4 / 12, 0.5], [6 / 12, 0.5]);
  await expect.poll(async () => (await nlae(page)).view.selection).not.toBeNull();
  const sel = (await nlae(page)).view.selection!;

  await page.getByTestId("remove-stretch").click();
  const turn = page.getByTestId("turn").first();
  await expect(turn.getByTestId("applied-step")).toHaveAttribute("data-op", "remove_time");
  // Applied at once: the lanes keep the original's timeline, the stretch marked where it was.
  await expect(page.getByTestId("edit-removed")).toBeVisible();
  const removed = sel.t1 - sel.t0;
  await expect.poll(async () => (await editMap(page)).editMap?.duration_s).toBeCloseTo(12 - removed, 3);

  // Processed playback jumps over the removed stretch: from 3.8 s it passes
  // 6.2 s long before the 2.4 s that playing through would take.
  await page.getByTestId("monitor-stack").click();
  const box = (await page.getByTestId("waveform").boundingBox())!;
  await page.mouse.click(box.x + box.width * (3.8 / 12), box.y + box.height / 2);
  await page.getByTestId("play").click();
  await expect.poll(async () => (await nlae(page)).playhead, { timeout: 1800, intervals: [50] }).toBeGreaterThan(6.2);
  await page.getByTestId("stop").click();

  // Insert 0.5 s of silence at 9 s.
  await page.mouse.click(box.x + box.width * (9 / 12), box.y + box.height / 2);
  await expect.poll(async () => (await nlae(page)).playhead).toBeCloseTo(9, 1);
  await page.getByTestId("insert-length").fill("0.5");
  await page.getByTestId("insert-silence").click();
  const second = page.getByTestId("turn").nth(1);
  await expect(second.getByTestId("applied-step")).toHaveAttribute("data-op", "insert_silence");
  await expect(page.getByTestId("edit-inserted")).toBeVisible();
  await expect(page.getByTestId("output-length")).toContainText(`Output ${(12 - removed + 0.5).toFixed(3)} s (original 12.000 s)`);

  // The export: shorter by what was removed, longer by the silence, with a
  // cue marker at each edit; its hash is the logged one.
  const [download] = await Promise.all([page.waitForEvent("download"), page.getByTestId("export-wav").click()]);
  const bytes = readFileSync(await download.path());
  const c = chunks(bytes);
  const frames = c["data"].length / 4; // 32-bit float, mono
  expect(frames / 48000).toBeCloseTo(12 - removed + 0.5, 3);
  expect(c["cue "].readUInt32LE(0)).toBe(2);
  const labels = c["LIST"].toString("latin1");
  expect(labels).toContain(`removed ${sel.t0.toFixed(3)}-${sel.t1.toFixed(3)} s of the original`);
  expect(labels).toContain("inserted 0.500 s of silence at 9.000 s of the original");
  const [exported] = await logged(page, "render.exported");
  expect(exported.data.file_sha256).toBe(`sha256:${createHash("sha256").update(bytes).digest("hex")}`);
  expect((exported.data.edits as unknown[]).length).toBe(2);

  // Undo, twice: the original length again, nothing marked.
  await page.getByTestId("undo").click();
  await expect(page.getByTestId("edit-inserted")).toHaveCount(0);
  await page.getByTestId("undo").click();
  await expect(page.getByTestId("edit-removed")).toHaveCount(0);
  await expect(page.getByTestId("output-length")).toHaveCount(0);
  await expect.poll(async () => (await editMap(page)).editMap?.edited).toBe(false);
});
