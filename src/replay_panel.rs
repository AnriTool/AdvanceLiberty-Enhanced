//! Small replay browser and recording indicator, independent of guest menus.
use crate::replay::{Command, Entry};
use sdl2::{
    event::Event,
    keyboard::Keycode,
    pixels::Color,
    rect::Rect,
    render::{Canvas, RenderTarget},
};
#[derive(Default)]
pub struct Panel {
    pub visible: bool,
    pub recording: bool,
    pub playing: bool,
    pub command: Option<Command>,
    entries: Vec<Entry>,
    row: usize,
    message: Option<(String, std::time::Instant)>,
}
impl Panel {
    pub fn entries(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.row = 0;
        self.visible = true;
    }
    pub fn clear_notice(&mut self) {
        self.message = None;
    }
    pub fn notice(&mut self, text: String) {
        self.message = Some((text, std::time::Instant::now()));
    }
    pub fn event(&mut self, event: &Event, may_record: bool) -> bool {
        if let Event::KeyDown {
            keycode: Some(key),
            repeat: false,
            ..
        } = *event
        {
            if self.playing {
                if matches!(
                    key,
                    Keycode::Escape | Keycode::Return | Keycode::KpEnter | Keycode::F3
                ) {
                    self.command = Some(Command::Stop);
                }
                return key != Keycode::F11;
            }
            if self.visible {
                match key {
                    Keycode::F4 | Keycode::Escape => self.visible = false,
                    Keycode::Up => self.row = self.row.saturating_sub(1),
                    Keycode::Down => {
                        self.row = (self.row + 1).min(self.entries.len().saturating_sub(1))
                    }
                    Keycode::PageUp => self.row = self.row.saturating_sub(10),
                    Keycode::PageDown => {
                        self.row = (self.row + 10).min(self.entries.len().saturating_sub(1))
                    }
                    Keycode::Return | Keycode::KpEnter => {
                        if let Some(entry) = self.entries.get(self.row) {
                            self.command = Some(Command::Play(entry.path.clone()));
                            self.visible = false;
                        }
                    }
                    _ => {}
                }
                return true;
            }
            match key {
                Keycode::F2 => {
                    if may_record || self.recording {
                        self.command = Some(Command::Record);
                    }
                    return true;
                }
                Keycode::F3 => {
                    self.command = Some(Command::Latest);
                    return true;
                }
                Keycode::F4 => {
                    self.command = Some(Command::Browse);
                    return true;
                }
                _ => {}
            }
        }
        if self.playing || self.visible {
            if self.playing
                && matches!(
                    event,
                    Event::ControllerButtonDown {
                        button: sdl2::controller::Button::Start | sdl2::controller::Button::Guide,
                        ..
                    }
                )
            {
                self.command = Some(Command::Stop);
            }
            return !matches!(
                event,
                Event::Window { .. }
                    | Event::ControllerDeviceAdded { .. }
                    | Event::ControllerDeviceRemoved { .. }
            );
        }
        false
    }
    pub fn draw<T: RenderTarget>(&self, canvas: &mut Canvas<T>) -> Result<(), String> {
        let old = canvas.logical_size();
        let width = if old.1 == 160 { 720 } else { old.0.max(720) };
        canvas
            .set_logical_size(width, 480)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            if self.recording {
                canvas.set_draw_color(Color::RGB(12, 18, 27));
                canvas.fill_rect(Rect::new(width as i32 - 88, 6, 80, 26))?;
                canvas.set_draw_color(Color::RGB(200, 20, 25));
                canvas.fill_rect(Rect::new(width as i32 - 78, 12, 9, 9))?;
                crate::options::text_with_font(
                    canvas,
                    width as i32 - 63,
                    10,
                    "REC",
                    Color::RGB(255, 60, 60),
                    2,
                    None,
                )?;
            }
            if self.visible {
                let x = (width as i32 - 500) / 2;
                canvas.set_draw_color(Color::RGB(12, 18, 27));
                canvas.fill_rect(Rect::new(x, 30, 500, 370))?;
                let mut text = |y, s: &str, selected| {
                    crate::options::text_with_font(
                        canvas,
                        x + 14,
                        y,
                        s,
                        if selected {
                            Color::RGB(255, 220, 70)
                        } else {
                            Color::RGB(230, 235, 240)
                        },
                        if y == 74 || y == 372 { 1 } else { 2 },
                        None,
                    )
                };
                text(45, "REPLAYS (UTC)", true)?;
                text(74, "Up/Down: select  Enter: play  F4/Esc: close", false)?;
                if self.entries.is_empty() {
                    text(120, "No saved replays. F2 starts recording.", false)?;
                }
                let start = self.row.saturating_sub(9);
                for (i, entry) in self.entries.iter().enumerate().skip(start).take(10) {
                    text(
                        110 + (i - start) as i32 * 25,
                        &format!("{} {}", if i == self.row { ">" } else { " " }, entry.label),
                        i == self.row,
                    )?;
                }
                text(372, &format!("{} replay(s)", self.entries.len()), false)?;
            }
            if let Some((message, time)) = &self.message {
                if time.elapsed().as_secs() < 6 {
                    canvas.set_draw_color(Color::RGB(12, 18, 27));
                    canvas.fill_rect(Rect::new(12, 438, width.saturating_sub(24), 30))?;
                    crate::options::text_with_font(
                        canvas,
                        20,
                        448,
                        &message.chars().take(100).collect::<String>(),
                        Color::RGB(255, 220, 70),
                        1,
                        None,
                    )?;
                }
            }
            Ok(())
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
    fn key(keycode: Keycode, repeat: bool) -> Event {
        Event::KeyDown {
            timestamp: 0,
            window_id: 0,
            keycode: Some(keycode),
            scancode: None,
            keymod: sdl2::keyboard::Mod::NOMOD,
            repeat,
        }
    }
    #[test]
    fn shortcuts_browser_selection_and_playback_abort() {
        let mut panel = Panel::default();
        assert!(panel.event(&key(Keycode::F2, false), false));
        assert!(panel.command.is_none());
        panel.event(&key(Keycode::F2, false), true);
        assert!(matches!(panel.command.take(), Some(Command::Record)));
        panel.event(&key(Keycode::F2, true), true);
        assert!(panel.command.is_none());
        panel.entries(vec![
            Entry {
                path: "first".into(),
                label: "first".into(),
            },
            Entry {
                path: "last".into(),
                label: "last".into(),
            },
        ]);
        panel.event(&key(Keycode::Down, false), true);
        panel.event(&key(Keycode::Return, false), true);
        assert!(
            matches!(panel.command.take(),Some(Command::Play(path)) if path == std::path::Path::new("last"))
        );
        assert!(!panel.visible);
        panel.playing = true;
        for abort in [Keycode::Escape, Keycode::Return, Keycode::KpEnter] {
            assert!(panel.event(&key(abort, false), true));
            assert!(matches!(panel.command.take(), Some(Command::Stop)));
        }
        assert!(panel.event(&key(Keycode::F4, false), true));
        assert!(panel.command.is_none());
        assert!(!panel.event(&key(Keycode::F11, false), true));
    }
}
