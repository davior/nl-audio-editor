// Spoken commands: the microphone is streamed to Deepgram, which sends the
// words back as it recognises them. The key goes only in the WebSocket's
// subprotocol (a browser cannot set headers on a WebSocket), so it never
// reaches the core, a project, a log or an export. By default it is held in
// memory for the session; "remember on this device" keeps it in this
// browser's local storage, and says so.
//
// Only the user's dictation is streamed, never a project's audio: the editor
// pauses playback while the microphone is open, and the browser's echo
// cancellation is on. The dictation itself is not stored; the console logs
// the transcript when it is sent.
import type { Segment } from "../core/types";

export const SPEECH_PROVIDER = "deepgram";

export interface SpeechConfig {
  /** The streaming address; the query is added to it. */
  url: string;
  model: string;
  language: string;
  key: string;
  remember: boolean;
  /** Ask Deepgram not to keep the dictation to improve its models (`mip_opt_out`). */
  optOut: boolean;
}

export const SPEECH_DEFAULTS: Omit<SpeechConfig, "key" | "remember"> = {
  url: "wss://api.deepgram.com/v1/listen",
  model: "nova-3",
  language: "en",
  optOut: false,
};

const STORE = "nlae.speech";
let session: SpeechConfig | null = null;

export function loadSpeechConfig(): SpeechConfig {
  if (session) return session;
  try {
    const saved = localStorage.getItem(STORE);
    if (saved) return (session = { ...SPEECH_DEFAULTS, key: "", remember: false, ...JSON.parse(saved) });
  } catch {
    /* storage unavailable */
  }
  return { ...SPEECH_DEFAULTS, key: "", remember: false };
}

export function saveSpeechConfig(cfg: SpeechConfig): void {
  session = cfg;
  try {
    // Remember the settings, never the key, unless asked to.
    localStorage.setItem(STORE, JSON.stringify(cfg.remember ? cfg : { ...cfg, key: "", remember: false }));
  } catch {
    /* storage unavailable: the session copy still works */
  }
}

/** The stream's address with its query. The key is not part of it. */
export function listenUrl(cfg: SpeechConfig, params: [string, string][]): URL {
  let u: URL;
  try {
    u = new URL(cfg.url.trim());
  } catch {
    throw new Error(`“${cfg.url}” is not a web address (it should start with wss://).`);
  }
  if (u.protocol !== "wss:" && u.protocol !== "ws:")
    throw new Error(`“${cfg.url}” is not a streaming address (it should start with wss://).`);
  u.search = new URLSearchParams(params).toString();
  return u;
}

function refused(host: string, cfg: SpeechConfig): string {
  return (
    `${host} refused the connection: check the key and the address.` +
    (cfg.optOut ? " Opting out of Deepgram's model improvement needs a paid account." : "")
  );
}

/** What the recogniser has sent in one listening session. */
export interface Heard {
  /** Final results, in order. */
  finals: Segment[];
  /** Words still being recognised; replaced as they firm up. */
  interim: string;
  interimConfidence: number;
  /** Whether any speech was detected. */
  speech: boolean;
  requestId: string | null;
}

export const nothingHeard = (): Heard => ({ finals: [], interim: "", interimConfidence: 0, speech: false, requestId: null });

type Message = {
  type?: string;
  is_final?: boolean;
  request_id?: string;
  channel?: { alternatives?: { transcript?: string; confidence?: number }[] };
};

/** Fold one message from the stream into `h`; returns the message's kind. */
export function hear(h: Heard, msg: unknown): "results" | "pause" | "speech" | "metadata" | "other" {
  const m = (msg ?? {}) as Message;
  switch (m.type) {
    case "Results": {
      const alt = m.channel?.alternatives?.[0];
      const text = (alt?.transcript ?? "").trim();
      const confidence = Math.min(1, Math.max(0, Number(alt?.confidence) || 0));
      if (text) h.speech = true;
      if (m.is_final) {
        if (text) h.finals.push({ text, confidence });
        h.interim = "";
        h.interimConfidence = 0;
      } else {
        h.interim = text;
        h.interimConfidence = confidence;
      }
      return "results";
    }
    case "UtteranceEnd":
      return "pause";
    case "SpeechStarted":
      h.speech = true;
      return "speech";
    case "Metadata":
      if (typeof m.request_id === "string") h.requestId = m.request_id;
      return "metadata";
    default:
      return "other";
  }
}

/** The console box while listening: what was typed before, the words heard, then the words still being recognised. */
export function compose(before: string, finals: Segment[], interim = ""): string {
  return [before, ...finals.map((s) => s.text), interim]
    .map((s) => s.trim())
    .filter(Boolean)
    .join(" ");
}

