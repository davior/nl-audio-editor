// Labels end to end: a one-line note on a selected stretch or area, added with a right-click (or
// the ＋ Label button), listed beside the stack, never drawn on the lanes, selecting its place
// again when clicked, changed and deleted from the list, and kept when the project is reopened.
import { expect, test, type Page } from "@playwright/test";
import { drag, fx, importClip, logged, nlae, start, storedEvents, viewSaved } from "./helpers";

/** What a lane has drawn, to compare. */
const pixels = (page: Page, lane: "waveform" | "spectrogram") =>
  page.getByTestId(lane).evaluate((c) => (c as HTMLCanvasElement).toDataURL());

const clearSelection = (page: Page) => page.locator(".selections").getByRole("button", { name: "clear" }).click();

const where = (t0: number, t1: number) => `${t0.toFixed(3)}–${t1.toFixed(3)} s`;

test("a stretch is labelled with a right-click: listed beside the stack, drawn nowhere, and selected again by a click", async ({
  page,
}) => {
  await start(page);
  await importClip(page);
  const waveform = page.getByTestId("waveform");
  const box = (await waveform.boundingBox())!;

  // With nothing selected a right-click says what to do. It selects nothing and seeks nowhere.
  await waveform.click({ button: "right", position: { x: box.width * 0.5, y: 70 } });
  await expect(page.getByTestId("label-popover-hint")).toContainText("select a stretch");
  await expect(page.getByTestId("label-input")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("label-popover")).toHaveCount(0);
  expect((await nlae(page)).playhead).toBe(0);
  expect((await nlae(page)).view.selection).toBeNull();
  await expect(page.getByTestId("label-range")).toHaveCount(0);

  // 12 s clip: select 4–6 s, and keep what the lanes look like with nothing selected.
  await drag(page, "waveform", [4 / 12, 0.5], [6 / 12, 0.5]);
  await expect.poll(async () => (await nlae(page)).view.selection).not.toBeNull();
  await clearSelection(page);
  await expect.poll(async () => (await nlae(page)).view.selection).toBeNull();
  const bare = { waveform: await pixels(page, "waveform"), spectrogram: await pixels(page, "spectrogram") };

  await drag(page, "waveform", [4 / 12, 0.5], [6 / 12, 0.5]);
  await expect.poll(async () => (await nlae(page)).view.selection).not.toBeNull();
  const sel = (await nlae(page)).view.selection!;
  await expect(page.getByTestId("labels-empty")).toBeVisible();

  // The right-click opens the form for that stretch, with the cursor in the box.
  await waveform.click({ button: "right", position: { x: box.width * 0.5, y: 70 } });
  await expect(page.getByTestId("label-popover-place")).toHaveText(`Label ${where(sel.t0, sel.t1)}`);
  await expect(page.getByTestId("label-input")).toBeFocused();
  await expect(page.getByTestId("label-add")).toBeDisabled();
  expect((await nlae(page)).view.selection).toEqual(sel);
  expect((await nlae(page)).playhead).toBe(0);
  await page.getByTestId("label-input").fill("cough");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("label-popover")).toHaveCount(0);

  // It is in the list, with its place, and the stretch is still the selection.
  const rows = page.getByTestId("label-row");
  await expect(rows).toHaveCount(1);
  await expect(rows.first().getByTestId("label-text")).toHaveText("cough");
  await expect(rows.first().getByTestId("label-place")).toHaveText(where(sel.t0, sel.t1));
  await expect(rows.first()).toHaveClass(/selected/);
  await expect(page.getByTestId("label-note")).toContainText("cough");
  await expect(page.getByTestId("label-count")).toHaveText("1");

  // It is the user's action in the log, with the place exactly as selected.
  const [added] = await logged(page, "label.added");
  expect(added.actor).toMatchObject({ kind: "user" });
  expect(added.data).toMatchObject({ t0: sel.t0, t1: sel.t1, text: "cough" });
  expect(added.data).not.toHaveProperty("f_lo");

  // The list sits to the right of the stack, level with its top.
  const stack = (await page.getByTestId("console").boundingBox())!;
  const list = (await page.getByTestId("labels").boundingBox())!;
  expect(list.x).toBeGreaterThanOrEqual(stack.x + stack.width);
  expect(Math.abs(list.y - stack.y)).toBeLessThan(2);

  // Nothing stays on the clip: with the selection cleared the lanes look as they did before.
  await clearSelection(page);
  await expect.poll(async () => (await nlae(page)).view.selection).toBeNull();
  await expect(rows.first()).not.toHaveClass(/selected/);
  expect(await pixels(page, "waveform")).toBe(bare.waveform);
  expect(await pixels(page, "spectrogram")).toBe(bare.spectrogram);
  await expect(page.getByTestId("edit-removed")).toHaveCount(0);

  // Clicking the row selects the same stretch again, exactly.
  await rows.first().getByTestId("label-select").click();
  await expect.poll(async () => (await nlae(page)).view.selection).toEqual(sel);
  await expect(rows.first()).toHaveClass(/selected/);
  await expect(page.getByTestId("selection")).toContainText(where(sel.t0, sel.t1));

  // Zoomed in on somewhere else, the click brings the stretch into view.
  await clearSelection(page);
  for (let i = 0; i < 5; i++) await page.getByTestId("zoom-in").click();
  await expect.poll(async () => (await nlae(page)).view.t1 - (await nlae(page)).view.t0).toBeLessThan(1);
  await rows.first().getByTestId("label-select").click();
  await expect.poll(async () => (await nlae(page)).view.selection).toEqual(sel);
  const v = (await nlae(page)).view;
  expect(v.t0).toBeLessThanOrEqual(sel.t0);
  expect(v.t1).toBeGreaterThanOrEqual(sel.t1);
});

