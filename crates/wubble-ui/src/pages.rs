//! The native pages' models from the core view-models (`wubble_core::pages`): pure mappings,
//! one function per page, so a page is a function of its view-model. Favicons come in through a
//! lookup the caller owns (the store's rgba rows), so this crate names no store type.

use std::collections::HashSet;

use slint::Image;
use wubble_core::pages::{
    self, BookmarkRow, FlatBookmark, FolderChoice, HistoryLine, LoginView, VaultStatus,
};

use crate::{
    BookmarkLineItem, FolderChoiceItem, HistoryLineItem, LoginItem, PasswordsView, VaultState,
};

/// The history page's lines.
pub fn history_items(lines: &[HistoryLine], icon: impl Fn(&str) -> Image) -> Vec<HistoryLineItem> {
    lines
        .iter()
        .map(|line| match line {
            HistoryLine::Day { label } => HistoryLineItem {
                header: true,
                key: -1,
                label: label.as_str().into(),
                title: Default::default(),
                url: Default::default(),
                favicon: Image::default(),
            },
            HistoryLine::Visit(row) => HistoryLineItem {
                header: false,
                key: id32(row.visit_id),
                label: Default::default(),
                // A page without a title shows its address.
                title: if row.title.is_empty() {
                    &row.url
                } else {
                    &row.title
                }
                .as_str()
                .into(),
                url: row.url.as_str().into(),
                favicon: icon(&row.url),
            },
        })
        .collect()
}

fn id32(id: i64) -> i32 {
    i32::try_from(id).unwrap_or(-1)
}

/// The bookmark tree's visible lines.
pub fn bookmark_items(
    lines: &[FlatBookmark],
    expanded: &HashSet<i64>,
    icon: impl Fn(&str) -> Image,
) -> Vec<BookmarkLineItem> {
    lines
        .iter()
        .map(|l| BookmarkLineItem {
            id: id32(l.item.id),
            depth: i32::try_from(l.depth).unwrap_or(0),
            title: l.item.title.as_str().into(),
            url: l.item.url.as_deref().unwrap_or_default().into(),
            path: Default::default(),
            favicon: l.item.url.as_deref().map(&icon).unwrap_or_default(),
            is_folder: l.item.is_folder,
            expanded: l.item.is_folder && expanded.contains(&l.item.id),
        })
        .collect()
}

/// A search's hits, each with its folder path.
pub fn bookmark_hits(
    hits: &[(BookmarkRow, String)],
    icon: impl Fn(&str) -> Image,
) -> Vec<BookmarkLineItem> {
    hits.iter()
        .map(|(r, path)| BookmarkLineItem {
            id: id32(r.id),
            depth: 0,
            title: r.title.as_str().into(),
            url: r.url.as_deref().unwrap_or_default().into(),
            path: path.as_str().into(),
            favicon: r.url.as_deref().map(&icon).unwrap_or_default(),
            is_folder: false,
            expanded: false,
        })
        .collect()
}

/// The bookmark sheet's folder picker.
pub fn folder_items(choices: &[FolderChoice]) -> Vec<FolderChoiceItem> {
    choices
        .iter()
        .map(|c| FolderChoiceItem {
            id: c.id.map_or(-1, id32),
            label: c.label.as_str().into(),
            depth: i32::try_from(c.depth).unwrap_or(0),
        })
        .collect()
}

/// The passwords page's status card.
pub fn passwords_view(status: &VaultStatus, notice: &str) -> PasswordsView {
    let (title, body) = pages::status_text(status);
    PasswordsView {
        state: match status {
            VaultStatus::Locked => VaultState::Locked,
            VaultStatus::Unlocked => VaultState::Unlocked,
            VaultStatus::Unavailable(_) => VaultState::Unavailable,
            VaultStatus::Unreadable { .. } => VaultState::Unreadable,
        },
        title: title.into(),
        body: body.as_str().into(),
        notice: notice.into(),
    }
}

