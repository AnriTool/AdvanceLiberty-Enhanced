use crate::control_context::Context;
use sdl2::keyboard::Scancode;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::Path,
};

pub const DEFAULT: &str = "# SDL scancode names; physical key positions, independent of keyboard layout.\n# Separate alternatives with commas. P, Escape, F5, F6, F8 and F11 are reserved.\n# Edit this file, then press F5 in the game to reload.\nA = Space, Z\nB = Left Shift, Right Shift, X\nSelect = Tab\nStart = Return\nRight = D, Right\nLeft = A, Left\nUp = W, Up\nDown = S, Down\nR = E\nL = Q\n";
const ACTIONS: [&str; 10] = [
    "A", "B", "Select", "Start", "Right", "Left", "Up", "Down", "R", "L",
];

pub const ENHANCED_DEFAULT: &str = "[OnFoot]\nUp = W, Up\nDown = S, Down\nLeft = A, Left\nRight = D, Right\nFire = J, MouseLeft\nSprint = Left Shift, Right Shift\nJump = Space\nEnterVehicle = F\nChangeWeapon = Tab\nPause = Return\nAuxiliaryR = H\nAim = MouseRight\n\n[Vehicle]\nAccelerate = W, Up\nBrakeReverse = S, Down\nSteerLeft = A, Left\nSteerRight = D, Right\nParkingBrake = Space\nHorn = Left Shift, Right Shift\nSpecialMission = CapsLock\nTurretModifier = Left Ctrl, Right Ctrl\nExitVehicle = F\nPause = Return\nChangeWeapon = Tab\n";
const FOOT: [&str; 12] = [
    "Up",
    "Down",
    "Left",
    "Right",
    "Fire",
    "Sprint",
    "Jump",
    "EnterVehicle",
    "ChangeWeapon",
    "Pause",
    "AuxiliaryR",
    "Aim",
];
const VEHICLE: [&str; 11] = [
    "Accelerate",
    "BrakeReverse",
    "SteerLeft",
    "SteerRight",
    "ParkingBrake",
    "Horn",
    "SpecialMission",
    "TurretModifier",
    "ExitVehicle",
    "Pause",
    "ChangeWeapon",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Binding {
    Key(Scancode),
    MouseLeft,
    MouseMiddle,
    MouseRight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Classic,
    Enhanced,
}

#[derive(Clone)]
pub struct Controls {
    keys: [Vec<Binding>; 10],
    mode: Mode,
    mouse_aim: bool,
    foot: [Vec<Binding>; 12],
    vehicle: [Vec<Binding>; 11],
    shortcuts: [Vec<Binding>; 1],
}

#[derive(Default)]
pub struct InputLatch {
    previous_context: Option<(Context, Mode)>,
    previous_held: HashSet<Binding>,
    suppressed: HashSet<Binding>,
}
impl InputLatch {
    pub fn aiming(&self, controls: &Controls, context: Context) -> bool {
        controls.mouse_aim()
            && context == Context::OnFoot
            && controls.foot[11]
                .iter()
                .any(|key| self.previous_held.contains(key) && !self.suppressed.contains(key))
    }
    pub fn sample(
        &mut self,
        controls: &Controls,
        context: Context,
        pressed: impl Fn(Binding) -> bool,
    ) -> u16 {
        let held: HashSet<_> = controls
            .keys
            .iter()
            .chain(controls.foot.iter())
            .chain(controls.vehicle.iter())
            .flatten()
            .copied()
            .filter(|&key| pressed(key))
            .collect();
        self.suppressed.retain(|key| held.contains(key));
        if controls.mode == Mode::Enhanced {
            if let Some((old_context, old_mode)) = self.previous_context {
                if (old_context, old_mode) != (context, controls.mode) {
                    let movement = |ctx, key| -> Option<usize> {
                        let groups = match ctx {
                            Context::OnFoot => [
                                &controls.foot[0],
                                &controls.foot[1],
                                &controls.foot[2],
                                &controls.foot[3],
                            ],
                            Context::Vehicle => [
                                &controls.vehicle[0],
                                &controls.vehicle[1],
                                &controls.vehicle[2],
                                &controls.vehicle[3],
                            ],
                            Context::Menu => return None,
                        };
                        groups.iter().position(|group| group.contains(key))
                    };
                    for key in held.intersection(&self.previous_held) {
                        let continuous_movement = movement(old_context, key).is_some()
                            && movement(old_context, key) == movement(context, key);
                        if !continuous_movement {
                            self.suppressed.insert(*key);
                        }
                    }
                }
            }
        } else {
            self.suppressed.clear();
        }
        self.previous_context = Some((context, controls.mode));
        self.previous_held = held;
        controls.input(context, |key| {
            self.previous_held.contains(&key) && !self.suppressed.contains(&key)
        })
    }
}
fn binding(names: &str, line: usize, optional: bool) -> Result<Vec<Binding>, String> {
    if optional && names.trim().eq_ignore_ascii_case("None") {
        return Ok(Vec::new());
    }
    let mut keys = Vec::new();
    for name in names.split(',') {
        let key =
            match name.trim().to_ascii_lowercase().as_str() {
                "mouseleft" | "lmb" => Binding::MouseLeft,
                "mousemiddle" | "mmb" => Binding::MouseMiddle,
                "mouseright" | "rmb" => Binding::MouseRight,
                _ => Binding::Key(Scancode::from_name(name.trim()).ok_or_else(|| {
                    format!("Controls line {line}: unknown or empty SDL key name")
                })?),
            };
        if matches!(
            key,
            Binding::Key(
                Scancode::P
                    | Scancode::Escape
                    | Scancode::F5
                    | Scancode::F6
                    | Scancode::F8
                    | Scancode::F11
                    | Scancode::F12
            )
        ) {
            return Err(format!(
                "Controls line {line}: P, Escape, F5, F6, F8, F11 and F12 are reserved"
            ));
        }
        if keys.contains(&key) {
            return Err(format!("Controls line {line}: duplicate key"));
        }
        keys.push(key);
    }
    Ok(keys)
}
fn validate_profile(
    section: &str,
    actions: &[&str],
    bindings: &[Vec<Binding>],
) -> Result<(), String> {
    let mut assigned = HashMap::new();
    for (action, keys) in actions.iter().zip(bindings) {
        for &key in keys {
            if let Some(previous) = assigned.insert(key, action) {
                let name = match key {
                    Binding::Key(code) => code.name(),
                    Binding::MouseLeft => "MouseLeft",
                    Binding::MouseMiddle => "MouseMiddle",
                    Binding::MouseRight => "MouseRight",
                };
                return Err(format!("Controls [{section}]: key '{name}' is assigned to both '{previous}' and '{action}'"));
            }
        }
    }
    Ok(())
}
impl Controls {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut keys: [Vec<Binding>; 10] = Default::default();
        let mut foot = [
            "W, Up",
            "S, Down",
            "A, Left",
            "D, Right",
            "J, MouseLeft",
            "Left Shift, Right Shift",
            "Space",
            "F",
            "Tab",
            "Return",
            "H",
            "MouseRight",
        ]
        .map(|s| binding(s, 0, true).unwrap());
        let mut vehicle = [
            "W, Up",
            "S, Down",
            "A, Left",
            "D, Right",
            "Space",
            "Left Shift, Right Shift",
            "CapsLock",
            "Left Ctrl, Right Ctrl",
            "F",
            "Return",
            "Tab",
        ]
        .map(|s| binding(s, 0, true).unwrap());
        let mut mode = Mode::Classic; // Existing ten-button files retain their original behavior.
        let mut mouse_aim = false;
        let mut shortcuts = [vec![Binding::Key(Scancode::V)]];
        let mut seen = HashSet::new();
        let mut section = 0usize;
        for (line, raw) in text.lines().enumerate() {
            let raw = raw.split('#').next().unwrap().trim();
            if raw.is_empty() {
                continue;
            }
            let error = |message: &str| format!("Controls line {}: {message}", line + 1);
            if raw.starts_with('[') && raw.ends_with(']') {
                section = match raw[1..raw.len() - 1].to_ascii_lowercase().as_str() {
                    "gba" => 0,
                    "onfoot" => 1,
                    "vehicle" => 2,
                    "shortcuts" => 3,
                    _ => return Err(error("unknown section (GBA, OnFoot, Vehicle, Shortcuts)")),
                };
                continue;
            }
            let (action, names) = raw
                .split_once('=')
                .ok_or_else(|| error("expected action = keys"))?;
            let action = action.trim();
            let identity = (section, action.to_ascii_lowercase());
            if !seen.insert(identity) {
                return Err(error("duplicate action"));
            }
            if action.eq_ignore_ascii_case("Mode") && section == 0 {
                mode = match names.trim().to_ascii_lowercase().as_str() {
                    "classic" => Mode::Classic,
                    "enhanced" => Mode::Enhanced,
                    _ => return Err(error("Mode must be Classic or Enhanced")),
                };
                continue;
            }
            if action.eq_ignore_ascii_case("MouseAim") && section == 0 {
                mouse_aim = match names.trim().to_ascii_lowercase().as_str() {
                    "true" => true,
                    "false" => false,
                    _ => return Err(error("MouseAim must be true or false")),
                };
                continue;
            }
            let actions = match section {
                0 => ACTIONS.as_slice(),
                1 => FOOT.as_slice(),
                3 => &["CycleCamera"],
                _ => VEHICLE.as_slice(),
            };
            let index = actions
                .iter()
                .position(|a| a.eq_ignore_ascii_case(action))
                .ok_or_else(|| error("unknown action in this section"))?;
            let values = binding(names, line + 1, section != 0)?;
            match section {
                0 => keys[index] = values,
                1 => foot[index] = values,
                3 => shortcuts[index] = values,
                _ => vehicle[index] = values,
            };
        }
        if keys.iter().any(Vec::is_empty) {
            return Err("Controls must define all ten GBA buttons".into());
        }
        // Older profiles could already use RMB for another action. Do not
        // reject or change that binding when adding a new optional action.
        if !seen.contains(&(1, "aim".to_owned()))
            && foot[..11].iter().any(|b| b.contains(&Binding::MouseRight))
        {
            foot[11].clear();
        }
        validate_profile("GBA", &ACTIONS, &keys)?;
        validate_profile("OnFoot", &FOOT, &foot)?;
        validate_profile("Vehicle", &VEHICLE, &vehicle)?;
        // Preserve old customized profiles which already use V for gameplay.
        let gameplay = keys.iter().chain(foot.iter()).chain(vehicle.iter());
        if !seen.contains(&(3, "cyclecamera".to_owned()))
            && gameplay
                .clone()
                .any(|b| b.contains(&Binding::Key(Scancode::V)))
        {
            shortcuts[0].clear();
        }
        if shortcuts[0]
            .iter()
            .any(|key| gameplay.clone().any(|b| b.contains(key)))
        {
            return Err("Camera shortcut conflicts with a gameplay binding".into());
        }
        Ok(Self {
            keys,
            mode,
            mouse_aim,
            foot,
            vehicle,
            shortcuts,
        })
    }

    pub fn actions(section: usize) -> &'static [&'static str] {
        match section {
            0 => &ACTIONS,
            1 => &FOOT,
            3 => &["CycleCamera"],
            _ => &VEHICLE,
        }
    }
    pub fn bindings(&self, section: usize, row: usize) -> &[Binding] {
        match section {
            0 => &self.keys[row],
            1 => &self.foot[row],
            3 => &self.shortcuts[row],
            _ => &self.vehicle[row],
        }
    }
    pub fn binding_name(key: Binding) -> &'static str {
        match key {
            Binding::Key(code) => code.name(),
            Binding::MouseLeft => "MouseLeft",
            Binding::MouseMiddle => "MouseMiddle",
            Binding::MouseRight => "MouseRight",
        }
    }
    pub fn binding_text(&self, section: usize, row: usize) -> String {
        let names: Vec<_> = self
            .bindings(section, row)
            .iter()
            .map(|&b| Self::binding_name(b))
            .collect();
        if names.is_empty() {
            "None".into()
        } else {
            names.join(", ")
        }
    }
    pub fn encode(&self) -> String {
        let mut text = format!(
            "# Native options; SDL physical key names.\nMode = {:?}\nMouseAim = {}\n",
            self.mode, self.mouse_aim
        );
        for (section, name) in ["GBA", "OnFoot", "Vehicle", "Shortcuts"].iter().enumerate() {
            text.push_str(&format!("\n[{name}]\n"));
            for (row, action) in Self::actions(section).iter().enumerate() {
                text.push_str(&format!("{action} = {}\n", self.binding_text(section, row)));
            }
        }
        text
    }
    // Validate a complete candidate before changing any profile. In particular,
    // reserved keys and conflicts use exactly the same rules as file loading.
    pub fn rebind(
        &mut self,
        section: usize,
        row: usize,
        key: Option<Binding>,
        add: bool,
    ) -> Result<(), String> {
        if section > 3 || row >= Self::actions(section).len() {
            return Err("Unknown action".into());
        }
        let mut candidate = self.clone();
        let keys = match section {
            0 => &mut candidate.keys[row],
            1 => &mut candidate.foot[row],
            3 => &mut candidate.shortcuts[row],
            _ => &mut candidate.vehicle[row],
        };
        if !add {
            keys.clear();
        }
        if let Some(key) = key {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        *self = Self::parse(&candidate.encode())?;
        Ok(())
    }
    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn cycles_camera(&self, key: Binding) -> bool {
        self.shortcuts[0].contains(&key)
    }
    pub fn mouse_aim(&self) -> bool {
        self.mode == Mode::Enhanced && self.mouse_aim
    }
    pub fn mouse_aim_setting(&self) -> bool {
        self.mouse_aim
    }
    pub fn toggle_mouse_aim(&mut self) {
        self.mouse_aim = !self.mouse_aim;
    }
    pub fn toggle_mode(&mut self) -> Result<(), String> {
        validate_profile("OnFoot", &FOOT, &self.foot)?;
        validate_profile("Vehicle", &VEHICLE, &self.vehicle)?;
        self.mode = match self.mode {
            Mode::Classic => Mode::Enhanced,
            Mode::Enhanced => Mode::Classic,
        };
        Ok(())
    }
    pub fn default_file() -> String {
        format!("Mode = Enhanced\n\n[GBA]\n{DEFAULT}\n{ENHANCED_DEFAULT}\n[Shortcuts]\nCycleCamera = V\n")
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text)
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => file
                .write_all(Self::default_file().as_bytes())
                .map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
        Self::load(path)
    }

    #[cfg(test)]
    pub fn buttons(&self, pressed: impl Fn(Scancode) -> bool) -> u16 {
        self.original_buttons(|binding| match binding {
            Binding::Key(key) => pressed(key),
            _ => false,
        })
    }
    fn original_buttons(&self, pressed: impl Fn(Binding) -> bool) -> u16 {
        self.keys
            .iter()
            .enumerate()
            .fold(0x3ff, |mask, (bit, keys)| {
                if keys.iter().any(|&key| pressed(key)) {
                    mask & !(1 << bit)
                } else {
                    mask
                }
            })
    }
    #[cfg(test)]
    pub fn contextual_buttons(&self, context: Context, pressed: impl Fn(Scancode) -> bool) -> u16 {
        self.input(context, |binding| match binding {
            Binding::Key(key) => pressed(key),
            _ => false,
        })
    }
    pub fn input(&self, context: Context, pressed: impl Fn(Binding) -> bool) -> u16 {
        if self.mode == Mode::Classic || context == Context::Menu {
            return self.original_buttons(pressed);
        }
        let held = |keys: &[Binding]| keys.iter().any(|&key| pressed(key));
        let mut buttons = 0x3ff;
        match context {
            Context::OnFoot => {
                let sprinting = held(&self.foot[5]);
                for (i, mask) in [64, 128, 32, 16, 1, 2, 3, 512, 4, 8, 256]
                    .into_iter()
                    .enumerate()
                {
                    // Fire must not create the original A+B jump while running.
                    // The dedicated Jump action still deliberately sends A+B.
                    if held(&self.foot[i]) && !(i == 4 && sprinting) {
                        buttons &= !mask;
                    }
                }
            }
            Context::Vehicle => {
                // Horn = Up+R, mission = L+R. All physical keys belong to this profile.
                for (i, mask) in [1, 2, 32, 16, 256, 64 | 256, 512 | 256, 0, 512, 8, 4]
                    .into_iter()
                    .enumerate()
                {
                    if held(&self.vehicle[i]) {
                        buttons &= !mask;
                    }
                }
                if held(&self.vehicle[7]) && (held(&self.vehicle[2]) || held(&self.vehicle[3])) {
                    buttons &= !3;
                }
            }
            Context::Menu => unreachable!(),
        }
        buttons
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn camera_shortcut_defaults_migrates_and_rejects_conflicts() {
        use super::*;
        let v = Binding::Key(Scancode::V);
        let c = Binding::Key(Scancode::C);
        let mut controls = Controls::parse(DEFAULT).unwrap();
        assert!(controls.cycles_camera(v));
        controls.rebind(3, 0, Some(c), false).unwrap();
        assert!(controls.cycles_camera(c));
        assert!(!controls.cycles_camera(v));
        assert!(Controls::parse(&controls.encode())
            .unwrap()
            .cycles_camera(c));
        let before = controls.encode();
        for key in [Scancode::W, Scancode::F12] {
            assert!(controls
                .rebind(3, 0, Some(Binding::Key(key)), false)
                .is_err());
            assert_eq!(controls.encode(), before);
        }
        assert!(controls.rebind(1, 6, Some(c), false).is_err());
        controls.rebind(3, 0, None, false).unwrap();
        assert_eq!(controls.binding_text(3, 0), "None");
        let legacy = DEFAULT.replace("L = Q", "L = V");
        let migrated = Controls::parse(&legacy).unwrap();
        assert!(!migrated.cycles_camera(v));
        assert!(Controls::parse(&format!("{legacy}\n[Shortcuts]\nCycleCamera = V")).is_err());
    }
    use super::*;
    #[test]
    fn aim_is_a_remappable_hold_without_gba_buttons_and_requires_release_across_contexts() {
        let mut c = Controls::parse(&Controls::default_file()).unwrap();
        c.toggle_mouse_aim();
        let mut latch = InputLatch::default();
        assert_eq!(latch.sample(&c, Context::OnFoot, |_| false), 0x3ff);
        assert!(!latch.aiming(&c, Context::OnFoot));
        assert_eq!(
            latch.sample(&c, Context::OnFoot, |b| b == Binding::MouseRight),
            0x3ff
        );
        assert!(latch.aiming(&c, Context::OnFoot));
        assert_eq!(
            latch.sample(&c, Context::OnFoot, |b| matches!(
                b,
                Binding::MouseRight | Binding::MouseLeft
            )),
            0x3fe
        );
        latch.sample(&c, Context::Menu, |b| b == Binding::MouseRight);
        latch.sample(&c, Context::OnFoot, |b| b == Binding::MouseRight);
        assert!(!latch.aiming(&c, Context::OnFoot));
        latch.sample(&c, Context::OnFoot, |_| false);
        c.rebind(1, 11, Some(Binding::Key(Scancode::G)), false)
            .unwrap();
        latch.sample(&c, Context::OnFoot, |b| b == Binding::Key(Scancode::G));
        assert!(latch.aiming(&c, Context::OnFoot));
        assert!(!latch.aiming(&c, Context::Vehicle));
        c.toggle_mode().unwrap();
        assert!(!latch.aiming(&c, Context::OnFoot));
    }
    #[test]
    fn aim_setting_roundtrips_without_changing_buttons_and_classic_disables_it() {
        let mut controls = Controls::parse(&Controls::default_file()).unwrap();
        assert!(!controls.mouse_aim());
        let held = |b| matches!(b, Binding::Key(Scancode::W | Scancode::J));
        let old = controls.input(Context::OnFoot, held);
        controls.toggle_mouse_aim();
        assert!(controls.mouse_aim());
        assert_eq!(controls.input(Context::OnFoot, held), old);
        let mut reloaded = Controls::parse(&controls.encode()).unwrap();
        assert!(reloaded.mouse_aim());
        reloaded.toggle_mode().unwrap();
        assert!(!reloaded.mouse_aim());
        assert!(reloaded.mouse_aim_setting());
        assert!(
            Controls::parse(&format!("MouseAim = unknown\n{}", Controls::default_file())).is_err()
        );
    }
    #[test]
    fn sprint_ignores_fire_but_preserves_explicit_jump_and_classic_combinations() {
        let mut controls = Controls::parse(&Controls::default_file()).unwrap();
        for fire in [Binding::Key(Scancode::J), Binding::MouseLeft] {
            assert_eq!(controls.input(Context::OnFoot, |b| b == fire), 0x3fe);
            assert_eq!(
                controls.input(Context::OnFoot, |b| b == fire
                    || b == Binding::Key(Scancode::LShift)),
                0x3fd
            );
            assert_eq!(
                controls.input(Context::OnFoot, |b| b == fire
                    || matches!(b, Binding::Key(Scancode::LShift | Scancode::Space))),
                0x3fc
            );
        }
        let remapped = Controls::parse(
            &Controls::default_file()
                .replace("Aim = MouseRight\n", "")
                .replace("Fire = J, MouseLeft", "Fire = K, MouseRight")
                .replace("Sprint = Left Shift, Right Shift", "Sprint = C"),
        )
        .unwrap();
        assert_eq!(
            remapped.input(Context::OnFoot, |b| matches!(
                b,
                Binding::Key(Scancode::K | Scancode::C) | Binding::MouseRight
            )),
            0x3fd
        );
        assert_eq!(
            controls
                .contextual_buttons(Context::Vehicle, |k| matches!(k, Scancode::W | Scancode::S)),
            0x3fc
        );
        controls.toggle_mode().unwrap();
        assert_eq!(
            controls.contextual_buttons(Context::OnFoot, |k| matches!(
                k,
                Scancode::Space | Scancode::LShift
            )),
            0x3fc
        );
    }
    #[test]
    fn gba_layout_is_independent_of_modern_actions_and_diagnostics_name_conflicts() {
        let classic = "A = Z\nB = X\nSelect = Left Shift\nStart = Return\nRight = Right\nLeft = Left\nUp = Up\nDown = Down\nR = S\nL = A\n";
        let mut controls = Controls::parse(&format!(
            "Mode = Enhanced\n[GBA]\n{classic}\n{ENHANCED_DEFAULT}"
        ))
        .unwrap();
        assert_eq!(
            controls.contextual_buttons(Context::OnFoot, |k| k == Scancode::S),
            0x3ff & !128
        );
        assert_eq!(
            controls.contextual_buttons(Context::Vehicle, |k| k == Scancode::S),
            0x3ff & !2
        );
        assert_eq!(
            controls.contextual_buttons(Context::Vehicle, |k| k == Scancode::LShift),
            0x3ff & !(64 | 256)
        );
        assert_eq!(
            controls.contextual_buttons(Context::Menu, |k| k == Scancode::S),
            0x3ff & !256
        );
        controls.toggle_mode().unwrap();
        assert_eq!(
            controls.contextual_buttons(Context::Vehicle, |k| k == Scancode::S),
            0x3ff & !256
        );
        assert_eq!(
            controls.contextual_buttons(Context::OnFoot, |k| k == Scancode::LShift),
            0x3ff & !4
        );
        let error = Controls::parse(&Controls::default_file().replace("Jump = Space", "Jump = J"))
            .err()
            .unwrap();
        for detail in ["OnFoot", "J", "Fire", "Jump"] {
            assert!(error.contains(detail), "{error}");
        }
    }
    #[test]
    fn entering_car_keeps_movement_but_requires_new_press_for_changed_actions() {
        let controls = Controls::parse(&Controls::default_file()).unwrap();
        let mut latch = InputLatch::default();
        let held = |binding| {
            matches!(
                binding,
                Binding::Key(Scancode::W | Scancode::LShift | Scancode::Space | Scancode::F)
            )
        };
        assert_eq!(
            latch.sample(&controls, Context::OnFoot, held),
            0x3ff & !(64 | 3 | 512)
        );
        assert_eq!(latch.sample(&controls, Context::Vehicle, held), 0x3fe); // W keeps accelerating; no horn, handbrake or exit.
        assert_eq!(latch.sample(&controls, Context::Vehicle, |_| false), 0x3ff);
        assert_eq!(
            latch.sample(&controls, Context::Vehicle, |b| b
                == Binding::Key(Scancode::LShift)),
            0x3ff & !(64 | 256)
        );
        let mut menu = InputLatch::default();
        let w = |b| b == Binding::Key(Scancode::W);
        assert_eq!(menu.sample(&controls, Context::Vehicle, w), 0x3fe);
        assert_eq!(menu.sample(&controls, Context::Menu, w), 0x3ff);
        assert_eq!(menu.sample(&controls, Context::Menu, |_| false), 0x3ff);
        assert_eq!(menu.sample(&controls, Context::Menu, w), 0x3bf);
    }
    #[test]
    fn modern_actions_follow_context_and_keep_menu_and_classic_masks() {
        let mut controls = Controls::parse(&Controls::default_file()).unwrap();
        let one =
            |controls: &Controls, context, key| controls.contextual_buttons(context, |k| k == key);
        for (key, foot, vehicle) in [
            (Scancode::W, 64, 1),
            (Scancode::S, 128, 2),
            (Scancode::A, 32, 32),
            (Scancode::D, 16, 16),
            (Scancode::Space, 3, 256),
            (Scancode::LShift, 2, 64 | 256),
            (Scancode::J, 1, 0),
            (Scancode::F, 512, 512),
            (Scancode::Tab, 4, 4),
            (Scancode::CapsLock, 0, 512 | 256),
        ] {
            assert_eq!(
                one(&controls, Context::OnFoot, key),
                0x3ff & !foot,
                "foot {key:?}"
            );
            assert_eq!(
                one(&controls, Context::Vehicle, key),
                0x3ff & !vehicle,
                "vehicle {key:?}"
            );
            assert_eq!(
                one(&controls, Context::Menu, key),
                controls.buttons(|k| k == key)
            );
        }
        assert_eq!(
            controls.contextual_buttons(Context::Vehicle, |k| matches!(
                k,
                Scancode::LCtrl | Scancode::A
            )),
            0x3ff & !(3 | 32)
        );
        assert_eq!(one(&controls, Context::Vehicle, Scancode::LCtrl), 0x3ff);
        controls.toggle_mode().unwrap();
        assert_eq!(controls.mode(), Mode::Classic);
        for context in [Context::Menu, Context::OnFoot, Context::Vehicle] {
            assert_eq!(one(&controls, context, Scancode::Space), 0x3fe);
        }
        controls.toggle_mode().unwrap();
        assert_eq!(one(&controls, Context::OnFoot, Scancode::Space), 0x3fc);
    }
    #[test]
    fn mouse_fire_and_independent_remaps_support_unbinding_and_chords() {
        let text = Controls::default_file()
            .replace("Aim = MouseRight\n", "")
            .replace("Fire = J, MouseLeft", "Fire = K, MouseRight")
            .replace("Jump = Space", "Jump = C")
            .replace("Accelerate = W, Up", "Accelerate = I")
            .replace("Horn = Left Shift, Right Shift", "Horn = None");
        let controls = Controls::parse(&text).unwrap();
        assert_eq!(
            controls.input(Context::OnFoot, |b| b == Binding::MouseRight),
            0x3fe
        );
        assert_eq!(
            controls.input(Context::OnFoot, |b| b == Binding::MouseLeft),
            0x3ff
        );
        assert_eq!(
            controls.input(Context::Vehicle, |b| b == Binding::MouseRight),
            0x3ff
        );
        assert_eq!(
            controls.contextual_buttons(Context::OnFoot, |k| k == Scancode::C),
            0x3fc
        );
        assert_eq!(
            controls.contextual_buttons(Context::Vehicle, |k| k == Scancode::I),
            0x3fe
        );
        assert_eq!(
            controls.contextual_buttons(Context::Vehicle, |k| k == Scancode::W),
            0x3ff
        );
        assert_eq!(
            controls.contextual_buttons(Context::OnFoot, |k| k == Scancode::W),
            0x3bf
        );
        assert_eq!(
            controls.contextual_buttons(Context::Vehicle, |k| k == Scancode::LShift),
            0x3ff
        );
    }
    #[test]
    fn enhanced_sections_reject_conflicts_but_old_custom_gba_files_still_load() {
        for invalid in [
            Controls::default_file().replace("Jump = Space", "Jump = J"),
            Controls::default_file().replace("Fire = J, MouseLeft", "Fire = F8"),
            Controls::default_file().replace("Fire = J, MouseLeft", "Fire = MouseLeft, LMB"),
            Controls::default_file().replace("[Vehicle]", "[Vehicl]"),
            Controls::default_file().replace("BrakeReverse = S, Down", "BrakeReverse = W"),
        ] {
            assert!(Controls::parse(&invalid).is_err(), "{invalid}");
        }
        let mut old = Controls::parse(&DEFAULT.replace("R = E", "R = J")).unwrap();
        assert_eq!(old.mode(), Mode::Classic);
        assert_eq!(
            old.contextual_buttons(Context::Vehicle, |k| k == Scancode::J),
            0x2ff
        );
        old.toggle_mode().unwrap();
        assert_eq!(old.mode(), Mode::Enhanced);
        assert_eq!(
            old.contextual_buttons(Context::OnFoot, |k| k == Scancode::J),
            0x3fe
        );
    }
    #[test]
    fn remapping_changes_only_requested_button() {
        let controls = Controls::parse(&DEFAULT.replace("Space, Z", "C")).unwrap();
        assert_eq!(controls.buttons(|k| k == Scancode::Space), 0x3ff);
        assert_eq!(
            controls.buttons(|k| matches!(k, Scancode::C | Scancode::W | Scancode::LShift)),
            0x3ff & !(1 | 64 | 2)
        );
    }
    #[test]
    fn rejects_conflicts_incomplete_and_reserved_bindings() {
        for bad in [
            DEFAULT.replace("Space, Z", "P"),
            DEFAULT.replace("Space, Z", "F11"),
            DEFAULT.replace("Space, Z", "F6"),
            DEFAULT.replace("Space, Z", "W"),
            DEFAULT.replace("Space, Z", "NoSuchKey"),
            DEFAULT.replace("L = Q", ""),
            format!("{DEFAULT}\nA = C"),
            DEFAULT.replace("Space, Z", ""),
        ] {
            assert!(Controls::parse(&bad).is_err(), "{bad}");
        }
    }
}
