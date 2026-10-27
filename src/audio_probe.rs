//! Opt-in, read-only USA sound-driver diagnostics; no host playback involved.
use gba_core::{capi::RT_API, Machine};
use std::{
    cell::RefCell,
    ffi::c_void,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

thread_local! { static TRACE: RefCell<Option<Probe>> = const { RefCell::new(None) }; }
struct Probe {
    writes: BufWriter<File>,
    frames: BufWriter<File>,
}
pub fn open(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)?;
    let mut writes = BufWriter::new(File::create_new(path.join("writes.csv"))?);
    writeln!(writes, "frame,clock,pc,lr,address,width,value,r0,r1,r2,r3")?;
    let frames = BufWriter::new(File::create_new(path.join("driver.bin"))?);
    TRACE.with(|p| *p.borrow_mut() = Some(Probe { writes, frames }));
    Ok(())
}
fn record(m: *mut c_void, address: u32, width: u32, value: u32) {
    if !(0x02027330..0x02027f00).contains(&address) {
        return;
    }
    let m = unsafe { &*(m as *const Machine) };
    TRACE.with(|p| {
        if let Some(p) = p.borrow_mut().as_mut() {
            let r = &m.cpu.regs;
            writeln!(
                p.writes,
                "{},{},{:08x},{:08x},{address:08x},{width},{value:08x},{:08x},{:08x},{:08x},{:08x}",
                m.bus.frames, m.bus.clock, r[15], r[14], r[0], r[1], r[2], r[3]
            )
            .expect("write sound trace");
        }
    });
}
pub extern "C" fn write8(m: *mut c_void, a: u32, v: u32) {
    record(m, a, 1, v);
    (RT_API.write8)(m, a, v);
}
pub extern "C" fn write16(m: *mut c_void, a: u32, v: u32) {
    record(m, a, 2, v);
    (RT_API.write16)(m, a, v);
}
pub extern "C" fn write32(m: *mut c_void, a: u32, v: u32) {
    record(m, a, 4, v);
    (RT_API.write32)(m, a, v);
}
pub fn frame(m: &Machine) -> std::io::Result<()> {
    TRACE.with(|p| {
        if let Some(p) = p.borrow_mut().as_mut() {
            p.frames.write_all(&m.bus.frames.to_le_bytes())?;
            p.frames.write_all(&m.bus.ewram[0x27330..0x27f00])?;
        }
        Ok(())
    })
}
pub fn close() {
    TRACE.with(|p| {
        p.borrow_mut().take();
    });
}
