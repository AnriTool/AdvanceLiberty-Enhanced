//! Host-owned pause screen. It never reads or writes guest RAM.
use crate::{
    controls::{Binding, Controls},
    render_settings::{Renderer, Settings, TextureMapping},
};
use sdl2::{
    event::Event, keyboard::Keycode, mouse::MouseButton, pixels::Color, rect::Rect, render::Canvas,
    video::Window,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preferences {
    pub render: Settings,
    pub fullscreen: bool,
    pub language: Option<String>,
}
impl Preferences {
    pub fn encode(&self) -> String {
        format!(
            "# Enhanced Options\nRenderer = {:?}\nTextures = {:?}\nFullscreen = {}\nHudRadar = {}\nHudStatus = {}\nHudMessages = {}\nAspect = {:?}\nLanguage = {}\n",
            self.render.renderer, self.render.textures, self.fullscreen,
            self.render.hud.radar.percent(),self.render.hud.status.percent(),self.render.hud.messages.percent(),self.render.aspect,
            self.language.as_deref().unwrap_or("Original")
        )
    }
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut p = Self::default();
        let mut seen = std::collections::HashSet::new();
        for raw in text.lines() {
            let raw = raw.split('#').next().unwrap().trim();
            if raw.is_empty() {
                continue;
            }
            let (key, value) = raw.split_once('=').ok_or("Expected setting = value")?;
            let (key, value) = (key.trim(), value.trim());
            if !seen.insert(key.to_owned()) {
                return Err(format!("Duplicate setting: {key}"));
            }
            match (key, value) {
                ("Language", "Original") => p.language = None,
                ("Language", value) if crate::localization::valid_file_name(value) => {
                    p.language = Some(value.to_owned())
                }
                ("Renderer", "Enhanced") => p.render.renderer = Renderer::Enhanced,
                ("Renderer", "Original") => p.render.renderer = Renderer::Original,
                ("Textures", "Perspective") => p.render.textures = TextureMapping::Perspective,
                ("Textures", "Affine") => p.render.textures = TextureMapping::Affine,
                ("Fullscreen", "true") => p.fullscreen = true,
                ("Fullscreen", "false") => p.fullscreen = false,
                ("Aspect", "Original") => p.render.aspect = crate::viewport::Aspect::Original,
                ("Aspect", "Wide") => p.render.aspect = crate::viewport::Aspect::Wide,
                ("HudRadar", value) => p.render.hud.radar = crate::hud_layout::Size::parse(value)?,
                ("HudStatus", value) => {
                    p.render.hud.status = crate::hud_layout::Size::parse(value)?
                }
                ("HudMessages", value) => {
                    p.render.hud.messages = crate::hud_layout::Size::parse(value)?
                }
                _ => return Err(format!("Invalid setting: {key} = {value}")),
            }
        }
        Ok(p)
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}
fn staged(path: &Path, text: &str) -> Result<PathBuf, String> {
    let temp = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&temp);
        return Err(e.to_string());
    }
    Ok(temp)
}
// Stage both files before committing, keep a backup of the user's bindings.
// A failed second rename restores the previous preferences file.
pub fn save(
    controls_path: &Path,
    prefs_path: &Path,
    controls: &Controls,
    prefs: &Preferences,
) -> Result<(), String> {
    let encoded = controls.encode();
    Controls::parse(&encoded)?;
    let presentation = prefs.encode();
    Preferences::parse(&presentation)?;
    let old = match fs::read(prefs_path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let a = staged(controls_path, &encoded)?;
    let b = match staged(prefs_path, &presentation) {
        Ok(p) => p,
        Err(e) => {
            let _ = fs::remove_file(a);
            return Err(e);
        }
    };
    let result = (|| -> Result<(), String> {
        if controls_path.exists() {
            fs::copy(
                controls_path,
                controls_path.with_extension("cfg.options-backup"),
            )
            .map_err(|e| e.to_string())?;
        }
        fs::rename(&b, prefs_path).map_err(|e| e.to_string())?;
        if let Err(e) = fs::rename(&a, controls_path) {
            let rollback = match old {
                Some(ref bytes) => fs::write(prefs_path, bytes),
                None => fs::remove_file(prefs_path),
            };
            return Err(format!("{e}; preferences rollback: {rollback:?}"));
        }
        Ok(())
    })();
    let _ = fs::remove_file(a);
    let _ = fs::remove_file(b);
    result
}

pub enum Command {
    None,
    Save,
    Close,
    Quit,
}
pub struct Menu {
    pub controls: Controls,
    pub prefs: Preferences,
    pub language_requested: bool,
    section: Option<usize>,
    hud_page: bool,
    graphics_page: bool,
    language_page: bool,
    languages: Vec<crate::localization::Choice>,
    row: usize,
    capture: Option<bool>, // false replaces, true adds an alternative
    pub message: String,
    confirm_quit: bool,
    exit_only: bool,
    cached: Option<sdl2::surface::Surface<'static>>,
    pub skin: Option<std::sync::Arc<crate::game_menu::Skin>>,
}
impl Menu {
    pub fn new(controls: Controls, prefs: Preferences) -> Self {
        Self {
            controls,
            prefs,
            section: None,
            hud_page: false,
            graphics_page: false,
            language_page: false,
            language_requested: false,
            languages: Vec::new(),
            row: 0,
            capture: None,
            message: String::new(),
            confirm_quit: false,
            exit_only: false,
            cached: None,
            skin: None,
        }
    }
    pub fn with_languages(mut self, languages: Vec<crate::localization::Choice>) -> Self {
        self.languages = languages;
        self
    }
    pub fn request_quit(&mut self) {
        self.confirm_quit = true;
        self.exit_only = true;
        self.row = 1;
    }
    fn text<T: sdl2::render::RenderTarget>(
        &self,
        canvas: &mut Canvas<T>,
        x: i32,
        y: i32,
        value: &str,
        color: Color,
        scale: u32,
    ) -> Result<(), String> {
        self.text_with_font(canvas, x, y, value, color, scale, None)
    }
    fn text_with_font<T: sdl2::render::RenderTarget>(
        &self,
        canvas: &mut Canvas<T>,
        x: i32,
        y: i32,
        value: &str,
        color: Color,
        scale: u32,
        font: Option<&crate::localization::Font>,
    ) -> Result<(), String> {
        if let Some(skin) = &self.skin {
            if value.is_ascii() {
                return skin.text(
                    canvas,
                    x,
                    y,
                    value.to_uppercase().as_bytes(),
                    if scale >= 3 { 2 } else { 1 },
                    690,
                );
            }
        }
        text_with_font(canvas, x, y, value, color, scale, font)
    }
    pub fn refresh_languages(&mut self, languages: Vec<crate::localization::Choice>) {
        self.languages = languages;
        self.invalidate();
    }
    fn language_label(&self) -> String {
        self.prefs
            .language
            .as_ref()
            .and_then(|file| self.languages.iter().find(|choice| &choice.file == file))
            .map_or_else(
                || "Original game / ROM".into(),
                |choice| choice.label.clone(),
            )
    }
    fn count(&self) -> usize {
        if self.confirm_quit {
            2
        } else if self.language_page {
            self.languages.len() + 2
        } else if self.graphics_page {
            3
        } else if self.hud_page {
            5
        } else {
            self.section
                .map_or(12, |s| Controls::actions(s).len() + usize::from(s == 1))
        }
    }
    fn first_row(&self) -> usize {
        self.row.saturating_sub(10)
    }
    fn back(&mut self) -> Command {
        if self.capture.take().is_some() {
            self.message.clear();
        } else if self.confirm_quit {
            if self.exit_only {
                return Command::Close;
            }
            self.confirm_quit = false;
            self.row = 11;
        } else if self.language_page {
            self.language_page = false;
            self.row = 8;
            self.message.clear();
        } else if self.graphics_page {
            self.graphics_page = false;
            self.row = 5;
            self.message.clear();
        } else if self.hud_page {
            self.hud_page = false;
            self.row = 7;
            self.message.clear();
        } else if let Some(s) = self.section.take() {
            self.row = s + 1;
            self.message.clear();
        } else {
            return Command::Close;
        }
        Command::None
    }
    fn activate(&mut self) -> Command {
        self.message.clear();
        if self.confirm_quit {
            return if self.row == 0 {
                Command::Quit
            } else {
                self.back()
            };
        }
        if self.language_page {
            if self.row == self.languages.len() + 1 {
                return self.back();
            }
            self.prefs.language = self
                .row
                .checked_sub(1)
                .map(|i| self.languages[i].file.clone());
            self.language_requested = true;
            self.message = "Language selected. Save to apply in this game.".into();
            return Command::None;
        }
        if self.graphics_page {
            match self.row {
                0 => self.prefs.render.toggle_textures(),
                1 => self.prefs.render.aspect.toggle(),
                _ => return self.back(),
            }
            return Command::None;
        }
        if self.hud_page {
            self.prefs.render.hud.change(self.row, false);
            return Command::None;
        }
        if self.section == Some(1) && self.row == Controls::actions(1).len() {
            self.controls.toggle_mouse_aim();
            return Command::None;
        }
        if self.section.is_some() {
            self.capture = Some(false);
            return Command::None;
        }
        match self.row {
            0 => {
                let _ = self.controls.toggle_mode();
            }
            1..=3 => {
                self.section = Some(self.row - 1);
                self.row = 0;
            }
            4 => self.prefs.render.toggle_renderer(),
            5 => {
                self.graphics_page = true;
                self.row = 0;
            }
            6 => self.prefs.fullscreen = !self.prefs.fullscreen,
            7 => {
                self.hud_page = true;
                self.row = 0;
            }
            8 => {
                self.language_page = true;
                self.row = self
                    .prefs
                    .language
                    .as_ref()
                    .and_then(|file| self.languages.iter().position(|c| &c.file == file))
                    .map_or(0, |i| i + 1);
            }
            9 => return Command::Save,
            10 => return Command::Close,
            11 => {
                self.confirm_quit = true;
                self.row = 1;
            }
            _ => {}
        }
        Command::None
    }
    pub fn event(&mut self, event: &Event) -> Command {
        self.cached = None;
        if let Event::KeyDown {
            keycode: Some(Keycode::Escape),
            repeat: false,
            ..
        } = event
        {
            return self.back();
        }
        if let Some(add) = self.capture {
            let key = match *event {
                Event::KeyDown {
                    scancode: Some(code),
                    repeat: false,
                    ..
                } => Some(Binding::Key(code)),
                Event::MouseButtonDown { mouse_btn, .. } => match mouse_btn {
                    MouseButton::Left => Some(Binding::MouseLeft),
                    MouseButton::Middle => Some(Binding::MouseMiddle),
                    MouseButton::Right => Some(Binding::MouseRight),
                    _ => None,
                },
                _ => None,
            };
            if let Some(key) = key {
                match self
                    .controls
                    .rebind(self.section.unwrap(), self.row, Some(key), add)
                {
                    Ok(()) => {
                        self.capture = None;
                        self.message = "Binding updated (not saved yet)".into();
                    }
                    Err(e) => self.message = e,
                }
            }
            return Command::None;
        }
        if matches!(event, Event::KeyDown { repeat:true, keycode:Some(k), .. } if !matches!(*k,Keycode::Up|Keycode::Down))
        {
            return Command::None;
        }
        let key = match *event {
            Event::KeyDown {
                keycode: Some(key), ..
            } => Some(key),
            Event::ControllerButtonDown { button, .. } => match button {
                sdl2::controller::Button::DPadUp => Some(Keycode::Up),
                sdl2::controller::Button::DPadDown => Some(Keycode::Down),
                sdl2::controller::Button::A => Some(Keycode::Return),
                sdl2::controller::Button::B => Some(Keycode::Escape),
                _ => None,
            },
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } if (30..690).contains(&x) && y >= 104 => {
                let visible = ((y - 104) / 25) as usize;
                let row = visible + self.first_row();
                if visible < 11 && row < self.count() {
                    self.row = row;
                    return self.activate();
                }
                None
            }
            _ => None,
        };
        match key {
            Some(Keycode::Up) => self.row = (self.row + self.count() - 1) % self.count(),
            Some(Keycode::Down) => self.row = (self.row + 1) % self.count(),
            Some(Keycode::Escape) => return self.back(),
            Some(Keycode::Return | Keycode::KpEnter) => return self.activate(),
            Some(
                Keycode::Left
                | Keycode::Right
                | Keycode::Insert
                | Keycode::Delete
                | Keycode::Backspace,
            ) if self.section == Some(1) && self.row == Controls::actions(1).len() => {
                return self.activate();
            }
            Some(Keycode::Left | Keycode::Right) if self.graphics_page && self.row < 2 => {
                return self.activate()
            }
            Some(Keycode::Left | Keycode::Right) if self.hud_page => {
                self.prefs
                    .render
                    .hud
                    .change(self.row, key == Some(Keycode::Left));
            }
            Some(Keycode::Left | Keycode::Right)
                if self.section.is_none() && matches!(self.row, 0 | 4 | 5 | 6) =>
            {
                return self.activate()
            }
            Some(Keycode::Insert) if self.section.is_some() => self.capture = Some(true),
            Some(Keycode::Delete | Keycode::Backspace) if self.section.is_some() => {
                self.message =
                    match self
                        .controls
                        .rebind(self.section.unwrap(), self.row, None, false)
                    {
                        Ok(()) => "Binding cleared (not saved yet)".into(),
                        Err(e) => e,
                    };
            }
            _ => {}
        }
        Command::None
    }
    pub fn invalidate(&mut self) {
        self.cached = None;
    }
    pub fn draw(
        &mut self,
        canvas: &mut Canvas<Window>,
        texture: &mut sdl2::render::Texture<'_>,
    ) -> Result<(), String> {
        if self.cached.is_none() {
            let mut surface =
                sdl2::surface::Surface::new(720, 480, sdl2::pixels::PixelFormatEnum::RGB24)?
                    .into_canvas()?;
            self.draw_contents(&mut surface)?;
            let surface = surface.into_surface();
            texture
                .update(
                    None,
                    surface.without_lock().ok_or("Menu surface is locked")?,
                    surface.pitch() as usize,
                )
                .map_err(|e| e.to_string())?;
            self.cached = Some(surface);
        }
        canvas
            .set_logical_size(720, 480)
            .map_err(|e| e.to_string())?;
        canvas.set_draw_color(Color::RGB(13, 22, 39));
        canvas.clear();
        canvas.copy(texture, None, None)?;
        canvas.present();
        Ok(())
    }
    fn draw_contents<T: sdl2::render::RenderTarget>(
        &self,
        canvas: &mut Canvas<T>,
    ) -> Result<(), String> {
        canvas.set_draw_color(Color::RGB(13, 22, 39));
        canvas.clear();
        if let Some(skin) = &self.skin {
            skin.background(canvas)?;
        } else {
            canvas.set_draw_color(Color::RGB(32, 51, 73));
            canvas.fill_rect(Rect::new(20, 20, 680, 440))?;
        }
        self.text(
            canvas,
            38,
            36,
            "ENHANCED OPTIONS",
            Color::RGB(255, 213, 92),
            3,
        )?;
        let subtitle = if self.confirm_quit {
            "Quit game? Unsaved options will be discarded."
        } else if self.language_page {
            "LANGUAGE / APPLIES WHEN SAVED"
        } else if self.graphics_page {
            "GRAPHICS / ENHANCED RENDERER"
        } else if self.hud_page {
            "HUD SIZE / ENHANCED RENDERER"
        } else {
            match self.section {
                Some(0) => "CLASSIC / GBA",
                Some(1) => "ENHANCED / ON FOOT",
                Some(_) => "ENHANCED / VEHICLE",
                None => "PC settings - changes apply when saved",
            }
        };
        self.text(canvas, 38, 74, subtitle, Color::RGB(169, 192, 208), 2)?;
        let rows: Vec<(String, String)> = if self.confirm_quit {
            vec![
                ("Quit game".into(), String::new()),
                ("Back".into(), String::new()),
            ]
        } else if self.language_page {
            let mut rows = vec![(
                "Original game / ROM".into(),
                if self.prefs.language.is_none() {
                    "Selected".into()
                } else {
                    String::new()
                },
            )];
            rows.extend(self.languages.iter().map(|choice| {
                let name = choice
                    .label
                    .rsplit_once(" (")
                    .map_or(choice.label.as_str(), |(name, _)| name);
                let label = if name.chars().count() > 21 {
                    format!("{}...", name.chars().take(18).collect::<String>())
                } else {
                    name.to_owned()
                };
                (
                    label,
                    if self.prefs.language.as_ref() == Some(&choice.file) {
                        format!("Selected: {}", choice.file)
                    } else {
                        choice.file.clone()
                    },
                )
            }));
            rows.push(("Back".into(), String::new()));
            rows
        } else if self.graphics_page {
            vec![
                (
                    "Texture style".into(),
                    format!("{:?}", self.prefs.render.textures),
                ),
                (
                    "Aspect ratio".into(),
                    if self.prefs.render.aspect == crate::viewport::Aspect::Wide {
                        "16:9 (experimental)".into()
                    } else {
                        "Original 3:2".into()
                    },
                ),
                ("Back".into(), String::new()),
            ]
        } else if self.hud_page {
            let hud = self.prefs.render.hud;
            let percent = |s: crate::hud_layout::Size| format!("{}%", s.percent());
            vec![
                (
                    "All elements".into(),
                    hud.all().map_or("Custom".into(), percent),
                ),
                ("Radar and markers".into(), percent(hud.radar)),
                ("Status and weapon".into(), percent(hud.status)),
                ("Messages".into(), percent(hud.messages)),
                ("Reset HUD sizes".into(), String::new()),
            ]
        } else if let Some(section) = self.section {
            let mut rows: Vec<_> = Controls::actions(section)
                .iter()
                .enumerate()
                .map(|(i, a)| (label(a).into(), self.controls.binding_text(section, i)))
                .collect();
            if section == 1 {
                rows.push((
                    "Mouse aim (exp.)".into(),
                    if self.controls.mouse_aim_setting() {
                        "On"
                    } else {
                        "Off"
                    }
                    .into(),
                ));
            }
            rows
        } else {
            vec![
                ("Control mode".into(), format!("{:?}", self.controls.mode())),
                ("Classic bindings".into(), ">".into()),
                ("On-foot bindings".into(), ">".into()),
                ("Vehicle bindings".into(), ">".into()),
                (
                    "World renderer".into(),
                    format!("{:?}", self.prefs.render.renderer),
                ),
                ("Graphics".into(), ">".into()),
                (
                    "Fullscreen".into(),
                    if self.prefs.fullscreen {
                        "On".into()
                    } else {
                        "Off".into()
                    },
                ),
                ("HUD size".into(), ">".into()),
                ("Language".into(), self.language_label()),
                ("Save and return".into(), String::new()),
                ("Discard and return".into(), String::new()),
                ("Quit game...".into(), String::new()),
            ]
        };
        for (i, (label, value)) in rows.iter().enumerate().skip(self.first_row()).take(11) {
            let y = 104 + (i - self.first_row()) as i32 * 25;
            if i == self.row {
                canvas.set_draw_color(Color::RGB(53, 93, 114));
                canvas.fill_rect(Rect::new(30, y, 660, 24))?;
            }
            let color = if i == self.row {
                Color::RGB(255, 225, 128)
            } else {
                Color::RGB(230, 237, 239)
            };
            let font = if self.language_page {
                i.checked_sub(1)
                    .and_then(|index| self.languages.get(index))
                    .map(|c| &c.font)
            } else if !self.graphics_page
                && !self.hud_page
                && !self.confirm_quit
                && self.section.is_none()
                && i == 8
            {
                self.prefs
                    .language
                    .as_ref()
                    .and_then(|file| self.languages.iter().find(|c| &c.file == file))
                    .map(|c| &c.font)
            } else {
                None
            };
            self.text_with_font(
                canvas,
                40,
                y + 5,
                label,
                color,
                2,
                if self.language_page { font } else { None },
            )?;
            self.text_with_font(
                canvas,
                302,
                y + 5,
                &value.chars().take(31).collect::<String>(),
                color,
                2,
                font,
            )?;
        }
        let aim_row = self.section == Some(1) && self.row == Controls::actions(1).len();
        let hint = if aim_row {
            "Enter / left / right: toggle   Esc: back"
        } else if self.language_page {
            "Enter: select language   Esc: back"
        } else if self.capture.is_some() {
            "Press a key / mouse button. Esc: cancel capture"
        } else if self.graphics_page {
            "Left / right: change   Enter: select   Esc: back"
        } else if self.hud_page {
            "Left / right: size   Enter: next   Esc: back"
        } else if self.section.is_some() {
            "Enter: replace  Ins: add  Del: clear  Esc: back"
        } else {
            "Arrows: select  Enter: change  Esc: discard"
        };
        self.text(canvas, 38, 392, hint, Color::RGB(169, 192, 208), 2)?;
        let message = if self.language_page && self.message.is_empty() {
            "Packs: languages/*.json. Save to apply now."
        } else if aim_row {
            "Hold Aim to face cursor. Default: right mouse button."
        } else if self.graphics_page && self.row == 1 {
            "16:9 expands the world. Menus keep original aspect."
        } else if self.hud_page && self.message.is_empty() {
            "Enhanced world view only. Original HUD stays at 100%."
        } else if self.message.is_empty() && self.graphics_page && self.row == 0 {
            "Affine: experimental PS1-style texture distortion."
        } else {
            &self.message
        };
        let chars: Vec<_> = message.chars().collect();
        for (i, line) in chars.chunks(53).take(2).enumerate() {
            self.text(
                canvas,
                38,
                420 + i as i32 * 17,
                &line.iter().collect::<String>(),
                Color::RGB(255, 184, 113),
                2,
            )?;
        }
        canvas.present();
        Ok(())
    }
}
fn label(action: &str) -> &str {
    match action {
        "BrakeReverse" => "Brake / reverse",
        "SteerLeft" => "Steer left",
        "SteerRight" => "Steer right",
        "ParkingBrake" => "Parking brake",
        "SpecialMission" => "Special mission",
        "TurretModifier" => "Turret modifier",
        "ExitVehicle" => "Exit vehicle",
        "EnterVehicle" => "Enter vehicle",
        "ChangeWeapon" => "Change weapon",
        "AuxiliaryR" => "Auxiliary R",
        "Aim" => "Aim (hold)",
        _ => action,
    }
}
// Small original 5x7 font. No ROM assets, SDL_ttf or external font files needed.
fn glyph(c: char) -> [u8; 7] {
    let c = match c {
        '—' | '–' => '-',
        'à' | 'á' | 'ä' | 'À' | 'Á' | 'Ä' => 'A',
        'è' | 'é' | 'È' | 'É' => 'E',
        'ì' | 'í' | 'Ì' | 'Í' => 'I',
        'ò' | 'ó' | 'ö' | 'Ò' | 'Ó' | 'Ö' => 'O',
        'ù' | 'ú' | 'ü' | 'Ù' | 'Ú' | 'Ü' => 'U',
        'ñ' | 'Ñ' => 'N',
        'ç' | 'Ç' => 'C',
        _ => c,
    };
    match c.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        '.' => [0, 0, 0, 0, 0, 6, 6],
        ',' => [0, 0, 0, 0, 6, 4, 8],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '>' => [8, 4, 2, 1, 2, 4, 8],
        '[' => [14, 8, 8, 8, 8, 8, 14],
        ']' => [14, 2, 2, 2, 2, 2, 14],
        '=' => [0, 0, 31, 0, 31, 0, 0],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        '\'' => [4, 4, 8, 0, 0, 0, 0],
        '?' => [14, 17, 1, 2, 4, 0, 4],
        '%' => [25, 26, 2, 4, 8, 11, 19],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        ' ' => [0; 7],
        _ => [31, 17, 1, 2, 4, 0, 4],
    }
}
fn text_with_font<T: sdl2::render::RenderTarget>(
    canvas: &mut Canvas<T>,
    x: i32,
    y: i32,
    value: &str,
    color: Color,
    scale: u32,
    font: Option<&crate::localization::Font>,
) -> Result<(), String> {
    canvas.set_draw_color(color);
    for (i, c) in value.chars().enumerate() {
        for (row, bits) in font
            .and_then(|font| font.ui_glyph(c))
            .unwrap_or_else(|| glyph(c))
            .iter()
            .enumerate()
        {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    canvas.fill_rect(Rect::new(
                        x + (i as i32 * 6 + col) * scale as i32,
                        y + row as i32 * scale as i32,
                        scale,
                        scale,
                    ))?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdl2::keyboard::{Mod, Scancode};
    fn key(k: Keycode) -> Event {
        Event::KeyDown {
            timestamp: 0,
            window_id: 0,
            keycode: Some(k),
            scancode: Scancode::from_name(&k.name()),
            keymod: Mod::NOMOD,
            repeat: false,
        }
    }
    #[test]
    fn drafts_rebind_conflicts_cancel_and_preserve_other_profiles() {
        let controls = Controls::parse(&Controls::default_file()).unwrap();
        let original = controls.encode();
        let mut m = Menu::new(controls, Preferences::default());
        m.row = 2;
        m.event(&key(Keycode::Return)); // on foot
        m.row = 6;
        m.event(&key(Keycode::Return)); // jump
        m.event(&key(Keycode::J)); // conflict
        assert!(m.capture.is_some());
        assert!(m.message.contains("Fire"));
        assert_eq!(m.controls.encode(), original);
        m.event(&key(Keycode::Escape));
        assert!(m.capture.is_none());
        m.event(&key(Keycode::Return));
        m.event(&key(Keycode::C));
        assert_eq!(m.controls.binding_text(1, 6), "C");
        assert_eq!(m.controls.binding_text(2, 4), "Space");
        m.event(&key(Keycode::Insert));
        m.event(&key(Keycode::Space));
        assert_eq!(m.controls.binding_text(1, 6), "C, Space");
        m.event(&key(Keycode::Escape));
        assert!(matches!(m.event(&key(Keycode::Escape)), Command::Close));
    }
    #[test]
    fn hud_preferences_migrate_and_menu_reset_preserves_other_options() {
        let old = Preferences::parse("Renderer = Original\nTextures = Affine\nFullscreen = true")
            .unwrap();
        assert_eq!(old.render.hud, Default::default());
        let mut menu = Menu::new(
            Controls::parse(&Controls::default_file()).unwrap(),
            old.clone(),
        );
        menu.row = 7;
        menu.event(&key(Keycode::Return));
        assert!(menu.hud_page);
        menu.event(&key(Keycode::Left));
        assert_eq!(
            menu.prefs.render.hud.all(),
            Some(crate::hud_layout::Size::Large)
        );
        menu.row = 1;
        menu.event(&key(Keycode::Left));
        assert_eq!(menu.prefs.render.hud.all(), None);
        assert_eq!(menu.prefs.render.hud.radar, crate::hud_layout::Size::Medium);
        menu.row = 4;
        menu.event(&key(Keycode::Return));
        assert_eq!(menu.prefs, old);
        menu.event(&key(Keycode::Escape));
        assert!(!menu.hud_page);
        assert_eq!(menu.row, 7);
        assert!(Preferences::parse("HudRadar = 0").is_err());
        assert!(Preferences::parse("HudStatus = 200").is_err());
    }
    #[test]
    fn persistence_roundtrip_and_backup() {
        let dir = std::env::temp_dir().join(format!("options-save-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let cp = dir.join("controls.cfg");
        let pp = dir.join("enhanced.cfg");
        let original = Controls::default_file();
        fs::write(&cp, &original).unwrap();
        let mut controls = Controls::parse(&original).unwrap();
        controls
            .rebind(1, 6, Some(Binding::Key(Scancode::C)), false)
            .unwrap();
        let prefs = Preferences {
            render: Settings {
                renderer: Renderer::Original,
                textures: TextureMapping::Affine,
                aspect: crate::viewport::Aspect::Wide,
                hud: crate::hud_layout::Settings {
                    radar: crate::hud_layout::Size::Half,
                    ..Default::default()
                },
            },
            fullscreen: true,
            language: Some("ru-demo.json".into()),
        };
        // A failure staging presentation options must not modify bindings.
        assert!(save(&cp, &dir.join("missing/enhanced.cfg"), &controls, &prefs).is_err());
        assert_eq!(fs::read_to_string(&cp).unwrap(), original);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        save(&cp, &pp, &controls, &prefs).unwrap();
        assert_eq!(Preferences::load(&pp).unwrap(), prefs);
        assert_eq!(Controls::load(&cp).unwrap().encode(), controls.encode());
        assert_eq!(
            fs::read_to_string(cp.with_extension("cfg.options-backup")).unwrap(),
            original
        );
        save(&cp, &pp, &controls, &Preferences::default()).unwrap(); // replacing existing files on Windows
        assert_eq!(Preferences::load(&pp).unwrap(), Preferences::default());
        assert!(Preferences::parse("Renderer = Broken").is_err());
        assert!(Preferences::parse("Fullscreen = true\nFullscreen = false").is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn language_selection_is_a_draft_and_old_preferences_still_load() {
        assert_eq!(
            Preferences::parse("Fullscreen = false").unwrap().language,
            None
        );
        for bad in [
            "../ru.json",
            "C:\\ru.json",
            "ru.json#x",
            "ru.json/other.json",
        ] {
            assert!(!crate::localization::valid_file_name(bad));
        }
        assert!(Preferences::parse("Language = ../ru.json").is_err());
        let mut menu = Menu::new(
            Controls::parse(crate::controls::DEFAULT).unwrap(),
            Preferences::default(),
        )
        .with_languages(vec![crate::localization::Choice {
            file: "ru-demo.json".into(),
            label: "Русский (ru)".into(),
            font: crate::localization::test_font(),
        }]);
        menu.row = 8;
        menu.event(&key(Keycode::Return));
        assert!(menu.language_page);
        menu.event(&key(Keycode::Down));
        menu.event(&key(Keycode::Return));
        assert_eq!(menu.prefs.language.as_deref(), Some("ru-demo.json"));
        assert!(menu.message.contains("apply"));
        menu.event(&key(Keycode::Escape));
        assert_eq!(menu.row, 8);
        menu.event(&key(Keycode::Down));
        assert!(matches!(menu.event(&key(Keycode::Return)), Command::Save));
        menu.row = 11;
        assert_eq!(menu.first_row(), 1);
        assert!(menu.languages[0].font.ui_glyph('Р').is_some());
        assert_eq!(
            menu.languages[0].font.ui_glyph('р'),
            menu.languages[0].font.ui_glyph('Р')
        );
    }
    #[test]
    fn classic_cannot_be_unbound_and_reserved_keys_stay_reserved() {
        let mut c = Controls::parse(&Controls::default_file()).unwrap();
        let original = c.encode();
        assert!(c.rebind(0, 0, None, false).is_err());
        assert!(c
            .rebind(1, 6, Some(Binding::Key(Scancode::Escape)), false)
            .is_err());
        assert_eq!(c.encode(), original);
        c.rebind(1, 6, None, false).unwrap();
        assert_eq!(c.binding_text(1, 6), "None");
    }
}
