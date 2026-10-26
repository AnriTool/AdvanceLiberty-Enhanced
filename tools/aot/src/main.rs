//! Build a guarded EWRAM AOT supplement from observed instruction windows.
//! No guest code is executed or dynamically compiled by the game runtime.
use recomp_core::{
    analyze::{Analysis, Block, View},
    emit::emit_c_chunked,
};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};
#[cfg(test)]
mod differential;
mod native_ops;
mod variants;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    let ram_variants = if let Some(i) = args.iter().position(|a| a == "--ram-variants") {
        args.remove(i);
        true
    } else {
        false
    };
    if args.len() != 4 {
        return Err(
            "Usage: advance-aot-supplement AUDIT.csv ROM OUTPUT.dll GCC [--ram-variants]".into(),
        );
    }
    let output = PathBuf::from(&args[2]);
    let source = output.with_extension("c");
    if output.exists() || source.exists() {
        return Err("Output DLL/C already exists; choose a new name".into());
    }
    let rom = fs::read(&args[1])?;
    if rom.len() != 0x1000000 || &rom[0xac..0xb0] != b"BGTE" {
        return Err("Expected the supported unmodified USA ROM".into());
    }
    let audit = fs::read_to_string(&args[0])?;
    if ram_variants {
        let (code, count, skipped) = variants::generate(&audit, &rom)?;
        fs::write(&source, code)?;
        compile(&source, &output, &args[3])?;
        println!("RAM AOT supplement: {count} guarded instruction variants; {skipped} unsupported skipped -> {}", output.display());
        return Ok(());
    }
    let mut lines = audit.lines();
    if lines.next() != Some("reason,key,steps,first_frame,last_frame,bytes32") {
        return Err("Unexpected audit header".into());
    }
    let mut entries = BTreeMap::new();
    for line in lines {
        let fields: Vec<_> = line.split(',').collect();
        if fields.len() != 6 {
            return Err("Invalid audit row".into());
        }
        let key = u32::from_str_radix(fields[1], 16)?;
        if key >> 24 != 2 {
            continue;
        }
        let bytes = fields[5];
        if bytes.len() < 8 {
            return Err("Missing instruction bytes".into());
        }
        let word = u32::from_le_bytes([
            u8::from_str_radix(&bytes[0..2], 16)?,
            u8::from_str_radix(&bytes[2..4], 16)?,
            u8::from_str_radix(&bytes[4..6], 16)?,
            u8::from_str_radix(&bytes[6..8], 16)?,
        ]);
        if entries.insert(key, word).is_some_and(|old| old != word) {
            return Err(format!(
                "Multiple code variants at {key:08x}; separate libraries required"
            )
            .into());
        }
    }
    if entries.is_empty() {
        return Err("No EWRAM instructions found".into());
    }
    let mut ram = vec![0u8; 0x40000];
    let mut blocks = Vec::new();
    for (&key, &word) in &entries {
        let addr = key & !1;
        let thumb = key & 1 != 0;
        let offset = (addr & 0x3ffff) as usize;
        let size = if thumb { 2 } else { 4 };
        if offset + size > ram.len() {
            return Err("Instruction crosses RAM boundary".into());
        }
        ram[offset..offset + size].copy_from_slice(&word.to_le_bytes()[..size]);
        let instr = if thumb {
            armv4t::decode_thumb(word as u16, addr)
        } else {
            armv4t::decode_arm(word, addr)
        };
        blocks.push(Block {
            addr,
            thumb,
            instrs: vec![instr],
        });
    }
    let view = View {
        rom: &rom,
        ewram: Some(&ram),
        iwram: None,
        bios: None,
    };
    let mut c_source = String::new();
    emit_c_chunked(&Analysis { blocks }, &view, usize::MAX, |chunk, _| {
        c_source.push_str(chunk);
        Ok(())
    })?;
    fs::write(&source, c_source)?;
    compile(&source, &output, &args[3])?;
    println!(
        "AOT supplement: {} guarded EWRAM entries -> {}",
        entries.len(),
        output.display()
    );
    Ok(())
}

fn compile(
    source: &std::path::Path,
    output: &std::path::Path,
    gcc: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // MinGW's compiler subprocesses need its DLL directory on PATH as well.
    // GCC derives cc1's location from argv[0]; Windows verbatim (\\?\) paths
    // returned by canonicalize break that lookup in this MinGW distribution.
    let compiler = std::path::absolute(gcc)?;
    let mut paths = vec![compiler
        .parent()
        .ok_or("Compiler has no parent")?
        .to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let status = Command::new(&compiler)
        .env("PATH", std::env::join_paths(paths)?)
        .args(["-shared", "-O2", "-std=c11"])
        .arg(source)
        .arg("-o")
        .arg(output)
        .status()?;
    if !status.success() {
        return Err("C compilation failed".into());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
