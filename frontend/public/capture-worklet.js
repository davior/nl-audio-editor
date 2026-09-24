// Captures raw input blocks and posts them to the page. No processing.
class NlaeCapture extends AudioWorkletProcessor {
  process(inputs) {
    const input = inputs[0];
    if (input && input.length > 0) {
      this.port.postMessage(input.map((ch) => ch.slice(0)));
    }
    return true;
  }
}
registerProcessor("nlae-capture", NlaeCapture);
