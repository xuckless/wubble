//! One media download's life: one enum of states and one pure [`transition`]. A job fetches
//! its tracks one after another (a pair: video, then audio), each as numbered segments, up to
//! `limit` at once; the engine's writer appends them in order. Pausing, an interruption and a
//! failed fetch keep the segments written so far: resuming rewinds the track's file to them
//! and fetches the rest (segment-level resume). The downloads list shows the job through
//! [`list_event`].

use std::collections::BTreeSet;

use crate::download::DownloadEvent;

/// Where the fetching of a job stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Segments per track.
    pub counts: Vec<usize>,
    /// The track being fetched.
    pub track: usize,
    /// Segments `0..done` of the track are fetched (the file holds them once written).
    pub done: usize,
    /// Fetched segments past a gap, waiting for the ones before.
    pub ahead: BTreeSet<usize>,
    /// The next segment to start.
    pub next: usize,
    /// Fetches running.
    pub running: usize,
    /// Most fetches at once (`media.concurrency`).
    pub limit: usize,
}

impl Progress {
    /// Fetching stopped: what runs is dropped, what is done stays.
    fn stopped(self) -> Progress {
        Progress {
            ahead: BTreeSet::new(),
            next: self.done,
            running: 0,
            ..self
        }
    }

    /// Segments fetched over all tracks.
    pub fn fetched(&self) -> usize {
        self.counts[..self.track].iter().sum::<usize>() + self.done + self.ahead.len()
    }

