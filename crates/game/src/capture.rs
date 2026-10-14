//! Native, deterministic capture input; no gameplay or oracle state changes.
use std::collections::BTreeSet;
use th06_engine::{Input, Key};

pub fn frames(value: &str) -> Result<BTreeSet<u32>, String> {
    let frames: BTreeSet<u32> = value
        .split(',')
        .map(|part| {
            part.trim()
                .parse()
                .map_err(|_| format!("Invalid capture frame: {part}"))
        })
        .collect::<Result<_, _>>()?;
    if frames.is_empty() || frames.contains(&0) {
        return Err("Capture frames count completed updates/replay ticks, starting at 1".into());
    }
    Ok(frames)
}

struct Range {
    start: u32,
    end: u32,
    key: Key,
}
#[derive(Default)]
pub struct Script {
    ranges: Vec<Range>,
    previous: Vec<Key>,
}
impl Script {
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut ranges = Vec::new();
        for (index, line) in source.lines().enumerate() {
            let fields: Vec<_> = line
                .split('#')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect();
            if fields.is_empty() {
                continue;
            }
            let number = |position: usize| -> Result<u32, String> {
                fields
                    .get(position)
                    .ok_or_else(|| format!("Line {} is incomplete", index + 1))?
                    .parse()
                    .map_err(|_| format!("Line {} has an invalid frame", index + 1))
            };
            let key = |position: usize| -> Result<Key, String> {
                match fields
                    .get(position)
                    .map(|name| name.to_ascii_uppercase())
                    .as_deref()
                {
                    Some("Z" | "SHOOT") => Ok(Key::Shoot),
                    Some("X" | "BOMB") => Ok(Key::Bomb),
                    Some("SHIFT" | "FOCUS") => Ok(Key::Focus),
                    Some("CTRL" | "CONTROL" | "SKIP") => Ok(Key::Skip),
                    Some("ESC" | "ESCAPE" | "PAUSE") => Ok(Key::Pause),
                    Some("Q" | "QUIT") => Ok(Key::Quit),
                    Some("S") => Ok(Key::S),
                    Some("HOME") => Ok(Key::Home),
                    Some("ENTER" | "RETURN") => Ok(Key::Enter),
                    Some("UP") => Ok(Key::Up),
                    Some("DOWN") => Ok(Key::Down),
                    Some("LEFT") => Ok(Key::Left),
                    Some("RIGHT") => Ok(Key::Right),
                    other => Err(format!(
                        "Line {} has an unsupported key: {other:?}",
                        index + 1
                    )),
                }
            };
            match fields[0] {
                "key" if fields.len() == 4 => {
                    let (start, end) = (number(1)?, number(2)?);
                    if start >= end {
                        return Err(format!(
                            "Line {} has an empty/reversed key range",
                            index + 1
                        ));
                    }
                    ranges.push(Range {
                        start,
                        end,
                        key: key(3)?,
                    });
                }
                "tap" if fields.len() == 5 => {
                    let (first, period, count, key) = (number(1)?, number(2)?, number(3)?, key(4)?);
                    if period < 5 || count > 100_000 {
                        return Err(format!(
                            "Line {} has overlapping or excessive taps",
                            index + 1
                        ));
                    }
                    for i in 0..count {
                        let start = i
                            .checked_mul(period)
                            .and_then(|offset| first.checked_add(offset))
                            .ok_or("Tap frame overflow")?;
                        ranges.push(Range {
                            start,
                            end: start.checked_add(4).ok_or("Tap frame overflow")?,
                            key,
                        });
                    }
                }
                _ => {
                    return Err(format!(
                        "Line {}: expected key <start> <end> <name> or tap <first> <period> <count> <name>",
                        index + 1
                    ));
                }
            }
        }
        Ok(Self {
            ranges,
            previous: Vec::new(),
        })
    }
    /// Script frames are zero-based and ranges are half-open, as in refcap.
    pub fn input(&mut self, frame: u32) -> Input {
        let mut held = Vec::new();
        for range in &self.ranges {
            if range.start <= frame && frame < range.end && !held.contains(&range.key) {
                held.push(range.key);
            }
        }
        let pressed: Vec<_> = held
            .iter()
            .filter(|key| !self.previous.contains(key))
            .copied()
            .collect();
        let input = Input::synthetic(&held, &pressed);
        self.previous = held;
        input
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_four_frame_taps_have_one_press_and_retrigger() {
        let mut script = Script::parse("# same as th06cap\nkey 0 20 SHIFT\ntap 1 8 2 Z").unwrap();
        assert!(!script.input(0).pressed(Key::Shoot));
        assert!(script.input(1).pressed(Key::Shoot));
        assert!(!script.input(2).pressed(Key::Shoot));
        assert!(!script.input(5).held(Key::Shoot));
        assert!(script.input(9).pressed(Key::Shoot));
        assert!(Script::parse("tap 0 2 5 Z").is_err());
        assert!(Script::parse("key 20 10 X").is_err());
        assert!(frames("0,1").is_err());
        assert_eq!(frames("60,1,60").unwrap().len(), 2);
    }
}
