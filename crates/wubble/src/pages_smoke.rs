//! `--smoke pages`: on a profile seeded before the window opens (three visits on three days, a
//! folder and a link, two downloads, two saved logins under a fake keyring), open each native
//! page through its action and check its rows are drawn from the stored data: history, bookmarks,
//! downloads, then passwords, which shows nothing until its Unlock callback is sent. No page adds
//! a view. Every wait is "until the host sees X", bounded by a count of CEF work runs.

use std::path::Path;

use wubble_core::download::{DownloadEntry, DownloadState};
use wubble_core::pages::InternalPage;
use wubble_store::{BookmarkId, Store, Transition};
use wubble_vault::{FakeKeyRing, NewLogin, Origin, StoreLogins, Vault};
use zeroize::Zeroizing;

use crate::nav_smoke::MAX_WORK_RUNS;
use crate::persist::profile_name;

/// The seeded rows each page must show: history (three visits and a header per day), bookmarks
/// (the folder and the top-level link; the folder is collapsed), downloads, unlocked logins.
pub const HISTORY_LINES: usize = 6;
pub const BOOKMARK_LINES: usize = 2;
pub const DOWNLOADS: usize = 2;
pub const LOGINS: usize = 2;

/// The pages in the order the smoke opens them.
const ORDER: [InternalPage; 4] = [
    InternalPage::History,
    InternalPage::Bookmarks,
    InternalPage::Downloads,
    InternalPage::Passwords,
];

/// What the host sees.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// The window can open views (the frame rate is known).
    pub ready: bool,
    /// The internal page the visible tab shows.
    pub page: Option<InternalPage>,
    /// Rows the shown page draws (login rows for passwords).
    pub rows: usize,
    /// The passwords page shows its unlocked state.
    pub unlocked: bool,
    /// Views the engine holds.
    pub views: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// Asked for `ORDER[i]`; waiting for its rows.
    Showing(usize),
    /// The passwords page is up and empty; asked it to unlock.
    Unlocking,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PagesSmoke {
    pub stage: Stage,
    /// Views at the start: no page may add one.
    views: usize,
    runs: u32,
}

