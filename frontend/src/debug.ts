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
