// M1 end to end: typed requests in the console, applied at once; changing,
// removing and restoring them in the stack; export, undo and redo. The model is
// the stand-in the global setup starts, so no network or key is needed.
import { expect, test, type Locator, type Page } from "@playwright/test";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { drag, fx, importClip, lanesDrawn, logged, nlae, say, start, whereStored, type Event } from "./helpers";
import { MOCK_URL, seenByMock } from "./mock-model";

const KEY = "sk-nlae-e2e-7c1f9a2b40d6";

/** Point the console at the stand-in provider, through the settings dialog. */
async function useMockModel(page: Page, key = KEY) {
  await page.getByTestId("assistant-settings").click();
  await page.getByTestId("settings-preset").selectOption("custom");
  await page.getByTestId("settings-url").fill(MOCK_URL);
  await page.getByTestId("settings-model").fill("mock-1");
  await page.getByTestId("settings-provider").fill("mock");
  await page.getByTestId("settings-key").fill(key);
  await page.getByTestId("settings-save").click();
  await expect(page.getByTestId("assistant-settings")).toHaveText("mock · mock-1");
}

const opsOf = (steps: Locator) => steps.evaluateAll((els) => els.map((e) => e.getAttribute("data-op")));

test("1. “clean this recording up” applies the reference plan at once, and exports what the command line renders", async ({ page }) => {
  await start(page);
  await importClip(page);
  const turn = await say(page, "clean this recording up");
  await expect(turn).toHaveAttribute("data-status", "applied");
  await expect(turn).toHaveAttribute("data-via", "local");
  expect(await opsOf(turn.getByTestId("applied-step"))).toEqual(fx.cliSteps);
  await expect(page.getByTestId("stack-step")).toHaveCount(4);
  // The same stack as the command line's for the same recording and recipe.
  await expect.poll(async () => (await nlae(page)).project.stackHash).toBe(fx.cliStackHash);
  // The monitor plays the result, over the whole recording.
  await expect.poll(async () => (await nlae(page)).view.monitor).toBe("stack");
  await lanesDrawn(page, { columns: 0.2, cells: 0.02 });

  // Asking to listen switches the monitor.
  const listen = await say(page, "play the residual");
  await expect(listen).toHaveAttribute("data-via", "local");
  await expect.poll(async () => (await nlae(page)).view.monitor).toBe("residual");

  const [replayed] = await logged(page, "recipe.replayed");
  expect(replayed.data.recipe).toBe("spoken-word-cleanup");
  const [applied] = await logged(page, "step.applied");
  expect(applied.data.kind).toBe("plan");
  const steps = applied.data.steps as { intent: string }[];
  expect(steps.map((s) => s.intent)).toEqual(Array(4).fill("clean this recording up"));
  await logged(page, "step.previewed", 0);
  await expect(page.getByTestId("approval-note")).toContainText("4 steps");

  // Export: the file is the command line's render, and its hash is logged.
  await page.getByTestId("export-format").selectOption("f32");
  const [download] = await Promise.all([page.waitForEvent("download"), page.getByTestId("export-wav").click()]);
  expect(download.suggestedFilename()).toBe("golden_a_short.wav");
  const sha = `sha256:${createHash("sha256")
    .update(readFileSync(await download.path()))
    .digest("hex")}`;
  expect(sha).toBe(fx.cliRender.file_sha256);
  const [exported] = await logged(page, "render.exported");
  const { stack_hash, output_hash, file_sha256, limiter } = fx.cliRender;
  expect(exported.data).toMatchObject({ format: "f32", file: "golden_a_short.wav", stack_hash, output_hash, file_sha256, limiter });
});

test("2. “the hum is distracting” goes to the model and is applied; a value is changed in the stack, and all of it is logged", async ({
  page,
}) => {
  await start(page);
  await importClip(page);
  await useMockModel(page);
  const turn = await say(page, "the hum is distracting");
  await expect(turn).toHaveAttribute("data-status", "applied");
  await expect(turn).toHaveAttribute("data-via", "model");
  await expect(turn.getByTestId("turn-text")).toContainText("50 Hz hum");
  const step = turn.getByTestId("applied-step");
  await expect(step).toHaveAttribute("data-op", "line_reduce");
  await expect(step.getByTestId("measurements")).not.toBeEmpty();
  await expect(page.getByTestId("stack-step")).toHaveCount(1);
  await expect(page.getByTestId("step-meta")).toContainText("mock-1 (mock) via console — “the hum is distracting”");

  // Change a value from the card: the stack's editor opens on the step.
  await step.getByTestId("edit-applied").click();
  const editor = page.getByTestId("step-editor");
  await editor.getByTestId("param-max_depth_db").fill("30");
  await editor.getByTestId("apply-edit").click();
  const [edited] = await logged(page, "step.edited");
  expect((edited.data.to_params as Record<string, unknown>).max_depth_db).toBe(30);
  expect(edited.actor).toMatchObject({ kind: "user" });
  await expect(page.getByTestId("stack-step")).toContainText("edited");

  // The exchange holds exactly what the provider was sent, and the applied step points to it.
  const [ex] = await logged(page, "assistant.exchange");
  expect(ex.actor).toMatchObject({ kind: "assistant", model: "mock-1", provider: "mock" });
  expect(ex.data).toMatchObject({ provider: "mock", model: "mock-1", host: "127.0.0.1:4180", prompt_version: 2 });
  const sent = (await seenByMock()).filter((s) => s.words === "the hum is distracting");
  expect(sent.length).toBeGreaterThan(0);
  expect(sent.map((s) => JSON.parse(s.body))).toContainEqual(ex.data.request);
  const [applied] = await logged(page, "step.applied");
  expect(applied.data.exchange).toBe(ex.hash);
  // The person applied it; the model proposed it.
  expect(applied.actor).toMatchObject({ kind: "user" });
  const step0 = (applied.data.steps as { intent: string; rationale: string; actor: Record<string, unknown> }[])[0];
  expect(step0.actor).toMatchObject({ kind: "assistant", model: "mock-1", provider: "mock" });
  expect(step0.intent).toBe("the hum is distracting");
  expect(step0.rationale).toContain("50 Hz hum");
});

