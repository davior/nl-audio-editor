import { useState } from "react";

type Ev = { seq: number; ts: string; type: string; hash: string; actor: { kind: string; model?: string } };

export interface LogFailure {
  line: number;
  expected_seq: number;
  reason: string;
}

/** The project's event log. Only verified events are listed; a break is shown where it happens. */
export function LogViewer({ events, failure }: { events: Record<string, unknown>[]; failure: LogFailure | null }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="panel" data-testid="log">
      <div className="panel-head">
        <h3>Event log</h3>
        <button onClick={() => setOpen(!open)} data-testid="log-toggle">
          {open ? "Hide" : `Show ${events.length}`}
        </button>
      </div>
      {open && (
        <div className="log-wrap">
          <table className="log">
            <tbody>
              {(events as unknown as Ev[]).map((e) => (
                <tr key={e.seq} data-testid="log-row">
                  <td>{e.seq}</td>
                  <td>{e.ts.replace("T", " ").replace("Z", "")}</td>
                  <td>{e.type}</td>
                  <td>{e.actor.model ?? e.actor.kind}</td>
                  <td className="mono">{e.hash.slice(7, 19)}…</td>
                </tr>
              ))}
              {failure && (
                <tr className="unverified" data-testid="log-break">
                  <td colSpan={5}>
                    ✗ Line {Number(failure.line)} (seq {Number(failure.expected_seq)}) and everything after it: not verified.{" "}
                    {failure.reason}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
