// Spoken requests end to end. The console streams Chromium's fake microphone
// to the stand-in recogniser the global setup starts, so no network or key is
// needed. The words fill the box as they are recognised, nothing is sent until
// Enter, and what was heard is logged next to what was sent.
import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { importClip, logged, nlae, say, start, whereStored } from "./helpers";
import { DG_KEY_PREFIX, DG_URL, handshakesSeen, scriptDeepgram, seenByDeepgram } from "./mock-deepgram";

/** A key of each test's own, so its script and streams are its own. */
const keyFor = (name: string) => `${DG_KEY_PREFIX}${name}-${Math.random().toString(36).slice(2, 10)}`;

/** Point dictation at the stand-in, through the settings dialog. */
async function useSpeech(page: Page, key: string, improve = true) {
  await page.getByTestId("assistant-settings").click();
  await page.getByTestId("speech-url").fill(DG_URL);
  await page.getByTestId("speech-key").fill(key);
  if (!improve) await page.getByTestId("speech-improve").uncheck();
  await page.getByTestId("settings-save").click();
  await expect(page.getByTestId("settings")).toHaveCount(0);
}

const mic = (page: Page) => page.getByTestId("console-mic");
const box = (page: Page) => page.getByTestId("console-input");

test("1. dictated words fill the box as they are recognised, are corrected and sent with Enter, and both are logged", async ({ page }) => {
  const key = keyFor("fill");
  await scriptDeepgram(key, { finals: ["Cut 3100 to 3200 hertz", "by 10 dB."], step: 1 });
  await start(page);
  await importClip(page);
  await useSpeech(page, key);

  // Playing when the microphone opens: playback pauses, and waits.
  await page.getByTestId("play").click();
  await expect.poll(async () => (await nlae(page)).playing).toBe(true);
  await mic(page).click();
  await expect(mic(page)).toHaveAttribute("data-state", "listening");
  await expect.poll(async () => (await nlae(page)).playing).toBe(false);
  await expect(page.getByTestId("play")).toBeDisabled();

  // Interim words first, replaced as they firm up; the speaker's pause ends listening.
  await expect(box(page)).toHaveValue("Cut 3100 to");
  await expect(box(page)).toHaveValue("Cut 3100 to 3200 hertz");
  await expect(box(page)).toHaveValue("Cut 3100 to 3200 hertz by 10");
  await expect(box(page)).toHaveValue("Cut 3100 to 3200 hertz by 10 dB.");
  await expect(mic(page)).toHaveAttribute("data-state", "off");
  await expect(page.getByTestId("play")).toBeEnabled();
  await expect(page.getByTestId("turn")).toHaveCount(0);

  // The user corrects what was misheard, then sends it.
  await box(page).fill("Cut 3100 to 3200 hertz by 12 dB.");
  await box(page).press("Enter");
  const turn = page.getByTestId("turn").first();
  await expect(turn).toHaveAttribute("data-status", "proposal");
  await expect(turn).toHaveAttribute("data-via", "local");
  await expect(turn).toHaveAttribute("data-spoken", "yes");
  await expect(turn.getByTestId("proposal-step")).toHaveAttribute("data-op", "band_cut");

  // What was heard and what was sent are logged first; the preview refers to them.
  const [spoken] = await logged(page, "speech.transcribed");
  expect(spoken.actor.kind).toBe("user");
  expect(spoken.data).toMatchObject({
    provider: "deepgram",
    model: "nova-3",
    host: "127.0.0.1:4181",
    heard: "Cut 3100 to 3200 hertz by 10 dB.",
    words: "Cut 3100 to 3200 hertz by 12 dB.",
    edited: true,
    segments: [
      { text: "Cut 3100 to 3200 hertz", confidence: 0.96 },
      { text: "by 10 dB.", confidence: 0.96 },
    ],
  });
  const [preview] = await logged(page, "step.previewed");
  expect(preview.data.dictation).toBe(spoken.hash);
  expect(preview.seq).toBeGreaterThan(spoken.seq);
  expect((preview.data.steps as { intent: string; params: Record<string, unknown> }[])[0]).toMatchObject({
    intent: "Cut 3100 to 3200 hertz by 12 dB.",
    params: { depth_db: 12 },
  });

  // What reached the recogniser: 16-bit audio at the stated rate, the key only in the subprotocol.
  const [stream] = await seenByDeepgram(key);
  expect(stream.protocols).toBe(`token, ${key}`);
  expect(stream.url).not.toContain(key);
  const q = new URL(stream.url, DG_URL).searchParams;
  expect([...q]).toEqual(spoken.data.params);
  expect(q.get("encoding")).toBe("linear16");
  expect(q.getAll("keyterm")).toContain("DC offset");
  expect(q.has("mip_opt_out")).toBe(false);
  expect(stream.oddFrames).toBe(0);
  expect(stream.nonZeroSamples).toBeGreaterThan(0);
  expect(stream.controls).toEqual(["CloseStream"]);
  expect(spoken.data.request_ids).toEqual([stream.requestId]);
  expect(spoken.data.audio_s as number).toBeCloseTo(stream.audioBytes / 2 / Number(q.get("sample_rate")), 2);

  // The key is nowhere the app keeps things.
  const stored = await whereStored(page, key);
  expect(stored.files).toBeGreaterThan(3);
  expect(stored.found).toEqual([]);
  const [download] = await Promise.all([page.waitForEvent("download"), page.getByTestId("save-bundle").click()]);
  expect(readFileSync(await download.path()).includes(Buffer.from(key))).toBe(false);
});

