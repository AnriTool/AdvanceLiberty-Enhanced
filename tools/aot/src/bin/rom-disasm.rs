//! Local Thumb inspection for locating explicit render-call boundaries.
use armv4t::{MemOffset, Op};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("Usage: rom-disasm ROM START_HEX END_HEX (Thumb)".into());
    }
    let rom = std::fs::read(&args[0])?;
    let start = u32::from_str_radix(args[1].trim_start_matches("0x"), 16)?;
    let end = u32::from_str_radix(args[2].trim_start_matches("0x"), 16)?;
    if start & 1 != 0
        || end <= start
        || start < 0x08000000
        || (end - 0x08000000) as usize > rom.len()
    {
        return Err("Invalid ROM range".into());
    }
    let half = |addr: u32| {
        u16::from_le_bytes(
            rom[(addr - 0x08000000) as usize..(addr - 0x08000000) as usize + 2]
                .try_into()
                .unwrap(),
        )
    };
    let mut addr = start;
    while addr + 2 <= end {
        let instr = armv4t::decode_thumb(half(addr), addr);
        if addr + 4 <= end {
            let next = armv4t::decode_thumb(half(addr + 2), addr + 2);
            if let Some((target, ret)) = armv4t::fuse_thumb_bl(&instr, &next) {
                println!("{addr:08x} BL {target:08x} return={ret:08x}");
                addr += 4;
                continue;
            }
        }
        let mut annotation = String::new();
        if let Op::Mem {
            load: true,
            rn: 15,
            offset: MemOffset::Imm(offset),
            ..
        } = instr.op
        {
            let literal = (instr.pc_value() & !3).wrapping_add(offset as u32);
            if let Some(bytes) = rom
                .get(literal.wrapping_sub(0x08000000) as usize..)
                .and_then(|x| x.get(..4))
            {
                annotation = format!(
                    " literal[{literal:08x}]={:08x}",
                    u32::from_le_bytes(bytes.try_into().unwrap())
                );
            }
        }
        println!(
            "{addr:08x} {:04x} {:?} {:?}{annotation}",
            half(addr),
            instr.cond,
            instr.op
        );
        addr += 2;
    }
    Ok(())
}
