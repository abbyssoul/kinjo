//! The visible window over a taller list.
//!
//! Every scrollable surface — the browse panels, the four pickers, and the help
//! overlay — shows `height` rows of `total`. Which rows those are is decided
//! here, once, so that "the selected row is on screen" is a property of one
//! calculation rather than a habit four render paths have to keep up.
//!
//! A [`Window`] is derived from content plus geometry and nothing else. It is
//! computed for the frame being drawn, so a resize simply produces a different
//! window from the same state; there is no offset to keep in sync and no state
//! for a renderer to write back.

use std::ops::Range;

/// The slice of a list that is currently visible: `offset..offset + len`.
///
/// Constructed only through the two anchors below, both of which guarantee the
/// window lies inside `0..total`, so `range()` is always a valid slice index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    offset: usize,
    len: usize,
    total: usize,
}

impl Window {
    /// The window of `height` rows over `total` rows that shows `selected`.
    ///
    /// This is the invariant the pickers rely on: whatever the selected index
    /// and however short the terminal, the returned window contains it. Moving
    /// within the window leaves the window alone; moving past an edge scrolls by
    /// exactly enough to bring the selection back to that edge, so navigation
    /// never jumps further than the eye can follow.
    ///
    /// A `selected` beyond `total` cannot pull the window past the content: the
    /// offset is clamped to the last full window either way.
    pub(crate) fn containing(total: usize, height: usize, selected: usize) -> Self {
        // Everything fits, there is nowhere to look, or the selection is already
        // within the first window: all three show the content from the top.
        let offset = if height == 0 || total <= height || selected < height {
            0
        } else {
            (selected + 1 - height).min(total - height)
        };
        Self::new(total, height, offset)
    }

    /// The window of `height` rows over `total` rows starting at `scroll`.
    ///
    /// For content that is read rather than selected from (the help overlay).
    /// `scroll` is clamped to [`Window::max_scroll`], so an offset left over
    /// from a taller terminal — or from scrolling and then resizing — can never
    /// show a window of blank rows past the end.
    pub(crate) fn at(total: usize, height: usize, scroll: usize) -> Self {
        Self::new(total, height, scroll.min(Self::max_scroll(total, height)))
    }

    fn new(total: usize, height: usize, offset: usize) -> Self {
        Self {
            offset,
            len: height.min(total.saturating_sub(offset)),
            total,
        }
    }

    /// The largest offset that still fills the window with content. Zero when
    /// everything fits, so a caller clamping a scroll action needs no special
    /// case for short content or a zero-height viewport.
    pub(crate) fn max_scroll(total: usize, height: usize) -> usize {
        total.saturating_sub(height)
    }

    pub(crate) fn offset(self) -> usize {
        self.offset
    }

    /// How much content the window is a view of.
    pub(crate) fn total(self) -> usize {
        self.total
    }

    /// The visible indices. Empty for empty content or a zero-height viewport,
    /// which is what makes those cases render as nothing rather than underflow.
    pub(crate) fn range(self) -> Range<usize> {
        self.offset..self.offset + self.len
    }

    /// Whether content is hidden above or below. The only condition under which
    /// a scrollbar or range indicator has anything true to say.
    pub(crate) fn is_clipped(self) -> bool {
        self.len < self.total
    }

    /// The window as `first-last/total`, 1-based and inclusive, for a title
    /// chip. `None` when there is no content to describe.
    pub(crate) fn range_label(self) -> Option<String> {
        (self.len > 0).then(|| {
            format!(
                "{}-{}/{}",
                self.offset + 1,
                self.offset + self.len,
                self.total
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_over_content_that_fits_shows_all_of_it() {
        let window = Window::containing(5, 10, 4);

        assert_eq!(window.offset(), 0);
        assert_eq!(window.range(), 0..5);
        assert!(!window.is_clipped());
    }

    #[test]
    fn a_selection_inside_the_first_window_does_not_scroll() {
        // Moving down through rows that are already on screen must not shift
        // the list under the user.
        for selected in 0..5 {
            assert_eq!(
                Window::containing(20, 5, selected).offset(),
                0,
                "{selected}"
            );
        }
    }

    #[test]
    fn a_selection_below_the_window_scrolls_it_by_exactly_one_row() {
        assert_eq!(Window::containing(20, 5, 5).offset(), 1);
        assert_eq!(Window::containing(20, 5, 6).offset(), 2);
    }

    #[test]
    fn a_window_never_scrolls_past_the_end_of_the_content() {
        let last = Window::containing(20, 5, 19);

        assert_eq!(last.offset(), 15);
        assert_eq!(last.range(), 15..20);
    }

    /// The safety core: at every index, of every list length, at every height,
    /// the selected row is inside the window that will be drawn.
    #[test]
    fn every_selection_is_visible_at_every_size() {
        for total in 0..25usize {
            for height in 1..12usize {
                for selected in 0..total {
                    let window = Window::containing(total, height, selected);
                    assert!(
                        window.range().contains(&selected),
                        "total={total} height={height} selected={selected} \
                         window={:?}",
                        window.range()
                    );
                    assert!(window.range().end <= total);
                }
            }
        }
    }

    #[test]
    fn an_empty_list_yields_an_empty_window() {
        let window = Window::containing(0, 10, 0);

        assert!(window.range().is_empty());
        assert!(!window.is_clipped());
        assert_eq!(window.range_label(), None);
    }

    /// A modal squeezed to nothing must produce no rows rather than underflow.
    #[test]
    fn a_zero_height_viewport_yields_an_empty_window() {
        let window = Window::containing(20, 0, 7);

        assert!(window.range().is_empty());
        assert!(window.is_clipped());
        assert_eq!(window.range_label(), None);
    }

    #[test]
    fn a_one_row_viewport_shows_the_selected_row_alone() {
        let window = Window::containing(20, 1, 7);

        assert_eq!(window.range(), 7..8);
        assert!(window.is_clipped());
        assert_eq!(window.range_label().as_deref(), Some("8-8/20"));
    }

    /// A selection that is out of bounds is still not allowed to scroll the
    /// window off the content.
    #[test]
    fn a_selection_past_the_end_clamps_to_the_last_window() {
        let window = Window::containing(20, 5, 99);

        assert_eq!(window.range(), 15..20);
    }

    #[test]
    fn a_scrolled_window_starts_where_it_was_asked_to() {
        let window = Window::at(20, 5, 3);

        assert_eq!(window.range(), 3..8);
        assert!(window.is_clipped());
    }

    /// Scrolling to the bottom and then growing the terminal must not leave the
    /// content hanging above a band of blank rows.
    #[test]
    fn a_scrolled_window_is_clamped_to_the_content() {
        assert_eq!(Window::at(20, 5, 99).range(), 15..20);
        assert_eq!(Window::at(20, 30, 15).range(), 0..20);
        assert_eq!(Window::at(0, 5, 3).range(), 0..0);
    }

    #[test]
    fn max_scroll_is_zero_when_everything_fits() {
        assert_eq!(Window::max_scroll(5, 10), 0);
        assert_eq!(Window::max_scroll(20, 5), 15);
        assert_eq!(Window::max_scroll(0, 0), 0);
    }

    #[test]
    fn a_range_label_counts_from_one() {
        assert_eq!(
            Window::containing(42, 10, 0).range_label().as_deref(),
            Some("1-10/42")
        );
        assert_eq!(
            Window::containing(42, 10, 41).range_label().as_deref(),
            Some("33-42/42")
        );
    }
}
