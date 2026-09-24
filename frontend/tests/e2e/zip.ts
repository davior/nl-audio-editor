// Just enough ZIP to edit a stored entry the way a careful forger would:
// same length, both CRCs fixed, so the archive itself is valid.

const TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

export function crc32(b: Uint8Array): number {
  let c = 0xffffffff;
  for (const x of b) c = TABLE[(c ^ x) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** Replace a stored entry's bytes (same length) and fix its CRC in the local header and the central directory. */
export function patchStoredEntry(zip: Buffer, name: string, edit: (data: Buffer) => Buffer): Buffer {
  const out = Buffer.from(zip);
  let eocd = -1;
  for (let i = out.length - 22; i >= 0; i--) {
    if (out.readUInt32LE(i) === 0x06054b50) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) throw new Error("no end-of-directory record");
  const count = out.readUInt16LE(eocd + 10);
  let p = out.readUInt32LE(eocd + 16);
  for (let k = 0; k < count; k++) {
    if (out.readUInt32LE(p) !== 0x02014b50) throw new Error("bad central directory");
    const size = out.readUInt32LE(p + 20);
    const nlen = out.readUInt16LE(p + 28);
    const xlen = out.readUInt16LE(p + 30);
    const clen = out.readUInt16LE(p + 32);
    const local = out.readUInt32LE(p + 42);
    const entry = out.toString("utf8", p + 46, p + 46 + nlen);
    if (entry === name) {
      const start = local + 30 + out.readUInt16LE(local + 26) + out.readUInt16LE(local + 28);
      const data = edit(Buffer.from(out.subarray(start, start + size)));
      if (data.length !== size) throw new Error("only same-length edits are supported");
      data.copy(out, start);
      const crc = crc32(data);
      out.writeUInt32LE(crc, local + 14);
      out.writeUInt32LE(crc, p + 16);
      return out;
    }
    p += 46 + nlen + xlen + clen;
  }
  throw new Error(`no entry ${name}`);
}
