//! End-to-end tests for the GridTerminal wrapper.
//!
//! These exercise the `alacritty_terminal` integration through the
//! public [`super::GridTerminal`] API only — internals are deliberately
//! untouched so we notice the day Alacritty changes them and our
//! wrapper still works.

use super::cell::Color;
use super::{GridDims, GridManager, GridTerminal};

fn make() -> GridTerminal {
    GridTerminal::new(GridDims { cols: 10, rows: 3 })
}

fn cell_at(update: &super::GridUpdate, y: u16, x: usize) -> &super::Cell {
    let row = update
        .dirty_rows
        .iter()
        .find(|r| r.y == y)
        .expect("row should be in update");
    &row.cells[x]
}

#[test]
fn fresh_terminal_starts_at_rev_zero() {
    let term = make();
    assert_eq!(term.rev(), 0);
    assert!(!term.alt_screen());
}

#[test]
fn first_snapshot_is_always_full_even_when_requested_as_diff() {
    // The frontend may call `snapshot_diff` first by accident — we
    // promote it to a full snapshot so it has a baseline to merge
    // against.
    let mut term = make();
    let update = term.snapshot_diff();
    assert!(update.full, "first diff should upgrade to full");
    assert_eq!(update.dirty_rows.len(), term.rows() as usize);
}

#[test]
fn write_lands_printable_chars_in_grid() {
    let mut term = make();
    term.write(b"hi");
    let update = term.snapshot_full();
    assert_eq!(cell_at(&update, 0, 0).ch, 'h');
    assert_eq!(cell_at(&update, 0, 1).ch, 'i');
    assert_eq!(cell_at(&update, 0, 2).ch, ' ');
}

#[test]
fn write_advances_rev() {
    let mut term = make();
    let before = term.rev();
    term.write(b"x");
    assert_eq!(term.rev(), before + 1);
}

#[test]
fn empty_write_does_not_advance_rev() {
    let mut term = make();
    let before = term.rev();
    term.write(b"");
    assert_eq!(term.rev(), before);
}

#[test]
fn cursor_moves_with_input() {
    let mut term = make();
    term.write(b"ab");
    let update = term.snapshot_full();
    assert_eq!(update.cursor.x, 2);
    assert_eq!(update.cursor.y, 0);
}

#[test]
fn diff_after_quiet_period_is_small() {
    let mut term = make();
    // Take baseline.
    let _ = term.snapshot_full();

    // Write nothing → diff should be effectively a noop. Alacritty may
    // still report the cursor row as "damaged" the first call after
    // reset (it tracks cursor moves separately from cell mutations);
    // we accept up to one redundant row but no more.
    let update = term.snapshot_diff();
    assert!(!update.full, "should not be promoted to full");
    assert!(
        update.dirty_rows.len() <= 1,
        "expected at most 1 dirty row after a quiet period, got {}",
        update.dirty_rows.len()
    );
}

#[test]
fn diff_reports_only_touched_row_after_write() {
    let mut term = make();
    let _ = term.snapshot_full();

    term.write(b"yo");
    let update = term.snapshot_diff();
    assert!(!update.full);
    let touched: Vec<u16> = update.dirty_rows.iter().map(|r| r.y).collect();
    assert!(
        touched.contains(&0),
        "row 0 should be dirty after write, got rows {:?}",
        touched
    );
}

#[test]
fn alt_screen_flag_flips_on_csi_1049() {
    let mut term = make();
    assert!(!term.alt_screen());
    // CSI ? 1049 h enters alt-screen (vim, htop, less, …).
    term.write(b"\x1b[?1049h");
    assert!(term.alt_screen());
    term.write(b"\x1b[?1049l");
    assert!(!term.alt_screen());
}

#[test]
fn sgr_color_lands_on_cell() {
    let mut term = make();
    // SGR 31 = red foreground.
    term.write(b"\x1b[31mR\x1b[0m");
    let update = term.snapshot_full();
    let cell = cell_at(&update, 0, 0);
    assert_eq!(cell.ch, 'R');
    assert!(
        matches!(cell.fg, Color::Indexed { value: 1 }),
        "expected red foreground (named 1), got {:?}",
        cell.fg
    );
}

#[test]
fn truecolor_sgr_lands_on_cell() {
    let mut term = make();
    // SGR 38 ; 2 ; r ; g ; b = truecolor fg.
    term.write(b"\x1b[38;2;16;32;48mX\x1b[0m");
    let update = term.snapshot_full();
    let cell = cell_at(&update, 0, 0);
    let expected = (16u32 << 16) | (32u32 << 8) | 48u32;
    assert_eq!(cell.fg, Color::Rgb { value: expected });
}

#[test]
fn resize_changes_dimensions_and_returns_full_baseline() {
    let mut term = make();
    let _ = term.snapshot_full();
    term.resize(GridDims { cols: 20, rows: 5 });
    assert_eq!(term.cols(), 20);
    assert_eq!(term.rows(), 5);
    // After a resize the next snapshot must be promoted to full so
    // the frontend doesn't try to diff against the old smaller grid.
    let update = term.snapshot_diff();
    assert!(update.full);
    assert_eq!(update.dirty_rows.len(), 5);
}

#[test]
fn resize_clamps_to_minimum_dimensions() {
    let term = GridTerminal::new(GridDims { cols: 0, rows: 0 });
    assert!(term.cols() >= 2);
    assert!(term.rows() >= 1);
}

#[test]
fn manager_creates_and_disposes_sessions() {
    let mgr = GridManager::new();
    assert_eq!(mgr.len(), 0);

    let term = mgr.get_or_create("sess-1", GridDims { cols: 80, rows: 24 });
    assert_eq!(mgr.len(), 1);
    assert_eq!(term.lock().cols(), 80);

    // Second call returns the same instance (rev should be shared).
    term.lock().write(b"hi");
    let rev_after_write = term.lock().rev();
    let term2 = mgr.get_or_create("sess-1", GridDims { cols: 1, rows: 1 });
    assert_eq!(term2.lock().rev(), rev_after_write);
    assert_eq!(term2.lock().cols(), 80);
    assert_eq!(mgr.len(), 1);

    mgr.dispose("sess-1");
    assert_eq!(mgr.len(), 0);
    assert!(mgr.get("sess-1").is_none());
}
