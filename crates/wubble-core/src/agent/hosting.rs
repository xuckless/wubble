//! Whether wubble hosts its MCP server: one lifecycle, switched live from Settings
//! (`agent.mcp_enabled`, off by default). Turning it on binds the profile's private socket
//! (0600, never a TCP port) and writes the manifest a relay finds it by; turning it off ends
//! every session (as Stop All Agents does), stops accepting, and removes the socket and the
//! manifest. The listener thread and the files are the host's, at the edge (`mcp.rs`).

/// Where hosting is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Hosting {
    #[default]
    Off,
    /// Asked to bind; the socket is not up yet. `by_owner`: the owner turned it on (wubble then
    /// adds itself to the agent apps), else a start with it already on (wubble only keeps its own
    /// entries current).
    Starting {
        by_owner: bool,
    },
    On,
    /// The socket could not be bound; the switch is still on.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The owner turned the switch on.
    TurnOn,
    /// The browser started with the switch on.
    Resume,
    TurnOff,
    Bound,
    BindFailed(String),
    /// The browser quits: like off, but the harness entries wubble added stay (the relay says
    /// the browser is not running until it starts again).
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Bind the socket and listen (answered by [`Event::Bound`] or [`Event::BindFailed`]).
    Bind,
    /// Stop accepting, remove the socket file.
    Unbind,
    /// End every session cleanly (Stop All Agents' path).
    StopSessions,
    WriteManifest,
    RemoveManifest,
    /// Add wubble to the detected harnesses the owner has not opted out of (`harness`,
    /// `Moment::TurnOn`).
    Register,
    /// Keep wubble's own entries current, add none (`Moment::Resume`).
    Refresh,
    /// Remove the entries wubble added.
    Unregister,
}

pub fn transition(state: Hosting, event: Event) -> (Hosting, Vec<Effect>) {
    use Effect::*;
    use Hosting::*;
    match (state, event) {
        (Off | Failed(_), Event::TurnOn) => (Starting { by_owner: true }, vec![Bind]),
        (Off, Event::Resume) => (Starting { by_owner: false }, vec![Bind]),
        (Starting { by_owner: true }, Event::Bound) => (On, vec![WriteManifest, Register]),
        (Starting { by_owner: false }, Event::Bound) => (On, vec![WriteManifest, Refresh]),
        (Starting { .. }, Event::BindFailed(why)) => (Failed(why), vec![]),
        (On | Starting { .. }, Event::TurnOff) => {
            (Off, vec![StopSessions, Unbind, RemoveManifest, Unregister])
        }
        (Failed(_), Event::TurnOff) => (Off, vec![Unregister]),
        (On | Starting { .. }, Event::Quit) => (Off, vec![StopSessions, Unbind, RemoveManifest]),
        (state, _) => (state, vec![]),
    }
}

impl Hosting {
    /// Where the Settings switch is drawn: on from the owner's press until it is turned off,
    /// through Starting and a failed bind (whose status says why), so it never shows on while
    /// nothing was asked, nor off while the switch is on.
    pub fn switch_on(&self) -> bool {
        *self != Hosting::Off
    }

    /// Whether sessions may be served (the socket is up).
    pub fn serving(&self) -> bool {
        *self == Hosting::On
    }

    /// The Settings group's status line; `remote_port` is a DevTools port that refuses every
    /// session while it is set.
    pub fn status(&self, remote_port: Option<u16>) -> String {
        match (self, remote_port) {
            (Hosting::Off, _) => "Off: nothing is hosted, no agent can reach wubble.".into(),
            (Hosting::Starting { .. }, _) => "Starting…".into(),
            (Hosting::On, Some(port)) => format!(
                "Hosting, but every session is refused while the debugging port ({port}) is \
                 set: turn it off under Privacy and restart wubble."
            ),
            (Hosting::On, None) => {
                "Hosting: agents on this computer can ask for a session; each needs your approval."
                    .into()
            }
            (Hosting::Failed(why), _) => format!("Could not start: {why}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Effect::*;

    #[test]
    fn on_binds_then_writes_and_registers() {
        let (s, e) = transition(Hosting::Off, Event::TurnOn);
        assert_eq!(
            (s.clone(), e),
            (Hosting::Starting { by_owner: true }, vec![Bind])
        );
        assert!(!s.serving());
        let (s, e) = transition(s, Event::Bound);
        assert_eq!((s.clone(), e), (Hosting::On, vec![WriteManifest, Register]));
        assert!(s.serving());
        // on again: nothing
        assert_eq!(transition(s, Event::TurnOn), (Hosting::On, vec![]));
    }

    #[test]
    fn off_ends_sessions_and_removes_everything_quit_keeps_the_entries() {
        assert_eq!(
            transition(Hosting::On, Event::TurnOff),
            (
                Hosting::Off,
                vec![StopSessions, Unbind, RemoveManifest, Unregister]
            )
        );
        assert_eq!(
            transition(Hosting::On, Event::Quit),
            (Hosting::Off, vec![StopSessions, Unbind, RemoveManifest])
        );
        assert_eq!(
            transition(Hosting::Off, Event::TurnOff),
            (Hosting::Off, vec![])
        );
        assert_eq!(
            transition(Hosting::Off, Event::Quit),
            (Hosting::Off, vec![])
        );
    }

    #[test]
    fn a_failed_bind_says_why_and_can_be_tried_again() {
        let (s, e) = transition(
            Hosting::Starting { by_owner: true },
            Event::BindFailed("in use".into()),
        );
        assert_eq!((s.clone(), e), (Hosting::Failed("in use".into()), vec![]));
        assert!(s.status(None).contains("in use"));
        assert_eq!(
            transition(s.clone(), Event::TurnOn),
            (Hosting::Starting { by_owner: true }, vec![Bind])
        );
        assert_eq!(
            transition(s, Event::TurnOff),
            (Hosting::Off, vec![Unregister])
        );
    }

    #[test]
    fn the_status_says_why_sessions_are_refused() {
        assert!(Hosting::On.status(Some(9222)).contains("9222"));
        assert!(Hosting::On.status(None).contains("approval"));
        assert!(Hosting::Off.status(None).contains("nothing is hosted"));
    }

    /// A start with the switch on keeps wubble's entries current but adds none: an entry the
    /// owner deleted from an app's file stays deleted.
    #[test]
    fn a_start_refreshes_the_owner_s_turn_on_registers() {
        let (s, e) = transition(Hosting::Off, Event::Resume);
        assert_eq!(
            (s.clone(), e),
            (Hosting::Starting { by_owner: false }, vec![Bind])
        );
        assert_eq!(
            transition(s, Event::Bound),
            (Hosting::On, vec![WriteManifest, Refresh])
        );
        let owner = transition(Hosting::Off, Event::TurnOn).0;
        assert_eq!(
            transition(owner, Event::Bound),
            (Hosting::On, vec![WriteManifest, Register])
        );
        // a resume while on or after a failure does nothing
        assert_eq!(
            transition(Hosting::On, Event::Resume),
            (Hosting::On, vec![])
        );
    }

    #[test]
    fn the_switch_is_drawn_from_the_state() {
        assert!(!Hosting::Off.switch_on());
        assert!(Hosting::Starting { by_owner: true }.switch_on());
        assert!(Hosting::On.switch_on());
        assert!(Hosting::Failed("in use".into()).switch_on());
    }
}
