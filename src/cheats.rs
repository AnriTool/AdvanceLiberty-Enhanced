//! PC cheat panel. Only emits ordinary, recordable GBA button presses.
use crate::control_context::Context;
use sdl2::{
    event::Event,
    keyboard::{Keycode, Mod},
    pixels::Color,
    rect::Rect,
    render::{Canvas, RenderTarget},
};
use std::collections::VecDeque;

// Main-menu input loop; unlike the code checker it still runs after unlocking.
pub const MAIN_CHECK: u32 = 0x080023fd;
const RELEASE: u16 = 0x3ff;
const NAMES: [&str; 9] = [
    "All weapons",
    "Full health",
    "Full armor",
    "$15,000 cash",
    "Lower wanted level",
    "Raise wanted level",
    "Wanted max / zero",
    "Gang hostility",
    "Level select (main menu)",
];
// Hardware bits: A=1, B=2, R=256, L=512. USA 08caa264..08caa2db.
const ENDS: [[u16; 2]; 9] = [
    [1, 1],
    [2, 2],
    [1, 512],
    [512, 512],
    [1, 256],
    [256, 1],
    [256, 256],
    [2, 256],
    [512, 256],
];

#[derive(Default)]
pub struct Panel {
    pub visible: bool,
    row: usize,
    queue: VecDeque<u16>,
    enabled: bool,
    gameplay: bool,
    main_frame: Option<u64>,
    main_menu: bool,
    message: &'static str,
    menu_sequence: bool,
}
impl Panel {
    pub fn sync(&mut self, bus: &gba_core::MemMap) {
        self.enabled = bus.rom.get(0xac..0xb0) == Some(b"BGTE")
            && bus.ewram.get(0x11020..0x11024) == Some(&[1, 0, 0, 0][..]);
        self.gameplay = crate::control_context::detect(bus) != Context::Menu;
        self.main_menu = self
            .main_frame
            .is_some_and(|f| bus.frames.saturating_sub(f) <= 2)
            && !self.gameplay;
        if self.main_menu && !self.menu_sequence && !self.queue.is_empty() {
            self.queue.clear();
            self.message = "Sequence stopped: left gameplay.";
        }
    }
    pub fn observe(&mut self, frame: u64) {
        self.main_frame = Some(frame);
    }
    fn pulse(&mut self, pressed: u16) {
        self.queue
            .extend(std::iter::repeat_n(RELEASE & !pressed, 6));
        self.queue.extend(std::iter::repeat_n(RELEASE, 6));
    }
    fn activate(&mut self) {
        self.menu_sequence = false;
        self.queue.extend(std::iter::repeat_n(RELEASE, 6));
        self.pulse(1 | 2 | 8);
        self.message = "Switching native cheat mode...";
    }
    fn select(&mut self) {
        if !self.queue.is_empty() {
            return;
        }
        if self.row == 8 {
            if !self.main_menu {
                self.message = "Level select needs the main menu.";
                return;
            }
        } else {
            if !self.gameplay {
                self.message = "Start or resume gameplay first.";
                return;
            }
            if !self.enabled {
                self.activate();
                return;
            }
        }
        self.queue.extend(std::iter::repeat_n(RELEASE, 6));
        for mask in [32, 16, 64, 128, ENDS[self.row][0], ENDS[self.row][1]] {
            self.pulse(mask);
        }
        self.menu_sequence = self.row == 8;
        if self.row == 8 {
            // Keep Start held before the A edge required by the original menu.
            self.queue.extend(std::iter::repeat_n(RELEASE & !8, 6));
            self.pulse(8 | 1);
        }
        self.message = "Sending native button sequence...";
        eprintln!("Cheat sequence: {}", NAMES[self.row]);
    }
    /// Consumed keys never reach movement, firing or menu navigation.
    pub fn event(&mut self, event: &Event) -> bool {
        let key = match *event {
            Event::KeyDown {
                keycode: Some(k),
                keymod,
                repeat: false,
                ..
            } => {
                if k == Keycode::F12 && keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) {
                    self.visible = !self.visible;
                    if self.visible && self.gameplay && !self.enabled && self.queue.is_empty() {
                        self.activate();
                    }
                    return true;
                }
                Some(k)
            }
            Event::ControllerButtonDown { button, .. } if self.visible => match button {
                sdl2::controller::Button::DPadUp => Some(Keycode::Up),
                sdl2::controller::Button::DPadDown => Some(Keycode::Down),
                sdl2::controller::Button::A => Some(Keycode::Return),
                sdl2::controller::Button::B => Some(Keycode::Escape),
                _ => None,
            },
            _ => None,
        };
        if !self.visible {
            return false;
        }
        match key {
            Some(Keycode::Escape) => self.visible = false,
            Some(Keycode::Num0 | Keycode::Kp0) => {
                if self.gameplay && self.queue.is_empty() {
                    self.activate();
                }
            }
            Some(Keycode::Up) => self.row = (self.row + 8) % 9,
            Some(Keycode::Down) => self.row = (self.row + 1) % 9,
            Some(Keycode::Return | Keycode::KpEnter) => self.select(),
            Some(k) => {
                let digit = match k {
                    Keycode::Num1 | Keycode::Kp1 => Some(0),
                    Keycode::Num2 | Keycode::Kp2 => Some(1),
                    Keycode::Num3 | Keycode::Kp3 => Some(2),
                    Keycode::Num4 | Keycode::Kp4 => Some(3),
                    Keycode::Num5 | Keycode::Kp5 => Some(4),
                    Keycode::Num6 | Keycode::Kp6 => Some(5),
                    Keycode::Num7 | Keycode::Kp7 => Some(6),
                    Keycode::Num8 | Keycode::Kp8 => Some(7),
                    Keycode::Num9 | Keycode::Kp9 => Some(8),
                    _ => None,
                };
                if let Some(row) = digit {
                    self.row = row;
                    self.select();
                }
            }
            _ => {}
        }
        matches!(
            event,
            Event::KeyDown { .. }
                | Event::KeyUp { .. }
                | Event::MouseButtonDown { .. }
                | Event::MouseButtonUp { .. }
                | Event::MouseWheel { .. }
                | Event::ControllerButtonDown { .. }
                | Event::ControllerButtonUp { .. }
        )
    }
    pub fn captures_input(&self) -> bool {
        self.visible || !self.queue.is_empty()
    }
    pub fn input(&mut self) -> u16 {
        if let Some(keys) = self.queue.pop_front() {
            if self.queue.is_empty() {
                self.message = "Sequence sent to the game.";
                if self.menu_sequence {
                    self.visible = false;
                }
            }
            keys
        } else {
            RELEASE
        }
    }
    pub fn draw<T: RenderTarget>(
        &self,
        canvas: &mut Canvas<T>,
        skin: Option<&crate::game_menu::Skin>,
    ) -> Result<(), String> {
        if !self.visible {
            return Ok(());
        }
        let old = canvas.logical_size();
        let width = if old.1 == 160 { 720 } else { old.0 };
        canvas
            .set_logical_size(width, 480)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            let x = width as i32 - 408;
            canvas.set_draw_color(Color::RGB(12, 18, 27));
            canvas.fill_rect(Rect::new(x, 8, 400, 350))?;
            let text = |canvas: &mut Canvas<T>, y, value: &str, color| {
                if let Some(skin) = skin {
                    skin.small_text(canvas, x + 10, y, value, color, if y == 18 { 2 } else { 1 })
                } else {
                    crate::options::text_with_font(
                        canvas,
                        x + 10,
                        y,
                        value,
                        color,
                        if y == 18 { 2 } else { 1 },
                        None,
                    )
                }
            };
            text(
                canvas,
                18,
                if self.enabled {
                    "CHEATS - NATIVE MODE ON"
                } else {
                    "CHEATS - NATIVE MODE OFF"
                },
                Color::RGB(255, 220, 70),
            )?;
            text(
                canvas,
                44,
                "1-9: apply | Up/Down + Enter",
                Color::RGB(180, 195, 210),
            )?;
            text(
                canvas,
                60,
                "Ctrl+F12 / Esc: close",
                Color::RGB(180, 195, 210),
            )?;
            for (i, name) in NAMES.iter().enumerate() {
                let color = if i == self.row {
                    Color::RGB(255, 220, 70)
                } else {
                    Color::RGB(235, 235, 235)
                };
                let y = 86 + i as i32 * 23;
                crate::options::text_with_font(
                    canvas,
                    x + 10,
                    y,
                    &format!("{} {}", if i == self.row { ">" } else { " " }, i + 1),
                    color,
                    2,
                    None,
                )?;
                if let Some(skin) = skin {
                    skin.small_text(canvas, x + 52, y, name, color, 2)?;
                } else {
                    crate::options::text_with_font(canvas, x + 52, y, name, color, 2, None)?;
                }
            }
            text(
                canvas,
                300,
                "0: toggle native cheat mode",
                Color::RGB(180, 195, 210),
            )?;
            text(
                canvas,
                316,
                "The game continues while this is open.",
                Color::RGB(180, 195, 210),
            )?;
            text(canvas, 340, self.message, Color::RGB(180, 220, 190))
        })();
        canvas
            .set_logical_size(old.0, old.1)
            .map_err(|e| e.to_string())?;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(k: Keycode) -> Event {
        Event::KeyDown {
            timestamp: 0,
            window_id: 0,
            keycode: Some(k),
            scancode: None,
            keymod: Mod::NOMOD,
            repeat: false,
        }
    }
    #[test]
    fn sequences_are_isolated_edges_and_never_write_guest_memory() {
        let mut p = Panel {
            visible: true,
            enabled: true,
            gameplay: true,
            ..Default::default()
        };
        assert!(p.event(&key(Keycode::Num1)));
        let mut edges = Vec::new();
        let mut prev = RELEASE;
        while !p.queue.is_empty() {
            let v = p.input();
            if v != RELEASE && prev == RELEASE {
                edges.push(RELEASE ^ v);
            }
            prev = v;
        }
        assert_eq!(edges, [32, 16, 64, 128, 1, 1]);
        assert_eq!(prev, RELEASE);
        assert!(p.event(&key(Keycode::Escape)));
        assert!(!p.captures_input());
        assert!(!p.event(&key(Keycode::Num1)));
    }
    #[test]
    fn activation_is_not_repeated_for_an_already_enabled_game_and_menu_is_guarded() {
        let mut p = Panel {
            gameplay: true,
            ..Default::default()
        };
        let mut e = key(Keycode::F12);
        if let Event::KeyDown { keymod, .. } = &mut e {
            *keymod = Mod::LCTRLMOD;
        }
        p.event(&e);
        assert!(p.visible);
        assert!(p.queue.contains(&(RELEASE & !11)));
        p.queue.clear();
        p.enabled = true;
        p.event(&e);
        p.event(&e);
        assert!(p.queue.is_empty());
        p.row = 8;
        p.select();
        assert!(p.queue.is_empty());
        p.main_menu = true;
        p.gameplay = false;
        p.select();
        assert!(!p.queue.is_empty());
    }
    #[test]
    #[ignore = "requires local USA ROM and diagnostic output directory"]
    fn native_sequences_fixture() {
        let root = std::path::PathBuf::from(std::env::var_os("ADVANCE_LIBERTY_CHEAT_TEST_DIR").unwrap());
        std::fs::create_dir_all(&root).unwrap();
        let rom = std::fs::read("roms/usa.gba").unwrap();
        for (row, index) in [4, 2, 7, 6, 0, 1, 5, 3, 9].into_iter().enumerate() {
            let offset = 0xcaa264 + index * 12;
            let expected = [32, 16, 64, 128, ENDS[row][0], ENDS[row][1]];
            for (i, value) in expected.into_iter().enumerate() {
                assert_eq!(
                    u16::from_le_bytes(rom[offset + i * 2..offset + i * 2 + 2].try_into().unwrap()),
                    value
                );
            }
        }
        let original = std::fs::read_to_string(
            "data/sessions/session-ad81ffa934e64a2f9ff36aaf67bc1ba3/input.gba-input",
        )
        .unwrap();
        let mut trace = String::from("gba-input v1\n");
        for line in original.lines().skip(1) {
            let frame = line
                .split_whitespace()
                .next()
                .unwrap()
                .parse::<u64>()
                .unwrap();
            if frame < 2000 {
                trace.push_str(line);
                trace.push('\n');
            }
        }
        trace.push_str("2000 3ff\n");
        let mut p = Panel {
            gameplay: true,
            ..Default::default()
        };
        let emit = |p: &mut Panel, frame: u64, trace: &mut String| {
            let mut i = 0;
            while !p.queue.is_empty() {
                trace.push_str(&format!("{} {:03x}\n", frame + i, p.input()));
                i += 1;
            }
        };
        p.activate();
        emit(&mut p, 2010, &mut trace);
        p.enabled = true;
        for row in 0..8 {
            p.row = row;
            p.select();
            emit(&mut p, 2100 + row as u64 * 100, &mut trace);
        }
        std::fs::write(root.join("cheats.gba-input"), trace).unwrap();
        // Same queue, main menu before the recorded Load Game confirmation.
        let mut trace = String::from("gba-input v1\n");
        for line in original.lines().skip(1) {
            let frame = line
                .split_whitespace()
                .next()
                .unwrap()
                .parse::<u64>()
                .unwrap();
            if frame < 470 {
                trace.push_str(line);
                trace.push('\n');
            }
        }
        trace.push_str("470 3ff\n");
        p.row = 8;
        p.gameplay = false;
        p.main_menu = true;
        p.select();
        emit(&mut p, 480, &mut trace);
        std::fs::write(root.join("levels.gba-input"), trace).unwrap();
    }
}
