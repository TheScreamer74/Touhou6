// AudioWorkletGlobalScope omits the Encoding API in Chromium. wasm-bindgen
// creates its UTF-8 codecs while evaluating the module, before initSync runs.
// Import this file BEFORE the wasm-bindgen module to provide those codecs.
if (typeof globalThis.TextEncoder === "undefined") {
  globalThis.TextEncoder = class {
    encode(input = "") {
      const bytes = [];
      for (const character of String(input)) {
        let code = character.codePointAt(0);
        if (code >= 0xd800 && code <= 0xdfff) code = 0xfffd;
        if (code < 0x80) bytes.push(code);
        else if (code < 0x800) bytes.push(0xc0 | code >> 6, 0x80 | code & 0x3f);
        else if (code < 0x10000) bytes.push(0xe0 | code >> 12, 0x80 | code >> 6 & 0x3f, 0x80 | code & 0x3f);
        else bytes.push(0xf0 | code >> 18, 0x80 | code >> 12 & 0x3f, 0x80 | code >> 6 & 0x3f, 0x80 | code & 0x3f);
      }
      return new Uint8Array(bytes);
    }
    encodeInto(input, destination) {
      let read = 0, written = 0;
      for (const character of String(input)) {
        const bytes = this.encode(character);
        if (written + bytes.length > destination.length) break;
        destination.set(bytes, written);
        read += character.length;
        written += bytes.length;
      }
      return { read, written };
    }
  };
}
if (typeof globalThis.TextDecoder === "undefined") {
  globalThis.TextDecoder = class {
    constructor(label = "utf-8", options = {}) {
      if (!/^(utf-8|utf8)$/i.test(label)) throw new RangeError("Only UTF-8 is supported in the MIDI audio thread");
      this.fatal = Boolean(options.fatal);
      this.ignoreBOM = Boolean(options.ignoreBOM);
    }
    decode(input = new Uint8Array()) {
      const bytes = ArrayBuffer.isView(input)
        ? new Uint8Array(input.buffer, input.byteOffset, input.byteLength) : new Uint8Array(input);
      let text = "", firstCharacter = true;
      for (let i = 0; i < bytes.length;) {
        const first = bytes[i++];
        let code = first, count = 0, valid = true;
        if (first >= 0xc2 && first <= 0xdf) { code = first & 0x1f; count = 1; }
        else if (first >= 0xe0 && first <= 0xef) { code = first & 0x0f; count = 2; }
        else if (first >= 0xf0 && first <= 0xf4) { code = first & 7; count = 3; }
        else if (first >= 0x80) valid = false;
        for (let n = 0; n < count; n++) {
          const lower = n === 0 && first === 0xe0 ? 0xa0 : n === 0 && first === 0xf0 ? 0x90 : 0x80;
          const upper = n === 0 && first === 0xed ? 0x9f : n === 0 && first === 0xf4 ? 0x8f : 0xbf;
          if (i >= bytes.length || bytes[i] < lower || bytes[i] > upper) { valid = false; break; }
          code = code << 6 | bytes[i++] & 0x3f;
        }
        if (!valid) {
          if (this.fatal) throw new TypeError("Invalid UTF-8 in MIDI audio thread");
          code = 0xfffd;
        }
        if (this.ignoreBOM || !firstCharacter || code !== 0xfeff) text += String.fromCodePoint(code);
        firstCharacter = false;
      }
      return text;
    }
  };
}

// The bootstrap imports the same versioned wasm-bindgen module as the game,
// then passes its compiled WebAssembly.Module into an isolated audio instance.
export function registerMidiProcessor(initSync, MidiSynth) {
  class Th06MidiProcessor extends AudioWorkletProcessor {
    constructor(options) {
      super();
      initSync({ module: options.processorOptions.module });
      this.synth = null;
      this.playing = false;
      this.frames = 0;
      this.port.onmessage = event => {
        try {
          const message = event.data;
          switch (message.type) {
            case "bank":
              this.synth?.free();
              this.synth = new MidiSynth(message.bytes, sampleRate);
              this.playing = false;
              this.port.postMessage({ type: "bank-ready" });
              break;
            case "play":
              if (!this.synth) throw new Error("MIDI SoundFont is not loaded");
              this.synth.play(message.bytes);
              this.playing = true;
              this.frames = 0;
              this.port.postMessage({ type: "playing", name: message.name });
              break;
            case "fade": this.synth?.fade(message.seconds); break;
            case "stop": this.synth?.stop(); this.playing = false; break;
          }
        } catch (error) {
          this.playing = false;
          this.port.postMessage({ type: "error", message: String(error) });
        }
      };
    }
    process(_inputs, outputs) {
      const output = outputs[0];
      if (!output || output.length < 2) return true;
      if (this.synth && this.playing) {
        this.synth.render(output[0], output[1]);
        this.frames += output[0].length;
      } else for (const channel of output) channel.fill(0);
      return true;
    }
  }
  registerProcessor("th06-midi", Th06MidiProcessor);
}
