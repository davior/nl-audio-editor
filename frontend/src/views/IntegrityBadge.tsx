import { useState } from "react";
import type { Report } from "../core/types";

export function IntegrityBadge({ report, events }: { report: Report; events: number }) {
  const [open, setOpen] = useState(false);
  const problems = report.problems ?? [];
  const ok = problems.length === 0;
  const detailed = "log" in report ? report : null;
  return (
    <div className={`badge ${ok ? "ok" : "bad"}`} data-testid="integrity" data-ok={ok ? "true" : "false"}>
      <button className="badge-btn" onClick={() => setOpen(!open)} title="Integrity of the source, the log and the stack">
        {ok ? "✓ Verified" : "✗ Not verified"}
      </button>
      {open && (
        <div className="badge-detail">
          {detailed && (
            <>
              <div>Source {detailed.source_ok ? "matches" : "does NOT match"} its recorded hash</div>
              <div>
                Log: {detailed.log.ok ? `${detailed.log.verified} events verified` : `broken at line ${detailed.log.first_failure?.line}`}
              </div>
              {detailed.lineage.map((l) => (
                <div key={l.project}>
                  Ancestor {l.project}: {l.verification.ok ? "verified" : "broken"}
                </div>
              ))}
            </>
          )}
          {!detailed && <div>{events} events recorded this session</div>}
          {problems.map((p, i) => (
            <div key={i} className="problem" data-testid="integrity-problem">
              {p}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
