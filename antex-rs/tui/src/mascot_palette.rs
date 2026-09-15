use ratatui::style::Color;

use crate::terminal_palette::StdoutColorLevel;
use crate::terminal_palette::effective_stdout_color_level;
use crate::terminal_palette::indexed_color;
use crate::terminal_palette::rgb_color;

const CHESTNUT_RGB: (u8, u8, u8) = (177, 108, 70);
const DARK_BROWN_RGB: (u8, u8, u8) = (108, 59, 43);
const SAND_RGB: (u8, u8, u8) = (207, 169, 126);
const TEAL_RGB: (u8, u8, u8) = (104, 158, 151);

const CHESTNUT_ANSI256: u8 = 137;
const DARK_BROWN_ANSI256: u8 = 95;
const SAND_ANSI256: u8 = 180;
const TEAL_ANSI256: u8 = 73;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MascotColor {
    Chestnut,
    DarkBrown,
    Sand,
    Teal,
}

pub(crate) fn mascot_color(color: MascotColor) -> Color {
    mascot_color_for_level(color, effective_stdout_color_level())
}

fn mascot_color_for_level(color: MascotColor, color_level: StdoutColorLevel) -> Color {
    match color_level {
        StdoutColorLevel::TrueColor => rgb_color(match color {
            MascotColor::Chestnut => CHESTNUT_RGB,
            MascotColor::DarkBrown => DARK_BROWN_RGB,
            MascotColor::Sand => SAND_RGB,
            MascotColor::Teal => TEAL_RGB,
        }),
        StdoutColorLevel::Ansi256 => indexed_color(match color {
            MascotColor::Chestnut => CHESTNUT_ANSI256,
            MascotColor::DarkBrown => DARK_BROWN_ANSI256,
            MascotColor::Sand => SAND_ANSI256,
            MascotColor::Teal => TEAL_ANSI256,
        }),
        StdoutColorLevel::Ansi16 => match color {
            MascotColor::Chestnut => Color::LightRed,
            MascotColor::DarkBrown => Color::Red,
            MascotColor::Sand => Color::Reset,
            MascotColor::Teal => Color::Cyan,
        },
        StdoutColorLevel::Unknown => Color::Reset,
    }
}

pub(crate) fn is_preserved_mascot_color(color: Color) -> bool {
    match color {
        Color::Rgb(r, g, b) => {
            [CHESTNUT_RGB, DARK_BROWN_RGB, SAND_RGB, TEAL_RGB].contains(&(r, g, b))
        }
        Color::Indexed(index) => [
            CHESTNUT_ANSI256,
            DARK_BROWN_ANSI256,
            SAND_ANSI256,
            TEAL_ANSI256,
        ]
        .contains(&index),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn mascot_palette_has_explicit_reduced_color_fallbacks() {
        assert_eq!(
            mascot_color_for_level(MascotColor::Chestnut, StdoutColorLevel::Ansi16),
            Color::LightRed
        );
        assert_eq!(
            mascot_color_for_level(MascotColor::DarkBrown, StdoutColorLevel::Unknown),
            Color::Reset
        );
    }
}
