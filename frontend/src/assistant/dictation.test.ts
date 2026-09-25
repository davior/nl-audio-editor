import { describe, expect, it } from "vitest";
import { compose, hear, listenUrl, nothingHeard, SPEECH_DEFAULTS } from "./dictation";

const results = (transcript: string, isFinal: boolean, confidence = 0.9) => ({
  type: "Results",
  is_final: isFinal,
  channel: { alternatives: [{ transcript, confidence }] },
});

describe("hear", () => {
  it("replaces interim words as they firm up, and keeps final ones in order", () => {
    const h = nothingHeard();
    expect(hear(h, results("cut three", false))).toBe("results");
    expect(h.interim).toBe("cut three");
    expect(hear(h, results("Cut 3100 to 3200", true, 0.97))).toBe("results");
    expect(h.finals).toEqual([{ text: "Cut 3100 to 3200", confidence: 0.97 }]);
    expect(h.interim).toBe("");
    hear(h, results("hertz", false));
    hear(h, results("hertz by 12 dB.", true, 0.91));
    expect(h.finals.map((s) => s.text)).toEqual(["Cut 3100 to 3200", "hertz by 12 dB."]);
    expect(h.speech).toBe(true);
  });

  it("ignores empty finals, and notes pauses, speech and the request id", () => {
    const h = nothingHeard();
    hear(h, results("", true));
    expect(h.finals).toEqual([]);
    expect(h.speech).toBe(false);
    expect(hear(h, { type: "SpeechStarted" })).toBe("speech");
    expect(h.speech).toBe(true);
    expect(hear(h, { type: "UtteranceEnd", last_word_end: 2.1 })).toBe("pause");
    expect(hear(h, { type: "Metadata", request_id: "req-7" })).toBe("metadata");
    expect(h.requestId).toBe("req-7");
    expect(hear(h, { type: "Something new" })).toBe("other");
    expect(hear(h, null)).toBe("other");
  });

  it("keeps confidences within 0–1", () => {
    const h = nothingHeard();
    hear(h, results("louder", true, 1.7));
    hear(h, { type: "Results", is_final: true, channel: { alternatives: [{ transcript: "please" }] } });
    expect(h.finals.map((s) => s.confidence)).toEqual([1, 0]);
  });
});

describe("compose", () => {
  it("puts what was typed first, then the words heard, then those still being recognised", () => {
    const finals = [
      { text: "the bangs", confidence: 0.9 },
      { text: "here.", confidence: 0.8 },
    ];
    expect(compose("compress ", finals, "and")).toBe("compress the bangs here. and");
    expect(compose("", [], "")).toBe("");
    expect(compose("  ", [], " undo ")).toBe("undo");
  });
});

describe("listenUrl", () => {
  const cfg = { ...SPEECH_DEFAULTS, key: "dg-secret", remember: false };
  it("adds the query, repeating names as given, and never the key", () => {
    const u = listenUrl(cfg, [
      ["model", "nova-3"],
      ["keyterm", "DC offset"],
      ["keyterm", "hum"],
    ]);
    expect(u.host).toBe("api.deepgram.com");
    expect(u.searchParams.getAll("keyterm")).toEqual(["DC offset", "hum"]);
    expect(u.href).not.toContain("dg-secret");
  });
  it("refuses an address that is not a stream", () => {
    expect(() => listenUrl({ ...cfg, url: "https://api.deepgram.com/v1/listen" }, [])).toThrow(/streaming address/);
    expect(() => listenUrl({ ...cfg, url: "api.deepgram.com" }, [])).toThrow(/web address/);
  });
});
