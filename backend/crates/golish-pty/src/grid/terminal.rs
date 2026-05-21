//! Thin wrapper over [`alacritty_terminal::Term`] that exposes only
//! what the GridTerminal frontend protocol needs: feed PTY bytes in,
//! get serialisable [`GridUpdate`]s out.
//!
//! Keeping alacritty behind this wrapper means future renderer changes
//! (e.g. switching from `alacritty_terminal` to `vt100-ctt` as a
//! fallback) don't ripple out through the entire emitter pipeline.

use alacritty_terminal::event::{Event, EventListener, VoidListener};
use alacritty_terminal::grid::{Dimensions, Indexed};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::{test::TermSize, Config, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, Processor};
use alacritty_terminal::Term;

use super::cell::{Cell, CellAttrs, Color, Cursor, CursorStyle};
use super::snapshot::{GridUpdate, RowUpdate};

/// Frontend-supplied viewport dimensions. `cols` and `rows` are
/// validated against [`MIN_COLS`] / [`MIN_ROWS`] before being applied
/// to the alacritty term — see [`GridTerminal::new`] and
/// [`GridTerminal::resize`].
#[derive(Debug, Clone, Copy)]
pub struct GridDims {
    pub cols: u16,
    pub rows: u16,
}

const MIN_COLS: u16 = 2;
const MIN_ROWS: u16 = 1;
const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

/// Wraps one `alacritty_terminal::Term<VoidListener>` plus the
/// associated vte ANSI parser. Single-threaded: callers wrap this in a
/// `Mutex` (see [`super::GridManager`]).
pub struct GridTerminal {
    term: Term<VoidListener>,
    parser: Processor,
    /// Bumped on every successful [`Self::write`] / [`Self::resize`]
    /// so diff consumers can detect missed frames.
    rev: u64,
    /// True when the *previous* snapshot consumer received a "full"
    /// update; flipped back to false after the next diff is taken.
    served_full_since_creation: bool,
}

impl GridTerminal {
    pub fn new(dims: GridDims) -> Self {
        let (cols, rows) = clamp_dims(dims);
        let size = TermSize::new(cols.into(), rows.into());
        let config = Config {
            scrolling_history: DEFAULT_SCROLLBACK_LINES,
            ..Default::default()
        };

        Self {
            term: Term::new(config, &size, VoidListener),
            parser: Processor::new(),
            rev: 0,
            served_full_since_creation: false,
        }
    }

    /// Push raw PTY output bytes through the parser → term state
    /// machine. Bumps [`Self::rev`] when any bytes are processed.
    pub fn write(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.parser.advance(&mut self.term, bytes);
        self.rev = self.rev.saturating_add(1);
    }

    /// Resize the terminal to a new viewport. No-ops when the requested
    /// dimensions match the current grid.
    pub fn resize(&mut self, dims: GridDims) {
        let (cols, rows) = clamp_dims(dims);
        let current_cols = self.term.columns() as u16;
        let current_rows = self.term.screen_lines() as u16;
        if current_cols == cols && current_rows == rows {
            return;
        }
        self.term.resize(TermSize::new(cols.into(), rows.into()));
        self.rev = self.rev.saturating_add(1);
        // Resize implicitly invalidates the whole grid; force the next
        // snapshot consumer onto the full-snapshot path so the frontend
        // doesn't try to apply a partial diff against a smaller grid.
        self.served_full_since_creation = false;
    }

    /// Current grid width (columns).
    pub fn cols(&self) -> u16 {
        self.term.columns() as u16
    }

    /// Current grid height (rows).
    pub fn rows(&self) -> u16 {
        self.term.screen_lines() as u16
    }

    /// Current monotonic revision number; advances on every [`Self::write`]
    /// / [`Self::resize`].
    pub fn rev(&self) -> u64 {
        self.rev
    }

