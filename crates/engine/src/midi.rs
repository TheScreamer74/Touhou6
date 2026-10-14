//! Original MidiOutput.cpp scheduler, including its 1 ms integer clock and
//! global CC2/CC4 loop snapshots. Sound generation is delegated to the user's
//! SoundFont; the original game likewise delegated synthesis to a MIDI device.
use std::io::Cursor;
use std::sync::Arc;

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

#[derive(Clone)]
struct Track {
    data: Arc<[u8]>,
    cursor: usize,
    count: u64,
    status: u8,
    playing: bool,
}

fn delta(data: &[u8], cursor: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    for _ in 0..4 {
        let byte = *data.get(*cursor)?;
        *cursor += 1;
        value = (value << 7) | u32::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Short(u8, u8, u8),
    SystemExclusive(Vec<u8>),
}

#[derive(Clone)]
struct LoopState {
    tracks: Vec<(usize, u64)>,
    tempo: u64,
    milliseconds: u64,
    elapsed_ticks: u64,
}

#[derive(Clone)]
pub struct Sequence {
    tracks: Vec<Track>,
    timebase: u64,
    tempo: u64,
    milliseconds: u64,
    elapsed_ticks: u64,
    loop_state: Option<LoopState>,
    volumes: [u8; 16],
    fade: Option<(u32, u32)>,
    fade_multiplier: f32,
    fade_last_level: u32,
}

impl Sequence {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        let read_u16 = |offset| {
            data.get(offset..offset + 2)
                .and_then(|bytes| bytes.try_into().ok())
                .map(u16::from_be_bytes)
        };
        if data.get(..4) != Some(b"MThd") || data.len() > 8 * 1024 * 1024 {
            return Err("Invalid or oversized MIDI file".into());
        }
        let header = u32::from_be_bytes(
            data.get(4..8)
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or("Truncated MIDI header")?,
        ) as usize;
        let format = read_u16(8).ok_or("Truncated MIDI header")?;
        let count = read_u16(10).ok_or("Truncated MIDI header")?;
        let timebase = read_u16(12).ok_or("Truncated MIDI header")?;
        if header < 6
            || format > 1
            || count == 0
            || count > 256
            || timebase == 0
            || timebase & 0x8000 != 0
        {
            return Err("Unsupported MIDI header".into());
        }
        let mut cursor = 8usize.checked_add(header).ok_or("Invalid MIDI header")?;
        let mut tracks = Vec::new();
        for _ in 0..count {
            if data.get(cursor..cursor.saturating_add(4)) != Some(b"MTrk") {
                return Err("Missing MIDI track".into());
            }
            let length = u32::from_be_bytes(
                data.get(cursor + 4..cursor + 8)
                    .and_then(|bytes| bytes.try_into().ok())
                    .ok_or("Truncated MIDI track")?,
            ) as usize;
            cursor += 8;
            let end = cursor.checked_add(length).ok_or("Oversized MIDI track")?;
            let bytes: Arc<[u8]> = data.get(cursor..end).ok_or("Truncated MIDI track")?.into();
            let mut position = 0;
            let count = delta(&bytes, &mut position).ok_or("Invalid MIDI delta time")?;
            tracks.push(Track {
                data: bytes,
                cursor: position,
                count: u64::from(count),
                status: 0,
                playing: true,
            });
            cursor = end;
        }
        Ok(Self {
            tracks,
            timebase: u64::from(timebase),
            tempo: 1_000_000,
            milliseconds: 0,
            elapsed_ticks: 0,
            loop_state: None,
            volumes: [0; 16],
            fade: None,
            fade_multiplier: 1.0,
            fade_last_level: 0,
        })
    }

    fn now(&self) -> u64 {
        self.elapsed_ticks + self.milliseconds * self.timebase * 1000 / self.tempo
    }

    /// Original SetFadeOut interval, in multimedia timer callbacks.
    pub fn fade_milliseconds(&mut self, milliseconds: u32) {
        self.fade = Some((0, milliseconds.max(1)));
        self.fade_multiplier = 0.0;
    }

    /// Read-only counters for comparison with the executable's timer callback.
    #[doc(hidden)]
    pub fn reference_clock(&self) -> (u32, u64, u64) {
        (self.tempo as u32, self.milliseconds, self.elapsed_ticks)
    }

    /// One original multimedia timer callback. Errors stop the malformed track.
    pub fn tick(&mut self) -> Vec<Message> {
        let mut messages = Vec::new();
        if let Some((elapsed, total)) = &mut self.fade {
            if *elapsed >= *total {
                self.fade_multiplier = 0.0;
                return messages;
            }
            self.fade_multiplier = 1.0 - *elapsed as f32 / *total as f32;
            let level = (self.fade_multiplier * 128.0) as u32;
            if level != self.fade_last_level {
                for (channel, original) in self.volumes.iter().enumerate() {
                    messages.push(Message::Short(
                        0xb0 | channel as u8,
                        7,
                        (f32::from(*original) * self.fade_multiplier) as u8,
                    ));
                }
            }
            self.fade_last_level = level;
            *elapsed += 1;
        }
        let mut loaded = false;
        // A malformed zero-duration loop must not lock the audio callback.
        let mut budget = 65_536usize;
        for index in 0..self.tracks.len() {
            if !self.tracks[index].playing {
                continue;
            }
            loaded = true;
            while self.tracks[index].playing && self.tracks[index].count <= self.now() {
                if budget == 0 || self.process(index, &mut messages).is_none() {
                    self.tracks[index].playing = false;
                    break;
                }
                budget -= 1;
            }
        }
        self.milliseconds += 1;
        if !loaded {
            for track in &mut self.tracks {
                track.cursor = 0;
                track.count = u64::from(delta(&track.data, &mut track.cursor).unwrap_or(u32::MAX));
                track.playing = true;
            }
            self.milliseconds = 0;
            self.elapsed_ticks = 0;
            self.loop_state = None;
            self.fade = None;
            self.fade_multiplier = 1.0;
        }
        messages
    }

    fn process(&mut self, index: usize, messages: &mut Vec<Message>) -> Option<()> {
        let track = &mut self.tracks[index];
        let mut status = *track.data.get(track.cursor)?;
        if status < 0x80 {
            status = track.status;
        } else {
            track.cursor += 1;
        }
        let mut args = [0u8; 2];
        if (0x80..0xf0).contains(&status) {
            let length = if status & 0xe0 == 0xc0 { 1 } else { 2 };
            for byte in args.iter_mut().take(length) {
                *byte = *track.data.get(track.cursor)?;
                if *byte >= 0x80 {
                    return None;
                }
                track.cursor += 1;
            }
        } else if status == 0xf0 || status == 0xf7 || status == 0xff {
            let kind = if status == 0xff {
                let kind = *track.data.get(track.cursor)?;
                track.cursor += 1;
                Some(kind)
            } else {
                None
            };
            let length = delta(&track.data, &mut track.cursor)? as usize;
            let end = track.cursor.checked_add(length)?;
            let bytes = track.data.get(track.cursor..end)?;
            match kind {
                Some(0x2f) => {
                    track.playing = false;
                    return Some(());
                }
                Some(0x51) if length == 3 => {
                    let tempo = bytes
                        .iter()
                        .fold(0u64, |tempo, byte| tempo * 257 + u64::from(*byte));
                    if tempo == 0 {
                        return None;
                    }
                    self.elapsed_ticks += self.milliseconds * self.timebase * 1000 / self.tempo;
                    self.milliseconds = 0;
                    self.tempo = tempo;
                }
                None if status == 0xf0 => {
                    let mut packet = vec![0xf0];
                    packet.extend_from_slice(bytes);
                    messages.push(Message::SystemExclusive(packet));
                }
                _ => {}
            }
            track.cursor = end;
        } else {
            return None;
        }

        if status & 0xf0 == 0xb0 && args[0] == 2 {
            self.loop_state = Some(LoopState {
                tracks: self
                    .tracks
                    .iter()
                    .map(|track| (track.cursor, track.count))
                    .collect(),
                tempo: self.tempo,
                milliseconds: self.milliseconds,
                elapsed_ticks: self.elapsed_ticks,
            });
        } else if status & 0xf0 == 0xb0
            && args[0] == 4
            && let Some(state) = &self.loop_state
        {
            for (track, (cursor, count)) in self.tracks.iter_mut().zip(&state.tracks) {
                track.cursor = *cursor;
                track.count = *count;
            }
            self.tempo = state.tempo;
            self.milliseconds = state.milliseconds;
            self.elapsed_ticks = state.elapsed_ticks;
        }
        if status & 0xf0 == 0xb0 && args[0] == 7 {
            self.volumes[usize::from(status & 15)] = args[1];
            args[1] = (f32::from(args[1]) * self.fade_multiplier) as u8;
        }
        if status < 0xf0 {
            messages.push(Message::Short(status, args[0], args[1]));
        }
        let track = &mut self.tracks[index];
        track.status = status;
        track.count += u64::from(delta(&track.data, &mut track.cursor)?);
        Some(())
    }
}

