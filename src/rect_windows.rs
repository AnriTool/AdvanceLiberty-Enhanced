//! WIN0/WIN1 screen regions. OBJ-window mode remains explicitly unsupported.
fn half(io: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(io[offset..offset + 2].try_into().unwrap())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wide_bridge_windows_preserve_center_and_continue_clipped_deck_edges() {
        let mut io = vec![0u8; 1024];
        for (o, v) in [
            (0, 0x3741u16),
            (0x40, 0x00f0),
            (0x44, 0x00a0),
            (0x48, 0x17),
            (0x4a, 0x37),
            (0x50, 0x1044),
            (0x52, 0x1f00),
        ] {
            io[o..o + 2].copy_from_slice(&v.to_le_bytes());
        }
        let view = crate::viewport::View::new(3, crate::viewport::Aspect::Wide);
        for zoom in [0.95, 1.0, 1.05] {
            let w = Windows::new(&io).with_view([0.25, -0.5], zoom);
            let original = w.prepare(3);
            let wide = w.prepare_view(view);
            for y in 0..480 {
                for x in 0..720 {
                    assert_eq!(original.mask(x, y), wide.mask(x + view.padding, y));
                }
            }
            assert_eq!(wide.mask(0, 240), original.mask(0, 240));
            assert_eq!(wide.mask(853, 240), original.mask(719, 240));
        }
    }
    #[test]
    fn prepared_masks_match_scalar_with_wrap_empty_overlap_and_fractional_view() {
        for flags in [0u16, 0x2000, 0x4000, 0x6000] {
            for ranges in [
                [0x50a0u16, 0x2864, 0x6090, 0x3860],
                [0xe008, 0x980c, 0x1010, 0x00a0],
            ] {
                let mut io = vec![0u8; 1024];
                for (o, v) in [
                    (0, 0x1741 | flags),
                    (0x40, ranges[0]),
                    (0x44, ranges[1]),
                    (0x42, ranges[2]),
                    (0x46, ranges[3]),
                    (0x48, 0x2717),
                    (0x4a, 0x37),
                    (0x50, 0x1044),
                    (0x52, 0x1f00),
                ] {
                    io[o..o + 2].copy_from_slice(&v.to_le_bytes());
                }
                for scale in [1, 3] {
                    for zoom in [0.91, 1.0, 1.13] {
                        let windows = Windows::new(&io).with_view([-5.17, 10.31], zoom);
                        let prepared = windows.prepare(scale);
                        for y in 0..160 * scale {
                            for x in 0..240 * scale {
                                assert_eq!(prepared.mask(x, y), windows.mask(x, y, scale));
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn bridge_edges_follow_fractional_camera_and_zoom_while_ui_windows_stay_fixed() {
        let mut io = vec![0u8; 1024];
        for (o, v) in [
            (0, 0x3741u16),
            (0x40, 0x50a0),
            (0x44, 0x2864),
            (0x48, 0x17),
            (0x4a, 0x37),
            (0x50, 0x1044),
            (0x52, 0x1f00),
        ] {
            io[o..o + 2].copy_from_slice(&v.to_le_bytes());
        }
        let windows = Windows::new(&io).with_view([-5.0, 10.0], 1.1);
        assert_eq!(windows.mask(244, 150, 3), 0x37);
        assert_eq!(windows.mask(245, 150, 3), 0x17);
        assert_eq!(windows.mask(508, 150, 3), 0x17);
        assert_eq!(windows.mask(509, 150, 3), 0x37);
        io[0x50..0x52].fill(0);
        let ui = Windows::new(&io).with_view([-5.0, 10.0], 1.1);
        assert_eq!(ui.mask(239, 150, 3), 0x37);
        assert_eq!(ui.mask(240, 150, 3), 0x17);
    }
}
pub struct Windows {
    enabled: u16,
    horizontal: [u16; 2],
    vertical: [u16; 2],
    inside: [u16; 2],
    outside: u16,
    world_bridge: bool,
    shift: [f64; 2],
    zoom: f64,
}
pub struct PreparedWindows {
    horizontal: Vec<u8>,
    vertical: Vec<u8>,
    values: [u16; 4],
    world_bridge: bool,
}
impl PreparedWindows {
    #[inline]
    pub fn mask(&self, x: usize, y: usize) -> u16 {
        if self.horizontal.is_empty() {
            return 0x3f;
        }
        self.values[(self.horizontal[x] & self.vertical[y]) as usize]
    }
    #[inline]
    pub fn bridge_deck(&self, mask: u16) -> bool {
        self.world_bridge && mask & 32 == 0
    }
}
impl Windows {
    pub fn new(io: &[u8]) -> Self {
        let inside = half(io, 0x48);
        Self {
            enabled: half(io, 0) & 0x6000,
            horizontal: [half(io, 0x40), half(io, 0x42)],
            vertical: [half(io, 0x44), half(io, 0x46)],
            inside: [inside & 0x3f, (inside >> 8) & 0x3f],
            outside: half(io, 0x4a) & 0x3f,
            // GTA USA bridge reveal: the same layers on either side,
            // with color effects disabled on the bridge deck region.
            world_bridge: half(io, 0) & 0x6000 == 0x2000
                && inside & 0x3f == 0x17
                && half(io, 0x4a) & 0x3f == 0x37
                && half(io, 0x50) == 0x1044
                && half(io, 0x52) & 31 == 0
                && (half(io, 0x52) >> 8) & 31 >= 16,
            shift: [0.0; 2],
            zoom: 1.0,
        }
    }
    pub fn with_view(mut self, shift: [f64; 2], zoom: f64) -> Self {
        self.shift = shift;
        self.zoom = zoom;
        self
    }
    #[cfg(test)]
    pub fn prepare(&self, scale: usize) -> PreparedWindows {
        self.prepare_view(crate::viewport::View::new(scale, Default::default()))
    }
    pub fn prepare_view(&self, view: crate::viewport::View) -> PreparedWindows {
        let scale = view.scale;
        let axis = |length: usize, count: usize, offset: f64, ranges: [u16; 2], shift: f64| {
            if self.enabled == 0 {
                return Vec::new();
            }
            (0..count)
                .map(|p| {
                    let mut value = p as f64 / scale as f64 - offset;
                    if self.world_bridge {
                        value = (120.0 + (value - 120.0) / self.zoom + shift)
                            .clamp(0.0, length as f64 - 1e-9);
                    }
                    let mut bits = 0;
                    for (i, range) in ranges.into_iter().enumerate() {
                        if self.enabled & (0x2000 << i) != 0 && Self::contains(range, value) {
                            bits |= 1 << i;
                        }
                    }
                    bits
                })
                .collect()
        };
        PreparedWindows {
            horizontal: axis(
                240,
                view.width,
                view.offset(),
                self.horizontal,
                self.shift[0],
            ),
            vertical: axis(160, view.height, 0.0, self.vertical, self.shift[1]),
            values: [self.outside, self.inside[0], self.inside[1], self.inside[0]],
            world_bridge: self.world_bridge,
        }
    }
    #[inline]
    fn contains(range: u16, value: f64) -> bool {
        let start = (range >> 8) as f64;
        let end = (range & 255) as f64;
        if start <= end {
            value >= start && value < end
        } else {
            value >= start || value < end
        }
    }
    #[inline]
    #[cfg(test)]
    pub fn mask(&self, x: usize, y: usize, scale: usize) -> u16 {
        if self.enabled == 0 {
            return 0x3f;
        }
        let (mut x, mut y) = (x as f64 / scale as f64, y as f64 / scale as f64);
        if self.world_bridge {
            // Screen-clipped edges represent a deck continuing off-screen.
            x = (120.0 + (x - 120.0) / self.zoom + self.shift[0]).clamp(0.0, 240.0 - 1e-9);
            y = (120.0 + (y - 120.0) / self.zoom + self.shift[1]).clamp(0.0, 160.0 - 1e-9);
        }
        for i in 0..2 {
            if self.enabled & (0x2000 << i) != 0
                && Self::contains(self.horizontal[i], x)
                && Self::contains(self.vertical[i], y)
            {
                return self.inside[i];
            }
        }
        self.outside
    }
}
