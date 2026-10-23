//! Account for every guest instruction routed through the interpreter.
//! Read backing arrays directly: diagnostics must not trigger bus side effects.
use gba_core::Machine;
use std::{collections::BTreeMap, path::Path};

#[derive(Default)]
pub struct Audit {
    pub enabled: bool,
    rows: BTreeMap<(&'static str, u32, Vec<u8>), (u64, u64, u64)>,
}

impl Audit {
    pub fn record(&mut self, reason: &'static str, m: &Machine) {
        if !self.enabled {
            return;
        }
        let pc = m.cpu.regs[15];
        let key = pc | m.cpu.thumb() as u32;
        let (memory, offset): (&[u8], usize) = match pc >> 24 {
            2 => (&m.bus.ewram, (pc & 0x3ffff) as usize),
            3 => (&m.bus.iwram, (pc & 0x7fff) as usize),
            8..=13 => (&m.bus.rom, (pc & 0x1ffffff) as usize),
            _ => (&[], 0),
        };
        let bytes = memory.get(offset..).unwrap_or_default();
        let bytes = bytes[..bytes.len().min(32)].to_vec();
        let row = self
            .rows
            .entry((reason, key, bytes))
            .or_insert((0, m.bus.frames, 0));
        row.0 += 1;
        row.2 = m.bus.frames;
    }

    pub fn total(&self) -> u64 {
        self.rows.values().map(|row| row.0).sum()
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        self.write_to(std::io::BufWriter::new(std::fs::File::create_new(path)?))
    }

    /// Only used for the recorder's own partial file inside a new session folder.
    pub fn checkpoint(&self, path: &Path) -> std::io::Result<()> {
        self.write_to(std::io::BufWriter::new(std::fs::File::create(path)?))
    }

    fn write_to(&self, mut out: impl std::io::Write) -> std::io::Result<()> {
        writeln!(out, "reason,key,steps,first_frame,last_frame,bytes32")?;
        for ((reason, key, bytes), (steps, first, last)) in &self.rows {
            write!(out, "{reason},{key:08x},{steps},{first},{last},")?;
            for byte in bytes {
                write!(out, "{byte:02x}")?;
            }
            writeln!(out)?;
        }
        out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_mode_reason_and_changed_ram_without_touching_machine() {
        let mut m = Machine::new(vec![0; 4]);
        m.cpu.regs[15] = 0x03000000;
        let mut audit = Audit {
            enabled: true,
            ..Audit::default()
        };
        audit.record("dispatch", &m);
        audit.record("dispatch", &m);
        m.bus.iwram[0] = 1;
        audit.record("dispatch", &m);
        audit.record("guard", &m);
        m.cpu.cpsr |= gba_core::cpu::FLAG_T;
        audit.record("dispatch", &m);
        assert_eq!(audit.total(), 5);
        assert_eq!(audit.rows.len(), 4);
        assert_eq!(m.cpu.regs[15], 0x03000000);
        assert_eq!(m.bus.frames, 0);
    }
}
