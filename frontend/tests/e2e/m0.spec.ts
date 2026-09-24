// The M0 interface end to end, in Chromium with a fake microphone.
import { expect, test } from "@playwright/test";
import { drag, fx, importClip, lanesDrawn, nlae, start, viewSaved } from "./helpers";

test("1. a recording loads and both lanes render", async ({ page }) => {
  await start(page);
  await importClip(page);
  const s = await nlae(page);
  // The original is kept byte for byte: its hash is the file's hash.
  expect(s.project.sourceSha).toBe(fx.sourceSha);
  expect(s.project.steps).toBe(0);
  await expect(page.getByTestId("integrity")).toHaveAttribute("data-ok", "true");
  await expect(page.getByTestId("library-entry")).toHaveCount(1);
  await expect(page.getByTestId("project-name")).toHaveText("golden_a_short");
  // The lanes come first; the analysis follows in its own worker and is logged.
  await expect.poll(async () => (await nlae(page)).analysis).toBe("done");
  await expect(page.getByTestId("analysing")).toHaveCount(0);
  await page.getByTestId("log-toggle").click();
  await expect(page.getByTestId("log")).toContainText("analysis.computed");
});

test("2. seek and play", async ({ page }) => {
  await start(page);
  await importClip(page);
  const box = (await page.getByTestId("waveform").boundingBox())!;
  await page.mouse.click(box.x + box.width * 0.25, box.y + box.height / 2);
  await expect.poll(async () => (await nlae(page)).playhead).toBeCloseTo(3, 1);

  await page.getByTestId("play").click();
  await expect.poll(async () => (await nlae(page)).playing).toBe(true);
  await expect.poll(async () => (await nlae(page)).playhead, { timeout: 10_000 }).toBeGreaterThan(3.4);

  await page.getByTestId("play").click();
  await expect.poll(async () => (await nlae(page)).playing).toBe(false);
  const paused = (await nlae(page)).playhead;
  await page.waitForTimeout(300);
  expect((await nlae(page)).playhead).toBe(paused);

  // Stop returns to where playback started.
  await page.getByTestId("play").click();
  await expect.poll(async () => (await nlae(page)).playing).toBe(true);
  await page.getByTestId("stop").click();
  await expect.poll(async () => (await nlae(page)).playhead).toBeCloseTo(paused, 3);
});

test("3. a time range and a time × frequency area can be selected", async ({ page }) => {
  await start(page);
  await importClip(page);
  // Linear scale up to 8 kHz on a 12 s recording: x → 12·u s, y → 8000·(1 − v) Hz.
  await drag(page, "spectrogram", [0.25, 0.9], [0.5, 0.8]);
  await expect.poll(async () => (await nlae(page)).view.tfSelection).not.toBeNull();
  const tf = (await nlae(page)).view.tfSelection!;
  expect(tf.t0).toBeCloseTo(3, 1);
  expect(tf.t1).toBeCloseTo(6, 1);
  expect(Math.abs(tf.f_lo - 800)).toBeLessThan(40);
  expect(Math.abs(tf.f_hi - 1600)).toBeLessThan(40);
  await expect(page.getByTestId("tf-selection")).toContainText("Area 3.0");

  await drag(page, "waveform", [0.5, 0.5], [0.75, 0.5]);
  await expect.poll(async () => (await nlae(page)).view.selection).not.toBeNull();
  const sel = (await nlae(page)).view.selection!;
  expect(sel.t0).toBeCloseTo(6, 1);
  expect(sel.t1).toBeCloseTo(9, 1);
  await expect(page.getByTestId("play-selection")).toBeEnabled();
});

