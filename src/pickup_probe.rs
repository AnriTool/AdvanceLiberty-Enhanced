//! Read-only tracing of the verified USA cash pool. Enabled only by --pickup-probe.
use gba_core::Machine;
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

const BASE: usize = 0x35cf0;
const STRIDE: usize = 0x68;
pub struct Probe {
    events: BufWriter<File>,
    states: BufWriter<File>,
    destroying: Option<u32>,
    world: BufWriter<File>,
}
impl Probe {
    pub fn open(path: &Path) -> io::Result<Self> {
        std::fs::create_dir(path)?;
        let mut result = Self {
            events: BufWriter::new(File::create_new(path.join("events.csv"))?),
            states: BufWriter::new(File::create_new(path.join("states.csv"))?),
            destroying: None,
            world: BufWriter::new(File::create_new(path.join("world-events.csv"))?),
        };
        writeln!(result.events,"frame,key,event,slot,active_mask,active,stage,subtick,collected,ticks,flags,affine_free,r0,r1,r2,r3,r4,r5,r6,r7,r8,r9,r10,r12,lr")?;
        writeln!(
            result.states,
            "frame,slot,active,stage,subtick,collected,ticks,flags,x_raw,y_raw,z_raw"
        )?;
        writeln!(result.world,"frame,key,event,entity,kind,flags,data,position,timer_address,r0,r1,r2,r3,r4,r5,r8,lr,affine_free")?;
        Ok(result)
    }
    #[inline]
    pub fn interested(key: u32) -> bool {
        matches!(
            key,
            0x08061b15
                | 0x08061b5b
                | 0x080607c9
                | 0x080607cd
                | 0x0806069f
                | 0x080605cb
                | 0x08060723
                | 0x08060665
                | 0x08061be7
                | 0x08060747
                | 0x0804976d
                | 0x080609f1
                | 0x0800d895
                | 0x08061da3
                | 0x08060781
        )
    }
    pub fn observe(&mut self, m: &Machine, key: u32) -> io::Result<()> {
        let r = &m.cpu.regs;
        let world_event = match key {
            0x0804976d => Some(("corpse_timer", r[5], r[1])),
            0x080609f1 => Some(("weapon_spawn", r[4], 0)),
            0x0800d895 => Some(("world_remove", r[0], 0)),
            0x08061da3 => Some(("resource_mark", r[2], 0)),
            _ => None,
        };
        if let Some((event, entity, timer)) = world_event {
            let b = &m.bus.ewram;
            let p = entity.wrapping_sub(0x02000000) as usize;
            let word = |offset: usize| {
                b.get(p + offset..p + offset + 4)
                    .map_or(0, |v| u32::from_le_bytes(v.try_into().unwrap()))
            };
            write!(
                self.world,
                "{},{key:08x},{event},{entity:08x},{},{:08x},{:08x},{:08x},{timer:08x}",
                m.bus.frames,
                b.get(p + 8).copied().unwrap_or(0),
                word(12),
                word(32),
                word(24)
            )?;
            for i in [0, 1, 2, 3, 4, 5, 8, 14] {
                write!(self.world, ",{:08x}", r[i])?;
            }
            writeln!(self.world, ",{}", affine_free(b).map_or(-1, |n| n as i32))?;
            return Ok(());
        }
        let (event, offset) = match key {
            0x08061be7 => match r[0] {
                1 => ("low_palette_reserve", None),
                // 0801314c counts free entries in the 32-entry OBJ affine pool.
                // Keep this event name stable for existing investigation CSVs.
                2 => ("low_graphics_reserve", None),
                3 => ("low_heap_reserve", None),
                _ => return Ok(()),
            },
            0x08061b15 => ("spawn_request", None),
            0x08061b5b => ("spawn_stage", r[12].checked_sub(0x02000000 + BASE as u32)),
            0x080607c9 => ("allocate_free", r[3].checked_sub(0x02000000 + BASE as u32)),
            0x080607cd => ("evict_oldest", r[7].checked_mul(STRIDE as u32)),
            0x0806069f => ("expire_stage", Some(r[8])),
            0x080605cb => ("collect", Some(r[8])),
            0x08060723 => {
                self.destroying = Some(r[8]);
                ("destroy", Some(r[8]))
            }
            0x08060747 if self.destroying == Some(r[8]) => {
                self.destroying = None;
                ("destroy_done", Some(r[8]))
            }
            0x08060665 => ("collected_animation_end", Some(r[8])),
            _ => return Ok(()),
        };
        let slot = offset
            .filter(|o| *o % STRIDE as u32 == 0 && *o / (STRIDE as u32) < 4)
            .map(|o| (o / STRIDE as u32) as usize);
        let b = &m.bus.ewram;
        let mask = (0..4).fold(0, |mask, i| {
            mask | (u32::from(b[BASE + i * STRIDE + 0x58] != 0) << i)
        });
        let p = BASE + slot.unwrap_or(0) * STRIDE;
        let word = |o| u32::from_le_bytes(b[p + o..p + o + 4].try_into().unwrap());
        write!(
            self.events,
            "{},{key:08x},{event},{},{mask},{},{},{},{},{},{},{}",
            m.bus.frames,
            slot.map_or(-1, |s| s as i32),
            b[p + 0x58],
            b[p + 0x5a],
            b[p + 0x59],
            b[p + 0x5b],
            word(0x60),
            word(0x64),
            affine_free(&m.bus.ewram).map_or(-1, |n| n as i32)
        )?;
        for i in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14] {
            write!(self.events, ",{:08x}", r[i])?;
        }
        writeln!(self.events)
    }
    pub fn frame(&mut self, m: &Machine) -> io::Result<()> {
        let b = &m.bus.ewram;
        for i in 0..4 {
            let p = BASE + i * STRIDE;
            let word = |o| u32::from_le_bytes(b[p + o..p + o + 4].try_into().unwrap());
            writeln!(
                self.states,
                "{},{i},{},{},{},{},{},{},{},{},{}",
                m.bus.frames,
                b[p + 0x58],
                b[p + 0x5a],
                b[p + 0x59],
                b[p + 0x5b],
                word(0x60),
                word(0x64),
                word(0x48) as i32,
                word(0x4c) as i32,
                word(0x50) as i32
            )?;
        }
        Ok(())
    }
    pub fn finish(&mut self) -> io::Result<()> {
        self.events.flush()?;
        self.world.flush()?;
        self.states.flush()
    }
}

pub fn affine_free(bytes: &[u8]) -> Option<usize> {
    let read = |offset: usize| {
        Some(u32::from_le_bytes(
            bytes.get(offset..offset + 4)?.try_into().ok()?,
        ))
    };
    let mut node = read(0x22e40)?;
    let mut count = 0;
    while node != 0 {
        let offset = node.checked_sub(0x02022ac0)? as usize;
        if offset % 28 != 0 || offset >= 32 * 28 || count >= 32 {
            return None;
        }
        count += 1;
        node = read(0x22ac0 + offset + 20)?;
    }
    Some(count)
}
