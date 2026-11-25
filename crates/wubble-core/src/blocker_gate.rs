//! The content blocker's start-up gate: with no compiled-engine cache (a first run, a list
//! change before quitting), the lists compile for seconds and every request passes unfiltered
//! meanwhile. CEF's IO thread may never wait, so the hold is the host's: a web page the host is
//! about to load (a restored tab, a typed address, the start page, `wubble <url>`) waits here
//! until the blocker is ready, it is switched off, or the deadline passes. A page's own
//! navigations inside a loaded page are never asked. uBlock Origin holds requests at start-up
//! the same way (its "suspend network activity until all filter lists are loaded"); design
//! reference only.
//!
//! One enum, one transition; every (state, event) pair is a row of the `match`.

use crate::tab::TabId;

/// How long a load waits for the lists at most; then it loads anyway (and the host logs it).
pub const DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gate {
    /// The lists are compiling: the loads the host started wait, one per tab (its newest), in
    /// the order the tabs first asked.
    Waiting { held: Vec<(TabId, String)> },
    /// Loads go at once.
    Ready,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateEvent {
    /// The host is about to load `url` as `tab`'s main frame.
    Navigate { tab: TabId, url: String },
    /// The blocker swapped its compiled lists in, or gave up compiling them: nothing more is
    /// coming either way.
    BlockerReady,
    /// The blocker was switched off: nothing would be filtered.
    BlockerOff,
    /// [`DEADLINE`] passed.
    Deadline,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateEffect {
    /// Load `url` in `tab` now.
    Load { tab: TabId, url: String },
}

impl Gate {
    /// Ready from the start when the compiled cache loaded or the blocker is off.
    pub fn start(ready: bool) -> Self {
        if ready {
            Self::Ready
        } else {
            Self::Waiting { held: vec![] }
        }
    }
}

/// Only web pages are filtered, so only they wait.
fn filtered(url: &str) -> bool {
    crate::url::is_recordable(url)
}

pub fn transition(gate: Gate, event: GateEvent) -> (Gate, Vec<GateEffect>) {
    use GateEvent::*;
    match (gate, event) {
        (Gate::Waiting { mut held }, Navigate { tab, url }) if filtered(&url) => {
            match held.iter_mut().find(|(t, _)| *t == tab) {
                Some(slot) => slot.1 = url,
                None => held.push((tab, url)),
            }
            (Gate::Waiting { held }, vec![])
        }
        // A page the blocker never sees, or the gate open: at once. A tab that waited and now
        // loads an internal page forgets the web page it waited with.
        (Gate::Waiting { mut held }, Navigate { tab, url }) => {
            held.retain(|(t, _)| *t != tab);
            (Gate::Waiting { held }, vec![GateEffect::Load { tab, url }])
        }
        (Gate::Ready, Navigate { tab, url }) => (Gate::Ready, vec![GateEffect::Load { tab, url }]),
        (Gate::Waiting { held }, BlockerReady | BlockerOff | Deadline) => (
            Gate::Ready,
            held.into_iter()
                .map(|(tab, url)| GateEffect::Load { tab, url })
                .collect(),
        ),
        // Once open the gate stays open: a later list change rebuilds while the old lists
        // still filter.
        (Gate::Ready, BlockerReady | BlockerOff | Deadline) => (Gate::Ready, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use GateEvent::*;

    const A: TabId = TabId(1);
    const B: TabId = TabId(2);

    fn nav(tab: TabId, url: &str) -> GateEvent {
        Navigate {
            tab,
            url: url.into(),
        }
    }

    fn load(tab: TabId, url: &str) -> GateEffect {
        GateEffect::Load {
            tab,
            url: url.into(),
        }
    }

    fn waiting(held: &[(TabId, &str)]) -> Gate {
        Gate::Waiting {
            held: held.iter().map(|(t, u)| (*t, (*u).to_owned())).collect(),
        }
    }

    #[test]
    fn the_start_state() {
        assert_eq!(Gate::start(true), Gate::Ready);
        assert_eq!(Gate::start(false), waiting(&[]));
    }

    #[test]
    fn transitions() {
        let rows: Vec<(&str, Gate, GateEvent, Gate, Vec<GateEffect>)> = vec![
            (
                "ready loads at once",
                Gate::Ready,
                nav(A, "https://a.test/"),
                Gate::Ready,
                vec![load(A, "https://a.test/")],
            ),
            (
                "a web page waits",
                waiting(&[]),
                nav(A, "https://a.test/"),
                waiting(&[(A, "https://a.test/")]),
                vec![],
            ),
            (
                "http waits too",
                waiting(&[]),
                nav(A, "http://a.test/"),
                waiting(&[(A, "http://a.test/")]),
                vec![],
            ),
            (
                "a second tab queues behind",
                waiting(&[(A, "https://a.test/")]),
                nav(B, "https://b.test/"),
                waiting(&[(A, "https://a.test/"), (B, "https://b.test/")]),
                vec![],
            ),
            (
                "the same tab again keeps its newest, in its place",
                waiting(&[(A, "https://a.test/"), (B, "https://b.test/")]),
                nav(A, "https://c.test/"),
                waiting(&[(A, "https://c.test/"), (B, "https://b.test/")]),
                vec![],
            ),
            (
                "an internal page loads at once",
                waiting(&[]),
                nav(A, "wubble://start"),
                waiting(&[]),
                vec![load(A, "wubble://start")],
            ),
            (
                "about:blank loads at once",
                waiting(&[]),
                nav(B, "about:blank"),
                waiting(&[]),
                vec![load(B, "about:blank")],
            ),
            (
                "an internal page replaces the web page the tab waited with",
                waiting(&[(A, "https://a.test/"), (B, "https://b.test/")]),
                nav(A, "file:///tmp/x.html"),
                waiting(&[(B, "https://b.test/")]),
                vec![load(A, "file:///tmp/x.html")],
            ),
            (
                "ready releases in order",
                waiting(&[(A, "https://a.test/"), (B, "https://b.test/")]),
                BlockerReady,
                Gate::Ready,
                vec![load(A, "https://a.test/"), load(B, "https://b.test/")],
            ),
            (
                "off releases",
                waiting(&[(A, "https://a.test/")]),
                BlockerOff,
                Gate::Ready,
                vec![load(A, "https://a.test/")],
            ),
            (
                "the deadline releases",
                waiting(&[(A, "https://a.test/")]),
                Deadline,
                Gate::Ready,
                vec![load(A, "https://a.test/")],
            ),
            (
                "an empty gate opens with nothing to load",
                waiting(&[]),
                BlockerReady,
                Gate::Ready,
                vec![],
            ),
            ("ready again is nothing", Gate::Ready, BlockerReady, Gate::Ready, vec![]),
            ("off when ready is nothing", Gate::Ready, BlockerOff, Gate::Ready, vec![]),
            ("a late deadline is nothing", Gate::Ready, Deadline, Gate::Ready, vec![]),
        ];
        for (name, gate, event, next, effects) in rows {
            assert_eq!(transition(gate, event), (next, effects), "{name}");
        }
    }
}
