//! What wubble opens when it starts (`settings::STARTUP_MODE`), the home page
//! (`settings::HOME_URL`), and the "Restore pages?" offer after a run that did not quit cleanly.
//!
//! Chrome's "On startup" setting, behaviour ported, no code copied: `SessionStartupPref`
//! (chrome/browser/prefs/session_startup_pref.h, BSD-3-Clause) has three types, the New Tab page
//! (`DEFAULT`), continue where you left off (`LAST`) and a set of pages (`URLS`). With the New
//! Tab page or a set of pages the last session's tabs are not restored, but pinned tabs are
//! (`StartupBrowserCreatorImpl::DetermineStartupTabs`: pinned tabs always open). wubble keeps
//! more than Chrome's pinned tabs: Spaces, pinned tabs and folders with their tabs are the
//! user's layout, and only the loose tabs of the last session are left out, onto the
//! recently-closed list. After a crash Chrome offers "Restore pages?" (its session crashed
//! bubble, `SessionCrashedBubbleView`); here the offer is one enum with a pure
//! [`transition`].

use std::collections::HashMap;

use crate::model::{Command, Model, apply};
use crate::omnibox::{Intent, classify};
use crate::pages::InternalPage;
use crate::settings;
use crate::space::{ContainerId, Item, SpaceId};
use crate::tab::TabId;

/// `settings::STARTUP_MODE`: what wubble opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupMode {
    /// Every tab as it was (wubble's default; Chrome's "Continue where you left off").
    LastSession,
    /// Spaces, pinned tabs and folders, and a start page in each window.
    StartPage,
    /// Spaces, pinned tabs and folders, and the pages of `settings::STARTUP_PAGES`.
    SpecificPages,
}

/// The stored words of [`StartupMode`], in its order: the row's choices ([`MODE_LABELS`]').
pub const MODE_WORDS: [&str; 3] = settings::select_words(&MODE_LABELS);

impl StartupMode {
    pub const ALL: [StartupMode; 3] = [Self::LastSession, Self::StartPage, Self::SpecificPages];

    pub fn id(self) -> &'static str {
        MODE_WORDS[self as usize]
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    /// The mode the stored rows ask for (the default when absent or invalid).
    pub fn from_stored(stored: &HashMap<String, String>) -> Self {
        Self::from_id(crate::apply::value_of(&settings::STARTUP_MODE, stored))
            .unwrap_or(Self::LastSession)
    }
}

/// What the settings page's Select shows for each stored word of `settings::STARTUP_MODE`
/// (the design's settings row `startup`: Start page | Last session).
pub const MODE_LABELS: [(&str, &str); 3] = [
    ("last-session", "Last session"),
    ("start-page", "Start page"),
    ("pages", "Specific pages"),
];

/// An address the user typed for a page to open (a startup page, the home page): what the
/// command bar would load for it, or `None` when it is no address (blank, or words to search).
pub fn page_address(text: &str) -> Option<String> {
    match classify(text) {
        Intent::Url(url) => Some(url),
        Intent::Search(_) => None,
    }
}

/// The stored startup pages, each as the address it loads; entries that are no address are
/// skipped.
pub fn startup_pages(stored: &HashMap<String, String>) -> Vec<String> {
    crate::apply::list(&settings::STARTUP_PAGES, stored)
        .iter()
        .filter_map(|p| page_address(p))
        .collect()
}

/// Where Home goes (`Action::GoHome`): the home page's address, or the start page when it is
/// empty or no address.
pub fn home_url(stored: &HashMap<String, String>) -> String {
    page_address(crate::apply::value_of(&settings::HOME_URL, stored))
        .unwrap_or_else(|| InternalPage::Start.url().to_owned())
}

/// A loose tab of the last session the start left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub url: String,
    pub title: String,
    pub container: ContainerId,
    pub space: SpaceId,
    /// It was its Space's shown tab.
    pub active: bool,
}

/// The last session's pages the start did not open, for the offer to bring back.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LastSession {
    /// In their Spaces' order.
    pub tabs: Vec<Dropped>,
    /// The start pages this start opened in their place: restoring closes those still on the
    /// start page.
    pub start_tabs: Vec<TabId>,
}

/// The loaded session as the startup mode opens it.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    pub model: Model,
    pub last: LastSession,
}