    pub fn total(&self) -> usize {
        self.counts.iter().sum()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    /// Reading the variant's playlists and keys; `paused` by the user before they came, so
    /// the plan enters [`Job::Paused`].
    Resolving {
        paused: bool,
    },
    Fetching(Progress),
    Paused(Progress),
    /// A fetch failed; resuming continues from the segments done.
    Interrupted(Progress, String),
    /// Every track is written: joining them into the file. A join cannot be stopped:
    /// `cancelled` by the user, its saved file is deleted once it ends.
    Joining {
        cancelled: bool,
    },
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobEvent {
    /// Every track's segments are known.
    Planned {
        counts: Vec<usize>,
        limit: usize,
    },
    /// The stream cannot be saved (protected, live, unreadable); also once fetching found
    /// it protected.
    Refused(String),
    SegmentDone {
        track: usize,
        index: usize,
    },
    SegmentFailed(String),
    Joined(Result<(), String>),
    // From the user.
    Pause,
    Resume,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobEffect {
    /// Fetch one segment and hand it to the writer.
    Fetch { track: usize, index: usize },
    /// Cancel every running fetch of the job.
    StopFetches,
    /// Cut the track's file back to its first `keep` segments (and drop any held past them).
    Rewind { track: usize, keep: usize },
    /// Every segment of the track is fetched: the writer finishes its file.
    CloseTrack(usize),
    /// Join the tracks into the saved file.
    Join,
    /// Delete the job's partial files (and what a cancelled join saved).
    Discard,
}

/// The fetches that keep `p` at its limit.
fn fill(mut p: Progress) -> (Progress, Vec<JobEffect>) {
    let count = p.counts[p.track];
    let mut effects = Vec::new();
    while p.running < p.limit.max(1) && p.next < count {
        effects.push(JobEffect::Fetch {
            track: p.track,
            index: p.next,
        });
        p.next += 1;
        p.running += 1;
    }
    (p, effects)
}

/// From the first track with segments (an empty track is closed at once).
fn begin(mut p: Progress) -> (Job, Vec<JobEffect>) {
    let mut effects = Vec::new();
    while p.track < p.counts.len() && p.counts[p.track] == 0 {
        effects.push(JobEffect::CloseTrack(p.track));
        p.track += 1;
    }
    if p.track == p.counts.len() {
        effects.push(JobEffect::Join);
        return (Job::Joining { cancelled: false }, effects);
    }
    let (p, fetches) = fill(p);
    effects.extend(fetches);
    (Job::Fetching(p), effects)
}

impl Job {
    /// Still fetching, or able to: it holds partial files and no saved one.
    pub fn is_live(&self) -> bool {
        matches!(
            self,
            Job::Resolving { .. } | Job::Fetching(_) | Job::Paused(_) | Job::Interrupted(..)
        )
    }
}

pub fn transition(job: Job, event: JobEvent) -> (Job, Vec<JobEffect>) {
    use JobEffect as Fx;
    use JobEvent as E;
    match (job, event) {
        (Job::Resolving { paused }, E::Planned { counts, limit }) => {
            if counts.iter().all(|&c| c == 0) {
                return (Job::Failed("nothing to download".into()), vec![]);
            }
            let p = Progress {
                counts,
                track: 0,
                done: 0,
                ahead: BTreeSet::new(),
                next: 0,
                running: 0,
                limit,
            };
            if paused {
                return (Job::Paused(p), vec![]);
            }
            begin(p)
        }
        (Job::Resolving { .. }, E::Pause) => (Job::Resolving { paused: true }, vec![]),
        (Job::Resolving { .. }, E::Resume) => (Job::Resolving { paused: false }, vec![]),
        (Job::Resolving { .. }, E::Refused(why)) => (Job::Failed(why), vec![Fx::StopFetches]),
        (Job::Fetching(_) | Job::Paused(_) | Job::Interrupted(..), E::Refused(why)) => {
            (Job::Failed(why), vec![Fx::StopFetches, Fx::Discard])
        }
        (Job::Fetching(mut p), E::SegmentDone { track, index })
            if track == p.track && index < p.next =>
        {
            p.running = p.running.saturating_sub(1);
            if index >= p.done {
                p.ahead.insert(index);
            }
            while p.ahead.remove(&p.done) {
                p.done += 1;
            }
            if p.done < p.counts[p.track] {
                let (p, effects) = fill(p);
                return (Job::Fetching(p), effects);
            }
            let mut effects = vec![Fx::CloseTrack(p.track)];
            let (job, more) = begin(Progress {
                track: p.track + 1,
                done: 0,
                ahead: BTreeSet::new(),
                next: 0,
                running: 0,
                ..p
            });
            effects.extend(more);
            (job, effects)
        }
        (Job::Fetching(p), E::SegmentFailed(why)) => {
            (Job::Interrupted(p.stopped(), why), vec![Fx::StopFetches])
        }
        (Job::Fetching(p), E::Pause) => (Job::Paused(p.stopped()), vec![Fx::StopFetches]),
        (Job::Paused(p) | Job::Interrupted(p, _), E::Resume) => {
            let mut effects = vec![Fx::Rewind {
                track: p.track,
                keep: p.done,
            }];
            // `begin`: a plan paused before it started may open on empty tracks.
            let (job, more) = begin(p);
            effects.extend(more);
            (job, effects)
        }
        (job, E::Cancel) if job.is_live() => (Job::Cancelled, vec![Fx::StopFetches, Fx::Discard]),
        (Job::Joining { .. }, E::Cancel) => (Job::Joining { cancelled: true }, vec![]),
        (Job::Joining { cancelled: true }, E::Joined(_)) => (Job::Cancelled, vec![Fx::Discard]),
        (Job::Joining { .. }, E::Joined(Ok(()))) => (Job::Done, vec![]),
        (Job::Joining { .. }, E::Joined(Err(why))) => (Job::Failed(why), vec![Fx::Discard]),
        // Late answers of stopped fetches, and anything else out of place.
        (job, _) => (job, vec![]),
    }
}

/// What the downloads list hears of a job entering `job`: the list's own pause and cancel are
/// already its state, the rest is reported.
pub fn list_event(job: &Job) -> Option<DownloadEvent> {
    match job {
        Job::Interrupted(_, why) => Some(DownloadEvent::Interrupted(why.clone())),
        Job::Done => Some(DownloadEvent::Finished),
        Job::Failed(why) => Some(DownloadEvent::Failed(why.clone())),
        Job::Fetching(_) => Some(DownloadEvent::Resumed),
        Job::Resolving { .. } | Job::Paused(_) | Job::Joining { .. } | Job::Cancelled => None,
    }
}

/// The expected size of a download from what is fetched so far: the known total, else the
/// bytes so far scaled from the segments done to all.
pub fn estimate_total(known: Option<u64>, bytes: u64, fetched: usize, total: usize) -> Option<u64> {
    known.or_else(|| (fetched > 0 && total > 0).then(|| bytes * total as u64 / fetched as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use JobEffect as Fx;
    use JobEvent as E;

    fn run(job: Job, events: Vec<E>) -> (Job, Vec<Fx>) {
        events.into_iter().fold((job, vec![]), |(job, mut all), e| {
            let (job, fx) = transition(job, e);
            all.extend(fx);
            (job, all)
        })
    }

    fn done(track: usize, index: usize) -> E {
        E::SegmentDone { track, index }
    }

    fn fetch(track: usize, index: usize) -> Fx {
        Fx::Fetch { track, index }
    }

    #[test]
    fn a_pair_fetches_both_tracks_then_joins() {
        let (job, fx) = transition(
            Job::Resolving { paused: false },
            E::Planned {
                counts: vec![3, 1],
                limit: 2,
            },
        );
        assert_eq!(fx, vec![fetch(0, 0), fetch(0, 1)]);
        // Out of order: 1 waits for 0.
        let (job, fx) = transition(job, done(0, 1));
        assert_eq!(fx, vec![fetch(0, 2)]);
        let Job::Fetching(p) = &job else { panic!() };
        assert_eq!((p.done, p.fetched(), p.total()), (0, 1, 4));
        let (job, fx) = run(job, vec![done(0, 0), done(0, 2)]);
        assert_eq!(fx, vec![Fx::CloseTrack(0), fetch(1, 0)]);
        let (job, fx) = transition(job, done(1, 0));
        assert_eq!(
            (job.clone(), fx),
            (
                Job::Joining { cancelled: false },
                vec![Fx::CloseTrack(1), Fx::Join]
            )
        );
        assert_eq!(transition(job, E::Joined(Ok(()))).0, Job::Done);
    }

    #[test]
    fn pause_and_resume_keep_what_is_done() {
        let (job, _) = run(
            Job::Resolving { paused: false },
            vec![
                E::Planned {
                    counts: vec![5],
                    limit: 3,
                },
                done(0, 0),
                done(0, 2),
            ],
        );
        let (job, fx) = transition(job, E::Pause);
        assert_eq!(fx, vec![Fx::StopFetches]);
        let Job::Paused(p) = &job else { panic!() };
        assert_eq!((p.done, p.next, p.running, p.ahead.len()), (1, 1, 0, 0));
        // A stopped fetch answering late changes nothing.
        let (job, fx) = transition(job, done(0, 3));
        assert!(fx.is_empty());
        let (job, fx) = transition(job, E::Resume);
        assert_eq!(
            fx,
            vec![
                Fx::Rewind { track: 0, keep: 1 },
                fetch(0, 1),
                fetch(0, 2),
                fetch(0, 3)
            ]
        );
        assert!(matches!(job, Job::Fetching(_)));
    }

    #[test]
    fn a_failed_fetch_interrupts_and_resumes() {
        let (job, _) = run(
            Job::Resolving { paused: false },
            vec![
                E::Planned {
                    counts: vec![2],
                    limit: 1,
                },
                done(0, 0),
            ],
        );
        let (job, fx) = transition(job, E::SegmentFailed("HTTP 503".into()));
        assert_eq!(fx, vec![Fx::StopFetches]);
        assert!(matches!(&job, Job::Interrupted(p, why) if p.done == 1 && why == "HTTP 503"));
        assert_eq!(
            list_event(&job),
            Some(DownloadEvent::Interrupted("HTTP 503".into()))
        );
        let (job, fx) = transition(job, E::Resume);
        assert_eq!(fx, vec![Fx::Rewind { track: 0, keep: 1 }, fetch(0, 1)]);
        assert_eq!(list_event(&job), Some(DownloadEvent::Resumed));
    }

    #[test]
    fn cancel_from_every_live_state() {
        let fetching = transition(
            Job::Resolving { paused: false },
            E::Planned {
                counts: vec![2],
                limit: 1,
            },
        )
        .0;
        let paused = transition(fetching.clone(), E::Pause).0;
        let interrupted = transition(fetching.clone(), E::SegmentFailed("x".into())).0;
        for job in [
            Job::Resolving { paused: false },
            fetching,
            paused,
            interrupted,
        ] {
            assert_eq!(
                transition(job, E::Cancel),
                (Job::Cancelled, vec![Fx::StopFetches, Fx::Discard])
            );
        }
        // A cancel is ignored by the finished states.
        for job in [Job::Done, Job::Failed("x".into()), Job::Cancelled] {
            assert_eq!(transition(job.clone(), E::Cancel), (job, vec![]));
        }
    }

    #[test]
    fn refusals_and_empty_plans_fail() {
        let (job, fx) = transition(
            Job::Resolving { paused: false },
            E::Refused("Protected, cannot be saved".into()),
        );
        assert_eq!(
            (job.clone(), fx),
            (
                Job::Failed("Protected, cannot be saved".into()),
                vec![Fx::StopFetches]
            )
        );
        assert_eq!(
            list_event(&job),
            Some(DownloadEvent::Failed("Protected, cannot be saved".into()))
        );
        assert_eq!(
            transition(
                Job::Resolving { paused: false },
                E::Planned {
                    counts: vec![0],
                    limit: 4
                }
            )
            .0,
            Job::Failed("nothing to download".into())
        );
        // An empty audio track after the video closes at once.
        let (job, fx) = run(
            Job::Resolving { paused: false },
            vec![
                E::Planned {
                    counts: vec![1, 0],
                    limit: 4,
                },
                done(0, 0),
            ],
        );
        assert_eq!(
            (job, fx[1..].to_vec()),
            (
                Job::Joining { cancelled: false },
                vec![Fx::CloseTrack(0), Fx::CloseTrack(1), Fx::Join]
            )
        );
        let (failed, fx) = transition(
            Job::Joining { cancelled: false },
            E::Joined(Err("ffmpeg failed".into())),
        );
        assert_eq!(failed, Job::Failed("ffmpeg failed".into()));
        assert_eq!(fx, vec![Fx::Discard]);
    }

    #[test]
    fn a_stream_found_protected_while_fetching_stops() {
        let (job, _) = transition(
            Job::Resolving { paused: false },
            E::Planned {
                counts: vec![4],
                limit: 2,
            },
        );
        let (job, fx) = transition(job, E::Refused("Protected, cannot be saved".into()));
        assert_eq!(job, Job::Failed("Protected, cannot be saved".into()));
        assert_eq!(fx, vec![Fx::StopFetches, Fx::Discard]);
    }

    #[test]
    fn segments_never_started_are_ignored() {
        let (job, _) = transition(
            Job::Resolving { paused: false },
            E::Planned {
                counts: vec![9],
                limit: 1,
            },
        );
        let (same, fx) = transition(job.clone(), done(0, 5));
        assert_eq!((same, fx), (job.clone(), vec![]));
        let (same, fx) = transition(job.clone(), done(1, 0));
        assert_eq!((same, fx), (job, vec![]));
    }

    #[test]
    fn a_cancel_while_joining_deletes_what_the_join_saves() {
        let joining = Job::Joining { cancelled: false };
        let (job, fx) = transition(joining, E::Cancel);
        assert_eq!(
            (job.clone(), fx),
            (Job::Joining { cancelled: true }, vec![])
        );
        assert_eq!(list_event(&job), None);
        for result in [Ok(()), Err("ffmpeg failed".into())] {
            assert_eq!(
                transition(job.clone(), E::Joined(result)),
                (Job::Cancelled, vec![Fx::Discard])
            );
        }
    }

    #[test]
    fn a_pause_before_the_plan_holds_it() {
        let planned = E::Planned {
            counts: vec![0, 2],
            limit: 4,
        };
        let (job, fx) = transition(Job::Resolving { paused: false }, E::Pause);
        assert_eq!((job.clone(), fx), (Job::Resolving { paused: true }, vec![]));
        let (job, fx) = transition(job, planned.clone());
        assert!(fx.is_empty());
        assert!(matches!(&job, Job::Paused(p) if p.running == 0 && p.track == 0));
        assert_eq!(list_event(&job), None);
        // Resuming starts past the empty first track.
        let (job, fx) = transition(job, E::Resume);
        assert_eq!(
            fx,
            vec![
                Fx::Rewind { track: 0, keep: 0 },
                Fx::CloseTrack(0),
                fetch(1, 0),
                fetch(1, 1)
            ]
        );
        assert!(matches!(&job, Job::Fetching(p) if p.track == 1));
        // A resume before the plan came lets it start.
        let (job, _) = transition(Job::Resolving { paused: true }, E::Resume);
        let (_, fx) = transition(job, planned);
        assert_eq!(fx, vec![Fx::CloseTrack(0), fetch(1, 0), fetch(1, 1)]);
    }

    #[test]
    fn totals_are_estimated() {
        assert_eq!(estimate_total(Some(10), 3, 1, 9), Some(10));
        assert_eq!(estimate_total(None, 300, 1, 4), Some(1200));
        assert_eq!(estimate_total(None, 0, 0, 4), None);
    }
}
