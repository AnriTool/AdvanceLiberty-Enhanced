//! Presentation preferences, independent of SDL and guest state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Renderer {
    #[default]
    Enhanced,
    Original,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureMapping {
    #[default]
    Perspective,
    Affine,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub renderer: Renderer,
    pub textures: TextureMapping,
    pub hud: crate::hud_layout::Settings,
}
impl Settings {
    pub fn toggle_renderer(&mut self) {
        self.renderer = match self.renderer {
            Renderer::Enhanced => Renderer::Original,
            Renderer::Original => Renderer::Enhanced,
        };
    }
    #[allow(dead_code)] // Retained for a future preferences menu; no gameplay hotkey.
    pub fn toggle_textures(&mut self) {
        self.textures = match self.textures {
            TextureMapping::Perspective => TextureMapping::Affine,
            TextureMapping::Affine => TextureMapping::Perspective,
        };
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toggles_restore_defaults_and_keep_preferences_independent() {
        let mut s = Settings::default();
        assert_eq!(s.renderer, Renderer::Enhanced);
        assert_eq!(s.textures, TextureMapping::Perspective);
        s.toggle_renderer();
        s.toggle_textures();
        assert_eq!(s.renderer, Renderer::Original);
        assert_eq!(s.textures, TextureMapping::Affine);
        s.toggle_renderer();
        assert_eq!(s.textures, TextureMapping::Affine);
        s.toggle_textures();
        assert_eq!(s, Settings::default());
    }
}
