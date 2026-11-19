//! The GPU process, watched by its process id: CEF reports no GPU-process crash (only renderer
//! deaths), so wubble notices one when the id of its process labelled "GPU" changes or goes.
//! Chromium restarts the GPU process itself; the host logs it, counts it and asks each view for
//! a fresh frame.

/// What one look at the GPU process says, against the last one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Nothing to tell: the same process, or the first sighting.
    Same,
    /// The process is gone and none has replaced it yet.
    Gone { was: u32 },
    /// Another process now does the work; `was` is `None` when the old one was seen gone
    /// earlier.
    Restarted { was: Option<u32>, now: u32 },
}

/// What the process list shows now (`now`: the GPU process id, if any) against what it showed
/// last time (`last`) and whether a process was ever seen (`seen`).
pub fn watch(last: Option<u32>, now: Option<u32>, seen: bool) -> Change {
    match (last, now) {
        (Some(a), Some(b)) if a != b => Change::Restarted {
            was: Some(a),
            now: b,
        },
        (Some(was), None) => Change::Gone { was },
        (None, Some(now)) if seen => Change::Restarted { was: None, now },
        _ => Change::Same,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_process_is_not_a_restart() {
        assert_eq!(watch(None, Some(7), false), Change::Same);
        assert_eq!(watch(None, None, false), Change::Same);
    }

    #[test]
    fn the_same_process_is_quiet() {
        assert_eq!(watch(Some(7), Some(7), true), Change::Same);
    }

    #[test]
    fn a_changed_id_is_a_restart() {
        assert_eq!(
            watch(Some(7), Some(9), true),
            Change::Restarted {
                was: Some(7),
                now: 9
            }
        );
    }

    #[test]
    fn a_vanished_process_is_gone_then_restarted_when_one_appears() {
        assert_eq!(watch(Some(7), None, true), Change::Gone { was: 7 });
        assert_eq!(watch(None, None, true), Change::Same);
        assert_eq!(
            watch(None, Some(9), true),
            Change::Restarted { was: None, now: 9 }
        );
    }
}
