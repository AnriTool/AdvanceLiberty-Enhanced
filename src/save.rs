//! Cartridge save persistence, not emulator save states.
use gba_core::Machine;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    name.into()
}

pub struct Save {
    path: PathBuf,
    lock_path: PathBuf,
    lock: Option<File>,
    committed: Vec<u8>,
    pending: Vec<u8>,
    stable_frames: u32,
}

impl Save {
    pub fn open(path: PathBuf, machine: &mut Machine) -> io::Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let lock_path = sidecar(&path, ".lock");
        let lock = OpenOptions::new().write(true).create_new(true).open(&lock_path)
            .map_err(|e| io::Error::new(e.kind(), format!("Cannot lock save {}: {e}. Close the other runtime; after a crash remove the .lock file only when no runtime is running.", path.display())))?;
        let mut save = Self {
            path,
            lock_path,
            lock: Some(lock),
            committed: Vec::new(),
            pending: Vec::new(),
            stable_frames: 0,
        };
        match fs::read(&save.path) {
            Ok(bytes) => {
                // GTA Advance uses EEPROM. Upstream stores the full 8 KiB array;
                // existing tools also export its active 512-byte region.
                if !matches!(bytes.len(), 512 | 8192) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Invalid save size {}: expected 512 or 8192 bytes; file left unchanged",
                            bytes.len()
                        ),
                    ));
                }
                machine.bus.load_save_data(&bytes);
                eprintln!("Loaded save: {}", save.path.display());
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                eprintln!("New save: {}", save.path.display())
            }
            Err(e) => return Err(e),
        }
        let bytes = machine
            .bus
            .save_data()
            .ok_or_else(|| io::Error::other("ROM has no cartridge save media"))?;
        save.committed = bytes.to_vec();
        save.pending = bytes.to_vec();
        Ok(save)
    }

    /// Wait one second of unchanged cartridge contents before writing, so a
    /// game's multi-block save transaction is not persisted halfway through.
    pub fn update(&mut self, machine: &Machine, exiting: bool) -> io::Result<()> {
        let bytes = machine
            .bus
            .save_data()
            .ok_or_else(|| io::Error::other("Save media disappeared"))?;
        if bytes != self.pending {
            self.pending.clear();
            self.pending.extend_from_slice(bytes);
            self.stable_frames = 0;
        } else {
            self.stable_frames = self.stable_frames.saturating_add(1);
        }
        if self.pending == self.committed || (!exiting && self.stable_frames < 60) {
            return Ok(());
        }
        // Keep the previous disk version before replacing the primary file.
        // Neither copy is ever truncated in place.
        match fs::read(&self.path) {
            Ok(old) => atomic_write(&sidecar(&self.path, ".bak"), &old)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        atomic_write(&self.path, &self.pending)?;
        self.committed.clone_from(&self.pending);
        eprintln!("Saved progress: {}", self.path.display());
        Ok(())
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = sidecar(path, ".tmp");
    let mut file = File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path)
}

impl Drop for Save {
    fn drop(&mut self) {
        drop(self.lock.take());
        let _ = fs::remove_file(&self.lock_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn machine() -> Machine {
        Machine::new(b"EEPROM_V".to_vec())
    }
    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "advance-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }
    #[test]
    fn persistence_backup_and_exclusive_access() {
        let dir = directory();
        let path = dir.join("progress.sav");
        let mut m = machine();
        let mut save = Save::open(path.clone(), &mut m).unwrap();
        assert!(Save::open(path.clone(), &mut machine()).is_err());
        m.bus.eeprom.as_mut().unwrap().data[0] = 42;
        save.update(&m, false).unwrap();
        assert!(!path.exists());
        for _ in 0..60 {
            save.update(&m, false).unwrap();
        }
        assert_eq!(fs::read(&path).unwrap()[0], 42);
        m.bus.eeprom.as_mut().unwrap().data[0] = 43;
        save.update(&m, true).unwrap();
        assert_eq!(fs::read(sidecar(&path, ".bak")).unwrap()[0], 42);
        drop(save);
        let mut restored = machine();
        let save = Save::open(path, &mut restored).unwrap();
        assert_eq!(restored.bus.save_data(), m.bus.save_data());
        drop(save);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn accepts_short_eeprom_and_rejects_corruption_without_overwrite() {
        let dir = directory();
        let path = dir.join("progress.sav");
        fs::write(&path, vec![7; 512]).unwrap();
        let mut m = machine();
        let save = Save::open(path.clone(), &mut m).unwrap();
        assert_eq!(m.bus.save_data().unwrap()[511], 7);
        assert_eq!(m.bus.save_data().unwrap()[512], 255);
        drop(save);
        fs::write(&path, [1, 2, 3]).unwrap();
        assert!(Save::open(path.clone(), &mut m).is_err());
        assert_eq!(fs::read(&path).unwrap(), [1, 2, 3]);
        assert!(!sidecar(&path, ".lock").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
