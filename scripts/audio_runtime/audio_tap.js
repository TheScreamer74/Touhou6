// Init script: tap every connection to AudioContext.destination through a recorder
// AudioWorklet. Per 128-frame block it records RMS, peak and max |x[n]-x[n-1]|
// (across block edges) for L/R; optional raw PCM capture windows for offline checks.
(() => {
  const RECORDER = `
  class Tap extends AudioWorkletProcessor {
    constructor() {
      super(); this.prev = [0, 0]; this.stats = []; this.pcm = null;
      this.port.onmessage = e => {
        if (e.data === "pcm-on") this.pcm = [[], []];
        if (e.data === "pcm-off") { const p = this.pcm; this.pcm = null; this.port.postMessage({ pcm: p }); }
      };
    }
    process(inputs, outputs) {
      const inp = inputs[0], out = outputs[0];
      const L = inp[0] || new Float32Array(128), R = inp[1] || L;
      const s = [currentFrame, 0, 0, 0, 0, 0, 0, inp.length];
      for (let c = 0; c < 2; c++) {
        const x = c ? R : L; let sq = 0, pk = 0, md = 0, p = this.prev[c];
        for (let i = 0; i < x.length; i++) {
          const v = x[i]; sq += v * v; const a = Math.abs(v); if (a > pk) pk = a;
          const d = Math.abs(v - p); if (d > md) md = d; p = v;
        }
        this.prev[c] = p; s[1 + c * 3] = Math.sqrt(sq / x.length); s[2 + c * 3] = pk; s[3 + c * 3] = md;
        if (out[c]) out[c].set(x);
        if (this.pcm) this.pcm[c].push(x.slice());
      }
      this.stats.push(s);
      if (this.stats.length >= 64) { this.port.postMessage({ stats: this.stats }); this.stats = []; }
      return true;
    }
  }
  registerProcessor("th06-tap", Tap);`;
  const url = URL.createObjectURL(new Blob([RECORDER], { type: "text/javascript" }));
  const taps = new WeakMap();
  window.__tap = { stats: [], ctx: null, pcm: null, events: [] };
  const origConnect = AudioNode.prototype.connect;
  function busFor(ctx) {
    let t = taps.get(ctx);
    if (t) return t.bus;
    const bus = ctx.createGain();
    t = { bus }; taps.set(ctx, t); window.__tap.ctx = ctx;
    ctx.audioWorklet.addModule(url).then(() => {
      const node = new AudioWorkletNode(ctx, "th06-tap", { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [2], channelCount: 2, channelCountMode: "explicit" });
      node.port.onmessage = e => {
        if (e.data.stats) for (const s of e.data.stats) window.__tap.stats.push(s);
        if (e.data.pcm) window.__tap.pcm = e.data.pcm;
      };
      origConnect.call(bus, node); origConnect.call(node, ctx.destination);
      t.node = node; window.__tap.ready = ctx.sampleRate;
    }).catch(err => { window.__tap.error = String(err); origConnect.call(bus, ctx.destination); });
    return bus;
  }
  AudioNode.prototype.connect = function (target, ...rest) {
    if (target instanceof AudioDestinationNode && target.context) return origConnect.call(this, busFor(target.context), ...rest);
    return origConnect.call(this, target, ...rest);
  };
  window.__tapMark = label => window.__tap.events.push({ label, frame: window.__tap.ctx ? Math.round(window.__tap.ctx.currentTime * window.__tap.ctx.sampleRate) : null, t: performance.now(), state: window.__tap.ctx && window.__tap.ctx.state });
  window.__tapPcm = on => { const t = window.__tap.ctx && taps.get(window.__tap.ctx); if (t && t.node) t.node.port.postMessage(on ? "pcm-on" : "pcm-off"); };
})();
