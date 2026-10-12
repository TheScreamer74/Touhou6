//! BGM and sound effects. Native uses rodio (streams BGM from disk).
//! Web uses the Web Audio API with in-memory PCM buffers (no filesystem on
//! the browser — every wav is uploaded by the player and held in memory).
//! Both fail soft: with no output the game runs silent.

/// SoundPlayer.cpp: 26 shared files, 32 independently retriggered variants.
pub const SFX_FILES: [&str; 26] = [
    "plst00", "enep00", "pldead00", "power0", "power1", "tan00", "tan01", "tan02", "ok00",
    "cancel00", "select00", "gun00", "cat00", "lazer00", "lazer01", "enep01", "nep00", "damage00",
    "item00", "kira00", "kira01", "kira02", "extend", "timeout", "graze", "powerup",
];
const SFX_VARIANTS: [(usize, i32); 32] = [
    (0, -1500),
    (0, -2000),
    (1, -1200),
    (1, -1400),
    (2, -1000),
    (3, -500),
    (4, -500),
    (5, -1700),
    (6, -1700),
    (7, -1700),
    (8, -1000),
    (9, -1000),
    (10, -1900),
    (11, -1200),
    (12, -900),
    (5, -1500),
    (13, -900),
    (14, -900),
    (15, -600),
    (16, -400),
    (17, -1100),
    (18, -900),
    (5, -1800),
    (6, -1800),
    (7, -1800),
    (19, -300),
    (20, -600),
    (21, -800),
    (22, -100),
    (23, -500),
    (24, -1000),
    (25, -1000),
];

#[derive(Default)]
struct SoundQueue(std::cell::RefCell<Vec<usize>>);
impl SoundQueue {
    fn queue(&self, index: usize) {
        let mut queue = self.0.borrow_mut();
        if index < SFX_VARIANTS.len() && queue.len() < 3 && !queue.contains(&index) {
            queue.push(index);
        }
    }
    fn named(&self, name: &str) {
        if let Some(index) = SFX_VARIANTS
            .iter()
            .position(|(file, _)| SFX_FILES[*file] == name)
        {
            self.queue(index);
        }
    }
    fn drain(&self) -> Vec<usize> {
        self.0.borrow_mut().drain(..).collect()
    }
}

fn sfx_gain(index: usize) -> f32 {
    10.0_f32.powf(SFX_VARIANTS[index].1 as f32 / 2000.0)
}

#[cfg(test)]
mod sound_queue_tests {
    use super::*;
    #[test]
    fn original_three_variant_queue_deduplicates_without_merging_shared_files() {
        let queue = SoundQueue::default();
        queue.queue(7);
        queue.queue(7);
        queue.queue(15);
        queue.queue(22);
        queue.queue(28);
        assert_eq!(queue.drain(), vec![7, 15, 22]);
        queue.named("ok00");
        queue.named("cancel00");
        queue.named("select00");
        assert_eq!(queue.drain(), vec![10, 11, 12]);
        assert!((sfx_gain(0) - 0.177_827_94).abs() < 1e-7);
        assert!((sfx_gain(1) - 0.1).abs() < 1e-7);
    }
}

/// `.pos` values count frames in the encoded track, before Web Audio resamples
/// it to the output device's sample rate. Keep that rate when converting the
/// loop positions to seconds; AudioBuffer::sample_rate is the output rate.
#[cfg(any(target_arch = "wasm32", test))]
fn encoded_sample_rate(data: &[u8]) -> Option<u32> {
    if data.get(..4) == Some(b"RIFF") && data.get(8..12) == Some(b"WAVE") {
        let mut offset = 12usize;
        while let Some(header) = data.get(offset..offset.checked_add(8)?) {
            let size = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
            let body = offset.checked_add(8)?;
            let end = body.checked_add(size)?;
            let chunk = data.get(body..end)?;
            if &header[..4] == b"fmt " {
                if chunk.len() < 16 {
                    return None;
                }
                let rate = u32::from_le_bytes(chunk[4..8].try_into().ok()?);
                return (rate > 0).then_some(rate);
            }
            offset = end.checked_add(size & 1)?;
        }
    } else if data.get(..4) == Some(b"OggS") && data.get(4) == Some(&0) {
        // The first Ogg/Vorbis packet is the 30-byte identification header.
        // Its sample rate survives the WAV -> Vorbis conversion unchanged.
        let segments = usize::from(*data.get(26)?);
        let lacing = data.get(27..27usize.checked_add(segments)?)?;
        let packet_size = lacing
            .iter()
            .take_while(|&&size| size == 255)
            .map(|&size| usize::from(size))
            .sum::<usize>();
        let last = *lacing.get(packet_size / 255)?;
        let packet_size = packet_size.checked_add(usize::from(last))?;
        let body = 27usize.checked_add(segments)?;
        let packet = data.get(body..body.checked_add(packet_size)?)?;
        if packet.len() >= 30 && packet.get(..7) == Some(b"\x01vorbis") {
            let rate = u32::from_le_bytes(packet[12..16].try_into().ok()?);
            return (rate > 0).then_some(rate);
        }
    }
    None
}