test("2. a dictated “undo” waits in the box until Enter", async ({ page }) => {
  const key = keyFor("undo");
  await scriptDeepgram(key, { finals: ["Undo."] });
  await start(page);
  await importClip(page);
  const cut = await say(page, "cut 3,100 to 3,200 Hz by 12 dB");
  await cut.getByTestId("accept").click();
  await expect(page.getByTestId("stack-step")).toHaveCount(1);
  await useSpeech(page, key);

  await mic(page).click();
  await expect(box(page)).toHaveValue("Undo.");
  await expect(mic(page)).toHaveAttribute("data-state", "off");
  // Heard, not acted on.
  await expect(page.getByTestId("turn")).toHaveCount(1);
  await expect(page.getByTestId("stack-step")).toHaveCount(1);

  await box(page).press("Enter");
  await expect(page.getByTestId("turn")).toHaveCount(2);
  await expect(page.getByTestId("turn").nth(1)).toHaveAttribute("data-spoken", "yes");
  await expect(page.getByTestId("stack-step")).toHaveCount(0);
  const [spoken] = await logged(page, "speech.transcribed");
  expect(spoken.data).toMatchObject({ heard: "Undo.", words: "Undo.", edited: false });
  const [removed] = await logged(page, "step.excluded");
  expect(removed.seq).toBeGreaterThan(spoken.seq);
});

test("3. while listening, Escape discards what was heard, and Enter only stops, leaving the words to check", async ({ page }) => {
  const key = keyFor("keys");
  await scriptDeepgram(key, { finals: ["play the residual"], step: 1, pause: false });
  await start(page);
  await importClip(page);
  await useSpeech(page, key);

  await box(page).fill("please");
  await mic(page).click();
  await expect(box(page)).toHaveValue("please play the");
  await box(page).press("Escape");
  await expect(mic(page)).toHaveAttribute("data-state", "off");
  await expect(box(page)).toHaveValue("please");

  await box(page).fill("");
  await mic(page).click();
  await expect(box(page)).toHaveValue("play the residual");
  await expect(mic(page)).toHaveAttribute("data-state", "listening");
  await box(page).press("Enter");
  await expect(mic(page)).toHaveAttribute("data-state", "off");
  await expect(box(page)).toHaveValue("play the residual");
  await expect(page.getByTestId("turn")).toHaveCount(0);

  await box(page).press("Enter");
  const turn = page.getByTestId("turn").first();
  await expect(turn).toHaveAttribute("data-status", "done");
  await expect(turn).toHaveAttribute("data-spoken", "yes");
  await expect.poll(async () => (await nlae(page)).view.monitor).toBe("residual");
  // Only what was sent is logged.
  const spoken = await logged(page, "speech.transcribed");
  expect(spoken[0].data).toMatchObject({ heard: "play the residual", edited: false });
  expect(await seenByDeepgram(key)).toHaveLength(2);
});

