//! Host menu clicks from the user's ROM, mixed independently of paused game audio.
use sdl2::audio::{AudioCallback, AudioDevice, AudioSpecDesired};
use std::{collections::VecDeque, sync::Arc};

pub fn load_click(rom: &[u8]) -> Result<Arc<[i16]>, String> {
    let word = |offset: usize| -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            rom.get(offset..offset + 4)
                .ok_or("Truncated menu sound")?
                .try_into()
                .unwrap(),
        ))
    };
    if rom.get(0xac..0xb0) != Some(b"BGTE") {
        return Err("Menu sound requires USA BGTE ROM".into());
    }
    // 08034db8: sound 0x105 -> PCM header in 08ca74dc[id - 201].
    let pointer = word(0xca74dc + (0x105 - 201) * 4)?;
    let offset = pointer
        .checked_sub(0x08000000)
        .ok_or("Invalid menu sound pointer")? as usize;
    if word(offset)? != 0 {
        return Err("Unsupported menu sound encoding".into());
    }
    let rate = word(offset + 4)? >> 10;
    let len = word(offset + 12)? as usize;
    if !(4000..=48000).contains(&rate) || !(1..=65536).contains(&len) {
        return Err("Invalid menu sound format".into());
    }
    let data = rom
        .get(offset + 16..offset + 16 + len)
        .ok_or("Truncated menu PCM")?;
    let frames = (len as u64 * 65536).div_ceil(rate as u64) as usize;
    // The game FIFO uses half volume (SOUNDCNT_H), yielding 32 i16 units
    // per signed PCM unit. The previous *128 gain made host clicks 4x louder.
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let value = data[(i as u64 * rate as u64 / 65536) as usize] as i8 as i16 * 32;
        samples.extend_from_slice(&[value, value]);
    }
    Ok(samples.into())
}
struct Voice {
    samples: Arc<[i16]>,
    position: usize,
    volume: u8,
}
#[derive(Default)]
struct Mixer {
    game: VecDeque<i16>,
    click: Option<Voice>,
    #[cfg(test)]
    clicks: u64,
}
impl Mixer {
    fn set_volume(&mut self, volume: u8) {
        if volume == 0 {
            self.click = None;
        } else if let Some(voice) = &mut self.click {
            voice.volume = volume.min(3);
        }
    }
    fn render(&mut self, out: &mut [i16]) {
        for sample in out {
            let mut value = self.game.pop_front().unwrap_or(0) as i32;
            if let Some(voice) = &mut self.click {
                // 080328cc uses ROM shifts [32, 2, 1, 0]: Off, 1/4, 1/2, full.
                let gain = [0, 1, 2, 4][voice.volume.min(3) as usize];
                value += voice.samples[voice.position] as i32 * gain / 4;
                voice.position += 1;
                if voice.position == voice.samples.len() {
                    self.click = None;
                }
            }
            *sample = value.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }
}
impl AudioCallback for Mixer {
    type Channel = i16;
    fn callback(&mut self, out: &mut [i16]) {
        self.render(out);
    }
}
pub struct Output {
    device: AudioDevice<Mixer>,
}
impl Output {
    pub fn open(subsystem: &sdl2::AudioSubsystem) -> Result<Self, String> {
        let device = subsystem.open_playback(
            None,
            &AudioSpecDesired {
                freq: Some(65536),
                channels: Some(2),
                samples: Some(2048),
            },
            |_| Mixer::default(),
        )?;
        device.resume();
        Ok(Self { device })
    }
    pub fn set_volume(&mut self, volume: u8) {
        self.device.lock().set_volume(volume);
    }
    pub fn clear(&mut self) {
        self.device.lock().game.clear();
    }
    pub fn size(&mut self) -> u32 {
        (self.device.lock().game.len() * 2) as u32
    }
    pub fn queue_audio(&mut self, samples: &[i16]) -> Result<(), String> {
        self.device.lock().game.extend(samples.iter().copied());
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn test_state(&mut self) -> (u64, usize) {
        let mixer = self.device.lock();
        (
            mixer.clicks,
            mixer
                .click
                .as_ref()
                .map_or(0, |v| v.samples.len() - v.position),
        )
    }
    pub fn click(&mut self, samples: Arc<[i16]>, volume: u8) {
        let mut mixer = self.device.lock();
        #[cfg(test)]
        if volume != 0 && !samples.is_empty() {
            mixer.clicks += 1;
        }
        mixer.click = if volume == 0 || samples.is_empty() {
            None
        } else {
            Some(Voice {
                samples,
                position: 0,
                volume: volume.min(3),
            })
        };
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paused_game_clear_preserves_click_and_resume_mixes_without_delaying_game() {
        let mut mixer = Mixer::default();
        mixer.game.extend([111, 222]);
        mixer.click = Some(Voice {
            samples: Arc::from([90, 90, 60, 60]),
            position: 0,
            volume: 3,
        });
        mixer.game.clear();
        let mut first = [0; 2];
        mixer.render(&mut first);
        assert_eq!(first, [90, 90]);
        mixer.game.extend([1000, -1000]);
        let mut resumed = [0; 4];
        mixer.render(&mut resumed);
        assert_eq!(resumed, [1060, -940, 0, 0]);
        assert!(mixer.click.is_none());
    }
    #[test]
    fn native_volume_steps_and_changes_apply_to_the_active_click() {
        for (level, expected) in [(0, 0), (1, 1000), (2, 2000), (3, 4000)] {
            let mut mixer = Mixer::default();
            mixer.click = Some(Voice {
                samples: Arc::from([4000, -4000]),
                position: 0,
                volume: level,
            });
            let mut out = [0; 2];
            mixer.render(&mut out);
            assert_eq!(out, [expected, -expected]);
        }
        let mut mixer = Mixer::default();
        mixer.click = Some(Voice {
            samples: Arc::from([4000; 8]),
            position: 0,
            volume: 3,
        });
        mixer.set_volume(1);
        let mut low = [0; 2];
        mixer.render(&mut low);
        assert_eq!(low, [1000; 2]);
        mixer.set_volume(2);
        let mut medium = [0; 2];
        mixer.render(&mut medium);
        assert_eq!(medium, [2000; 2]);
        mixer.set_volume(0);
        assert!(mixer.click.is_none());
        mixer.game.extend([123, 456]);
        let mut off = [0; 2];
        mixer.render(&mut off);
        assert_eq!(off, [123, 456]);
    }

    #[test]
    fn mixer_scales_sfx_and_saturates_mixed_samples() {
        let mut mixer = Mixer::default();
        mixer.game.extend([32760, -32760]);
        mixer.click = Some(Voice {
            samples: Arc::from([300, -300]),
            position: 0,
            volume: 1,
        });
        let mut out = [0; 2];
        mixer.render(&mut out);
        assert_eq!(out, [32767, -32768]);
    }
    #[test]
    fn pcm_decoder_uses_signed_samples_rate_and_rejects_bad_assets() {
        let mut rom = vec![0; 0xca74dc + 300];
        rom[0xac..0xb0].copy_from_slice(b"BGTE");
        rom[0xca74dc + 60 * 4..0xca74dc + 60 * 4 + 4].copy_from_slice(&0x08000100u32.to_le_bytes());
        rom[0x104..0x108].copy_from_slice(&(16384u32 << 10).to_le_bytes());
        rom[0x10c..0x110].copy_from_slice(&2u32.to_le_bytes());
        rom[0x110..0x112].copy_from_slice(&[127, 128]);
        let click = load_click(&rom).unwrap();
        assert_eq!(click.len(), 16);
        assert!(click[..8].iter().all(|v| *v == 4064));
        assert!(click[8..].iter().all(|v| *v == -4096));
        rom[0x100] = 1;
        assert!(load_click(&rom).is_err());
        assert!(load_click(&[]).is_err());
    }
}
