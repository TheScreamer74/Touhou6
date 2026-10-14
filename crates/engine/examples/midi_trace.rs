//! Emit the same event stream and clock digest as refcap/midi_trace.h.
//! Game MIDI data and output CSVs must remain outside the source repository.
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use th06_engine::midi::{Message, Sequence};

fn hash_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let directory = PathBuf::from(args.next().ok_or("usage: midi_trace MIDI_DIR OUTPUT_DIR")?);
    let output = PathBuf::from(args.next().ok_or("missing OUTPUT_DIR")?);
    std::fs::create_dir_all(&output)?;
    let mut events = BufWriter::new(std::fs::File::create(output.join("midi-events.csv"))?);
    let mut summary = BufWriter::new(std::fs::File::create(output.join("midi-summary.csv"))?);
    writeln!(events, "track,callback,status,a,b,sysex")?;
    writeln!(summary, "track,callbacks,messages,clock_hash")?;
    for track in 1..=17 {
        let bytes = std::fs::read(directory.join(format!("th06_{track:02}.mid")))?;
        let mut sequence = Sequence::parse(&bytes)?;
        let mut hash = 14_695_981_039_346_656_037u64;
        let mut messages = 0;
        for callback in 0..603_000 {
            if callback == 600_000 {
                sequence.fade_milliseconds(2000);
            }
            for message in sequence.tick() {
                match message {
                    Message::Short(status, a, b) => {
                        writeln!(events, "{track},{callback},{status},{a},{b},")?
                    }
                    Message::SystemExclusive(bytes) => {
                        write!(events, "{track},{callback},240,0,0,")?;
                        for byte in bytes {
                            write!(events, "{byte:02x}")?;
                        }
                        writeln!(events)?;
                    }
                }
                messages += 1;
            }
            let (tempo, milliseconds, elapsed_ticks) = sequence.reference_clock();
            hash = hash_bytes(hash, &tempo.to_le_bytes());
            hash = hash_bytes(hash, &milliseconds.to_le_bytes());
            hash = hash_bytes(hash, &elapsed_ticks.to_le_bytes());
        }
        writeln!(summary, "{track},603000,{messages},{hash:016x}")?;
    }
    Ok(())
}