test("4. opting out of Deepgram's model improvement goes with the stream, and is remembered without the key", async ({ page }) => {
  const key = keyFor("optout");
  await scriptDeepgram(key, { finals: ["remove the hum"] });
  await start(page);
  await importClip(page);
  await useSpeech(page, key, false);

  await mic(page).click();
  await expect(box(page)).toHaveValue("remove the hum");
  await expect(mic(page)).toHaveAttribute("data-state", "off");
  const [stream] = await seenByDeepgram(key);
  expect(new URL(stream.url, DG_URL).searchParams.get("mip_opt_out")).toBe("true");
  const saved = JSON.parse((await page.evaluate(() => localStorage.getItem("nlae.speech")))!);
  expect(saved).toMatchObject({ optOut: true, key: "", remember: false, url: DG_URL });
});

test("5. without a key the settings open; Test connection reaches the recogniser; a wrong key is refused and the microphone closes", async ({
  page,
}) => {
  const key = keyFor("test");
  await start(page);
  await importClip(page);

  await mic(page).click();
  await expect(page.getByTestId("settings")).toBeVisible();
  await page.getByTestId("speech-url").fill(DG_URL);
  await page.getByTestId("speech-key").fill(key);
  await page.getByTestId("speech-test").click();
  await expect(page.getByTestId("speech-test-result")).toContainText("127.0.0.1:4181 accepted the stream");
  await expect(page.getByTestId("speech-test-result")).toContainText("(request req-");
  const [probe] = await seenByDeepgram(key);
  expect(probe.audioBytes).toBe(0);
  expect(probe.controls).toEqual(["CloseStream"]);

  await page.getByTestId("speech-key").fill("not-a-deepgram-key");
  await page.getByTestId("speech-test").click();
  await expect(page.getByTestId("speech-test-result")).toContainText("127.0.0.1:4181 refused the connection");
  await page.getByTestId("settings-save").click();

  await mic(page).click();
  await expect(page.getByTestId("mic-error")).toContainText("127.0.0.1:4181 refused the connection");
  await expect(mic(page)).toHaveAttribute("data-state", "off");
  await expect(page.getByTestId("play")).toBeEnabled();
  await expect(page.getByTestId("turn")).toHaveCount(0);
});

test("6. Escape while the stream is still opening abandons it: nothing is streamed, and the microphone stays off", async ({ page }) => {
  const key = keyFor("abandon");
  await scriptDeepgram(key, { finals: ["remove the hum"], openDelay: 1500 });
  await start(page);
  await importClip(page);
  await useSpeech(page, key);

  await box(page).fill("typed");
  await mic(page).click();
  await expect(mic(page)).toHaveAttribute("data-state", "starting");
  await expect.poll(() => handshakesSeen(key)).toBe(1);
  await box(page).press("Escape");
  await expect(mic(page)).toHaveAttribute("data-state", "off");
  await expect(box(page)).toHaveValue("typed");

  // Well after the stream would have opened: still off, and nothing reached the recogniser.
  await page.waitForTimeout(2500);
  await expect(mic(page)).toHaveAttribute("data-state", "off");
  await expect(box(page)).toHaveValue("typed");
  await expect(page.getByTestId("play")).toBeEnabled();
  const streams = await seenByDeepgram(key);
  expect(streams.reduce((n, s) => n + s.audioBytes, 0)).toBe(0);
});