pub fn soundfont(bytes: &[u8]) -> Result<Arc<SoundFont>, String> {
    if bytes.len() > 256 * 1024 * 1024 {
        return Err("SoundFont exceeds 256 MiB".into());
    }
    SoundFont::new(&mut Cursor::new(bytes))
        .map(Arc::new)
        .map_err(|error| format!("Invalid SoundFont: {error}"))
}

/// A separate WASM instance lives on the browser's audio rendering thread.
/// No DOM, game state, file loading or rendering runs in this object.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub struct MidiSynth {
    bank: Arc<SoundFont>,
    player: Option<Player>,
    rate: u32,
    left: Vec<f32>,
    right: Vec<f32>,
}
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
impl MidiSynth {
    #[wasm_bindgen::prelude::wasm_bindgen(constructor)]
    pub fn new(bank: &[u8], rate: u32) -> Result<MidiSynth, wasm_bindgen::JsValue> {
        Ok(Self {
            bank: soundfont(bank).map_err(|error| wasm_bindgen::JsValue::from_str(&error))?,
            player: None,
            rate,
            left: vec![0.0; 128],
            right: vec![0.0; 128],
        })
    }
    pub fn play(&mut self, bytes: &[u8]) -> Result<(), wasm_bindgen::JsValue> {
        self.player = Some(
            Player::new(&self.bank, bytes, self.rate)
                .map_err(|error| wasm_bindgen::JsValue::from_str(&error))?,
        );
        Ok(())
    }
    pub fn stop(&mut self) {
        self.player = None;
    }
    pub fn fade(&mut self, seconds: f32) {
        if let Some(player) = &mut self.player {
            player.fade(seconds);
        }
    }
    pub fn render(&mut self, left: &js_sys::Float32Array, right: &js_sys::Float32Array) {
        let frames = left.length().min(right.length()) as usize;
        if self.left.len() != frames {
            self.left.resize(frames, 0.0);
            self.right.resize(frames, 0.0);
        }
        if let Some(player) = &mut self.player {
            player.render(&mut self.left, &mut self.right);
        } else {
            self.left.fill(0.0);
            self.right.fill(0.0);
        }
        left.copy_from(&self.left);
        right.copy_from(&self.right);
    }
}

