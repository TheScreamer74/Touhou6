// Pass through real PCM while measuring progress on the audio thread.
class MidiMeter extends AudioWorkletProcessor {
  constructor() { super(); this.frames = 0; this.energy = 0; }
  process(inputs, outputs) {
    const input = inputs[0], output = outputs[0];
    if (input?.length) {
      for (let channel = 0; channel < output.length; channel++) output[channel].set(input[channel] || input[0]);
      for (const value of input[0]) this.energy += value * value;
      this.frames += input[0].length;
      if (this.frames % 12800 === 0) this.port.postMessage({ frames: this.frames, energy: this.energy, time: currentTime });
    }
    return true;
  }
}
registerProcessor("th06-midi-meter", MidiMeter);
