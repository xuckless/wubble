//! Browsers CEF creates beside the engine's views: DevTools windows, and
//! whatever they open in turn (DevTools of DevTools, a link from DevTools).
//!
//! `cef::shutdown` with any browser still alive crashes (a DevTools window left
//! open at quit: `CefBrowserInfo::RemoveFrame` while Chromium tears the window
//! down), so each of these counts in the lifecycle like a view. They are given
//! [`ExtraBrowsers::client`], whose life-span handler reports every
//! `OnAfterCreated` / `OnBeforeClose`; CEF hands a browser's client on to the
//! popups and DevTools it opens, so their descendants report there too.

use std::sync::{Arc, Mutex, PoisonError};

use cef::{
    Client, DisplayHandler, ImplBrowser, ImplBrowserHost, ImplClient, ImplDisplayHandler,
    ImplFrame, ImplLifeSpanHandler, LifeSpanHandler, WrapClient, WrapDisplayHandler,
    WrapLifeSpanHandler, rc::Rc,
};

use super::state::Event;

/// What CEF reported, in order, until [`ExtraBrowsers::drain`].
#[derive(Clone, Debug, PartialEq, Eq)]
enum Report {
    Created(i32),
    Title(i32, String),
    Closed(i32),
}

type Reports = Arc<Mutex<Vec<Report>>>;

fn push(reports: &Reports, report: Report) {
    reports
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(report);
}

/// One created and not yet closed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Open {
    id: i32,
    /// The engine asked it to close.
    closing: bool,
    title: String,
}

/// The open extra browsers, by CEF identifier, and their reports.
#[derive(Default)]
pub struct ExtraBrowsers {
    reports: Reports,
    open: Vec<Open>,
}

impl ExtraBrowsers {
    /// The client to create an extra browser with (`ShowDevTools`'s `client`).
    pub fn client(&self) -> cef::Client {
        ExtraClient::new(
            ExtraLifeSpan::new(self.reports.clone()),
            ExtraDisplay::new(self.reports.clone()),
        )
    }

    /// Takes CEF's reports as lifecycle events, in order. A close of a browser
    /// never seen created (not ours) is dropped.
    pub fn drain(&mut self) -> Vec<Event> {
        let reports =
            std::mem::take(&mut *self.reports.lock().unwrap_or_else(PoisonError::into_inner));
        let mut events = Vec::new();
        for report in reports {
            match report {
                Report::Created(id) => {
                    self.open.push(Open {
                        id,
                        ..Open::default()
                    });
                    events.push(Event::BrowserOpened);
                }
                Report::Title(id, title) => {
                    if let Some(open) = self.open.iter_mut().find(|open| open.id == id) {
                        open.title = title;
                    }
                }
                Report::Closed(id) => {
                    if let Some(at) = self.open.iter().position(|open| open.id == id) {
                        self.open.remove(at);
                        events.push(Event::BrowserClosed);
                    }
                }
            }
        }
        events
    }

    /// Created and not yet reported closed.
    pub fn open(&self) -> usize {
        self.open.len()
    }

    /// Each open one's page title, in creation order.
    pub fn titles(&self) -> Vec<String> {
        self.open.iter().map(|open| open.title.clone()).collect()
    }

    /// Runs `script` in each open one's main frame. For smokes.
    pub fn run_script(&self, script: &str) {
        let script: cef::CefString = script.into();
        let url: cef::CefString = "about:wubble-smoke".into();
        for open in &self.open {
            if let Some(frame) =
                cef::browser_host_get_browser_by_identifier(open.id).and_then(|b| b.main_frame())
            {
                frame.execute_java_script(Some(&script), Some(&url), 0);
            }
        }
    }

    /// Asks CEF to close every open one not asked before; each reports closed
    /// on a later pump.
    pub fn close_all(&mut self) {
        for open in self.open.iter_mut().filter(|open| !open.closing) {
            open.closing = true;
            if let Some(host) =
                cef::browser_host_get_browser_by_identifier(open.id).and_then(|b| b.host())
            {
                host.close_browser(1);
            }
        }
    }
}

cef::wrap_client! {
    struct ExtraClient {
        life_span: cef::LifeSpanHandler,
        display: cef::DisplayHandler,
    }

    impl Client {
        fn life_span_handler(&self) -> Option<cef::LifeSpanHandler> {
            Some(self.life_span.clone())
        }

        fn display_handler(&self) -> Option<cef::DisplayHandler> {
            Some(self.display.clone())
        }
    }
}

cef::wrap_display_handler! {
    struct ExtraDisplay {
        reports: Reports,
    }

    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut cef::Browser>, title: Option<&cef::CefString>) {
            if let Some(browser) = browser {
                let title = title.map(|t| t.to_string()).unwrap_or_default();
                push(&self.reports, Report::Title(browser.identifier(), title));
            }
        }
    }
}

cef::wrap_life_span_handler! {
    struct ExtraLifeSpan {
        reports: Reports,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut cef::Browser>) {
            if let Some(browser) = browser {
                push(&self.reports, Report::Created(browser.identifier()));
            }
        }

        fn on_before_close(&self, browser: Option<&mut cef::Browser>) {
            if let Some(browser) = browser {
                push(&self.reports, Report::Closed(browser.identifier()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(reports: &[Report]) -> ExtraBrowsers {
        let extra = ExtraBrowsers::default();
        extra.reports.lock().unwrap().extend_from_slice(reports);
        extra
    }

    #[test]
    fn each_created_browser_counts_until_it_closes() {
        let mut extra = with(&[
            Report::Created(7),
            Report::Created(8),
            Report::Title(8, "DevTools".into()),
            Report::Closed(7),
        ]);
        assert_eq!(
            extra.drain(),
            [
                Event::BrowserOpened,
                Event::BrowserOpened,
                Event::BrowserClosed
            ]
        );
        assert_eq!(extra.titles(), ["DevTools"]);
        push(&extra.reports, Report::Closed(8));
        assert_eq!(extra.drain(), [Event::BrowserClosed]);
        assert_eq!(extra.open(), 0);
    }

    #[test]
    fn a_close_never_seen_created_is_not_counted() {
        let mut extra = with(&[Report::Closed(3)]);
        assert!(extra.drain().is_empty());
        let mut twice = with(&[Report::Created(3), Report::Closed(3), Report::Closed(3)]);
        assert_eq!(twice.drain(), [Event::BrowserOpened, Event::BrowserClosed]);
    }
}
