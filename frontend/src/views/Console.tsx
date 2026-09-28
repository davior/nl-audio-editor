// The stack and the console, one list: say what you want, typed or spoken,
// and each request is listed with the steps it put on the stack under it.
// Routine requests are handled locally; the rest go to the chosen model with
// words and analysis numbers only. A request applies at once, to the whole
// recording, and is reviewed in place: any step can be removed, restored or
// changed, and undone or redone. The requests are rebuilt from the log, so the
// list is the same when the project is opened again. Spoken words fill the
// box as they are recognised and are sent, like typed ones, with Enter; what
// was heard is logged next to what was sent.
import { useEffect, useMemo, useRef, useState } from "react";
import { compose, Listening, loadSpeechConfig, SPEECH_PROVIDER, type Listened } from "../assistant/dictation";
import { complete, loadConfig, type ProviderConfig } from "../assistant/provider";
import { core } from "../core/client";
import type { AppliedResult, LoggedRequest, ProjectSummary, Segment, Selection, Turn, Which } from "../core/types";
import { debug } from "../debug";
import { buildRows, nextAfter, type Placed, type SessionTurn } from "./history";
import { Settings } from "./Settings";
import type { Descriptor } from "./StepEditor";
import { changeWords, stepOf, StepRow, type StackActions } from "./StepRow";

/** The dictation behind the words in the box, until they are sent or cleared. */
interface Spoken {
  model: string;
  host: string;
  params: [string, string][];
  requestIds: string[];
  segments: Segment[];
  audioS: number;
  latencyMs: number;
}

type Mic = "off" | "starting" | "listening" | "finishing";

/** Listening stops by itself after this long, or this long without speech. */
const MAX_LISTEN_MS = 30_000;
const NO_SPEECH_MS = 8_000;

/** The stack's side of the list: its steps' controls, and the project's. */
export interface StackProps {
  actions?: StackActions;
  /** Something else is changing the project. */
  busy: boolean;
  /** The selected step, if any: its values can be changed, and it can be heard on its own. */
  selected: string | null;
  onSelect: (id: string | null) => void;
  descriptors: Descriptor[];
  onClone: (atStep?: string) => void;
  canClone: boolean;
  /** Rate the stack as it stands, 1–5. */
  onRate?: (overall: number) => void;
  /** With time edits: the output's length and the original's (seconds). */
  output?: { duration: number; original: number } | null;
  /** When the log did not verify: only what was recorded before this line is listed. */
  brokenAtLine: number | null;
  /** A clone's parent, whose steps it inherited. */
  inheritedFrom?: string;
}

interface Props {
  summary: ProjectSummary;
  selection: Selection | null;
  /** Why no request can be made right now, if none can. */
  unavailable: string | null;
  onSummary: (s: ProjectSummary) => void;
  onListen: (which: Which) => void;
  save: () => Promise<void>;
  /** A request made with a button elsewhere; handled as if typed (each new `n` once). */
  request?: { words: string; n: number } | null;
  /** The microphone opened (true) or closed: playback pauses meanwhile, so no recording is streamed. */
  onDictating: (on: boolean) => void;
  stack: StackProps;
}

/** How a request was answered, in a line: spoken, handled here or by which model. */
function viaWords(r: LoggedRequest): string {
  const how =
    r.via === "model"
      ? `via ${[r.provider, r.model].filter(Boolean).join(" · ")}`
      : r.origin === "cli"
        ? "from the command line"
        : r.origin === "panel"
          ? "made by hand"
          : "handled here";
  return `${r.spoken ? "spoken · " : ""}${how}`;
}

/** The answer to a logged request, in words. */
function answerWords(r: LoggedRequest, applied: boolean): string | null {
  if (r.problems?.length) return "The model's proposal could not be used.";
  if (r.recipe) return `The built-in ${r.recipe} recipe, measured on this recording.`;
  if (r.text) return r.text;
  return applied ? null : "No answer was recorded.";
}

