use sdl2::{
    event::{Event, WindowEvent},
    keyboard::Keycode,
    pixels::PixelFormatEnum,
    render::{Canvas, Texture, TextureCreator},
    video::{Window, WindowContext},
};
use std::time::{Duration, Instant};

// GBA: 280896 cycles per frame at 16777216 Hz, approximately 59.7275 Hz.
const FRAME_TIME: Duration = Duration::from_nanos(16_742_706);

// The creator lives in main, outside Desktop, so cached textures have a safe
// Rust lifetime without self references or raw SDL pointers.
pub struct Textures<'a> {
    creator: &'a TextureCreator<WindowContext>,
    original: Option<Texture<'a>>,
    preview: Option<Texture<'a>>,
    wide: Option<Texture<'a>>,
    menu: Option<Texture<'a>>,
    allocations: u64,
}
impl<'a> Textures<'a> {
    pub fn new(creator: &'a TextureCreator<WindowContext>) -> Self {
        Self {
            creator,
            original: None,
            preview: None,
            wide: None,
            menu: None,
            allocations: 0,
        }
    }
    fn menu(&mut self) -> Result<&mut Texture<'a>, String> {
        if self.menu.is_none() {
            self.menu = Some(
                self.creator
                    .create_texture_streaming(PixelFormatEnum::RGB24, 720, 480)
                    .map_err(|e| e.to_string())?,
            );
            self.allocations += 1;
        }
        Ok(self.menu.as_mut().unwrap())
    }
    fn sized(&mut self, width: u32, height: u32) -> Result<&mut Texture<'a>, String> {
        let slot = if (width, height) == (854, 480) {
            &mut self.wide
        } else if width == 720 {
            &mut self.preview
        } else {
            &mut self.original
        };
        if slot.is_none() {
            *slot = Some(
                self.creator
                    .create_texture_streaming(PixelFormatEnum::RGB24, width, height)
                    .map_err(|e| e.to_string())?,
            );
            self.allocations += 1;
        }
        Ok(slot.as_mut().unwrap())
    }
}

pub struct Desktop {
    camera_cycles: u32,
    cheats: crate::cheats::Panel,
    aim_pointer: Option<sdl2::mouse::Cursor>,
    arrow_pointer: Option<sdl2::mouse::Cursor>,
    pointer_is_aim: bool,
    canvas: Canvas<Window>,
    events: sdl2::EventPump,
    audio: Option<crate::menu_audio::Output>,
    menu_click: Option<std::sync::Arc<[i16]>>,
    sfx_volume: u8,
    _sdl: sdl2::Sdl,
    pixels: Vec<u8>,
    frame_size: (u32, u32),
    aim_projection: Option<crate::mouse_aim::Projection>,
    aim_cursor: Option<[f64; 2]>,
    last_aim: Option<(u32, u16)>,
    paused: bool,
    focused: bool,
    menu: Option<crate::options::Menu>,
    menu_skin: Option<std::sync::Arc<crate::game_menu::Skin>>,
    game_menu: crate::game_menu::Frontend,
    save_exit_prior_pause: Option<bool>,
    release_input: bool,
    preferences_path: std::path::PathBuf,
    language: Option<String>,
    language_change: Option<Option<String>>,
    language_scan: bool,
    languages: Vec<crate::localization::Choice>,
    next_frame: Instant,
    controls: crate::controls::Controls,
    input_latch: crate::controls::InputLatch,
    controls_path: std::path::PathBuf,
    gamepad: Option<crate::gamepad::Gamepad>,
    title_updated: Instant,
    title_initialized: bool,
    notice: &'static str,
    preview_status: &'static str,
    render_settings: crate::render_settings::Settings,
    renderer_available: bool,
    present_frames: u64,
    present_ns: u128,
    fps_started: Instant,
    fps_frames: u64,
    fps: Option<f64>,
}

fn rgb(pixel: u16) -> [u8; 3] {
    let expand = |v: u16| ((v << 3) | (v >> 2)) as u8;
    [
        expand(pixel & 31),
        expand((pixel >> 5) & 31),
        expand((pixel >> 10) & 31),
    ]
}

fn crosshair_bits() -> ([u8; 128], [u8; 128]) {
    let (mut data, mut mask) = ([0u8; 128], [0u8; 128]);
    for y in 0i32..32 {
        for x in 0i32..32 {
            let (dx, dy) = ((x - 15).abs(), (y - 15).abs());
            let outline = dx <= 2 && (4..=12).contains(&dy) || dy <= 2 && (4..=12).contains(&dx);
            let white = dx == 0 && (5..=11).contains(&dy) || dy == 0 && (5..=11).contains(&dx);
            let i = (y * 4 + x / 8) as usize;
            let bit = 0x80 >> (x % 8);
            if outline {
                mask[i] |= bit;
                if !white {
                    data[i] |= bit;
                }
            }
        }
    }
    (data, mask)
}
impl Desktop {
    fn update_aim_pointer(&mut self, active: bool) {
        if active != self.pointer_is_aim {
            if let Some(cursor) = if active {
                &self.aim_pointer
            } else {
                &self.arrow_pointer
            } {
                cursor.set();
            }
            self.pointer_is_aim = active;
        }
    }
    pub fn new(controls_path: std::path::PathBuf) -> Result<Self, String> {
        let preferences_path = controls_path.with_file_name("enhanced.cfg");
        let preferences =
            crate::options::Preferences::load(&preferences_path).unwrap_or_else(|e| {
                eprintln!("Enhanced Options: {e}; using presentation defaults");
                Default::default()
            });
        let sdl = sdl2::init()?;
        let controls = crate::controls::Controls::open(&controls_path)?;
        eprintln!(
            "Controls: {:?} ({})",
            controls.mode(),
            controls_path.display()
        );
        let gamepad = match crate::gamepad::Gamepad::new(&sdl) {
            Ok(pad) => Some(pad),
            Err(e) => {
                eprintln!("Gamepad support unavailable: {e}");
                None
            }
        };
        sdl2::hint::set("SDL_RENDER_SCALE_QUALITY", "0");
        let window = sdl
            .video()?
            .window(
                "AdvanceRecomp | Native dispatch: -- | interpreted: 0 | FPS: --",
                960,
                if preferences.render.aspect == crate::viewport::Aspect::Wide {
                    540
                } else {
                    640
                },
            )
            .position_centered()
            .resizable()
            .build()
            .map_err(|e| e.to_string())?;
        let mut canvas = window.into_canvas().build().map_err(|e| e.to_string())?;
        canvas
            .set_logical_size(240, 160)
            .map_err(|e| e.to_string())?;
        if preferences.fullscreen {
            canvas
                .window_mut()
                .set_fullscreen(sdl2::video::FullscreenType::Desktop)?;
        }
        let events = sdl.event_pump()?;
        let (data, mask) = crosshair_bits();
        let aim_pointer = sdl2::mouse::Cursor::new(&data, &mask, 32, 32, 15, 15).ok();
        let arrow_pointer = sdl2::mouse::Cursor::from_system(sdl2::mouse::SystemCursor::Arrow).ok();

        let audio = match sdl
            .audio()
            .and_then(|subsystem| crate::menu_audio::Output::open(&subsystem))
        {
            Ok(audio) => Some(audio),
            Err(e) => {
                eprintln!("Audio unavailable: {e}");
                None
            }
        };
        Ok(Self {
            camera_cycles: 0,
            cheats: Default::default(),
            aim_pointer,
            arrow_pointer,
            pointer_is_aim: false,
            title_updated: Instant::now(),
            title_initialized: false,
            notice: "",
            preview_status: "",
            render_settings: preferences.render,
            language: preferences.language,
            language_change: None,
            language_scan: false,
            languages: Vec::new(),
            renderer_available: false,
            gamepad,
            controls,
            input_latch: Default::default(),
            controls_path,
            canvas,
            events,
            audio,
            menu_click: None,
            sfx_volume: 3,
            _sdl: sdl,
            pixels: vec![0; 240 * 160 * 3],
            frame_size: (240, 160),
            aim_projection: None,
            aim_cursor: None,
            last_aim: None,
            paused: false,
            focused: true,
            menu: None,
            menu_skin: None,
            game_menu: Default::default(),
            save_exit_prior_pause: None,
            release_input: false,
            preferences_path,
            next_frame: Instant::now(),
            present_frames: 0,
            present_ns: 0,
            fps_started: Instant::now(),
            fps_frames: 0,
            fps: None,
        })
    }

