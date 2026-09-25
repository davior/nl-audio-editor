// A stand-in for Deepgram's streaming recogniser, so spoken requests can be
// tested without a network or a key. It accepts a stream authenticated the
// way a browser must (the key in the `token` subprotocol), keeps what it was
// sent, and "hears" what the test scripted for the key it was given, paced by
// the audio as it arrives: interim words, then a final result, then
// `UtteranceEnd`. `CloseStream` finalises what is left, sends `Metadata` and
// closes, as Deepgram does. Scripts and records are kept per key, so tests
// running in parallel do not see each other's streams.
import { createServer, type IncomingMessage, type Server } from "node:http";
import { WebSocketServer } from "ws";

export const DG_PORT = 4181;
export const DG_URL = `ws://127.0.0.1:${DG_PORT}/v1/listen`;
/** Keys the stand-in accepts start with this; any other key is refused (401). */
export const DG_KEY_PREFIX = "dg-e2e-";

/** What streams opened with a key will hear. */
export interface Script {
  /** Final results, in order; each is preceded by an interim result with its first half. */
  finals: string[];
  /** Seconds of audio between one result and the next (default 0.4). */
  step?: number;
  /** Send `UtteranceEnd` after the last final result (default true). */
  pause?: boolean;
  /** Hold each stream's opening handshake this long (ms), as a slow network would. */
  openDelay?: number;
}

/** One stream, as the stand-in saw it. */
export interface Stream {
  key: string;
  /** Path and query, as requested. */
  url: string;
  /** The `Sec-WebSocket-Protocol` header. */
  protocols: string;
  audioBytes: number;
  frames: number;
  /** Frames that were not whole 16-bit samples. */
  oddFrames: number;
  nonZeroSamples: number;
  /** Text messages from the client, by type. */
  controls: string[];
  requestId: string;
}

function results(text: string, isFinal: boolean, start: number, requestId: string) {
  return JSON.stringify({
    type: "Results",
    channel_index: [0, 1],
    start,
    duration: 0.4,
    is_final: isFinal,
    speech_final: isFinal,
    channel: { alternatives: [{ transcript: text, confidence: isFinal ? 0.96 : 0.71, words: [] }] },
    metadata: { request_id: requestId },
  });
}

function body(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const parts: Buffer[] = [];
    req.on("data", (c: Buffer) => parts.push(c));
    req.on("end", () => resolve(Buffer.concat(parts).toString("utf8")));
    req.on("error", reject);
  });
}

/** The key a browser sends: `Sec-WebSocket-Protocol: token, <key>`. */
function keyOf(req: IncomingMessage): string | null {
  const offered = (req.headers["sec-websocket-protocol"] ?? "").split(",").map((s) => s.trim());
  return offered[0] === "token" && offered[1] ? offered[1] : null;
}

