//! Observation-only PCM sidecar. Samples follow the same DMA/FIFO clock as the
//! canonical bytes; they never enter guest memory or the hardware audio mix.
use std::collections::{BTreeMap, VecDeque};
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PcmOverlay {
    pub memory: BTreeMap<u32, f32>,
    pending: [VecDeque<f32>; 2],
    held: [f32; 2],
    pub tap: [Vec<(f32, u32)>; 2],
    source: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dma_shadow_tracks_fifo_drops_holds_and_reset_without_guest_samples() {
        let mut overlay = PcmOverlay::default();
        overlay.memory.insert(0x03006740, 3.5);
        overlay.memory.insert(0x03006741, 9.0);
        overlay.dma(Some(0x03006740));
        overlay.push(0, true);
        overlay.push(0, false); // full hardware FIFO drops the second byte
        overlay.dma(None);
        overlay.push(0, true); // CPU FIFO write has no shadow
        overlay.drain(0, true, true, 1755);
        overlay.drain(0, true, true, 1755);
        overlay.drain(0, false, true, 1755);
        assert_eq!(overlay.tap[0], [(3.5, 1755), (0.0, 1755), (0.0, 1755)]);
        overlay.push(1, true);
        overlay.reset(1);
        assert!(overlay.valid_snapshot([0, 0]));
    }
}
impl PcmOverlay {
    pub fn valid_snapshot(&self, depths: [usize; 2]) -> bool {
        self.source.is_none()
            && self.memory.len() <= 1024
            && self
                .memory
                .iter()
                .all(|(&a, &v)| (0x03000000..0x03008000).contains(&a) && v.is_finite())
            && (0..2).all(|i| {
                self.pending[i].len() == depths[i]
                    && self.held[i].is_finite()
                    && self.pending[i].iter().all(|v| v.is_finite())
                    && self.tap[i].len() <= 0x10000
                    && self.tap[i].iter().all(|(v, p)| v.is_finite() && *p > 0)
            })
    }
    pub(crate) fn dma(&mut self, source: Option<u32>) {
        self.source = source;
    }
    pub(crate) fn push(&mut self, channel: usize, accepted: bool) {
        let value = self
            .source
            .and_then(|a| self.memory.get(&a))
            .copied()
            .unwrap_or(0.0);
        self.source = self.source.map(|a| a.wrapping_add(1));
        if accepted {
            self.pending[channel].push_back(value);
        }
    }
    pub(crate) fn drain(&mut self, channel: usize, popped: bool, tap: bool, period: u32) {
        if popped {
            self.held[channel] = self.pending[channel].pop_front().unwrap_or(0.0);
        }
        if tap && self.tap[channel].len() < 0x10000 {
            self.tap[channel].push((self.held[channel], period));
        }
    }
    pub(crate) fn reset(&mut self, channel: usize) {
        self.pending[channel].clear();
    }
}
