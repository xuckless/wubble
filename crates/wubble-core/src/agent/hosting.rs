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
    /// Asked to bind; the socket is not up yet.
    Starting,
    On,
    /// The socket could not be bound; the switch is still on.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    TurnOn,
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
    /// Add wubble to the detected harnesses the owner has not opted out of (`harness`).
    Register,
    /// Remove the entries wubble added.
    Unregister,
}

pub fn transition(state: Hosting, event: Event) -> (Hosting, Vec<Effect>) {
    use Effect::*;
    use Hosting::*;
    match (state, event) {
        (Off | Failed(_), Event::TurnOn) => (Starting, vec![Bind]),
        (Starting, Event::Bound) => (On, vec![WriteManifest, Register]),
        (Starting, Event::BindFailed(why)) => (Failed(why), vec![]),
        (On | Starting, Event::TurnOff) => {
            (Off, vec![StopSessions, Unbind, RemoveManifest, Unregister])
        }
        (Failed(_), Event::TurnOff) => (Off, vec![Unregister]),
        (On | Starting, Event::Quit) => (Off, vec![StopSessions, Unbind, RemoveManifest]),
        (state, _) => (state, vec![]),
    }
}

impl Hosting {
    /// Whether sessions may be served (the socket is up).
    pub fn serving(&self) -> bool {
        *self == Hosting::On
    }

    /// The Settings group's status line; `remote_port` is a DevTools port that refuses every
    /// session while it is set.
    pub fn status(&self, remote_port: Option<u16>) -> String {
        match (self, remote_port) {
            (Hosting::Off, _) => "Off: nothing is hosted, no agent can reach wubble.".into(),
            (Hosting::Starting, _) => "Starting…".into(),
            (Hosting::On, Some(port)) => format!(
                "Hosting, but every session is refused while the debugging port ({port}) is \
                 set: turn it off under Privacy."
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
        assert_eq!((s.clone(), e), (Hosting::Starting, vec![Bind]));
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
        let (s, e) = transition(Hosting::Starting, Event::BindFailed("in use".into()));
        assert_eq!((s.clone(), e), (Hosting::Failed("in use".into()), vec![]));
        assert!(s.status(None).contains("in use"));
        assert_eq!(
            transition(s.clone(), Event::TurnOn),
            (Hosting::Starting, vec![Bind])
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
}
