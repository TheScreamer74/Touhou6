//! BGM and sound effects. Native uses rodio (streams BGM from disk).
//! Web uses the Web Audio API with in-memory PCM buffers (no filesystem on
//! the browser — every wav is uploaded by the player and held in memory).
//! Both fail soft: with no output the game runs silent.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::HashMap;
    use std::io::{BufReader, Cursor};
    use std::sync::Arc;

    use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

    pub struct Audio {
        _stream: OutputStream,
        handle: OutputStreamHandle,
        bgm: Option<Sink>,
        bgm_name: Option<String>,
        sfx: HashMap<String, Arc<[u8]>>,
        bgm_data: HashMap<String, Arc<[u8]>>,
    }

    impl Audio {
        pub fn new() -> Option<Self> {
            let (stream, handle) = OutputStream::try_default().ok()?;
            Some(Self {
                _stream: stream,
                handle,
                bgm: None,
                bgm_name: None,
                sfx: HashMap::new(),
                bgm_data: HashMap::new(),
            })
        }

        pub fn register_sfx(&mut self, name: &str, wav: Vec<u8>) {
            self.sfx.insert(name.to_string(), wav.into());
        }

        /// Register a BGM track by name (basename, e.g. "th06_03.wav") so it
        /// can be started later by name — matching the web backend's API.
        pub fn register_bgm(&mut self, name: &str, wav: Vec<u8>) {
            self.bgm_data.insert(name.to_string(), wav.into());
        }

        pub fn play_sfx(&self, name: &str) {
            if let Some(data) = self.sfx.get(name) {
                let cursor = Cursor::new(data.clone());
                if let Ok(source) = Decoder::new(cursor) {
                    let _ = self.handle.play_raw(source.convert_samples());
                }
            }
        }

        /// Play a registered BGM track on infinite loop. No-op when already
        /// playing it.
        pub fn play_bgm(&mut self, name: &str) {
            if self.bgm_name.as_deref() == Some(name) {
                return;
            }
            if let Some(old) = self.bgm.take() {
                old.stop();
            }
            let Some(data) = self.bgm_data.get(name) else { return };
            let cursor = Cursor::new(data.clone());
            let Ok(source) = Decoder::new(BufReader::new(cursor)) else { return };
            let Ok(sink) = Sink::try_new(&self.handle) else { return };
            sink.set_volume(0.6);
            sink.append(source.repeat_infinite());
            self.bgm = Some(sink);
            self.bgm_name = Some(name.to_string());
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::Audio;

#[cfg(target_arch = "wasm32")]
mod web {
    use std::collections::HashMap;

    use web_sys::{AudioBuffer, AudioBufferSourceNode, AudioContext, GainNode};

    /// Decoded PCM held as an AudioBuffer, ready to (re)play cheaply.
    pub struct Audio {
        ctx: AudioContext,
        bgm_gain: GainNode,
        sfx: HashMap<String, AudioBuffer>,
        bgm: HashMap<String, AudioBuffer>,
        bgm_node: Option<AudioBufferSourceNode>,
        bgm_name: Option<String>,
    }

    impl Audio {
        pub fn new() -> Option<Self> {
            let ctx = AudioContext::new().ok()?;
            // Built during the file-select user gesture, so this is allowed.
            let _ = ctx.resume();
            let bgm_gain = ctx.create_gain().ok()?;
            bgm_gain.gain().set_value(0.6);
            let _ = bgm_gain.connect_with_audio_node(&ctx.destination());
            Some(Self {
                ctx,
                bgm_gain,
                sfx: HashMap::new(),
                bgm: HashMap::new(),
                bgm_node: None,
                bgm_name: None,
            })
        }

        pub fn register_sfx(&mut self, name: &str, wav: Vec<u8>) {
            if let Some(buf) = self.decode_wav(&wav) {
                self.sfx.insert(name.to_string(), buf);
            }
        }

        /// Web has no filesystem; BGM wavs are registered by name from the
        /// player's uploaded `bgm/` folder.
        pub fn register_bgm(&mut self, name: &str, wav: Vec<u8>) {
            if let Some(buf) = self.decode_wav(&wav) {
                self.bgm.insert(name.to_string(), buf);
            }
        }

        pub fn play_sfx(&self, name: &str) {
            let Some(buf) = self.sfx.get(name) else { return };
            if let Ok(src) = self.ctx.create_buffer_source() {
                src.set_buffer(Some(buf));
                let _ = src.connect_with_audio_node(&self.ctx.destination());
                let _ = src.start();
            }
        }

        /// Play a registered BGM track on infinite loop. No-op when already
        /// playing it. `name` is the wav basename (e.g. "th06_03.wav").
        pub fn play_bgm(&mut self, name: &str) {
            if self.bgm_name.as_deref() == Some(name) {
                return;
            }
            if let Some(old) = self.bgm_node.take() {
                // web-sys marks both stop() forms deprecated; it is still the
                // correct way to halt a source node.
                #[allow(deprecated)]
                let _ = old.stop();
            }
            let Some(buf) = self.bgm.get(name) else { return };
            let Ok(src) = self.ctx.create_buffer_source() else { return };
            src.set_buffer(Some(buf));
            src.set_loop(true);
            let _ = src.connect_with_audio_node(&self.bgm_gain);
            let _ = src.start();
            self.bgm_node = Some(src);
            self.bgm_name = Some(name.to_string());
        }

        /// Browsers suspend AudioContext until a user gesture; call this from
        /// the first key/click so audio can start.
        pub fn resume(&self) {
            let _ = self.ctx.resume();
        }

        /// Parse a PCM WAV (8/16-bit) into an AudioBuffer. The th06 wavs are
        /// uncompressed PCM, so this avoids the async `decodeAudioData` path.
        fn decode_wav(&self, data: &[u8]) -> Option<AudioBuffer> {
            let (channels, sample_rate, samples) = parse_pcm_wav(data)?;
            let frames = samples.len() / channels as usize;
            if frames == 0 {
                return None;
            }
            let buf = self
                .ctx
                .create_buffer(channels, frames as u32, sample_rate as f32)
                .ok()?;
            for ch in 0..channels as usize {
                let mut plane: Vec<f32> = Vec::with_capacity(frames);
                for f in 0..frames {
                    plane.push(samples[f * channels as usize + ch]);
                }
                buf.copy_to_channel(&mut plane, ch as i32).ok()?;
            }
            Some(buf)
        }
    }

    /// Minimal RIFF/WAVE PCM reader → interleaved f32 samples.
    fn parse_pcm_wav(d: &[u8]) -> Option<(u32, u32, Vec<f32>)> {
        if d.len() < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
            return None;
        }
        let mut pos = 12;
        let mut channels = 0u32;
        let mut sample_rate = 0u32;
        let mut bits = 0u16;
        let mut data: Option<&[u8]> = None;
        while pos + 8 <= d.len() {
            let id = &d[pos..pos + 4];
            let size = u32::from_le_bytes([d[pos + 4], d[pos + 5], d[pos + 6], d[pos + 7]]) as usize;
            let body = pos + 8;
            if body + size > d.len() {
                break;
            }
            match id {
                b"fmt " if size >= 16 => {
                    channels = u16::from_le_bytes([d[body + 2], d[body + 3]]) as u32;
                    sample_rate = u32::from_le_bytes([
                        d[body + 4], d[body + 5], d[body + 6], d[body + 7],
                    ]);
                    bits = u16::from_le_bytes([d[body + 14], d[body + 15]]);
                }
                b"data" => data = Some(&d[body..body + size]),
                _ => {}
            }
            pos = body + size + (size & 1); // chunks are word-aligned
        }
        let data = data?;
        if channels == 0 {
            return None;
        }
        let samples = match bits {
            16 => data
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
                .collect(),
            8 => data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
            _ => return None,
        };
        Some((channels, sample_rate, samples))
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::Audio;