test("an area is labelled from the spectrogram; the text is changed, the label deleted, and it is kept when reopened", async ({ page }) => {
  await start(page);
  await importClip(page);
  // Linear scale up to 8 kHz on a 12 s recording.
  await drag(page, "spectrogram", [0.25, 0.9], [0.5, 0.8]);
  await expect.poll(async () => (await nlae(page)).view.tfSelection).not.toBeNull();
  const tf = (await nlae(page)).view.tfSelection!;

  // The button beside the area's readout opens the same form.
  await page.getByTestId("label-area").click();
  await expect(page.getByTestId("label-popover-place")).toHaveText(`Label ${where(tf.t0, tf.t1)} × ${tf.f_lo}–${tf.f_hi} Hz`);
  await page.getByTestId("label-input").fill("  whine  ");
  await page.getByTestId("label-add").click();

  const rows = page.getByTestId("label-row");
  await expect(rows).toHaveCount(1);
  await expect(rows.first().getByTestId("label-text")).toHaveText("whine");
  await expect(rows.first().getByTestId("label-place")).toHaveText(`${where(tf.t0, tf.t1)} × ${tf.f_lo}–${tf.f_hi} Hz`);
  const [added] = await logged(page, "label.added");
  expect(added.data).toMatchObject({ t0: tf.t0, t1: tf.t1, f_lo: tf.f_lo, f_hi: tf.f_hi, text: "whine" });

  // Changing the text: Escape drops it, Enter keeps it, and the log keeps what it replaced.
  await rows.first().getByTestId("label-edit").click();
  const edit = page.getByTestId("label-edit-input");
  await expect(edit).toBeFocused();
  await expect(edit).toHaveValue("whine");
  await edit.fill("something else");
  await edit.press("Escape");
  await expect(edit).toHaveCount(0);
  await expect(rows.first().getByTestId("label-text")).toHaveText("whine");
  await rows.first().getByTestId("label-edit").click();
  await edit.fill("whine, steady");
  await edit.press("Enter");
  await expect(rows.first().getByTestId("label-text")).toHaveText("whine, steady");
  const [edited] = await logged(page, "label.edited");
  expect(edited.data).toMatchObject({ label_id: added.data.label_id, text: "whine, steady", before: "whine" });
  // Its place stays where it was.
  await expect(rows.first().getByTestId("label-place")).toHaveText(`${where(tf.t0, tf.t1)} × ${tf.f_lo}–${tf.f_hi} Hz`);

  // Reopened, the list is the same, and still shows which place is selected.
  await viewSaved(page);
  const before = (await nlae(page)).project.events;
  await page.reload();
  await page.waitForFunction(() => window.__nlae?.ready === true);
  await expect(page.getByTestId("editor")).toBeVisible();
  await expect.poll(async () => (await nlae(page)).project.events).toBe(before);
  await expect(rows).toHaveCount(1);
  await expect(rows.first().getByTestId("label-text")).toHaveText("whine, steady");
  await expect(rows.first()).toHaveClass(/selected/);
  await expect(page.getByTestId("integrity")).toHaveAttribute("data-ok", "true");

  // Clear the area, and the click brings back that area and no time range.
  await clearSelection(page);
  await expect.poll(async () => (await nlae(page)).view.tfSelection).toBeNull();
  await rows.first().getByTestId("label-select").click();
  await expect.poll(async () => (await nlae(page)).view.tfSelection).toEqual(tf);
  expect((await nlae(page)).view.selection).toBeNull();

  // Deleting takes it off the list; its words stay in the log.
  await rows.first().getByTestId("label-delete").click();
  await expect(rows).toHaveCount(0);
  await expect(page.getByTestId("labels-empty")).toBeVisible();
  const [removed] = await logged(page, "label.removed");
  expect(removed.data).toEqual({ label_id: added.data.label_id });
  const kinds = (await storedEvents(page)).map((e) => e.type).filter((t) => t.startsWith("label."));
  expect(kinds).toEqual(["label.added", "label.edited", "label.removed"]);
});

