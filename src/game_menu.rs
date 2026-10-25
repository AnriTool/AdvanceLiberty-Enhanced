//! Host presentation/navigation, backed by the game's normal menu commands.
//! Observations are read-only. Actions become recorded GBA button pulses.
use gba_core::Machine;
use sdl2::{
    event::Event,
    keyboard::Keycode,
    mouse::MouseButton,
    pixels::Color,
    rect::Rect,
    render::{Canvas, RenderTarget},
};
use std::sync::Arc;

pub const ROOT: u32 = 0x0832c3d0;
const OPTIONS: u32 = 0x0832f518;
const MENUS: [u32; 7] = [
    ROOT, 0x0832f518, 0x0832f280, 0x0832ed50, 0x0832efe8, 0x0832bea0, 0x0832db28,
];
fn word(b: &[u8], a: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(a..a + 4)?.try_into().ok()?))
}
fn rgb(v: u16) -> Color {
    let c = |s: u32| {
        let x = ((v >> s) & 31u16) as u8;
        (x << 3) | (x >> 2)
    };
    Color::RGB(c(0), c(5), c(10))
}

pub fn current_region(bus: &gba_core::MemMap) -> Option<usize> {
    if bus.rom.get(0xac..0xb0) != Some(b"BGTE") {
        return None;
    }
    // USA getter 08050f00 reads the current island at 020303d0.
    // Unload writes 0x7e7e; keep the last valid skin through that transition.
    word(&bus.ewram, 0x303d0)
        .filter(|r| *r < 3)
        .map(|r| r as usize)
}