test("3. a removal is logged with its reason; the step keeps its place and can be restored", async ({ page }) => {
  await start(page);
  await importClip(page);
  const before = (await nlae(page)).project.stackHash;
  const turn = await say(page, "cut 3,100 to 3,200 Hz by 12 dB");
  await expect(turn).toHaveAttribute("data-via", "local");
  await expect(turn.getByTestId("applied-step")).toHaveAttribute("data-op", "band_cut");
  await expect(turn.getByTestId("applied-step")).toContainText("3100–3200 Hz");
  await expect(page.getByTestId("stack-step")).toHaveCount(1);
  const after = (await nlae(page)).project.stackHash;

  await page.getByTestId("remove-step").click();
  await page.getByTestId("remove-reason").fill("it takes too much of the voice");
  await page.getByTestId("remove-confirm").click();
  const [excluded] = await logged(page, "step.excluded");
  expect(excluded.data.reason).toBe("it takes too much of the voice");
  await expect(page.getByTestId("stack-step")).toHaveCount(0);
  await expect(page.getByTestId("stack-step-removed")).toContainText("it takes too much of the voice");
  await expect(turn.getByTestId("applied-step")).toContainText("removed");
  await expect.poll(async () => (await nlae(page)).project.stackHash).toBe(before);

  // Restored to its place, with the values it had.
  await page.getByTestId("restore-step").click();
  await logged(page, "step.restored");
  await expect(page.getByTestId("stack-step")).toHaveCount(1);
  await expect.poll(async () => (await nlae(page)).project.stackHash).toBe(after);
});

test("4. an area on the spectrogram and “compress the peaks here” give a spectral compressor on that area", async ({ page }) => {
  await start(page);
  await importClip(page);
  await drag(page, "spectrogram", [0.25, 0.9], [0.5, 0.8]);
  await expect.poll(async () => (await nlae(page)).view.tfSelection).not.toBeNull();
  const area = (await nlae(page)).view.tfSelection!;
  const turn = await say(page, "compress the peaks here");
  await expect(turn).toHaveAttribute("data-via", "local");
  const step = turn.getByTestId("applied-step");
  await expect(step).toHaveAttribute("data-op", "spectral_compressor");
  await expect(step.getByTestId("measurements")).not.toBeEmpty();
  const [applied] = await logged(page, "step.applied");
  const scope = (applied.data.steps as { scope: Record<string, number | string> }[])[0].scope;
  expect(scope).toEqual({ kind: "tf_patch", t0: area.t0, t1: area.t1, f_lo: area.f_lo, f_hi: area.f_hi });
});

test("5. a model that proposes an impossible value is asked once to correct itself; a question is answered in words", async ({ page }) => {
  await start(page);
  await importClip(page);
  await useMockModel(page);
  const turn = await say(page, "make it much louder");
  await expect(turn).toHaveAttribute("data-status", "applied");
  await expect(turn.getByTestId("applied-step")).toHaveAttribute("data-op", "gain");
  await expect(turn.getByTestId("applied-step")).toContainText("6.0 dB");
  const [first, second] = await logged(page, "assistant.exchange", 2);
  expect(JSON.stringify(first.data.problems)).toContain("gain_db");
  expect(second.data.corrects).toBe(first.hash);
  expect(second.data.problems ?? null).toBeNull();

  // A question is answered in words; the stack is left as it is.
  const question = await say(page, "who is speaking loudest?");
  await expect(question).toHaveAttribute("data-status", "reply");
  await expect(question.getByTestId("turn-text")).toContainText("Which part of the recording");
  await logged(page, "assistant.exchange", 3);
  await expect(page.getByTestId("stack-step")).toHaveCount(1);
});