/// The session `model` as `mode` opens it. Last session keeps it as it is. Start page and
/// specific pages keep every Space, pinned tab and folder (with its tabs), leave the loose tabs
/// out (returned in [`LastSession`]) and open a start page in each Main window; with specific
/// pages, the focused window opens `pages` instead (the first one shown). `opening`: a page
/// from the command line takes the focused window, which opens nothing of its own.
pub fn open(model: Model, mode: StartupMode, pages: &[String], opening: bool) -> Opened {
    if mode == StartupMode::LastSession {
        return Opened {
            model,
            last: LastSession::default(),
        };
    }
    let mut tabs = vec![];
    let mut loose = vec![];
    for space in &model.spaces {
        for item in &space.items {
            let Item::Tab(id) = item else {
                continue;
            };
            loose.push(*id);
            if let Some(t) = model.tabs.get(id)
                && !t.url.is_empty()
                && InternalPage::from_url(&t.url) != Some(InternalPage::Start)
            {
                tabs.push(Dropped {
                    url: t.url.clone(),
                    title: t.title.clone(),
                    container: t.container,
                    space: space.id,
                    active: space.active == Some(*id),
                });
            }
        }
    }
    let (mut model, _) = apply(model, Command::CloseTabs(loose));
    let focused = model.focused();
    let windows: Vec<_> = model
        .windows
        .iter()
        .filter_map(|w| w.space().map(|s| (w.id, s)))
        .collect();
    let start = InternalPage::Start.url();
    let mut start_tabs = vec![];
    for (window, space) in windows {
        if opening && window == focused {
            continue;
        }
        let urls: Vec<&str> =
            if mode == StartupMode::SpecificPages && window == focused && !pages.is_empty() {
                pages.iter().map(String::as_str).collect()
            } else {
                vec![start]
            };
        let Some(container) = model
            .spaces
            .iter()
            .find(|s| s.id == space)
            .map(|s| s.container)
        else {
            continue;
        };
        for (i, url) in urls.into_iter().enumerate() {
            let newest = model.tabs.keys().next_back().copied();
            (model, _) = apply(
                model,
                Command::ExtensionTab {
                    url: url.to_owned(),
                    container,
                    space: Some(space),
                    show: i == 0,
                },
            );
            let added = model.tabs.keys().next_back().copied();
            if let Some(tab) = added.filter(|t| Some(*t) != newest)
                && url == start
            {
                start_tabs.push(tab);
            }
        }
    }
    Opened {
        model,
        last: LastSession { tabs, start_tabs },
    }
}

/// The offer of the last session's pages: none, the "Restore pages?" notice up, or kept for the
/// command bar's "Open pages from last session" after the notice went.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Offer {
    /// Nothing was left out (Last session), or it was restored.
    #[default]
    Idle,
    /// The last run did not quit cleanly: the notice asks.
    Asking(LastSession),
    /// Only the command bar's row offers it.
    Kept(LastSession),
}

impl Offer {
    /// The offer at start: the notice when the last run did not quit cleanly, else only the row.
    pub fn at_start(last: LastSession, clean_exit: bool) -> Offer {
        if last.tabs.is_empty() {
            Offer::Idle
        } else if clean_exit {
            Offer::Kept(last)
        } else {
            Offer::Asking(last)
        }
    }

    pub fn asking(&self) -> bool {
        matches!(self, Offer::Asking(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferEvent {
    /// The notice's "Restore", or the command bar's row.
    Restore,
    /// The notice closed unanswered.
    Dismiss,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferEffect {
    /// Open these pages again in their Spaces, and close the start pages still on the start page.
    Reopen(LastSession),
    /// Take the notice away.
    HideNotice,
    /// Asked to restore with nothing to restore.
    NothingToRestore,
}

/// The offer's one transition.
pub fn transition(offer: Offer, event: OfferEvent) -> (Offer, Vec<OfferEffect>) {
    match (offer, event) {
        (Offer::Asking(last), OfferEvent::Restore) => (
            Offer::Idle,
            vec![OfferEffect::HideNotice, OfferEffect::Reopen(last)],
        ),
        (Offer::Kept(last), OfferEvent::Restore) => (Offer::Idle, vec![OfferEffect::Reopen(last)]),
        (Offer::Idle, OfferEvent::Restore) => (Offer::Idle, vec![OfferEffect::NothingToRestore]),
        (Offer::Asking(last), OfferEvent::Dismiss) => (Offer::Kept(last), vec![]),
        (offer, OfferEvent::Dismiss) => (offer, vec![]),
    }
}

/// The notice's text and button.
pub const RESTORE_NOTICE: (&str, &str) = ("Restore pages? wubble did not quit properly", "Restore");

/// The command bar row's toast when there is nothing to open.
pub const NOTHING_TO_RESTORE: &str = "No pages from last session to open";
