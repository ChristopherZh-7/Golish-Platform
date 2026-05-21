//! Frontend-facing serialisation types for the virtual terminal grid.
//!
//! Mirrors the relevant subset of `alacritty_terminal::term::cell::Cell`
//! and `vte::ansi::Color`, projected into the small JSON-friendly shape
//! the frontend `GridTerminal` component expects. The conversion is
//! intentionally lossy in places — features we don't render yet
//! (hyperlinks, dim, double underlines) are folded into "default", and
//! 24-bit RGB is preserved as a packed `0xRRGGBB` integer so the
//! protocol stays compact (one `i32` per channel instead of a 3-element
//! array per cell).

use serde::Serialize;

use alacritty_terminal::term::cell::Flags as AlacrittyFlags;
use alacritty_terminal::vte::ansi::{Color as AlacrittyColor, NamedColor, Rgb};

/// One cell on the virtual terminal grid.
///
/// `ch` is the printable codepoint (space for empty cells). Wide-char
/// continuation cells use `'\0'`; the frontend treats those as "skip,
/// already covered by the previous cell".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Cell {
    /// Printable codepoint. `'\0'` marks the trailing column of a
    /// double-width character (frontend skips them).
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub attrs: CellAttrs,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            attrs: CellAttrs::default(),
        }
    }
}

/// Cell colour. Indexed colours are stored as a `u8` (0..=255 matches
/// xterm 256-colour palette); RGB is a packed 24-bit `0xRRGGBB`. The
/// frontend resolves indexed values against the active theme, RGB is
/// rendered as-is.
///
/// Struct-variant shape (`{ "kind": "rgb", "value": 0x123456 }`) is the
/// frontend wire contract — see `frontend/components/GridTerminal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Color {
    Default,
    Indexed { value: u8 },
    Rgb { value: u32 },
}

impl From<AlacrittyColor> for Color {
    fn from(value: AlacrittyColor) -> Self {
        match value {
            AlacrittyColor::Named(named) => Self::from(named),
            AlacrittyColor::Spec(rgb) => Self::from(rgb),
            AlacrittyColor::Indexed(idx) => Color::Indexed { value: idx },
        }
    }
}

impl From<NamedColor> for Color {
    fn from(value: NamedColor) -> Self {
        match value {
            NamedColor::Foreground => Color::Default,
            NamedColor::Background => Color::Default,
            other => Color::Indexed { value: other as u8 },
        }
    }
}

impl From<Rgb> for Color {
    fn from(rgb: Rgb) -> Self {
        Color::Rgb {
            value: ((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32,
        }
    }
}

bitflags::bitflags! {
    /// Visual attribute flags. Maps to CSS classes on the frontend
    /// (`golish-grid-bold`, `golish-grid-italic`, …). Kept narrow on
    /// purpose — Alacritty supports e.g. dotted / dashed underlines,
    /// but Phase B intentionally renders them all as a single
    /// underline to keep the CSS small.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct CellAttrs: u16 {
        const BOLD          = 1 << 0;
        const ITALIC        = 1 << 1;
        const UNDERLINE     = 1 << 2;
        const INVERSE       = 1 << 3;
        const STRIKETHROUGH = 1 << 4;
        const DIM           = 1 << 5;
        const HIDDEN        = 1 << 6;
        const BLINK         = 1 << 7;
    }
}

impl Serialize for CellAttrs {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u16(self.bits())
    }
}