test("4. save, reload and reopen: hash, view and selection are identical and the chain verifies", async ({ page, browser }) => {
  await start(page);
  await importClip(page);
  await page.getByTestId("zoom-in").click();
  await page.getByTestId("zoom-in").click();
  await page.getByTestId("freq-scale").selectOption("log");
  await page.getByTestId("db-range").selectOption("60");
  await page.getByTestId("monitor-stack").click();
  await drag(page, "spectrogram", [0.2, 0.7], [0.6, 0.4]);
  await drag(page, "waveform", [0.3, 0.5], [0.5, 0.5]);
  await viewSaved(page);
  const before = await nlae(page);
  expect(before.view.t1 - before.view.t0).toBeCloseTo(3, 6);
  expect(before.view.tfSelection).not.toBeNull();
  expect(before.view.selection).not.toBeNull();

  await page.reload();
  await page.waitForFunction(() => window.__nlae?.ready === true);
  await expect(page.getByTestId("editor")).toBeVisible();
  // The editor reads the log after it appears: wait for it, and reopening logs nothing.
  await expect.poll(async () => (await nlae(page)).project.events).toBe(before.project.events);
  const after = await nlae(page);
  expect(after.view).toEqual(before.view);
  expect(after.project.id).toBe(before.project.id);
  expect(after.project.sourceSha).toBe(fx.sourceSha);
  expect(after.project.stackHash).toBe(before.project.stackHash);
  const badge = page.getByTestId("integrity");
  await expect(badge).toHaveAttribute("data-ok", "true");
  await badge.getByRole("button").click();
  await expect(badge).toContainText(`Log: ${after.project.events} events verified`);

  // The bundle carries the same state to a browser profile that has never seen the project.
  const [download] = await Promise.all([page.waitForEvent("download"), page.getByTestId("save-bundle").click()]);
  expect(download.suggestedFilename()).toBe("golden_a_short.nlae");
  const bundle = await download.path();
  const other = await browser.newContext();
  const page2 = await other.newPage();
  await start(page2);
  await page2.getByTestId("bundle-input").setInputFiles(bundle);
  await expect(page2.getByTestId("editor")).toBeVisible();
  const opened = await nlae(page2);
  expect(opened.view).toEqual(before.view);
  expect(opened.project.id).toBe(before.project.id);
  expect(opened.project.sourceSha).toBe(fx.sourceSha);
  // The export itself is logged, so the bundle holds one more event.
  expect(opened.project.events).toBe(before.project.events + 1);
  await expect(page2.getByTestId("integrity")).toHaveAttribute("data-ok", "true");
  await other.close();
});

test("5. a clone shows its lineage and verifies back to the import", async ({ page }) => {
  await start(page);
  await page.getByTestId("bundle-input").setInputFiles(fx.cliBundle);
  await expect(page.getByTestId("stack-step")).toHaveCount(4);
  const parent = (await nlae(page)).project;

  await page.getByTestId("clone-here").nth(1).click();
  await expect(page.getByTestId("lineage")).toContainText("Clone of golden_a_short after step 2 (noise_reduce)");
  await expect(page.getByTestId("stack-step")).toHaveCount(2);
  await expect(page.getByTestId("stack-step").first()).toContainText("inherited");
  const child = (await nlae(page)).project;
  expect(child.parent).toBe(parent.id);
  expect(child.id).not.toBe(parent.id);
  expect(child.sourceSha).toBe(parent.sourceSha);
  expect(child.stackHash).toBe(fx.cliStepStackHashes[1]);

  // In the library the clone sits under the project it came from.
  const entry = page.locator(`[data-testid="library-entry"][data-id="${child.id}"]`);
  await expect(entry).toHaveAttribute("data-parent", parent.id);
  await expect(entry).toHaveAttribute("data-depth", "1");

  // Reopened from storage, the clone's log and its parent's log both verify.
  await page.reload();
  await page.waitForFunction(() => window.__nlae?.ready === true);
  await expect(page.getByTestId("lineage")).toBeVisible();
  const reopened = (await nlae(page)).project;
  expect(reopened.id).toBe(child.id);
  expect(reopened.lineageVerified).toEqual([true]);
  await expect(page.getByTestId("integrity")).toHaveAttribute("data-ok", "true");

  // The parent's own log records the clone.
  await page.locator(`[data-testid="library-entry"][data-id="${parent.id}"]`).click();
  await expect.poll(async () => (await nlae(page)).project.id).toBe(parent.id);
  await page.getByTestId("log-toggle").click();
  await expect(page.getByTestId("log")).toContainText("project.clone_made");
});

