//! Read-only investigation of USA camera candidates. No presentation changes.
use gba_core::{capi::RT_API, Machine};
use std::{
    cell::RefCell,
    ffi::c_void,
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

struct Trace {
    frames: BufWriter<File>,
    writes: BufWriter<File>,
    start: u64,
    end: u64,
    error: Option<io::Error>,
}
thread_local! { static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) }; }

pub fn open(path: &Path, start: u64, end: u64) -> io::Result<()> {
    std::fs::create_dir(path)?;
    let mut frames = BufWriter::new(File::create_new(path.join("frames.csv"))?);
    let mut writes = BufWriter::new(File::create_new(path.join("writes.csv"))?);
    writeln!(frames, "frame,clock,x_raw,y_raw,z_raw,bg2_x,bg2_y")?;
    writeln!(writes, "frame,clock,pc,lr,address,width,value")?;
    TRACE.with(|t| {
        *t.borrow_mut() = Some(Trace {
            frames,
            writes,
            start,
            end,
            error: None,
        })
    });
    Ok(())
}
fn record(m: *mut c_void, address: u32, width: u32, value: u32) {
    // Only the candidate camera's three words, including overlapping stores.
    if address >> 24 != 3 {
        return;
    }
    let offset = address & 0x7fff;
    if offset >= 0x20 || offset + width <= 0x14 {
        return;
    }
    let m = unsafe { &*(m as *const Machine) };
    TRACE.with(|t| {
        if let Some(t) = t.borrow_mut().as_mut() {
            if (t.start..=t.end).contains(&m.bus.frames) && t.error.is_none() {
                if let Err(error) = writeln!(
                    t.writes,
                    "{},{},{:08x},{:08x},{address:08x},{width},{value:08x}",
                    m.bus.frames, m.bus.clock, m.cpu.regs[15], m.cpu.regs[14]
                ) {
                    t.error = Some(error);
                }
            }
        }
    });
}
pub extern "C" fn write8(m: *mut c_void, a: u32, v: u32) {
    record(m, a, 1, v & 0xff);
    (RT_API.write8)(m, a, v);
}
pub extern "C" fn write16(m: *mut c_void, a: u32, v: u32) {
    record(m, a, 2, v & 0xffff);
    (RT_API.write16)(m, a, v);
}
pub extern "C" fn write32(m: *mut c_void, a: u32, v: u32) {
    record(m, a, 4, v);
    (RT_API.write32)(m, a, v);
}

pub fn frame(m: &Machine) -> io::Result<()> {
    TRACE.with(|t| {
        let mut t = t.borrow_mut();
        let t = t.as_mut().expect("camera trace was opened");
        if let Some(error) = t.error.take() {
            return Err(error);
        }
        if (t.start..=t.end).contains(&m.bus.frames) {
            let word = |b: &[u8], i| i32::from_le_bytes(b[i..i + 4].try_into().unwrap());
            writeln!(
                t.frames,
                "{},{},{},{},{},{},{}",
                m.bus.frames,
                m.bus.clock,
                word(&m.bus.iwram, 0x14),
                word(&m.bus.iwram, 0x18),
                word(&m.bus.iwram, 0x1c),
                word(&m.bus.io, 0x28),
                word(&m.bus.io, 0x2c)
            )?;
        }
        Ok(())
    })
}
pub fn finish() -> io::Result<()> {
    TRACE.with(|t| {
        if let Some(t) = t.borrow_mut().as_mut() {
            if let Some(error) = t.error.take() {
                return Err(error);
            }
            t.frames.flush()?;
            t.writes.flush()?;
        }
        Ok(())
    })
}