/// The saved logins; the password text only for `revealed`.
pub fn login_items(rows: &[LoginView], revealed: Option<(&str, &str)>) -> Vec<LoginItem> {
    rows.iter()
        .map(|r| {
            let shown = revealed.filter(|(id, _)| *id == r.id).map(|(_, pw)| pw);
            LoginItem {
                id: r.id.as_str().into(),
                site: r.site().into(),
                origin: r.origin.as_str().into(),
                username: r.username.as_deref().unwrap_or("(cannot be read)").into(),
                readable: r.username.is_some(),
                revealed: shown.unwrap_or_default().into(),
                has_revealed: shown.is_some(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use wubble_core::pages::{HistoryRow, bookmark_tree, flatten};

    use super::*;

    fn visit(url: &str, title: &str) -> HistoryLine {
        HistoryLine::Visit(HistoryRow {
            url: url.into(),
            title: title.into(),
            visit_ms: 1,
            visit_id: 1,
        })
    }

    #[test]
    fn history_lines_map_headers_and_visits() {
        let lines = [
            HistoryLine::Day {
                label: "Today".into(),
            },
            visit("https://a.test/", "A"),
            visit("https://b.test/", ""),
        ];
        let items = history_items(&lines, |_| Image::default());
        assert_eq!(items.len(), 3);
        assert!(items[0].header && items[0].label == "Today");
        assert!(!items[1].header && items[1].title == "A" && items[1].url == "https://a.test/");
        assert_eq!(items[2].title, "https://b.test/");
    }

    fn row(id: i64, parent: Option<i64>, folder: bool, title: &str) -> BookmarkRow {
        BookmarkRow {
            id,
            parent,
            is_folder: folder,
            position: 0,
            title: title.into(),
            url: (!folder).then(|| format!("https://{title}.test/")),
        }
    }

    #[test]
    fn bookmark_lines_carry_depth_and_expansion() {
        let rows = [row(1, None, true, "F"), row(2, Some(1), false, "a")];
        let tree = bookmark_tree(&rows);
        let open = HashSet::from([1]);
        let items = bookmark_items(&flatten(&tree, &open), &open, |_| Image::default());
        assert_eq!(items.len(), 2);
        assert!(items[0].is_folder && items[0].expanded && items[0].depth == 0);
        assert_eq!(
            (items[1].depth, items[1].url.as_str()),
            (1, "https://a.test/")
        );
        let closed = HashSet::new();
        assert_eq!(
            bookmark_items(&flatten(&tree, &closed), &closed, |_| Image::default()).len(),
            1
        );
        let hits = bookmark_hits(&[(rows[1].clone(), "F".into())], |_| Image::default());
        assert_eq!(hits[0].path, "F");
        assert_eq!(folder_items(&pages::folder_choices(&rows, None))[0].id, -1);
    }

    #[test]
    fn a_password_shows_only_for_the_revealed_row() {
        let login = |id: &str| LoginView {
            id: id.into(),
            origin: "https://a.test".into(),
            username: Some("me".into()),
            created_ms: 0,
            last_used_ms: None,
            times_used: 0,
        };
        let rows = [login("1"), login("2")];
        let items = login_items(&rows, Some(("2", "hunter2")));
        assert!(!items[0].has_revealed && items[0].revealed.is_empty());
        assert!(items[1].has_revealed && items[1].revealed == "hunter2");
        assert_eq!(items[0].site, "a.test");
        let none = login_items(&rows, None);
        assert!(none.iter().all(|i| !i.has_revealed));
    }

    #[test]
    fn an_unreadable_login_is_marked_so_it_offers_no_edit() {
        let row = |username: Option<&str>| LoginView {
            id: "1".into(),
            origin: "https://a.test".into(),
            username: username.map(Into::into),
            created_ms: 0,
            last_used_ms: None,
            times_used: 0,
        };
        let items = login_items(&[row(Some("me")), row(None)], None);
        assert!(items[0].readable && items[0].username == "me");
        assert!(!items[1].readable && items[1].username == "(cannot be read)");
    }

    #[test]
    fn the_status_maps_to_a_state() {
        assert_eq!(
            passwords_view(&VaultStatus::Locked, "").state,
            VaultState::Locked
        );
        let v = passwords_view(&VaultStatus::Unreadable { rows: 2 }, "x");
        assert_eq!(v.state, VaultState::Unreadable);
        assert_eq!(v.title, "Saved passwords cannot be read");
        assert_eq!(v.notice, "x");
    }
}
