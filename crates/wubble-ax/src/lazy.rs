//! A tab's accessibility events kept as CEF wrote them until something reads its tree. An agent
//! session's tabs keep CEF's tree on (its calls read the page through it) while no screen reader
//! runs; parsing and applying each event as it comes cost ten idle tabs 6.6 of the main thread's
//! 8.8 % (pages that animate while hidden). Chrome builds no tree for a hidden tab unless
//! something reads it; here the events wait, unparsed, and a call applies them first
//! ([`Lazy::settle`]). Too many waiting, they are dropped and CEF's tree is turned off for the tab
//! until a call turns it on again, which sends one whole tree.

/// Events kept at most before they are dropped ([`Push::Overflowed`]).
pub const MAX_EVENTS: usize = 4096;

/// Bytes of JSON kept at most before they are dropped.
pub const MAX_BYTES: usize = 16 << 20;

/// One tab's waiting events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lazy {
    /// The events since the last read, oldest first: CEF's JSON and whether it is a location
    /// change (`PageTree::move_nodes`) or a tree change (`PageTree::apply`), and their bytes.
    Queued {
        events: Vec<(String, bool)>,
        bytes: usize,
    },
    /// More came than [`MAX_EVENTS`] or [`MAX_BYTES`]: dropped, CEF's tree off for the tab; one
    /// that still comes is ignored.
    Dropped,
}

impl Default for Lazy {
    fn default() -> Self {
        Self::Queued {
            events: Vec::new(),
            bytes: 0,
        }
    }
}

/// What a pushed event asks of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Push {
    /// Kept (or ignored while dropped): nothing to do.
    Kept,
    /// The queue overflowed: drop the tab's mirror and turn CEF's tree off for it.
    Overflowed,
}

/// What a read asks of the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settle {
    /// Apply these to the tab's mirror, in order.
    Apply(Vec<(String, bool)>),
    /// Turn CEF's tree on again for the tab: a whole tree comes, queued like any event.
    Reopen,
}

impl Lazy {
    /// `json` came for the tab (`location`: a location change).
    pub fn push(&mut self, json: &str, location: bool) -> Push {
        let Self::Queued { events, bytes } = self else {
            return Push::Kept;
        };
        if events.len() >= MAX_EVENTS || *bytes + json.len() > MAX_BYTES {
            *self = Self::Dropped;
            return Push::Overflowed;
        }
        *bytes += json.len();
        events.push((json.to_owned(), location));
        Push::Kept
    }

    /// Something reads the tab's tree now: the queue is empty after it.
    pub fn settle(&mut self) -> Settle {
        match std::mem::take(self) {
            Self::Queued { events, .. } => Settle::Apply(events),
            Self::Dropped => Settle::Reopen,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(events: &[(&str, bool)]) -> Lazy {
        Lazy::Queued {
            events: events.iter().map(|&(j, l)| (j.to_owned(), l)).collect(),
            bytes: events.iter().map(|(j, _)| j.len()).sum(),
        }
    }

    #[test]
    fn events_wait_unparsed_until_a_read() {
        let full = Lazy::Queued {
            events: vec![("x".to_owned(), false); MAX_EVENTS],
            bytes: MAX_EVENTS,
        };
        let heavy = Lazy::Queued {
            events: vec![],
            bytes: MAX_BYTES - 1,
        };
        // (name, before, event, after, push's answer)
        for (name, mut lazy, (json, location), after, push) in [
            ("the first", Lazy::default(), ("{}", false), queued(&[("{}", false)]), Push::Kept),
            (
                "after another, in order",
                queued(&[("a", false)]),
                ("bb", true),
                queued(&[("a", false), ("bb", true)]),
                Push::Kept,
            ),
            ("the same twice", queued(&[("a", false)]), ("a", false), queued(&[("a", false), ("a", false)]), Push::Kept),
            ("one past the count", full, ("y", false), Lazy::Dropped, Push::Overflowed),
            ("one past the bytes", heavy.clone(), ("yy", false), Lazy::Dropped, Push::Overflowed),
            (
                "up to the bytes",
                heavy,
                ("y", false),
                Lazy::Queued {
                    events: vec![("y".to_owned(), false)],
                    bytes: MAX_BYTES,
                },
                Push::Kept,
            ),
            ("one while dropped", Lazy::Dropped, ("z", false), Lazy::Dropped, Push::Kept),
        ] {
            assert_eq!(lazy.push(json, location), push, "{name}");
            assert_eq!(lazy, after, "{name}");
        }
    }

    #[test]
    fn a_read_takes_the_queue_or_reopens() {
        // (name, before, the read's answer)
        for (name, mut lazy, settle) in [
            ("nothing waits", Lazy::default(), Settle::Apply(vec![])),
            (
                "the events, oldest first",
                queued(&[("a", false), ("b", true)]),
                Settle::Apply(vec![("a".to_owned(), false), ("b".to_owned(), true)]),
            ),
            ("dropped: CEF's tree on again", Lazy::Dropped, Settle::Reopen),
        ] {
            assert_eq!(lazy.settle(), settle, "{name}");
            assert_eq!(lazy, Lazy::default(), "{name}: empty after");
            assert_eq!(lazy.settle(), Settle::Apply(vec![]), "{name}: a second read");
        }
    }
}