pub struct Player {
    sequence: Sequence,
    synth: Synthesizer,
    rate: u64,
    frames: u64,
    milliseconds: u64,
}
impl Player {
    pub fn new(bank: &Arc<SoundFont>, data: &[u8], rate: u32) -> Result<Self, String> {
        let sequence = Sequence::parse(data)?;
        let synth = Synthesizer::new(bank, &SynthesizerSettings::new(rate as i32))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            sequence,
            synth,
            rate: u64::from(rate),
            frames: 0,
            milliseconds: 0,
        })
    }
    pub fn fade(&mut self, seconds: f32) {
        self.sequence
            .fade_milliseconds((seconds * 1000.0).max(1.0) as u32);
    }
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let mut offset = 0;
        while offset < left.len().min(right.len()) {
            while self.milliseconds * self.rate <= self.frames * 1000 {
                for message in self.sequence.tick() {
                    match message {
                        Message::Short(status, first, second) => self.synth.process_midi_message(
                            i32::from(status & 15),
                            i32::from(status & 0xf0),
                            i32::from(first),
                            i32::from(second),
                        ),
                        Message::SystemExclusive(packet) => {
                            // GM System On / Roland GS Reset; original files use GS.
                            if packet.get(1..5) == Some(&[0x7e, 0x7f, 9, 1])
                                || packet.get(1..9).is_some_and(|p| {
                                    p[0] == 0x41 && p[2..8] == [0x42, 0x12, 0x40, 0, 0x7f, 0]
                                })
                            {
                                self.synth.reset();
                            }
                        }
                    }
                }
                self.milliseconds += 1;
            }
            let next_tick = (self.milliseconds * self.rate).div_ceil(1000);
            let length = ((next_tick - self.frames) as usize)
                .max(1)
                .min(left.len() - offset)
                .min(right.len() - offset);
            self.synth.render(
                &mut left[offset..offset + length],
                &mut right[offset..offset + length],
            );
            self.frames += length as u64;
            offset += length;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn midi(track: &[u8]) -> Vec<u8> {
        let mut bytes = b"MThd\0\0\0\x06\0\0\0\x01\x01\xe0MTrk".to_vec();
        bytes.extend_from_slice(&(track.len() as u32).to_be_bytes());
        bytes.extend_from_slice(track);
        bytes
    }
    #[test]
    fn original_clock_running_status_and_loop_markers() {
        let bytes = midi(&[
            0, 0xb0, 2, 0, 0, 0x90, 60, 100, 1, 60, 0, 0, 0xb0, 4, 0, 0, 0xff, 0x2f, 0,
        ]);
        let mut sequence = Sequence::parse(&bytes).unwrap();
        assert_eq!(
            sequence.tick(),
            vec![Message::Short(0xb0, 2, 0), Message::Short(0x90, 60, 100)]
        );
        assert!(sequence.tick().is_empty());
        assert!(sequence.tick().is_empty());
        assert_eq!(
            sequence.tick(),
            vec![
                Message::Short(0x90, 60, 0),
                Message::Short(0xb0, 4, 0),
                Message::Short(0x90, 60, 100)
            ]
        );
    }
    #[test]
    fn malformed_track_is_bounded() {
        assert!(Sequence::parse(b"").is_err());
        assert!(Sequence::parse(&midi(&[0xff; 5])).is_err());
        let mut sequence = Sequence::parse(&midi(&[0, 0x90, 60])).unwrap();
        assert!(sequence.tick().is_empty());
    }
    #[test]
    fn original_fade_sends_only_on_128_step_changes() {
        let mut sequence =
            Sequence::parse(&midi(&[0, 0xb0, 7, 100, 0x87, 0x68, 0xff, 0x2f, 0])).unwrap();
        assert_eq!(sequence.tick(), vec![Message::Short(0xb0, 7, 100)]);
        sequence.fade = Some((0, 1000));
        sequence.fade_multiplier = 0.0;
        let first = sequence.tick();
        assert_eq!(first.len(), 16);
        assert_eq!(first[0], Message::Short(0xb0, 7, 100));
        let second = sequence.tick();
        assert_eq!(second.len(), 16);
        assert_eq!(second[0], Message::Short(0xb0, 7, 99));
        assert!(sequence.tick().is_empty());
        assert!(sequence.tick().is_empty());
    }
    #[test]
    #[ignore = "requires user-supplied original MD extraction"]
    fn all_original_tracks_complete_multiple_loop_cycles() {
        let dir = std::env::var("TH06_MIDI_DIR").unwrap();
        for index in 1..=17 {
            let data = std::fs::read(format!("{dir}/th06_{index:02}.mid")).unwrap();
            let mut sequence = Sequence::parse(&data).unwrap();
            let mut notes = 0;
            let mut ends = 0;
            for _ in 0..600_000 {
                for message in sequence.tick() {
                    if let Message::Short(status, first, velocity) = message {
                        notes += usize::from(status & 0xf0 == 0x90 && velocity != 0);
                        ends += usize::from(status & 0xf0 == 0xb0 && first == 4);
                    }
                }
            }
            assert!(
                notes > 100 && ends > 1,
                "track {index}: {notes} notes, {ends} loops"
            );
        }
    }
    #[test]
    #[ignore = "requires original MIDI and separately licensed GM SoundFont"]
    fn original_tracks_synthesize_real_stereo_pcm_at_both_device_rates() {
        let dir = std::env::var("TH06_MIDI_DIR").unwrap();
        let bank =
            soundfont(&std::fs::read(std::env::var("TH06_SOUNDFONT").unwrap()).unwrap()).unwrap();
        for rate in [44100, 48000] {
            for index in 1..=17 {
                let data = std::fs::read(format!("{dir}/th06_{index:02}.mid")).unwrap();
                let mut player = Player::new(&bank, &data, rate).unwrap();
                let mut left = vec![0.0; rate as usize * 5];
                let mut right = vec![0.0; left.len()];
                player.render(&mut left, &mut right);
                assert!(left.iter().chain(&right).all(|value| value.is_finite()));
                let energy = left
                    .iter()
                    .chain(&right)
                    .map(|value| f64::from(*value).powi(2))
                    .sum::<f64>();
                assert!(energy > 0.01, "silent track {index} at {rate}: {energy}");
                player.fade(0.5);
                player.render(&mut left[..rate as usize], &mut right[..rate as usize]);
            }
        }
    }
}
