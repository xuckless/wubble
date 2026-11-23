//! `--smoke private`'s script (`data_smoke::Ending::Private`): a normal tab on the normal host
//! and a private window on the private host, driven through the host's own paths (commands
//! dispatched as the shell sends them, no key presses). In the private window: per-site
//! choices and a zoom, a download, a tab opened and closed; then what the running host shows
//! the normal side, and the private window closed. The parent searches the profile afterwards
//! (`data_smoke::private_leaks`).

use wubble_core::action::Action;
use wubble_core::download::DownloadState;
use wubble_core::model::Command;
use wubble_core::pagetheme::PageTheme;
use wubble_core::site::SiteChange;
use wubble_core::space::{Side, Storage};
use wubble_core::suggest::Source;

use super::{DataSmoke, Host, Pass, Pending, Poll, Step, expect, fill, filled, when};
use crate::data_smoke::{NORMAL_HOST, PRIVATE_HOST};

pub(super) const STEPS: &[Step] = &[
    step!("ready", Host::data_start, |h, _| when(
        h.frame_rate.is_some()
    )),
    step!("the normal tab on the normal host", nothing, |h, c| h
        .data_title(c, "normalmark A")),
    step!(
        "a zoom and a page theme on the normal host",
        Host::private_normal_choices,
        |_, _| Pass
    ),
    step!(
        "a private window on the private host",
        |h, c| {
            let url = c.server.url_at(PRIVATE_HOST, "/b");
            h.dispatch(Command::NewPrivateWindow { url: Some(url) });
        },
        Host::private_opened
    ),
    step!(
        "per-site choices and a zoom in the private tab",
        Host::private_choices_made,
        |h, _| {
            expect(h.private_choices != Default::default(), || {
                "the private tab's choices were not kept for it".into()
            })
        }
    ),
    // given a `--shot-dir`: the private window (its Private chip and tint), once it settled
    step!("the private window shot", nothing, Host::private_shot),
    step!(
        "a download in the private tab",
        |h, c| {
            if let Some(tab) = c.private_tab {
                let url = c.server.url_at(PRIVATE_HOST, "/small");
                h.dispatch(Command::Navigate { tab, url });
            }
        },
        Host::private_downloaded
    ),
    step!(
        "a second private tab",
        |h, c| {
            let container = h.private_container();
            if let Some(w) = c.private_window
                && h.enter(w)
            {
                let url = c.server.url_at(PRIVATE_HOST, "/b2");
                h.dispatch(Command::NewTab {
                    url,
                    container,
                    opener: None,
                });
            }
        },
        Host::private_second_open
    ),
    step!(
        "the second private tab closed",
        |h, c| {
            if let Some(tab) = c.second {
                h.dispatch(Command::CloseTab(tab));
            }
        },
        |h, c| when(c.second.is_some_and(|t| !h.model.tabs.contains_key(&t)))
    ),
    step!(
        "the session saved while the private window is open",
        |h, _| h.save_now(),
        |_, _| Pass
    ),
    step!(
        "what the running host shows",
        nothing,
        Host::private_kept_apart
    ),
    step!(
        "the private window closed",
        |h, c| {
            if let Some(w) = c.private_window {
                h.dispatch(Command::CloseWindow(w));
            }
        },
        Host::private_gone
    ),
    step!("the session saved again", |h, _| h.save_now(), |_, _| Pass),
    step!(
        "the store answered everything sent so far",
        |h, c| h.store.visits("", None, 1, fill(&c.visits)),
        |_, c| when(filled(&c.visits).is_some())
    ),
];

fn nothing(_: &mut Host, _: &mut DataSmoke) {}

impl Host {
    /// The private windows' in-memory container, while there is one.
    fn private_container(&self) -> Option<wubble_core::space::ContainerId> {
        self.model
            .containers
            .iter()
            .find(|c| c.storage == Storage::Memory)
            .map(|c| c.id)
    }

    /// A zoom and a theme for the normal page: what the parent finds stored, so it can see.
    fn private_normal_choices(&mut self, c: &mut DataSmoke) {
        if let Some(tab) = c.web {
            self.perform(Action::ZoomIn);
            let page = c.server.url_at(NORMAL_HOST, "/a");
            self.set_site_theme(tab, &page, PageTheme::Dark);
        }
    }

    fn private_opened(&mut self, c: &mut DataSmoke) -> Poll {
        let tab = self
            .model
            .tabs
            .values()
            .find(|t| self.model.side_of_tab(t.id) == Side::Private && t.title == "privmark B");
        let Some(tab) = tab.map(|t| t.id) else {
            return Pending;
        };
        c.private_tab = Some(tab);
        c.private_window = self
            .model
            .location(tab)
            .and_then(|(space, _)| self.model.space_window(space));
        expect(c.private_window.is_some(), || {
            "the private tab has no window".into()
        })
    }

