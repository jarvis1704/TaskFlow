use iced::Color;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTheme {
    Dark,
    Light,
}

impl Default for AppTheme {
    fn default() -> Self {
        AppTheme::Dark
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct ColorTokens {
    pub bg_base: Color,
    pub bg_surface: Color,
    pub bg_surface_hover: Color,
    pub border_subtle: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub accent_primary: Color,
    pub accent_success: Color,
    pub accent_warning: Color,
    pub accent_danger: Color,
}

impl AppTheme {
    pub fn colors(&self) -> ColorTokens {
        match self {
            AppTheme::Dark => ColorTokens {
                bg_base: Color::from_rgb8(0x15, 0x16, 0x1B),
                bg_surface: Color::from_rgb8(0x1C, 0x1E, 0x26),
                bg_surface_hover: Color::from_rgb8(0x26, 0x28, 0x33),
                border_subtle: Color::from_rgb8(0x2A, 0x2D, 0x3A),
                text_primary: Color::from_rgb8(0xE7, 0xE8, 0xEC),
                text_secondary: Color::from_rgb8(0x8B, 0x8D, 0x98),
                accent_primary: Color::from_rgb8(0x7C, 0x9E, 0xFF),
                accent_success: Color::from_rgb8(0x5F, 0xD9, 0xA4),
                accent_warning: Color::from_rgb8(0xF2, 0xB8, 0x6C),
                accent_danger: Color::from_rgb8(0xF2, 0x74, 0x6C),
            },
            AppTheme::Light => ColorTokens {
                bg_base: Color::from_rgb8(0xF8, 0xF9, 0xFA),
                bg_surface: Color::from_rgb8(0xFF, 0xFF, 0xFF),
                bg_surface_hover: Color::from_rgb8(0xF0, 0xF1, 0xF3),
                border_subtle: Color::from_rgb8(0xE2, 0xE4, 0xE8),
                text_primary: Color::from_rgb8(0x1A, 0x1B, 0x1E),
                text_secondary: Color::from_rgb8(0x6B, 0x6E, 0x7B),
                accent_primary: Color::from_rgb8(0x4F, 0x75, 0xFF), // slightly darker for contrast in light mode
                accent_success: Color::from_rgb8(0x2E, 0xB8, 0x72),
                accent_warning: Color::from_rgb8(0xD9, 0x8C, 0x38),
                accent_danger: Color::from_rgb8(0xD9, 0x48, 0x38),
            },
        }
    }
}
