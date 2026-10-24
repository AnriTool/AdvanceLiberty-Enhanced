//! Offline scene-span replay. Copies never call the live instrumentation/API.
use crate::{ground_projection, native_host, BlockFn};
use gba_core::{
    capi::{RtApi, RT_API},
    Machine, Mode,
};
use std::{
    cell::Cell,
    collections::HashMap,
    ffi::c_void,
    io::Write,
    path::{Path, PathBuf},
};

const ENTRY: u32 = 0x08009e93; // before camera X/Y truncation and derived bounds
const END: u32 = 0x08000327; // after building upload in the main loop
thread_local! { static DEFERRED: Cell<bool> = const { Cell::new(false) }; }
extern "C" fn defer(m: *mut c_void) -> u32 {
    DEFERRED.with(|d| d.set(true));
    let m = unsafe { &*(m as *const Machine) };
    m.cpu.regs[15] | m.cpu.thumb() as u32
}
fn service(m: &Machine) -> bool {
    m.bus.halted
        || (m.cpu.regs[15] == gba_core::machine::IRQ_RETURN_ADDR && m.cpu.mode() == Mode::Irq)
        || (m.bus.irq_pending() && !m.cpu.flag(gba_core::cpu::FLAG_I))
}
fn step(m: &mut Machine, api: &RtApi, table: &HashMap<u32, Vec<BlockFn>>) -> Result<(), String> {
    if service(m) {
        if m.bus.halted
            && m.bus.irq_line()
            && !(m.bus.irq_pending() && !m.cpu.flag(gba_core::cpu::FLAG_I))
        {
            return Err("Fork would interpret an instruction on HALT wake".into());
        }
        m.step();
        return Ok(());
    }
    let key = m.cpu.regs[15] | m.cpu.thumb() as u32;
    let entries = table
        .get(&key)
        .ok_or_else(|| format!("Fork native miss {key:08x}"))?;
    for f in entries {
        DEFERRED.with(|d| d.set(false));
        let clock = m.bus.clock;
        f(api, m as *mut Machine as *mut c_void);
        if !DEFERRED.with(|d| d.get()) {
            return Ok(());
        }
        if m.bus.clock != clock || m.cpu.regs[15] | m.cpu.thumb() as u32 != key {
            return Err("Fork rejected candidate changed PC/clock".into());
        }
    }
    Err(format!("Fork has no accepted native variant {key:08x}"))
}
// Bounded presentation-only native span. A missing AOT block is an error;
// this path never enters the interpreter or the live instrumentation API.
pub(crate) fn presentation_span(m: &mut Machine, end: u32) -> Result<(), String> {
    let api = native_host::Api::new(RtApi {
        interp_one: defer,
        ..RT_API
    });
    m.cpu.cpsr |= gba_core::cpu::FLAG_I;
    crate::RESCUE.with(|table| {
        let table = table.borrow();
        for _ in 0..200_000 {
            if key(m) == end {
                return Ok(());
            }
            if m.bus.halted {
                return Err("Presentation span halted".into());
            }
            step(m, &api.base, &table)?;
        }
        Err("Presentation span exceeded budget".into())
    })
}
pub(crate) fn presentation_observe(
    m: &mut Machine,
    end: u32,
    mut observe: impl FnMut(&mut Machine, u32) -> Result<(), String>,
) -> Result<(), String> {
    let api = native_host::Api::new(RtApi {
        interp_one: defer,
        ..RT_API
    });
    m.cpu.cpsr |= gba_core::cpu::FLAG_I;
    crate::RESCUE.with(|table| {
        let table = table.borrow();
        for _ in 0..1_000_000 {
            let pc = key(m);
            if pc == end {
                return Ok(());
            }
            if m.bus.halted {
                return Err("Presentation geometry halted".into());
            }
            observe(m, pc)?;
            if key(m) != pc {
                continue;
            }
            step(m, &api.base, &table)?;
        }
        Err("Presentation geometry exceeded budget".into())
    })
}
fn key(m: &Machine) -> u32 {
    m.cpu.regs[15] | m.cpu.thumb() as u32
}
fn host_boundary(m: &mut Machine, keys: &[(u64, u16)]) {
    m.bus.audio_buf.clear();
    let i = keys.partition_point(|&(f, _)| f <= m.bus.frames);
    m.bus.keys = if i == 0 { 0x3ff } else { keys[i - 1].1 };
    m.bus.frame_ready = false;
}
fn camera(m: &Machine) -> Vec<u8> {
    m.bus.iwram[0x14..0x20].to_vec()
}
fn changed(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}
fn difference(a: &Machine, b: &Machine) -> Option<&'static str> {
    if a.cpu != b.cpu {
        return Some("CPU including banked state");
    }
    if (a.bus.clock, a.bus.frames, a.bus.frame_ready)
        != (b.bus.clock, b.bus.frames, b.bus.frame_ready)
    {
        return Some("time/frame");
    }
    for (name, left, right) in [
        ("ewram", &a.bus.ewram, &b.bus.ewram),
        ("iwram", &a.bus.iwram, &b.bus.iwram),
        ("io", &a.bus.io, &b.bus.io),
        ("vram", &a.bus.vram, &b.bus.vram),
        ("oam", &a.bus.oam, &b.bus.oam),
        ("palette", &a.bus.palette, &b.bus.palette),
        ("sram", &a.bus.sram, &b.bus.sram),
        ("bios", &a.bus.bios, &b.bus.bios),
    ] {
        if left != right {
            return Some(name);
        }
    }
    if a.bus.framebuffer != b.bus.framebuffer {
        return Some("framebuffer");
    }
    if a.bus.audio_buf != b.bus.audio_buf {
        return Some("audio");
    }
    if a.bus.save_data() != b.bus.save_data() {
        return Some("cartridge save");
    }
    if (
        a.bus.reg_ie,
        a.bus.reg_if,
        a.bus.ime,
        a.bus.halted,
        a.bus.intr_wait_armed,
        a.bus.keys,
    ) != (
        b.bus.reg_ie,
        b.bus.reg_if,
        b.bus.ime,
        b.bus.halted,
        b.bus.intr_wait_armed,
        b.bus.keys,
    ) {
        return Some("IRQ/HALT/input");
    }
    if (
        a.bus.aff_ref,
        a.bus.aff_dirty,
        a.bus.bios_open,
        a.bus.pc_in_bios,
        a.bus.unhandled_swis,
        a.bus.fifo_sample,
    ) != (
        b.bus.aff_ref,
        b.bus.aff_dirty,
        b.bus.bios_open,
        b.bus.pc_in_bios,
        b.bus.unhandled_swis,
        b.bus.fifo_sample,
    ) {
        return Some("exposed device latches");
    }
    None
}
pub(crate) fn ppm(path: &Path, pixels: &[u16], width: usize, height: usize) -> std::io::Result<()> {
    assert_eq!(pixels.len(), width * height);
    let mut file = std::io::BufWriter::new(std::fs::File::create_new(path)?);
    write!(file, "P6\n{width} {height}\n255\n")?;
    for p in pixels {
        for shift in [0, 5, 10] {
            let c = ((p >> shift) & 31) as u8;
            file.write_all(&[(c << 3) | (c >> 2)])?;
        }
    }
    file.flush()
}
struct Trial {
    end: Machine,
    present: Machine,
    end_checked: bool,
}
pub struct Replay {
    root: PathBuf,
    report: std::fs::File,
    start: u64,
    count: u32,
    done: u32,
    active: Option<Trial>,
    rejections: std::fs::File,
    offsets: bool,
}
impl Replay {
    pub fn open(root: PathBuf, start: u64, count: u32, offsets: bool) -> std::io::Result<Self> {
        std::fs::create_dir(&root)?;
        let mut report = std::fs::File::create_new(root.join("trials.csv"))?;
        let mut rejections = std::fs::File::create_new(root.join("rejections.csv"))?;
        writeln!(rejections, "start_frame,reason")?;
        writeln!(report,"trial,start_frame,upload_frame,present_frame,native_steps,building_calls,ground_calls,obj_calls,upload_calls,building_changed_bytes,oam_changed_bytes,camera_unchanged,end_state,present_state")?;
        Ok(Self {
            root,
            report,
            start,
            count,
            done: 0,
            active: None,
            rejections,
            offsets,
        })
    }
    pub fn observe(
        &mut self,
        m: &Machine,
        table: &HashMap<u32, Vec<BlockFn>>,
        keys: &[(u64, u16)],
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(trial) = self.active.as_mut() {
            if !trial.end_checked && key(m) == END {
                if let Some(diff) = difference(m, &trial.end) {
                    return Err(format!("Scene replay upload mismatch: {diff}").into());
                }
                trial.end_checked = true;
                self.report.write_all(b"MATCH,")?;
                self.report.flush()?;
            }
            return Ok(());
        }
        if self.done == self.count || m.bus.frames < self.start || key(m) != ENTRY {
            return Ok(());
        }
        if m.cpu.regs[7] != 0x03000014 {
            return Err("Camera projection base r7 disagrees with USA structure".into());
        }
        let mut fork = m.try_fork_for_research()?;
        let _pool_guard = crate::object_projection::pool_guard();
        let api = native_host::Api::new(RtApi {
            interp_one: defer,
            write8: crate::object_projection::write8,
            write16: crate::object_projection::write16,
            write32: crate::object_projection::write32,
            ..RT_API
        });
        let initial_camera = camera(m);
        let mut stages = [0u64; 4];
        let mut ground = ground_projection::Observer::default();
        let mut buildings = crate::building_projection::Observer::default();
        let mut steps = 0u64;
        loop {
            if fork.bus.frame_ready {
                host_boundary(&mut fork, keys);
            }
            if !service(&fork) && key(&fork) == END {
                break;
            }
            if steps > 0 && !service(&fork) && key(&fork) == ENTRY {
                writeln!(
                    self.rejections,
                    "{},scene_replaced_before_upload",
                    m.bus.frames
                )?;
                self.rejections.flush()?;
                return Ok(());
            }
            for (i, entry) in [0x0800d2b1, 0x08010545, 0x080127ed, 0x0800fe29]
                .iter()
                .enumerate()
            {
                if key(&fork) == *entry {
                    stages[i] += 1;
                }
            }
            ground.observe(&fork, key(&fork))?;
            buildings.observe(&fork, key(&fork))?;
            step(&mut fork, &api.base, table)?;
            steps += 1;
            if steps > 20_000_000 || fork.bus.frames > m.bus.frames + 12 {
                return Err("Scene replay span budget exceeded".into());
            }
        }
        if stages != [1, 1, 1, 1] {
            writeln!(self.rejections, "{},not_one_complete_scene", m.bus.frames)?;
            self.rejections.flush()?;
            return Ok(());
        }
        if initial_camera != camera(&fork) {
            writeln!(
                self.rejections,
                "{},camera_changed_before_upload",
                m.bus.frames
            )?;
            self.rejections.flush()?;
            return Ok(());
        }
        let dir = self.root.join(format!("trial-{:02}", self.done));
        std::fs::create_dir(&dir)?;
        std::fs::write(dir.join("entry.iwram"), &m.bus.iwram)?;
        std::fs::write(dir.join("entry.ewram"), &m.bus.ewram)?;
        std::fs::write(dir.join("upload.iwram"), &fork.bus.iwram)?;
        ground.write(&dir.join("ground-model.csv"))?;
        buildings.export(&fork.bus, &dir.join("buildings"))?;
        if self.done == 0 {
            ground_projection::export(&fork.bus, &self.root.join("ground-fractional"))?;
        }
        let mut present = fork.try_fork_for_research()?;
        let mut present_steps = 0u64;
        while !present.bus.frame_ready {
            step(&mut present, &api.base, table)?;
            present_steps += 1;
            if present_steps > 20_000_000 {
                return Err("Scene replay presentation budget exceeded".into());
            }
        }
        ppm(&dir.join("fork.ppm"), &present.bus.framebuffer, 240, 160)?;
        if self.offsets && self.done == 0 {
            self.offset_variants(m, &fork, &present, table, keys, &api.base)?;
        }
        write!(
            self.report,
            "{},{},{},{},{},{},{},{},{},{},{},{},",
            self.done,
            m.bus.frames,
            fork.bus.frames,
            present.bus.frames,
            steps,
            stages[0],
            stages[1],
            stages[2],
            stages[3],
            changed(&m.bus.ewram[0x36a00..], &fork.bus.ewram[0x36a00..]),
            changed(&m.bus.oam, &fork.bus.oam),
            initial_camera == camera(&fork)
        )?;
        self.report.flush()?;
        self.active = Some(Trial {
            end: fork,
            present,
            end_checked: false,
        });
        Ok(())
    }
    pub fn frame(&mut self, m: &Machine) -> Result<(), Box<dyn std::error::Error>> {
        let Some(trial) = self.active.as_ref() else {
            return Ok(());
        };
        if m.bus.frames < trial.present.bus.frames {
            return Ok(());
        }
        if !trial.end_checked {
            return Err("Scene reached presentation before upload was checked".into());
        }
        if let Some(diff) = difference(m, &trial.present) {
            return Err(format!("Scene replay presentation mismatch: {diff}").into());
        }
        ppm(
            &self.root.join(format!("trial-{:02}/live.ppm", self.done)),
            &m.bus.framebuffer,
            240,
            160,
        )?;
        writeln!(self.report, "MATCH")?;
        self.report.flush()?;
        self.done += 1;
        self.active = None;
        Ok(())
    }
    pub fn finish(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.active.is_some() || self.done != self.count {
            return Err(format!(
                "Incomplete scene replay: {} of {} trials",
                self.done, self.count
            )
            .into());
        }
        Ok(())
    }
    fn offset_variants(
        &self,
        source: &Machine,
        baseline: &Machine,
        baseline_frame: &Machine,
        table: &HashMap<u32, Vec<BlockFn>>,
        keys: &[(u64, u16)],
        api: &RtApi,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut report = std::fs::File::create_new(self.root.join("offsets.csv"))?;
        writeln!(report,"delta_x_raw,status,frame,changed_pixels,building_bytes,oam_bytes,ground_io_bytes,details")?;
        writeln!(
            report,
            "0,BASELINE,{},0,0,0,0,original_camera",
            baseline_frame.bus.frames
        )?;
        for delta in [-32768i32, 32768, 65536] {
            let result = (|| -> Result<(Machine, Machine, ground_projection::Observer), String> {
                let mut branch = source.try_fork_for_research().map_err(str::to_owned)?;
                let x = i32::from_le_bytes(branch.bus.iwram[0x14..0x18].try_into().unwrap());
                branch.bus.iwram[0x14..0x18].copy_from_slice(&x.wrapping_add(delta).to_le_bytes());
                let expected = camera(&branch);
                let mut stages = [0u32; 4];
                let mut ground = ground_projection::Observer::default();
                let mut steps = 0u64;
                loop {
                    if branch.bus.frame_ready {
                        host_boundary(&mut branch, keys);
                    }
                    if !service(&branch) && key(&branch) == END {
                        break;
                    }
                    if steps > 0 && !service(&branch) && key(&branch) == ENTRY {
                        return Err("scene_replaced_before_upload".into());
                    }
                    for (i, entry) in [0x0800d2b1, 0x08010545, 0x080127ed, 0x0800fe29]
                        .iter()
                        .enumerate()
                    {
                        if key(&branch) == *entry {
                            stages[i] += 1;
                        }
                    }
                    ground.observe(&branch, key(&branch))?;
                    step(&mut branch, api, table)?;
                    steps += 1;
                    if steps > 20_000_000 || branch.bus.frames > source.bus.frames + 12 {
                        return Err("span_budget".into());
                    }
                }
                if stages != [1, 1, 1, 1] {
                    return Err("not_one_complete_scene".into());
                }
                if camera(&branch) != expected {
                    return Err("camera_overwritten".into());
                }
                let mut present = branch.try_fork_for_research().map_err(str::to_owned)?;
                while !present.bus.frame_ready {
                    step(&mut present, api, table)?;
                    steps += 1;
                    if steps > 40_000_000 {
                        return Err("presentation_budget".into());
                    }
                }
                if present.bus.frames != baseline_frame.bus.frames || camera(&present) != expected {
                    return Err("presentation_phase_or_camera_changed".into());
                }
                Ok((branch, present, ground))
            })();
            match result {
                Ok((upload, present, ground)) => {
                    let dir = self.root.join(format!("offset-{delta}"));
                    std::fs::create_dir(&dir)?;
                    ground.write(&dir.join("ground-model.csv"))?;
                    ppm(&dir.join("frame.ppm"), &present.bus.framebuffer, 240, 160)?;
                    std::fs::write(dir.join("upload.iwram"), &upload.bus.iwram)?;
                    std::fs::write(dir.join("upload.oam"), &upload.bus.oam)?;
                    std::fs::write(dir.join("upload.io"), &upload.bus.io)?;
                    let pixels = present
                        .bus
                        .framebuffer
                        .iter()
                        .zip(&baseline_frame.bus.framebuffer)
                        .filter(|(a, b)| a != b)
                        .count();
                    writeln!(
                        report,
                        "{delta},RENDERED,{},{pixels},{},{},{},all_four_stages",
                        present.bus.frames,
                        changed(&upload.bus.ewram[0x36a00..], &baseline.bus.ewram[0x36a00..]),
                        changed(&upload.bus.oam, &baseline.bus.oam),
                        changed(&upload.bus.io[0x20..0x30], &baseline.bus.io[0x20..0x30])
                    )?;
                }
                Err(reason) => {
                    writeln!(report, "{delta},SKIPPED,,,,,,{}", reason.replace(',', ";"))?;
                }
            }
        }
        report.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn presentation_span_never_interprets_a_missing_native_block() {
        let mut m = Machine::new(vec![0; 32]);
        m.cpu.regs[15] = 0x08001000;
        m.cpu.cpsr |= gba_core::cpu::FLAG_T;
        let clock = m.bus.clock;
        let registers = m.cpu.regs;
        assert!(presentation_span(&mut m, 0x08001003)
            .unwrap_err()
            .contains("native miss"));
        assert_eq!(m.cpu.regs, registers);
        assert_eq!(m.bus.clock, clock);
    }
}
