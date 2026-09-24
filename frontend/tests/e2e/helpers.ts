// Shared by the end-to-end specs: the state the app publishes for tests, and
// the steps most tests begin with.
import { expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { ViewState } from "../../src/core/types";
import type { Fixtures } from "./global-setup";
import { BASE_URL, FIX } from "./paths";

export interface Nlae {
  ready?: boolean;
  project: {
    id: string;
    name: string;
    sourceSha: string;
    stackHash: string;
    steps: number;
    events: number;
    readOnly: string | null;
    ok: boolean;
    parent: string | null;
    problems: string[];
    lineageVerified: boolean[];
  };
  view: ViewState;
  viewSaved: ViewState;
  playing: boolean;
  playhead: number;
  waveform: { columns: number; columnsWithSignal: number; which: string };
  spectrogram: { columns: number; rows: number; litCells: number; which: string; complete: boolean };
  analysis: "running" | "done" | "failed" | "not needed";
  saving: number;
}

export const fx: Fixtures = JSON.parse(readFileSync(join(FIX, "fixtures.json"), "utf8"));

export const nlae = (page: Page) => page.evaluate(() => window.__nlae as unknown as Nlae);

export async function start(page: Page) {
  await page.goto(BASE_URL);
  await page.waitForFunction(() => window.__nlae?.ready === true);
}

/**
 * Both lanes have drawn what the monitor selects: enough waveform columns carry signal and enough
 * spectrogram cells are lit.
 */
export async function lanesDrawn(page: Page, share = { columns: 0.5, cells: 0.2 }) {
  await page.waitForFunction((share) => {
    const s = window.__nlae as unknown as Nlae | undefined;
    return (
      !!s?.waveform &&
      !!s.spectrogram &&
      s.waveform.which === s.view.monitor &&
      s.spectrogram.which === s.view.monitor &&
      s.spectrogram.complete &&
      s.waveform.columnsWithSignal > share.columns * s.waveform.columns &&
      s.spectrogram.litCells > share.cells * s.spectrogram.columns * s.spectrogram.rows
    );
  }, share);
}

export async function importClip(page: Page) {
  await page.getByTestId("import-input").setInputFiles(fx.wav);
  await expect(page.getByTestId("editor")).toBeVisible();
  await lanesDrawn(page);
  // The analysis follows the lanes, and the library copy is written in the
  // background; wait for both so event counts and the library are settled.
  await expect.poll(async () => (await nlae(page)).analysis).toBe("done");
  await expect.poll(async () => (await nlae(page)).saving).toBe(0);
}

/** Drag across a lane between two points given as fractions of its size. */
export async function drag(page: Page, lane: "waveform" | "spectrogram", from: [number, number], to: [number, number]) {
  const box = (await page.getByTestId(lane).boundingBox())!;
  await page.mouse.move(box.x + box.width * from[0], box.y + box.height * from[1]);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * to[0], box.y + box.height * to[1], { steps: 8 });
  await page.mouse.up();
}

export async function viewSaved(page: Page) {
  await page.waitForFunction(() => {
    const s = window.__nlae as unknown as Nlae | undefined;
    return !!s && JSON.stringify(s.view) === JSON.stringify(s.viewSaved);
  });
}