    /// Whether the terminal is currently on the alternate screen
    /// buffer. Most TUIs (`vim`, `htop`, …) only need GridTerminal
    /// rendering while this is true — the regular Block UI handles the
    /// non-alt case just fine.
    pub fn alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// Whether DEC mode 1 (application cursor) is active. Frontend
    /// uses this to choose between `ESC [ X` (normal) and `ESC O X`
    /// (application) when encoding arrow keys.
    pub fn app_cursor_mode(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    /// Emit a *full* snapshot of the current grid. Always safe to call;
    /// expensive (O(rows × cols)) so callers should prefer
    /// [`Self::snapshot_diff`] once an initial baseline has been sent.
    pub fn snapshot_full(&mut self) -> GridUpdate {
        let cols = self.cols();
        let rows = self.rows();
        let mut row_updates = Vec::with_capacity(rows as usize);

        for y in 0..rows {
            let cells = self.row_cells(y);
            row_updates.push(RowUpdate { y, cells });
        }

        // Reset alacritty's damage tracker so the next diff doesn't
        // double-emit everything we just sent.
        self.term.reset_damage();
        self.served_full_since_creation = true;

        GridUpdate {
            rev: self.rev,
            cols,
            rows,
            full: true,
            dirty_rows: row_updates,
            cursor: self.cursor_snapshot(),
            alt_screen: self.alt_screen(),
            app_cursor_mode: self.app_cursor_mode(),
        }
    }

    /// Incremental snapshot since the previous [`Self::snapshot_diff`]
    /// or [`Self::snapshot_full`] call. The first call on a fresh
    /// terminal is upgraded to a full snapshot so the frontend always
    /// has a baseline.
    pub fn snapshot_diff(&mut self) -> GridUpdate {
        if !self.served_full_since_creation {
            return self.snapshot_full();
        }

        let cols = self.cols();
        let rows = self.rows();

        // Pull damage out of alacritty before we mutate anything else;
        // `reset_damage` consumes it.
        let damaged_lines: Vec<u16> = collect_damaged_rows(&mut self.term);
        let cursor = self.cursor_snapshot();

        let mut dirty_rows = Vec::with_capacity(damaged_lines.len());
        for y in damaged_lines {
            if y >= rows {
                continue;
            }
            dirty_rows.push(RowUpdate {
                y,
                cells: self.row_cells(y),
            });
        }

        self.term.reset_damage();

        GridUpdate {
            rev: self.rev,
            cols,
            rows,
            full: false,
            dirty_rows,
            cursor,
            alt_screen: self.alt_screen(),
            app_cursor_mode: self.app_cursor_mode(),
        }
    }

    fn cursor_snapshot(&self) -> Cursor {
        let grid = self.term.grid();
        let point: Point<Line> = grid.cursor.point;
        let y = point.line.0.max(0) as u16;
        let x = point.column.0 as u16;
        let visible = self.term.mode().contains(TermMode::SHOW_CURSOR);
        let style = match self.term.cursor_style().shape {
            CursorShape::Block => CursorStyle::Block,
            CursorShape::Underline => CursorStyle::Underline,
            CursorShape::Beam => CursorStyle::Bar,
            CursorShape::HollowBlock => CursorStyle::Block,
            CursorShape::Hidden => CursorStyle::Block,
        };
        Cursor {
            x,
            y,
            visible,
            style,
        }
    }

    fn row_cells(&self, y: u16) -> Vec<Cell> {
        let cols = self.cols() as usize;
        let grid = self.term.grid();
        let line = Line(y as i32);
        let mut row = Vec::with_capacity(cols);
        for col in 0..cols {
            let alacritty_cell = &grid[line][Column(col)];
            row.push(Cell {
                ch: alacritty_cell.c,
                fg: Color::from(alacritty_cell.fg),
                bg: Color::from(alacritty_cell.bg),
                attrs: CellAttrs::from(alacritty_cell.flags),
            });
        }
        row
    }
}

fn clamp_dims(dims: GridDims) -> (u16, u16) {
    (dims.cols.max(MIN_COLS), dims.rows.max(MIN_ROWS))
}

fn collect_damaged_rows<L: EventListener>(term: &mut Term<L>) -> Vec<u16> {
    use alacritty_terminal::term::TermDamage;

    let mut out = Vec::new();
    match term.damage() {
        TermDamage::Full => {
            let rows = term.screen_lines() as u16;
            out.extend(0..rows);
        }
        TermDamage::Partial(iter) => {
            for line in iter {
                if line.line <= u16::MAX as usize {
                    out.push(line.line as u16);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

// Silence unused warning for VoidListener::send_event — needed for the
// trait bound on `collect_damaged_rows` but never actually fires
// because we always use `VoidListener`.
#[allow(dead_code)]
fn _assert_void_listener_compat() {
    fn _take<L: EventListener>(_: &L) {}
    let v = VoidListener;
    _take(&v);
    let _ = Event::Wakeup;
    let _: Indexed<&char> = Indexed {
        point: Point::new(Line(0), Column(0)),
        cell: &'x',
    };
}
