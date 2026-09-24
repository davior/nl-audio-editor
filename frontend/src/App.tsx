import { useCallback, useEffect, useRef, useState } from "react";
import type { Capture } from "./audio/recorder";
import { core, transferFiles } from "./core/client";
import * as Comlink from "comlink";
import type { ProjectSummary } from "./core/types";
import { debug, timing } from "./debug";
import { list, load, persist, persistSource, type LibraryEntry } from "./storage/opfs";
import { sync } from "./storage/sync";
import { bytesEqual, isPrefix } from "./storage/tree";
import { Editor } from "./views/Editor";
import { Library } from "./views/Library";

interface Current {
  summary: ProjectSummary;
  detached: boolean;
  notice: string | null;
}

const LAST = "nlae.lastProject";

function remember(id: string | null) {
  try {
    if (id) localStorage.setItem(LAST, id);
    else localStorage.removeItem(LAST);
  } catch {
    /* storage unavailable: nothing to remember */
  }
}

function lastOpened(): string | null {
  try {
    return localStorage.getItem(LAST);
  } catch {
    return null;
  }
}

export function App() {
  const [entries, setEntries] = useState<LibraryEntry[]>([]);
  const [current, setCurrent] = useState<Current | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [parity, setParity] = useState<string | null>(null);
  const [saving, setSaving] = useState(0);
  const currentRef = useRef<Current | null>(null);
  currentRef.current = current;
  const flushRef = useRef<(() => Promise<void>) | null>(null);

  useEffect(() => debug("saving", saving), [saving]);

  const onError = useCallback((e: unknown) => {
    const msg = e instanceof Error ? e.message : String(e);
    console.error(e);
    setError(msg);
    debug("error", msg);
  }, []);

  const refresh = useCallback(async () => {
    try {
      const l = await list();
      setEntries(l);
      debug("library", l);
    } catch (e) {
      onError(e);
    }
  }, [onError]);

  /** Show a project, saving the one on screen first and releasing it in the core. */
  const show = useCallback(
    async (summary: ProjectSummary, detached = false, notice: string | null = null) => {
      const prev = currentRef.current?.summary.id;
      if (flushRef.current) await flushRef.current().catch(onError);
      setCurrent({ summary, detached, notice });
      remember(detached ? null : summary.id);
      if (prev && prev !== summary.id) await core.close(prev);
      await refresh();
    },
    [onError, refresh],
  );

  const run = useCallback(
    async (label: string, f: () => Promise<void>) => {
      setBusy(label);
      setError(null);
      try {
        await f();
      } catch (e) {
        onError(e);
      } finally {
        setBusy(null);
      }
    },
    [onError],
  );

  const openFromLibrary = useCallback(
    (id: string) =>
      run("Opening…", async () => {
        let t = performance.now();
        const files = await load(id);
        timing("open.read", t);
        t = performance.now();
        const s = await core.openProject(transferFiles(files));
        timing("open.verify", t);
        await show(s);
      }),
    [run, show],
  );

  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    (async () => {
      await refresh();
      const last = lastOpened();
      if (last && (await list()).some((e) => e.id === last)) await openFromLibrary(last);
      debug("ready", true);
    })();
  }, [refresh, openFromLibrary]);

  const importFile = (file: File) =>
    run("Importing…", async () => {
      void navigator.storage.persist?.();
      let t = performance.now();
      const bytes = new Uint8Array(await file.arrayBuffer());
      timing("import.read", t);
      t = performance.now();
      const s = await core.importFile(Comlink.transfer(bytes, [bytes.buffer]), file.name, new Date(file.lastModified).toISOString());
      timing("import.create", t);
      // The editor opens at once; the recording, then the project, are saved
      // to the library meanwhile (the recording straight from the file).
      t = performance.now();
      setSaving((n) => n + 1);
      sync(s.id, s.manifest, { skipSource: true, before: () => persistSource(s.manifest, file) })
        .then(() => {
          timing("import.save", t);
          return refresh();
        })
        .catch(onError)
        .finally(() => setSaving((n) => n - 1));
      await show(s);
    });

  const recorded = (wav: Uint8Array, capture: Capture) =>
    run("Saving the recording…", async () => {
      void navigator.storage.persist?.();
      const name = `recording-${capture.started.replace(/[:.]/g, "-")}.wav`;
      const s = await core.importRecording(Comlink.transfer(wav, [wav.buffer as ArrayBuffer]), name, capture);
      await sync(s.id, s.manifest);
      await show(s);
    });

  // A bundle joins the library only if it verifies, and never replaces a
  // library copy unless it continues that copy's history exactly.
  const openBundle = (file: File) =>
    run("Opening bundle…", async () => {
      const bundle = new Uint8Array(await file.arrayBuffer());
      const s = await core.openBundle(Comlink.transfer(bundle, [bundle.buffer]));
      const incoming = await core.takeChangedFiles(s.id);
      if (s.readOnly) {
        await show(s, true, "This bundle did not verify, so it was not added to the library.");
        return;
      }
      if (!(await list()).some((e) => e.id === s.id)) {
        await persist(s.id, incoming, s.manifest);
        await show(s, false, `Added “${s.manifest.project.name}” to the library.`);
        return;
      }
      const local = await load(s.id);
      const a = local["events.jsonl"];
      const b = incoming["events.jsonl"];
      if (bytesEqual(a, b)) {
        await show(s, false, "This project is already in the library with the same history.");
      } else if (isPrefix(a, b)) {
        await persist(s.id, incoming, s.manifest);
        await show(s, false, "The bundle continues the library copy's history; the library copy was updated.");
      } else if (isPrefix(b, a)) {
        const t = await core.openProject(transferFiles(local));
        await show(t, false, "The library already holds a later state of this project; opened the library copy.");
      } else {
        await show(s, true, "The library holds a different history for this project; the bundle is open without replacing it.");
      }
    });

  const checkParity = () =>
    run("Checking…", async () => {
      const r = await core.parity();
      const ok = r.resolved === r.expected_resolved && r.render === r.expected_render;
      setParity(ok ? "bit-identical to the native build" : `DIFFERS from the native build (${r.render})`);
      debug("parity", { ...r, ok });
    });

  return (
    <div className="app">
      <header className="top">
        <h1>
          nlae <span className="muted">forensic audio editor · M0</span>
        </h1>
        {busy && (
          <span className="busy" data-testid="busy">
            {busy}
          </span>
        )}
        {saving > 0 && (
          <span className="busy" data-testid="saving">
            Saving to the library…
          </span>
        )}
      </header>
      {error && (
        <div className="banner bad error" data-testid="error">
          {error}
          <button className="small" onClick={() => setError(null)}>
            dismiss
          </button>
        </div>
      )}
      <Library
        entries={entries}
        currentId={current?.summary.id ?? null}
        busy={!!busy}
        onOpen={openFromLibrary}
        onImport={importFile}
        onOpenBundle={openBundle}
        onRecorded={recorded}
        onError={onError}
      />
      <main>
        {current ? (
          <Editor
            key={current.summary.id}
            summary={current.summary}
            detached={current.detached}
            notice={current.notice}
            onChanged={(s) => setCurrent((c) => (c && c.summary.id === s.id ? { ...c, summary: s } : c))}
            onOpenClone={(child) => show(child, current.detached, null)}
            onClose={async () => {
              if (flushRef.current) await flushRef.current().catch(onError);
              const id = current.summary.id;
              setCurrent(null);
              remember(null);
              await core.close(id);
            }}
            onError={onError}
            flushRef={flushRef}
          />
        ) : (
          <div className="empty">
            <p>Import a recording, record one, or open a bundle. The original is stored byte for byte and never changed.</p>
            <p className="muted">
              Every action is recorded in a hash-chained log. Clones copy a project at any step so different streams of editing can be
              followed side by side.
            </p>
          </div>
        )}
        <footer>
          <button className="small" onClick={checkParity} data-testid="parity" disabled={!!busy}>
            Check core parity
          </button>
          {parity && <span data-testid="parity-result"> The browser core is {parity}.</span>}
        </footer>
      </main>
    </div>
  );
}