    pub fn paused(&self) -> bool {
        self.menu.is_some()
            || (!self.game_menu.save_exit_active() && (self.paused || !self.focused))
    }
    pub fn save_exit_needs_commit(&self) -> bool {
        self.game_menu.save_exit_needs_commit()
    }
    pub fn saving_game_for_exit(&self) -> bool {
        self.game_menu.save_exit_active()
    }
    pub fn finish_save_exit(&mut self, result: Result<(), String>) {
        self.game_menu.finish_save_exit(result);
    }
    pub fn set_menu_skin(
        &mut self,
        rom: &[u8],
        language: Option<&crate::localization::Language>,
    ) -> Result<(), String> {
        if self.menu_click.is_none() {
            match crate::menu_audio::load_click(rom) {
                Ok(click) => self.menu_click = Some(click),
                Err(e) => eprintln!("Menu sound unavailable: {e}"),
            }
        }
        let region = self.menu_skin.as_ref().map_or(0, |s| s.region());
        let mut skin = crate::game_menu::Skin::load(rom)?;
        std::sync::Arc::get_mut(&mut skin)
            .unwrap()
            .apply_language(language)?;
        std::sync::Arc::get_mut(&mut skin)
            .unwrap()
            .set_region(region);
        self.menu_skin = Some(skin);
        if let Some(menu) = self.menu.as_mut() {
            menu.skin = self.menu_skin.clone();
            menu.invalidate();
        }
        Ok(())
    }
    pub(crate) fn play_menu_click(&mut self) {
        if let (Some(audio), Some(click)) = (&mut self.audio, &self.menu_click) {
            audio.click(click.clone(), self.sfx_volume);
        }
    }
    pub fn sync_menu_region(&mut self, bus: &gba_core::MemMap) {
        if bus.ewram.get(0x3e4) == Some(&1) {
            let volume = bus.ewram[0x10c61].min(3);
            if self.sfx_volume != volume {
                self.sfx_volume = volume;
                if let Some(audio) = &mut self.audio {
                    audio.set_volume(volume);
                }
            }
        }
        let Some(region) = crate::game_menu::current_region(bus) else {
            return;
        };
        let Some(skin) = self.menu_skin.as_mut() else {
            return;
        };
        if skin.region() == region {
            return;
        }
        std::sync::Arc::make_mut(skin).set_region(region);
        if let Some(menu) = self.menu.as_mut() {
            menu.skin = self.menu_skin.clone();
            menu.invalidate();
        }
    }
    pub fn observe_game_menu(&mut self, machine: &gba_core::Machine, key: u32) {
        self.sync_menu_region(&machine.bus);
        self.game_menu.observe(machine, key);
    }
    pub fn menu_pixels(&self) -> Result<Option<Vec<u8>>, String> {
        let Some(skin) = &self.menu_skin else {
            return Ok(None);
        };
        if self.game_menu.view.is_none() {
            return Ok(None);
        }
        let mut canvas =
            sdl2::surface::Surface::new(720, 480, PixelFormatEnum::RGB24)?.into_canvas()?;
        self.game_menu.draw(&mut canvas, skin)?;
        let surface = canvas.into_surface();
        Ok(Some(
            surface
                .without_lock()
                .ok_or("Menu surface is locked")?
                .to_vec(),
        ))
    }
    fn open_options(&mut self, settings: crate::render_settings::Settings, quit: bool) {
        self.game_menu.cancel_navigation();
        let mut menu = crate::options::Menu::new(
            self.controls.clone(),
            crate::options::Preferences {
                render: settings,
                fullscreen: self.canvas.window().fullscreen_state()
                    != sdl2::video::FullscreenType::Off,
                language: self.language.clone(),
            },
        )
        .with_languages(self.languages.clone());
        menu.skin = self.menu_skin.clone();
        menu.can_save_game = self.menu_skin.is_some() && self.game_menu.can_save_exit();
        if quit {
            menu.request_quit();
        }
        self.menu = Some(menu);
        self.play_menu_click();
        self.language_scan = true;
        self.title_initialized = false;
    }
    pub fn set_languages(&mut self, languages: Vec<crate::localization::Choice>) {
        if self
            .language
            .as_ref()
            .is_some_and(|file| !languages.iter().any(|choice| &choice.file == file))
        {
            self.language = None;
        }
        if let Some(menu) = self.menu.as_mut() {
            menu.refresh_languages(languages.clone());
        }
        self.languages = languages;
    }
    pub fn take_language_scan(&mut self) -> bool {
        std::mem::take(&mut self.language_scan)
    }
    pub fn take_language_change(&mut self) -> Option<Option<String>> {
        self.language_change.take()
    }
    pub fn language_error(&mut self, previous: Option<String>, error: &str) {
        self.language = previous;
        self.notice = "Language could not be loaded; see console";
        eprintln!("Language unchanged: {error}");
    }
    pub fn update_status(
        &mut self,
        coverage: crate::coverage::Coverage,
        interpreter: bool,
    ) -> Result<(), String> {
        if self.title_initialized && self.title_updated.elapsed() < Duration::from_millis(500) {
            return Ok(());
        }
        let mut title = coverage.title(interpreter, self.paused(), "");
        title.push_str(&match self.fps {
            Some(fps) if !self.paused() => format!(" | FPS: {fps:.1}"),
            _ => " | FPS: --".to_owned(),
        });
        if !self.preview_status.is_empty() {
            title.push_str(" | ");
            title.push_str(self.preview_status);
        }
        self.canvas
            .window_mut()
            .set_title(&title)
            .map_err(|e| e.to_string())?;
        self.title_updated = Instant::now();
        self.title_initialized = true;
        Ok(())
    }

    pub fn title(&self) -> &str {
        self.canvas.window().title()
    }

    pub fn final_status(
        &mut self,
        coverage: crate::coverage::Coverage,
        interpreter: bool,
    ) -> Result<(), String> {
        self.title_initialized = false;
        self.update_status(coverage, interpreter)
    }

