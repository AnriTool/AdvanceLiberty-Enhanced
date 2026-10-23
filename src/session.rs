//! Reproducible human play traces, separate from cartridge progress.
use gba_core::Machine;
use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

pub struct Session {
    directory: PathBuf,
    input: BufWriter<File>,
    frames: BufWriter<File>,
    previous_keys: Option<u16>,
}

impl Session {
    pub fn open(
        directory: PathBuf,
        m: &Machine,
        rom: &str,
        libraries: &[String],
    ) -> io::Result<Self> {
        // Never reuse a directory or overwrite another session's data.
        fs::create_dir(&directory)?;
        let mut input = BufWriter::new(File::create_new(directory.join("input.gba-input"))?);
        writeln!(input, "gba-input v1")?;
        input.flush()?;
        let mut frames = BufWriter::new(File::create_new(directory.join("frames.csv"))?);
        writeln!(frames, "frame,framebuffer_upstream_hash,audio_samples_hash")?;
        frames.flush()?;
        if let Some(save) = m.bus.save_data() {
            let mut initial = File::create_new(directory.join("initial.sav"))?;
            initial.write_all(save)?;
        }
        let mut metadata = File::create_new(directory.join("info.txt"))?;
        writeln!(metadata, "advance-session v1\nrom={}\ninitial_frame={}\ninput=input.gba-input\ninitial_save=initial.sav", std::path::absolute(rom)?.display(), m.bus.frames)?;
        for library in libraries {
            writeln!(
                metadata,
                "library={}",
                std::path::absolute(library)?.display()
            )?;
        }
        Ok(Self {
            directory,
            input,
            frames,
            previous_keys: None,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn input(&mut self, frame: u64, keys: u16) -> io::Result<()> {
        if self.previous_keys != Some(keys) {
            writeln!(self.input, "{frame} {keys:03x}")?;
            self.previous_keys = Some(keys);
        }
        Ok(())
    }

    pub fn frame(&mut self, m: &Machine) -> io::Result<()> {
        let samples: Vec<_> = m.bus.audio_buf.iter().map(|&s| s as u16).collect();
        writeln!(
            self.frames,
            "{},{:016x},{:016x}",
            m.bus.frames,
            crate::hash(&m.bus.framebuffer),
            crate::hash(&samples)
        )?;
        if m.bus.frames % 60 == 0 {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.input.flush()?;
        self.frames.flush()
    }

    pub fn finish(&mut self, frames: u64, native: u64, interpreted: u64) -> io::Result<()> {
        self.flush()?;
        let mut summary = File::create_new(self.directory.join("summary.txt"))?;
        writeln!(
            summary,
            "frames={frames}\nnative_dispatches={native}\ninterpreted_steps={interpreted}"
        )?;
        println!("Recorded session: {}", self.directory.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_effective_input_without_repeated_rows_and_refuses_existing_directory() {
        let parent = std::env::temp_dir();
        let directory = parent.join(format!(
            "advance-session-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let m = Machine::new(vec![0; 4]);
        let mut session = Session::open(directory.clone(), &m, "test.gba", &[]).unwrap();
        session.input(0, 0x3ff).unwrap();
        session.input(1, 0x3ff).unwrap();
        session.input(2, 0x3fe).unwrap();
        session.finish(3, 10, 2).unwrap();
        assert_eq!(
            fs::read_to_string(directory.join("input.gba-input")).unwrap(),
            "gba-input v1\n0 3ff\n2 3fe\n"
        );
        assert!(Session::open(directory.clone(), &m, "test.gba", &[]).is_err());
        assert!(fs::read_to_string(directory.join("summary.txt"))
            .unwrap()
            .contains("interpreted_steps=2"));
    }
}
