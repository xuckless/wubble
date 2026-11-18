//! One download's life. welding reports it (`DownloadRequested` while the
//! host picks a path, then started, progress, finished or cancelled) and the
//! host steers it (pause, resume, cancel, or declining the path). Pause and
//! resume act at once: welding keeps CEF's latest item callback.
//!
//! A download whose life ended (finished, failed, cancelled, declined) has no
//! state: the view forgets it.

use std::path::PathBuf;

use crate::page::{DownloadId, PageEvent, RequestId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Download {
    /// Waiting for the host to answer `request` with a path.
    Requested {
        request: RequestId,
    },
    Running {
        received: u64,
        total: Option<u64>,
    },
    Paused {
        received: u64,
        total: Option<u64>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadInput {
    // From CEF.
    Offered {
        request: RequestId,
        url: String,
        suggested_name: String,
        total: Option<u64>,
    },
    Started {
        url: String,
        suggested_name: String,
        path: PathBuf,
        total: Option<u64>,
    },
    Progress {
        received: u64,
        total: Option<u64>,
    },
    Done {
        path: PathBuf,
        error: Option<String>,
    },
    Cancelled,
    // From the host.
    Pause,
    Resume,
    Cancel,
    /// The host answered the path request with none.
    Declined,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadEffect {
    Emit(PageEvent),
    Pause,
    Resume,
    Cancel,
}

/// `state` is `None` for a download the view does not know (yet, or any
/// more). Every pair not listed is a no-op.
pub fn transition(
    state: Option<Download>,
    id: DownloadId,
    input: DownloadInput,
) -> (Option<Download>, Vec<DownloadEffect>) {
    use {Download::*, DownloadEffect::Emit, DownloadInput as In};
    let live = matches!(state, Some(Running { .. } | Paused { .. }));
    match (state, input) {
        (
            None,
            In::Offered {
                request,
                url,
                suggested_name,
                total,
            },
        ) => (
            Some(Requested { request }),
            vec![Emit(PageEvent::DownloadRequested {
                id: request,
                download: id,
                url,
                suggested_name,
                total,
            })],
        ),
        (
            None | Some(Requested { .. }),
            In::Started {
                url,
                suggested_name,
                path,
                total,
            },
        ) => (
            Some(Running { received: 0, total }),
            vec![Emit(PageEvent::DownloadStarted {
                download: id,
                url,
                suggested_name,
                path,
                total,
            })],
        ),
        (
            Some(Running {
                received: r,
                total: t,
            }),
            In::Progress { received, total },
        )
        | (
            Some(Paused {
                received: r,
                total: t,
            }),
            In::Progress { received, total },
        ) => {
            let next = match state {
                Some(Paused { .. }) => Paused { received, total },
                _ => Running { received, total },
            };
            let effects = if (r, t) == (received, total) {
                vec![]
            } else {
                vec![Emit(PageEvent::DownloadProgress {
                    download: id,
                    received,
                    total,
                })]
            };
            (Some(next), effects)
        }
        (Some(Running { received, total }), In::Pause) => (
            Some(Paused { received, total }),
            vec![DownloadEffect::Pause, Emit(PageEvent::DownloadPaused(id))],
        ),
        (Some(Paused { received, total }), In::Resume) => (
            Some(Running { received, total }),
            vec![DownloadEffect::Resume, Emit(PageEvent::DownloadResumed(id))],
        ),
        // CEF confirms with Cancelled.
        (Some(Running { .. } | Paused { .. }), In::Cancel) => (state, vec![DownloadEffect::Cancel]),
        (Some(Requested { .. }), In::Declined) => {
            (None, vec![Emit(PageEvent::DownloadCancelled(id))])
        }
        (Some(_), In::Done { path, error: None }) if live => (
            None,
            vec![Emit(PageEvent::DownloadFinished { download: id, path })],
        ),
        (
            Some(_),
            In::Done {
                error: Some(error), ..
            },
        ) if live => (
            None,
            vec![Emit(PageEvent::DownloadFailed {
                download: id,
                error,
            })],
        ),
        (Some(_), In::Cancelled) => (None, vec![Emit(PageEvent::DownloadCancelled(id))]),
        (state, _) => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::{Download::*, DownloadEffect::*, DownloadInput as In, *};

    const ID: DownloadId = DownloadId(4);
    const REQ: RequestId = RequestId(9);

    fn offered() -> In {
        In::Offered {
            request: REQ,
            url: "https://x/f.zip".into(),
            suggested_name: "f.zip".into(),
            total: Some(100),
        }
    }

    fn started() -> In {
        In::Started {
            url: "https://x/f.zip".into(),
            suggested_name: "f.zip".into(),
            path: "/d/f.zip".into(),
            total: Some(100),
        }
    }

    fn run(inputs: Vec<In>) -> (Option<Download>, Vec<DownloadEffect>) {
        inputs
            .into_iter()
            .fold((None, vec![]), |(state, mut all), input| {
                let (next, effects) = transition(state, ID, input);
                all.extend(effects);
                (next, all)
            })
    }

    fn progress(received: u64) -> In {
        In::Progress {
            received,
            total: Some(100),
        }
    }

    #[test]
    fn offered_started_paused_resumed_finished() {
        let (state, effects) = run(vec![
            offered(),
            started(),
            progress(10),
            progress(10),
            In::Pause,
            progress(20),
            In::Resume,
            In::Done {
                path: "/d/f.zip".into(),
                error: None,
            },
        ]);
        assert_eq!(state, None, "a finished download is forgotten");
        let kinds: Vec<_> = effects
            .iter()
            .map(|e| match e {
                Emit(PageEvent::DownloadRequested { .. }) => "requested",
                Emit(PageEvent::DownloadStarted { .. }) => "started",
                Emit(PageEvent::DownloadProgress { .. }) => "progress",
                Emit(PageEvent::DownloadPaused(_)) => "paused",
                Emit(PageEvent::DownloadResumed(_)) => "resumed",
                Emit(PageEvent::DownloadFinished { .. }) => "finished",
                Pause => "PAUSE",
                Resume => "RESUME",
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "requested",
                "started",
                "progress",
                "PAUSE",
                "paused",
                "progress",
                "RESUME",
                "resumed",
                "finished"
            ]
        );
    }

    #[test]
    fn progress_while_paused_stays_paused() {
        let (state, _) = run(vec![started(), In::Pause, progress(30)]);
        assert_eq!(
            state,
            Some(Paused {
                received: 30,
                total: Some(100)
            })
        );
    }

    #[test]
    fn failure_and_cancel_end_it() {
        let (state, effects) = run(vec![
            started(),
            In::Done {
                path: "/d/f.zip".into(),
                error: Some("network".into()),
            },
        ]);
        assert_eq!(state, None);
        assert_eq!(
            effects.last(),
            Some(&Emit(PageEvent::DownloadFailed {
                download: ID,
                error: "network".into()
            }))
        );
        let (state, effects) = run(vec![started(), In::Cancel, In::Cancelled]);
        assert_eq!(state, None);
        assert_eq!(
            effects[1..],
            [Cancel, Emit(PageEvent::DownloadCancelled(ID))]
        );
    }

    #[test]
    fn declining_the_path_cancels() {
        let (state, effects) = run(vec![offered(), In::Declined]);
        assert_eq!(state, None);
        assert_eq!(
            effects.last(),
            Some(&Emit(PageEvent::DownloadCancelled(ID)))
        );
    }

    #[test]
    fn impossible_moves_are_no_ops() {
        let requested = Some(Requested { request: REQ });
        let running = Some(Running {
            received: 1,
            total: None,
        });
        let paused = Some(Paused {
            received: 1,
            total: None,
        });
        for (state, input) in [
            (None, In::Pause),
            (None, In::Resume),
            (None, In::Cancel),
            (None, progress(5)),
            (None, In::Cancelled),
            (None, In::Declined),
            (requested, In::Pause),
            (requested, In::Cancel),
            (requested, progress(5)),
            (
                requested,
                In::Done {
                    path: "/x".into(),
                    error: None,
                },
            ),
            (running, In::Resume),
            (running, In::Declined),
            (running, offered()),
            (paused, In::Pause),
            (paused, started()),
        ] {
            assert_eq!(
                transition(state, ID, input.clone()),
                (state, vec![]),
                "{state:?} + {input:?}"
            );
        }
    }
}
