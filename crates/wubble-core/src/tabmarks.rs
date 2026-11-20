//! What a tab row marks besides its title: whether an agent session drives the tab, and the
//! sound badge (the extended row's speaker, the rail's badge on the favicon's top-right corner).
//!
//! The shell only draws what [`Marks::of`] says and reports a press on the badge; one state
//! (the page plays sound, the tab is muted) gives one badge on both layouts.

/// The sound badge's state: the page plays sound, or the tab is muted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    /// Audible and not muted: a press mutes the tab (the host toggles the view's mute).
    Playing,
    /// Muted, audible or not: a press unmutes the tab.
    Muted,
}

impl Sound {
    /// What a press on the badge does, for assistive technology and a tooltip.
    pub fn press_label(self) -> &'static str {
        match self {
            Sound::Playing => "Mute tab",
            Sound::Muted => "Unmute tab",
        }
    }
}

/// A tab row's marks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    /// An agent session drives the tab: the agent badge on its favicon's bottom-right corner.
    pub agent: bool,
    /// None while the tab is quiet and not muted. A muted tab keeps its badge whether or not it
    /// plays, so it can be unmuted.
    pub sound: Option<Sound>,
}

impl Marks {
    pub fn of(agent: bool, audible: bool, muted: bool) -> Marks {
        let sound = if muted {
            Some(Sound::Muted)
        } else if audible {
            Some(Sound::Playing)
        } else {
            None
        };
        Marks { agent, sound }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quiet_tab_has_no_sound_badge() {
        assert_eq!(Marks::of(false, false, false).sound, None);
    }

    #[test]
    fn a_playing_tab_shows_the_badge_and_a_press_mutes_it() {
        let s = Marks::of(false, true, false).sound.unwrap();
        assert_eq!(s, Sound::Playing);
        assert_eq!(s.press_label(), "Mute tab");
    }

    #[test]
    fn a_muted_tab_keeps_the_badge_playing_or_not_and_a_press_unmutes_it() {
        for audible in [false, true] {
            let s = Marks::of(false, audible, true).sound.unwrap();
            assert_eq!(s, Sound::Muted);
            assert_eq!(s.press_label(), "Unmute tab");
        }
    }

    #[test]
    fn the_agent_mark_is_independent_of_the_sound() {
        assert!(Marks::of(true, false, false).agent);
        assert!(!Marks::of(false, true, true).agent);
        assert_eq!(Marks::of(true, true, false).sound, Some(Sound::Playing));
    }
}
