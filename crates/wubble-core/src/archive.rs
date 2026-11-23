//! Auto-archive: which tabs have not been visited for long enough to move to the Archive.
//!
//! A pure rule over the model with the wall clock passed in. The host runs it on its lifecycle
//! tick, stores the archived tabs (`wubble-store`'s `archive` table) and closes them.
//! The idea (stale tabs leave the list but are not lost) is Arc's; nothing is taken from it.

use std::collections::HashSet;

use crate::lifecycle::{Activity, Pin};
use crate::model::Model;
use crate::space::Side;
use crate::tab::TabId;

/// The stored words of [`ArchiveAfter`], in its order: the setting's choices.
pub const ARCHIVE_WORDS: [&str; 5] = ["off", "12h", "24h", "7d", "30d"];

const HOUR_MS: i64 = 3_600_000;

/// How long a tab may stay unvisited before it is archived (`settings::ARCHIVE_AFTER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchiveAfter {
    Off,
    Hours12,
    Hours24,
    Days7,
    Days30,
}

impl ArchiveAfter {
    pub const ALL: [ArchiveAfter; 5] = [
        Self::Off,
        Self::Hours12,
        Self::Hours24,
        Self::Days7,
        Self::Days30,
    ];

    /// The stored word, one of the row's choices.
    pub fn id(self) -> &'static str {
        ARCHIVE_WORDS[self as usize]
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    /// The age, in milliseconds, from which a tab is due; `None` when archiving is off.
    pub fn millis(self) -> Option<i64> {
        match self {
            Self::Off => None,
            Self::Hours12 => Some(12 * HOUR_MS),
            Self::Hours24 => Some(24 * HOUR_MS),
            Self::Days7 => Some(7 * 24 * HOUR_MS),
            Self::Days30 => Some(30 * 24 * HOUR_MS),
        }
    }
}