export interface ListenHandlers {
  /** Something new was heard (a copy). */
  onHeard(h: Heard): void;
  /** The speaker paused after saying something (Deepgram's `UtteranceEnd`). */
  onPause(): void;
  /** The stream was closed by the other end, or failed; with what was heard until then. */
  onFailure(message: string, heard: Listened): void;
}

/** One listening session, finished. */
export interface Listened {
  heard: Heard;
  host: string;
  params: [string, string][];
  /** Seconds of audio streamed. */
  audioS: number;
  /** From the end of listening to the last result. */
  latencyMs: number;
}

/** Dictation is not evidence: the browser's processing helps recognition, and echo cancellation keeps speaker playback out. */
export const DICTATION_CONSTRAINTS: MediaTrackConstraints = {
  echoCancellation: true,
  noiseSuppression: true,
  autoGainControl: true,
  channelCount: 1,
};

const OPEN_TIMEOUT_MS = 10_000;
const FINISH_TIMEOUT_MS = 2_000;

/** The microphone, streamed to the recogniser until finished or cancelled. */
export class Listening {
  private readonly heard = nothingHeard();
  private readonly queue: ArrayBuffer[] = [];
  private readonly closed: Promise<void>;
  private bytes = 0;
  private wasOpen = false;
  private ending = false;
  private cancelled = false;
  private released = false;
  private node: AudioWorkletNode | null = null;

  private constructor(
    private readonly ws: WebSocket,
    private readonly ctx: AudioContext,
    private readonly stream: MediaStream,
    private readonly cfg: SpeechConfig,
    private readonly url: URL,
    private readonly params: [string, string][],
    handlers: ListenHandlers,
  ) {
    ws.binaryType = "arraybuffer";
    this.closed = new Promise((resolve) => ws.addEventListener("close", () => resolve(), { once: true }));
    ws.addEventListener("open", () => {
      this.wasOpen = true;
      for (const b of this.queue.splice(0)) this.send(b);
    });
    ws.addEventListener("message", (e: MessageEvent) => {
      if (typeof e.data !== "string") return;
      let msg: unknown;
      try {
        msg = JSON.parse(e.data);
      } catch {
        return;
      }
      const kind = hear(this.heard, msg);
      if (kind === "pause") {
        if (this.heard.finals.length > 0 && !this.ending) handlers.onPause();
      } else if (kind !== "other") {
        handlers.onHeard(structuredClone(this.heard));
      }
    });
    ws.addEventListener("close", (e: CloseEvent) => {
      // A stream refused before it opened is reported by `opened`.
      if (this.ending || !this.wasOpen) return;
      const sampleRate = this.ctx.sampleRate;
      this.ending = true;
      this.release();
      handlers.onFailure(
        e.wasClean && e.reason ? `${url.host} closed the stream: ${e.reason}` : `${url.host} closed the stream (code ${e.code}).`,
        this.result(sampleRate, 0),
      );
    });
  }

  /** Whether any speech has been detected yet. */
  get speech(): boolean {
    return this.heard.speech;
  }

  private result(sampleRate: number, latencyMs: number): Listened {
    const heard = structuredClone(this.heard);
    // Words shown but never finalised (the stream ended first) are kept as heard.
    if (heard.interim) heard.finals.push({ text: heard.interim, confidence: heard.interimConfidence });
    heard.interim = "";
    return { heard, host: this.url.host, params: this.params, audioS: this.bytes / 2 / sampleRate, latencyMs };
  }

  /**
   * Open the microphone and the stream. `paramsFor` builds the query once the capture rate is known.
   * Aborting `signal` while it opens closes everything at once: audio captured meanwhile is not sent.
   */
  static async open(
    cfg: SpeechConfig,
    paramsFor: (sampleRate: number) => Promise<[string, string][]>,
    handlers: ListenHandlers,
    signal?: AbortSignal,
  ): Promise<Listening> {
    const key = cfg.key.trim();
    if (!key) throw new Error("Add a Deepgram key in the settings first.");
    const stream = await navigator.mediaDevices.getUserMedia({ audio: DICTATION_CONSTRAINTS });
    const ctx = new AudioContext();
    let l: Listening | null = null;
    const abandon = () => l?.cancel();
    try {
      signal?.throwIfAborted();
      await ctx.audioWorklet.addModule("/dictation-worklet.js");
      const params = await paramsFor(ctx.sampleRate);
      signal?.throwIfAborted();
      const url = listenUrl(cfg, params);
      l = new Listening(new WebSocket(url, ["token", key]), ctx, stream, cfg, url, params, handlers);
      signal?.addEventListener("abort", abandon, { once: true });
      // Capture starts at once, so nothing said while the stream opens is lost.
      l.capture();
      await l.opened();
      return l;
    } catch (e) {
      if (l) l.cancel();
      else {
        stream.getTracks().forEach((t) => t.stop());
        void ctx.close();
      }
      throw e;
    } finally {
      signal?.removeEventListener("abort", abandon);
    }
  }

