// Assistant settings: which provider and model the console asks, and the key.
// The key stays in memory unless "remember on this device" is ticked; it is
// sent in the request header only and never written to a project.
import { useState } from "react";
import { PRESETS, saveConfig, testConnection, type Preset, type ProviderConfig } from "../assistant/provider";

export function Settings({ initial, onClose }: { initial: ProviderConfig; onClose: (saved: boolean) => void }) {
  const [cfg, setCfg] = useState<ProviderConfig>(initial);
  const [test, setTest] = useState<{ ok: boolean; text: string } | null>(null);
  const set = (patch: Partial<ProviderConfig>) => {
    setCfg((c) => ({ ...c, ...patch }));
    setTest(null);
  };
  const runTest = async () => {
    setTest({ ok: true, text: "Asking…" });
    try {
      const r = await testConnection(cfg);
      setTest({ ok: true, text: `${r.host} answered in ${r.latencyMs} ms${r.reply ? `: “${r.reply.slice(0, 40)}”` : ""}` });
    } catch (e) {
      setTest({ ok: false, text: e instanceof Error ? e.message : String(e) });
    }
  };
  return (
    <div className="modal-back" data-testid="settings">
      <div className="modal">
        <h3>Assistant</h3>
        <label>
          Provider
          <select
            value={cfg.preset}
            onChange={(e) => {
              const p = e.target.value as Preset;
              setCfg((c) => ({ ...c, ...PRESETS[p], key: p === c.preset ? c.key : "" }));
              setTest(null);
            }}
            data-testid="settings-preset"
          >
            <option value="deepseek">DeepSeek (cloud)</option>
            <option value="ollama">Ollama (on this computer)</option>
            <option value="custom">Other OpenAI-compatible</option>
          </select>
        </label>
        <label>
          Address
          <input
            value={cfg.baseUrl}
            onChange={(e) => set({ baseUrl: e.target.value })}
            placeholder="https://…"
            data-testid="settings-url"
          />
        </label>
        <label>
          Model
          <input value={cfg.model} onChange={(e) => set({ model: e.target.value })} data-testid="settings-model" />
        </label>
        {cfg.preset === "custom" && (
          <label>
            Name recorded for this provider
            <input value={cfg.provider} onChange={(e) => set({ provider: e.target.value })} data-testid="settings-provider" />
          </label>
        )}
        <label>
          Key
          <input
            type="password"
            value={cfg.key}
            onChange={(e) => set({ key: e.target.value })}
            placeholder={cfg.preset === "ollama" ? "not needed" : "sk-…"}
            autoComplete="off"
            data-testid="settings-key"
          />
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={cfg.remember}
            onChange={(e) => set({ remember: e.target.checked })}
            data-testid="settings-remember"
          />
          Remember on this device (kept in this browser's local storage)
        </label>
        <p className="muted small-text">
          The model is sent your words and numbers from the analysis — never audio — and every exchange is recorded in the project's log.
          The key is never recorded. If requests fail from the browser, the provider may not accept calls from a web page (CORS); Ollama
          needs <code>OLLAMA_ORIGINS</code> to include this page's address.
        </p>
        {test && (
          <p className={`small-text ${test.ok ? "good" : "bad"}`} data-testid="settings-test-result">
            {test.text}
          </p>
        )}
        <div className="modal-actions">
          <button onClick={runTest} data-testid="settings-test" title="Sends one short message that carries nothing from any project">
            Test connection
          </button>
          <button onClick={() => onClose(false)}>Cancel</button>
          <button
            className="accept"
            onClick={() => {
              saveConfig(cfg);
              onClose(true);
            }}
            data-testid="settings-save"
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