    pub fn poll(
        &mut self,
        settings: &mut crate::render_settings::Settings,
        renderer_available: bool,
        context: crate::control_context::Context,
    ) -> Result<Option<u16>, String> {
        if self.game_menu.save_exit_complete() {
            return Ok(None);
        }
        if let Some(error) = self.game_menu.take_save_exit_error() {
            if let Some(paused) = self.save_exit_prior_pause.take() {
                self.paused = paused;
            }
            self.open_options(*settings, true);
            self.menu.as_mut().unwrap().message = error;
        }
        self.renderer_available = renderer_available;
        let events: Vec<_> = self.events.poll_iter().collect();
        for event in events {
            if matches!(event, Event::Quit { .. }) {
                if self.game_menu.save_exit_active() {
                    continue;
                }
                return Ok(None);
            }
            if let Event::Window { win_event, .. } = event {
                match win_event {
                    WindowEvent::FocusLost => self.focused = false,
                    WindowEvent::FocusGained => self.focused = true,
                    _ => {}
                }
            }
            if self.game_menu.save_exit_active() {
                continue;
            }
            if self.menu.is_some() {
                if !self.focused {
                    continue;
                }
                let command = self.menu.as_mut().unwrap().event(&event);
                if self.menu.as_mut().unwrap().take_click() {
                    self.play_menu_click();
                }
                match command {
                    crate::options::Command::None => {}
                    crate::options::Command::Quit => return Ok(None),
                    crate::options::Command::SaveGameAndQuit => {
                        if self.game_menu.begin_save_exit() {
                            self.save_exit_prior_pause = Some(self.paused);
                            self.paused = false;
                            self.menu = None;
                            self.release_input = false;
                        } else {
                            let menu = self.menu.as_mut().unwrap();
                            menu.message = "Save is unavailable on this screen.".into();
                            menu.invalidate();
                        }
                    }
                    crate::options::Command::Close => {
                        self.menu = None;
                        self.release_input = true;
                    }
                    crate::options::Command::Save => {
                        if let Err(e) = self.save_options(settings) {
                            eprintln!("Cannot save Enhanced Options: {e}");
                            let menu = self.menu.as_mut().unwrap();
                            menu.message = format!("Save failed: {e}");
                            menu.invalidate();
                        } else {
                            self.menu = None;
                            self.release_input = true;
                        }
                    }
                }
                continue;
            }
            if self.release_input {
                continue;
            }
            if self.focused && !self.paused() && self.cheats.event(&event) {
                self.game_menu.cancel_navigation();
                if !self.cheats.visible {
                    self.release_input = true;
                }
                self.input_latch = Default::default();
                continue;
            }
            if self.focused
                && !self.paused()
                && context != crate::control_context::Context::Menu
                && self.game_menu.view.is_none()
                && !self.game_menu.modal
                && !self.game_menu.busy()
                && !self.cheats.captures_input()
            {
                let key = match event {
                    Event::KeyDown {
                        scancode: Some(code),
                        repeat: false,
                        ..
                    } => Some(crate::controls::Binding::Key(code)),
                    Event::MouseButtonDown { mouse_btn, .. } => match mouse_btn {
                        sdl2::mouse::MouseButton::Left => Some(crate::controls::Binding::MouseLeft),
                        sdl2::mouse::MouseButton::Middle => {
                            Some(crate::controls::Binding::MouseMiddle)
                        }
                        sdl2::mouse::MouseButton::Right => {
                            Some(crate::controls::Binding::MouseRight)
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if key.is_some_and(|key| self.controls.cycles_camera(key)) {
                    self.camera_cycles = self.camera_cycles.saturating_add(1);
                    continue;
                }
            }
            if self.menu_skin.is_some()
                && self.game_menu.view.is_some()
                && !matches!(
                    event,
                    Event::KeyDown {
                        keycode: Some(
                            Keycode::P | Keycode::F5 | Keycode::F6 | Keycode::F8 | Keycode::F11
                        ),
                        ..
                    } | Event::ControllerDeviceAdded { .. }
                        | Event::ControllerDeviceRemoved { .. }
                )
            {
                if self.focused {
                    let action = self.game_menu.event(&event);
                    if self.game_menu.take_click() {
                        self.play_menu_click();
                    }
                    match action {
                        crate::game_menu::Action::Options => self.open_options(*settings, false),
                        crate::game_menu::Action::Quit => self.open_options(*settings, true),
                        crate::game_menu::Action::None => {}
                    }
                }
                continue;
            }
            if self.menu_skin.is_some()
                && self.game_menu.modal
                && matches!(
                    event,
                    Event::KeyDown {
                        keycode: Some(Keycode::Escape | Keycode::Return | Keycode::KpEnter),
                        repeat: false,
                        ..
                    } | Event::ControllerButtonDown {
                        button: sdl2::controller::Button::B
                            | sdl2::controller::Button::Start
                            | sdl2::controller::Button::Guide,
                        ..
                    }
                )
            {
                self.game_menu.press(1);
                continue;
            }
            if self.menu_skin.is_some() && context == crate::control_context::Context::Menu {
                if let Event::KeyDown {
                    keycode: Some(key),
                    repeat: false,
                    ..
                } = event
                {
                    let mask = match key {
                        Keycode::Return | Keycode::KpEnter => Some(1),
                        Keycode::Escape | Keycode::Backspace => Some(2),
                        _ => None,
                    };
                    if let Some(mask) = mask {
                        self.game_menu.press(mask);
                        continue;
                    }
                }
            }
            if matches!(
                event,
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    repeat: false,
                    ..
                } | Event::ControllerButtonDown {
                    button: sdl2::controller::Button::Guide,
                    ..
                }
            ) {
                if self.menu_skin.is_some()
                    && !self.paused
                    && context != crate::control_context::Context::Menu
                {
                    self.game_menu.press(8);
                } else {
                    self.open_options(*settings, false);
                }
                continue;
            }
            match event {
                Event::KeyDown {
                    keycode: Some(Keycode::F6),
                    repeat: false,
                    ..
                } => {
                    if renderer_available {
                        settings.toggle_renderer();
                        self.title_initialized = false;
                    }
                }
                Event::Quit { .. } => return Ok(None),
                Event::KeyDown {
                    keycode: Some(Keycode::F8),
                    repeat: false,
                    ..
                } => match self.controls.toggle_mode() {
                    Ok(()) => eprintln!("Controls: {:?}", self.controls.mode()),
                    Err(error) => eprintln!("Controls unchanged: {error}"),
                },
                Event::KeyDown {
                    keycode: Some(Keycode::P),
                    repeat: false,
                    ..
                } => self.paused = !self.paused,
                Event::KeyDown {
                    keycode: Some(Keycode::F5),
                    repeat: false,
                    ..
                } => match crate::controls::Controls::load(&self.controls_path) {
                    Ok(controls) => {
                        self.controls = controls;
                        eprintln!("Controls reloaded: {}", self.controls_path.display());
                        self.notice = "Controls reloaded";
                    }
                    Err(e) => {
                        eprintln!("Controls unchanged: {e}");
                        self.notice = "Invalid controls file - previous controls retained";
                    }
                },
                Event::KeyDown {
                    keycode: Some(Keycode::F11),
                    repeat: false,
                    ..
                } => {
                    let next = if self.canvas.window().fullscreen_state()
                        == sdl2::video::FullscreenType::Off
                    {
                        sdl2::video::FullscreenType::Desktop
                    } else {
                        sdl2::video::FullscreenType::Off
                    };
                    self.canvas.window_mut().set_fullscreen(next)?;
                }
                Event::Window {
                    win_event: WindowEvent::FocusLost,
                    ..
                } => self.focused = false,
                Event::Window {
                    win_event: WindowEvent::FocusGained,
                    ..
                } => self.focused = true,
                Event::ControllerDeviceAdded { .. } | Event::ControllerDeviceRemoved { .. } => {
                    if let Some(pad) = self.gamepad.as_mut() {
                        pad.refresh();
                    }
                }
                _ => {}
            }
        }
        self.render_settings = *settings;
        let keyboard = self.events.keyboard_state();
        let mouse = self.events.mouse_state();
        self.aim_cursor = None;
        let releasing_input = self.release_input;
        if releasing_input {
            // Consume held menu/capture buttons in both profiles and on gamepads.
            // Check releases even during P-pause, or its resume key stays blocked.
            self.release_input = keyboard.pressed_scancodes().next().is_some()
                || mouse.pressed_mouse_buttons().next().is_some()
                || self
                    .gamepad
                    .as_ref()
                    .is_some_and(|pad| pad.buttons() != 0x3ff);
            self.input_latch = Default::default();
        }
        if self.paused() {
            self.update_aim_pointer(false);
            self.fps_started = Instant::now();
            self.fps_frames = 0;
            self.fps = None;
            if let Some(audio) = &mut self.audio {
                audio.clear();
            }
            self.next_frame = Instant::now();
            std::thread::sleep(Duration::from_millis(10));
            return Ok(Some(0x3ff));
        }
        if self.cheats.captures_input() {
            self.update_aim_pointer(false);
            let visible = self.cheats.visible;
            let keys = self.cheats.input();
            if visible && !self.cheats.visible {
                self.release_input = true;
            }
            return Ok(Some(keys));
        }
        if self.menu_skin.is_some()
            && (self.game_menu.view.is_some()
                || self.game_menu.busy()
                || self.game_menu.save_exit_active())
        {
            self.update_aim_pointer(false);
            let busy = self.game_menu.busy();
            let keys = self.game_menu.input();
            if busy && !self.game_menu.busy() {
                self.release_input = true;
            }
            return Ok(Some(keys));
        }
        if releasing_input {
            self.update_aim_pointer(false);
            return Ok(Some(0x3ff));
        }
        let mut keyboard_buttons =
            self.input_latch
                .sample(&self.controls, context, |binding| match binding {
                    crate::controls::Binding::Key(key) => keyboard.is_scancode_pressed(key),
                    crate::controls::Binding::MouseLeft => mouse.left(),
                    crate::controls::Binding::MouseMiddle => mouse.middle(),
                    crate::controls::Binding::MouseRight => mouse.right(),
                });
        if self.menu_skin.is_some() && context == crate::control_context::Context::Menu {
            keyboard_buttons |= 8; // Enter confirms fallback pages; never also send Start.
        }
        if !self.paused()
            && !releasing_input
            && self.input_latch.aiming(&self.controls, context)
            && context == crate::control_context::Context::OnFoot
            && self
                .gamepad
                .as_ref()
                .is_none_or(|pad| pad.buttons() == 0x3ff)
            && self.canvas.window().window_flags()
                & sdl2::sys::SDL_WindowFlags::SDL_WINDOW_MOUSE_FOCUS as u32
                != 0
        {
            let (mut x, mut y) = (0.0f32, 0.0f32);
            // GetMouseState is in window units; SDL accounts for viewport,
            // letterboxing and the renderer's HiDPI output scale here.
            unsafe {
                sdl2::sys::SDL_RenderWindowToLogical(
                    self.canvas.raw(),
                    mouse.x(),
                    mouse.y(),
                    &mut x,
                    &mut y,
                );
            }
            self.aim_cursor = Some([x as f64, y as f64]);
        } else {
            self.last_aim = None;
        }
        self.update_aim_pointer(self.aim_cursor.is_some_and(|p| {
            p[0] >= 0.0
                && p[1] >= 0.0
                && p[0] < self.frame_size.0 as f64
                && p[1] < self.frame_size.1 as f64
        }));
        let mut buttons = crate::gamepad::merge(
            keyboard_buttons,
            self.gamepad.as_ref().map_or(0x3ff, |pad| pad.buttons()),
        );
        if self.menu_skin.is_some() && self.game_menu.modal {
            buttons |= 8;
        }
        Ok(Some(buttons))
    }
    pub fn take_camera_cycles(&mut self) -> u32 {
        std::mem::take(&mut self.camera_cycles)
    }

    pub fn initial_render_settings(&self) -> crate::render_settings::Settings {
        self.render_settings
    }
    pub fn set_aim_projection(&mut self, projection: Option<crate::mouse_aim::Projection>) {
        self.aim_projection = projection;
    }
    pub fn mouse_aim_enabled(&self) -> bool {
        self.controls.mouse_aim()
            && !self.paused()
            && !self.release_input
            && !self.cheats.captures_input()
    }
    pub fn sync_cheats(&mut self, bus: &gba_core::MemMap) {
        self.cheats.sync(bus);
    }
    pub fn observe_cheat_menu(&mut self, frame: u64) {
        self.cheats.observe(frame);
    }
    pub fn aim_angle(&mut self, bus: &gba_core::MemMap) -> Option<u16> {
        if self.paused() || self.release_input || !self.controls.mouse_aim() {
            self.last_aim = None;
            return None;
        }
        let (actor, _, world) = crate::mouse_aim::player(bus)?;
        let p = self.aim_projection?;
        let cursor = self.aim_cursor?;
        if actor != p.actor || (0..2).any(|i| cursor[i] < 0.0 || cursor[i] >= p.size[i] as f64) {
            self.last_aim = None;
            return None;
        }
        let angle = p.angle(actor, world, cursor).or_else(|| {
            self.last_aim
                .filter(|&(a, _)| a == actor)
                .map(|(_, angle)| angle)
        });
        self.last_aim = angle.map(|angle| (actor, angle));
        angle
    }

    fn save_options(
        &mut self,
        settings: &mut crate::render_settings::Settings,
    ) -> Result<(), String> {
        let menu = self.menu.as_ref().unwrap();
        let old = self.canvas.window().fullscreen_state();
        let next = if menu.prefs.fullscreen {
            sdl2::video::FullscreenType::Desktop
        } else {
            sdl2::video::FullscreenType::Off
        };
        self.canvas.window_mut().set_fullscreen(next)?;
        if let Err(e) = crate::options::save(
            &self.controls_path,
            &self.preferences_path,
            &menu.controls,
            &menu.prefs,
        ) {
            let _ = self.canvas.window_mut().set_fullscreen(old);
            return Err(e);
        }
        if settings.aspect != menu.prefs.render.aspect && !menu.prefs.fullscreen {
            let (width, _) = self.canvas.window().size();
            let view = crate::viewport::View::new(3, menu.prefs.render.aspect);
            if let Err(e) = self
                .canvas
                .window_mut()
                .set_size(width, (width as usize * view.height / view.width) as u32)
            {
                eprintln!("Window resize: {e}");
            }
        }
        self.controls = menu.controls.clone();
        if self.language != menu.prefs.language || menu.language_requested {
            self.language_change = Some(menu.prefs.language.clone());
        }
        self.language = menu.prefs.language.clone();
        *settings = menu.prefs.render;
        self.render_settings = *settings;
        self.title_initialized = false;
        eprintln!(
            "Enhanced Options saved: {}",
            self.preferences_path.display()
        );
        Ok(())
    }
    pub fn present_paused(&mut self, textures: &mut Textures<'_>) -> Result<(), String> {
        if let Some(menu) = &mut self.menu {
            menu.draw(&mut self.canvas, textures.menu()?)?;
        } else {
            let (width, height) = self.frame_size;
            self.canvas
                .set_logical_size(width, height)
                .map_err(|e| e.to_string())?;
            self.canvas.set_draw_color(sdl2::pixels::Color::BLACK);
            self.canvas.clear();
            self.canvas
                .copy(textures.sized(width, height)?, None, None)?;
            self.canvas.present();
        }
        Ok(())
    }

    pub fn set_preview_status(&mut self, status: &'static str) {
        self.preview_status = status;
    }
    pub fn set_render_settings(
        &mut self,
        settings: crate::render_settings::Settings,
        available: bool,
    ) {
        self.render_settings = settings;
        self.renderer_available = available;
    }
    pub fn texture_creator(&self) -> TextureCreator<WindowContext> {
        self.canvas.texture_creator()
    }
    pub fn finish(&self, textures: &Textures<'_>) {
        eprintln!(
            "desktop_present frames={} cpu_ms={:.3} texture_allocations={}",
            self.present_frames,
            self.present_ns as f64 / 1e6,
            textures.allocations
        );
    }
    pub fn present(
        &mut self,
        source: &gba_core::MemMap,
        preview: Option<&[u16]>,
        textures: &mut Textures<'_>,
    ) -> Result<(), String> {
        let start = Instant::now();
        let custom_menu = self.game_menu.view.is_some() && self.menu_skin.is_some();
        if custom_menu {
            if let Some(skin) = &self.menu_skin {
                let mut surface =
                    sdl2::surface::Surface::new(720, 480, PixelFormatEnum::RGB24)?.into_canvas()?;
                self.game_menu.draw(&mut surface, skin)?;
                let surface = surface.into_surface();
                let texture = textures.menu()?;
                texture
                    .update(
                        None,
                        surface.without_lock().ok_or("Menu surface is locked")?,
                        surface.pitch() as usize,
                    )
                    .map_err(|e| e.to_string())?;
                self.canvas
                    .set_logical_size(720, 480)
                    .map_err(|e| e.to_string())?;
                self.canvas.set_draw_color(sdl2::pixels::Color::BLACK);
                self.canvas.clear();
                self.canvas.copy(texture, None, None)?;
                self.frame_size = (720, 480);
                // Keep the normal frame pacing/audio bookkeeping below.
            }
        }
        if !custom_menu {
            let frame = preview.unwrap_or(&source.framebuffer);
            let (width, height) = if preview.is_some() {
                let width = frame.len() / 480;
                if ![720, 854].contains(&width) || frame.len() != width * 480 {
                    return Err("Invalid Enhanced frame size".into());
                }
                (width as u32, 480u32)
            } else {
                (240, 160)
            };
            self.frame_size = (width, height);
            if self.canvas.logical_size() != (width, height) {
                self.canvas
                    .set_logical_size(width, height)
                    .map_err(|e| e.to_string())?;
            }
            self.pixels.resize(width as usize * height as usize * 3, 0);
            for (out, &pixel) in self.pixels.chunks_exact_mut(3).zip(frame) {
                out.copy_from_slice(&rgb(pixel));
            }
            let pixels = self.pixels.as_slice();
            let texture = textures.sized(width, height)?;
            texture
                .update(None, pixels, width as usize * 3)
                .map_err(|e| e.to_string())?;
            self.canvas.set_draw_color(sdl2::pixels::Color::BLACK);
            self.canvas.clear();
            self.canvas.copy(texture, None, None)?;
        }
        self.cheats
            .draw(&mut self.canvas, self.menu_skin.as_deref())?;
        self.canvas.present();
        self.present_frames += 1;
        self.present_ns += start.elapsed().as_nanos();
        {
            if let Some(audio) = &mut self.audio {
                // Bound latency after stalls; never allow an unlimited audio backlog.
                if audio.size() > 65536 / 5 * 4 {
                    audio.clear();
                }
                if self.menu_skin.is_some() && self.game_menu.mute_navigation_audio() {
                    audio.clear();
                } else {
                    audio.queue_audio(&source.audio_buf)?;
                }
            }
            self.next_frame += FRAME_TIME;
            let now = Instant::now();
            if self.next_frame > now {
                std::thread::sleep(self.next_frame - now);
            } else if now.duration_since(self.next_frame) > FRAME_TIME * 4 {
                self.next_frame = now;
            }
        }
        self.fps_frames += 1;
        let elapsed = self.fps_started.elapsed();
        if elapsed >= Duration::from_millis(500) {
            self.fps = Some(self.fps_frames as f64 / elapsed.as_secs_f64());
            self.fps_frames = 0;
            self.fps_started = Instant::now();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires isolated SDL dummy driver"]
    fn camera_shortcut_real_sdl() {
        use super::*;
        use crate::control_context::Context;
        use crate::controls::Binding;
        use sdl2::keyboard::{Mod, Scancode};
        let root = std::env::temp_dir().join(format!("camera-shortcut-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut ui = Desktop::new(root.join("controls.cfg")).unwrap();
        let events = ui._sdl.event().unwrap();
        let mut settings = Default::default();
        let push = |code, repeat| {
            events
                .push_event(Event::KeyDown {
                    timestamp: 0,
                    window_id: 0,
                    keycode: Keycode::from_scancode(code),
                    scancode: Some(code),
                    keymod: Mod::NOMOD,
                    repeat,
                })
                .unwrap()
        };
        push(Scancode::V, false);
        ui.poll(&mut settings, true, Context::OnFoot).unwrap();
        assert_eq!(ui.take_camera_cycles(), 1);
        assert_eq!(ui.take_camera_cycles(), 0);
        push(Scancode::V, true);
        ui.poll(&mut settings, true, Context::Vehicle).unwrap();
        assert_eq!(ui.take_camera_cycles(), 0);
        push(Scancode::V, false);
        ui.poll(&mut settings, true, Context::Vehicle).unwrap();
        assert_eq!(ui.take_camera_cycles(), 1);
        for context in [Context::Menu, Context::OnFoot] {
            ui.paused = context == Context::OnFoot;
            push(Scancode::V, false);
            ui.poll(&mut settings, true, context).unwrap();
            assert_eq!(ui.take_camera_cycles(), 0);
        }
        ui.paused = false;
        ui.focused = false;
        push(Scancode::V, false);
        ui.poll(&mut settings, true, Context::OnFoot).unwrap();
        assert_eq!(ui.take_camera_cycles(), 0);
        ui.focused = true;
        ui.controls
            .rebind(3, 0, Some(Binding::Key(Scancode::C)), false)
            .unwrap();
        push(Scancode::V, false);
        ui.poll(&mut settings, true, Context::OnFoot).unwrap();
        assert_eq!(ui.take_camera_cycles(), 0);
        push(Scancode::C, false);
        ui.poll(&mut settings, true, Context::Vehicle).unwrap();
        assert_eq!(ui.take_camera_cycles(), 1);
        ui.controls.toggle_mode().unwrap();
        push(Scancode::C, false);
        ui.poll(&mut settings, false, Context::OnFoot).unwrap();
        assert_eq!(ui.take_camera_cycles(), 1); // Classic controls / Original renderer too.
        ui.open_options(settings, false);
        push(Scancode::C, false);
        ui.poll(&mut settings, true, Context::OnFoot).unwrap();
        assert_eq!(ui.take_camera_cycles(), 0);
        ui.menu = None;
        ui.release_input = true;
        push(Scancode::C, false);
        ui.poll(&mut settings, true, Context::OnFoot).unwrap();
        assert_eq!(ui.take_camera_cycles(), 0);
    }
    #[test]
    #[ignore = "requires USA ROM, isolated SDL driver and diagnostic directory"]
    fn island_menu_backgrounds_refresh_pause_and_cached_options() {
        use super::*;
        let root = std::path::PathBuf::from(std::env::var_os("GTA_ISLAND_TEST_DIR").unwrap())
            .join("menus");
        std::fs::create_dir_all(&root).unwrap();
        let rom = std::fs::read("roms/usa.gba").unwrap();
        let mut m = gba_core::Machine::new(rom);
        let path = root.join("controls.cfg");
        std::fs::write(&path, crate::controls::Controls::default_file()).unwrap();
        let mut ui = Desktop::new(path).unwrap();
        ui.set_menu_skin(&m.bus.rom, None).unwrap();
        let creator = ui.texture_creator();
        let mut textures = Textures::new(&creator);
        let mut images = Vec::new();
        for (i, region) in [0u32, 1, 2, 0].into_iter().enumerate() {
            m.bus.ewram[0x303d0..0x303d4].copy_from_slice(&region.to_le_bytes());
            m.cpu.regs[0] = crate::game_menu::ROOT;
            m.cpu.regs[1] = 0;
            ui.observe_game_menu(&m, 0x080011ed);
            let mut ppm = b"P6\n720 480\n255\n".to_vec();
            ppm.extend(ui.menu_pixels().unwrap().unwrap());
            std::fs::write(root.join(format!("pause-{i}-{region}.ppm")), ppm).unwrap();
            if i == 0 {
                ui.open_options(crate::render_settings::Settings::default(), false);
            }
            ui.present_paused(&mut textures).unwrap();
            let pixels = ui.canvas.read_pixels(None, PixelFormatEnum::RGB24).unwrap();
            let (w, h) = ui.canvas.output_size().unwrap();
            let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
            ppm.extend(&pixels);
            std::fs::write(root.join(format!("options-{i}-{region}.ppm")), ppm).unwrap();
            assert_eq!(ui.menu_skin.as_ref().unwrap().region(), region as usize);
            assert_eq!(
                ui.menu.as_ref().unwrap().skin.as_ref().unwrap().region(),
                region as usize
            );
            images.push(pixels);
        }
        assert_ne!(images[0], images[1]);
        assert_ne!(images[1], images[2]);
        assert_eq!(images[0], images[3]);
        // No valid region during unload: retain the active skin, including
        // rebuilding fonts after a language change.
        m.bus.ewram[0x303d0..0x303d4].copy_from_slice(&1u32.to_le_bytes());
        ui.sync_menu_region(&m.bus);
        m.bus.ewram[0x303d0..0x303d4].copy_from_slice(&0x7e7eu32.to_le_bytes());
        ui.sync_menu_region(&m.bus);
        ui.set_menu_skin(&m.bus.rom, None).unwrap();
        assert_eq!(ui.menu_skin.as_ref().unwrap().region(), 1);
    }
    #[test]
    #[ignore = "requires isolated SDL dummy driver and output directory"]
    fn cheat_panel_real_sdl() {
        use super::*;
        use sdl2::keyboard::Mod;
        let root = std::path::PathBuf::from(std::env::var_os("GTA_CHEAT_TEST_DIR").unwrap());
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("controls.cfg");
        std::fs::write(&path, crate::controls::Controls::default_file()).unwrap();
        let mut ui = Desktop::new(path).unwrap();
        let events = ui._sdl.event().unwrap();
        let mut settings = crate::render_settings::Settings::default();
        let mut bus = gba_core::MemMap::new(vec![0; 0x200]);
        let creator = ui.texture_creator();
        let mut textures = Textures::new(&creator);
        let event = |key, keymod| Event::KeyDown {
            timestamp: 0,
            window_id: 0,
            keycode: Some(key),
            scancode: None,
            keymod,
            repeat: false,
        };
        events
            .push_event(event(Keycode::F12, Mod::LCTRLMOD))
            .unwrap();
        assert_eq!(
            ui.poll(&mut settings, true, crate::control_context::Context::Menu)
                .unwrap(),
            Some(0x3ff)
        );
        assert!(ui.cheats.visible);
        // Original and both Enhanced widths use the same readable panel scale.
        for width in [240, 720, 854] {
            let preview = vec![0u16; width * 480];
            ui.present(
                &bus,
                (width != 240).then_some(preview.as_slice()),
                &mut textures,
            )
            .unwrap();
            let (w, h) = ui.canvas.output_size().unwrap();
            let pixels = ui.canvas.read_pixels(None, PixelFormatEnum::RGB24).unwrap();
            assert!(pixels.chunks_exact(3).any(|p| p == [255, 220, 70]));
            let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
            ppm.extend(pixels);
            std::fs::write(root.join(format!("panel-{width}.ppm")), ppm).unwrap();
        }
        // Escape closes only the panel and does not send Start into the game.
        events
            .push_event(event(Keycode::Escape, Mod::NOMOD))
            .unwrap();
        assert_eq!(
            ui.poll(&mut settings, true, crate::control_context::Context::Menu)
                .unwrap(),
            Some(0x3ff)
        );
        assert!(!ui.cheats.visible);
        bus.framebuffer.fill(31);
        ui.present(&bus, None, &mut textures).unwrap();
        assert!(ui
            .canvas
            .read_pixels(None, PixelFormatEnum::RGB24)
            .unwrap()
            .chunks_exact(3)
            .all(|p| p == [255, 0, 0]));
    }
    #[test]
    #[ignore = "requires USA ROM and isolated SDL event queue"]
    fn localized_pause_and_settings_real_sdl() {
        use super::*;
        let directory = std::env::var_os("GTA_MENU_TEST_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("menu-localized-{}", std::process::id()))
            });
        std::fs::create_dir_all(&directory).unwrap();
        let mut pack: serde_json::Value = serde_json::from_slice(
            &std::fs::read("resources/legacy-language-v1-font.json").unwrap(),
        )
        .unwrap();
        pack["version"] = 2.into();
        pack["language"] = "ru-menu".into();
        pack["name"] = "Russian menu test".into();
        pack["entries"] = serde_json::json!({});
        pack["menu"] = serde_json::from_slice::<serde_json::Value>(
            &std::fs::read("resources/menu-ru-example.json").unwrap(),
        )
        .unwrap()["menu"]
            .clone();
        let pack_path = directory.join("pack.json");
        std::fs::write(&pack_path, serde_json::to_vec_pretty(&pack).unwrap()).unwrap();
        let rom = std::fs::read("roms/usa.gba").unwrap();
        let mut language = crate::localization::Language::load(&pack_path, &rom).unwrap();
        let mut machine = gba_core::Machine::new(rom);
        language.install(&mut machine);
        machine.cpu.regs[0] = crate::game_menu::ROOT;
        let controls = directory.join("controls.cfg");
        std::fs::write(&controls, crate::controls::Controls::default_file()).unwrap();
        let mut ui = Desktop::new(controls).unwrap();
        ui.set_menu_skin(&machine.bus.rom, Some(&language)).unwrap();
        let skin = ui.menu_skin.as_ref().unwrap();
        assert_eq!(skin.translate("Quit to Desktop"), "На рабочий стол");
        assert_eq!(skin.label(9), skin.host("quit_to_menu"));
        assert!(skin.host("quit_to_desktop").iter().any(|b| *b >= 128));
        ui.observe_game_menu(&machine, 0x080011ed);
        let mut ppm = b"P6\n720 480\n255\n".to_vec();
        ppm.extend(ui.menu_pixels().unwrap().unwrap());
        std::fs::write(directory.join("pause-ru.ppm"), ppm).unwrap();
        let creator = ui.texture_creator();
        let mut textures = Textures::new(&creator);
        ui.open_options(crate::render_settings::Settings::default(), false);
        ui.present_paused(&mut textures).unwrap();
        let pixels = ui.canvas.read_pixels(None, PixelFormatEnum::RGB24).unwrap();
        let (w, h) = ui.canvas.output_size().unwrap();
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        ppm.extend(pixels);
        std::fs::write(directory.join("settings-ru.ppm"), ppm).unwrap();
        ui.set_menu_skin(&machine.bus.rom, None).unwrap();
        assert_eq!(
            ui.menu_skin.as_ref().unwrap().host("quit_to_desktop"),
            b"QUIT TO DESKTOP"
        );
    }
    #[test]
    #[ignore = "requires USA ROM and isolated SDL event queue"]
    fn native_menu_frontend_real_sdl() {
        use super::*;
        let directory = std::env::var_os("GTA_MENU_TEST_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("native-menu-{}", std::process::id()))
            });
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("controls.cfg");
        std::fs::write(&path, crate::controls::Controls::default_file()).unwrap();
        let rom = std::fs::read("roms/usa.gba").unwrap();
        let mut machine = gba_core::Machine::new(rom);
        machine.cpu.regs[0] = crate::game_menu::ROOT;
        let mut ui = Desktop::new(path).unwrap();
        ui.set_menu_skin(&machine.bus.rom, None).unwrap();
        ui.observe_game_menu(&machine, 0x080011ed);
        let cpu = machine.cpu.clone();
        let ram = machine.bus.ewram.clone();
        let creator = ui.texture_creator();
        let mut textures = Textures::new(&creator);
        let pixels = ui.menu_pixels().unwrap().unwrap();
        let mut ppm = b"P6\n720 480\n255\n".to_vec();
        ppm.extend(pixels);
        std::fs::write(directory.join("pause.ppm"), ppm).unwrap();
        ui.present(&machine.bus, None, &mut textures).unwrap();
        assert_eq!(ui.canvas.logical_size(), (720, 480));
        let events = ui._sdl.event().unwrap();
        let mut settings = crate::render_settings::Settings::default();
        let send = |ui: &mut Desktop, settings: &mut crate::render_settings::Settings, key| {
            events
                .push_event(Event::KeyDown {
                    timestamp: 0,
                    window_id: ui.canvas.window().id(),
                    keycode: Some(key),
                    scancode: None,
                    keymod: sdl2::keyboard::Mod::NOMOD,
                    repeat: false,
                })
                .unwrap();
            ui.poll(settings, true, crate::control_context::Context::Menu)
                .unwrap()
                .unwrap()
        };
        for _ in 0..6 {
            assert!(matches!(
                send(&mut ui, &mut settings, Keycode::Down),
                0x3ff | 0x37f
            ));
        }
        send(&mut ui, &mut settings, Keycode::Return);
        assert!(ui.paused());
        assert!(ui.menu.is_some());
        ui.present_paused(&mut textures).unwrap();
        ui.menu
            .as_mut()
            .unwrap()
            .draw(&mut ui.canvas, textures.menu().unwrap())
            .unwrap();
        // Capture the actual cached menu pixels uploaded by Menu::draw.
        let pixels = ui.canvas.read_pixels(None, PixelFormatEnum::RGB24).unwrap();
        let (w, h) = ui.canvas.output_size().unwrap();
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        ppm.extend(pixels);
        std::fs::write(directory.join("enhanced.ppm"), ppm).unwrap();
        let previous = ui.controls.mode();
        send(&mut ui, &mut settings, Keycode::Return); // draft mode
        send(&mut ui, &mut settings, Keycode::Escape); // discard back to pause
        assert!(ui.menu.is_none());
        assert_eq!(ui.controls.mode(), previous);
        assert!(ui.game_menu.view.is_some());
        // Clear the close latch, then route an original Options action.
        ui.release_input = false;
        ui.game_menu.row = 1;
        let keys = send(&mut ui, &mut settings, Keycode::Return);
        assert_eq!(keys, 0x37f);
        machine.cpu.regs[1] = 1;
        ui.observe_game_menu(&machine, 0x080011ed);
        let mut confirmed = false;
        for _ in 0..30 {
            let keys = ui
                .poll(&mut settings, true, crate::control_context::Context::Menu)
                .unwrap()
                .unwrap();
            if keys == 0x3fe {
                confirmed = true;
                break;
            }
        }
        assert!(confirmed);
        assert_eq!(machine.bus.ewram, ram);
        machine.cpu.regs[1] = 0;
        assert!(machine.cpu == cpu);
        // Fallback pages use the same logical confirmation/back controls even
        // when the user's classic A/B are Z/X. Enter must not also send Start.
        ui.game_menu.cancel_navigation();
        ui.observe_game_menu(&machine, 0x080013ad);
        ui.release_input = false;
        assert_eq!(send(&mut ui, &mut settings, Keycode::Return), 0x3fe);
        ui.game_menu.cancel_navigation();
        ui.release_input = false;
        assert_eq!(send(&mut ui, &mut settings, Keycode::Escape), 0x3fd);
        assert!(ui.menu.is_none());
        ui.game_menu.cancel_navigation();
        ui.release_input = false;
        ui.observe_game_menu(&machine, 0x080011ed);
        ui.game_menu.row = 7;
        send(&mut ui, &mut settings, Keycode::Return);
        assert!(ui.menu.as_ref().unwrap().can_save_game);
        ui.present_paused(&mut textures).unwrap();
        let pixels = ui.canvas.read_pixels(None, PixelFormatEnum::RGB24).unwrap();
        let (w, h) = ui.canvas.output_size().unwrap();
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        ppm.extend(pixels);
        std::fs::write(directory.join("exit-save.ppm"), ppm).unwrap();
        send(&mut ui, &mut settings, Keycode::Up);
        send(&mut ui, &mut settings, Keycode::Up);
        send(&mut ui, &mut settings, Keycode::Return);
        assert!(ui.game_menu.save_exit_active());
        assert!(!ui.paused());
        events.push_event(Event::Quit { timestamp: 0 }).unwrap();
        assert!(ui
            .poll(&mut settings, true, crate::control_context::Context::Menu)
            .unwrap()
            .is_some());
        ui.observe_game_menu(&machine, 0x08007a15);
        ui.observe_game_menu(&machine, 0x08007ad1);
        assert!(ui.save_exit_needs_commit());
        ui.finish_save_exit(Err("disk full".into()));
        assert!(ui
            .poll(&mut settings, true, crate::control_context::Context::Menu)
            .unwrap()
            .is_some());
        assert!(ui.paused());
        assert!(ui.menu.as_ref().unwrap().message.contains("disk full"));
        // Only a confirmed disk commit permits the next poll to close.
        ui.menu = None;
        ui.release_input = false;
        ui.observe_game_menu(&machine, 0x080011ed);
        ui.open_options(settings.clone(), true);
        send(&mut ui, &mut settings, Keycode::Up);
        send(&mut ui, &mut settings, Keycode::Up);
        send(&mut ui, &mut settings, Keycode::Return);
        ui.observe_game_menu(&machine, 0x08007a15);
        ui.observe_game_menu(&machine, 0x08007ad1);
        ui.finish_save_exit(Ok(()));
        assert!(ui
            .poll(&mut settings, true, crate::control_context::Context::Menu)
            .unwrap()
            .is_none());
    }
    #[test]
    #[ignore = "requires isolated SDL event queue"]
    fn language_options_real_sdl() {
        use super::*;
        use crate::control_context::Context;
        let directory = std::env::var_os("GTA_LANGUAGE_OPTIONS_TEST_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("language-options-{}", std::process::id()))
            });
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("controls.cfg");
        let original = crate::controls::Controls::default_file();
        std::fs::write(&path, &original).unwrap();
        let choices = vec![crate::localization::Choice {
            file: "ru-demo.json".into(),
            label: "Русский — проверка (ru)".into(),
            font: crate::localization::test_font(),
        }];
        let mut ui = Desktop::new(path.clone()).unwrap();
        ui.set_languages(choices.clone());
        let creator = ui.texture_creator();
        let mut textures = Textures::new(&creator);
        let events = ui._sdl.event().unwrap();
        let mut settings = crate::render_settings::Settings::default();
        let send = |ui: &mut Desktop, settings: &mut crate::render_settings::Settings, key| {
            events
                .push_event(Event::KeyDown {
                    timestamp: 0,
                    window_id: ui.canvas.window().id(),
                    keycode: Some(key),
                    scancode: sdl2::keyboard::Scancode::from_keycode(key),
                    keymod: sdl2::keyboard::Mod::NOMOD,
                    repeat: false,
                })
                .unwrap();
            ui.poll(settings, true, Context::OnFoot).unwrap();
        };
        for save in [false, true] {
            send(&mut ui, &mut settings, Keycode::Escape);
            for _ in 0..8 {
                send(&mut ui, &mut settings, Keycode::Down);
            }
            send(&mut ui, &mut settings, Keycode::Return);
            send(&mut ui, &mut settings, Keycode::Down);
            send(&mut ui, &mut settings, Keycode::Return);
            assert_eq!(ui.language, None);
            assert!(!directory.join("enhanced.cfg").exists());
            assert_eq!(
                ui.menu.as_ref().unwrap().prefs.language.as_deref(),
                Some("ru-demo.json")
            );
            ui.present_paused(&mut textures).unwrap();
            let (width, height) = ui.canvas.output_size().unwrap();
            let mut pixels = format!("P6\n{width} {height}\n255\n").into_bytes();
            pixels.extend(ui.canvas.read_pixels(None, PixelFormatEnum::RGB24).unwrap());
            std::fs::write(directory.join("language-menu.ppm"), pixels).unwrap();
            send(&mut ui, &mut settings, Keycode::Escape);
            if save {
                send(&mut ui, &mut settings, Keycode::Down);
                send(&mut ui, &mut settings, Keycode::Return);
            } else {
                send(&mut ui, &mut settings, Keycode::Escape);
            }
            assert!(ui.menu.is_none());
            ui.poll(&mut settings, true, Context::OnFoot).unwrap();
            assert_eq!(ui.language.as_deref(), save.then_some("ru-demo.json"));
            assert_eq!(
                ui.take_language_change(),
                if save {
                    Some(Some("ru-demo.json".into()))
                } else {
                    None
                }
            );
            assert_eq!(
                crate::controls::Controls::load(&path).unwrap().encode(),
                crate::controls::Controls::parse(&original)
                    .unwrap()
                    .encode()
            );
            if !save {
                assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            }
        }
        let preferences =
            crate::options::Preferences::load(&directory.join("enhanced.cfg")).unwrap();
        assert_eq!(preferences.language.as_deref(), Some("ru-demo.json"));
        std::fs::copy(
            directory.join("enhanced.cfg"),
            directory.join("selected.cfg"),
        )
        .unwrap();
        drop(textures);
        drop(creator);
        drop(events);
        drop(ui);
        let mut ui = Desktop::new(path).unwrap();
        ui.set_languages(choices);
        assert_eq!(ui.language.as_deref(), Some("ru-demo.json"));
        ui.set_languages(Vec::new());
        assert_eq!(ui.language, None);
    }

    use super::*;
    #[test]
    fn pixel_crosshair_has_transparent_hotspot_white_arms_and_black_outline() {
        let (data, mask) = crosshair_bits();
        let pixel = |x: usize, y: usize| {
            let b = 0x80 >> (x % 8);
            (data[y * 4 + x / 8] & b != 0, mask[y * 4 + x / 8] & b != 0)
        };
        assert_eq!(pixel(15, 15), (false, false));
        assert_eq!(pixel(15, 7), (false, true));
        assert_eq!(pixel(14, 7), (true, true));
        assert_eq!(pixel(0, 0), (false, false));
    }
    use sdl2::keyboard::Scancode;
    #[test]
    #[ignore = "run alone with SDL dummy video/audio to verify Enhanced menu sound on pause"]
    fn enhanced_menu_audio_real_sdl() {
        use crate::control_context::Context;
        let root = std::env::temp_dir().join(format!("menu-sound-sdl-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut ui = Desktop::new(root.join("controls.cfg")).unwrap();
        ui.set_menu_skin(&std::fs::read("roms/usa.gba").unwrap(), None)
            .unwrap();
        assert!(ui.menu_click.as_ref().unwrap().iter().any(|v| *v != 0));
        let events = ui._sdl.event().unwrap();
        let mut settings = ui.initial_render_settings();
        let send =
            |ui: &mut Desktop, settings: &mut crate::render_settings::Settings, key: Keycode| {
                events
                    .push_event(Event::KeyDown {
                        timestamp: 0,
                        window_id: ui.canvas.window().id(),
                        keycode: Some(key),
                        scancode: None,
                        keymod: sdl2::keyboard::Mod::NOMOD,
                        repeat: false,
                    })
                    .unwrap();
                ui.poll(settings, true, Context::OnFoot).unwrap();
            };
        ui.paused = true;
        send(&mut ui, &mut settings, Keycode::Escape);
        let first = ui.audio.as_mut().unwrap().test_state();
        assert_eq!(first.0, 1);
        assert!(first.1 > 0);
        for _ in 0..8 {
            ui.poll(&mut settings, true, Context::OnFoot).unwrap();
        }
        let later = ui.audio.as_mut().unwrap().test_state();
        assert!(
            later.1 < first.1,
            "audio callback must continue while paused"
        );
        for k in [
            Keycode::Down,
            Keycode::Return,
            Keycode::Escape,
            Keycode::Escape,
        ] {
            send(&mut ui, &mut settings, k);
        }
        assert!(ui.menu.is_none());
        assert!(ui.paused());
        assert_eq!(ui.audio.as_mut().unwrap().test_state().0, 5);
        let mut source = gba_core::MemMap::new(Vec::new());
        source.ewram[0x3e4] = 1;
        for level in [1, 2, 3] {
            source.ewram[0x10c61] = level;
            ui.sync_menu_region(&source);
            assert_eq!(ui.sfx_volume, level);
        }
        source.ewram[0x10c61] = 0;
        ui.sync_menu_region(&source);
        assert_eq!(ui.audio.as_mut().unwrap().test_state().1, 0);
        send(&mut ui, &mut settings, Keycode::Escape);
        assert!(ui.menu.is_some());
        assert_eq!(ui.audio.as_mut().unwrap().test_state().0, 5);
        drop(events);
        drop(ui);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "run alone with SDL dummy/software to verify native menu output and persistence"]
    fn options_menu_real_sdl() {
        use crate::control_context::Context;
        let directory = std::env::temp_dir().join(format!("options-sdl-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("controls.cfg");
        let original = crate::controls::Controls::default_file();
        std::fs::write(&path, &original).unwrap();
        let mut ui = Desktop::new(path.clone()).unwrap();
        let creator = ui.texture_creator();
        let mut textures = Textures::new(&creator);
        let events = ui._sdl.event().unwrap();
        let mut settings = ui.initial_render_settings();
        let mut source = gba_core::MemMap::new(Vec::new());
        source.framebuffer.fill(31 << 10);
        ui.present(&source, None, &mut textures).unwrap();
        let send =
            |ui: &mut Desktop, settings: &mut crate::render_settings::Settings, k: Keycode| {
                events
                    .push_event(Event::KeyDown {
                        timestamp: 0,
                        window_id: ui.canvas.window().id(),
                        keycode: Some(k),
                        scancode: Scancode::from_keycode(k),
                        keymod: sdl2::keyboard::Mod::NOMOD,
                        repeat: false,
                    })
                    .unwrap();
                assert_eq!(
                    ui.poll(settings, true, Context::OnFoot).unwrap(),
                    Some(0x3ff)
                );
            };
        let capture = |ui: &Desktop, name: &str| {
            if let Ok(dir) = std::env::var("GTA_OPTIONS_CAPTURE") {
                let (w, h) = ui.canvas.output_size().unwrap();
                let mut bytes = format!("P6\n{w} {h}\n255\n").into_bytes();
                bytes.extend(ui.canvas.read_pixels(None, PixelFormatEnum::RGB24).unwrap());
                std::fs::write(std::path::Path::new(&dir).join(name), bytes).unwrap();
            }
        };
        send(&mut ui, &mut settings, Keycode::Escape);
        assert!(ui.paused());
        ui.present_paused(&mut textures).unwrap();
        capture(&ui, "root.ppm");
        for k in [Keycode::Down, Keycode::Down, Keycode::Return] {
            send(&mut ui, &mut settings, k);
        }
        for _ in 0..6 {
            send(&mut ui, &mut settings, Keycode::Down);
        }
        send(&mut ui, &mut settings, Keycode::Return); // capture Jump
        send(&mut ui, &mut settings, Keycode::J); // conflict
        ui.present_paused(&mut textures).unwrap();
        capture(&ui, "conflict.ppm");
        assert_eq!(ui.controls.binding_text(1, 6), "Space");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        send(&mut ui, &mut settings, Keycode::C); // valid replacement
        ui.present_paused(&mut textures).unwrap();
        capture(&ui, "bindings.ppm");
        send(&mut ui, &mut settings, Keycode::Escape); // root; selected on-foot row 2
        for _ in 0..2 {
            send(&mut ui, &mut settings, Keycode::Down);
        }
        send(&mut ui, &mut settings, Keycode::Return); // renderer original, draft only
        assert_eq!(
            settings.renderer,
            crate::render_settings::Renderer::Enhanced
        );
        send(&mut ui, &mut settings, Keycode::Down); // Graphics root
        send(&mut ui, &mut settings, Keycode::Return);
        send(&mut ui, &mut settings, Keycode::Down); // aspect
        send(&mut ui, &mut settings, Keycode::Return); // wide draft
        ui.present_paused(&mut textures).unwrap();
        capture(&ui, "graphics-menu.ppm");
        assert_eq!(settings.aspect, crate::viewport::Aspect::Original);
        send(&mut ui, &mut settings, Keycode::Escape); // root 5
        for _ in 0..2 {
            send(&mut ui, &mut settings, Keycode::Down);
        }
        send(&mut ui, &mut settings, Keycode::Return); // HUD submenu
        send(&mut ui, &mut settings, Keycode::Left); // all: 100 -> 85
        send(&mut ui, &mut settings, Keycode::Down); // radar
        send(&mut ui, &mut settings, Keycode::Left); // radar: 85 -> 75
        ui.present_paused(&mut textures).unwrap();
        capture(&ui, "hud-menu.ppm");
        assert_eq!(settings.hud, crate::hud_layout::Settings::default()); // still a draft
        send(&mut ui, &mut settings, Keycode::Escape); // root HUD row 7
        send(&mut ui, &mut settings, Keycode::Down); // language
        send(&mut ui, &mut settings, Keycode::Down); // save
                                                     // A failed save leaves the menu open and active settings untouched.
        ui.preferences_path = directory.join("missing/enhanced.cfg");
        send(&mut ui, &mut settings, Keycode::Return);
        assert!(ui.paused());
        assert!(ui
            .menu
            .as_ref()
            .unwrap()
            .message
            .starts_with("Save failed:"));
        assert_eq!(ui.controls.binding_text(1, 6), "Space");
        assert_eq!(
            settings.renderer,
            crate::render_settings::Renderer::Enhanced
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        ui.preferences_path = directory.join("enhanced.cfg");
        send(&mut ui, &mut settings, Keycode::Return); // save
        assert!(!ui.paused());
        assert_eq!(
            settings.renderer,
            crate::render_settings::Renderer::Original
        );
        assert_eq!(ui.controls.binding_text(1, 6), "C");
        assert_eq!(settings.hud.radar, crate::hud_layout::Size::Medium);
        assert_eq!(settings.hud.status, crate::hud_layout::Size::Large);
        assert_eq!(settings.aspect, crate::viewport::Aspect::Wide);
        // Original -> wide -> original uses cached textures and restores sizes.
        let wide = vec![31; 854 * 480];
        ui.present(&source, Some(&wide), &mut textures).unwrap();
        assert_eq!(ui.canvas.logical_size(), (854, 480));
        ui.canvas.window_mut().set_size(960, 640).unwrap();
        ui.present(&source, None, &mut textures).unwrap();
        assert_eq!(ui.canvas.logical_size(), (240, 160));
        assert!(ui
            .canvas
            .read_pixels(None, PixelFormatEnum::RGB24)
            .unwrap()
            .chunks_exact(3)
            .all(|rgb| rgb == [0, 0, 255]));
        // Cancel another edited draft; no active state or file changes.
        let saved = std::fs::read_to_string(&path).unwrap();
        send(&mut ui, &mut settings, Keycode::Escape);
        send(&mut ui, &mut settings, Keycode::Return); // control mode
        send(&mut ui, &mut settings, Keycode::Escape); // discard
        assert_eq!(ui.controls.mode(), crate::controls::Mode::Enhanced);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
        // Closing options must preserve P-pause and still let P resume input.
        for save in [false, true] {
            send(&mut ui, &mut settings, Keycode::P);
            assert!(ui.paused());
            send(&mut ui, &mut settings, Keycode::Escape);
            ui.present_paused(&mut textures).unwrap();
            if save {
                for _ in 0..9 {
                    send(&mut ui, &mut settings, Keycode::Down);
                }
                send(&mut ui, &mut settings, Keycode::Return);
            } else {
                send(&mut ui, &mut settings, Keycode::Escape);
            }
            ui.present_paused(&mut textures).unwrap();
            assert!(ui.menu.is_none());
            assert!(ui.paused());
            ui.poll(&mut settings, true, Context::OnFoot).unwrap();
            send(&mut ui, &mut settings, Keycode::P);
            assert!(
                !ui.paused(),
                "P must resume after options close (save={save})"
            );
            send(&mut ui, &mut settings, Keycode::Escape);
            assert!(ui.menu.is_some(), "Menu hotkeys must work after resuming");
            send(&mut ui, &mut settings, Keycode::Escape);
        }
        drop(textures);
        drop(creator);
        drop(events);
        drop(ui);
        let ui = Desktop::new(path).unwrap();
        assert_eq!(
            ui.initial_render_settings().renderer,
            crate::render_settings::Renderer::Original
        );
        assert_eq!(ui.controls.binding_text(1, 6), "C");
        assert_eq!(
            ui.initial_render_settings().aspect,
            crate::viewport::Aspect::Wide
        );
        assert_eq!(
            ui.initial_render_settings().hud.radar,
            crate::hud_layout::Size::Medium
        );
        assert_eq!(
            ui.initial_render_settings().hud.messages,
            crate::hud_layout::Size::Large
        );
        drop(ui);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "run alone with SDL dummy/software to exercise the real event queue and output"]
    fn mouse_aim_options_and_viewport_real_sdl() {
        use crate::control_context::Context;
        let directory = std::env::temp_dir().join(format!("mouse-aim-sdl-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("controls.cfg");
        let mut ui = Desktop::new(path.clone()).unwrap();
        let creator = ui.texture_creator();
        let mut textures = Textures::new(&creator);
        let events = ui._sdl.event().unwrap();
        let mut settings = ui.initial_render_settings();
        let send = |ui: &mut Desktop, settings: &mut crate::render_settings::Settings, key| {
            events
                .push_event(Event::KeyDown {
                    timestamp: 0,
                    window_id: ui.canvas.window().id(),
                    keycode: Some(key),
                    scancode: sdl2::keyboard::Scancode::from_keycode(key),
                    keymod: sdl2::keyboard::Mod::NOMOD,
                    repeat: false,
                })
                .unwrap();
            ui.poll(settings, true, Context::OnFoot).unwrap();
        };
        // Exercise the real menu queue, including last-row navigation/capture.
        for save in [false, true] {
            send(&mut ui, &mut settings, Keycode::Escape);
            send(&mut ui, &mut settings, Keycode::Down);
            send(&mut ui, &mut settings, Keycode::Down);
            send(&mut ui, &mut settings, Keycode::Return);
            send(&mut ui, &mut settings, Keycode::Up); // wraps to the new last row
            send(&mut ui, &mut settings, Keycode::Return);
            assert!(ui.menu.as_ref().unwrap().controls.mouse_aim());
            send(&mut ui, &mut settings, Keycode::Up); // Aim (hold)
            assert_eq!(
                ui.menu.as_ref().unwrap().controls.binding_text(1, 11),
                "MouseRight"
            );
            send(&mut ui, &mut settings, Keycode::Return);
            send(&mut ui, &mut settings, Keycode::G);
            assert_eq!(ui.menu.as_ref().unwrap().controls.binding_text(1, 11), "G");
            ui.present_paused(&mut textures).unwrap();
            if let Ok(dir) = std::env::var("GTA_OPTIONS_CAPTURE") {
                let (w, h) = ui.canvas.output_size().unwrap();
                let mut bytes = format!("P6\n{w} {h}\n255\n").into_bytes();
                bytes.extend(ui.canvas.read_pixels(None, PixelFormatEnum::RGB24).unwrap());
                std::fs::write(
                    std::path::Path::new(&dir).join("mouse-aim-options.ppm"),
                    bytes,
                )
                .unwrap();
            }
            send(&mut ui, &mut settings, Keycode::Escape);
            if save {
                for _ in 0..7 {
                    send(&mut ui, &mut settings, Keycode::Down);
                }
                send(&mut ui, &mut settings, Keycode::Return);
            } else {
                send(&mut ui, &mut settings, Keycode::Escape);
            }
            assert_eq!(ui.controls.mouse_aim(), save);
            ui.poll(&mut settings, true, Context::OnFoot).unwrap();
        }
        assert!(crate::controls::Controls::load(&path).unwrap().mouse_aim());
        assert_eq!(
            crate::controls::Controls::load(&path)
                .unwrap()
                .binding_text(1, 11),
            "G"
        );
        // SDL's own coordinate transform accounts for black bars and resizing.
        for (w, h) in [(240, 160), (720, 480), (854, 480)] {
            ui.canvas.set_logical_size(w, h).unwrap();
            let (mut wx, mut wy) = (0, 0);
            let (mut lx, mut ly) = (0.0, 0.0);
            unsafe {
                sdl2::sys::SDL_RenderLogicalToWindow(
                    ui.canvas.raw(),
                    w as f32 * 0.7,
                    h as f32 * 0.3,
                    &mut wx,
                    &mut wy,
                );
                sdl2::sys::SDL_RenderWindowToLogical(ui.canvas.raw(), wx, wy, &mut lx, &mut ly);
            }
            assert!((lx - w as f32 * 0.7).abs() < 1.5);
            assert!((ly - h as f32 * 0.3).abs() < 1.5);
        }
        // Feed a checked actor and the displayed projection through the same
        // cursor-to-heading path used by main, then exercise its stop gates.
        let mut rom = vec![0; 0x200];
        rom[0xac..0xb0].copy_from_slice(b"BGTE");
        rom[0x100..0x104].copy_from_slice(&2u32.to_le_bytes());
        let mut bus = gba_core::MemMap::new(rom);
        bus.io[..2].copy_from_slice(&0x1741u16.to_le_bytes());
        for (address, value) in [
            (0xf10, 0x02001000u32),
            (0x1008, 2),
            (0x1010, 0x08000100),
            (0x1018, 0x02002000),
            (0x101c, 0x02002100),
            (0x2100, 0x02002000),
            (0x2130, 0x02001000),
            (0x2000, 100 * 65536),
            (0x2004, 100 * 65536),
        ] {
            bus.ewram[address..address + 4].copy_from_slice(&value.to_le_bytes());
        }
        ui.set_aim_projection(Some(crate::mouse_aim::Projection {
            actor: 0x02001000,
            screen: [427.0, 240.0],
            world: [100.0, 100.0],
            pixels_per_world: [3.0; 2],
            size: [854, 480],
        }));
        ui.aim_cursor = Some([487.0, 180.0]);
        assert_eq!(ui.aim_angle(&bus), Some(128));
        ui.aim_cursor = Some([427.0, 240.0]);
        assert_eq!(ui.aim_angle(&bus), Some(128)); // near the actor: retain heading
        ui.aim_cursor = Some([900.0, 240.0]);
        assert_eq!(ui.aim_angle(&bus), None); // outside the actual world viewport
        ui.aim_cursor = Some([487.0, 240.0]);
        ui.focused = false;
        assert_eq!(ui.aim_angle(&bus), None);
        ui.focused = true;
        ui.controls.toggle_mode().unwrap();
        assert_eq!(ui.aim_angle(&bus), None);
        drop(textures);
        drop(creator);
        drop(events);
        drop(ui);
        let ui = Desktop::new(path).unwrap();
        assert!(ui.controls.mouse_aim());
        drop(ui);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "run alone with SDL dummy/software to exercise the real event queue and output"]
    fn f_keys_switch_real_sdl_output_and_reuse_textures() {
        use crate::render_settings::{Renderer, Settings, TextureMapping};
        let directory = std::env::temp_dir().join(format!("render-keys-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut ui = Desktop::new(directory.join("controls.cfg")).unwrap();
        let creator = ui.texture_creator();
        let mut textures = Textures::new(&creator);
        let events = ui._sdl.event().unwrap();
        let mut settings = Settings::default();
        let mut source = gba_core::MemMap::new(Vec::new());
        source.framebuffer.fill(31 << 10);
        let enhanced = vec![31; 720 * 480];
        for (key, repeat, renderer, mapping) in [
            (
                Keycode::F6,
                false,
                Renderer::Original,
                TextureMapping::Perspective,
            ),
            (
                Keycode::F6,
                true,
                Renderer::Original,
                TextureMapping::Perspective,
            ),
            (
                Keycode::F7,
                false,
                Renderer::Original,
                TextureMapping::Perspective,
            ),
            (
                Keycode::F6,
                false,
                Renderer::Enhanced,
                TextureMapping::Perspective,
            ),
            (
                Keycode::F7,
                false,
                Renderer::Enhanced,
                TextureMapping::Perspective,
            ),
            (
                Keycode::F6,
                false,
                Renderer::Original,
                TextureMapping::Perspective,
            ),
        ] {
            events
                .push_event(Event::KeyDown {
                    timestamp: 0,
                    window_id: ui.canvas.window().id(),
                    keycode: Some(key),
                    scancode: Scancode::from_keycode(key),
                    keymod: sdl2::keyboard::Mod::NOMOD,
                    repeat,
                })
                .unwrap();
            ui.poll(&mut settings, true, crate::control_context::Context::Menu)
                .unwrap();
            assert_eq!(settings.renderer, renderer);
            assert_eq!(settings.textures, mapping);
            let preview = (renderer == Renderer::Enhanced).then_some(enhanced.as_slice());
            ui.present(&source, preview, &mut textures).unwrap();
            let pixels = ui.canvas.read_pixels(None, PixelFormatEnum::RGB24).unwrap();
            let expected = if renderer == Renderer::Enhanced {
                [255, 0, 0]
            } else {
                [0, 0, 255]
            };
            assert!(pixels.chunks_exact(3).all(|p| p == expected));
        }
        assert_eq!(textures.allocations, 2);
        for (repeat, expected) in [
            (false, crate::controls::Mode::Classic),
            (true, crate::controls::Mode::Classic),
            (false, crate::controls::Mode::Enhanced),
        ] {
            events
                .push_event(Event::KeyDown {
                    timestamp: 0,
                    window_id: ui.canvas.window().id(),
                    keycode: Some(Keycode::F8),
                    scancode: Some(Scancode::F8),
                    keymod: sdl2::keyboard::Mod::NOMOD,
                    repeat,
                })
                .unwrap();
            ui.poll(
                &mut settings,
                true,
                crate::control_context::Context::Vehicle,
            )
            .unwrap();
            assert_eq!(ui.controls.mode(), expected);
            assert_eq!(settings.renderer, Renderer::Original);
        }
        std::fs::write(
            &ui.controls_path,
            crate::controls::Controls::default_file().replace("Jump = Space", "Jump = C"),
        )
        .unwrap();
        let reload = || {
            events.push_event(Event::KeyDown {
                timestamp: 0,
                window_id: ui.canvas.window().id(),
                keycode: Some(Keycode::F5),
                scancode: Some(Scancode::F5),
                keymod: sdl2::keyboard::Mod::NOMOD,
                repeat: false,
            })
        };
        // Keep the event independent of ui's mutable poll borrow.
        reload().unwrap();
        ui.poll(&mut settings, true, crate::control_context::Context::OnFoot)
            .unwrap();
        assert_eq!(
            ui.controls
                .contextual_buttons(crate::control_context::Context::OnFoot, |k| k
                    == Scancode::C),
            0x3fc
        );
        ui.fps_started = Instant::now() - Duration::from_secs(1);
        ui.fps_frames = 58;
        ui.present(&source, None, &mut textures).unwrap();
        assert!((40.0..70.0).contains(&ui.fps.unwrap()));
        ui.set_preview_status("Camera preview");
        ui.final_status(
            crate::coverage::Coverage {
                native_dispatches: 9999,
                interpreted_steps: 1,
            },
            false,
        )
        .unwrap();
        assert!(ui.title().contains("interpreted: 1"));
        assert!(ui.title().contains("FPS: "));
        assert!(ui.title().contains("Camera preview"));
        for removed in ["F6", "F11", "Textures:", "Renderer:"] {
            assert!(!ui.title().contains(removed));
        }
        ui.paused = true;
        ui.final_status(crate::coverage::Coverage::default(), false)
            .unwrap();
        assert!(ui.title().contains("FPS: --"));
    }
    #[test]
    fn gba_channel_order() {
        assert_eq!(rgb(31), [255, 0, 0]);
        assert_eq!(rgb(31 << 10), [0, 0, 255]);
    }
    #[test]
    fn active_low_diagonal_and_run() {
        assert_eq!(
            crate::controls::Controls::parse(crate::controls::DEFAULT)
                .unwrap()
                .buttons(|k| matches!(k, Scancode::W | Scancode::D | Scancode::LShift)),
            0x3ff & !(64 | 16 | 2)
        );
        assert_eq!(
            crate::controls::Controls::parse(crate::controls::DEFAULT)
                .unwrap()
                .buttons(|_| false),
            0x3ff
        );
    }
}
