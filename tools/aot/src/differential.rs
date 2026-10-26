//! Compile synthetic code and compare generated machine code against the oracle.
use gba_core::{
    capi::{RcgBlock, RT_API},
    Machine,
};
use std::{ffi::c_void, fmt::Write};

#[test]
#[ignore = "requires a host C compiler; set ADVANCE_AOT_TEST_GCC"]
fn compiled_pc_and_shift_operations_match_oracle() {
    let gcc = std::env::var("ADVANCE_AOT_TEST_GCC").expect("set ADVANCE_AOT_TEST_GCC");
    let root = std::env::temp_dir().join(format!("advance-aot-diff-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let mut audit = String::from("reason,key,steps,first_frame,last_frame,bytes32\n");
    let instructions = [
        (0x03000000u32, 0xe08ff100u32), // ADD pc, pc, r0, LSL #2
        (0x03000004, 0xe1a0f00e),       // MOV pc, lr
        (0x03000009, 0x4687),           // Thumb MOV pc, r0
        (0x0300000d, 0x4082),           // Thumb LSL r2, r0
        (0x03000011, 0x40c2),           // Thumb LSR r2, r0
        (0x03000015, 0x4102),           // Thumb ASR r2, r0
        (0x03000019, 0x41c2),           // Thumb ROR r2, r0
        (0x0300001d, 0xf7ff),           // Thumb BL high, negative displacement
        (0x03000021, 0xffff),           // Thumb BL low, maximal offset and LR wraparound
    ];
    for (key, word) in instructions {
        write!(audit, "internal,{key:08x},1,0,0,").unwrap();
        for byte in word.to_le_bytes() {
            write!(audit, "{byte:02x}").unwrap();
        }
        audit.push('\n');
    }
    let (source, count, skipped) = crate::variants::generate(&audit, &[]).unwrap();
    assert_eq!((count, skipped), (instructions.len(), 0));
    let c_path = root.join("synthetic.c");
    let dll_path = root.join(format!("synthetic.{}", std::env::consts::DLL_EXTENSION));
    std::fs::write(&c_path, source).unwrap();
    super::compile(&c_path, &dll_path, &gcc).unwrap();
    let library = unsafe { libloading::Library::new(&dll_path) }.unwrap();
    let table = unsafe {
        let ptr: libloading::Symbol<*const RcgBlock> = library.get(b"rcg_blocks").unwrap();
        std::slice::from_raw_parts(*ptr, count)
    };
    let mut comparisons = 0;
    for (key, word) in instructions {
        let block = table.iter().find(|b| b.key == key).unwrap();
        for value in [0u32, 1, 0x7fffffff, 0x80000000, 0xffffffff, 0xabcdef01] {
            for amount in [0u32, 1, 31, 32, 33, 63, 64, 127, 255, 256, 257] {
                for carry in [0u32, 0x20000000] {
                    let mut expected = Machine::new(vec![0; 4]);
                    expected.cpu.cpsr = 0x9000001f | carry | ((key & 1) << 5);
                    expected.cpu.regs[15] = key & !1;
                    expected.cpu.regs[0] = amount;
                    expected.cpu.regs[2] = value;
                    expected.cpu.regs[14] = value;
                    let offset = (key & 0x7ffe) as usize;
                    expected.bus.iwram[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
                    let mut actual = expected.try_fork_for_research().unwrap();
                    let result = (block.func)(&RT_API, &mut actual as *mut _ as *mut c_void);
                    let oracle = (RT_API.interp_one)(&mut expected as *mut _ as *mut c_void);
                    assert_eq!(
                        result, oracle,
                        "key={key:08x} value={value:08x} amount={amount}"
                    );
                    assert_eq!(actual.cpu.regs, expected.cpu.regs);
                    assert_eq!(actual.cpu.cpsr, expected.cpu.cpsr);
                    assert_eq!(actual.bus.clock, expected.bus.clock);
                    comparisons += 1;
                }
            }
        }
    }
    println!("{comparisons} compiled-code comparisons passed");
    drop(library);
    // Keep synthetic sources/results for inspection; no ROM bytes are involved.
}