export function startMockDeepgram(): Promise<Server> {
  const scripts = new Map<string, Script>();
  const streams: Stream[] = [];
  const handshakes = new Map<string, number>();
  let n = 0;
  const server = createServer(async (req, res) => {
    const u = new URL(req.url ?? "/", "http://localhost");
    const key = u.searchParams.get("key") ?? "";
    try {
      if (req.method === "POST" && u.pathname === "/script") {
        scripts.set(key, JSON.parse(await body(req)) as Script);
        res.writeHead(204).end();
      } else if (req.method === "GET" && u.pathname === "/seen") {
        res.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify(streams.filter((s) => s.key === key)));
      } else if (req.method === "GET" && u.pathname === "/handshakes") {
        res.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify(handshakes.get(key) ?? 0));
      } else {
        res.writeHead(404).end();
      }
    } catch (e) {
      res.writeHead(500).end(String(e));
    }
  });
  const wss = new WebSocketServer({
    server,
    verifyClient: ({ req }, done) => {
      const path = new URL(req.url ?? "/", "http://localhost").pathname;
      if (path !== "/v1/listen") return done(false, 404, "Not Found");
      const key = keyOf(req);
      if (!key?.startsWith(DG_KEY_PREFIX)) return done(false, 401, "Unauthorized");
      handshakes.set(key, (handshakes.get(key) ?? 0) + 1);
      setTimeout(() => done(true), scripts.get(key)?.openDelay ?? 0);
    },
    handleProtocols: (protocols) => (protocols.has("token") ? "token" : false),
  });
  wss.on("connection", (ws, req) => {
    const key = keyOf(req)!;
    const query = new URL(req.url ?? "/", "http://localhost").searchParams;
    const bytesPerSecond = 2 * Number(query.get("sample_rate") || 16000);
    const script: Script = scripts.get(key) ?? { finals: ["remove the hum"] };
    const step = script.step ?? 0.4;
    const s: Stream = {
      key,
      url: req.url ?? "",
      protocols: req.headers["sec-websocket-protocol"] ?? "",
      audioBytes: 0,
      frames: 0,
      oddFrames: 0,
      nonZeroSamples: 0,
      controls: [],
      requestId: `req-${++n}`,
    };
    streams.push(s);

    // What is due at each moment of audio: speech detected, then for each final
    // result an interim one with its first half, then the pause.
    const due: { at: number; final?: boolean; send: () => void }[] = [];
    due.push({ at: step * 0.5, send: () => ws.send(JSON.stringify({ type: "SpeechStarted", timestamp: 0 })) });
    script.finals.forEach((text, i) => {
      const words = text.split(" ");
      const interim = words.slice(0, Math.ceil(words.length / 2)).join(" ");
      due.push({ at: step * (2 * i + 1), send: () => ws.send(results(interim, false, i, s.requestId)) });
      due.push({ at: step * (2 * i + 2), final: true, send: () => ws.send(results(text, true, i, s.requestId)) });
    });
    if (script.pause !== false)
      due.push({
        at: step * (2 * script.finals.length + 0.5),
        send: () => ws.send(JSON.stringify({ type: "UtteranceEnd", last_word_end: 0 })),
      });
    const heard = () => s.audioBytes / bytesPerSecond;

    ws.on("message", (data, isBinary) => {
      if (isBinary) {
        const b = data as Buffer;
        s.frames++;
        s.audioBytes += b.length;
        if (b.length % 2 !== 0) s.oddFrames++;
        for (let i = 0; i + 1 < b.length; i += 2) if (b.readInt16LE(i) !== 0) s.nonZeroSamples++;
        while (due.length > 0 && due[0].at <= heard()) due.shift()!.send();
        return;
      }
      let msg: { type?: string };
      try {
        msg = JSON.parse(data.toString());
      } catch {
        return;
      }
      s.controls.push(msg.type ?? "?");
      if (msg.type === "CloseStream") {
        // Whatever is still to be finalised is, then the summary, then the stream closes.
        for (const d of due.splice(0)) if (d.final) d.send();
        ws.send(JSON.stringify({ type: "Metadata", request_id: s.requestId, duration: heard(), channels: 1 }));
        ws.close(1000);
      }
    });
  });
  // Closing the server ends any stream still open, so the test run can finish.
  const close = server.close.bind(server);
  server.close = (cb) => {
    wss.clients.forEach((c) => c.terminate());
    return close(cb);
  };
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(DG_PORT, "127.0.0.1", () => resolve(server));
  });
}

/** Set what streams opened with `key` will hear. */
export async function scriptDeepgram(key: string, script: Script): Promise<void> {
  await fetch(`http://127.0.0.1:${DG_PORT}/script?key=${encodeURIComponent(key)}`, { method: "POST", body: JSON.stringify(script) });
}

/** The streams opened with `key`, as the stand-in saw them. */
export async function seenByDeepgram(key: string): Promise<Stream[]> {
  return (await fetch(`http://127.0.0.1:${DG_PORT}/seen?key=${encodeURIComponent(key)}`)).json() as Promise<Stream[]>;
}

/** How many streams were requested with `key`, whether or not they opened. */
export async function handshakesSeen(key: string): Promise<number> {
  return (await fetch(`http://127.0.0.1:${DG_PORT}/handshakes?key=${encodeURIComponent(key)}`)).json() as Promise<number>;
}
