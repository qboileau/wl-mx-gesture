use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl Color {
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self { red, green, blue, alpha }
    }
}

impl TryFrom<String> for Color {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let invalid = || format!("`{text}` is not a #rrggbb or #rrggbbaa colour");
        let hex =
            text.strip_prefix('#').filter(|hex| matches!(hex.len(), 6 | 8) && hex.is_ascii()).ok_or_else(invalid)?;
        let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).map_err(|_| invalid());
        let alpha = if hex.len() == 8 { byte(6)? } else { u8::MAX };
        Ok(Self::rgba(byte(0)?, byte(2)?, byte(4)?, alpha))
    }
}

/// Look of the wheel overlay; sizes are logical pixels.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Style {
    pub outer_radius: f32,
    pub inner_radius: f32,
    pub background: Color,
    pub highlight: Color,
    pub text: Color,
    pub highlighted_text: Color,
    /// Font family name; the system sans-serif font when unset.
    pub font: Option<String>,
    pub font_size: f32,
    pub icon_size: f32,
    /// Freedesktop icon theme; the desktop's (GTK, then KDE settings) when unset.
    pub icon_theme: Option<String>,
    /// Recolours icons with the text colour, which suits monochrome themes; disable for coloured icons.
    pub tint_icons: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            outer_radius: 150.0,
            inner_radius: 50.0,
            background: Color::rgba(0x1E, 0x1E, 0x2E, 0xE6),
            highlight: Color::rgba(0x89, 0xB4, 0xFA, 0xFF),
            text: Color::rgba(0xCD, 0xD6, 0xF4, 0xFF),
            highlighted_text: Color::rgba(0x1E, 0x1E, 0x2E, 0xFF),
            font: None,
            font_size: 14.0,
            icon_size: 32.0,
            icon_theme: None,
            tint_icons: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colours_with_optional_alpha() {
        assert_eq!(Color::try_from("#89b4fa".to_owned()), Ok(Color::rgba(0x89, 0xB4, 0xFA, 0xFF)));
        assert_eq!(Color::try_from("#1E1E2E80".to_owned()), Ok(Color::rgba(0x1E, 0x1E, 0x2E, 0x80)));
    }

    #[test]
    fn rejects_malformed_colours() {
        for text in ["89b4fa", "#89b4f", "#89b4fazz", "#ééééé"] {
            assert!(Color::try_from(text.to_owned()).is_err(), "{text}");
        }
    }
}
