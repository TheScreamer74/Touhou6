//! Optional intent/voice trace, independent of an audio output device.
//! `TH06_AUDIO_TRACE=/outside/repo/trace.csv` enables the native diagnostic.
//! Frame IDs are supplied by the game before each calculation-chain pass;
//! queue flush is tagged after the final replay pass, as in main.cpp.
use std::io::{BufWriter, Write};
use std::sync::{Mutex, OnceLock};

struct Trace {
    writer: BufWriter<std::fs::File>,
    present: u64,
    frame: i32,
    stage: usize,
    queue: Vec<usize>,
    bgm: Option<String>,
}
impl Trace {
    fn row(&mut self, event: &str, value: &str, detail: &str) {
        let _ = writeln!(
            self.writer,
            "{},{},{},{event},{value},{detail}",
            self.present, self.frame, self.stage
        );
    }
    fn sfx(&mut self, index: u8) {
        let index = usize::from(index);
        self.row("SFX_REQUEST", &index.to_string(), "");
        if index < super::SFX_VARIANTS.len() && self.queue.len() < 3 && !self.queue.contains(&index)
        {
            self.queue.push(index);
        }
    }
    fn score_extend(&mut self) {
        // GameManager requests SOUND_1UP before gameFrames++; ItemManager's
        // life pickup requests the same sound after that increment.
        let frame = self.frame;
        if frame >= 0 {
            self.frame -= 1;
        }
        self.sfx(28);
        self.frame = frame;
    }
}
fn with_trace(action: impl FnOnce(&mut Trace)) {
    static TRACE: OnceLock<Mutex<Option<Trace>>> = OnceLock::new();
    let trace = TRACE.get_or_init(|| {
        let enabled = std::env::var_os("TH06_AUDIO_TRACE")
            .and_then(|path| std::fs::File::create(path).ok())
            .map(|file| {
                let mut writer = BufWriter::new(file);
                let _ = writeln!(writer, "present,frame,stage,event,value,detail");
                Trace {
                    writer,
                    present: 0,
                    frame: -1,
                    stage: 0,
                    queue: Vec::new(),
                    bgm: None,
                }
            });
        Mutex::new(enabled)
    });
    if let Ok(mut guard) = trace.lock()
        && let Some(trace) = guard.as_mut()
    {
        action(trace);
    }
}

pub fn context(frame: Option<i32>, stage: usize) {
    with_trace(|trace| {
        trace.frame = frame.unwrap_or(-1);
        trace.stage = stage;
    });
}
pub fn sfx(index: u8) {
    with_trace(|trace| trace.sfx(index));
}
pub fn score_extend() {
    with_trace(Trace::score_extend);
}
pub fn sfx_named(name: &str) {
    if let Some(index) = super::SFX_VARIANTS
        .iter()
        .position(|(file, _)| super::SFX_FILES[*file] == name)
    {
        sfx(index as u8);
    }
}
pub fn bgm(name: &str, enabled: bool) {
    with_trace(|trace| {
        trace.row("BGM_REQUEST", name, "");
        if !enabled {
            return;
        }
        if let Some(old) = trace.bgm.take() {
            trace.row("BGM_STOP", &old, "");
        }
        trace.row("BGM_PLAY", name, "1");
        trace.bgm = Some(name.to_string());
    });
}
pub fn stop() {
    with_trace(|trace| {
        trace.row("BGM_STOP_REQUEST", "", "");
        if let Some(old) = trace.bgm.take() {
            trace.row("BGM_STOP", &old, "");
        }
    });
}
pub fn fade(seconds: f32) {
    with_trace(|trace| trace.row("BGM_FADE", &seconds.to_string(), ""));
}
pub fn flush(enabled: bool) {
    with_trace(|trace| {
        if enabled {
            for index in std::mem::take(&mut trace.queue) {
                trace.row("SFX_PLAY", &index.to_string(), "");
            }
        }
        let _ = trace.writer.flush();
        trace.present += 1;
    });
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn score_extend_clock_precedes_item_life_without_moving_presentation() {
        let path = std::env::temp_dir().join(format!(
            "th06-sound-clock-{}-{}.csv",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut trace = Trace {
            writer: BufWriter::new(std::fs::File::create(&path).unwrap()),
            present: 12,
            frame: 17,
            stage: 4,
            queue: Vec::new(),
            bgm: None,
        };
        trace.score_extend();
        trace.sfx(0);
        trace.sfx(28); // ItemManager life pickup, after gameFrames++.
        assert_eq!((trace.present, trace.frame, trace.stage), (12, 17, 4));
        assert_eq!(trace.queue, [28, 0]);
        trace.writer.flush().unwrap();
        drop(trace);
        let rows = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            rows,
            "12,16,4,SFX_REQUEST,28,\n12,17,4,SFX_REQUEST,0,\n12,17,4,SFX_REQUEST,28,\n"
        );
    }
}
