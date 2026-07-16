//! Where the browse screen's panels are, worked out once per frame.
//!
//! The geometry used to be discovered by drawing: the renderer split the screen,
//! wrote the pane rectangles and the details bounds back into `App` through
//! `Cell`, and mouse and scroll handling read them afterwards. That made "render
//! has run at least once, against this terminal size, with this row selected" a
//! precondition of handling a click — a temporal interface no signature stated
//! and no test could fail on. A click arriving before the first frame, or after
//! a resize, was hit-tested against a stale rectangle.
//!
//! A [`LayoutSnapshot`] is that geometry as a value instead. It is a pure
//! function of the terminal area and the [`Content`] being shown, so the event
//! loop can compute it *before* both drawing and input, and hand the same one to
//! each. Render reads it; nothing writes back.
//!
//! Modal geometry is deliberately not here: pickers and the help overlay are
//! centred on the whole terminal area rather than on a panel, and their windows
//! are the viewport module's business.

use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};

use super::viewport::Window;

/// How much there is to show, as the numbers layout actually needs: the rows of
/// the active list, and the lines of the details pane for the selected row.
///
/// This is what makes the snapshot pure. Both counts are properties of app
/// state, so they are computed by the caller that owns that state, and the
/// geometry below follows from them without consulting anything else.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Content {
    /// Rows in the left-hand list: logical services, hosts, types, or commands.
    pub(crate) list_total: usize,
    /// Lines the details pane has for the current selection.
    pub(crate) details_total: usize,
}

/// The browse screen's panels and the bounds that follow from them.
///
/// Constructed only by [`LayoutSnapshot::compute`], so every field is consistent
/// with one terminal area and one view of the content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LayoutSnapshot {
    area: Rect,
    top_bar: Rect,
    filter_bar: Rect,
    list: Rect,
    details: Rect,
    footer: Rect,
    content: Content,
}

impl Default for LayoutSnapshot {
    /// The layout of a terminal with no room in it. Every rectangle is empty and
    /// every viewport is zero, so an event handled before the first frame hits
    /// nothing rather than hitting whatever a `Rect::default()` happens to
    /// overlap.
    fn default() -> Self {
        Self::compute(Rect::default(), Content::default())
    }
}

impl LayoutSnapshot {
    /// The panels of a `area`-sized terminal showing `content`.
    ///
    /// Total: this is the only place the browse screen is divided up. A zero-
    /// or one-row terminal yields empty rectangles and zero-height viewports
    /// rather than an underflow, so the callers below need no size special
    /// cases of their own.
    pub(crate) fn compute(area: Rect, content: Content) -> Self {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // top stats bar
                Constraint::Length(1), // filter / search bar
                Constraint::Min(6),    // body
                Constraint::Length(1), // footer hints
            ])
            .split(area);
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(chunks[2]);

        Self {
            area,
            top_bar: chunks[0],
            filter_bar: chunks[1],
            list: body[0],
            details: body[1],
            footer: chunks[3],
            content,
        }
    }

    pub(crate) fn area(self) -> Rect {
        self.area
    }

    pub(crate) fn top_bar(self) -> Rect {
        self.top_bar
    }

    pub(crate) fn filter_bar(self) -> Rect {
        self.filter_bar
    }

    pub(crate) fn list(self) -> Rect {
        self.list
    }

    pub(crate) fn details(self) -> Rect {
        self.details
    }

    pub(crate) fn footer(self) -> Rect {
        self.footer
    }

    /// The window of list rows on screen when `selected` is focused. Both the
    /// renderer's row slice and the click hit test below come from this, so a
    /// click lands on the row the user sees by construction.
    pub(crate) fn list_window(self, selected: usize) -> Window {
        Window::containing(self.content.list_total, viewport_of(self.list), selected)
    }

    /// The window of details lines on screen at `scroll`.
    pub(crate) fn details_window(self, scroll: usize) -> Window {
        Window::at(self.content.details_total, self.details_viewport(), scroll)
    }

    /// Visible height of the details pane, which is how far a half-page scroll
    /// goes.
    pub(crate) fn details_viewport(self) -> usize {
        viewport_of(self.details)
    }

    /// The largest `details_scroll` that still shows content. Zero when the
    /// details fit, so scrolling a short pane simply does nothing.
    pub(crate) fn details_max_scroll(self) -> usize {
        Window::max_scroll(self.content.details_total, self.details_viewport())
    }

    /// The list index under `position` while `selected` is focused, or `None`
    /// when the point is on a border, past the last row, or outside the panel
    /// altogether.
    pub(crate) fn list_row_at(self, position: Position, selected: usize) -> Option<usize> {
        if !self.list.contains(position) {
            return None;
        }
        // The first content row sits below the top border; the bottom border and
        // anything past the last row are not selectable.
        let row = (position.y.checked_sub(self.list.y + 1))? as usize;
        let window = self.list_window(selected);
        let index = window.offset() + row;
        window.range().contains(&index).then_some(index)
    }

    /// Whether `position` is over the details pane, and so scrolls it rather
    /// than moving the list selection.
    pub(crate) fn is_over_details(self, position: Position) -> bool {
        self.details.contains(position)
    }
}

