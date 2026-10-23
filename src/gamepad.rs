use sdl2::{
    controller::{Axis, Button, GameController},
    GameControllerSubsystem,
};

// Digital GBA directions: a quarter of stick travel, with a neutral centre.
const DEADZONE: i16 = 8192;

pub struct Gamepad {
    controller: Option<GameController>,
    subsystem: GameControllerSubsystem,
}

impl Gamepad {
    pub fn new(sdl: &sdl2::Sdl) -> Result<Self, String> {
        let mut pad = Self {
            controller: None,
            subsystem: sdl.game_controller()?,
        };
        pad.refresh();
        Ok(pad)
    }

    pub fn refresh(&mut self) {
        if self.controller.as_ref().is_some_and(|pad| pad.attached()) {
            return;
        }
        if self.controller.take().is_some() {
            eprintln!("Gamepad disconnected; keyboard remains available");
        }
        for index in 0..self.subsystem.num_joysticks().unwrap_or(0) {
            if !self.subsystem.is_game_controller(index) {
                continue;
            }
            match self.subsystem.open(index) {
                Ok(pad) => {
                    eprintln!("Gamepad connected: {}", pad.name());
                    self.controller = Some(pad);
                    break;
                }
                Err(e) => eprintln!("Cannot open gamepad: {e}"),
            }
        }
    }

    pub fn buttons(&self) -> u16 {
        let Some(pad) = self.controller.as_ref().filter(|pad| pad.attached()) else {
            return 0x3ff;
        };
        mapping(
            |button| pad.button(button),
            pad.axis(Axis::LeftX),
            pad.axis(Axis::LeftY),
        )
    }
}

fn mapping(pressed: impl Fn(Button) -> bool, x: i16, y: i16) -> u16 {
    let mut mask = 0x3ff;
    for (bit, button) in [
        (0, Button::A),
        (1, Button::B),
        (2, Button::Back),
        (3, Button::Start),
        (4, Button::DPadRight),
        (5, Button::DPadLeft),
        (6, Button::DPadUp),
        (7, Button::DPadDown),
        (8, Button::RightShoulder),
        (9, Button::LeftShoulder),
    ] {
        if pressed(button) {
            mask &= !(1 << bit);
        }
    }
    if x > DEADZONE {
        mask &= !(1 << 4);
    }
    if x < -DEADZONE {
        mask &= !(1 << 5);
    }
    if y < -DEADZONE {
        mask &= !(1 << 6);
    }
    if y > DEADZONE {
        mask &= !(1 << 7);
    }
    mask
}

pub fn merge(keyboard: u16, gamepad: u16) -> u16 {
    let mut mask = keyboard & gamepad;
    // Opposite directions from different devices cancel each other.
    for pair in [(1 << 4) | (1 << 5), (1 << 6) | (1 << 7)] {
        if mask & pair == 0 {
            mask |= pair;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn virtual_controller_connect_input_disconnect() {
        let sdl = sdl2::init().unwrap();
        let mut pad = Gamepad::new(&sdl).unwrap();
        // Test only our virtual device, even if a physical pad is connected.
        pad.controller = None;
        unsafe {
            use sdl2::sys::*;
            let index = SDL_JoystickAttachVirtual(
                SDL_JoystickType::SDL_JOYSTICK_TYPE_GAMECONTROLLER,
                6,
                15,
                0,
            );
            assert!(index >= 0, "{}", sdl2::get_error());
            pad.controller = Some(pad.subsystem.open(index as u32).unwrap());
            let joystick = SDL_JoystickOpen(index);
            assert!(!joystick.is_null());
            assert_eq!(SDL_JoystickSetVirtualButton(joystick, 0, 1), 0);
            assert_eq!(SDL_JoystickSetVirtualAxis(joystick, 0, i16::MAX), 0);
            SDL_JoystickUpdate();
            assert_eq!(pad.buttons(), 0x3ff & !(1 | 16));
            assert_eq!(SDL_JoystickDetachVirtual(index), 0);
            assert_eq!(pad.buttons(), 0x3ff);
            SDL_JoystickClose(joystick);
            pad.refresh();
        }
    }
    #[test]
    fn neutral_drift_diagonals_and_extremes() {
        assert_eq!(mapping(|_| false, DEADZONE, -DEADZONE), 0x3ff);
        assert_eq!(
            mapping(|b| b == Button::B, i16::MAX, i16::MIN),
            0x3ff & !(2 | 16 | 64)
        );
        assert_eq!(mapping(|_| false, i16::MIN, i16::MAX), 0x3ff & !(32 | 128));
    }
    #[test]
    fn keyboard_and_pad_combine_without_opposite_directions() {
        let pad = mapping(
            |b| matches!(b, Button::A | Button::Start | Button::LeftShoulder),
            0,
            0,
        );
        assert_eq!(merge(0x3ff & !64, pad), 0x3ff & !(1 | 8 | 512 | 64));
        assert_eq!(merge(0x3ff & !16, mapping(|_| false, i16::MIN, 0)), 0x3ff);
        assert_eq!(merge(0x3ff, 0x3ff), 0x3ff);
    }
}
