//! End-to-end menu regressions against the local USA ROM and AOT libraries.
use super::*;
use gba_core::{
    capi::{RcgBlock, RtApi, RT_API},
    Mode,
};
use std::{cell::Cell, collections::HashMap, ffi::c_void};
fn menu_session() -> std::path::PathBuf {
    std::env::var_os("ADVANCE_LIBERTY_MENU_SESSION")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "test-data/menu-session".into())
}
thread_local! {static REJECTED:Cell<bool>=const{Cell::new(false)};}
extern "C" fn defer(m: *mut c_void) -> u32 {
    REJECTED.with(|v| v.set(true));
    let m = unsafe { &*(m as *const Machine) };
    m.cpu.regs[15] | m.cpu.thumb() as u32
}
struct Engine {
    _libraries: Vec<libloading::Library>,
    table: HashMap<u32, Vec<crate::BlockFn>>,
    sounds: usize,
    audible_sounds: usize,
}
impl Engine {
    fn load() -> Self {
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read("native/usa/manifest.json").unwrap()).unwrap();
        let mut engine = Self {
            _libraries: Vec::new(),
            table: HashMap::new(),
            sounds: 0,
            audible_sounds: 0,
        };
        for library in manifest["libraries"].as_array().unwrap() {
            let library =
                unsafe { libloading::Library::new(library["path"].as_str().unwrap()).unwrap() };
            unsafe {
                let ptr = library.get::<*const RcgBlock>(b"rcg_blocks").unwrap();
                let count = library.get::<*const u64>(b"rcg_block_count").unwrap();
                for block in std::slice::from_raw_parts(*ptr, **count as usize) {
                    engine.table.entry(block.key).or_default().push(block.func);
                }
            }
            engine._libraries.push(library);
        }
        engine
    }
    fn frame(&mut self, m: &mut Machine, f: &mut Frontend, buttons: u16) {
        m.bus.keys = buttons;
        m.bus.audio_buf.clear();
        m.bus.frame_ready = false;
        let api = crate::native_host::Api::new(RtApi {
            interp_one: defer,
            ..RT_API
        });
        let mut steps = 0;
        while !m.bus.frame_ready {
            steps += 1;
            assert!(steps < 5_000_000, "native menu frame stalled");
            if m.bus.halted
                || (m.cpu.regs[15] == gba_core::machine::IRQ_RETURN_ADDR
                    && m.cpu.mode() == Mode::Irq)
                || (m.bus.irq_pending() && !m.cpu.flag(gba_core::cpu::FLAG_I))
            {
                m.step();
                continue;
            }
            let key = m.cpu.regs[15] | m.cpu.thumb() as u32;
            if Frontend::interested(key) {
                f.observe(m, key);
            }
            if key == 0x08005c15 && m.cpu.regs[0] == 0x105 {
                self.sounds += 1;
                if !f.mute_navigation_audio() {
                    self.audible_sounds += 1;
                }
            }
            let mut accepted = false;
            if let Some(entries) = self.table.get(&key) {
                for entry in entries {
                    REJECTED.with(|v| v.set(false));
                    entry(&api.base, m as *mut Machine as *mut c_void);
                    if !REJECTED.with(|v| v.get()) {
                        accepted = true;
                        break;
                    }
                }
            }
            if !accepted {
                m.step();
            }
        }
    }
    fn until(
        &mut self,
        m: &mut Machine,
        f: &mut Frontend,
        label: &str,
        predicate: impl Fn(&Machine, &Frontend) -> bool,
    ) {
        for _ in 0..360 {
            let buttons = f.input();
            self.frame(m, f, buttons);
            if predicate(m, f) && !f.busy() {
                return;
            }
        }
        panic!(
            "{label}: frame={}, view={:?}, cursor={}, busy={}",
            m.bus.frames,
            f.view,
            f.row,
            f.busy()
        );
    }
    fn choose(&mut self, m: &mut Machine, f: &mut Frontend, row: usize, next: u32) {
        f.row = row;
        f.event(&Event::KeyDown {
            timestamp: 0,
            window_id: 0,
            keycode: Some(Keycode::Return),
            scancode: None,
            keymod: sdl2::keyboard::Mod::NOMOD,
            repeat: false,
        });
        self.until(m, f, "choose", |_, f| {
            f.view.as_ref().is_some_and(|v| v.descriptor == next)
        });
    }
}
#[test]
#[ignore = "requires local USA ROM, AOT libraries and recorded boot"]
fn real_native_camera_roundtrip_resume_and_quit() {
    let mut engine = Engine::load();
    let mut m = Machine::new(std::fs::read("roms/usa.gba").unwrap());
    let source = menu_session();
    m.bus
        .load_save_data(&std::fs::read(source.join("initial.sav")).unwrap());
    let input = std::fs::read_to_string(source.join("input.gba-input")).unwrap();
    let keys: Vec<(u64, u16)> = input
        .lines()
        .filter_map(|line| {
            let (a, b) = line.split_once(' ')?;
            Some((a.parse().ok()?, u16::from_str_radix(b, 16).ok()?))
        })
        .collect();
    let mut f = Frontend::default();
    for frame in 0..850 {
        let index = keys.partition_point(|(at, _)| *at <= frame);
        engine.frame(
            &mut m,
            &mut f,
            if index == 0 { 0x3ff } else { keys[index - 1].1 },
        );
    }
    assert_eq!(f.view.as_ref().unwrap().descriptor, ROOT);
    engine.choose(&mut m, &mut f, 4, 0x0832bea0);
    engine.choose(&mut m, &mut f, 3, ROOT);
    assert_eq!(m.bus.ewram[0x10c66], 3);
    engine.choose(&mut m, &mut f, 4, 0x0832bea0);
    let before = engine.sounds;
    let before_audible = engine.audible_sounds;
    engine.choose(&mut m, &mut f, 0, ROOT);
    assert_eq!(m.bus.ewram[0x10c66], 0);
    assert!(
        engine.sounds - before <= 5,
        "unexpected repeated selection sounds"
    );
    assert!(
        engine.audible_sounds - before_audible <= 2,
        "hidden alignment sounds must be silent"
    );
    f.row = 0;
    f.event(&Event::KeyDown {
        timestamp: 0,
        window_id: 0,
        keycode: Some(Keycode::Return),
        scancode: None,
        keymod: sdl2::keyboard::Mod::NOMOD,
        repeat: false,
    });
    engine.until(&mut m, &mut f, "resume", |m, f| {
        f.view.is_none()
            && crate::control_context::detect(&m.bus) != crate::control_context::Context::Menu
    });
    // Finish the game's normal resume fade before requesting another pause.
    for _ in 0..45 {
        engine.frame(&mut m, &mut f, 0x3ff);
    }
    f.press(8);
    engine.until(&mut m, &mut f, "reopen", |_, f| {
        f.view.as_ref().is_some_and(|v| v.descriptor == ROOT)
    });
    engine.choose(&mut m, &mut f, 5, 0x0832db28);
    engine.choose(&mut m, &mut f, 0, ROOT);
    f.event(&Event::KeyDown {
        timestamp: 0,
        window_id: 0,
        keycode: Some(Keycode::Escape),
        scancode: None,
        keymod: sdl2::keyboard::Mod::NOMOD,
        repeat: false,
    });
    engine.until(&mut m, &mut f, "escape resumes", |m, f| {
        f.view.is_none()
            && crate::control_context::detect(&m.bus) != crate::control_context::Context::Menu
    });
    for _ in 0..45 {
        engine.frame(&mut m, &mut f, 0x3ff);
    }
    f.press(8);
    engine.until(&mut m, &mut f, "reopen after escape", |_, f| {
        f.view.as_ref().is_some_and(|v| v.descriptor == ROOT)
    });
    engine.choose(&mut m, &mut f, 5, 0x0832db28);
    f.row = 2;
    f.event(&Event::KeyDown {
        timestamp: 0,
        window_id: 0,
        keycode: Some(Keycode::Return),
        scancode: None,
        keymod: sdl2::keyboard::Mod::NOMOD,
        repeat: false,
    });
    engine.until(&mut m, &mut f, "quit to startup", |m, f| {
        f.view.is_none()
            && crate::control_context::detect(&m.bus) == crate::control_context::Context::Menu
    });
}

