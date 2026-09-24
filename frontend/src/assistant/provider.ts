// The provider client: sends a request body the core built to an
// OpenAI-compatible chat-completions endpoint. The key is added here, in the
// request headers only, so it never reaches the core, a project, a log or an
// export. By default it is held in memory for the session; "remember on this
// device" keeps it in this browser's local storage, and says so.

export type Preset = "deepseek" | "ollama" | "custom";

export interface ProviderConfig {
  preset: Preset;
  /** Base URL; `/chat/completions` is appended. */
  baseUrl: string;
  model: string;
  /** Recorded on every assistant step as `actor.provider`. */
  provider: string;
  key: string;
  remember: boolean;
}

export const PRESETS: Record<Preset, Omit<ProviderConfig, "key" | "remember">> = {
  deepseek: { preset: "deepseek", baseUrl: "https://api.deepseek.com", model: "deepseek-chat", provider: "deepseek" },
  ollama: { preset: "ollama", baseUrl: "http://localhost:11434/v1", model: "qwen2.5", provider: "ollama" },
  custom: { preset: "custom", baseUrl: "", model: "", provider: "custom" },
};

const STORE = "nlae.provider";
let session: ProviderConfig | null = null;

export function loadConfig(): ProviderConfig {
  if (session) return session;
  try {
    const saved = localStorage.getItem(STORE);
    if (saved) return (session = { ...PRESETS.deepseek, key: "", remember: true, ...JSON.parse(saved) });
  } catch {
    /* storage unavailable */
  }
  return { ...PRESETS.deepseek, key: "", remember: false };
}

export function saveConfig(cfg: ProviderConfig): void {
  session = cfg;
  try {
    if (cfg.remember) localStorage.setItem(STORE, JSON.stringify(cfg));
    else {
      // Remember the choice of provider, never the key, unless asked to.
      localStorage.setItem(STORE, JSON.stringify({ ...cfg, key: "", remember: false }));
    }
  } catch {
    /* storage unavailable: the session copy still works */
  }
}

export function endpoint(cfg: ProviderConfig): string {
  const url = `${cfg.baseUrl.trim().replace(/\/+$/, "")}/chat/completions`;
  try {
    const u = new URL(url);
    if (u.protocol !== "https:" && u.protocol !== "http:") throw new Error();
  } catch {
    throw new Error(`“${cfg.baseUrl}” is not a web address (it should start with https:// or http://).`);
  }
  return url;
}

export interface Completion {
  response: unknown;
  latencyMs: number;
  host: string;
}

/** Send a request body (JSON text built by the core). */
export async function complete(cfg: ProviderConfig, body: string): Promise<Completion> {
  if (!cfg.baseUrl.trim() || !cfg.model.trim()) throw new Error("Choose a provider and model in the assistant settings first.");
  const url = endpoint(cfg);
  const started = performance.now();
  let r: Response;
  try {
    r = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(cfg.key ? { Authorization: `Bearer ${cfg.key}` } : {}),
      },
      body,
    });
  } catch (e) {
    throw new Error(
      `Could not reach ${new URL(url).host}. The provider may not accept requests from a web page (CORS), or the network blocks it. (${String(e)})`,
    );
  }
  if (!r.ok) {
    const text = (await r.text()).slice(0, 300);
    throw new Error(`${new URL(url).host} answered ${r.status}: ${text}`);
  }
  return { response: await r.json(), latencyMs: Math.round(performance.now() - started), host: new URL(url).host };
}

/** A one-line exchange that carries nothing from any project: is the provider reachable, and does the key work? */
export async function testConnection(cfg: ProviderConfig): Promise<{ host: string; latencyMs: number; reply: string }> {
  const body = JSON.stringify({
    model: cfg.model,
    messages: [{ role: "user", content: "Reply with the word OK." }],
    max_tokens: 5,
    temperature: 0,
  });
  const { response, latencyMs, host } = await complete(cfg, body);
  const reply = (response as { choices?: { message?: { content?: string } }[] }).choices?.[0]?.message?.content ?? "";
  return { host, latencyMs, reply: reply.trim() };
}
