//! ReplayManager orchestration. The on-disk representation is the original T6RP.
use crate::{save, stage::Stage};
use th06_engine::Input;
use th06_formats::replay::{CAPTURE_MASK, Replay};

pub const SLOTS: usize = 15;

pub fn path(directory: &str, slot: usize) -> String {
    format!(
        "{}/th6_{:02}.rpy",
        directory.trim_end_matches('/'),
        slot + 1
    )
}

pub fn load_slots(directory: &str) -> Vec<Option<Replay>> {
    (0..SLOTS)
        .map(|slot| save::load_bytes(&path(directory, slot)).and_then(|b| Replay::parse(&b).ok()))
        .collect()
}

pub enum Session {
    Recording {
        replay: Replay,
        frame: i32,
    },
    Playback {
        replay: Replay,
        frame: i32,
        previous: u16,
    },
}

impl Session {
    pub fn recording(character: u8, difficulty: u8, date: String) -> Self {
        Self::Recording {
            replay: Replay {
                character,
                difficulty,
                date,
                name: String::new(),
                score: 0,
                slowdown: 0.0,
                stages: std::array::from_fn(|_| None),
            },
            frame: 0,
        }
    }
    pub fn playback(replay: Replay) -> Self {
        Self::Playback {
            replay,
            frame: 0,
            previous: 0,
        }
    }
    pub fn is_playback(&self) -> bool {
        matches!(self, Self::Playback { .. })
    }
    pub fn data(&self) -> &Replay {
        match self {
            Self::Recording { replay, .. } | Self::Playback { replay, .. } => replay,
        }
    }
    pub fn begin_stage(&mut self, index: usize, stage: &mut Stage) -> Result<(), String> {
        match self {
            Self::Recording { replay, frame } => {
                *frame = 0;
                replay.stages[index] = Some(stage.replay_state());
            }
            Self::Playback {
                replay,
                frame,
                previous,
            } => {
                let state = replay.stages[index]
                    .as_ref()
                    .ok_or("Replay has no data for this stage")?;
                let score = index
                    .checked_sub(1)
                    .and_then(|i| replay.stages[i].as_ref())
                    .map_or(0, |s| s.score);
                stage.restore_replay_state(state, score);
                *frame = 0;
                *previous = 0;
            }
        }
        Ok(())
    }
    pub fn input(&mut self, index: usize, stage: &Stage, live: &Input) -> Input {
        if !stage.replay_frame_active(live) {
            return live.clone();
        }
        match self {
            Self::Recording { replay, frame } => {
                if let Some(s) = &mut replay.stages[index] {
                    s.record(*frame, live.buttons());
                }
                *frame += 1;
                live.clone()
            }
            Self::Playback {
                replay,
                frame,
                previous,
            } => {
                let buttons = replay.stages[index]
                    .as_ref()
                    .map_or(0, |s| s.buttons_at(*frame));
                let combined = (live.buttons() & !CAPTURE_MASK) | buttons;
                let input = Input::from_buttons(combined, *previous);
                *previous = combined;
                *frame += 1;
                input
            }
        }
    }
    pub fn end_stage(&mut self, index: usize, score: i64) {
        if let Self::Recording { replay, frame } = self {
            replay.score = score.clamp(0, u32::MAX as i64) as u32;
            if let Some(s) = &mut replay.stages[index] {
                s.finish(*frame, replay.score);
            }
        }
    }
    pub fn save(&mut self, directory: &str, slot: usize, name: &str) -> Result<(), String> {
        if slot >= SLOTS {
            return Err("Invalid replay slot".into());
        }
        let Self::Recording { replay, .. } = self else {
            return Err("Cannot overwrite a replay during playback".into());
        };
        replay.name = name.chars().take(8).collect();
        save::store_bytes(&path(directory, slot), &replay.encode())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn date() -> String {
    // Civil date from Unix days; avoids adding a platform date dependency.
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86400;
    let z = days as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    y += i64::from(month <= 2);
    format!("{month:02}/{day:02}/{:02}", y % 100)
}
#[cfg(target_arch = "wasm32")]
pub fn date() -> String {
    let d = js_sys::Date::new_0();
    format!(
        "{:02}/{:02}/{:02}",
        d.get_month() + 1,
        d.get_date(),
        d.get_full_year() % 100
    )
}

pub fn seed() -> u16 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u16
    }
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64 as u16
    }
}
