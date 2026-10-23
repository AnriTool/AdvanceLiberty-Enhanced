//! Color unit matching the local GBA PPU, including its six-bit green lane.
#[derive(Clone, Copy)]
pub struct Pixel {
    pub color: u16,
    pub layer: u8,
    pub semi: bool,
}
fn half(io: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(io[o..o + 2].try_into().unwrap())
}
/// Presentation-only replacement for the bridge's alternating OBJ priority.
pub fn bridge_translucency(object: u16, background: u16) -> u16 {
    [0, 5, 10].into_iter().fold(0, |color, shift| {
        color | (((((object >> shift) & 31) + ((background >> shift) & 31)) / 2) << shift)
    })
}
pub struct Effects {
    control: u16,
    mode: u16,
    eva: u32,
    evb: u32,
    evy: u32,
}
impl Effects {
    pub fn new(io: &[u8]) -> Self {
        let control = half(io, 0x50);
        Self {
            control,
            mode: (control >> 6) & 3,
            eva: (half(io, 0x52) & 31).min(16) as u32,
            evb: ((half(io, 0x52) >> 8) & 31).min(16) as u32,
            evy: (half(io, 0x54) & 31).min(16) as u32,
        }
    }
    pub fn inactive(&self, has_semi: bool) -> bool {
        let first = self.control & 0x3f != 0;
        let second = self.control & 0x3f00 != 0;
        !(second && (has_semi || (self.mode == 1 && first))) && !(self.mode >= 2 && first)
    }
    #[inline]
    #[cfg(test)]
    pub fn apply(&self, front: Pixel, back: Pixel) -> u16 {
        self.apply_window(front, back, true)
    }
    #[inline]
    pub fn apply_window(&self, front: Pixel, back: Pixel, allowed: bool) -> u16 {
        let control = self.control;
        let mode = self.mode;
        let alpha = (front.semi || (allowed && mode == 1 && control & (1 << front.layer) != 0))
            && control & (0x100 << back.layer) != 0;
        let brightness = if allowed && control & (1 << front.layer) != 0 {
            mode
        } else {
            0
        };
        if !alpha && brightness < 2 {
            return front.color & 0x7fff;
        }
        if alpha {
            if self.eva == 0 && self.evb == 16 {
                return back.color & 0x7fff;
            }
            if self.eva == 16 && self.evb == 0 {
                return front.color & 0x7fff;
            }
        }
        let (eva, evb, evy) = (self.eva, self.evb, self.evy);
        let channels = |c: u16| {
            [
                (c & 31) as u32,
                ((c >> 5) & 31) as u32 * 2,
                ((c >> 10) & 31) as u32,
            ]
        };
        let a = channels(front.color);
        let b = channels(back.color);
        let c: [u32; 3] = std::array::from_fn(|i| {
            let max = if i == 1 { 63 } else { 31 };
            if alpha {
                ((a[i] * eva + b[i] * evb + 8) >> 4).min(max)
            } else if brightness == 2 {
                a[i] + (((max - a[i]) * evy + 8) >> 4)
            } else {
                a[i] - ((a[i] * evy + 7) >> 4)
            }
        });
        (c[0] | ((c[1] >> 1) << 5) | (c[2] << 10)) as u16
    }
}
#[cfg(test)]
fn apply(io: &[u8], front: Pixel, back: Pixel) -> u16 {
    Effects::new(io).apply(front, back)
}
#[cfg(test)]
mod tests {
    use super::*;
    use gba_core::{ppu, MemMap};
    #[test]
    fn inactive_guard_preserves_colors_for_all_target_masks_and_semis() {
        let mut io = vec![0u8; 0x56];
        io[0x52..0x54].copy_from_slice(&0x0d07u16.to_le_bytes());
        io[0x54..0x56].copy_from_slice(&9u16.to_le_bytes());
        for mode in 0..4u16 {
            for first in 0..64u16 {
                for second in 0..64u16 {
                    io[0x50..0x52]
                        .copy_from_slice(&(first | mode << 6 | second << 8).to_le_bytes());
                    let effects = Effects::new(&io);
                    for semi in [false, true] {
                        if effects.inactive(semi) {
                            for front in 0..6 {
                                for back in 0..6 {
                                    assert_eq!(
                                        effects.apply(
                                            Pixel {
                                                color: 0x4bb7,
                                                layer: front,
                                                semi
                                            },
                                            Pixel {
                                                color: 0x107a,
                                                layer: back,
                                                semi: false
                                            }
                                        ),
                                        0x4bb7
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn effect_target_masks_and_backdrop_selection_match_ppu() {
        for mode in 0..4u16 {
            for first in [0, 4u16] {
                for second in [0, 0x2000u16] {
                    for coefficient in [0, 1, 7, 8, 15, 16, 31u16] {
                        let mut bus = MemMap::new(Vec::new());
                        bus.io[..2].copy_from_slice(&0x0403u16.to_le_bytes());
                        bus.io[0x50..0x52]
                            .copy_from_slice(&(first | second | (mode << 6)).to_le_bytes());
                        bus.io[0x52..0x54].copy_from_slice(
                            &(coefficient | ((31 - coefficient) << 8)).to_le_bytes(),
                        );
                        bus.io[0x54..0x56].copy_from_slice(&coefficient.to_le_bytes());
                        bus.vram[..2].copy_from_slice(&0x03b7u16.to_le_bytes());
                        bus.palette[..2].copy_from_slice(&0x06a9u16.to_le_bytes());
                        let actual = apply(
                            &bus.io,
                            Pixel {
                                color: 0x03b7,
                                layer: 2,
                                semi: false,
                            },
                            Pixel {
                                color: 0x06a9,
                                layer: 5,
                                semi: false,
                            },
                        );
                        ppu::render_scanline(&mut bus, 0);
                        assert_eq!(
                            actual, bus.framebuffer[0],
                            "mode={mode} first={first} second={second} coefficient={coefficient}"
                        );
                    }
                }
            }
        }
    }
}
