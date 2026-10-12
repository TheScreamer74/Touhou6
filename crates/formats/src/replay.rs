//! Original T6RP replay format (ReplayData.hpp / ReplayManager.cpp, v1.02h).

pub const END_FRAME: i32 = 9_999_999;
pub const CAPTURE_MASK: u16 = 0x01f7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplayInput {
    pub frame: i32,
    pub buttons: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayStage {
    /// Score at the end of this stage, as in the original format.
    pub score: u32,
    pub random_seed: u16,
    pub point_items: u16,
    pub power: u8,
    pub lives: i8,
    pub bombs: i8,
    pub rank: u8,
    pub power_item_count: i8,
    pub inputs: Vec<ReplayInput>,
}

impl ReplayStage {
    pub fn record(&mut self, frame: i32, buttons: u16) {
        let buttons = buttons & CAPTURE_MASK;
        if self.inputs.last().is_none_or(|i| i.buttons != buttons) {
            self.inputs.push(ReplayInput { frame, buttons });
        }
    }
    pub fn finish(&mut self, frame: i32, score: u32) {
        self.score = score;
        self.inputs.push(ReplayInput { frame, buttons: 0 });
        self.inputs.push(ReplayInput {
            frame: END_FRAME,
            buttons: 0,
        });
    }
    pub fn buttons_at(&self, frame: i32) -> u16 {
        let i = self.inputs.partition_point(|input| input.frame <= frame);
        i.checked_sub(1).map_or(0, |i| self.inputs[i].buttons)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Replay {
    pub character: u8,
    pub difficulty: u8,
    pub date: String,
    pub name: String,
    pub score: u32,
    pub slowdown: f32,
    pub stages: [Option<ReplayStage>; 7],
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn string(b: &[u8]) -> String {
    let end = b.iter().position(|&v| v == 0).unwrap_or(b.len());
    encoding_rs::SHIFT_JIS
        .decode_without_bom_handling(&b[..end])
        .0
        .into_owned()
}

impl Replay {
    pub fn parse(raw: &[u8]) -> Result<Self, String> {
        if raw.len() < 80 || raw.len() > 7 * 0x69780 + 80 || &raw[..4] != b"T6RP" {
            return Err("not a valid T6RP replay".into());
        }
        if u16_at(raw, 4) != 0x0102 || raw[6] >= 4 || raw[7] >= 5 {
            return Err("unsupported replay version, character or difficulty".into());
        }
        let mut b = raw.to_vec();
        let mut key = b[14];
        for byte in &mut b[15..] {
            *byte = byte.wrapping_sub(key);
            key = key.wrapping_add(7);
        }
        let checksum = b[14..]
            .iter()
            .fold(0x3f000318u32, |sum, &v| sum.wrapping_add(v as u32));
        if checksum != u32_at(&b, 8) {
            return Err("replay checksum mismatch".into());
        }
        let slowdown = f32::from_bits(u32_at(&b, 44));
        if !slowdown.is_finite() {
            return Err("invalid replay slowdown".into());
        }
        let mut out = Self {
            character: b[6],
            difficulty: b[7],
            date: string(&b[16..25]),
            name: string(&b[25..33]),
            score: u32_at(&b, 36),
            slowdown,
            stages: std::array::from_fn(|_| None),
        };
        let offsets: Vec<usize> = (0..7).map(|i| u32_at(&b, 52 + i * 4) as usize).collect();
        let mut previous_end = 80;
        for (i, &at) in offsets.iter().enumerate() {
            if at == 0 {
                continue;
            }
            let end = offsets[i + 1..]
                .iter()
                .copied()
                .find(|&p| p != 0)
                .unwrap_or(b.len());
            if at != previous_end
                || end < at
                || end > b.len()
                || end - at < 32
                || (end - at - 16) % 8 != 0
            {
                return Err("invalid replay stage offsets".into());
            }
            let r = &b[at..end];
            if r[8] > 128 || r[9] > 8 || r[10] > 8 || r[11] > 32 {
                return Err("invalid replay stage state".into());
            }
            let mut inputs = Vec::new();
            for bytes in r[16..].chunks_exact(8) {
                // StopRecordingReplay can run twice (the shipped demo does).
                // Playback stops at the first sentinel and never reads that tail.
                if inputs
                    .last()
                    .is_some_and(|i: &ReplayInput| i.frame == END_FRAME)
                {
                    break;
                }
                let frame = u32_at(bytes, 0) as i32;
                if !(0..=END_FRAME).contains(&frame)
                    || inputs.last().is_some_and(|i: &ReplayInput| frame < i.frame)
                {
                    return Err("unordered replay inputs".into());
                }
                inputs.push(ReplayInput {
                    frame,
                    buttons: u16_at(bytes, 4) & CAPTURE_MASK,
                });
            }
            if inputs[0].frame != 0 || inputs.last().unwrap().frame != END_FRAME {
                return Err("replay is missing its initial input or sentinel".into());
            }
            out.stages[i] = Some(ReplayStage {
                score: u32_at(r, 0),
                random_seed: u16_at(r, 4),
                point_items: u16_at(r, 6),
                power: r[8],
                lives: r[9] as i8,
                bombs: r[10] as i8,
                rank: r[11],
                power_item_count: r[12] as i8,
                inputs,
            });
            previous_end = end;
        }
        if !out.stages.iter().any(Option::is_some) || previous_end != b.len() {
            return Err("replay contains no stage or has trailing data".into());
        }
        Ok(out)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![0u8; 80];
        b[..4].copy_from_slice(b"T6RP");
        b[4..6].copy_from_slice(&0x0102u16.to_le_bytes());
        b[6] = self.character;
        b[7] = self.difficulty;
        b[14] = 0x40;
        for (dst, &src) in b[16..24].iter_mut().zip(self.date.as_bytes()) {
            *dst = src;
        }
        for (dst, &src) in b[25..33]
            .iter_mut()
            .zip(encoding_rs::SHIFT_JIS.encode(&self.name).0.iter())
        {
            *dst = src;
        }
        put32(&mut b, 36, self.score);
        put32(&mut b, 40, (self.slowdown + 1.12).to_bits());
        put32(&mut b, 44, self.slowdown.to_bits());
        put32(&mut b, 48, (self.slowdown + 2.34).to_bits());
        for (i, stage) in self.stages.iter().enumerate() {
            let Some(s) = stage else { continue };
            let at = b.len();
            put32(&mut b, 52 + i * 4, at as u32);
            b.extend([0u8; 16]);
            put32(&mut b, at, s.score);
            b[at + 4..at + 6].copy_from_slice(&s.random_seed.to_le_bytes());
            b[at + 6..at + 8].copy_from_slice(&s.point_items.to_le_bytes());
            b[at + 8] = s.power;
            b[at + 9] = s.lives as u8;
            b[at + 10] = s.bombs as u8;
            b[at + 11] = s.rank;
            b[at + 12] = s.power_item_count as u8;
            for input in &s.inputs {
                b.extend(input.frame.to_le_bytes());
                b.extend(input.buttons.to_le_bytes());
                b.extend([0u8; 2]);
            }
        }
        let checksum = b[14..]
            .iter()
            .fold(0x3f000318u32, |sum, &v| sum.wrapping_add(v as u32));
        put32(&mut b, 8, checksum);
        let mut key = b[14];
        for byte in &mut b[15..] {
            *byte = byte.wrapping_add(key);
            key = key.wrapping_add(7);
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Replay {
        let mut stage = ReplayStage {
            score: 12345,
            random_seed: 0xbeef,
            point_items: 23,
            power: 128,
            lives: 2,
            bombs: 3,
            rank: 16,
            power_item_count: 4,
            inputs: vec![ReplayInput {
                frame: 0,
                buttons: 0,
            }],
        };
        stage.record(10, 1);
        stage.record(11, 1);
        stage.record(20, 0x44);
        stage.finish(100, 12345);
        let mut r = Replay {
            character: 3,
            difficulty: 4,
            date: "10/12/26".into(),
            name: "TEST".into(),
            score: 12345,
            slowdown: 0.,
            stages: std::array::from_fn(|_| None),
        };
        r.stages[6] = Some(stage);
        r
    }
    #[test]
    fn original_format_roundtrip_and_input_transitions() {
        let r = sample();
        let parsed = Replay::parse(&r.encode()).unwrap();
        assert_eq!(parsed, r);
        let s = parsed.stages[6].as_ref().unwrap();
        assert_eq!(s.inputs.len(), 5);
        assert_eq!(s.buttons_at(9), 0);
        assert_eq!(s.buttons_at(10), 1);
        assert_eq!(s.buttons_at(20), 0x44);
        assert_eq!(s.buttons_at(100), 0);
    }
    #[test]
    fn truncated_and_corrupt_replays_are_rejected() {
        let mut b = sample().encode();
        for n in 0..80 {
            assert!(Replay::parse(&b[..n]).is_err());
        }
        b[81] ^= 4;
        assert!(Replay::parse(&b).is_err());
    }

    #[test]
    fn repeated_original_stop_recording_tail_is_ignored() {
        let mut replay = sample();
        let stage = replay.stages[6].as_mut().unwrap();
        stage.inputs.extend([
            ReplayInput {
                frame: 100,
                buttons: 0,
            },
            ReplayInput {
                frame: END_FRAME,
                buttons: 0,
            },
        ]);
        assert_eq!(Replay::parse(&replay.encode()).unwrap(), sample());
    }

    #[test]
    #[ignore = "requires the original CM.DAT demo00.rpy extracted in TH06_CM_DIR"]
    fn reads_an_original_game_replay_and_preserves_its_inputs() {
        let directory = std::env::var("TH06_CM_DIR").expect("TH06_CM_DIR");
        let bytes = std::fs::read(std::path::Path::new(&directory).join("demo00.rpy")).unwrap();
        let replay = Replay::parse(&bytes).expect("original demo replay");
        assert_eq!(replay.difficulty, 3);
        assert!(replay.stages[3].is_some());
        assert_eq!(Replay::parse(&replay.encode()).unwrap(), replay);
    }
}
