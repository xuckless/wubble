//! F6 and Shift+F6: keyboard focus moves between the window's regions, as Chromium's
//! `BrowserView::CycleFocus` moves it between the toolbar, the bookmarks bar and the page. From
//! the page F6 goes to the address first (wubble's F6 did only that before), then the sidebar's
//! tab list, then back to the page. The host reads where focus is ([`Region::of`]), asks for the
//! next region here and gives it focus. The shell remembers only that F6 put the keys in the
//! sidebar (until something else takes them): anything else that holds them, the web page, an
//! internal page or an overlay over it, is the page.

/// A part of the window keyboard focus can be sent to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// The address (the command bar opened on the visible page's URL).
    Address,
    /// The sidebar: the active tab's row, the rest of the sidebar by Tab from there.
    Sidebar,
    /// The page: web content, or the internal page drawn in its place.
    Page,
}

/// The cycle F6 walks.
const ORDER: [Region; 3] = [Region::Address, Region::Sidebar, Region::Page];

impl Region {
    /// Where focus is: the address while the command bar is open, the sidebar while F6 left
    /// the keys there, else the page (web content, an internal page or an overlay on it).
    pub fn of(sidebar_focused: bool, bar_open: bool) -> Region {
        if bar_open {
            Region::Address
        } else if sidebar_focused {
            Region::Sidebar
        } else {
            Region::Page
        }
    }
}

/// The region after `current` (before it when `back`, Shift+F6). A hidden sidebar is skipped.
pub fn next_region(current: Region, back: bool, sidebar_shown: bool) -> Region {
    let n = ORDER.len();
    let at = ORDER.iter().position(|r| *r == current).unwrap_or(0);
    (1..n)
        .map(|k| ORDER[if back { (at + n - k) % n } else { (at + k) % n }])
        .find(|r| sidebar_shown || *r != Region::Sidebar)
        .unwrap_or(current)
}

#[cfg(test)]
mod tests {
    use super::Region::*;
    use super::*;

    #[test]
    fn f6_cycles_address_sidebar_page_and_wraps() {
        assert_eq!(next_region(Page, false, true), Address);
        assert_eq!(next_region(Address, false, true), Sidebar);
        assert_eq!(next_region(Sidebar, false, true), Page);
    }

    #[test]
    fn shift_f6_walks_back() {
        assert_eq!(next_region(Page, true, true), Sidebar);
        assert_eq!(next_region(Sidebar, true, true), Address);
        assert_eq!(next_region(Address, true, true), Page);
    }

    #[test]
    fn a_hidden_sidebar_is_skipped() {
        assert_eq!(next_region(Page, false, false), Address);
        assert_eq!(next_region(Address, false, false), Page);
        assert_eq!(next_region(Address, true, false), Page);
        assert_eq!(next_region(Page, true, false), Address);
        // focus left on a sidebar that has since been hidden moves on as usual
        assert_eq!(next_region(Sidebar, false, false), Page);
    }

    #[test]
    fn the_open_bar_is_the_address_whatever_else_has_focus() {
        assert_eq!(Region::of(true, true), Address);
        assert_eq!(Region::of(false, true), Address);
        assert_eq!(Region::of(true, false), Sidebar);
    }

    #[test]
    fn keys_outside_the_sidebar_and_bar_are_the_page() {
        // an internal page (settings) or an overlay holds the keys: F6 leaves it for the
        // address, not for the page it is already on
        assert_eq!(Region::of(false, false), Page);
        assert_eq!(next_region(Region::of(false, false), false, true), Address);
    }
}
