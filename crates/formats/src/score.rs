//! EoSD 1.02h score.dat, from ResultScreen::{OpenScore,WriteScore}.
//! File pointers are 32-bit offsets; no host structure layout is used.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighScore {
    pub name: String,
    pub score: u32,
    pub character: u8,
    pub difficulty: u8,
    pub stage: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpellcardRecord {
    pub capture_score: u32,
    pub name_checksum: u8,
    pub character_shot_type: [u8; 5],
    pub name: Vec<u8>,
    pub attempts: u16,
    pub successes: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreData {
    pub stages_cleared: [[u8; 5]; 4],
    pub stages_cleared_without_continues: [[u8; 5]; 4],
    pub practice_scores: [[[u32; 4]; 6]; 4],
    pub high_scores: Vec<HighScore>,
    pub spellcards: Vec<SpellcardRecord>,
}

impl Default for ScoreData {
    fn default() -> Self {
        Self {
            // ParseClrd makes stage 1 available for every shot and difficulty.
            stages_cleared: [[1; 5]; 4],
            stages_cleared_without_continues: [[1; 5]; 4],
            practice_scores: [[[0; 4]; 6]; 4],
            high_scores: Vec::new(),
            spellcards: vec![SpellcardRecord::default(); 64],
        }
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn record(magic: &[u8; 4], size: usize) -> Vec<u8> {
    let mut b = vec![0; size];
    b[..4].copy_from_slice(magic);
    put16(&mut b, 4, size as u16);
    put16(&mut b, 6, size as u16);
    b[8] = 16;
    b
}

impl ScoreData {
    pub fn parse(raw: &[u8]) -> Result<Self, String> {
        if !(20..=0xa0000).contains(&raw.len()) {
            return Err("invalid score.dat size".into());
        }
        let mut b = raw.to_vec();
        let mut xor = 0u8;
        for i in 2..b.len() {
            xor = xor.wrapping_add(b[i - 1]).rotate_left(3);
            b[i] ^= xor;
        }
        let checksum = b[4..].iter().fold(0u16, |a, &v| a.wrapping_add(v as u16));
        if checksum != u16_at(&b, 2) {
            return Err("score.dat checksum mismatch".into());
        }
        let end = u32_at(&b, 16) as usize;
        let mut at = u32_at(&b, 8) as usize;
        if end != b.len() || at < 20 || at > end {
            return Err("invalid score.dat offsets".into());
        }
        let mut out = Self::default();
        let mut header = false;
        while at < end {
            if end - at < 12 {
                return Err("truncated score.dat record".into());
            }
            let size = u16_at(&b, at + 4) as usize;
            if size < 12 || size > end - at {
                return Err("invalid score.dat record length".into());
            }
            let r = &b[at..at + size];
            if r[8] == 16 {
                match &r[..4] {
                    b"TH6K" => header = true,
                    b"HSCR" if size >= 28 && r[16] < 4 && r[17] < 5 => {
                        let name = &r[19..28];
                        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                        out.high_scores.push(HighScore {
                            name: encoding_rs::SHIFT_JIS
                                .decode_without_bom_handling(&name[..len])
                                .0
                                .into_owned(),
                            score: u32_at(r, 12),
                            character: r[16],
                            difficulty: r[17],
                            stage: r[18],
                        });
                    }
                    b"CLRD" if size >= 24 && r[22] < 4 => {
                        let c = r[22] as usize;
                        out.stages_cleared_without_continues[c].copy_from_slice(&r[12..17]);
                        out.stages_cleared[c].copy_from_slice(&r[17..22]);
                    }
                    b"PSCR" if size >= 20 && r[16] < 4 && r[17] < 4 && r[18] < 6 => {
                        out.practice_scores[r[16] as usize][r[18] as usize][r[17] as usize] =
                            u32_at(r, 12);
                    }
                    b"CATK" if size >= 64 && u16_at(r, 16) < 64 => {
                        let name = &r[24..58];
                        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                        out.spellcards[u16_at(r, 16) as usize] = SpellcardRecord {
                            capture_score: u32_at(r, 12),
                            name_checksum: r[18],
                            character_shot_type: r[19..24].try_into().unwrap(),
                            name: name[..len].to_vec(),
                            attempts: u16_at(r, 60),
                            successes: u16_at(r, 62),
                        };
                    }
                    _ => {}
                }
            }
            at += size;
        }
        if !header {
            return Err("score.dat has no TH6K header".into());
        }
        Ok(out)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![0u8; 20];
        b.extend(record(b"TH6K", 12));
        for d in 0..5 {
            for c in 0..4 {
                let mut scores: Vec<_> = self
                    .high_scores
                    .iter()
                    .filter(|s| s.character == c && s.difficulty == d)
                    .collect();
                scores.sort_by_key(|s| std::cmp::Reverse(s.score));
                for s in scores.into_iter().take(10) {
                    let mut r = record(b"HSCR", 28);
                    put32(&mut r, 12, s.score);
                    r[16] = c;
                    r[17] = d;
                    r[18] = s.stage;
                    for (dst, &src) in r[19..27]
                        .iter_mut()
                        .zip(encoding_rs::SHIFT_JIS.encode(&s.name).0.iter())
                    {
                        *dst = src;
                    }
                    b.extend(r);
                }
            }
        }
        for c in 0..4 {
            let mut r = record(b"CLRD", 24);
            r[12..17].copy_from_slice(&self.stages_cleared_without_continues[c]);
            r[17..22].copy_from_slice(&self.stages_cleared[c]);
            r[22] = c as u8;
            b.extend(r);
        }
        for (i, s) in self.spellcards.iter().take(64).enumerate() {
            if s.attempts == 0 && s.successes == 0 && s.name.is_empty() && s.capture_score == 0 {
                continue;
            }
            let mut r = record(b"CATK", 64);
            put32(&mut r, 12, s.capture_score);
            put16(&mut r, 16, i as u16);
            r[18] = s.name_checksum;
            r[19..24].copy_from_slice(&s.character_shot_type);
            for (dst, &src) in r[24..58].iter_mut().zip(&s.name) {
                *dst = src;
            }
            put16(&mut r, 60, s.attempts);
            put16(&mut r, 62, s.successes);
            b.extend(r);
        }
        for c in 0..4 {
            for stage in 0..6 {
                for d in 0..4 {
                    let score = self.practice_scores[c][stage][d];
                    if score == 0 {
                        continue;
                    }
                    let mut r = record(b"PSCR", 20);
                    put32(&mut r, 12, score);
                    r[16] = c as u8;
                    r[17] = d as u8;
                    r[18] = stage as u8;
                    b.extend(r);
                }
            }
        }
        put16(&mut b, 4, 16);
        put32(&mut b, 8, 20);
        let len = b.len() as u32;
        put32(&mut b, 16, len);
        let checksum = b[4..].iter().fold(0u16, |a, &v| a.wrapping_add(v as u16));
        put16(&mut b, 2, checksum);
        // A fixed seed is valid: it obscures bytes without consuming gameplay RNG.
        b[1] = 0x5a;
        let mut xor = b[1];
        for byte in &mut b[2..] {
            let original = *byte;
            xor = xor.rotate_left(3);
            *byte ^= xor;
            xor = xor.wrapping_add(original);
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_records_roundtrip_and_corruption_is_rejected() {
        let mut s = ScoreData::default();
        s.stages_cleared_without_continues[3][1] = 99;
        s.practice_scores[2][5][3] = 12345678;
        s.high_scores.push(HighScore {
            name: "Reimu".into(),
            score: 999999990,
            character: 0,
            difficulty: 4,
            stage: 99,
        });
        s.spellcards[63] = SpellcardRecord {
            attempts: 9,
            successes: 4,
            name: b"Spell".to_vec(),
            ..Default::default()
        };
        let mut encoded = s.encode();
        assert_eq!(ScoreData::parse(&encoded).unwrap(), s);
        let last = encoded.len() - 1;
        encoded[last] ^= 1;
        assert!(ScoreData::parse(&encoded).is_err());
    }
    #[test]
    fn short_and_invalid_files_are_safe() {
        for n in 0..100 {
            assert!(ScoreData::parse(&vec![0; n]).is_err());
        }
    }
}
