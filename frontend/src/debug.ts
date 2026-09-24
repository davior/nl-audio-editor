// State published for end-to-end tests and for inspection from the console
// (window.__nlae). Read-only for anything outside this app.
declare global {
  interface Window {
    __nlae?: Record<string, unknown>;
  }
}

export function debug(key: string, value: unknown): void {
  window.__nlae = { ...(window.__nlae ?? {}), [key]: value };
}

/** Record how long a named stage took (seconds), for performance checks. */
export function timing(stage: string, since: number): void {
  const t = { ...((window.__nlae?.timings as Record<string, number> | undefined) ?? {}) };
  t[stage] = Math.round(performance.now() - since) / 1000;
  debug("timings", t);
}
