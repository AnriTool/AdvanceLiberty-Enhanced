//! Additional C emission for observed operations missing in upstream's emitter.
use armv4t::{AluOp, Cond, Instr, Op, Operand2, Shift, ShiftKind};

pub fn body(instr: &Instr) -> Option<String> {
    if instr.cond != Cond::Al {
        return None;
    }
    let reg = |r: u8, extra: u32| {
        if r == 15 {
            format!("0x{:08x}u", instr.pc_value().wrapping_add(extra))
        } else {
            format!("c[{r}]")
        }
    };
    let cycles = match instr.addr >> 24 {
        2 => {
            if instr.thumb {
                3
            } else {
                6
            }
        }
        3 => 1,
        8 => {
            if instr.thumb {
                2
            } else {
                4
            }
        }
        _ => return None,
    };
    match instr.op {
        Op::ThumbBlHigh { lr_partial } => {
            let next = instr.addr.wrapping_add(instr.size());
            Some(format!("    c[14] = 0x{lr_partial:08x}u;\n    a->tick(m, {cycles}u);\n    c[15] = 0x{next:08x}u;\n    return c[15] | 1u;\n"))
        }
        Op::ThumbBlLow { off } => {
            let offset = (off as u32).wrapping_shl(1);
            let ret = instr.addr.wrapping_add(2) | 1;
            Some(format!("    uint32_t target = c[14] + 0x{offset:08x}u;\n    c[14] = 0x{ret:08x}u;\n    a->tick(m, {cycles}u);\n    c[15] = target & ~1u;\n    return c[15] | 1u;\n"))
        }
        Op::Mrs { spsr, rd } if rd != 15 => {
            let next = instr.addr.wrapping_add(instr.size());
            Some(format!("    c[{rd}] = ((const AdvanceRtApi*)a)->read_psr(m, {}u);\n    a->tick(m, {cycles}u);\n    c[15] = 0x{next:08x}u;\n    return c[15] | ((c[16] >> 5) & 1u);\n", spsr as u32))
        }
        Op::MsrReg { spsr, fields, rm } => {
            let value = reg(rm, 0);
            let next = instr.addr.wrapping_add(instr.size());
            Some(format!("    ((const AdvanceRtApi*)a)->write_psr(m, {}u, {fields}u, {value});\n    a->tick(m, {cycles}u);\n    c[15] = 0x{next:08x}u;\n    return c[15] | ((c[16] >> 5) & 1u);\n", spsr as u32))
        }
        Op::Swi { imm } => {
            let num = if instr.thumb { imm } else { imm >> 16 };
            let next = instr.addr.wrapping_add(instr.size());
            Some(format!(
                "    return ((const AdvanceRtApi*)a)->swi(m, {num}u, 0x{next:08x}u, {cycles}u);\n"
            ))
        }
        Op::Alu {
            op,
            s: false,
            rd: 15,
            rn,
            op2,
        } if matches!(op, AluOp::Add | AluOp::Mov) => {
            let value = match op2 {
                Operand2::Imm { value, .. } => format!("0x{value:08x}u"),
                Operand2::Reg {
                    rm,
                    shift:
                        Shift::Imm {
                            kind: ShiftKind::Lsl,
                            amount,
                        },
                } if amount < 32 => format!("({} << {amount}u)", reg(rm, 0)),
                _ => return None,
            };
            let result = if op == AluOp::Mov {
                value
            } else {
                format!("({} + {value})", reg(rn, 0))
            };
            let mask = if instr.thumb { "~1u" } else { "~3u" };
            let thumb = instr.thumb as u32;
            Some(format!("    uint32_t target = {result};\n    a->tick(m, {cycles}u);\n    c[15] = target & {mask};\n    return c[15] | {thumb}u;\n"))
        }
        Op::Alu {
            op: AluOp::Mov,
            s: true,
            rd,
            op2:
                Operand2::Reg {
                    rm,
                    shift: Shift::Reg { kind, rs },
                },
            ..
        } if rd != 15 && rs != 15 => {
            let shift = match kind {
                ShiftKind::Lsl => "if (n < 32u) { carry = (x >> (32u-n)) & 1u; value = x << n; } else { carry = n == 32u ? x & 1u : 0u; value = 0u; }",
                ShiftKind::Lsr => "if (n < 32u) { carry = (x >> (n-1u)) & 1u; value = x >> n; } else { carry = n == 32u ? x >> 31 : 0u; value = 0u; }",
                ShiftKind::Asr => "if (n < 32u) { carry = (x >> (n-1u)) & 1u; value = (uint32_t)((int32_t)x >> n); } else { carry = x >> 31; value = carry ? 0xffffffffu : 0u; }",
                ShiftKind::Ror => "uint32_t k = n & 31u; value = ror32(x, k); carry = value >> 31;",
            };
            let x = reg(rm, 4);
            let n = reg(rs, 4);
            let next = instr.addr.wrapping_add(instr.size());
            let thumb = instr.thumb as u32;
            Some(format!("    uint32_t x = {x}, n = {n} & 255u;\n    uint32_t value = x, carry = (c[16] >> 29) & 1u;\n    if (n != 0u) {{ {shift} }}\n    c[{rd}] = value; f_nz(c, value); f_c(c, carry);\n    a->tick(m, {cycles}u);\n    c[15] = 0x{next:08x}u;\n    return c[15] | {thumb}u;\n"))
        }
        _ => None,
    }
}