#[cfg(any(target_arch = "wasm32", test))]
fn loop_seconds(frames: (u32, u32), source_rate: u32, duration: f64) -> Option<(f64, f64)> {
    let (start, end) = frames;
    if source_rate == 0 || start >= end || !duration.is_finite() {
        return None;
    }
    let start = f64::from(start) / f64::from(source_rate);
    let end = f64::from(end) / f64::from(source_rate);
    (end <= duration).then_some((start, end))
}

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
    use std::sync::{Arc, Mutex};

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
        midi: HashMap<String, Arc<[u8]>>,
        soundfont: Option<Arc<rustysynth::SoundFont>>,
        midi_player: Option<Arc<Mutex<crate::midi::Player>>>,
        sfx_queue: super::SoundQueue,
        sfx_voices: std::cell::RefCell<Vec<Option<Sink>>>,
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
                midi: HashMap::new(),
                soundfont: None,
                midi_player: None,
                sfx_queue: super::SoundQueue::default(),
                sfx_voices: std::cell::RefCell::new((0..32).map(|_| None).collect()),
                fade: super::Fade::default(),
            })
        }

        pub fn register_soundfont(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.soundfont = Some(crate::midi::soundfont(bytes)?);
            Ok(())
        }
        pub fn register_midi(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
            crate::midi::Sequence::parse(&bytes)?;
            self.midi.insert(name.to_string(), bytes.into());
            Ok(())
        }
        pub fn midi_available(&self) -> bool {
            self.soundfont.is_some()
                && (1..=17).all(|index| self.midi.contains_key(&format!("th06_{index:02}.mid")))
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
            self.sfx_queue.named(name);
        }
        pub fn play_sfx_idx(&self, index: u8) {
            self.sfx_queue.queue(usize::from(index));
        }
        pub fn flush_sfx(&self) {
            for index in self.sfx_queue.drain() {
                let name = super::SFX_FILES[super::SFX_VARIANTS[index].0];
                if let Some(data) = self.sfx.get(name) {
                    if let Some(old) = self.sfx_voices.borrow_mut()[index].take() {
                        old.stop();
                    }
                    let cursor = Cursor::new(data.clone());
                    if let Ok(source) = Decoder::new(cursor)
                        && let Ok(sink) = Sink::try_new(&self.handle)
                    {
                        sink.set_volume(super::sfx_gain(index));
                        sink.append(source);
                        self.sfx_voices.borrow_mut()[index] = Some(sink);
                    }
                }
            }
        }

        pub fn fade_bgm(&mut self, seconds: f32) {
            self.fade.start(seconds);
            if let Some(player) = &self.midi_player
                && let Ok(mut player) = player.lock()
            {
                player.fade(seconds);
            }
        }
        pub fn tick(&mut self) {
            if let Some(volume) = self.fade.tick() {
                if let Some(sink) = &self.bgm
                    && self.midi_player.is_none()
                {
                    sink.set_volume(volume * 0.6);
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
            self.midi_player = None;
        }

        /// Play a registered BGM track on infinite loop. No-op when already
        /// playing it.
        pub fn resume(&self) {}

        pub fn play_bgm(&mut self, name: &str) {
            if self.bgm_name.as_deref() == Some(name) {
                return;
            }
            self.stop_bgm();
            self.fade = super::Fade::default();
            if name.ends_with(".mid") {
                let Some(bank) = &self.soundfont else {
                    return;
                };
                let Some(bytes) = self.midi.get(name) else {
                    return;
                };
                let Ok(player) = crate::midi::Player::new(bank, bytes, 44100) else {
                    return;
                };
                let player = Arc::new(Mutex::new(player));
                let Ok(sink) = Sink::try_new(&self.handle) else {
                    return;
                };
                sink.set_volume(0.6);
                sink.append(MidiSource {
                    player: player.clone(),
                    samples: [0.0; 512],
                    cursor: 512,
                });
                self.bgm = Some(sink);
                self.midi_player = Some(player);
                self.bgm_name = Some(name.to_string());
                return;
            }
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

    struct MidiSource {
        player: Arc<Mutex<crate::midi::Player>>,
        samples: [f32; 512],
        cursor: usize,
    }
    impl Iterator for MidiSource {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            if self.cursor == self.samples.len() {
                let mut left = [0.0; 256];
                let mut right = [0.0; 256];
                self.player.lock().ok()?.render(&mut left, &mut right);
                for (sample, (left, right)) in self
                    .samples
                    .chunks_exact_mut(2)
                    .zip(left.into_iter().zip(right))
                {
                    sample.copy_from_slice(&[left, right]);
                }
                self.cursor = 0;
            }
            let sample = self.samples[self.cursor];
            self.cursor += 1;
            Some(sample)
        }
    }
    impl Source for MidiSource {
        fn current_frame_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> u16 {
            2
        }
        fn sample_rate(&self) -> u32 {
            44100
        }
        fn total_duration(&self) -> Option<std::time::Duration> {
            None
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::Audio;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::native::LoopSource;
    use super::{encoded_sample_rate, loop_seconds};

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

    #[test]
    fn stereo_loop_preserves_both_channels_at_the_boundary() {
        let src = LoopSource::for_test((0..10).map(|i| i as f32).collect(), 2, 44100, 4, 8);
        assert_eq!(
            take(src, 14),
            vec![0, 1, 2, 3, 4, 5, 6, 7, 4, 5, 6, 7, 4, 5]
        );
    }

    #[test]
    fn web_loop_positions_survive_resampling_to_48000_hz() {
        // The original title track is 44.1 kHz. decodeAudioData at 48 kHz
        // creates a longer buffer, but the musical loop still starts and ends
        // at exactly the same times. Dividing the .pos by 48000 ended it 4.81s
        // too early, cutting into the phrase before returning to the loop.
        let original_frames = 2_801_664u32;
        let decoded_frames = (f64::from(original_frames) * 48000.0 / 44100.0).ceil();
        let points = (264_615, 2_611_365);
        let at_44100 = loop_seconds(points, 44100, f64::from(original_frames) / 44100.0);
        let at_48000 = loop_seconds(points, 44100, decoded_frames / 48000.0);
        assert_eq!(at_44100, at_48000);
        let (start, end) = at_48000.unwrap();
        assert!((start - 6.000_340_136_054_422).abs() < 1e-12);
        assert!((end - 59.214_625_850_340_14).abs() < 1e-12);
        assert!(end - f64::from(points.1) / 48000.0 > 4.8);
    }

    #[test]
    fn rejects_loop_points_outside_the_decoded_track() {
        assert_eq!(loop_seconds((1, 2), 0, 3.0), None);
        assert_eq!(loop_seconds((2, 2), 44100, 3.0), None);
        assert_eq!(loop_seconds((2, 1), 44100, 3.0), None);
        assert_eq!(loop_seconds((0, 88200), 44100, 1.0), None);
        assert_eq!(loop_seconds((0, 44100), 44100, f64::NAN), None);
        assert_eq!(loop_seconds((0, 44100), 44100, 1.0), Some((0.0, 1.0)));
    }

    #[test]
    fn reads_wav_and_vorbis_source_rates_with_bounded_headers() {
        let mut wav = b"RIFF\0\0\0\0WAVEJUNK\x01\0\0\0\0\0fmt \x10\0\0\0".to_vec();
        let mut fmt = [0u8; 16];
        fmt[4..8].copy_from_slice(&44100u32.to_le_bytes());
        wav.extend(fmt);
        assert_eq!(encoded_sample_rate(&wav), Some(44100));
        assert_eq!(encoded_sample_rate(&wav[..wav.len() - 1]), None);

        let mut ogg = vec![0u8; 28 + 30];
        ogg[..4].copy_from_slice(b"OggS");
        ogg[26] = 1;
        ogg[27] = 30;
        ogg[28..35].copy_from_slice(b"\x01vorbis");
        ogg[40..44].copy_from_slice(&44100u32.to_le_bytes());
        assert_eq!(encoded_sample_rate(&ogg), Some(44100));
        assert_eq!(encoded_sample_rate(&ogg[..ogg.len() - 1]), None);
        ogg[27] = 255;
        assert_eq!(encoded_sample_rate(&ogg), None);
        assert_eq!(encoded_sample_rate(b""), None);
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::collections::HashMap;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use wasm_bindgen::{JsCast, closure::Closure};

    use web_sys::{
        AudioBuffer, AudioBufferSourceNode, AudioContext, AudioProcessingEvent, AudioWorkletNode,
        GainNode, ScriptProcessorNode,
    };

    /// Decoded PCM held as an AudioBuffer, ready to (re)play cheaply.
    pub struct Audio {
        ctx: AudioContext,
        bgm_gain: GainNode,
        sfx: HashMap<String, AudioBuffer>,
        bgm: HashMap<String, Rc<[u8]>>,
        bgm_node: Rc<RefCell<Option<AudioBufferSourceNode>>>,
        bgm_name: Rc<RefCell<Option<String>>>,
        /// Invalidates pending decodes even for an A -> B -> A track change.
        bgm_generation: Rc<Cell<u64>>,
        /// (loop_start, loop_end) in sample-frames from the track's `.pos`.
        bgm_loop: HashMap<String, (u32, u32)>,
        midi: HashMap<String, Rc<[u8]>>,
        soundfont: Option<std::sync::Arc<rustysynth::SoundFont>>,
        midi_node: Option<ScriptProcessorNode>,
        midi_callback: Option<Closure<dyn FnMut(AudioProcessingEvent)>>,
        midi_player: Option<Rc<RefCell<crate::midi::Player>>>,
        midi_worklet: Rc<RefCell<Option<AudioWorkletNode>>>,
        midi_worklet_ready: Option<js_sys::Promise>,
        midi_worklet_bank: Rc<RefCell<Option<Rc<[u8]>>>>,
        midi_on_worklet: bool,
        sfx_queue: super::SoundQueue,
        sfx_voices: RefCell<Vec<Option<(AudioBufferSourceNode, GainNode)>>>,
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
            let midi_worklet = Rc::new(RefCell::new(None));
            let midi_worklet_bank: Rc<RefCell<Option<Rc<[u8]>>>> = Rc::new(RefCell::new(None));
            let midi_worklet_ready = if let Ok(factory) =
                js_sys::Reflect::get(&js_sys::global(), &"__th06CreateMidiWorklet".into())
                    .and_then(|value| value.dyn_into::<js_sys::Function>())
                && let Ok(promise) = factory.call1(&wasm_bindgen::JsValue::NULL, ctx.as_ref())
            {
                let active_node = midi_worklet.clone();
                let bank = midi_worklet_bank.clone();
                let gain = bgm_gain.clone();
                Some(wasm_bindgen_futures::future_to_promise(async move {
                    if let Ok(value) =
                        wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&promise))
                            .await
                        && let Ok(node) = value.dyn_into::<AudioWorkletNode>()
                        && node.connect_with_audio_node(&gain).is_ok()
                    {
                        if let Some(bytes) = bank.borrow().as_ref() {
                            worklet_message(&node, "bank", Some(bytes), None, None);
                        }
                        *active_node.borrow_mut() = Some(node);
                    }
                    // A rejected/unsupported worklet leaves the compatibility
                    // backend available, without preventing the game starting.
                    Ok(wasm_bindgen::JsValue::UNDEFINED)
                }))
            } else {
                None
            };
            Some(Self {
                ctx,
                bgm_gain,
                sfx: HashMap::new(),
                bgm: HashMap::new(),
                bgm_node: Rc::new(RefCell::new(None)),
                bgm_name: Rc::new(RefCell::new(None)),
                bgm_generation: Rc::new(Cell::new(0)),
                bgm_loop: HashMap::new(),
                midi: HashMap::new(),
                soundfont: None,
                midi_node: None,
                midi_callback: None,
                midi_player: None,
                midi_worklet,
                midi_worklet_ready,
                midi_worklet_bank,
                midi_on_worklet: false,
                sfx_queue: super::SoundQueue::default(),
                sfx_voices: RefCell::new((0..32).map(|_| None).collect()),
                fade: super::Fade::default(),
            })
        }

        /// Await installation before the first MIDI play request. This also
        /// settles successfully when only the compatibility backend is usable.
        pub async fn prepare(&self) {
            if let Some(promise) = &self.midi_worklet_ready {
                let _ = wasm_bindgen_futures::JsFuture::from(promise.clone()).await;
            }
        }

        pub fn register_sfx(&mut self, name: &str, wav: Vec<u8>) {
            if let Some(buf) = self.decode_wav(&wav) {
                self.sfx.insert(name.to_string(), buf);
            }
        }

        pub fn register_soundfont(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.soundfont = Some(crate::midi::soundfont(bytes)?);
            *self.midi_worklet_bank.borrow_mut() = Some(bytes.into());
            if let Some(node) = self.midi_worklet.borrow().as_ref() {
                worklet_message(node, "bank", Some(bytes), None, None);
            }
            Ok(())
        }
        pub fn register_midi(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
            crate::midi::Sequence::parse(&bytes)?;
            self.midi.insert(name.to_string(), bytes.into());
            Ok(())
        }
        pub fn midi_available(&self) -> bool {
            self.soundfont.is_some()
                && (1..=17).all(|index| self.midi.contains_key(&format!("th06_{index:02}.mid")))
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
            self.sfx_queue.named(name);
        }
        pub fn play_sfx_idx(&self, index: u8) {
            self.sfx_queue.queue(usize::from(index));
        }
        pub fn flush_sfx(&self) {
            for index in self.sfx_queue.drain() {
                let name = super::SFX_FILES[super::SFX_VARIANTS[index].0];
                let Some(buf) = self.sfx.get(name) else {
                    continue;
                };
                if let Some((old, gain)) = self.sfx_voices.borrow_mut()[index].take() {
                    #[allow(deprecated)]
                    let _ = old.stop_with_when(0.0);
                    let _ = old.disconnect();
                    let _ = gain.disconnect();
                }
                if let (Ok(src), Ok(gain)) =
                    (self.ctx.create_buffer_source(), self.ctx.create_gain())
                {
                    src.set_buffer(Some(buf));
                    gain.gain().set_value(super::sfx_gain(index));
                    let _ = src.connect_with_audio_node(&gain);
                    let _ = gain.connect_with_audio_node(&self.ctx.destination());
                    let _ = src.start();
                    self.sfx_voices.borrow_mut()[index] = Some((src, gain));
                }
            }
        }

        pub fn fade_bgm(&mut self, seconds: f32) {
            self.fade.start(seconds);
            if self.midi_on_worklet
                && let Some(node) = self.midi_worklet.borrow().as_ref()
            {
                worklet_message(node, "fade", None, Some(seconds), None);
            }
            if let Some(player) = &self.midi_player {
                player.borrow_mut().fade(seconds);
            }
        }
        pub fn tick(&mut self) {
            let worklet_failed = self.midi_worklet.borrow().as_ref().is_some_and(|node| {
                js_sys::Reflect::get(node, &"__th06Failed".into())
                    .ok()
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            });
            if worklet_failed {
                let retry = self
                    .midi_on_worklet
                    .then(|| self.bgm_name.borrow().clone())
                    .flatten();
                if retry.is_some() {
                    self.stop_bgm();
                }
                if let Some(node) = self.midi_worklet.borrow_mut().take() {
                    let _ = node.disconnect();
                }
                if let Some(name) = retry {
                    self.play_bgm(&name);
                }
            }
            if let Some(volume) = self.fade.tick() {
                if self.midi_player.is_none() && !self.midi_on_worklet {
                    self.bgm_gain.gain().set_value(volume * 0.6);
                }
                if volume == 0.0 {
                    self.stop_bgm();
                }
            }
        }
        pub fn stop_bgm(&mut self) {
            if self.midi_on_worklet
                && let Some(node) = self.midi_worklet.borrow().as_ref()
            {
                worklet_message(node, "stop", None, None, None);
            }
            self.midi_on_worklet = false;
            if let Some(node) = self.midi_node.take() {
                node.set_onaudioprocess(None);
                let _ = node.disconnect();
            }
            self.midi_callback = None;
            self.midi_player = None;
            self.bgm_generation
                .set(self.bgm_generation.get().wrapping_add(1));
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
            if name.ends_with(".mid") {
                let Some(bank) = &self.soundfont else {
                    return;
                };
                let Some(bytes) = self.midi.get(name) else {
                    return;
                };
                if let Some(node) = self.midi_worklet.borrow().as_ref() {
                    worklet_message(node, "play", Some(bytes), None, Some(name));
                    self.midi_on_worklet = true;
                    *self.bgm_name.borrow_mut() = Some(name.to_string());
                    return;
                }
                let Ok(player) =
                    crate::midi::Player::new(bank, bytes, self.ctx.sample_rate() as u32)
                else {
                    return;
                };
                let Ok(node) = self.ctx.create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(2048, 0, 2) else { return; };
                let player = Rc::new(RefCell::new(player));
                let active = player.clone();
                let callback = Closure::<dyn FnMut(AudioProcessingEvent)>::new(
                    move |event: AudioProcessingEvent| {
                        let Ok(buffer) = event.output_buffer() else {
                            return;
                        };
                        let mut left = vec![0.0; buffer.length() as usize];
                        let mut right = vec![0.0; left.len()];
                        active.borrow_mut().render(&mut left, &mut right);
                        let _ = buffer.copy_to_channel(&left, 0);
                        let _ = buffer.copy_to_channel(&right, 1);
                    },
                );
                node.set_onaudioprocess(Some(callback.as_ref().unchecked_ref()));
                let _ = node.connect_with_audio_node(&self.bgm_gain);
                self.midi_node = Some(node);
                self.midi_callback = Some(callback);
                self.midi_player = Some(player);
                *self.bgm_name.borrow_mut() = Some(name.to_string());
                return;
            }
            let Some(bytes) = self.bgm.get(name) else {
                return;
            };
            let source_rate = super::encoded_sample_rate(bytes);
            let bytes = js_sys::Uint8Array::from(bytes.as_ref()).buffer();
            let Ok(promise) = self.ctx.decode_audio_data(&bytes) else {
                return;
            };
            let ctx = self.ctx.clone();
            let gain = self.bgm_gain.clone();
            let active_name = self.bgm_name.clone();
            let active_node = self.bgm_node.clone();
            let generation = self.bgm_generation.clone();
            let requested_generation = generation.get();
            let name = name.to_string();
            let loop_frames = self.bgm_loop.get(&name).copied();
            *active_name.borrow_mut() = Some(name.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let Ok(value) = wasm_bindgen_futures::JsFuture::from(promise).await else {
                    if generation.get() == requested_generation {
                        *active_name.borrow_mut() = None;
                    }
                    return;
                };
                if generation.get() != requested_generation {
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
                if let Some((start, end)) = loop_frames
                    .zip(source_rate)
                    .and_then(|(frames, rate)| super::loop_seconds(frames, rate, buf.duration()))
                {
                    src.set_loop_start(start);
                    src.set_loop_end(end);
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
                buf.copy_to_channel(&plane, ch as i32).ok()?;
            }
            Some(buf)
        }
    }

    fn worklet_message(
        node: &AudioWorkletNode,
        kind: &str,
        bytes: Option<&[u8]>,
        seconds: Option<f32>,
        name: Option<&str>,
    ) {
        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&message, &"type".into(), &kind.into());
        if let Some(bytes) = bytes {
            let _ =
                js_sys::Reflect::set(&message, &"bytes".into(), &js_sys::Uint8Array::from(bytes));
        }
        if let Some(seconds) = seconds {
            let _ = js_sys::Reflect::set(
                &message,
                &"seconds".into(),
                &wasm_bindgen::JsValue::from_f64(f64::from(seconds)),
            );
        }
        if let Some(name) = name {
            let _ = js_sys::Reflect::set(&message, &"name".into(), &name.into());
        }
        if let Ok(port) = node.port() {
            let _ = port.post_message(&message);
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
