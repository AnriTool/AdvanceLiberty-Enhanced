//! Presentation-only HUD layout in original 240x160 coordinates.
//! BG1 and screen-fixed radar/weapon OBJs share the same transforms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Size {
    Half,
    Small,
    Medium,
    Large,
    #[default]
    Original,
}
impl Size {
    pub fn percent(self) -> u8 {
        match self {
            Self::Half => 50,
            Self::Small => 65,
            Self::Medium => 75,
            Self::Large => 85,
            Self::Original => 100,
        }
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "50" => Ok(Self::Half),
            "65" => Ok(Self::Small),
            "75" => Ok(Self::Medium),
            "85" => Ok(Self::Large),
            "100" => Ok(Self::Original),
            _ => Err("HUD size must be 50, 65, 75, 85 or 100".into()),
        }
    }
    pub fn cycle(self, reverse: bool) -> Self {
        let sizes = [
            Self::Half,
            Self::Small,
            Self::Medium,
            Self::Large,
            Self::Original,
        ];
        let i = sizes.iter().position(|s| *s == self).unwrap();
        sizes[(i + if reverse { 4 } else { 1 }) % 5]
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub radar: Size,
    pub status: Size,
    pub messages: Size,
}
impl Settings {
    pub fn all(self) -> Option<Size> {
        (self.radar == self.status && self.status == self.messages).then_some(self.radar)
    }
    pub fn change(&mut self, row: usize, reverse: bool) {
        match row {
            0 => {
                let size = self.all().unwrap_or(Size::Original).cycle(reverse);
                *self = Self {
                    radar: size,
                    status: size,
                    messages: size,
                };
            }
            1 => self.radar = self.radar.cycle(reverse),
            2 => self.status = self.status.cycle(reverse),
            3 => self.messages = self.messages.cycle(reverse),
            4 => *self = Self::default(),
            _ => {}
        }
    }
}
#[derive(Clone, Copy)]
struct Group {
    rect: [usize; 4],
    anchor: [f64; 2],
    size: Size,
}
fn groups(settings: Settings) -> [Group; 3] {
    [
        Group {
            rect: [0, 0, 64, 64],
            anchor: [8.0, 8.0],
            size: settings.radar,
        },
        Group {
            rect: [120, 0, 240, 64],
            anchor: [232.0, 8.0],
            size: settings.status,
        },
        Group {
            rect: [0, 0, 240, 160],
            anchor: [120.0, 152.0],
            size: settings.messages,
        },
    ]
}
fn region(x: usize, y: usize) -> usize {
    if x < 64 && y < 64 {
        0
    } else if x >= 120 && y < 64 {
        1
    } else {
        2
    }
}
pub struct Layout {
    axes: [(Vec<Option<usize>>, Vec<Option<usize>>); 3],
}
impl Layout {
    #[cfg(test)]
    pub fn new(settings: Settings, scale: usize) -> Self {
        Self::new_view(
            settings,
            crate::viewport::View::new(scale, Default::default()),
            true,
        )
    }
    pub fn new_view(settings: Settings, view: crate::viewport::View, edges: bool) -> Self {
        let scale = view.scale;
        let axes = groups(settings)
            .into_iter()
            .enumerate()
            .map(|(i, g)| {
                let edge = if !edges {
                    0.0
                } else {
                    match i {
                        0 => -view.offset(),
                        1 => view.offset(),
                        _ => 0.0,
                    }
                };
                let axis = |count: usize, anchor: f64, start: usize, end: usize, shift: f64| {
                    (0..count)
                        .map(|p| {
                            let source = anchor
                                + (p as f64 / scale as f64 - shift - anchor) * 100.0
                                    / g.size.percent() as f64;
                            (source >= start as f64 && source < end as f64)
                                .then(|| source.floor() as usize)
                        })
                        .collect()
                };
                (
                    axis(
                        view.width,
                        g.anchor[0],
                        g.rect[0],
                        g.rect[2],
                        view.offset() + edge,
                    ),
                    axis(view.height, g.anchor[1], g.rect[1], g.rect[3], 0.0),
                )
            })
            .collect::<Vec<_>>()
            .try_into()
            .ok()
            .unwrap();
        Self { axes }
    }
    pub fn pixel(&self, source: &[Option<u16>], x: usize, y: usize) -> Option<u16> {
        for (i, (xs, ys)) in self.axes.iter().enumerate() {
            if let (Some(sx), Some(sy)) = (xs[x], ys[y]) {
                if region(sx, sy) == i {
                    if let Some(color) = source[sy * 240 + sx] {
                        return Some(color);
                    }
                }
            }
        }
        None
    }
}
pub fn edge_offset(position: [f64; 2], size: [f64; 2], padding: f64) -> f64 {
    for (i, g) in groups(Settings::default()).iter().take(2).enumerate() {
        if position[0] >= g.rect[0] as f64
            && position[1] >= g.rect[1] as f64
            && position[0] + size[0] <= g.rect[2] as f64
            && position[1] + size[1] <= g.rect[3] as f64
        {
            return if i == 0 { -padding } else { padding };
        }
    }
    0.0
}
// World actors must never be classified from screen position. Call this only
// for OBJs already verified as screen-fixed, priority-zero UI sprites.
pub fn sprite(settings: Settings, position: [f64; 2], size: [f64; 2]) -> ([f64; 2], f64) {
    for g in groups(settings).iter().take(2) {
        if position[0] >= g.rect[0] as f64
            && position[1] >= g.rect[1] as f64
            && position[0] + size[0] <= g.rect[2] as f64
            && position[1] + size[1] <= g.rect[3] as f64
        {
            let factor = g.size.percent() as f64 / 100.0;
            return (
                std::array::from_fn(|i| g.anchor[i] + (position[i] - g.anchor[i]) * factor),
                factor,
            );
        }
    }
    (position, 1.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wide_hud_pins_radar_left_status_right_and_messages_center() {
        let mut source = vec![None; 38400];
        source[8 * 240 + 8] = Some(1);
        source[8 * 240 + 232] = Some(2);
        source[144 * 240 + 120] = Some(3);
        let view = crate::viewport::View::new(3, crate::viewport::Aspect::Wide);
        let layout = Layout::new_view(Settings::default(), view, true);
        assert_eq!(layout.pixel(&source, 24, 24), Some(1));
        assert_eq!(
            layout.pixel(&source, 232 * 3 + 2 * view.padding, 24),
            Some(2)
        );
        assert_eq!(
            layout.pixel(&source, 120 * 3 + view.padding, 144 * 3),
            Some(3)
        );
    }
    #[test]
    fn default_layout_is_pixel_exact_at_native_and_enhanced_resolution() {
        let source: Vec<_> = (0..38400)
            .map(|i| {
                if i % 7 == 0 {
                    None
                } else {
                    Some((i & 32767) as u16)
                }
            })
            .collect();
        for scale in [1, 3] {
            let layout = Layout::new(Settings::default(), scale);
            for y in 0..160 * scale {
                for x in 0..240 * scale {
                    assert_eq!(
                        layout.pixel(&source, x, y),
                        source[y / scale * 240 + x / scale]
                    );
                }
            }
        }
    }
    #[test]
    fn radar_icons_weapon_and_text_share_anchors_and_expose_world() {
        let s = Settings {
            radar: Size::Half,
            status: Size::Medium,
            messages: Size::Half,
        };
        assert_eq!(sprite(s, [24.0, 26.0], [8.0; 2]), ([16.0, 17.0], 0.5));
        assert_eq!(sprite(s, [200.0, 8.0], [32.0; 2]), ([208.0, 8.0], 0.75));
        assert_eq!(sprite(s, [100.0, 80.0], [16.0; 2]), ([100.0, 80.0], 1.0));
        let mut source = vec![None; 38400];
        source[26 * 240 + 24] = Some(123);
        source[144 * 240 + 120] = Some(456);
        let layout = Layout::new(s, 3);
        assert_eq!(layout.pixel(&source, 16 * 3, 17 * 3), Some(123));
        assert_eq!(layout.pixel(&source, 24 * 3, 26 * 3), None);
        assert_eq!(layout.pixel(&source, 120 * 3, 148 * 3), Some(456));
    }
}
