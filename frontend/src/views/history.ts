// The console and the stack as one list: each request in the order it was
// made, with the steps it put on the stack under it. Steps keep their stack
// order; a request that put nothing there (an answer in words, one that could
// not be used) goes where it happened. The requests come from the log, so the
// list is the same when the project is opened again; what this session did
// without logging anything (listening, a typed undo, an error before anything
// was logged) is kept only for the session.
import type { LoggedRequest, StackEntry } from "../core/types";

/** A request made in this session, as the console shows it until the log has it. */
export interface SessionTurn {
  id: number;
  words: string;
  via: "local" | "model" | null;
  /** The model asked, when one was. */
  model?: string;
  status: "working" | "applied" | "reply" | "done" | "error";
  text?: string;
  problems?: string[];
  /** The words were dictated (possibly changed before sending). */
  spoken?: boolean;
  /** Made after every request logged before it (see `nextAfter`). */
  after: number;
  /** What the log names it by, once any of it is logged: its exchanges, its first step. */
  keys: string[];
}

/** A place in the stack, numbered from 1. */
export interface Placed {
  entry: StackEntry;
  n: number;
}

export type Row =
  | { kind: "request"; request: LoggedRequest; steps: Placed[] }
  /** Steps no request here made: a clone's, asked for in its parent. */
  | { kind: "inherited"; steps: Placed[] }
  | { kind: "session"; turn: SessionTurn };

/** Where a request made now goes: after every request logged so far. */
export function nextAfter(requests: LoggedRequest[]): number {
  return requests.reduce((m, r) => Math.max(m, r.seq + 1), 0);
}

/**
 * The list, top to bottom. A turn of this session gives way to its request
 * once the log has the answer; until then, what the log has of it waits, so
 * a request is never shown twice or not at all. A turn that ended in an error
 * keeps its own words for the session: the log has what was asked and
 * answered, not why it went no further. With `stepsOnly`, only the requests
 * with steps are listed (and one still being answered).
 */
export function buildRows(entries: StackEntry[], requests: LoggedRequest[], session: SessionTurn[], stepsOnly = false): Row[] {
  const owner = new Map<string, LoggedRequest>();
  for (const r of requests) for (const id of r.step_ids) owner.set(id, r);

  // The steps in stack order, grouped by the request that put them there.
  const groups: { at: number; row: Extract<Row, { steps: Placed[] }> }[] = [];
  entries.forEach((entry, i) => {
    const r = owner.get(entry.step_id);
    const placed = { entry, n: i + 1 };
    const last = groups[groups.length - 1]?.row;
    if (last && (r ? last.kind === "request" && last.request === r : last.kind === "inherited")) last.steps.push(placed);
    else if (r) groups.push({ at: r.seq, row: { kind: "request", request: r, steps: [placed] } });
    else groups.push({ at: -1, row: { kind: "inherited", steps: [placed] } });
  });
  const grouped = new Set(groups.flatMap(({ row }) => (row.kind === "request" ? [row.request] : [])));

  const logged = new Set(requests.map((r) => r.key));
  const turns = session.filter((t) => t.status === "working" || t.status === "error" || !t.keys.some((k) => logged.has(k)));
  const held = new Set(turns.flatMap((t) => t.keys));
  const others: { at: number; row: Row }[] = [
    ...requests
      .filter((r) => !grouped.has(r) && !held.has(r.key))
      .map((r): { at: number; row: Row } => ({ at: r.seq, row: { kind: "request", request: r, steps: [] } })),
    ...turns.map((t): { at: number; row: Row } => ({ at: t.after - 0.5, row: { kind: "session", turn: t } })),
  ]
    .filter(({ row }) => !stepsOnly || (row.kind === "session" && row.turn.status === "working"))
    .sort((a, b) => a.at - b.at);

  // Merged by where each happened in the log.
  const rows: Row[] = [];
  let i = 0;
  for (const g of groups) {
    while (i < others.length && others[i].at < g.at) rows.push(others[i++].row);
    rows.push(g.row);
  }
  while (i < others.length) rows.push(others[i++].row);
  return rows;
}