test("6. a tampered bundle is flagged at the edited event and opens read-only", async ({ page }) => {
  await start(page);
  await page.getByTestId("bundle-input").setInputFiles(fx.tamperedBundle);
  const badge = page.getByTestId("integrity");
  await expect(badge).toHaveAttribute("data-ok", "false");
  await badge.getByRole("button").click();
  await expect(page.getByTestId("integrity-problem").filter({ hasText: "event log broken" })).toContainText(
    `event log broken at line ${fx.tamperedLine} (seq ${fx.tamperedSeq})`,
  );
  await expect(page.getByTestId("read-only")).toBeVisible();
  await expect(page.getByTestId("save-bundle")).toBeDisabled();
  await expect(page.getByTestId("clone-current")).toHaveCount(0);
  // It was not added to the library.
  await expect(page.getByTestId("library-entry")).toHaveCount(0);
  // The log lists the events before the edit, then where verification stopped.
  await page.getByTestId("log-toggle").click();
  await expect(page.getByTestId("log-row")).toHaveCount(fx.tamperedLine - 1);
  await expect(page.getByTestId("log-break")).toContainText(`Line ${fx.tamperedLine} (seq ${fx.tamperedSeq})`);
  await expect(page.getByTestId("stack-partial")).toContainText(`before line ${fx.tamperedLine}`);
});

test("7. a bundle made with the command line shows its stack, rendered bit for bit", async ({ page }) => {
  await start(page);
  await page.getByTestId("bundle-input").setInputFiles(fx.cliBundle);
  await expect(page.getByTestId("notice")).toContainText("Added");
  const steps = page.getByTestId("stack-step");
  await expect(steps).toHaveCount(4);
  expect(await steps.evaluateAll((els) => els.map((e) => e.getAttribute("data-op")))).toEqual(fx.cliSteps);
  await expect(steps.nth(1)).toContainText("quietest");
  const s = await nlae(page);
  expect(s.project.id).toBe(fx.cliProjectId);
  expect(s.project.stackHash).toBe(fx.cliStackHash);
  await expect(page.getByTestId("integrity")).toHaveAttribute("data-ok", "true");

  // The browser's render of the stack is the one the command line recorded, sample for sample.
  await page.getByTestId("monitor-stack").click();
  await page.getByTestId("compute-render-hash").click();
  await expect(page.getByTestId("render-hash")).toHaveAttribute("title", fx.cliOutputHash, { timeout: 60_000 });

  // The residual monitor draws what the stack removed.
  await page.getByTestId("monitor-residual").click();
  await expect.poll(async () => (await nlae(page)).view.monitor).toBe("residual");
  await lanesDrawn(page);
  await expect(page.getByTestId("spectrogram-busy")).toHaveCount(0);
});

test("8. recording from the microphone logs the settings the browser applied", async ({ page }) => {
  await start(page);
  await page.getByTestId("record").click();
  await expect(page.getByTestId("stop-recording")).toBeVisible();
  await page.waitForTimeout(1500);
  await page.getByTestId("stop-recording").click();
  await expect(page.getByTestId("capture")).toBeVisible();
  for (const k of ["echoCancellation", "noiseSuppression", "autoGainControl"]) {
    const cells = page.getByTestId(`capture-${k}`).locator("td");
    await expect(cells.nth(1)).toHaveText("false");
    await expect(cells.nth(2)).toHaveText("false");
  }
  const s = await nlae(page);
  expect(s.project.ok).toBe(true);
  expect(s.project.name).toMatch(/^recording-/);
  await page.getByTestId("log-toggle").click();
  await expect(page.getByTestId("log")).toContainText("source.recorded");
  // The fake microphone plays short beeps with digital silence between them.
  await lanesDrawn(page, { columns: 0.02, cells: 0.01 });
});

test("the browser core computes bit-identically to the native build", async ({ page }) => {
  test.setTimeout(180_000);
  await start(page);
  await page.getByTestId("parity").click();
  await expect(page.getByTestId("parity-result")).toContainText("bit-identical", { timeout: 170_000 });
});
