//! Offline differential check. Never enabled by the normal launcher.
use gba_core::{capi::RT_API, Machine};
use std::{ffi::c_void, io::Write};

pub struct Check {
    output: std::fs::File,
    rom: Vec<u8>,
    remaining: u64,
    checked: u64,
    pub start_frame: u64,
}
impl Check {
    pub fn open(path: &str, start_frame: u64, count: u64, rom: &[u8]) -> std::io::Result<Self> {
        Ok(Self {
            output: std::fs::File::create_new(path)?,
            rom: rom.to_vec(),
            remaining: count,
            checked: 0,
            start_frame,
        })
    }
    pub fn snapshot(
        &mut self,
        m: &mut Machine,
    ) -> Result<Option<Machine>, Box<dyn std::error::Error>> {
        if self.remaining == 0 || m.bus.frames < self.start_frame {
            return Ok(None);
        }
        // ROM is immutable during execution. Avoid copying 16 MiB per block.
        let rom = std::mem::take(&mut m.bus.rom);
        let fork = m.try_fork_for_research();
        m.bus.rom = rom;
        let mut fork = fork?;
        fork.bus.rom = std::mem::take(&mut self.rom);
        Ok(Some(fork))
    }
    pub fn compare(
        &mut self,
        mut before: Machine,
        after: &Machine,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let pc = before.cpu.regs[15];
        let clock = before.bus.clock;
        let frame = before.bus.frames;
        let mut boundary = None;
        let mut steps = 0u64;
        while before.bus.clock < after.bus.clock && steps < 10_000_000 {
            (RT_API.interp_one)(&mut before as *mut Machine as *mut c_void);
            steps += 1;
            if boundary.is_none() && before.bus.frames != frame {
                boundary = Some((before.bus.clock, before.cpu.regs[15], steps));
            }
        }
        let mut differences = Vec::new();
        if before.bus.clock != after.bus.clock {
            differences.push(format!(
                "clock interpreter={} native={}",
                before.bus.clock, after.bus.clock
            ));
        }
        if before.cpu != after.cpu {
            differences.push(format!(
                "CPU interpreter={:08x?} cpsr={:08x} native={:08x?} cpsr={:08x}",
                before.cpu.regs, before.cpu.cpsr, after.cpu.regs, after.cpu.cpsr
            ));
        }
        for (name, left, right, base) in [
            ("ewram", &before.bus.ewram, &after.bus.ewram, 0x02000000u32),
            ("iwram", &before.bus.iwram, &after.bus.iwram, 0x03000000),
            ("io", &before.bus.io, &after.bus.io, 0x04000000),
            (
                "palette",
                &before.bus.palette,
                &after.bus.palette,
                0x05000000,
            ),
            ("vram", &before.bus.vram, &after.bus.vram, 0x06000000),
            ("oam", &before.bus.oam, &after.bus.oam, 0x07000000),
        ] {
            if let Some(i) = left.iter().zip(right).position(|(a, b)| a != b) {
                differences.push(format!(
                    "{name} first={:08x} interpreter={:02x} native={:02x} bytes_different={}",
                    base + i as u32,
                    left[i],
                    right[i],
                    left.iter().zip(right).filter(|(a, b)| a != b).count()
                ));
            }
        }
        if before.bus.framebuffer != after.bus.framebuffer {
            differences.push("framebuffer".to_string());
        }
        if before.bus.audio_buf != after.bus.audio_buf {
            differences.push(format!(
                "audio samples interpreter={} native={} first_diff={:?}",
                before.bus.audio_buf.len(),
                after.bus.audio_buf.len(),
                before
                    .bus
                    .audio_buf
                    .iter()
                    .zip(&after.bus.audio_buf)
                    .position(|(a, b)| a != b)
            ));
        }
        if (
            before.bus.reg_ie,
            before.bus.reg_if,
            before.bus.ime,
            before.bus.halted,
        ) != (
            after.bus.reg_ie,
            after.bus.reg_if,
            after.bus.ime,
            after.bus.halted,
        ) {
            differences.push("IRQ/HALT state".to_string());
        }
        self.rom = std::mem::take(&mut before.bus.rom);
        self.remaining -= 1;
        self.checked += 1;
        if let Some((boundary_clock, boundary_pc, boundary_steps)) = boundary {
            writeln!(self.output, "frame_boundary={} entry_pc={pc:08x} interpreter_pc={boundary_pc:08x} native_pc={:08x} interpreter_clock={boundary_clock} native_clock={} extra_cycles={} extra_instructions={}", after.bus.frames, after.cpu.regs[15], after.bus.clock, after.bus.clock.saturating_sub(boundary_clock), steps-boundary_steps)?;
        }
        if !differences.is_empty() {
            writeln!(self.output, "frame={} pc={pc:08x} start_clock={clock} native_cycles={} interpreter_steps={steps}\n{}", after.bus.frames, after.bus.clock-clock, differences.join("\n"))?;
            self.output.flush()?;
            return Err("Block differential mismatch (see --check-blocks report)".into());
        }
        if self.remaining == 0 {
            writeln!(self.output, "PASS: requested block comparisons completed")?;
            self.output.flush()?;
        }
        Ok(())
    }
    pub fn finish(&mut self) -> std::io::Result<()> {
        writeln!(
            self.output,
            "checked_blocks={} remaining_requested={}",
            self.checked, self.remaining
        )?;
        self.output.flush()
    }
}
