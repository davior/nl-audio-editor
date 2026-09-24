// Playback at the file's native sample rate; the browser resamples only for output.
import type { Pcm } from "../core/types";

export class Player {
  private ctx: AudioContext | null = null;
  private buffer: AudioBuffer | null = null;
  private node: AudioBufferSourceNode | null = null;
  private startedAt = 0;
  private from = 0;
  private to = 0;
  private looping = false;
  playing = false;
  onEnded: (() => void) | null = null;

  private context(): AudioContext {
    if (!this.ctx) this.ctx = new AudioContext();
    return this.ctx;
  }

  load(pcm: Pcm): void {
    this.stop();
    const ctx = this.context();
    const len = pcm.channels[0]?.length ?? 0;
    const b = ctx.createBuffer(pcm.channels.length, Math.max(1, len), pcm.sampleRate);
    pcm.channels.forEach((c, i) => b.copyToChannel(c as Float32Array<ArrayBuffer>, i));
    this.buffer = b;
  }

  get duration(): number {
    return this.buffer?.duration ?? 0;
  }

  async play(from: number, to?: number, loop = false): Promise<void> {
    if (!this.buffer) return;
    this.stop();
    const ctx = this.context();
    await ctx.resume();
    const n = ctx.createBufferSource();
    n.buffer = this.buffer;
    this.from = Math.max(0, Math.min(from, this.buffer.duration));
    this.to = Math.min(to ?? this.buffer.duration, this.buffer.duration);
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

  /** Current position in seconds. */
  position(): number {
    if (!this.playing || !this.ctx) return this.from;
    const t = this.ctx.currentTime - this.startedAt;
    if (!this.looping) return Math.min(this.from + t, this.to);
    const span = Math.max(1e-6, this.to - this.from);
    return this.from + (t % span);
  }
}