#[test]
#[ignore = "requires local USA ROM, AOT libraries and recorded boot"]
fn real_native_save_and_exit_writes_and_reloads_progress() {
    let mut engine = Engine::load();
    let rom = std::fs::read("roms/usa.gba").unwrap();
    let source = menu_session();
    let initial = std::fs::read(source.join("initial.sav")).unwrap();
    let input = std::fs::read_to_string(source.join("input.gba-input")).unwrap();
    let keys: Vec<(u64, u16)> = input
        .lines()
        .filter_map(|line| {
            let (a, b) = line.split_once(' ')?;
            Some((a.parse().ok()?, u16::from_str_radix(b, 16).ok()?))
        })
        .collect();
    let mut m = Machine::new(rom.clone());
    let directory = std::env::temp_dir().join(format!(
        "menu-save-exit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("progress.sav");
    std::fs::write(&path, &initial).unwrap();
    let mut save = crate::save::Save::open(path.clone(), &mut m).unwrap();
    let mut f = Frontend::default();
    for frame in 0..850 {
        let i = keys.partition_point(|(at, _)| *at <= frame);
        engine.frame(&mut m, &mut f, if i == 0 { 0x3ff } else { keys[i - 1].1 });
    }
    engine.choose(&mut m, &mut f, 4, 0x0832bea0);
    engine.choose(&mut m, &mut f, 3, ROOT);
    assert_eq!(m.bus.ewram[0x10c66], 3);
    // Also start from Options, so the automatic route must return to Pause.
    engine.choose(&mut m, &mut f, 1, OPTIONS);
    assert!(f.begin_save_exit());
    engine.until(&mut m, &mut f, "save completed in ROM", |_, f| {
        f.save_exit_needs_commit()
    });
    assert!(
        !f.save_exit_complete(),
        "ROM success alone must not close the app"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        initial,
        "no disk commit before game success"
    );
    save.update(&m, true).unwrap();
    f.finish_save_exit(Ok(()));
    assert!(f.save_exit_complete());
    let written = std::fs::read(&path).unwrap();
    assert_eq!(written, m.bus.save_data().unwrap());
    assert_ne!(written, initial);
    assert_eq!(
        std::fs::read(path.with_file_name("progress.sav.bak")).unwrap(),
        initial
    );
    drop(save);
    let mut loaded = Machine::new(rom);
    let _save = crate::save::Save::open(path, &mut loaded).unwrap();
    let mut next = Frontend::default();
    for frame in 0..850 {
        let i = keys.partition_point(|(at, _)| *at <= frame);
        engine.frame(
            &mut loaded,
            &mut next,
            if i == 0 { 0x3ff } else { keys[i - 1].1 },
        );
    }
    assert_eq!(next.view.as_ref().unwrap().descriptor, ROOT);
    assert_eq!(
        loaded.bus.ewram[0x10c66], 3,
        "saved camera setting must survive a full boot/load"
    );
}
