// Settings for the console: which provider and model it asks, and the
// recogniser that spoken requests are streamed to. Each key stays in memory
// unless "remember on this device" is ticked; neither is ever written to a
// project. The model's key travels in a request header, the recogniser's in
// the stream's subprotocol.
import { useState } from "react";
import { loadSpeechConfig, saveSpeechConfig, SPEECH_DEFAULTS, testSpeechConnection, type SpeechConfig } from "../assistant/dictation";
import { PRESETS, saveConfig, testConnection, type Preset, type ProviderConfig } from "../assistant/provider";
import { core } from "../core/client";

type Result = { ok: boolean; text: string } | null;

function TestResult({ result, testId }: { result: Result; testId: string }) {
  if (!result) return null;
  return (
    <p className={`small-text ${result.ok ? "good" : "bad"}`} data-testid={testId}>
      {result.text}
    </p>
  );
}

export function Settings({ initial, onClose }: { initial: ProviderConfig; onClose: (saved: boolean) => void }) {
  const [cfg, setCfg] = useState<ProviderConfig>(initial);
  const [speech, setSpeech] = useState<SpeechConfig>(loadSpeechConfig);
  const [test, setTest] = useState<Result>(null);
  const [speechTest, setSpeechTest] = useState<Result>(null);
  const set = (patch: Partial<ProviderConfig>) => {
    setCfg((c) => ({ ...c, ...patch }));
    setTest(null);
  };
  const setSp = (patch: Partial<SpeechConfig>) => {
    setSpeech((c) => ({ ...c, ...patch }));
    setSpeechTest(null);
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
  const runSpeechTest = async () => {
    setSpeechTest({ ok: true, text: "Connecting…" });
    try {
      const params = await core.listenParams(speech.model.trim(), speech.language.trim(), 16000, speech.optOut);
      const r = await testSpeechConnection(speech, params);
      setSpeechTest({
        ok: true,
        text: `${r.host} accepted the stream in ${r.latencyMs} ms${r.requestId ? ` (request ${r.requestId})` : ""}`,
      });
    } catch (e) {
      setSpeechTest({ ok: false, text: e instanceof Error ? e.message : String(e) });
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
        <TestResult result={test} testId="settings-test-result" />
        <div className="modal-actions">
          <button onClick={runTest} data-testid="settings-test" title="Sends one short message that carries nothing from any project">
            Test connection
          </button>
        </div>

        <h4>Speech (Deepgram)</h4>
        <label>
          Streaming address
          <input
            value={speech.url}
            onChange={(e) => setSp({ url: e.target.value })}
            placeholder={SPEECH_DEFAULTS.url}
            data-testid="speech-url"
          />
        </label>
        <label>
          Model
          <input value={speech.model} onChange={(e) => setSp({ model: e.target.value })} data-testid="speech-model" />
        </label>
        <label>
          Language
          <input value={speech.language} onChange={(e) => setSp({ language: e.target.value })} data-testid="speech-language" />
        </label>
        <label>
          Key
          <input
            type="password"
            value={speech.key}
            onChange={(e) => setSp({ key: e.target.value })}
            placeholder="Deepgram API key"
            autoComplete="off"
            data-testid="speech-key"
          />
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={speech.remember}
            onChange={(e) => setSp({ remember: e.target.checked })}
            data-testid="speech-remember"
          />
          Remember on this device (kept in this browser's local storage)
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={!speech.optOut}
            onChange={(e) => setSp({ optOut: !e.target.checked })}
            data-testid="speech-improve"
          />
          Let Deepgram keep my dictation to improve its models (its default and lower price)
        </label>
        <p className="muted small-text">
          <em>Speak</em> in the console streams your voice to Deepgram while the microphone is on, and the words come back as they are
          recognised. A recording is never sent: playback pauses while the microphone is on. The words fill the box and nothing happens
          until you press Enter; what was heard and what you sent are recorded in the project's log, the audio is not. Unticking the box
          above asks Deepgram not to keep your dictation, which costs more and needs a paid account. The key is never recorded.
        </p>
        <TestResult result={speechTest} testId="speech-test-result" />
        <div className="modal-actions">
          <button onClick={runSpeechTest} data-testid="speech-test" title="Opens a stream and closes it at once; nothing is spoken">
            Test connection
          </button>
        </div>

        <div className="modal-actions">
          <button onClick={() => onClose(false)}>Cancel</button>
          <button
            className="accept"
            onClick={() => {
              saveConfig(cfg);
              saveSpeechConfig({ ...speech, url: speech.url.trim(), model: speech.model.trim(), language: speech.language.trim() });
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