  private opened(): Promise<void> {
    return new Promise((resolve, reject) => {
      if (this.ws.readyState === WebSocket.OPEN) return resolve();
      const done = (error?: string) => {
        clearTimeout(timer);
        this.ws.removeEventListener("open", onOpen);
        this.ws.removeEventListener("close", onClose);
        if (error === undefined) return resolve();
        this.ending = true;
        reject(new Error(error));
      };
      const onOpen = () => done();
      // Refused before opening: a browser is not told why.
      const onClose = () => done(this.cancelled ? "Listening was cancelled." : refused(this.url.host, this.cfg));
      const timer = setTimeout(() => done(`${this.url.host} did not answer within ${OPEN_TIMEOUT_MS / 1000} s.`), OPEN_TIMEOUT_MS);
      this.ws.addEventListener("open", onOpen);
      this.ws.addEventListener("close", onClose);
    });
  }

  private capture() {
    const src = this.ctx.createMediaStreamSource(this.stream);
    const node = new AudioWorkletNode(this.ctx, "nlae-dictation");
    node.port.onmessage = (e: MessageEvent<ArrayBuffer>) => this.send(e.data);
    // The worklet must be pulled to run; it outputs nothing.
    const silent = this.ctx.createGain();
    silent.gain.value = 0;
    src.connect(node);
    node.connect(silent).connect(this.ctx.destination);
    this.node = node;
    void this.ctx.resume();
  }

  private send(b: ArrayBuffer) {
    if (this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(b);
      this.bytes += b.byteLength;
    } else if (this.ws.readyState === WebSocket.CONNECTING) {
      this.queue.push(b);
    }
  }

  /** Release the microphone and the audio graph (once). */
  private release() {
    if (this.released) return;
    this.released = true;
    if (this.node) this.node.port.onmessage = null;
    this.node?.disconnect();
    this.node = null;
    this.stream.getTracks().forEach((t) => t.stop());
    void this.ctx.close();
  }

  /** Stop listening: the recogniser finalises what it has, then the stream closes. Resolves with everything heard. */
  async finish(): Promise<Listened> {
    const sampleRate = this.ctx.sampleRate;
    this.ending = true;
    this.release();
    const started = performance.now();
    if (this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ type: "CloseStream" }));
      await Promise.race([this.closed, new Promise((r) => setTimeout(r, FINISH_TIMEOUT_MS))]);
    }
    const latencyMs = Math.round(performance.now() - started);
    if (this.ws.readyState !== WebSocket.CLOSED) this.ws.close();
    return this.result(sampleRate, latencyMs);
  }

  /** Stop at once and forget what was heard; nothing more is sent. */
  cancel() {
    this.cancelled = true;
    this.ending = true;
    this.release();
    this.queue.length = 0;
    if (this.ws.readyState === WebSocket.CONNECTING || this.ws.readyState === WebSocket.OPEN) this.ws.close();
  }
}

/** Open a stream and close it straight away: is the recogniser reachable, and does it accept the key and settings? Nothing is spoken. */
export async function testSpeechConnection(
  cfg: SpeechConfig,
  params: [string, string][],
): Promise<{ host: string; latencyMs: number; requestId: string | null }> {
  const key = cfg.key.trim();
  if (!key) throw new Error("Enter a Deepgram key first.");
  const url = listenUrl(cfg, params);
  const started = performance.now();
  const ws = new WebSocket(url, ["token", key]);
  return new Promise((resolve, reject) => {
    let requestId: string | null = null;
    let openedMs: number | null = null;
    let timer = setTimeout(() => {
      ws.close();
      reject(new Error(`${url.host} did not answer within ${OPEN_TIMEOUT_MS / 1000} s.`));
    }, OPEN_TIMEOUT_MS);
    ws.addEventListener("open", () => {
      openedMs = Math.round(performance.now() - started);
      ws.send(JSON.stringify({ type: "CloseStream" }));
      // It accepted the key; if it does not close the stream itself, close it.
      clearTimeout(timer);
      timer = setTimeout(() => ws.close(), FINISH_TIMEOUT_MS);
    });
    ws.addEventListener("message", (e: MessageEvent) => {
      const h = nothingHeard();
      try {
        if (typeof e.data === "string" && hear(h, JSON.parse(e.data)) === "metadata") requestId = h.requestId;
      } catch {
        /* not JSON: ignored */
      }
    });
    ws.addEventListener("close", () => {
      clearTimeout(timer);
      if (openedMs !== null) resolve({ host: url.host, latencyMs: openedMs, requestId });
      else reject(new Error(refused(url.host, cfg)));
    });
  });
}