export function Console({ summary, selection, unavailable, onSummary, onListen, save, request, onDictating, stack }: Props) {
  const id = summary.id;
  const [turns, setTurns] = useState<SessionTurn[]>([]);
  const [stepsOnly, setStepsOnly] = useState(false);
  const [words, setWords] = useState("");
  const [busy, setBusy] = useState(false);
  const next = useRef(1);
  const turnsEl = useRef<HTMLDivElement>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [configVersion, setConfigVersion] = useState(0);
  const cfg = useMemo<ProviderConfig>(() => loadConfig(), [configVersion]);
  // Dictation: the words fill the box as they are recognised; nothing is sent until Enter.
  const [mic, setMic] = useState<Mic>("off");
  const [micError, setMicError] = useState<string | null>(null);
  const listening = useRef<Listening | null>(null);
  // Each opening of the microphone; cancelling (or leaving) abandons one still opening.
  const attempt = useRef(0);
  const opening = useRef<AbortController | null>(null);
  const typedBefore = useRef("");
  const spoken = useRef<Spoken | null>(null);
  const micTimers = useRef<ReturnType<typeof setTimeout>[]>([]);
  const inputEl = useRef<HTMLInputElement>(null);

  useEffect(
    () =>
      debug(
        "console",
        turns.map((t) => ({ words: t.words, via: t.via, status: t.status })),
      ),
    [turns],
  );

  const { state, requests } = summary;
  const rows = buildRows(state.entries, requests, turns, stepsOnly);

  // The latest request stays in view; changing a step higher up does not move the list.
  useEffect(() => {
    const el = turnsEl.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [rows.length]);

  const update = (tid: number, patch: Partial<SessionTurn>) => setTurns((ts) => ts.map((t) => (t.id === tid ? { ...t, ...patch } : t)));

  const history: Turn[] = turns
    .filter((t) => t.status !== "working")
    .flatMap((t) => [{ role: "user", text: t.words }, ...(t.text ? [{ role: "assistant", text: t.text }] : [])]);

  // The logged request takes the turn's place in the same update (see `buildRows`).
  const showApplied = async (tid: number, r: AppliedResult, via: "local" | "model", keys: string[], text?: string) => {
    await save();
    onSummary(r.summary);
    update(tid, { status: "applied", via, text, keys: [...keys, r.steps[0].step_id] });
    onListen("stack");
  };

  /** An answer that put nothing on the stack; the log has it. */
  const showAnswered = async (tid: number, patch: Partial<SessionTurn>) => {
    await save();
    onSummary(await core.summary(id));
    update(tid, patch);
  };

  /** Take back the last change to the stack, or repeat the last one taken back. */
  const undoOrRedo = async (which: "undo" | "redo"): Promise<string> => {
    const r = which === "undo" ? await core.undo(id) : await core.redo(id);
    await save();
    onSummary(r.summary);
    const words = changeWords(r.summary.state, r.change);
    return which === "undo" ? `Undid ${words} (it stays in the log).` : `Redid ${words}.`;
  };

  const clearMicTimers = () => {
    micTimers.current.forEach(clearTimeout);
    micTimers.current = [];
  };

  const endListening = () => {
    clearMicTimers();
    setMic("off");
    onDictating(false);
    inputEl.current?.focus();
  };

  /** Keep what a listening session heard with the words in the box. */
  const keepHeard = (r: Listened) => {
    setWords(compose(typedBefore.current, r.heard.finals));
    if (r.heard.finals.length === 0) return;
    const s = spoken.current ?? {
      model: r.params.find(([k]) => k === "model")?.[1] ?? loadSpeechConfig().model,
      host: r.host,
      params: r.params,
      requestIds: [],
      segments: [],
      audioS: 0,
      latencyMs: 0,
    };
    s.segments.push(...r.heard.finals);
    if (r.heard.requestId) s.requestIds.push(r.heard.requestId);
    s.audioS += r.audioS;
    s.latencyMs = r.latencyMs;
    spoken.current = s;
  };

  const finishListening = async () => {
    const l = listening.current;
    if (!l) return;
    listening.current = null;
    setMic("finishing");
    clearMicTimers();
    try {
      keepHeard(await l.finish());
    } catch (e) {
      setMicError(e instanceof Error ? e.message : String(e));
    } finally {
      endListening();
    }
  };

  /** Stop at once; the box goes back to what was typed before. */
  const cancelListening = () => {
    attempt.current++;
    opening.current?.abort();
    const l = listening.current;
    listening.current = null;
    l?.cancel();
    setWords(typedBefore.current);
    endListening();
  };

  const startListening = async () => {
    if (listening.current || mic !== "off") return;
    const speech = loadSpeechConfig();
    if (!speech.key.trim()) {
      setMicError("Add a Deepgram key in the settings (Speech) to dictate.");
      setSettingsOpen(true);
      return;
    }
    setMicError(null);
    typedBefore.current = words;
    setMic("starting");
    onDictating(true);
    const n = ++attempt.current;
    const current = () => n === attempt.current;
    const ac = new AbortController();
    opening.current = ac;
    try {
      const l = await Listening.open(
        speech,
        (rate) => core.listenParams(speech.model, speech.language, rate, speech.optOut),
        {
          onHeard: (h) => {
            if (current()) setWords(compose(typedBefore.current, h.finals, h.interim));
          },
          onPause: () => {
            if (current()) void finishListening();
          },
          onFailure: (message, r) => {
            if (!current()) return;
            listening.current = null;
            keepHeard(r);
            setMicError(message);
            endListening();
          },
        },
        ac.signal,
      );
      if (!current()) {
        // Cancelled, or the console closed, while the stream was opening.
        l.cancel();
        return;
      }
      listening.current = l;
      setMic("listening");
      micTimers.current = [
        setTimeout(() => void finishListening(), MAX_LISTEN_MS),
        setTimeout(() => {
          if (listening.current && !listening.current.speech) void finishListening();
        }, NO_SPEECH_MS),
      ];
    } catch (e) {
      if (!current()) return;
      setMicError(e instanceof Error ? e.message : String(e));
      endListening();
    } finally {
      if (opening.current === ac) opening.current = null;
    }
  };

  // Leaving the console closes the microphone.
  useEffect(
    () => () => {
      attempt.current++;
      opening.current?.abort();
      listening.current?.cancel();
      listening.current = null;
      micTimers.current.forEach(clearTimeout);
    },
    [],
  );

  const ask = async (text?: string) => {
    const w = (text ?? words).trim();
    if (!w || busy) return;
    const tid = next.current++;
    // Words from the box may have been dictated; a request made with a button was not.
    const dictated = text === undefined ? spoken.current : null;
    if (text === undefined) spoken.current = null;
    setTurns((ts) => [
      ...ts,
      { id: tid, words: w, via: null, status: "working", spoken: !!dictated, after: nextAfter(requests), keys: [] },
    ]);
    if (text === undefined) setWords("");
    setBusy(true);
    try {
      // What was heard and what is sent are logged before anything is done with them.
      let dictation: string | undefined;
      if (dictated) {
        dictation = await core.recordDictation(id, {
          provider: SPEECH_PROVIDER,
          model: dictated.model,
          host: dictated.host,
          params: dictated.params,
          request_ids: dictated.requestIds,
          segments: dictated.segments,
          words: w,
          audio_s: dictated.audioS,
          latency_ms: dictated.latencyMs,
        });
        await save();
      }
      const route = await core.route(w, selection);
      if (route.route === "listen") {
        onListen(route.which as Which);
        update(tid, {
          via: "local",
          status: "done",
          text: `Listening to ${route.which === "stack" ? "the processed audio" : route.which === "source" ? "the original" : "what was removed"}.`,
        });
      } else if (route.route === "undo" || route.route === "redo") {
        update(tid, { via: "local", status: "done", text: await undoOrRedo(route.route) });
      } else if (route.route === "recipe") {
        await showApplied(
          tid,
          await core.applyRecipe(id, route.name, w, dictation),
          "local",
          [],
          `The built-in ${route.name} recipe, measured on this recording.`,
        );
      } else if (route.route === "steps") {
        await showApplied(
          tid,
          await core.applyRouted(id, route.steps, w, dictation),
          "local",
          [],
          route.steps.map((s) => s.understood).join(" "),
        );
      } else {
        const config = loadConfig();
        update(tid, { via: "model", model: `${config.provider} · ${config.model}`, text: `Asking ${config.model}…` });
        // The core decides each round: use the answer, describe operations the
        // model asked to see, or ask once for a correction. Every exchange is logged.
        let body = await core.assistantRequest(id, config.model, history, w, selection);
        const rounds = { described: false, corrected: false };
        let link: { corrects?: string; describes?: string } = {};
        // Every exchange of this request, as the log names them.
        const keys: string[] = [];
        for (;;) {
          const { response, latencyMs, host } = await complete(config, body);
          const next = await core.nextRound(body, JSON.stringify(response), rounds);
          const exchange = await core.recordExchange(id, {
            provider: config.provider,
            model: config.model,
            host,
            request: body,
            response,
            latency_ms: latencyMs,
            problems: next.next === "correct" || next.next === "failed" ? next.problems : null,
            dictation,
            ...link,
          });
          keys.push(exchange);
          update(tid, { keys: [...keys] });
          if (next.next === "describe") {
            rounds.described = true;
            link = { describes: exchange };
            body = next.request;
            update(tid, { text: `Looking up ${next.ids.join(", ")}…` });
            continue;
          }
          if (next.next === "correct") {
            rounds.corrected = true;
            link = { corrects: exchange };
            body = next.request;
            continue;
          }
          if (next.next === "failed") {
            await showAnswered(tid, { status: "error", problems: next.problems, text: "The model's proposal could not be used." });
          } else if (next.proposal.steps.length === 0) {
            await showAnswered(tid, { status: "reply", text: next.proposal.text ?? "" });
          } else {
            await showApplied(
              tid,
              await core.applyProposal(id, next.proposal, w, config.model, config.provider, exchange, dictation),
              "model",
              keys,
              next.proposal.text ?? undefined,
            );
          }
          break;
        }
      }
    } catch (e) {
      update(tid, { status: "error", text: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  };

  const askRef = useRef(ask);
  askRef.current = ask;
  useEffect(() => {
    if (request) void askRef.current(request.words);
  }, [request]);

  const { actions, selected, onSelect, descriptors, onClone, canClone, onRate, output, brokenAtLine, inheritedFrom } = stack;
  const stackBusy = stack.busy || busy;
  const lastUndo = state.undo[state.undo.length - 1];
  const lastRedo = state.redo[state.redo.length - 1];
  const removed = state.entries.length - state.steps.length;

  const stepList = (placed: Placed[], meta = false) => (
    <ol className="steps">
      {placed.map(({ entry, n }) => {
        const s = stepOf(state, entry.step_id);
        return (
          s && (
            <StepRow
              key={s.step_id}
              n={n}
              step={s}
              entry={entry}
              actions={actions}
              busy={stackBusy}
              selected={selected === s.step_id}
              onSelect={onSelect}
              descriptors={descriptors}
              onClone={onClone}
              canClone={canClone}
              meta={meta}
            />
          )
        );
      })}
    </ol>
  );

  return (
    <div className="panel console" data-testid="console">
      <div className="panel-head">
        <h3>Stack</h3>
        <div className="head-tools">
          {actions && (
            <>
              <button
                className="small"
                disabled={stackBusy || !lastUndo}
                onClick={actions.undo}
                data-testid="undo"
                title={lastUndo ? `Undo ${changeWords(state, lastUndo)} (it stays in the log)` : "Nothing to undo"}
              >
                Undo
              </button>
              <button
                className="small"
                disabled={stackBusy || !lastRedo}
                onClick={actions.redo}
                data-testid="redo"
                title={lastRedo ? `Redo ${changeWords(state, lastRedo)}` : "Nothing to redo"}
              >
                Redo
              </button>
            </>
          )}
          {canClone && (
            <button className="small" onClick={() => onClone()} data-testid="clone-current" title="Clone the project as it is now">
              Clone current
            </button>
          )}
          <label className="check small-text" title="List only the requests that put steps on the stack">
            <input type="checkbox" checked={stepsOnly} onChange={(e) => setStepsOnly(e.target.checked)} data-testid="steps-only" /> Steps
            only
          </label>
          <button className="small" onClick={() => setSettingsOpen(true)} data-testid="assistant-settings" title="Provider, model and key">
            {cfg.model ? `${cfg.provider} · ${cfg.model}` : "choose a model"}
          </button>
        </div>
      </div>
      {brokenAtLine !== null && (
        <div className="problem-note" data-testid="stack-partial">
          Only what was recorded before line {brokenAtLine} of the log is listed; nothing after it can be verified.
        </div>
      )}
      {output && (
        <div className="output-length" data-testid="output-length">
          Output {output.duration.toFixed(3)} s (original {output.original.toFixed(3)} s). Removed stretches are shaded on the lanes and
          skipped when playing <i>Processed</i>.
        </div>
      )}
      <div className="turns" data-testid="stack" ref={turnsEl}>
        {rows.length === 0 && (
          <p className="muted small-text">
            No steps yet; the original is untouched. Say what you want, e.g. “clean this recording up”, “the hum is distracting”, “cut 3,100
            to 3,200 Hz by 12 dB”, or “compress the bangs here” with an area selected. Routine requests are handled here; the rest go to the
            model with words and analysis numbers only, never audio.
          </p>
        )}
        {rows.map((row) => {
          if (row.kind === "inherited")
            return (
              <div key={`inherited:${row.steps[0].entry.step_id}`} className="turn inherited" data-testid="inherited">
                <div className="via">Inherited from {inheritedFrom ?? "the project it was cloned from"}, where they were asked for</div>
                {stepList(row.steps, true)}
              </div>
            );
          if (row.kind === "session") {
            const t = row.turn;
            return (
              <div
                key={`session:${t.id}`}
                className={`turn ${t.status}`}
                data-testid="turn"
                data-status={t.status}
                data-via={t.via ?? ""}
                data-spoken={t.spoken ? "yes" : "no"}
              >
                <div className="you">{t.words}</div>
                {t.via && (
                  <div className="via">
                    {t.spoken ? "spoken · " : ""}
                    {t.via === "local" ? "handled here" : `via ${t.model}`}
                  </div>
                )}
                {t.text && (
                  <div className="reply" data-testid="turn-text">
                    {t.text}
                  </div>
                )}
                {t.problems && (
                  <ul className="problems" data-testid="turn-problems">
                    {t.problems.map((p, i) => (
                      <li key={i}>{p}</li>
                    ))}
                  </ul>
                )}
              </div>
            );
          }
          const r = row.request;
          const applied = row.steps.length > 0;
          const status = applied ? "applied" : r.problems?.length ? "error" : "reply";
          const answer = answerWords(r, applied);
          return (
            <div
              key={`${r.key}:${row.steps[0]?.n ?? ""}`}
              className={`turn ${status}`}
              data-testid="turn"
              data-status={status}
              data-via={r.via}
              data-spoken={r.spoken ? "yes" : "no"}
            >
              <div className="you">{r.words || (r.origin === "cli" ? "Applied from the command line" : "Applied")}</div>
              <div className="via">{viaWords(r)}</div>
              {answer && (
                <div className="reply" data-testid="turn-text">
                  {answer}
                </div>
              )}
              {r.problems && r.problems.length > 0 && (
                <ul className="problems" data-testid="turn-problems">
                  {r.problems.map((p, i) => (
                    <li key={i}>{p}</li>
                  ))}
                </ul>
              )}
              {!applied && r.proposed > 0 && (
                <div className="muted small-text">
                  What it proposed ({r.proposed} step{r.proposed === 1 ? "" : "s"}) was not applied.
                </div>
              )}
              {applied && stepList(row.steps)}
            </div>
          );
        })}
      </div>
      {state.steps.length > 0 && (
        <div className="stack-actions">
          {onRate && (
            <span className="rate" title="How good is the result? Recorded for learning.">
              Rate:
              {[1, 2, 3, 4, 5].map((n) => (
                <button key={n} className="small" onClick={() => onRate(n)} data-testid={`rate-${n}`}>
                  {n}
                </button>
              ))}
            </span>
          )}
          <span className="muted" data-testid="approval-note">
            Exporting approves the stack as it stands: {state.steps.length} step{state.steps.length === 1 ? "" : "s"}
            {removed > 0 ? `; ${removed} removed, left out` : ""}.
          </span>
        </div>
      )}
      <form
        className="ask"
        onSubmit={(e) => {
          e.preventDefault();
          if (mic === "off") void ask();
        }}
        onKeyDown={(e) => {
          // While listening, Enter stops (the words stay in the box to be checked) and Escape discards.
          if (mic === "off") return;
          if (e.key === "Enter") {
            e.preventDefault();
            if (mic === "listening") void finishListening();
          } else if (e.key === "Escape" && (mic === "starting" || mic === "listening")) {
            e.preventDefault();
            cancelListening();
          }
        }}
      >
        <input
          ref={inputEl}
          value={words}
          onChange={(e) => {
            setWords(e.target.value);
            if (!e.target.value.trim()) spoken.current = null;
          }}
          placeholder={unavailable ?? (mic === "off" ? "What should happen?" : "Listening…")}
          disabled={!!unavailable || busy}
          readOnly={mic !== "off"}
          data-testid="console-input"
        />
        <button
          type="button"
          className={`mic ${mic}`}
          onClick={() => {
            if (mic === "off") void startListening();
            else if (mic === "listening") void finishListening();
          }}
          disabled={!!unavailable || busy || mic === "starting" || mic === "finishing"}
          aria-pressed={mic !== "off"}
          data-testid="console-mic"
          data-state={mic}
          title={
            mic === "off"
              ? "Speak your request (Deepgram). The words fill the box; press Enter to send them."
              : "Stop listening (Enter). Escape discards what was heard."
          }
        >
          {mic === "off" ? "Speak" : mic === "starting" ? "Opening…" : mic === "listening" ? "Stop" : "…"}
        </button>
        <button type="submit" disabled={!!unavailable || busy || !words.trim() || mic !== "off"} data-testid="console-send">
          {busy ? "…" : "Send"}
        </button>
      </form>
      {mic === "listening" && (
        <div className="mic-status small-text" data-testid="mic-status">
          Listening. The words appear as they are recognised; Enter or Stop when done, Escape to discard.
        </div>
      )}
      {micError && mic === "off" && (
        <div className="mic-error small-text bad" data-testid="mic-error">
          {micError}
        </div>
      )}
      {settingsOpen && (
        <Settings
          initial={cfg}
          onClose={() => {
            setSettingsOpen(false);
            setConfigVersion((v) => v + 1);
          }}
        />
      )}
    </div>
  );
}
