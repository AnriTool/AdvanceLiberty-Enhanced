use sdl2::keyboard::Scancode;
use std::{collections::HashSet, fs, io::Write, path::Path};

pub const DEFAULT: &str = "# SDL scancode names; physical key positions, independent of keyboard layout.\n# Separate alternatives with commas. P, Escape, F5, F6 and F11 are reserved.\n# Edit this file, then press F5 in the game to reload.\nA = Space, Z\nB = Left Shift, Right Shift, X\nSelect = Tab\nStart = Return\nRight = D, Right\nLeft = A, Left\nUp = W, Up\nDown = S, Down\nR = E\nL = Q\n";
const ACTIONS: [&str; 10] = [
    "A", "B", "Select", "Start", "Right", "Left", "Up", "Down", "R", "L",
];

pub struct Controls {
    keys: [Vec<Scancode>; 10],
}

impl Controls {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut keys: [Vec<Scancode>; 10] = Default::default();
        let mut actions = HashSet::new();
        let mut assigned = HashSet::new();
        for (line, raw) in text.lines().enumerate() {
            let raw = raw.split('#').next().unwrap().trim();
            if raw.is_empty() {
                continue;
            }
            let error = |message: &str| format!("Controls line {}: {message}", line + 1);
            let (action, names) = raw
                .split_once('=')
                .ok_or_else(|| error("expected action = keys"))?;
            let index = ACTIONS
                .iter()
                .position(|a| a.eq_ignore_ascii_case(action.trim()))
                .ok_or_else(|| error("unknown GBA button"))?;
            if !actions.insert(index) {
                return Err(error("duplicate GBA button"));
            }
            for name in names.split(',') {
                let key = Scancode::from_name(name.trim())
                    .ok_or_else(|| error("unknown or empty SDL key name"))?;
                if matches!(
                    key,
                    Scancode::P | Scancode::Escape | Scancode::F5 | Scancode::F6 | Scancode::F11
                ) {
                    return Err(error("P, Escape, F5, F6 and F11 are reserved"));
                }
                if !assigned.insert(key) {
                    return Err(error("key assigned more than once"));
                }
                keys[index].push(key);
            }
        }
        if actions.len() != 10 {
            return Err("Controls must define all ten GBA buttons".into());
        }
        Ok(Self { keys })
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
                .write_all(DEFAULT.as_bytes())
                .map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
        Self::load(path)
    }

    pub fn buttons(&self, pressed: impl Fn(Scancode) -> bool) -> u16 {
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