/// The rows a bordered panel has for content. Saturating, so a panel too short
/// to draw its own borders reports no room instead of wrapping around.
fn viewport_of(panel: Rect) -> usize {
    panel.height.saturating_sub(2) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(list_total: usize, details_total: usize) -> Content {
        Content {
            list_total,
            details_total,
        }
    }

    /// The reference fixture the app and render tests hit-test against: a
    /// 100×30 terminal, whose body leaves a bordered list panel on the left.
    fn normal() -> LayoutSnapshot {
        LayoutSnapshot::compute(Rect::new(0, 0, 100, 30), content(8, 40))
    }

    #[test]
    fn the_panels_of_a_normal_terminal_tile_it_without_overlapping() {
        let layout = normal();

        assert_eq!(layout.top_bar(), Rect::new(0, 0, 100, 1));
        assert_eq!(layout.filter_bar(), Rect::new(0, 1, 100, 1));
        assert_eq!(layout.footer(), Rect::new(0, 29, 100, 1));
        // The body fills what is left, split 58/42 between list and details.
        assert_eq!(layout.list(), Rect::new(0, 2, 58, 27));
        assert_eq!(layout.details(), Rect::new(58, 2, 42, 27));
    }

    #[test]
    fn the_details_bounds_follow_the_pane_and_its_content() {
        let layout = normal();

        // 27 rows of pane, less two borders, over 40 lines of content.
        assert_eq!(layout.details_viewport(), 25);
        assert_eq!(layout.details_max_scroll(), 15);
        assert_eq!(layout.details_window(0).range(), 0..25);
        assert_eq!(layout.details_window(15).range(), 15..40);
        // A scroll left over from taller content cannot show blank rows.
        assert_eq!(layout.details_window(99).range(), 15..40);
    }

    #[test]
    fn details_that_fit_cannot_be_scrolled() {
        let layout = LayoutSnapshot::compute(Rect::new(0, 0, 100, 30), content(8, 5));

        assert_eq!(layout.details_max_scroll(), 0);
        assert_eq!(layout.details_window(0).range(), 0..5);
        assert!(!layout.details_window(0).is_clipped());
    }

    #[test]
    fn a_click_on_a_content_row_finds_the_row_the_window_drew_there() {
        let layout = normal();

        // Content starts one row below the panel's top border.
        assert_eq!(layout.list_row_at(Position::new(5, 3), 0), Some(0));
        assert_eq!(layout.list_row_at(Position::new(5, 6), 0), Some(3));
        // The window follows the selection, so the same point is a different
        // row once the list has scrolled. Nine rows leave the panel four to
        // list twenty in, so the last of them sits on a window from 16.
        let scrolled = LayoutSnapshot::compute(Rect::new(0, 0, 100, 9), content(20, 40));
        assert_eq!(scrolled.list_window(19).offset(), 16);
        assert_eq!(scrolled.list_row_at(Position::new(5, 3), 19), Some(16));
        assert_eq!(scrolled.list_row_at(Position::new(5, 6), 19), Some(19));
        // The window ends at the content, so the row below the last is not a row.
        assert_eq!(scrolled.list_row_at(Position::new(5, 7), 19), None);
    }

    #[test]
    fn clicks_off_the_rows_select_nothing() {
        let layout = normal();

        // Top border, the details pane, and above the body entirely.
        assert_eq!(layout.list_row_at(Position::new(5, 2), 0), None);
        assert_eq!(layout.list_row_at(Position::new(70, 6), 0), None);
        assert_eq!(layout.list_row_at(Position::new(5, 0), 0), None);
        // Past the last of the eight rows, on a panel with room for 25.
        assert_eq!(layout.list_row_at(Position::new(5, 11), 0), None);
        // The bottom border.
        assert_eq!(layout.list_row_at(Position::new(5, 29), 0), None);
    }

    #[test]
    fn the_details_pane_answers_for_its_own_wheel_events() {
        let layout = normal();

        assert!(layout.is_over_details(Position::new(70, 6)));
        assert!(!layout.is_over_details(Position::new(5, 6)));
        assert!(!layout.is_over_details(Position::new(70, 0)));
    }

    /// A terminal too small for the panels must still produce a snapshot that
    /// answers every question safely — nothing may underflow, no point may hit
    /// a row that was never drawn, and every window must be empty rather than
    /// wrong.
    #[test]
    fn a_zero_size_terminal_is_safe_to_hit_test_and_scroll() {
        let layout = LayoutSnapshot::compute(Rect::new(0, 0, 0, 0), content(8, 40));

        assert_eq!(layout.details_viewport(), 0);
        // Nothing fits in a pane with no rows, so every line is "below" the
        // window. Scrolling there moves an offset over content nobody can see;
        // the window stays empty, and the first snapshot with room in it pulls
        // the offset back to something meaningful.
        assert_eq!(layout.details_max_scroll(), 40);
        assert!(layout.details_window(40).range().is_empty());
        assert!(layout.details_window(0).range().is_empty());
        assert!(layout.list_window(3).range().is_empty());
        assert_eq!(layout.list_row_at(Position::new(0, 0), 0), None);
        assert!(!layout.is_over_details(Position::new(0, 0)));
    }

    #[test]
    fn the_default_snapshot_is_the_zero_size_one() {
        let default = LayoutSnapshot::default();

        assert_eq!(
            default,
            LayoutSnapshot::compute(Rect::default(), Content::default())
        );
        // Which is what makes an event handled before the first frame land on
        // nothing at all.
        assert_eq!(default.list_row_at(Position::new(0, 0), 0), None);
        assert_eq!(default.details_max_scroll(), 0);
    }

    /// Panels squeezed below their own borders report no room rather than
    /// wrapping a `height - 2` around to a huge viewport.
    #[test]
    fn panels_shorter_than_their_borders_have_no_viewport() {
        for height in 0..=6u16 {
            let layout = LayoutSnapshot::compute(Rect::new(0, 0, 40, height), content(8, 40));
            assert!(
                layout.details_viewport() <= 4,
                "height={height} viewport={}",
                layout.details_viewport()
            );
            assert!(layout.list_window(7).range().end <= 8);
            assert!(layout.details_window(99).range().end <= 40);
        }
    }

    /// Resizing is a new snapshot, not a patched one: the same state at a new
    /// size answers with the new geometry and re-clamped bounds.
    #[test]
    fn a_resize_produces_a_snapshot_with_the_new_bounds() {
        let tall = LayoutSnapshot::compute(Rect::new(0, 0, 100, 30), content(8, 40));
        let short = LayoutSnapshot::compute(Rect::new(0, 0, 60, 18), content(8, 40));

        assert_eq!(tall.details_max_scroll(), 15);
        // Fewer rows on screen, so there is more content hidden below.
        assert_eq!(short.details_viewport(), 13);
        assert_eq!(short.details_max_scroll(), 27);
        assert_eq!(short.list(), Rect::new(0, 2, 35, 15));
        assert_eq!(short.details(), Rect::new(35, 2, 25, 15));
        // A point inside the old details pane can be outside the new one.
        assert!(tall.is_over_details(Position::new(70, 6)));
        assert!(!short.is_over_details(Position::new(70, 6)));
    }
}