/// The tabs to archive at `now_ms` (unix milliseconds), in tab id order.
///
/// A tab is due once `now_ms - last_active_ms >= after`. Never due: a tab on screen in any
/// window, or in a Little or Popup window; a pinned tab; a loading or downloading tab; one
/// playing sound (`audible`: the host owns it); an internal page; one without a URL; a pane of
/// a split; a private window's tab (nothing of it is stored); a tab whose stamp is unknown (0)
/// or in the future (the clock went back).
pub fn due(
    model: &Model,
    after: ArchiveAfter,
    now_ms: i64,
    audible: &HashSet<TabId>,
) -> Vec<TabId> {
    let Some(limit) = after.millis() else {
        return vec![];
    };
    let visible = model.visible_tabs();
    model
        .tabs
        .values()
        .filter(|t| {
            t.last_active_ms > 0
                && now_ms - t.last_active_ms >= limit
                && t.lifecycle.pin == Pin::Unpinned
                && t.lifecycle.activity == Activity::Idle
                && !t.url.is_empty()
                && !visible.contains(&t.id)
                && !audible.contains(&t.id)
                && !model.is_pinned(t.id)
                && !model.is_internal(t.id)
                && model.windowed(t.id).is_none()
                && model.split_of(t.id).is_none()
                && model.side_of_tab(t.id) != Side::Private
        })
        .map(|t| t.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::model::{Command, Effect, apply};
    use crate::sleep::Pressure;

    const H: i64 = 3_600_000;

    fn open(m: Model, url: &str) -> (Model, TabId) {
        let (m, fx) = apply(
            m,
            Command::NewTab {
                url: url.into(),
                container: None,
                opener: None,
            },
        );
        let Effect::CreateTab { tab, .. } = fx[0] else {
            panic!("no CreateTab")
        };
        (m, tab)
    }

    /// Tabs `a` (hidden, last active at wall 1000) and `b` (visible).
    fn pair() -> (Model, TabId, TabId) {
        let m = Model::new(Instant::now());
        let (m, a) = open(m, "https://a.test/");
        let (mut m, b) = open(m, "https://b.test/");
        m.tabs.get_mut(&a).unwrap().last_active_ms = 1_000;
        (m, a, b)
    }

    fn due24(m: &Model, now: i64) -> Vec<TabId> {
        due(m, ArchiveAfter::Hours24, now, &HashSet::new())
    }

    #[test]
    fn the_threshold_is_inclusive() {
        let (m, a, _) = pair();
        assert_eq!(due24(&m, 1_000 + 24 * H - 1), vec![]);
        assert_eq!(due24(&m, 1_000 + 24 * H), vec![a]);
        for (after, limit) in [
            (ArchiveAfter::Hours12, 12 * H),
            (ArchiveAfter::Days7, 7 * 24 * H),
            (ArchiveAfter::Days30, 30 * 24 * H),
        ] {
            assert_eq!(due(&m, after, 999 + limit, &HashSet::new()), vec![]);
            assert_eq!(due(&m, after, 1_000 + limit, &HashSet::new()), vec![a]);
        }
    }

    #[test]
    fn off_unknown_stamps_and_a_clock_gone_back_are_never_due() {
        let (mut m, a, _) = pair();
        assert_eq!(
            due(&m, ArchiveAfter::Off, i64::MAX, &HashSet::new()),
            vec![]
        );
        assert_eq!(due24(&m, 500), vec![], "negative age");
        m.tabs.get_mut(&a).unwrap().last_active_ms = 0;
        assert_eq!(due24(&m, i64::MAX), vec![]);
    }

    #[test]
    fn exempt_tabs_are_not_due() {
        let late = 1_000 + 24 * H;
        let none = HashSet::new();

        let (m, a, b) = pair();
        assert_eq!(due24(&m, late), vec![a]);
        // visible
        let mut v = m.clone();
        v.tabs.get_mut(&b).unwrap().last_active_ms = 1_000;
        assert_eq!(due24(&v, late), vec![a], "the visible tab is exempt");
        // pinned
        let (p, _) = apply(m.clone(), Command::PinTab(a));
        assert_eq!(due24(&p, late), vec![]);
        // busy
        let (busy, _) = apply(
            m.clone(),
            Command::Busy {
                tab: a,
                reason: crate::busy::Reason::Loading,
                on: true,
            },
        );
        assert_eq!(due24(&busy, late), vec![]);
        // audible
        assert_eq!(
            due(&m, ArchiveAfter::Hours24, late, &HashSet::from([a])),
            vec![]
        );
        assert_eq!(due(&m, ArchiveAfter::Hours24, late, &none), vec![a]);
        // empty url
        let mut e = m.clone();
        e.tabs.get_mut(&a).unwrap().url.clear();
        assert_eq!(due24(&e, late), vec![]);
        // internal page
        let mut i = m.clone();
        i.tabs.get_mut(&a).unwrap().url = "wubble://history".into();
        assert_eq!(due24(&i, late), vec![]);
        // a split pane
        let (s, _) = apply(m.clone(), Command::SplitWith(a));
        assert_eq!(due24(&s, late), vec![]);
        // a Little window's tab
        let (l, _) = apply(
            m.clone(),
            Command::OpenLittle {
                url: "https://l.test/".into(),
                space: None,
                container: None,
            },
        );
        let little = *l.tabs.keys().max().unwrap();
        let mut l = l;
        l.tabs.get_mut(&little).unwrap().last_active_ms = 1_000;
        assert_eq!(due24(&l, late), vec![a]);
    }

    #[test]
    fn due_tabs_come_in_id_order_and_the_words_round_trip() {
        let m = Model::new(Instant::now());
        let (m, a) = open(m, "https://a.test/");
        let (m, b) = open(m, "https://b.test/");
        let (mut m, c) = open(m, "https://c.test/");
        for t in [a, b] {
            m.tabs.get_mut(&t).unwrap().last_active_ms = 1_000;
        }
        assert_eq!(due24(&m, 1_000 + 24 * H), vec![a, b]);
        let _ = c;
        for after in ArchiveAfter::ALL {
            assert_eq!(ArchiveAfter::from_id(after.id()), Some(after));
        }
        assert_eq!(ArchiveAfter::from_id("1h"), None);
        assert_eq!(
            crate::settings::ARCHIVE_AFTER.default,
            ArchiveAfter::Hours24.id()
        );
        let _ = Pressure::None;
    }
}
