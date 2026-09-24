// A stand-in for an OpenAI-compatible provider, so the console's model path
// can be tested without a network or a key. It answers from the user's words,
// the way a model would with tool calls, and allows calls from a web page
// (CORS) like a provider that accepts browser requests. It keeps what it was
// sent, so tests can check what reached "the provider".
import { createServer, type IncomingMessage, type Server } from "node:http";

export const MOCK_PORT = 4180;
export const MOCK_URL = `http://127.0.0.1:${MOCK_PORT}/v1`;

/** What the mock received, per request. */
export interface Seen {
  words: string;
  correction: boolean;
  authorization: string | null;
  body: string;
}

type Message = { role: string; content?: string | null; tool_calls?: unknown[] };

function call(name: string, args: unknown, id = "call_1") {
  return { id, type: "function", function: { name, arguments: JSON.stringify(args) } };
}

function reply(content: string | null, calls: unknown[] = []) {
  return {
    id: "chatcmpl-mock",
    object: "chat.completion",
    model: "mock-1",
    choices: [
      {
        index: 0,
        finish_reason: calls.length ? "tool_calls" : "stop",
        message: { role: "assistant", content, ...(calls.length ? { tool_calls: calls } : {}) },
      },
    ],
  };
}

/** The selection the console sent with the words, if any. */
function selectionIn(text: string): { time: [number, number] | null; area: [number, number, number, number] | null } | null {
  const m = /\nSelection: (.*)$/.exec(text);
  if (!m || m[1] === "none") return null;
  return JSON.parse(m[1]);
}

function answer(messages: Message[]): { words: string; correction: boolean; response: unknown } {
  const users = messages.filter((m) => m.role === "user");
  const text = users[users.length - 1]?.content ?? "";
  const words = text.split("\n\n")[0].trim().toLowerCase();
  const correction = messages[messages.length - 1]?.role === "tool";
  let response: unknown;
  if (words.includes("hum is distracting")) {
    response = reply("The 50 Hz hum and its harmonics stand out in the pauses; cutting each line until it matches its surroundings.", [
      call("line_reduce", { params: { lines: "auto" }, scope: { kind: "clip" } }),
    ]);
  } else if (words.includes("knocking")) {
    const area = selectionIn(text)?.area;
    const scope = area ? { kind: "tf_patch", t0: area[0], t1: area[1], f_lo: area[2], f_hi: area[3] } : { kind: "clip" };
    response = reply("Pushing down the knocks where you selected them.", [
      call("spectral_compressor", { params: { mode: "transient" }, scope }),
    ]);
  } else if (words.includes("much louder")) {
    // First an impossible value; asked to correct itself, a valid one.
    response = correction
      ? reply("Raising the level by 6 dB instead.", [call("gain", { params: { gain_db: 6 }, scope: { kind: "clip" } }, "call_2")])
      : reply("Raising the level a lot.", [call("gain", { params: { gain_db: 90 }, scope: { kind: "clip" } })]);
  } else {
    response = reply("Which part of the recording do you mean? Select it, then ask again.");
  }
  return { words, correction, response };
}

function body(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const parts: Buffer[] = [];
    req.on("data", (c: Buffer) => parts.push(c));
    req.on("end", () => resolve(Buffer.concat(parts).toString("utf8")));
    req.on("error", reject);
  });
}

export function startMockModel(): Promise<Server> {
  const seen: Seen[] = [];
  const cors = {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Headers": "authorization, content-type",
    "Access-Control-Allow-Methods": "POST, GET, OPTIONS",
  };
  const server = createServer(async (req, res) => {
    try {
      if (req.method === "OPTIONS") {
        res.writeHead(204, cors).end();
      } else if (req.method === "GET" && req.url === "/seen") {
        res.writeHead(200, { ...cors, "Content-Type": "application/json" }).end(JSON.stringify(seen));
      } else if (req.method === "POST" && req.url === "/v1/chat/completions") {
        const text = await body(req);
        const request = JSON.parse(text) as { messages: Message[]; tools?: unknown[] };
        // A connection test carries no tools and no project.
        const out = request.tools ? answer(request.messages) : { words: "(connection test)", correction: false, response: reply("OK") };
        seen.push({ words: out.words, correction: out.correction, authorization: req.headers.authorization ?? null, body: text });
        res.writeHead(200, { ...cors, "Content-Type": "application/json" }).end(JSON.stringify(out.response));
      } else {
        res.writeHead(404, cors).end();
      }
    } catch (e) {
      res.writeHead(500, cors).end(String(e));
    }
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(MOCK_PORT, "127.0.0.1", () => resolve(server));
  });
}

/** Everything the mock has been sent since it started. */
export async function seenByMock(): Promise<Seen[]> {
  return (await fetch(`http://127.0.0.1:${MOCK_PORT}/seen`)).json() as Promise<Seen[]>;
}
