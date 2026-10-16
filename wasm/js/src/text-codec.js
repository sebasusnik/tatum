// TextDecoder and TextEncoder for the AudioWorklet scope, which has neither.
// wasm-bindgen's glue makes one of each the moment it is imported, so this
// module is imported before the glue: ES modules evaluate in import order.
// UTF-8 only, which is all the glue asks for.

if (typeof globalThis.TextDecoder === "undefined") {
  globalThis.TextDecoder = class TextDecoder {
    constructor(label = "utf-8", options = {}) {
      this.encoding = "utf-8";
      this.fatal = !!options.fatal;
      this.ignoreBOM = !!options.ignoreBOM;
    }

    decode(input) {
      if (input === undefined) return "";
      const bytes = input instanceof Uint8Array ? input : new Uint8Array(input.buffer ?? input);
      let out = "";
      let i = 0;
      if (!this.ignoreBOM && bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) i = 3;
      while (i < bytes.length) {
        const b = bytes[i++];
        let code;
        if (b < 0x80) {
          code = b;
        } else if (b >= 0xc0 && b < 0xe0) {
          code = ((b & 0x1f) << 6) | (bytes[i++] & 0x3f);
        } else if (b >= 0xe0 && b < 0xf0) {
          code = ((b & 0x0f) << 12) | ((bytes[i++] & 0x3f) << 6) | (bytes[i++] & 0x3f);
        } else if (b >= 0xf0) {
          code = ((b & 0x07) << 18) | ((bytes[i++] & 0x3f) << 12) | ((bytes[i++] & 0x3f) << 6) | (bytes[i++] & 0x3f);
        } else {
          if (this.fatal) throw new TypeError("invalid UTF-8");
          code = 0xfffd;
        }
        out += String.fromCodePoint(code);
      }
      return out;
    }
  };
}

if (typeof globalThis.TextEncoder === "undefined") {
  globalThis.TextEncoder = class TextEncoder {
    constructor() {
      this.encoding = "utf-8";
    }

    encode(text = "") {
      const out = [];
      for (const ch of text) {
        const code = ch.codePointAt(0);
        if (code < 0x80) {
          out.push(code);
        } else if (code < 0x800) {
          out.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
        } else if (code < 0x10000) {
          out.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
        } else {
          out.push(0xf0 | (code >> 18), 0x80 | ((code >> 12) & 0x3f), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
        }
      }
      return Uint8Array.from(out);
    }

    // The glue prefers this: write into the buffer it hands over, as far as
    // it fits, and say how much of the text went in.
    encodeInto(text, dest) {
      let read = 0;
      let written = 0;
      for (const ch of text) {
        const bytes = this.encode(ch);
        if (written + bytes.length > dest.length) break;
        dest.set(bytes, written);
        written += bytes.length;
        read += ch.length;
      }
      return { read, written };
    }
  };
}
