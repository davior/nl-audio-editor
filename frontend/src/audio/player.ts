// Playback at the file's native sample rate; the browser resamples only for output.
import type { Pcm } from "../core/types";
import type { TimeMap } from "./timemap";

export class Player {
  private ctx: AudioContext | null = null;
  private buffer: AudioBuffer | null = null;
  private node: AudioBufferSourceNode | null = null;
  private startedAt = 0;
  private from = 0;
  private to = 0;
  private looping = false;
  /** Where the loaded audio starts on the recording's time line (a preview window). */
  private offset = 0;
  /** For the stack's output with time edits: output time ↔ the recording's time line. */
  private map: TimeMap | null = null;
  playing = false;
  onEnded: (() => void) | null = null;

  private context(): AudioContext {
    if (!this.ctx) this.ctx = new AudioContext();
    return this.ctx;
  }

  /**
   * Load audio that starts `offset` seconds into the recording (0 for the whole
   * recording), or the output of time edits with `map` relating it to the recording.
   */
  load(pcm: Pcm, offset = 0, map: TimeMap | null = null): void {
    this.stop();
    this.offset = offset;
    this.map = map;
    this.from = 0;
    const ctx = this.context();
    const len = pcm.channels[0]?.length ?? 0;
    const b = ctx.createBuffer(pcm.channels.length, Math.max(1, len), pcm.sampleRate);
    pcm.channels.forEach((c, i) => b.copyToChannel(c as Float32Array<ArrayBuffer>, i));
    this.buffer = b;
  }

  get duration(): number {
    return this.buffer?.duration ?? 0;
  }

  /** Play from `from` (to `to`) seconds on the recording's time line. */
  async play(from: number, to?: number, loop = false): Promise<void> {
    if (!this.buffer) return;
    this.stop();
    const ctx = this.context();
    await ctx.resume();
    const n = ctx.createBufferSource();
    n.buffer = this.buffer;
    const inBuffer = (t: number) => (this.map ? this.map.toOutput(t) : t - this.offset);
    const at = Math.max(0, Math.min(inBuffer(from), this.buffer.duration));
    this.from = at;
    this.to = Math.min(to === undefined ? this.buffer.duration : Math.max(at, inBuffer(to)), this.buffer.duration);
    this.looping = loop;
    n.loop = loop;
    if (loop) {
      n.loopStart = this.from;
      n.loopEnd = this.to;
    }
    n.connect(ctx.destination);
    n.onended = () => {
      if (this.node === n) {
        this.playing = false;
        this.node = null;
        this.onEnded?.();
      }
    };
    if (loop) n.start(0, this.from);
    else n.start(0, this.from, Math.max(0, this.to - this.from));
    this.node = n;
    this.startedAt = ctx.currentTime;
    this.playing = true;
  }

  stop(): void {
    if (this.node) {
      const n = this.node;
      this.node = null;
      try {
        n.stop();
      } catch {
        /* already stopped */
      }
    }
    this.playing = false;
  }

  /** Current position in seconds on the recording's time line. */
  position(): number {
    let buffer: number;
    if (!this.playing || !this.ctx) buffer = this.from;
    else {
      const t = this.ctx.currentTime - this.startedAt;
      const span = Math.max(1e-6, this.to - this.from);
      buffer = this.looping ? this.from + (t % span) : Math.min(this.from + t, this.to);
    }
    return this.map ? this.map.toOriginal(buffer) : this.offset + buffer;
  }
}
