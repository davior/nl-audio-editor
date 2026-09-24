import { defineConfig } from "@playwright/test";
import { BASE_URL } from "./tests/e2e/paths";

// End-to-end tests against the production build (`pnpm build` first).
// Fixtures are made by the command-line tool in the global setup.

export default defineConfig({
  testDir: "tests/e2e",
  globalSetup: "./tests/e2e/global-setup.ts",
  timeout: 60_000,
  expect: { timeout: 20_000 },
  fullyParallel: true,
  workers: process.env.CI ? 2 : undefined,
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : [["list"]],
  use: {
    browserName: "chromium",
    baseURL: BASE_URL,
    viewport: { width: 1400, height: 1000 },
    acceptDownloads: true,
    permissions: ["microphone"],
    launchOptions: {
      args: ["--autoplay-policy=no-user-gesture-required", "--use-fake-ui-for-media-stream", "--use-fake-device-for-media-stream"],
    },
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "pnpm preview",
    url: BASE_URL,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
