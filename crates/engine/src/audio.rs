//! BGM and sound effects. Native uses rodio (streams BGM from disk).
//! Web uses the Web Audio API with in-memory PCM buffers (no filesystem on
//! the browser — every wav is uploaded by the player and held in memory).
//! Both fail soft: with no output the game runs silent.

/// CStreamingSound::UpdateFadeOut uses DirectSound hundredths of a dB.
#[derive(Default)]
struct Fade {
    remaining: i32,
    total: i32,
}
impl Fade {
    fn start(&mut self, seconds: f32) {
        self.total = (seconds * 60.0) as i32;
        self.remaining = self.total.max(1);
        self.total = self.remaining;
    }
    fn tick(&mut self) -> Option<f32> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        if self.remaining == 0 {
            return Some(0.0);
        }
        let db100 = self.remaining * 5000 / self.total - 5000;
        Some(10.0_f32.powf(db100 as f32 / 2000.0))
    }
}

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
        /// Per-track loop points (loop_start, loop_end) in sample-frames, from
        /// the track's `.pos` file — mirrors the decomp's LoadPos.
        bgm_loop: HashMap<String, (u32, u32)>,
        fade: super::Fade,
    }

    /// A BGM source that plays the intro once then loops the `[start, end)`
    /// frame range forever — the decomp's PlayBGM(TRUE) behaviour (SoundPlayer
    /// loops between m_loopStartPoint/m_loopEndPoint). Without a `.pos` file the
    /// whole track is the loop (start 0, end = length), matching PlayBGM(FALSE).
    pub(crate) struct LoopSource {
        samples: Vec<f32>,
        channels: u16,
        rate: u32,
        pos: usize,
        loop_start: usize, // sample index (frame * channels)
        loop_end: usize,
    }

    impl LoopSource {
        #[cfg(test)]
        pub(crate) fn for_test(
            samples: Vec<f32>,
            channels: u16,
            rate: u32,
            loop_start: usize,
            loop_end: usize,
        ) -> Self {
            LoopSource {
                samples,
                channels,
                rate,
                pos: 0,
                loop_start,
                loop_end,
            }
        }
    }

    impl Iterator for LoopSource {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            if self.pos >= self.loop_end {
                self.pos = self.loop_start;
            }
            let s = self.samples[self.pos];
            self.pos += 1;
            Some(s)
        }
    }

    impl Source for LoopSource {
        fn current_frame_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> u16 {
            self.channels
        }
        fn sample_rate(&self) -> u32 {
            self.rate
        }
        fn total_duration(&self) -> Option<std::time::Duration> {
            None // infinite
        }
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
                bgm_loop: HashMap::new(),
                fade: super::Fade::default(),
            })
        }

        pub fn register_sfx(&mut self, name: &str, wav: Vec<u8>) {
            self.sfx.insert(name.to_string(), wav.into());
        }

        /// Register a BGM track by name (basename, e.g. "th06_03.wav") so it
        /// can be started later by name — matching the web backend's API.
        /// `loop_frames` = (loop_start, loop_end) in sample-frames from the
        /// track's `.pos` file, or None to loop the whole track.
        pub fn register_bgm(&mut self, name: &str, wav: Vec<u8>, loop_frames: Option<(u32, u32)>) {
            self.bgm_data.insert(name.to_string(), wav.into());
            if let Some(lp) = loop_frames {
                self.bgm_loop.insert(name.to_string(), lp);
            }
        }

        pub fn play_sfx(&self, name: &str) {
            if let Some(data) = self.sfx.get(name) {
                let cursor = Cursor::new(data.clone());
                if let Ok(source) = Decoder::new(cursor) {
                    let _ = self.handle.play_raw(source.convert_samples());
                }
            }
        }

        pub fn fade_bgm(&mut self, seconds: f32) {
            self.fade.start(seconds);
        }
        pub fn tick(&mut self) {
            if let Some(volume) = self.fade.tick() {
                if let Some(sink) = &self.bgm {
                    sink.set_volume(volume);
                }
                if volume == 0.0 {
                    self.stop_bgm();
                }
            }
        }
        pub fn stop_bgm(&mut self) {
            if let Some(old) = self.bgm.take() {
                old.stop();
            }
            self.bgm_name = None;
        }

        /// Play a registered BGM track on infinite loop. No-op when already
        /// playing it.
        pub fn resume(&self) {}

        pub fn play_bgm(&mut self, name: &str) {
            if self.bgm_name.as_deref() == Some(name) {
                return;
            }
            if let Some(old) = self.bgm.take() {
                old.stop();
            }
            self.fade = super::Fade::default();
            let Some(data) = self.bgm_data.get(name) else {
                return;
            };
            let cursor = Cursor::new(data.clone());
            let Ok(decoder) = Decoder::new(BufReader::new(cursor)) else {
                return;
            };
            let channels = decoder.channels();
            let rate = decoder.sample_rate();
            let samples: Vec<f32> = decoder.convert_samples().collect();
            if samples.is_empty() {
                return;
            }
            // Loop points (frames from the `.pos`) -> sample indices; clamp and
            // fall back to the whole track if absent or out of range.
            let total = samples.len();
            let ch = channels.max(1) as usize;
            let (mut loop_start, mut loop_end) = match self.bgm_loop.get(name) {
                Some(&(s, e)) => (s as usize * ch, e as usize * ch),
                None => (0, total),
            };
            if loop_end > total || loop_end <= loop_start {
                loop_start = 0;
                loop_end = total;
            }
            let source = LoopSource {
                samples,
                channels,
                rate,
                pos: 0,
                loop_start,
                loop_end,
            };
            let Ok(sink) = Sink::try_new(&self.handle) else {
                return;
            };
            sink.set_volume(0.6);
            sink.append(source);
            self.bgm = Some(sink);
            self.bgm_name = Some(name.to_string());
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::Audio;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::native::LoopSource;

    // Drive the iterator directly; a LoopSource must play the intro [0,start)
    // once, then loop [start,end) forever (never restarting from 0).
    fn take(mut s: LoopSource, n: usize) -> Vec<i32> {
        (0..n).map(|_| s.next().unwrap() as i32).collect()
    }

    #[test]
    fn loops_at_loop_point_not_zero() {
        // "samples" 0..8 as f32; loop [4,8) (frames == samples here, 1 channel).
        let src = LoopSource::for_test((0..8).map(|i| i as f32).collect(), 1, 44100, 4, 8);
        // intro 0..4 plays once, then 4..8 repeats — never back to 0.
        assert_eq!(
            take(src, 14),
            vec![0, 1, 2, 3, 4, 5, 6, 7, 4, 5, 6, 7, 4, 5]
        );
    }

    #[test]
    fn whole_track_loop_when_no_points() {
        // start 0, end = len -> whole track repeats (PlayBGM(FALSE) fallback).
        let src = LoopSource::for_test((0..4).map(|i| i as f32).collect(), 1, 44100, 0, 4);
        assert_eq!(take(src, 10), vec![0, 1, 2, 3, 0, 1, 2, 3, 0, 1]);
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::collections::HashMap;
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast, closure::Closure};

    use web_sys::{AudioBuffer, AudioBufferSourceNode, AudioContext, GainNode};

    /// Decoded PCM held as an AudioBuffer, ready to (re)play cheaply.
    pub struct Audio {
        ctx: AudioContext,
        bgm_gain: GainNode,
        sfx: HashMap<String, AudioBuffer>,
        bgm: HashMap<String, Rc<[u8]>>,
        bgm_node: Rc<RefCell<Option<AudioBufferSourceNode>>>,
        bgm_name: Rc<RefCell<Option<String>>>,
        /// (loop_start, loop_end) in sample-frames from the track's `.pos`.
        bgm_loop: HashMap<String, (u32, u32)>,
        fade: super::Fade,
    }

    impl Audio {
        pub fn new() -> Option<Self> {
            let ctx = AudioContext::new().ok()?;
            // Built during the file-select user gesture, so this is allowed.
            let _ = ctx.resume();
            // Loading files and the GPU crosses async boundaries. Safari needs
            // resume() directly inside a subsequent trusted gesture callback.
            if let Some(document) = web_sys::window().and_then(|w| w.document()) {
                for event in ["keydown", "pointerdown"] {
                    let context = ctx.clone();
                    let callback = Closure::<dyn FnMut()>::new(move || {
                        let _ = context.resume();
                    });
                    if document
                        .add_event_listener_with_callback(event, callback.as_ref().unchecked_ref())
                        .is_ok()
                    {
                        callback.forget();
                    }
                }
            }
            let bgm_gain = ctx.create_gain().ok()?;
            bgm_gain.gain().set_value(0.6);
            let _ = bgm_gain.connect_with_audio_node(&ctx.destination());
            Some(Self {
                ctx,
                bgm_gain,
                sfx: HashMap::new(),
                bgm: HashMap::new(),
                bgm_node: Rc::new(RefCell::new(None)),
                bgm_name: Rc::new(RefCell::new(None)),
                bgm_loop: HashMap::new(),
                fade: super::Fade::default(),
            })
        }

        pub fn register_sfx(&mut self, name: &str, wav: Vec<u8>) {
            if let Some(buf) = self.decode_wav(&wav) {
                self.sfx.insert(name.to_string(), buf);
            }
        }

        /// Web has no filesystem; BGM wavs are registered by name from the
        /// player's uploaded `bgm/` folder.
        pub fn register_bgm(&mut self, name: &str, wav: Vec<u8>, loop_frames: Option<(u32, u32)>) {
            // Keep compressed bytes until this track plays. Decoding all 17
            // tracks up front retains hundreds of MB of PCM unnecessarily.
            self.bgm.insert(name.to_string(), wav.into());
            if let Some(lp) = loop_frames {
                self.bgm_loop.insert(name.to_string(), lp);
            }
        }

        pub fn play_sfx(&self, name: &str) {
            let Some(buf) = self.sfx.get(name) else {
                return;
            };
            if let Ok(src) = self.ctx.create_buffer_source() {
                src.set_buffer(Some(buf));
                let _ = src.connect_with_audio_node(&self.ctx.destination());
                let _ = src.start();
            }
        }

        pub fn fade_bgm(&mut self, seconds: f32) {
            self.fade.start(seconds);
        }
        pub fn tick(&mut self) {
            if let Some(volume) = self.fade.tick() {
                self.bgm_gain.gain().set_value(volume * 0.6);
                if volume == 0.0 {
                    self.stop_bgm();
                }
            }
        }
        pub fn stop_bgm(&mut self) {
            if let Some(old) = self.bgm_node.borrow_mut().take() {
                #[allow(deprecated)]
                let _ = old.stop();
            }
            *self.bgm_name.borrow_mut() = None;
        }

        /// Play a registered BGM track on infinite loop. No-op when already
        /// playing it. `name` is the wav basename (e.g. "th06_03.wav").
        pub fn play_bgm(&mut self, name: &str) {
            if self.bgm_name.borrow().as_deref() == Some(name) {
                return;
            }
            self.stop_bgm();
            self.fade = super::Fade::default();
            self.bgm_gain.gain().set_value(0.6);
            let Some(bytes) = self.bgm.get(name) else {
                return;
            };
            let bytes = js_sys::Uint8Array::from(bytes.as_ref()).buffer();
            let Ok(promise) = self.ctx.decode_audio_data(&bytes) else {
                return;
            };
            let ctx = self.ctx.clone();
            let gain = self.bgm_gain.clone();
            let active_name = self.bgm_name.clone();
            let active_node = self.bgm_node.clone();
            let name = name.to_string();
            let loop_frames = self.bgm_loop.get(&name).copied();
            *active_name.borrow_mut() = Some(name.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let Ok(value) = wasm_bindgen_futures::JsFuture::from(promise).await else {
                    if active_name.borrow().as_deref() == Some(&name) {
                        *active_name.borrow_mut() = None;
                    }
                    return;
                };
                if active_name.borrow().as_deref() != Some(&name) {
                    return;
                }
                let Ok(buf) = value.dyn_into::<AudioBuffer>() else {
                    return;
                };
                let Ok(src) = ctx.create_buffer_source() else {
                    return;
                };
                src.set_buffer(Some(&buf));
                src.set_loop(true);
                if let Some((s, e)) = loop_frames {
                    let rate = buf.sample_rate() as f64;
                    if rate > 0.0 && e > s && e <= buf.length() {
                        src.set_loop_start(s as f64 / rate);
                        src.set_loop_end(e as f64 / rate);
                    }
                }
                let _ = src.connect_with_audio_node(&gain);
                let _ = src.start();
                *active_node.borrow_mut() = Some(src);
            });
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
            let size =
                u32::from_le_bytes([d[pos + 4], d[pos + 5], d[pos + 6], d[pos + 7]]) as usize;
            let body = pos + 8;
            if body + size > d.len() {
                break;
            }
            match id {
                b"fmt " if size >= 16 => {
                    channels = u16::from_le_bytes([d[body + 2], d[body + 3]]) as u32;
                    sample_rate =
                        u32::from_le_bytes([d[body + 4], d[body + 5], d[body + 6], d[body + 7]]);
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