    /// In the private window, as its pill menu and zoom keys make them: a zoom, a page
    /// theme, a blocker exception and a never-sleep site for the private host.
    fn private_choices_made(&mut self, c: &mut DataSmoke) {
        let (Some(w), Some(tab)) = (c.private_window, c.private_tab) else {
            return;
        };
        if !self.enter(w) {
            return;
        }
        self.perform(Action::ZoomIn);
        let page = c.server.url_at(PRIVATE_HOST, "/b");
        self.set_site_theme(tab, &page, PageTheme::Dark);
        let host = format!("{PRIVATE_HOST}.localhost");
        self.write_site(tab, SiteChange::Blocker(host.clone()));
        self.write_site(tab, SiteChange::NeverSleep(host));
    }

    /// Shoots the private window after a few hundred runs of CEF work (its page drawn, the
    /// shell's entrance done); nothing without a `--shot-dir`.
    fn private_shot(&mut self, c: &mut DataSmoke) -> Poll {
        if !crate::shot::has_dir() {
            return Pass;
        }
        if c.runs < 300 {
            return Pending;
        }
        let Some(window) = c
            .private_window
            .filter(|&w| self.enter(w))
            .and_then(|_| self.w.window.upgrade())
        else {
            return super::Fail("no private window to shoot".into());
        };
        match crate::shot::take_scene(&window, 0, "private-window") {
            Ok(path) => {
                c.notes.push(format!("shot {}", path.display()));
                Pass
            }
            Err(e) => super::Fail(format!("the shot failed: {e}")),
        }
    }

    fn private_downloaded(&mut self, _: &mut DataSmoke) -> Poll {
        let memory = self.private_container();
        let Some(e) = self
            .downloads
            .entries()
            .iter()
            .find(|e| e.url.contains(PRIVATE_HOST))
        else {
            return Pending;
        };
        match &e.state {
            DownloadState::Complete => expect(Some(e.container) == memory, || {
                format!(
                    "the download is in {:?}, not the private windows' {memory:?}",
                    e.container
                )
            }),
            DownloadState::Failed(why) => super::Fail(format!("the download failed: {why}")),
            _ => Pending,
        }
    }

    fn private_second_open(&mut self, c: &mut DataSmoke) -> Poll {
        let second = self.model.tabs.values().find(|t| t.title == "privmark B2");
        let Some(tab) = second.map(|t| t.id) else {
            return Pending;
        };
        c.second = Some(tab);
        expect(self.model.side_of_tab(tab) == Side::Private, || {
            "the tab opened in the private window is not private".into()
        })
    }

    /// The private tab is in none of the normal window's lists, nothing of it is recorded, and
    /// the private window's bar would ask no server even with live suggestions on.
    fn private_kept_apart(&mut self, c: &mut DataSmoke) -> Poll {
        let (Some(w), Some(tab)) = (c.private_window, c.private_tab) else {
            return super::Fail("no private window".into());
        };
        // the normal Main window (the focused one is the private window)
        let Some((normal, normal_space)) = self.model.windows.iter().find_map(|x| {
            let space = x.space()?;
            (self.model.side_of_space(space) == Side::Normal).then_some((x.id, space))
        }) else {
            return super::Fail("no normal window".into());
        };
        let normal_space = Some(normal_space);
        let listed: Vec<_> = normal_space
            .map(|s| {
                self.model
                    .listed_spaces(s)
                    .flat_map(|s| self.model.order(s.id))
                    .collect()
            })
            .unwrap_or_default();
        if listed.contains(&tab) || self.model.window_tabs(normal).contains(&tab) {
            return super::Fail("the normal window lists the private tab".into());
        }
        if self.records(tab) {
            return super::Fail("the private tab's loads are recorded".into());
        }
        let (live, engine) = (self.live_suggestions, self.search_engine);
        self.live_suggestions = true;
        self.search_engine = Some(wubble_core::suggest::SearchEngine::DuckDuckGo);
        let entered = self.enter(w);
        let remote = wubble_core::suggest::sources("cats and dogs", &self.suggest_settings())
            .iter()
            .any(|s| matches!(s, Source::Remote { .. }));
        let normal_remote = self.enter(normal)
            && wubble_core::suggest::sources("cats and dogs", &self.suggest_settings())
                .iter()
                .any(|s| matches!(s, Source::Remote { .. }));
        (self.live_suggestions, self.search_engine) = (live, engine);
        if !entered || remote {
            return super::Fail("the private window's bar would ask a server".into());
        }
        expect(normal_remote, || {
            "the normal window's bar asks no server with live suggestions on: the check sees nothing"
                .into()
        })
    }

    /// The in-memory container went with the window, and its downloads and choices with it.
    fn private_gone(&mut self, c: &mut DataSmoke) -> Poll {
        if c.private_window
            .is_some_and(|w| self.model.window(w).is_some())
        {
            return Pending;
        }
        if let Some(id) = self.private_container() {
            return super::Fail(format!(
                "the in-memory container {id:?} outlived the private window"
            ));
        }
        if let Some(e) = self
            .downloads
            .entries()
            .iter()
            .find(|e| e.url.contains(PRIVATE_HOST))
        {
            return super::Fail(format!(
                "the private download stays in the list: {}",
                e.name
            ));
        }
        expect(self.private_choices == Default::default(), || {
            "the private choices outlived the private window".into()
        })
    }
}
