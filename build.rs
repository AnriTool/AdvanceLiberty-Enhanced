use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=resources/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let icon = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("resources/icon.ico")
        .to_string_lossy()
        .replace('\\', "/");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let numbers: Vec<_> = version.split(['.', '-']).take(3).collect();
    let numeric = format!("{},{},{},0", numbers[0], numbers[1], numbers[2]);
    let resource = out.join("application.rc");
    fs::write(
        &resource,
        format!(
            r#"
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "AdvanceLiberty: Enhanced\0"
      VALUE "ProductName", "AdvanceLiberty: Enhanced\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "ProductVersion", "{version}\0"
      VALUE "OriginalFilename", "AdvanceLiberty.exe\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
        ),
    )
    .unwrap();
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let compiled = out.join(if msvc {
        "application.res"
    } else {
        "application.o"
    });
    let mut command = if msvc {
        Command::new(resource_compiler())
    } else {
        Command::new(env::var_os("WINDRES").unwrap_or_else(|| "windres".into()))
    };
    if msvc {
        command
            .arg("/nologo")
            .arg("/fo")
            .arg(&compiled)
            .arg(&resource);
    } else {
        command
            .arg("-i")
            .arg(&resource)
            .arg("-o")
            .arg(&compiled)
            .arg("-O")
            .arg("coff");
    }
    let status = command.status().expect(
        "Windows resource compiler unavailable; install Windows SDK (MSVC) or set WINDRES (GNU)",
    );
    assert!(
        status.success(),
        "Windows application resource compilation failed"
    );
    println!(
        "cargo:rustc-link-arg-bin=AdvanceLiberty={}",
        compiled.display()
    );
}

fn resource_compiler() -> PathBuf {
    println!("cargo:rerun-if-env-changed=RC");
    println!("cargo:rerun-if-env-changed=WINDRES");
    if let Some(rc) = env::var_os("RC") {
        return rc.into();
    }
    // Cargo may run outside a Developer Command Prompt. Find the SDK installed
    // with Visual Studio as well as supporting rc.exe supplied through PATH.
    if let Some(programs) = env::var_os("ProgramFiles(x86)") {
        let bin = PathBuf::from(programs).join("Windows Kits/10/bin");
        if let Ok(entries) = fs::read_dir(bin) {
            let mut versions: Vec<_> = entries.flatten().map(|e| e.path()).collect();
            versions.sort();
            for version in versions.into_iter().rev() {
                let rc = version.join("x64/rc.exe");
                if rc.is_file() {
                    return rc;
                }
            }
        }
    }
    "rc.exe".into()
}
