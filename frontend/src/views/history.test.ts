import { describe, expect, it } from "vitest";
import type { LoggedRequest, StackEntry } from "../core/types";
import { buildRows, nextAfter, type Row, type SessionTurn } from "./history";

const entry = (id: string, active = true): StackEntry => ({ step_id: id, active, edits: 0, drifted: false });

const req = (key: string, seq: number, step_ids: string[] = []): LoggedRequest => ({
  seq,
  ts: "",
  words: key,
  via: "local",
  spoken: false,
  proposed: 0,
  step_ids,
  key,
});

const turn = (words: string, after: number, status: SessionTurn["status"], keys: string[] = []): SessionTurn => ({
  id: 0,
  words,
  via: null,
  status,
  after,
  keys,
});

/** Each row in short: a request's key and its steps' places, or a session turn's words. */
const shape = (rows: Row[]) =>
  rows.map((r) =>
    r.kind === "session"
      ? `(${r.turn.words})`
      : `${r.kind === "request" ? r.request.key : "inherited"}:${r.steps.map((p) => p.n).join(",")}`,
  );

describe("the stack and the console as one list", () => {
  it("puts each request's steps under it, numbered by their place, with answers in words where they happened", () => {
    const entries = [entry("i1"), entry("a1"), entry("a2", false), entry("c1")];
    const requests = [req("A", 3, ["a1", "a2"]), req("B", 6), req("C", 9, ["c1"])];
    expect(shape(buildRows(entries, requests, []))).toEqual(["inherited:1", "A:2,3", "B:", "C:4"]);
  });

  it("shows a request of this session until the log answers it, then the logged one in the same place", () => {
    const entries = [entry("a1")];
    const before = [req("A", 3, ["a1"])];
    const after = nextAfter(before);
    expect(after).toBe(4);
    // Asked; nothing logged yet.
    expect(shape(buildRows(entries, before, [turn("asking", after, "working")]))).toEqual(["A:1", "(asking)"]);
    // A first exchange is logged while it is still being answered: the turn stays, the exchange waits.
    const partial = [...before, req("x1", 5)];
    expect(shape(buildRows(entries, partial, [turn("asking", after, "working", ["x1"])]))).toEqual(["A:1", "(asking)"]);
    // Answered in words: the logged request takes its place.
    const answered = [...before, req("x2", 5)];
    expect(shape(buildRows(entries, answered, [turn("asking", after, "reply", ["x1", "x2"])]))).toEqual(["A:1", "x2:"]);
    // Applied: its step comes under the logged request.
    const applied = [...before, req("b1", 5, ["b1"])];
    const rows = buildRows([...entries, entry("b1")], applied, [turn("asking", after, "applied", ["x1", "b1"])]);
    expect(shape(rows)).toEqual(["A:1", "b1:2"]);
  });

  it("keeps a turn that ended in an error for the session, in place of what the log has of it", () => {
    const entries = [entry("a1")];
    const requests = [req("A", 3, ["a1"]), req("x1", 4)];
    // The model answered (logged), then applying its answer failed (not logged): the error stays.
    expect(shape(buildRows(entries, requests, [turn("refused", 4, "error", ["x1"])]))).toEqual(["A:1", "(refused)"]);
    // Opened again, only the log is left.
    expect(shape(buildRows(entries, requests, []))).toEqual(["A:1", "x1:"]);
  });

  it("keeps turns that logged nothing in the order they were made", () => {
    const entries = [entry("a1"), entry("c1")];
    const session = [turn("listen", 4, "done"), turn("failed", 4, "error")];
    const requests = [req("A", 3, ["a1"]), req("C", 7, ["c1"])];
    expect(shape(buildRows(entries, requests, session))).toEqual(["A:1", "(listen)", "(failed)", "C:2"]);
  });

  it("with steps only, leaves out what put nothing on the stack, but not a request being answered", () => {
    const entries = [entry("a1")];
    const requests = [req("A", 3, ["a1"]), req("B", 6)];
    const session = [turn("listen", 7, "done"), turn("asking", 7, "working")];
    expect(shape(buildRows(entries, requests, session, true))).toEqual(["A:1", "(asking)"]);
  });
});
