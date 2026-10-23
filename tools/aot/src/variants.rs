//! Independent one-instruction bodies let an address have multiple RAM versions.
use recomp_core::{
    analyze::{Analysis, Block, View},
    emit::emit_c_chunked,
};
use std::{collections::BTreeMap, fmt::Write};

pub fn generate(audit: &str, rom: &[u8]) -> Result<(String, usize, usize), String> {
    let mut lines = audit.lines();
    if lines.next() != Some("reason,key,steps,first_frame,last_frame,bytes32") {
        return Err("Unexpected audit header".into());
    }
    let mut entries: BTreeMap<u32, BTreeMap<u32, u64>> = BTreeMap::new();
    for line in lines {
        let fields: Vec<_> = line.split(',').collect();
        if fields.len() != 6 {
            return Err("Invalid audit row".into());
        }
        let key = u32::from_str_radix(fields[1], 16).map_err(|e| e.to_string())?;
        if !matches!(key >> 24, 2 | 3 | 8) {
            continue;
        }
        let size = if key & 1 != 0 { 2 } else { 4 };
        if key & !1 & (size - 1) != 0 {
            return Err("Misaligned instruction".into());
        }
        let hex = fields[5];
        if !hex.is_ascii() || hex.len() < size as usize * 2 {
            return Err("Invalid instruction bytes".into());
        }
        let mut word = 0u32;
        for i in 0..size as usize {
            word |= (u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?
                as u32)
                << (8 * i);
        }
        let count: u64 = fields[2].parse::<u64>().map_err(|e| e.to_string())?;
        *entries.entry(key).or_default().entry(word).or_default() += count;
    }
    let mut output = String::new();
    let mut registry = String::new();
    let mut accepted = 0;
    let mut skipped = 0;
    let mut ewram = vec![0u8; 0x40000];
    let mut iwram = vec![0u8; 0x8000];
    for (key, words) in entries {
        let mut words: Vec<_> = words.into_iter().collect();
        words.sort_by_key(|&(word, count)| (std::cmp::Reverse(count), word));
        for (word, _) in words {
            let addr = key & !1;
            let thumb = key & 1 != 0;
            let size = if thumb { 2 } else { 4 };
            if key >> 24 != 8 {
                let ram = if key >> 24 == 2 {
                    &mut ewram
                } else {
                    &mut iwram
                };
                let offset = addr as usize & (ram.len() - 1);
                ram.get_mut(offset..offset + size)
                    .ok_or("RAM boundary")?
                    .copy_from_slice(&word.to_le_bytes()[..size]);
            } else {
                let bytes = rom
                    .get((addr as usize & 0x1ffffff)..)
                    .ok_or("ROM boundary")?;
                if bytes.get(..size) != Some(&word.to_le_bytes()[..size]) {
                    return Err("Audit instruction disagrees with ROM".into());
                }
            }
            let instr = if thumb {
                armv4t::decode_thumb(word as u16, addr)
            } else {
                armv4t::decode_arm(word, addr)
            };
            let view = View {
                rom,
                ewram: Some(&ewram),
                iwram: Some(&iwram),
                bios: None,
            };
            let mut generated = String::new();
            emit_c_chunked(
                &Analysis {
                    blocks: vec![Block {
                        addr,
                        thumb,
                        instrs: vec![instr],
                    }],
                },
                &view,
                usize::MAX,
                |c, _| {
                    generated.push_str(c);
                    Ok(())
                },
            )?;
            // One interpreter call is the entry-guard rejection. Any additional
            // call means this instruction has no native implementation: do not
            // shadow a potentially better pre-existing translated block.
            if let Some(body) = crate::native_ops::body(&instr) {
                let shell = format!("    c[15] = 0x{addr:08x}u;\n    return a->interp_one(m);\n");
                if !generated.contains(&shell) {
                    return Err("Unsupported-instruction shell changed".into());
                }
                generated = generated.replace(&shell, &body);
            }
            let expected_guards = if key >> 24 == 8 { 0 } else { 1 };
            if generated.matches("a->interp_one(m)").count() != expected_guards {
                skipped += 1;
                continue;
            }
            let symbol = format!("b_{addr:08x}_{}", if thumb { "t" } else { "a" });
            let start = generated
                .find(&format!("uint32_t {symbol}("))
                .ok_or("Emitter function boundary changed")?;
            let end = generated
                .find("RCG_EXPORT const RcgBlock rcg_blocks[]")
                .ok_or("Emitter table boundary changed")?;
            if output.is_empty() {
                output.push_str(&generated[..start]);
                output.push_str("typedef struct { RtApi base; uint32_t (*read_psr)(void*, uint32_t); void (*write_psr)(void*, uint32_t, uint32_t, uint32_t); uint32_t (*swi)(void*, uint32_t, uint32_t, uint32_t); } AdvanceRtApi;\n");
            }
            let unique = format!("v_{accepted}_{addr:08x}");
            output.push_str(&generated[start..end].replace(&symbol, &unique));
            writeln!(registry, "    {{ 0x{key:08x}u, {unique} }},").unwrap();
            accepted += 1;
        }
    }
    if accepted == 0 {
        return Err("No natively supported RAM instructions".into());
    }
    writeln!(output, "RCG_EXPORT const RcgBlock rcg_blocks[] = {{\n{registry}}};\nRCG_EXPORT const uint64_t rcg_block_count = {accepted};").unwrap();
    output.push_str("RCG_EXPORT const uint32_t advance_native_rescue_v1 = 1u;\n");
    output.push_str("RCG_EXPORT const uint32_t advance_native_api_version = 1u;\n");
    Ok((output, accepted, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retains_two_guarded_versions_and_deduplicates_same_instruction() {
        let input = "reason,key,steps,first_frame,last_frame,bytes32\ndispatch,03000000,1,0,0,0100a0e3\nguard,03000000,5,0,0,0200a0e3\ninternal,03000000,2,0,0,0100a0e300000000\n";
        let (source, count, skipped) = generate(input, &[]).unwrap();
        assert_eq!((count, skipped), (2, 0));
        assert_eq!(
            source.matches("if (!a->guard(m, 0x03000000u, 4u,").count(),
            2
        );
        assert!(source.contains("v_0_03000000"));
        assert!(source.contains("v_1_03000000"));
        assert_eq!(
            source
                .matches("RCG_EXPORT const RcgBlock rcg_blocks[]")
                .count(),
            1
        );
        assert!(
            source.find("uint32_t r = 0x00000002u").unwrap()
                < source.find("uint32_t r = 0x00000001u").unwrap()
        );
    }
}
