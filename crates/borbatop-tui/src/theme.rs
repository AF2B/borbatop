//! Colors of the Borba Design System and a plain 16-color fallback.

use ratatui::style::Color;

/// Which palette to draw with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeKind {
    /// The Borba palette in 24-bit color.
    #[default]
    Borba,
    /// The terminal's own 16 colors, for terminals without truecolor.
    Ansi,
}

/// The colors the screen is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Terminal background.
    pub background: Color,
    /// Main text.
    pub foreground: Color,
    /// Secondary text.
    pub secondary: Color,
    /// Labels, bars and highlights (Electric Blue).
    pub accent: Color,
    /// Panel borders.
    pub border: Color,
    /// Errors and hot values.
    pub danger: Color,
    /// Healthy status.
    pub ok: Color,
    /// Background of the selected row.
    pub selected_background: Color,
    /// Text of the selected row.
    pub selected_foreground: Color,
}

impl Theme {
    /// The palette for `kind`.
    ///
    /// # Arguments
    ///
    /// * `kind` - palette to use
    #[must_use]
    pub const fn of(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::Borba => Self::borba(),
            ThemeKind::Ansi => Self::ansi(),
        }
    }

    /// Borba Design System: black terminal, white text, Electric Blue labels, 1px grey borders.
    #[must_use]
    pub const fn borba() -> Self {
        Self {
            background: Color::Rgb(0, 0, 0),
            foreground: Color::Rgb(255, 255, 255),
            secondary: Color::Rgb(0x88, 0x99, 0xA6),
            accent: Color::Rgb(0x00, 0x74, 0xD9),
            border: Color::Rgb(0x33, 0x33, 0x33),
            danger: Color::Rgb(0xEF, 0x44, 0x44),
            ok: Color::Rgb(0x22, 0xC5, 0x5E),
            selected_background: Color::Rgb(255, 255, 255),
            selected_foreground: Color::Rgb(0, 0, 0),
        }
    }

    /// The terminal's own palette.
    #[must_use]
    pub const fn ansi() -> Self {
        Self {
            background: Color::Reset,
            foreground: Color::White,
            secondary: Color::Gray,
            accent: Color::LightBlue,
            border: Color::DarkGray,
            danger: Color::LightRed,
            ok: Color::LightGreen,
            selected_background: Color::White,
            selected_foreground: Color::Black,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borba_palette_matches_the_design_tokens() {
        let theme = Theme::of(ThemeKind::Borba);
        assert_eq!(theme.accent, Color::Rgb(0x00, 0x74, 0xD9));
        assert_eq!(theme.secondary, Color::Rgb(0x88, 0x99, 0xA6));
        assert_eq!(theme.border, Color::Rgb(0x33, 0x33, 0x33));
        assert_eq!(theme.danger, Color::Rgb(0xEF, 0x44, 0x44));
        assert_eq!(theme.ok, Color::Rgb(0x22, 0xC5, 0x5E));
    }

    #[test]
    fn ansi_palette_keeps_the_terminal_background() {
        assert_eq!(Theme::of(ThemeKind::Ansi).background, Color::Reset);
    }
}