test("6. the model is sent the selection, never audio, and the key is never stored", async ({ page }) => {
  await start(page);
  await importClip(page);
  await useMockModel(page);
  await page.getByTestId("assistant-settings").click();
  await page.getByTestId("settings-test").click();
  await expect(page.getByTestId("settings-test-result")).toContainText("127.0.0.1:4180 answered");
  await page.getByRole("button", { name: "Cancel" }).click();

  await drag(page, "spectrogram", [0.4, 0.95], [0.6, 0.7]);
  await expect.poll(async () => (await nlae(page)).view.tfSelection).not.toBeNull();
  const area = (await nlae(page)).view.tfSelection!;
  const turn = await say(page, "the knocking in the selection is too much");
  await expect(turn).toHaveAttribute("data-via", "model");
  await expect(turn).toHaveAttribute("data-status", "applied");
  const [applied] = await logged(page, "step.applied");
  const scope = (applied.data.steps as { scope: Record<string, number | string> }[])[0].scope;
  expect(scope).toEqual({ kind: "tf_patch", t0: area.t0, t1: area.t1, f_lo: area.f_lo, f_hi: area.f_hi });

  // What reached the provider: the key in the header, words and numbers in the body.
  const sent = (await seenByMock()).filter((s) => s.words === "the knocking in the selection is too much");
  expect(sent.length).toBeGreaterThan(0);
  for (const s of sent) {
    expect(s.authorization).toBe(`Bearer ${KEY}`);
    expect(s.body).not.toContain(KEY);
    const longest = (v: unknown): number =>
      Array.isArray(v)
        ? Math.max(v.every((x) => typeof x === "number") ? v.length : 0, ...v.map(longest))
        : v && typeof v === "object"
          ? Math.max(0, ...Object.values(v).map(longest))
          : 0;
    expect(longest(JSON.parse(s.body))).toBeLessThanOrEqual(64);
  }

  // Nowhere the app keeps things: the library, browser storage, or a bundle.
  const stored = await whereStored(page, KEY);
  expect(stored.files).toBeGreaterThan(3);
  expect(stored.found).toEqual([]);
  const [download] = await Promise.all([page.waitForEvent("download"), page.getByTestId("save-bundle").click()]);
  expect(readFileSync(await download.path()).includes(Buffer.from(KEY))).toBe(false);
});

test("7. undo takes back the last change, from the stack panel or in words; redo repeats it; a rating is recorded", async ({ page }) => {
  await start(page);
  await importClip(page);
  const empty = (await nlae(page)).project.stackHash;
  for (const how of ["button", "words"]) {
    const turn = await say(page, "turn it up by 3 dB");
    await expect(turn.getByTestId("applied-step")).toHaveAttribute("data-op", "gain");
    await expect(page.getByTestId("stack-step")).toHaveCount(1);
    if (how === "button") {
      await page.getByTestId("rate-4").click();
      const [rated] = await logged(page, "stack.rated");
      expect(rated.data).toMatchObject({ overall: 4, target_kind: "stack" });
      await page.getByTestId("undo").click();
    } else {
      const undo = await say(page, "undo");
      await expect(undo).toHaveAttribute("data-via", "local");
      await expect(undo.getByTestId("turn-text")).toContainText("Undid applying gain");
    }
    await expect(page.getByTestId("stack-step")).toHaveCount(0);
    await expect.poll(async () => (await nlae(page)).project.stackHash).toBe(empty);
  }
  // Each undo excludes what it takes back, naming the change; the steps keep their places.
  const undone = await logged(page, "step.excluded", 2);
  expect(undone.every((e) => typeof e.data.undoes === "string")).toBe(true);
  await expect(page.getByTestId("stack-step-removed")).toHaveCount(2);

  // Redo, in words, brings back what was last taken back.
  const redo = await say(page, "redo");
  await expect(redo).toHaveAttribute("data-via", "local");
  await expect(page.getByTestId("stack-step")).toHaveCount(1);
  const [redone] = await logged(page, "step.restored");
  expect(typeof redone.data.redoes).toBe("string");
});

test("8. an operation the model sees only in the index is described when it asks, then used", async ({ page }) => {
  await start(page);
  await importClip(page);
  await useMockModel(page);
  const turn = await say(page, "make it brighter");
  await expect(turn).toHaveAttribute("data-status", "applied");
  await expect(turn).toHaveAttribute("data-via", "model");
  await expect(turn.getByTestId("applied-step")).toHaveAttribute("data-op", "tilt");

  // Two exchanges: the first offers tilt only in the index; the second describes it and offers it.
  const exchanges = await logged(page, "assistant.exchange", 2);
  const tools = (e: Event) => (e.data.request as { tools: { function: { name: string } }[] }).tools.map((t) => t.function.name);
  expect(tools(exchanges[0])).toContain("describe_operations");
  expect(tools(exchanges[0])).not.toContain("tilt");
  expect(tools(exchanges[1])).toContain("tilt");
  expect(exchanges[1].data.describes).toBe(exchanges[0].hash);
  expect(exchanges.map((e) => e.data.prompt_version)).toEqual([2, 2]);
  const [applied] = await logged(page, "step.applied");
  expect(applied.data.exchange).toBe(exchanges[1].hash);
  expect((applied.data.steps as unknown[])[0]).toMatchObject({
    op: "tilt",
    intent: "make it brighter",
    actor: { model: "mock-1", provider: "mock" },
  });
});
