//! Host viewport. The original 720x480 world stays centered at unchanged scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Aspect {
    #[default]
    Original,
    Wide,
}
impl Aspect {
    pub fn toggle(&mut self) {
        *self = match *self {
            Self::Original => Self::Wide,
            Self::Wide => Self::Original,
        };
    }
}
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub width: usize,
    pub height: usize,
    pub scale: usize,
    pub padding: usize,
}
impl View {
    pub fn new(scale: usize, aspect: Aspect) -> Self {
        let height = 160 * scale;
        let width = if aspect == Aspect::Original {
            240 * scale
        } else {
            (height * 16).div_ceil(9).div_ceil(2) * 2
        };
        Self {
            width,
            height,
            scale,
            padding: (width - 240 * scale) / 2,
        }
    }
    pub fn offset(self) -> f64 {
        self.padding as f64 / self.scale as f64
    }
    pub fn source_x(self, x: usize) -> f64 {
        (x as f64 - self.padding as f64) / self.scale as f64
    }
    pub fn extent(self) -> f64 {
        if self.padding == 0 {
            1.0
        } else {
            (120.0 + self.offset()) / 118.0
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wide_keeps_center_pixels_and_scale() {
        let v = View::new(3, Aspect::Wide);
        assert_eq!((v.width, v.height, v.padding), (854, 480, 67));
        for x in 0..720 {
            assert!((v.source_x(x + v.padding) - x as f64 / 3.0).abs() < 1e-12);
        }
        assert!(v.source_x(0) < 0.0);
        assert!(v.source_x(853) > 240.0);
    }
}
