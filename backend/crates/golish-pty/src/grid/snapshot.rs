//! Wire format the backend ships to the `GridTerminal` React component
//! whenever the virtual terminal changes. See
//! `docs/design/2026-05-15-grid-terminal-phase-b.md` §3 for the
//! protocol overview; this file is the single source of truth for the
//! JSON shape.
//!
//! Two flavours of payload:
//!
//! * `full = true` — every row of the viewport is included in
//!   `dirty_rows`. Sent on first subscribe, after [`Self::resize`], or
//!   when a diff would have been ≥ the same number of bytes as a full
//!   snapshot.
//! * `full = false` — only `dirty_rows` indices since the previous
//!   snapshot are included. Frontend overlays them onto its cached
//!   grid.
//!
//! `rev` is a monotonic counter so the frontend can detect dropped
//! events (and request a full snapshot via the companion
//! `request_grid_snapshot` Tauri command).

use serde::Serialize;

use super::cell::{Cell, Cursor};

/// One frame of grid state shipped to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct GridUpdate {
    /// Monotonic revision number; frontend uses this to detect
    /// non-contiguous deliveries.
    pub rev: u64,
    pub cols: u16,
    pub rows: u16,
    /// `true` when `dirty_rows` covers the entire viewport (no prior
    /// state needed). `false` when this is an incremental diff and the
    /// frontend should merge `dirty_rows` into its cached grid.
    pub full: bool,
    pub dirty_rows: Vec<RowUpdate>,
    pub cursor: Cursor,
    /// Whether the underlying terminal is on its alternate screen
    /// buffer right now. Frontend toggles GridTerminal visibility based
    /// on this — non-alt sessions stay in Block UI.
    pub alt_screen: bool,
    /// Whether DEC mode 1 (`APP_CURSOR`) is active. Arrow keys must
    /// be encoded as `ESC O <X>` rather than `ESC [ <X>` while this
    /// is true; consumed by `frontend/components/GridTerminal/keymap.ts`.
    pub app_cursor_mode: bool,
}

/// Cells of a single grid row, indexed by `y` (top of viewport = 0).
#[derive(Debug, Clone, Serialize)]
pub struct RowUpdate {
    pub y: u16,
    pub cells: Vec<Cell>,
}

impl GridUpdate {
    /// True when the payload contains nothing the frontend has to
    /// process. Used by the emitter to skip wire writes during quiet
    /// periods.
    pub fn is_noop(&self) -> bool {
        !self.full && self.dirty_rows.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::cell::{CellAttrs, Color, CursorStyle};

    fn dummy_cell(ch: char) -> Cell {
        Cell {
            ch,
            fg: Color::Default,
            bg: Color::Default,
            attrs: CellAttrs::empty(),
        }
    }

    fn dummy_update(full: bool) -> GridUpdate {
        GridUpdate {
            rev: 1,
            cols: 80,
            rows: 24,
            full,
            dirty_rows: Vec::new(),
            cursor: Cursor::default(),
            alt_screen: true,
            app_cursor_mode: false,
        }
    }

    #[test]
    fn empty_diff_is_noop() {
        let update = dummy_update(false);
        assert!(update.is_noop());
    }

    #[test]
    fn full_snapshot_is_never_noop() {
        let update = dummy_update(true);
        assert!(!update.is_noop());
    }

    #[test]
    fn diff_with_rows_is_not_noop() {
        let update = GridUpdate {
            rev: 1,
            cols: 2,
            rows: 1,
            full: false,
            dirty_rows: vec![RowUpdate {
                y: 0,
                cells: vec![dummy_cell('h'), dummy_cell('i')],
            }],
            cursor: Cursor::default(),
            alt_screen: true,
            app_cursor_mode: false,
        };
        assert!(!update.is_noop());
    }

    #[test]
    fn wire_shape_pins_field_names() {
        let update = GridUpdate {
            rev: 42,
            cols: 80,
            rows: 24,
            full: true,
            dirty_rows: vec![RowUpdate {
                y: 0,
                cells: vec![dummy_cell('A')],
            }],
            cursor: Cursor {
                x: 5,
                y: 2,
                visible: true,
                style: CursorStyle::Block,
            },
            alt_screen: true,
            app_cursor_mode: true,
        };
        let json = serde_json::to_value(&update).unwrap();
        // Pinning field names so a refactor that renames anything
        // breaks this test before it ever reaches the frontend.
        assert_eq!(json["rev"], 42);
        assert_eq!(json["cols"], 80);
        assert_eq!(json["rows"], 24);
        assert_eq!(json["full"], true);
        assert_eq!(json["alt_screen"], true);
        assert_eq!(json["app_cursor_mode"], true);
        assert_eq!(json["cursor"]["x"], 5);
        assert_eq!(json["cursor"]["y"], 2);
        assert_eq!(json["cursor"]["style"], "block");
        assert_eq!(json["dirty_rows"][0]["y"], 0);
        assert_eq!(json["dirty_rows"][0]["cells"][0]["ch"], "A");
    }
}
