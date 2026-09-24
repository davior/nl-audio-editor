// 32-bit float WAV (format 3), the lossless container for in-app recordings.
export function encodeWavF32(channels: Float32Array[], sampleRate: number): Uint8Array {
  const nch = channels.length;
  const len = channels[0]?.length ?? 0;
  const dataLen = len * nch * 4;
  const buf = new ArrayBuffer(58 + dataLen);
  const v = new DataView(buf);
  let o = 0;
  const str = (s: string) => {
    for (let i = 0; i < s.length; i++) v.setUint8(o++, s.charCodeAt(i));
  };
  const u32 = (x: number) => {
    v.setUint32(o, x, true);
    o += 4;
  };
  const u16 = (x: number) => {
    v.setUint16(o, x, true);
    o += 2;
  };
  str("RIFF");
  u32(50 + dataLen);
  str("WAVE");
  str("fmt ");
  u32(18);
  u16(3);
  u16(nch);
  u32(sampleRate);
  u32(sampleRate * nch * 4);
  u16(nch * 4);
  u16(32);
  u16(0);
  str("fact");
  u32(4);
  u32(len);
  str("data");
  u32(dataLen);
  for (let i = 0; i < len; i++) {
    for (let c = 0; c < nch; c++) {
      v.setFloat32(o, channels[c][i], true);
      o += 4;
    }
  }
  return new Uint8Array(buf);
}
