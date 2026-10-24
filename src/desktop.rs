use sdl2::{
    audio::{AudioQueue, AudioSpecDesired},
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
    menu: Option<Texture<'a>>,
    allocations: u64,
}
impl<'a> Textures<'a> {
    pub fn new(creator: &'a TextureCreator<WindowContext>) -> Self {
        Self {
            creator,
            original: None,
            preview: None,
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
    fn get(&mut self, preview: bool) -> Result<&mut Texture<'a>, String> {
        let slot = if preview {
            &mut self.preview
        } else {
            &mut self.original
        };
        if slot.is_none() {
            let (width, height) = if preview { (720, 480) } else { (240, 160) };
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
    canvas: Canvas<Window>,
    events: sdl2::EventPump,
    audio: Option<AudioQueue<i16>>,
    _sdl: sdl2::Sdl,
    pixels: Vec<u8>,
    paused: bool,
    focused: bool,
    menu: Option<crate::options::Menu>,
    release_input: bool,
    preferences_path: std::path::PathBuf,
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

impl Desktop {
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
                640,
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
        let audio = (|| {
            let subsystem = sdl.audio()?;
            subsystem.open_queue::<i16, _>(
                None,
                &AudioSpecDesired {
                    freq: Some(65536),
                    channels: Some(2),
                    samples: Some(2048),
                },
            )
        })();
        let audio = match audio {
            Ok(q) => {
                q.resume();
                Some(q)
            }
            Err(e) => {
                eprintln!("Audio unavailable: {e}");
                None
            }
        };
        Ok(Self {
            title_updated: Instant::now(),
            title_initialized: false,
            notice: "",
            preview_status: "",
            render_settings: preferences.render,
            renderer_available: false,
            gamepad,
            controls,
            input_latch: Default::default(),
            controls_path,
            canvas,
            events,
            audio,
            _sdl: sdl,
            pixels: vec![0; 240 * 160 * 3],
            paused: false,
            focused: true,
            menu: None,
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
        self.paused || !self.focused || self.menu.is_some()
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
        self.renderer_available = renderer_available;
        let events: Vec<_> = self.events.poll_iter().collect();
        for event in events {
            if matches!(event, Event::Quit { .. }) {
                return Ok(None);
            }
            if let Event::Window { win_event, .. } = event {
                match win_event {
                    WindowEvent::FocusLost => self.focused = false,
                    WindowEvent::FocusGained => self.focused = true,
                    _ => {}
                }
            }
            if self.menu.is_some() {
                if !self.focused {
                    continue;
                }
                let command = self.menu.as_mut().unwrap().event(&event);
                match command {
                    crate::options::Command::None => {}
                    crate::options::Command::Quit => return Ok(None),
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
                self.menu = Some(crate::options::Menu::new(
                    self.controls.clone(),
                    crate::options::Preferences {
                        render: *settings,
                        fullscreen: self.canvas.window().fullscreen_state()
                            != sdl2::video::FullscreenType::Off,
                    },
                ));
                self.title_initialized = false;
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
        if self.paused() {
            self.fps_started = Instant::now();
            self.fps_frames = 0;
            self.fps = None;
            if let Some(audio) = &self.audio {
                audio.clear();
            }
            self.next_frame = Instant::now();
            std::thread::sleep(Duration::from_millis(10));
            return Ok(Some(0x3ff));
        }
        let keyboard = self.events.keyboard_state();
        let mouse = self.events.mouse_state();
        if self.release_input {
            // Consume held menu/capture buttons in both profiles and on gamepads.
            // Resume actions only after a physical release, even in Classic mode.
            self.release_input = keyboard.pressed_scancodes().next().is_some()
                || mouse.pressed_mouse_buttons().next().is_some()
                || self
                    .gamepad
                    .as_ref()
                    .is_some_and(|pad| pad.buttons() != 0x3ff);
            self.input_latch = Default::default();
            return Ok(Some(0x3ff));
        }
        Ok(Some(crate::gamepad::merge(
            self.input_latch
                .sample(&self.controls, context, |binding| match binding {
                    crate::controls::Binding::Key(key) => keyboard.is_scancode_pressed(key),
                    crate::controls::Binding::MouseLeft => mouse.left(),
                    crate::controls::Binding::MouseMiddle => mouse.middle(),
                    crate::controls::Binding::MouseRight => mouse.right(),
                }),
            self.gamepad.as_ref().map_or(0x3ff, |pad| pad.buttons()),
        )))
    }

    pub fn initial_render_settings(&self) -> crate::render_settings::Settings {
        self.render_settings
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
            menu.prefs,
        ) {
            let _ = self.canvas.window_mut().set_fullscreen(old);
            return Err(e);
        }
        self.controls = menu.controls.clone();
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
            let preview = self.pixels.len() == 720 * 480 * 3;
            let (width, height) = if preview { (720, 480) } else { (240, 160) };
            self.canvas
                .set_logical_size(width, height)
                .map_err(|e| e.to_string())?;
            self.canvas.set_draw_color(sdl2::pixels::Color::BLACK);
            self.canvas.clear();
            self.canvas.copy(textures.get(preview)?, None, None)?;
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
        let frame = preview.unwrap_or(&source.framebuffer);
        let (width, height) = if preview.is_some() {
            (720u32, 480u32)
        } else {
            (240, 160)
        };
        let samples = &source.audio_buf;
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
        let texture = textures.get(preview.is_some())?;
        texture
            .update(None, pixels, width as usize * 3)
            .map_err(|e| e.to_string())?;
        self.canvas.set_draw_color(sdl2::pixels::Color::BLACK);
        self.canvas.clear();
        self.canvas.copy(texture, None, None)?;
        self.canvas.present();
        self.present_frames += 1;
        self.present_ns += start.elapsed().as_nanos();
        {
            if let Some(audio) = &self.audio {
                // Bound latency after stalls; never allow an unlimited audio backlog.
                if audio.size() > 65536 / 5 * 4 {
                    audio.clear();
                }
                audio.queue_audio(samples)?;
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
    use super::*;
    use sdl2::keyboard::Scancode;
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
        for _ in 0..3 {
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
        // Restore original resolution after drawing the 720x480 menu.
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
        // P-paused game must redraw its cached frame when the menu closes.
        ui.paused = true;
        send(&mut ui, &mut settings, Keycode::Escape);
        ui.present_paused(&mut textures).unwrap();
        send(&mut ui, &mut settings, Keycode::Escape);
        ui.present_paused(&mut textures).unwrap();
        assert!(ui.menu.is_none());
        assert!(ui.paused());
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
