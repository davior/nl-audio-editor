// The local library: import, record, open a bundle, and every project grouped
// by its source recording with clones under the project they came from.
import { useEffect, useRef, useState } from "react";
import { Recorder } from "../audio/recorder";
import type { Capture } from "../audio/recorder";
import type { LibraryEntry } from "../storage/opfs";
import { families, type TreeNode } from "../storage/tree";

export interface LibraryProps {
  entries: LibraryEntry[];
  currentId: string | null;
  busy: boolean;
  onOpen: (id: string) => void;
  onImport: (file: File) => void;
  onOpenBundle: (file: File) => void;
  onRecorded: (wav: Uint8Array, capture: Capture) => void;
  onError: (e: unknown) => void;
}

function Node({
  node,
  depth,
  currentId,
  onOpen,
}: {
  node: TreeNode;
  depth: number;
  currentId: string | null;
  onOpen: (id: string) => void;
}) {
  const e = node.entry;
  return (
    <li>
      <button
        className={`entry ${e.id === currentId ? "current" : ""}`}
        style={{ paddingLeft: 8 + depth * 14 }}
        onClick={() => onOpen(e.id)}
        data-testid="library-entry"
        data-id={e.id}
        data-parent={e.parent ?? ""}
        data-depth={depth}
        title={e.id}
      >
        {depth > 0 && <span className="branch">└ </span>}
        <span className="entry-name">{e.name}</span>
        <span className="entry-meta">
          {e.steps} step{e.steps === 1 ? "" : "s"}
        </span>
      </button>
      {node.children.length > 0 && (
        <ul>
          {node.children.map((c) => (
            <Node key={c.entry.id} node={c} depth={depth + 1} currentId={currentId} onOpen={onOpen} />
          ))}
        </ul>
      )}
    </li>
  );
}

export function Library({ entries, currentId, busy, onOpen, onImport, onOpenBundle, onRecorded, onError }: LibraryProps) {
  const audioInput = useRef<HTMLInputElement>(null);
  const bundleInput = useRef<HTMLInputElement>(null);
  const recorder = useRef<Recorder | null>(null);
  const [recording, setRecording] = useState<number | null>(null);
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    if (recording === null) return;
    const h = setInterval(() => setElapsed((performance.now() - recording) / 1000), 100);
    return () => clearInterval(h);
  }, [recording]);

  const startRecording = async () => {
    try {
      const r = new Recorder();
      await r.start();
      recorder.current = r;
      setElapsed(0);
      setRecording(performance.now());
    } catch (e) {
      onError(e);
    }
  };

  const stopRecording = async () => {
    const r = recorder.current;
    recorder.current = null;
    setRecording(null);
    if (!r) return;
    try {
      const { wav, capture } = await r.stop();
      onRecorded(wav, capture);
    } catch (e) {
      onError(e);
    }
  };

  const groups = families(entries);

  return (
    <aside className="library" data-testid="library">
      <h2>Library</h2>
      <div className="library-actions">
        <button onClick={() => audioInput.current?.click()} disabled={busy} data-testid="import">
          Import audio…
        </button>
        <button onClick={() => bundleInput.current?.click()} disabled={busy} data-testid="open-bundle">
          Open bundle…
        </button>
        {recording === null ? (
          <button
            onClick={startRecording}
            disabled={busy}
            data-testid="record"
            title="Records with the browser's echo cancellation, noise suppression and gain control switched off"
          >
            ● Record
          </button>
        ) : (
          <button onClick={stopRecording} className="recording" data-testid="stop-recording">
            ■ Stop {elapsed.toFixed(1)} s
          </button>
        )}
        <input
          ref={audioInput}
          type="file"
          accept="audio/*,.wav,.mp3,.flac,.ogg,.m4a"
          hidden
          data-testid="import-input"
          onChange={(e) => {
            const f = e.target.files?.[0];
            e.target.value = "";
            if (f) onImport(f);
          }}
        />
        <input
          ref={bundleInput}
          type="file"
          accept=".nlae,application/zip"
          hidden
          data-testid="bundle-input"
          onChange={(e) => {
            const f = e.target.files?.[0];
            e.target.value = "";
            if (f) onOpenBundle(f);
          }}
        />
      </div>
      {groups.length === 0 && <p className="muted">Nothing here yet. Import a recording, record one, or open a bundle.</p>}
      {groups.map((g) => (
        <section key={g.sourceSha} className="family" data-testid="library-source" data-sha={g.sourceSha}>
          <div className="family-head" title={g.sourceSha}>
            <span className="entry-name">{g.filename}</span>
            <span className="entry-meta">{g.duration.toFixed(1)} s</span>
          </div>
          <ul>
            {g.roots.map((n) => (
              <Node key={n.entry.id} node={n} depth={0} currentId={currentId} onOpen={onOpen} />
            ))}
          </ul>
        </section>
      ))}
    </aside>
  );
}