impl Default for PagesSmoke {
    fn default() -> Self {
        Self {
            stage: Stage::Start,
            views: 0,
            runs: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Open(InternalPage),
    /// Send the passwords page's Unlock callback.
    Unlock,
    Pass,
    Fail(String),
}

fn expected(page: InternalPage) -> usize {
    match page {
        InternalPage::History => HISTORY_LINES,
        InternalPage::Bookmarks => BOOKMARK_LINES,
        InternalPage::Downloads => DOWNLOADS,
        // Locked: nothing listed. The Task manager is not opened by this smoke.
        InternalPage::Passwords | InternalPage::Settings | InternalPage::Tasks => 0,
    }
}

pub fn next(s: PagesSmoke, seen: &Seen) -> (PagesSmoke, Option<Action>) {
    let to = |stage, views| PagesSmoke {
        stage,
        views,
        runs: 0,
    };
    let step = match s.stage {
        Stage::Start if seen.ready => {
            Some((to(Stage::Showing(0), seen.views), Action::Open(ORDER[0])))
        }
        Stage::Showing(_) | Stage::Unlocking if seen.views != s.views => Some((
            to(Stage::Done, s.views),
            Action::Fail(format!("a page opened a view: {seen:?}")),
        )),
        Stage::Showing(i) if seen.page == Some(ORDER[i]) && seen.rows == expected(ORDER[i]) => {
            match ORDER.get(i + 1) {
                Some(&page) => Some((to(Stage::Showing(i + 1), s.views), Action::Open(page))),
                None if !seen.unlocked => Some((to(Stage::Unlocking, s.views), Action::Unlock)),
                None => Some((
                    to(Stage::Done, s.views),
                    Action::Fail(format!("the passwords page opened unlocked: {seen:?}")),
                )),
            }
        }
        Stage::Unlocking if seen.page == Some(InternalPage::Passwords) && seen.unlocked => {
            if seen.rows == LOGINS {
                Some((to(Stage::Done, s.views), Action::Pass))
            } else if seen.rows > 0 {
                Some((
                    to(Stage::Done, s.views),
                    Action::Fail(format!("wrong number of logins: {seen:?}")),
                ))
            } else {
                None
            }
        }
        _ => None,
    };
    match step {
        Some((stage, action)) => (stage, Some(action)),
        None if s.stage == Stage::Done => (s, None),
        None if s.runs >= MAX_WORK_RUNS => (
            to(Stage::Done, s.views),
            Some(Action::Fail(format!(
                "stuck at {:?} after {MAX_WORK_RUNS} runs of CEF work: {seen:?}",
                s.stage
            ))),
        ),
        None => (
            PagesSmoke {
                runs: s.runs + 1,
                ..s
            },
            None,
        ),
    }
}

const DAY_MS: i64 = 86_400_000;

/// Fills the profile's database before the host starts: the rows the pages must show. The vault's
/// key goes into `keyring`, which the host's store thread is given too.
pub fn seed(db: &Path, keyring: &FakeKeyRing) -> Result<(), Box<dyn std::error::Error>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    let mut store = pollster::block_on(Store::open(db))?;
    pollster::block_on(async {
        for (i, (url, title)) in [
            ("https://a.test/", "Apple"),
            ("https://b.test/", "Bread"),
            ("https://c.test/", "Cake"),
        ]
        .into_iter()
        .enumerate()
        {
            let at = now - 3 * DAY_MS * i as i64;
            store.record_visit(url, title, at, Transition::Link).await?;
        }
        let folder = store.add_folder(None, "Folder", now).await?;
        store
            .add_bookmark(Some(folder), "In folder", "https://a.test/", now)
            .await?;
        store
            .add_bookmark(None::<BookmarkId>, "Top", "https://b.test/", now)
            .await?;
        for (id, state) in [(1, DownloadState::Complete), (2, DownloadState::Cancelled)] {
            store
                .add_download(&DownloadEntry {
                    id,
                    url: format!("https://a.test/{id}.zip"),
                    name: format!("{id}.zip"),
                    path: format!("/tmp/{id}.zip").into(),
                    received: 4,
                    total: Some(4),
                    state,
                    started_ms: now,
                })
                .await?;
        }
        Ok::<_, wubble_store::Error>(())
    })?;
    let mut vault = Vault::new(keyring.clone(), StoreLogins::new(store), &profile_name(db));
    vault.unlock();
    for site in ["https://a.test", "https://b.test"] {
        vault.save(
            NewLogin {
                origin: Origin::parse(site)?,
                form_action_origin: None,
                username: "smoke".into(),
                password: Zeroizing::new("smoke-pw".into()),
            },
            now,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> Seen {
        Seen {
            ready: true,
            views: 1,
            ..Seen::default()
        }
    }

    fn on(page: InternalPage, rows: usize) -> Seen {
        Seen {
            page: Some(page),
            rows,
            ..ready()
        }
    }

    #[test]
    fn opens_each_page_waits_for_its_rows_then_unlocks() {
        let (s, a) = next(PagesSmoke::default(), &ready());
        assert_eq!(a, Some(Action::Open(InternalPage::History)));
        let (s, a) = next(s, &on(InternalPage::History, 2));
        assert_eq!(a, None, "waits for all the rows");
        let (s, a) = next(s, &on(InternalPage::History, HISTORY_LINES));
        assert_eq!(a, Some(Action::Open(InternalPage::Bookmarks)));
        let (s, a) = next(s, &on(InternalPage::Bookmarks, BOOKMARK_LINES));
        assert_eq!(a, Some(Action::Open(InternalPage::Downloads)));
        let (s, a) = next(s, &on(InternalPage::Downloads, DOWNLOADS));
        assert_eq!(a, Some(Action::Open(InternalPage::Passwords)));
        let (s, a) = next(s, &on(InternalPage::Passwords, 0));
        assert_eq!(a, Some(Action::Unlock));
        let (s, a) = next(s, &on(InternalPage::Passwords, 0));
        assert_eq!(a, None);
        let unlocked = Seen {
            unlocked: true,
            ..on(InternalPage::Passwords, LOGINS)
        };
        assert_eq!(next(s, &unlocked).1, Some(Action::Pass));
    }

    #[test]
    fn a_new_view_or_an_unlocked_start_fails() {
        let s = PagesSmoke {
            stage: Stage::Showing(0),
            views: 1,
            runs: 0,
        };
        let more = Seen {
            views: 2,
            ..on(InternalPage::History, HISTORY_LINES)
        };
        assert!(matches!(next(s, &more).1, Some(Action::Fail(_))));
        let s = PagesSmoke {
            stage: Stage::Showing(3),
            views: 1,
            runs: 0,
        };
        let open = Seen {
            unlocked: true,
            ..on(InternalPage::Passwords, 0)
        };
        assert!(matches!(next(s, &open).1, Some(Action::Fail(_))));
    }

    #[test]
    fn gives_up_after_the_run_budget() {
        let mut s = PagesSmoke::default();
        let mut last = None;
        for _ in 0..=MAX_WORK_RUNS {
            (s, last) = next(s, &Seen::default());
        }
        assert!(matches!(last, Some(Action::Fail(_))));
        assert_eq!(s.stage, Stage::Done);
    }

    #[test]
    fn seeding_fills_every_store_the_pages_read() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("wubble.db");
        let keyring = FakeKeyRing::default();
        seed(&db, &keyring).unwrap();
        let store = pollster::block_on(Store::open(&db)).unwrap();
        let visits = pollster::block_on(store.list_visits(10, None, "")).unwrap();
        assert_eq!(visits.len(), 3);
        assert_eq!(pollster::block_on(store.all_bookmarks()).unwrap().len(), 3);
        assert_eq!(
            pollster::block_on(store.downloads(10)).unwrap().len(),
            DOWNLOADS
        );
        let mut vault = Vault::new(keyring, StoreLogins::new(store), &profile_name(&db));
        vault.unlock();
        assert_eq!(vault.entries().unwrap().len(), LOGINS);
    }
}