test("labels are listed along the recording whatever order they were made in; a long list scrolls to the one just added", async ({
  page,
}) => {
  await start(page);
  await importClip(page);
  const rows = page.getByTestId("label-row");
  const list = page.getByTestId("labels-list");
  const add = async (from: number, to: number, text: string) => {
    await drag(page, "waveform", [from / 12, 0.5], [to / 12, 0.5]);
    await page.getByTestId("label-range").click();
    await page.getByTestId("label-input").fill(text);
    await page.keyboard.press("Enter");
    await expect(rows.filter({ hasText: text })).toHaveCount(1);
  };
  // The row is inside the list's own window, not scrolled out of it.
  const shown = async (text: string) => {
    const l = (await list.boundingBox())!;
    const r = (await rows.filter({ hasText: text }).boundingBox())!;
    return r.y >= l.y - 1 && r.y + r.height <= l.y + l.height + 1;
  };
  const scrollTop = () => list.evaluate((el) => el.scrollTop);

  await add(8, 9, "late");
  await add(1, 2, "early");
  await add(4, 5, "middle");
  expect(await page.getByTestId("label-text").allTextContents()).toEqual(["early", "middle", "late"]);

  // Many more, each later than the last, so each lands at the foot of a list that outgrows its box.
  for (let i = 0; i < 17; i++) await add(8.6 + 0.15 * i, 8.7 + 0.15 * i, `bit ${i}`);
  await expect(page.getByTestId("label-count")).toHaveText("20");
  expect(await list.evaluate((el) => el.scrollHeight - el.clientHeight)).toBeGreaterThan(100);
  expect(await scrollTop()).toBeGreaterThan(100);
  expect(await shown("bit 16")).toBe(true);

  // Taking another label off the list does not drag it back to the one added last.
  await list.evaluate((el) => (el.scrollTop = 0));
  await rows.filter({ hasText: "early" }).getByTestId("label-delete").click();
  await expect(rows.filter({ hasText: "early" })).toHaveCount(0);
  await expect(page.getByTestId("label-count")).toHaveText("19");
  expect(await scrollTop()).toBe(0);

  // A label that belongs at the top brings the list back there.
  await list.evaluate((el) => (el.scrollTop = el.scrollHeight));
  await add(0.5, 0.6, "first");
  await expect.poll(scrollTop).toBe(0);
  expect(await shown("first")).toBe(true);
  expect(await page.getByTestId("label-text").first().textContent()).toBe("first");
});

test("a project that did not verify can be selected from but takes no labels", async ({ page }) => {
  await start(page);
  await page.getByTestId("bundle-input").setInputFiles(fx.tamperedBundle);
  await expect(page.getByTestId("read-only")).toBeVisible();
  await drag(page, "waveform", [0.3, 0.5], [0.5, 0.5]);
  await expect.poll(async () => (await nlae(page)).view.selection).not.toBeNull();

  // The form does not offer a box, and says why.
  await page.getByTestId("label-range").click();
  await expect(page.getByTestId("label-popover-hint")).toContainText("did not verify");
  await expect(page.getByTestId("label-input")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.getByTestId("waveform").click({ button: "right", position: { x: 300, y: 70 } });
  await expect(page.getByTestId("label-popover-hint")).toContainText("did not verify");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("label-row")).toHaveCount(0);
});
