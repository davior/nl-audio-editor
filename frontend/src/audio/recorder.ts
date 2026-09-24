// In-app recording with the browser's own processing switched off. What the
// browser actually applied is captured and logged with the recording.
import { encodeWavF32 } from "./wav";

export const CONSTRAINTS: MediaTrackConstraints = {
  echoCancellation: false,
  noiseSuppression: false,
  autoGainControl: false,
  channelCount: 1,
};

export interface Capture {
  requested: MediaTrackConstraints;
  applied: MediaTrackSettings;
  device_label: string;
  context_sample_rate: number;
  started: string;
  duration_s: number;
}

export class Recorder {
  private ctx: AudioContext | null = null;
  private stream: MediaStream | null = null;
  private node: AudioWorkletNode | null = null;
  private chunks: Float32Array[][] = [];
  private started = "";

  async start(): Promise<void> {
    this.stream = await navigator.mediaDevices.getUserMedia({ audio: CONSTRAINTS });
    this.ctx = new AudioContext();
    await this.ctx.audioWorklet.addModule("/capture-worklet.js");
    const src = this.ctx.createMediaStreamSource(this.stream);
    this.node = new AudioWorkletNode(this.ctx, "nlae-capture");
    this.chunks = [];
    this.node.port.onmessage = (e: MessageEvent<Float32Array[]>) => this.chunks.push(e.data);
    const silent = this.ctx.createGain();
    silent.gain.value = 0;
    src.connect(this.node);
    this.node.connect(silent).connect(this.ctx.destination);
    this.started = new Date().toISOString();
  }

  async stop(): Promise<{ wav: Uint8Array; capture: Capture }> {
    const track = this.stream?.getAudioTracks()[0];
    const applied = track?.getSettings() ?? {};
    const label = track?.label ?? "";
    this.stream?.getTracks().forEach((t) => t.stop());
    this.node?.disconnect();
    const rate = this.ctx?.sampleRate ?? 48000;
    await this.ctx?.close();
    const nch = this.chunks[0]?.length ?? 1;
    const total = this.chunks.reduce((n, c) => n + c[0].length, 0);
    const channels = Array.from({ length: nch }, () => new Float32Array(total));
    let at = 0;
    for (const c of this.chunks) {
      for (let ch = 0; ch < nch; ch++) channels[ch].set(c[ch], at);
      at += c[0].length;
    }
    const capture: Capture = {
      requested: CONSTRAINTS,
      applied,
      device_label: label,
      context_sample_rate: rate,
      started: this.started,
      duration_s: total / rate,
    };
    return { wav: encodeWavF32(channels, rate), capture };
  }
}
