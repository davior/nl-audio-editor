// Dictation for the recogniser: the microphone's first channel as 16-bit
// little-endian samples, posted about ten times a second. This is not the
// recording path: dictation is not evidence and is never stored.
class NlaeDictation extends AudioWorkletProcessor {
  constructor() {
    super();
    this.size = Math.round(sampleRate / 10);
    this.fresh();
  }

  fresh() {
    this.buffer = new ArrayBuffer(this.size * 2);
    this.view = new DataView(this.buffer);
    this.n = 0;
  }

  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (ch) {
      for (let i = 0; i < ch.length; i++) {
        const s = Math.max(-1, Math.min(1, ch[i]));
        this.view.setInt16(2 * this.n, s < 0 ? Math.round(s * 32768) : Math.round(s * 32767), true);
        if (++this.n === this.size) {
          this.port.postMessage(this.buffer, [this.buffer]);
          this.fresh();
        }
      }
    }
    return true;
  }
}
registerProcessor("nlae-dictation", NlaeDictation);
