// M1 end to end: typed requests in the console, previews, accept, change,
// reject, export and undo. The model is the stand-in the global setup starts,
// so no network or key is needed.
import { expect, test, type Locator, type Page } from "@playwright/test";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { drag, fx, importClip, lanesDrawn, logged, nlae, say, start, whereStored } from "./helpers";
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

test("1. “clean this recording up” previews the reference plan, is accepted, and exports what the command line renders", async ({
  page,
}) => {
  await start(page);
  await importClip(page);
  const turn = await say(page, "clean this recording up");
  await expect(turn).toHaveAttribute("data-status", "proposal");
  await expect(turn).toHaveAttribute("data-via", "local");
  expect(await opsOf(turn.getByTestId("proposal-step"))).toEqual(fx.cliSteps);

  // The lanes and monitors switch to the preview window.
  await expect.poll(async () => (await nlae(page)).view.monitor).toBe("preview:output");
  const pv = (await nlae(page)) as unknown as { preview: { window: [number, number] }; view: { t0: number; t1: number } };
  expect(pv.view.t0).toBeCloseTo(pv.preview.window[0], 6);
  expect(pv.view.t1).toBeCloseTo(pv.preview.window[1], 6);
  await expect(page.getByTestId("monitor-preview:before")).toBeVisible();
  await lanesDrawn(page, { columns: 0.2, cells: 0.02 });

  // Asking to listen switches monitor and keeps the proposal open.
  const listen = await say(page, "play the residual");
  await expect(listen).toHaveAttribute("data-via", "local");
  await expect.poll(async () => (await nlae(page)).view.monitor).toBe("preview:residual");
  await expect(turn).toHaveAttribute("data-status", "proposal");

  await turn.getByTestId("accept").click();
  await expect(turn.getByTestId("turn-outcome")).toHaveText("accepted");
  await expect(page.getByTestId("stack-step")).toHaveCount(4);
  // The same stack as the command line's for the same recording and recipe.
  await expect.poll(async () => (await nlae(page)).project.stackHash).toBe(fx.cliStackHash);
  await expect.poll(async () => (await nlae(page)).view.monitor).toBe("stack");
  const [replayed] = await logged(page, "recipe.replayed");
  expect(replayed.data.recipe).toBe("spoken-word-cleanup");
  const [plan] = await logged(page, "plan.accepted");
  const steps = plan.data.steps as { intent: string }[];
  expect(steps.map((s) => s.intent)).toEqual(Array(4).fill("clean this recording up"));

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

test("2. “the hum is distracting” goes to the model; a value is changed before accepting, and all of it is logged", async ({ page }) => {
  await start(page);
  await importClip(page);
  await useMockModel(page);
  const turn = await say(page, "the hum is distracting");
  await expect(turn).toHaveAttribute("data-status", "proposal");
  await expect(turn).toHaveAttribute("data-via", "model");
  await expect(turn.getByTestId("turn-text")).toContainText("50 Hz hum");
  const step = turn.getByTestId("proposal-step");
  await expect(step).toHaveAttribute("data-op", "line_reduce");
  await expect(step.getByTestId("measurements")).not.toBeEmpty();

  await step.getByTestId("modify").click();
  await step.getByTestId("param-max_depth_db").fill("30");
  await expect(step.getByTestId("changed")).toContainText("max_depth_db → 30");
  await turn.getByTestId("accept").click();
  await expect(turn.getByTestId("turn-outcome")).toHaveText("accepted");
  await expect(page.getByTestId("stack-step")).toHaveCount(1);
  await expect(page.getByTestId("step-meta")).toContainText("mock-1 (mock) via console — “the hum is distracting”");

  // The exchange holds exactly what the provider was sent, and the preview points to it.
  const [ex] = await logged(page, "assistant.exchange");
  expect(ex.actor).toMatchObject({ kind: "assistant", model: "mock-1", provider: "mock" });
  expect(ex.data).toMatchObject({ provider: "mock", model: "mock-1", host: "127.0.0.1:4180", prompt_version: 1 });
  const sent = (await seenByMock()).filter((s) => s.words === "the hum is distracting");
  expect(sent.length).toBeGreaterThan(0);
  expect(sent.map((s) => JSON.parse(s.body))).toContainEqual(ex.data.request);
  const [previewed] = await logged(page, "step.previewed");
  expect(previewed.data.exchange).toBe(ex.hash);

  const [modified] = await logged(page, "step.modified");
  expect((modified.data.to_params as Record<string, unknown>).max_depth_db).toBe(30);
  const [accepted] = await logged(page, "step.accepted");
  // The person accepted it; the model proposed it.
  expect(accepted.actor).toMatchObject({ kind: "user" });
  const step0 = accepted.data.step as {
    params: Record<string, unknown>;
    intent: string;
    rationale: string;
    actor: Record<string, unknown>;
  };
  expect(step0.actor).toMatchObject({ kind: "assistant", model: "mock-1", provider: "mock" });
  expect(step0.params.max_depth_db).toBe(30);
  expect(step0.intent).toBe("the hum is distracting");
  expect(step0.rationale).toContain("50 Hz hum");
});

test("3. a rejection is logged with its reason and leaves the stack as it was", async ({ page }) => {
  await start(page);
  await importClip(page);
  const before = (await nlae(page)).project.stackHash;
  const turn = await say(page, "cut 3,100 to 3,200 Hz by 12 dB");
  await expect(turn).toHaveAttribute("data-via", "local");
  await expect(turn.getByTestId("proposal-step")).toHaveAttribute("data-op", "band_cut");
  await expect(turn.getByTestId("proposal-step")).toContainText("3100–3200 Hz");
  await turn.getByTestId("reject").click();
  await turn.getByTestId("reject-reason").fill("it takes too much of the voice");
  await turn.getByTestId("reject-confirm").click();
  await expect(turn.getByTestId("turn-outcome")).toHaveText("rejected");
  const [rejected] = await logged(page, "step.rejected");
  expect(rejected.data.reason).toBe("it takes too much of the voice");
  await expect(page.getByTestId("stack-step")).toHaveCount(0);
  expect((await nlae(page)).project.stackHash).toBe(before);
  // The view returns to how it was before the preview.
  await expect.poll(async () => (await nlae(page)).view.monitor).toBe("source");
});

test("4. an area on the spectrogram and “compress the peaks here” give a spectral compressor on that area", async ({ page }) => {
  await start(page);
  await importClip(page);
  await drag(page, "spectrogram", [0.25, 0.9], [0.5, 0.8]);
  await expect.poll(async () => (await nlae(page)).view.tfSelection).not.toBeNull();
  const area = (await nlae(page)).view.tfSelection!;
  const turn = await say(page, "compress the peaks here");
  await expect(turn).toHaveAttribute("data-via", "local");
  const step = turn.getByTestId("proposal-step");
  await expect(step).toHaveAttribute("data-op", "spectral_compressor");
  await expect(step.getByTestId("measurements")).not.toBeEmpty();
  await turn.getByTestId("accept").click();
  await expect(turn.getByTestId("turn-outcome")).toHaveText("accepted");
  const [accepted] = await logged(page, "step.accepted");
  const scope = (accepted.data.step as { scope: Record<string, number | string> }).scope;
  expect(scope).toEqual({ kind: "tf_patch", t0: area.t0, t1: area.t1, f_lo: area.f_lo, f_hi: area.f_hi });
});

test("5. a model that proposes an impossible value is asked once to correct itself; a question is answered in words", async ({ page }) => {
  await start(page);
  await importClip(page);
  await useMockModel(page);
  const turn = await say(page, "make it much louder");
  await expect(turn).toHaveAttribute("data-status", "proposal");
  await expect(turn.getByTestId("proposal-step")).toHaveAttribute("data-op", "gain");
  await expect(turn.getByTestId("proposal-step")).toContainText("6.0 dB");
  const [first, second] = await logged(page, "assistant.exchange", 2);
  expect(JSON.stringify(first.data.problems)).toContain("gain_db");
  expect(second.data.corrects).toBe(first.hash);
  expect(second.data.problems ?? null).toBeNull();

  // Anything else sets the open proposal aside.
  const question = await say(page, "who is speaking loudest?");
  await expect(question).toHaveAttribute("data-status", "reply");
  await expect(question.getByTestId("turn-text")).toContainText("Which part of the recording");
  await expect(turn.getByTestId("turn-outcome")).toHaveText("set aside");
  await logged(page, "assistant.exchange", 3);
  await expect(page.getByTestId("stack-step")).toHaveCount(0);
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
  await turn.getByTestId("accept").click();
  await expect(turn.getByTestId("turn-outcome")).toHaveText("accepted");
  const [accepted] = await logged(page, "step.accepted");
  const scope = (accepted.data.step as { scope: Record<string, number | string> }).scope;
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

test("7. undo removes the top step, from the stack panel or in words, and a rating is recorded", async ({ page }) => {
  await start(page);
  await importClip(page);
  const empty = (await nlae(page)).project.stackHash;
  for (const how of ["button", "words"]) {
    const turn = await say(page, "turn it up by 3 dB");
    await expect(turn.getByTestId("proposal-step")).toHaveAttribute("data-op", "gain");
    await turn.getByTestId("accept").click();
    await expect(page.getByTestId("stack-step")).toHaveCount(1);
    if (how === "button") {
      await page.getByTestId("rate-4").click();
      const [rated] = await logged(page, "stack.rated");
      expect(rated.data).toMatchObject({ overall: 4, target_kind: "stack" });
      await page.getByTestId("undo").click();
    } else {
      const undo = await say(page, "undo");
      await expect(undo).toHaveAttribute("data-via", "local");
    }
    await expect(page.getByTestId("stack-step")).toHaveCount(0);
    await expect.poll(async () => (await nlae(page)).project.stackHash).toBe(empty);
  }
  await logged(page, "step.removed", 2);
});