impl From<AlacrittyFlags> for CellAttrs {
    fn from(value: AlacrittyFlags) -> Self {
        let mut out = CellAttrs::empty();
        if value.contains(AlacrittyFlags::BOLD) {
            out |= CellAttrs::BOLD;
        }
        if value.contains(AlacrittyFlags::ITALIC) {
            out |= CellAttrs::ITALIC;
        }
        if value.intersects(AlacrittyFlags::ALL_UNDERLINES) {
            out |= CellAttrs::UNDERLINE;
        }
        if value.contains(AlacrittyFlags::INVERSE) {
            out |= CellAttrs::INVERSE;
        }
        if value.contains(AlacrittyFlags::STRIKEOUT) {
            out |= CellAttrs::STRIKETHROUGH;
        }
        if value.contains(AlacrittyFlags::DIM) {
            out |= CellAttrs::DIM;
        }
        if value.contains(AlacrittyFlags::HIDDEN) {
            out |= CellAttrs::HIDDEN;
        }
        out
    }
}

/// Cursor position + display style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
    pub style: CursorStyle,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            visible: true,
            style: CursorStyle::Block,
        }
    }
}

/// Cursor presentation style — the frontend renders these as different
/// CSS pseudo-elements on the focused cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorStyle {
    Block,
    Underline,
    Bar,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cell_is_space() {
        let cell = Cell::default();
        assert_eq!(cell.ch, ' ');
        assert_eq!(cell.fg, Color::Default);
        assert_eq!(cell.bg, Color::Default);
        assert!(cell.attrs.is_empty());
    }

    #[test]
    fn rgb_packs_into_u32() {
        let c = Color::from(Rgb {
            r: 0x12,
            g: 0x34,
            b: 0x56,
        });
        assert_eq!(c, Color::Rgb { value: 0x123456 });
    }

    #[test]
    fn named_fg_bg_collapse_to_default() {
        assert_eq!(
            Color::from(AlacrittyColor::Named(NamedColor::Foreground)),
            Color::Default
        );
        assert_eq!(
            Color::from(AlacrittyColor::Named(NamedColor::Background)),
            Color::Default
        );
    }

    #[test]
    fn named_palette_color_kept_as_indexed() {
        let red = Color::from(AlacrittyColor::Named(NamedColor::Red));
        assert!(matches!(red, Color::Indexed { value: _ }));
    }

    #[test]
    fn flags_fold_into_compact_attrs() {
        let flags = AlacrittyFlags::BOLD
            | AlacrittyFlags::ITALIC
            | AlacrittyFlags::UNDERLINE
            | AlacrittyFlags::DOUBLE_UNDERLINE
            | AlacrittyFlags::INVERSE
            | AlacrittyFlags::STRIKEOUT;
        let attrs = CellAttrs::from(flags);
        // Underline & double-underline both fold into a single
        // UNDERLINE bit; the dimmer / hidden bits should stay clear.
        assert!(attrs.contains(CellAttrs::BOLD));
        assert!(attrs.contains(CellAttrs::ITALIC));
        assert!(attrs.contains(CellAttrs::UNDERLINE));
        assert!(attrs.contains(CellAttrs::INVERSE));
        assert!(attrs.contains(CellAttrs::STRIKETHROUGH));
        assert!(!attrs.contains(CellAttrs::DIM));
        assert!(!attrs.contains(CellAttrs::HIDDEN));
    }

    #[test]
    fn cursor_style_serialises_as_snake_case() {
        let json = serde_json::to_string(&CursorStyle::Underline).unwrap();
        assert_eq!(json, "\"underline\"");
    }

    #[test]
    fn cell_serialises_with_flat_fields() {
        let cell = Cell {
            ch: 'A',
            fg: Color::Rgb { value: 0xff00ff },
            bg: Color::Default,
            attrs: CellAttrs::BOLD | CellAttrs::UNDERLINE,
        };
        let json = serde_json::to_value(cell).unwrap();
        // The shape here is the wire contract with the frontend
        // GridTerminal component — pin it down so we notice any
        // accidental rename / restructure.
        assert_eq!(json["ch"], "A");
        assert_eq!(json["bg"]["kind"], "default");
        assert_eq!(json["fg"]["kind"], "rgb");
        assert_eq!(json["fg"]["value"], 0xff00ff);
        assert_eq!(json["attrs"], 0b0000_0101); // BOLD | UNDERLINE
    }
}