#[derive(Clone)]
pub struct Skin {
    backgrounds: Vec<Vec<Color>>,
    region: usize,
    glyphs: Vec<u8>,
    widths: Vec<u8>,
    palette: Vec<Color>,
    labels: Vec<Vec<u8>>,
    locale: crate::menu_text::Locale,
    host: std::collections::BTreeMap<String, Vec<u8>>,
    font: Option<crate::localization::Font>,
}
impl Skin {
    pub fn load(rom: &[u8]) -> Result<Arc<Self>, String> {
        if rom.get(0xac..0xb0) != Some(b"BGTE") {
            return Err("Menu requires USA BGTE ROM".into());
        }
        let read = |a| word(rom, a).ok_or("Invalid menu resource");
        let font = read(0x1770)?
            .checked_sub(0x08000000)
            .ok_or("Invalid font pointer")? as usize;
        let widths = read(0x1774)?
            .checked_sub(0x08000000)
            .ok_or("Invalid width pointer")? as usize;
        let mut backgrounds = Vec::with_capacity(3);
        // Original Portland / Staunton / Shoreside image descriptors, not tints.
        for descriptor in [0xc98f60, 0xc98f7c, 0xc98f98] {
            let pointer = |o| -> Result<usize, String> {
                let p = read(descriptor + o)?;
                if p >> 24 != 8 {
                    return Err("Invalid menu background pointer".into());
                }
                Ok((p - 0x08000000) as usize)
            };
            if read(descriptor + 20)? != 0x0014001e {
                return Err("Invalid menu background size".into());
            }
            let palette = pointer(0)?;
            let tiles = pointer(8)?;
            let map = pointer(16)?;
            let mut background = Vec::with_capacity(240 * 160);
            // 30x20 text tilemap, 4bpp tiles, per-cell flips and palette banks.
            for y in 0..160 {
                for x in 0..240 {
                    let a = map + (y / 8 * 30 + x / 8) * 2;
                    let t = read(a)? as u16;
                    let xx = if t & 1024 != 0 { 7 - x % 8 } else { x % 8 };
                    let yy = if t & 2048 != 0 { 7 - y % 8 } else { y % 8 };
                    let b = *rom
                        .get(tiles + (t as usize & 1023) * 32 + yy * 4 + xx / 2)
                        .ok_or("Invalid background tile")?;
                    let mut v = (b >> (4 * (xx % 2))) as usize & 15;
                    if v != 0 {
                        v += (t as usize >> 12) * 16;
                    }
                    background.push(rgb(read(palette + v * 2)? as u16));
                }
            }
            backgrounds.push(background);
        }
        let mut labels = Vec::new();
        for i in 0..618 {
            let a = read(0xec4324 + i * 4)?
                .checked_sub(0x08000000)
                .ok_or("Invalid text pointer")? as usize;
            let tail = rom.get(a..).ok_or("Invalid menu string")?;
            let n = tail
                .iter()
                .take(4096)
                .position(|b| *b == 0)
                .ok_or("Unterminated menu string")?;
            labels.push(tail[..n].to_vec());
        }
        Ok(Arc::new(Self {
            backgrounds,
            region: 0,
            glyphs: rom
                .get(font..font + 224 * 256)
                .ok_or("Invalid font atlas")?
                .to_vec(),
            widths: rom
                .get(widths..widths + 224)
                .ok_or("Invalid font widths")?
                .to_vec(),
            palette: (0..16)
                .map(|i| read(0x8312d0 + i * 2).map(|v| rgb(v as u16)))
                .collect::<Result<_, _>>()?,
            labels,
            locale: Default::default(),
            host: crate::menu_text::catalog()
                .iter()
                .map(|(id, text)| {
                    (
                        id.clone(),
                        crate::menu_text::encode(text, None).expect("English menu text"),
                    )
                })
                .collect(),
            font: None,
        }))
    }
    pub fn apply_language(
        &mut self,
        language: Option<&crate::localization::Language>,
    ) -> Result<(), String> {
        if let Some(language) = language {
            self.locale.strings = language.menu.clone();
            self.font = Some(language.menu_font.clone());
            for (i, code) in crate::language_font::private_codes()
                .into_iter()
                .take(language.menu_font.len())
                .enumerate()
            {
                let slot = code as usize - 32;
                self.glyphs[slot * 256..(slot + 1) * 256]
                    .copy_from_slice(&language.menu_color_font.tiles[i]);
                self.widths[slot] = language.menu_color_font.widths[i];
            }
        }
        self.host = crate::menu_text::catalog()
            .keys()
            .map(|id| {
                Ok((
                    id.clone(),
                    crate::menu_text::encode(self.locale.text(id), self.font.as_ref())?,
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(())
    }
    pub fn region(&self) -> usize {
        self.region
    }
    pub fn set_region(&mut self, region: usize) {
        if region < self.backgrounds.len() {
            self.region = region;
        }
    }
    pub fn host(&self, id: &str) -> &[u8] {
        self.host.get(id).map_or(b"?", Vec::as_slice)
    }
    pub fn translate(&self, text: &str) -> String {
        self.locale.translate(text)
    }
    pub fn small_text<T: RenderTarget>(
        &self,
        canvas: &mut Canvas<T>,
        x: i32,
        y: i32,
        text: &str,
        color: Color,
        scale: u32,
    ) -> Result<(), String> {
        crate::options::text_with_font(
            canvas,
            x,
            y,
            &self.translate(text),
            color,
            scale,
            self.font.as_ref(),
        )
    }
    pub fn can_encode(&self, text: &str) -> bool {
        crate::menu_text::encode(text, self.font.as_ref()).is_ok()
    }
    pub fn text_utf8<T: RenderTarget>(
        &self,
        canvas: &mut Canvas<T>,
        x: i32,
        y: i32,
        text: &str,
        scale: u32,
        limit: i32,
    ) -> Result<(), String> {
        let encoded = crate::menu_text::encode_lossy(&self.translate(text), self.font.as_ref());
        self.text(canvas, x, y, &encoded, scale, limit)
    }
    pub fn background<T: RenderTarget>(&self, canvas: &mut Canvas<T>) -> Result<(), String> {
        let background = &self.backgrounds[self.region];
        for y in 0..160 {
            let mut x = 0;
            while x < 240 {
                let c = background[y * 240 + x];
                let start = x;
                while x < 240 && background[y * 240 + x] == c {
                    x += 1;
                }
                canvas.set_draw_color(c);
                canvas.fill_rect(Rect::new(
                    start as i32 * 3,
                    y as i32 * 3,
                    (x - start) as u32 * 3,
                    3,
                ))?;
            }
        }
        Ok(())
    }
    pub fn label(&self, id: u32) -> &[u8] {
        if let Some(key) = crate::menu_text::native_key(id) {
            if id == 9 || self.locale.strings.get(key).is_some_and(|s| !s.is_empty()) {
                return self.host(key);
            }
        }
        self.labels.get(id as usize).map_or(b"?", Vec::as_slice)
    }
    fn width(&self, text: &[u8], scale: u32) -> i32 {
        text.iter()
            .map(|b| (self.widths[(*b).max(32) as usize - 32] as i32 + 2) * scale as i32)
            .sum()
    }
    pub fn text<T: RenderTarget>(
        &self,
        canvas: &mut Canvas<T>,
        x: i32,
        y: i32,
        text: &[u8],
        scale: u32,
        limit: i32,
    ) -> Result<(), String> {
        let mut cursor = x;
        for &b in text {
            let b = if b < 32 { b'?' } else { b };
            let index = b as usize - 32;
            let advance = (self.widths[index] as i32 + 2) * scale as i32;
            if cursor + advance > limit {
                break;
            }
            for yy in 0..16 {
                for xx in 0..16 {
                    let shade = self.glyphs
                        [index * 256 + (yy / 8 * 2 + xx / 8) * 64 + yy % 8 * 8 + xx % 8]
                        as usize;
                    if shade == 0 {
                        continue;
                    }
                    canvas.set_draw_color(self.palette[shade.min(15)]);
                    canvas.fill_rect(Rect::new(
                        cursor + xx as i32 * scale as i32,
                        y + yy as i32 * scale as i32,
                        scale,
                        scale,
                    ))?;
                }
            }
            cursor += advance;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct View {
    pub descriptor: u32,
    pub title: u32,
    pub ids: Vec<u32>,
    pub native_row: usize,
}
#[derive(Clone, Copy)]
struct Plan {
    descriptor: u32,
    row: usize,
    frames: u32,
    confirm: bool,
}
#[derive(Default)]
pub struct Frontend {
    pub view: Option<View>,
    pub row: usize,
    pub modal: bool,
    plan: Option<Plan>,
    pulse: Option<(u16, u8)>,
    back: bool,
    quiet_pulse: bool,
    save_exit: Option<SaveExit>,
    save_exit_error: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExitStage {
    Root,
    BackFrom(u32),
    Confirm,
    Save,
    Writing,
    Commit,
    Complete,
}
struct SaveExit {
    stage: ExitStage,
    frames: u32,
}
pub enum Action {
    None,
    Options,
    Quit,
}
impl Frontend {
    pub fn interested(key: u32) -> bool {
        matches!(
            key,
            0x080011ed
                | 0x080073d9
                | 0x08006dc1
                | 0x08007cf5
                | 0x08007a15
                | 0x08007b9d
                | 0x080013ad
                | 0x08007ad1
                | 0x08007a2d
                | 0x08007aeb
        )
    }
    pub fn observe(&mut self, m: &Machine, key: u32) {
        if matches!(key, 0x08007ad1 | 0x08007a2d | 0x08007aeb) {
            if self.save_exit_active() {
                match key {
                    0x08007ad1 => self.save_exit.as_mut().unwrap().stage = ExitStage::Commit,
                    0x08007a2d => {
                        self.fail_save_exit("The game cannot save now. Exit cancelled.".into())
                    }
                    _ => self
                        .fail_save_exit("The game reported a save error. Exit cancelled.".into()),
                }
            }
            return;
        }
        if key == 0x08007a15 {
            if let Some(exit) = self.save_exit.as_mut() {
                exit.stage = ExitStage::Writing;
            }
        }
        if key != 0x080011ed {
            self.view = None;
            self.plan = None;
            self.modal = matches!(key, 0x080073d9 | 0x08006dc1 | 0x08007cf5);
            return;
        }
        let descriptor = m.cpu.regs[0];
        if !MENUS.contains(&descriptor) {
            self.view = None;
            return;
        }
        let a = (descriptor - 0x08000000) as usize;
        let Some(count) = word(&m.bus.rom, a + 20).filter(|n| (1..=8).contains(n)) else {
            self.view = None;
            return;
        };
        let Some(title) = word(&m.bus.rom, a) else {
            return;
        };
        let mut ids = Vec::new();
        for i in 0..count as usize {
            let Some(id) = word(&m.bus.rom, a + 28 + i * 32).filter(|id| *id < 618) else {
                self.view = None;
                return;
            };
            ids.push(id);
        }
        let native_row = m.cpu.regs[1] as usize;
        if native_row >= ids.len() {
            self.view = None;
            return;
        }
        if self
            .view
            .as_ref()
            .is_none_or(|v| v.descriptor != descriptor)
        {
            self.row = native_row;
            self.plan = None;
        } else if self.plan.is_none()
            && self.row < ids.len()
            && self
                .view
                .as_ref()
                .is_some_and(|v| v.native_row != native_row)
        {
            self.row = native_row;
        }
        self.modal = false;
        self.view = Some(View {
            descriptor,
            title,
            ids,
            native_row,
        });
    }
    fn count(&self) -> usize {
        self.view.as_ref().map_or(0, |v| {
            v.ids.len()
                + if v.descriptor == ROOT {
                    2
                } else if v.descriptor == OPTIONS {
                    1
                } else {
                    0
                }
        })
    }
    pub fn busy(&self) -> bool {
        self.plan.is_some() || self.pulse.is_some() || self.back
    }
    pub fn cancel_navigation(&mut self) {
        self.plan = None;
        self.pulse = None;
        self.back = false;
        self.quiet_pulse = false;
    }
    pub fn can_save_exit(&self) -> bool {
        self.view.is_some() && self.save_exit.is_none()
    }
    pub fn begin_save_exit(&mut self) -> bool {
        if !self.can_save_exit() {
            return false;
        }
        self.cancel_navigation();
        self.save_exit_error = None;
        self.save_exit = Some(SaveExit {
            stage: ExitStage::Root,
            frames: 0,
        });
        true
    }
    pub fn save_exit_active(&self) -> bool {
        self.save_exit
            .as_ref()
            .is_some_and(|s| s.stage != ExitStage::Complete)
    }
    pub fn save_exit_needs_commit(&self) -> bool {
        self.save_exit
            .as_ref()
            .is_some_and(|s| s.stage == ExitStage::Commit)
    }
    pub fn save_exit_complete(&self) -> bool {
        self.save_exit
            .as_ref()
            .is_some_and(|s| s.stage == ExitStage::Complete)
    }
    pub fn finish_save_exit(&mut self, result: Result<(), String>) {
        if !self.save_exit_needs_commit() {
            return;
        }
        match result {
            Ok(()) => self.save_exit.as_mut().unwrap().stage = ExitStage::Complete,
            Err(error) => {
                self.fail_save_exit(format!("Cannot write save: {error}. Exit cancelled."))
            }
        }
    }
    fn fail_save_exit(&mut self, error: String) {
        self.save_exit = None;
        self.cancel_navigation();
        self.save_exit_error = Some(error);
    }
    pub fn take_save_exit_error(&mut self) -> Option<String> {
        self.save_exit_error.take()
    }
    fn advance_save_exit(&mut self) {
        let Some(exit) = self.save_exit.as_mut() else {
            return;
        };
        if matches!(exit.stage, ExitStage::Commit | ExitStage::Complete) {
            return;
        }
        exit.frames += 1;
        if exit.frames > 1800 {
            self.fail_save_exit("Save did not finish. Exit cancelled.".into());
            return;
        }
        if self.busy() {
            return;
        }
        let Some(v) = &self.view else {
            return;
        };
        let stage = self.save_exit.as_ref().unwrap().stage;
        let action = match stage {
            ExitStage::Root if v.descriptor == ROOT => Some((5, ExitStage::Confirm)),
            ExitStage::Root if v.descriptor == 0x0832db28 => Some((1, ExitStage::Save)),
            ExitStage::Root => {
                let descriptor = v.descriptor;
                self.back = true;
                self.save_exit.as_mut().unwrap().stage = ExitStage::BackFrom(descriptor);
                None
            }
            ExitStage::BackFrom(old) if v.descriptor != old => {
                self.save_exit.as_mut().unwrap().stage = ExitStage::Root;
                None
            }
            ExitStage::Confirm if v.descriptor == 0x0832db28 => Some((1, ExitStage::Save)),
            _ => None,
        };
        if let Some((row, next)) = action {
            self.row = row;
            self.plan = Some(Plan {
                descriptor: v.descriptor,
                row,
                frames: 0,
                confirm: true,
            });
            self.save_exit.as_mut().unwrap().stage = next;
        }
    }
    pub fn press(&mut self, mask: u16) {
        if !self.busy() {
            self.quiet_pulse = false;
            self.pulse = Some((0x3ff & !mask, 0));
        }
    }
    pub fn mute_navigation_audio(&self) -> bool {
        self.quiet_pulse
    }
    pub fn input(&mut self) -> u16 {
        self.advance_save_exit();
        if let Some((bits, tick)) = self.pulse {
            self.pulse = if tick >= 11 {
                None
            } else {
                Some((bits, tick + 1))
            };
            return if tick < 4 { bits } else { 0x3ff };
        }
        self.quiet_pulse = false;
        if self.back {
            self.back = false;
            self.plan = None;
            self.pulse = Some((0x3fd, 1));
            return 0x3fd;
        }
        if let Some(mut p) = self.plan {
            p.frames += 12;
            if p.frames > 360 {
                self.plan = None;
                return 0x3ff;
            }
            if let Some(v) = &self.view {
                if v.descriptor != p.descriptor {
                    self.plan = None;
                    return 0x3ff;
                }
                let mask = if v.native_row == p.row {
                    self.plan = None;
                    if !p.confirm {
                        return 0x3ff;
                    }
                    1
                } else {
                    self.plan = Some(p);
                    if v.native_row > p.row {
                        64
                    } else {
                        128
                    }
                };
                // A click/confirmation can require hidden cursor alignment.
                // Keep those intermediate beeps out of presentation audio;
                // ordinary Up/Down and the final confirmation remain audible.
                self.quiet_pulse = p.confirm && mask != 1;
                self.pulse = Some((0x3ff & !mask, 1));
                return 0x3ff & !mask;
            } else {
                self.plan = None;
            }
        }
        0x3ff
    }
    pub fn event(&mut self, e: &Event) -> Action {
        if self.save_exit_active() {
            return Action::None;
        }
        let key = match *e {
            Event::KeyDown {
                keycode: Some(k),
                repeat: false,
                ..
            } => Some(k),
            Event::ControllerButtonDown { button, .. } => match button {
                sdl2::controller::Button::DPadUp => Some(Keycode::Up),
                sdl2::controller::Button::DPadDown => Some(Keycode::Down),
                sdl2::controller::Button::A => Some(Keycode::Return),
                sdl2::controller::Button::B
                | sdl2::controller::Button::Start
                | sdl2::controller::Button::Guide => Some(Keycode::Escape),
                _ => None,
            },
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } if (30..690).contains(&x) && (88..408).contains(&y) => {
                let row = ((y - 88) / 40) as usize;
                if row >= self.count() {
                    return Action::None;
                }
                self.row = row;
                Some(Keycode::Return)
            }
            _ => None,
        };
        let count = self.count();
        if count == 0 {
            return Action::None;
        }
        match key {
            Some(Keycode::Up | Keycode::Down) => {
                self.row = if key == Some(Keycode::Up) {
                    self.row.saturating_sub(1)
                } else {
                    (self.row + 1).min(count - 1)
                };
                let v = self.view.as_ref().unwrap();
                self.plan = if self.row < v.ids.len() {
                    Some(Plan {
                        descriptor: v.descriptor,
                        row: self.row,
                        frames: 0,
                        confirm: false,
                    })
                } else {
                    None
                };
            }
            Some(Keycode::Escape) => {
                self.plan = None;
                self.back = true;
            }
            Some(Keycode::Return | Keycode::KpEnter | Keycode::J) => {
                let v = self.view.as_ref().unwrap();
                if self.row == v.ids.len() {
                    return Action::Options;
                }
                if self.row > v.ids.len() {
                    return Action::Quit;
                }
                self.plan = Some(Plan {
                    descriptor: v.descriptor,
                    row: self.row,
                    frames: 0,
                    confirm: true,
                });
            }
            _ => {}
        }
        Action::None
    }
    pub fn draw<T: RenderTarget>(&self, canvas: &mut Canvas<T>, skin: &Skin) -> Result<(), String> {
        if self.save_exit_active() {
            skin.background(canvas)?;
            skin.text(canvas, 40, 32, skin.host("quit_to_desktop"), 2, 680)?;
            skin.text(canvas, 40, 180, skin.host("saving_game"), 2, 680)?;
            canvas.present();
            return Ok(());
        }
        let Some(v) = &self.view else {
            return Ok(());
        };
        skin.background(canvas)?;
        let title = if v.title == 0 {
            skin.label(0x18)
        } else {
            skin.label(v.title)
        };
        skin.text(
            canvas,
            ((720 - skin.width(title, 2)) / 2).max(40),
            26,
            title,
            2,
            680,
        )?;
        for i in 0..self.count() {
            let y = 88 + i as i32 * 40;
            if i == self.row {
                canvas.set_draw_color(Color::RGB(12, 25, 60));
                canvas.fill_rect(Rect::new(30, y, 660, 37))?;
                canvas.set_draw_color(Color::RGB(255, 235, 160));
                canvas.draw_rect(Rect::new(30, y, 660, 37))?;
            }
            let label = if i < v.ids.len() {
                skin.label(v.ids[i])
            } else if i == v.ids.len() {
                skin.host("enhanced_options")
            } else {
                skin.host("quit_to_desktop")
            };
            skin.text(
                canvas,
                ((720 - skin.width(label, 2)) / 2).max(48),
                y + 3,
                label,
                2,
                672,
            )?;
        }
        skin.text(canvas, 40, 434, skin.host("pause_hint"), 1, 680)?;
        canvas.present();
        Ok(())
    }
}

#[cfg(test)]
#[path = "game_menu_native_tests.rs"]
mod native_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn island_region_rejects_unload_sentinel_and_other_roms() {
        let mut rom = vec![0; 0x100];
        rom[0xac..0xb0].copy_from_slice(b"BGTE");
        let mut bus = gba_core::MemMap::new(rom);
        for value in [0u32, 1, 2, 3, 0x7e7e, u32::MAX] {
            bus.ewram[0x303d0..0x303d4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(current_region(&bus), (value < 3).then_some(value as usize));
        }
        bus.ewram[0x303d0..0x303d4].fill(0);
        bus.rom[0xac] = 0;
        assert_eq!(current_region(&bus), None);
    }
    use super::*;
    fn event(key: Keycode) -> Event {
        Event::KeyDown {
            timestamp: 0,
            window_id: 0,
            keycode: Some(key),
            scancode: None,
            keymod: sdl2::keyboard::Mod::NOMOD,
            repeat: false,
        }
    }
    fn machine() -> Machine {
        let mut rom = vec![0; 0x32c3d0 + 24 + 6 * 32];
        let a = (ROOT - 0x08000000) as usize;
        rom[a + 20..a + 24].copy_from_slice(&6u32.to_le_bytes());
        for (i, id) in [8u32, 0x3d, 0xd, 0xc, 0x231, 9].iter().enumerate() {
            rom[a + 28 + i * 32..a + 32 + i * 32].copy_from_slice(&id.to_le_bytes());
        }
        let mut m = Machine::new(rom);
        m.cpu.regs[0] = ROOT;
        m
    }
    #[test]
    fn navigation_waits_for_observed_selection_and_records_only_gba_buttons() {
        let mut m = machine();
        let cpu = m.cpu.clone();
        let ram = m.bus.ewram.clone();
        let mut f = Frontend::default();
        f.observe(&m, 0x080011ed);
        for _ in 0..3 {
            f.event(&event(Keycode::Down));
        }
        assert_eq!(f.row, 3);
        assert!(m.cpu == cpu);
        assert_eq!(m.bus.ewram, ram);
        f.event(&event(Keycode::Return));
        let mut previous = 0x3ff;
        let mut downs = 0;
        let mut confirms = 0;
        for _ in 0..100 {
            let bits = f.input();
            assert!(matches!(bits, 0x3ff | 0x37f | 0x3fe));
            if bits != previous && bits != 0x3ff {
                if bits == 0x37f {
                    downs += 1;
                    m.cpu.regs[1] += 1;
                    f.observe(&m, 0x080011ed);
                } else {
                    confirms += 1;
                    assert_eq!(m.cpu.regs[1], 3);
                }
            }
            previous = bits;
            if !f.busy() {
                break;
            }
        }
        assert_eq!((downs, confirms), (3, 1));
        assert!(!f.busy());
        assert_eq!(m.bus.ewram, ram);
    }
    #[test]
    fn settings_and_desktop_exit_are_host_actions_and_stalled_navigation_is_bounded() {
        let m = machine();
        let mut f = Frontend::default();
        f.observe(&m, 0x080011ed);
        f.row = 6;
        assert!(matches!(f.event(&event(Keycode::Return)), Action::Options));
        assert!(!f.busy());
        f.row = 7;
        assert!(matches!(f.event(&event(Keycode::Return)), Action::Quit));
        f.row = 2;
        f.event(&event(Keycode::Return));
        for _ in 0..500 {
            assert_ne!(f.input(), 0x3fe);
        }
        assert!(!f.busy());
        f.observe(&m, 0x08006dc1);
        assert!(f.modal);
        assert!(f.view.is_none());
        f.observe(&m, 0x080013ad);
        assert!(!f.modal);
    }
    #[test]
    fn high_to_auto_uses_up_and_navigation_does_not_confirm() {
        let mut m = machine();
        m.cpu.regs[1] = 3;
        let mut f = Frontend::default();
        f.observe(&m, 0x080011ed);
        f.event(&event(Keycode::Up));
        assert_eq!(f.input(), 0x3bf);
        m.cpu.regs[1] = 2;
        f.observe(&m, 0x080011ed);
        for _ in 0..20 {
            assert_ne!(f.input(), 0x3fe);
        }
        assert!(!f.busy());
        f.event(&event(Keycode::Up));
        f.event(&event(Keycode::Up));
        let mut previous = 0x3ff;
        let mut up = 0;
        for _ in 0..60 {
            let bits = f.input();
            assert_ne!(bits, 0x37f);
            assert_ne!(bits, 0x3fe);
            if bits == 0x3bf && previous != bits {
                up += 1;
                m.cpu.regs[1] -= 1;
                f.observe(&m, 0x080011ed);
            }
            previous = bits;
        }
        assert_eq!(up, 2);
        assert_eq!(f.row, 0);
        assert!(!f.busy());
        f.event(&event(Keycode::Return));
        assert_eq!(f.input(), 0x3fe);
    }
    #[test]
    fn save_exit_requires_game_success_and_disk_commit_and_errors_never_exit() {
        let m = machine();
        let mut f = Frontend::default();
        assert!(!f.begin_save_exit());
        f.observe(&m, 0x080011ed);
        assert!(f.begin_save_exit());
        assert!(!f.save_exit_complete());
        f.observe(&m, 0x08007a15);
        f.observe(&m, 0x08007a2d);
        assert!(!f.save_exit_active());
        assert!(!f.save_exit_complete());
        assert!(f.take_save_exit_error().unwrap().contains("cannot save"));
        f.observe(&m, 0x080011ed);
        assert!(f.begin_save_exit());
        f.observe(&m, 0x08007a15);
        f.observe(&m, 0x08007aeb);
        assert!(!f.save_exit_complete());
        assert!(f.take_save_exit_error().is_some());
        f.observe(&m, 0x080011ed);
        assert!(f.begin_save_exit());
        f.observe(&m, 0x08007a15);
        f.observe(&m, 0x08007ad1);
        assert!(f.save_exit_needs_commit());
        assert!(!f.save_exit_complete());
        f.finish_save_exit(Err("disk full".into()));
        assert!(!f.save_exit_complete());
        assert!(f.take_save_exit_error().unwrap().contains("disk full"));
    }
}
