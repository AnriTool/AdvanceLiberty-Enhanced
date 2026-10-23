//! Read-only diagnostic screenshots of the original framebuffer.
use gba_core::Machine;
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Write},
    path::PathBuf,
};

pub struct Capture {
    directory: PathBuf,
    frames: BTreeSet<u64>,
}

impl Capture {
    pub fn open(directory: PathBuf, frames: BTreeSet<u64>) -> io::Result<Self> {
        fs::create_dir(&directory)?;
        Ok(Self { directory, frames })
    }

    pub fn frame(&self, m: &Machine) -> io::Result<()> {
        if !self.frames.contains(&m.bus.frames) {
            return Ok(());
        }
        let stem = self.directory.join(format!("{:06}", m.bus.frames));
        std::fs::write(stem.with_extension("ewram"), &m.bus.ewram)?;
        std::fs::write(stem.with_extension("oam"), &m.bus.oam)?;
        std::fs::write(stem.with_extension("vram"), &m.bus.vram)?;
        std::fs::write(stem.with_extension("io"), &m.bus.io)?;
        std::fs::write(stem.with_extension("iwram"), &m.bus.iwram)?;
        std::fs::write(stem.with_extension("palette"), &m.bus.palette)?;
        let mut raw = io::BufWriter::new(fs::File::create_new(stem.with_extension("rgb555"))?);
        let mut ppm = io::BufWriter::new(fs::File::create_new(stem.with_extension("ppm"))?);
        ppm.write_all(b"P6\n240 160\n255\n")?;
        for &pixel in &m.bus.framebuffer {
            raw.write_all(&pixel.to_le_bytes())?;
            let expand = |v: u16| ((v << 3) | (v >> 2)) as u8;
            ppm.write_all(&[
                expand(pixel & 31),
                expand((pixel >> 5) & 31),
                expand((pixel >> 10) & 31),
            ])?;
        }
        raw.flush()?;
        ppm.flush()
    }

    pub fn presentation(
        &self,
        frame: u64,
        image: &[u16],
        graphics: Option<&gba_core::MemMap>,
    ) -> io::Result<()> {
        if self.frames.contains(&frame) {
            if let Some(graphics) = graphics {
                let stem = self.directory.join(format!("{frame:06}-scene"));
                std::fs::write(stem.with_extension("oam"), &graphics.oam)?;
                std::fs::write(stem.with_extension("vram"), &graphics.vram)?;
                std::fs::write(stem.with_extension("io"), &graphics.io)?;
                std::fs::write(stem.with_extension("iwram"), &graphics.iwram)?;
                std::fs::write(stem.with_extension("palette"), &graphics.palette)?;
            }
            crate::scene_replay::ppm(
                &self.directory.join(format!("{frame:06}-enhanced.ppm")),
                image,
                720,
                480,
            )?;
        }
        Ok(())
    }
}
