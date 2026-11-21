//! A windowed run's verdict: the line a smoke (or `--measure`, or the browser) ends with, and
//! its exit code. One owner for that text; it is printed before CEF is torn down, so a
//! teardown that hangs or crashes after a passed smoke still shows the pass.

use std::process::ExitCode;

use crate::{Mode, Outcome};

/// What a run prints when it ends: `out` on stdout, then `failed` on stderr. A run passed when
/// nothing failed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    pub out: Option<String>,
    pub failed: Option<String>,
}

impl Verdict {
    /// The verdict of `mode`'s run that ended with `outcome`. Pure.
    pub fn of(mode: &Mode, outcome: &Outcome) -> Self {
        let failure = outcome.failure.as_ref().map(ToString::to_string);
        match mode {
            Mode::Browse { .. } => Self {
                out: None,
                failed: failure.map(|e| format!("wubble: {e}")),
            },
            Mode::Measure { .. } => Self {
                out: None,
                failed: match (failure, &outcome.measured) {
                    (Some(e), _) => Some(format!("measure: FAILED: {e}")),
                    (None, Some(Ok(()))) => None,
                    (None, Some(Err(e))) => Some(format!("measure: {e}")),
                    (None, None) => Some("measure: FAILED: nothing was recorded".into()),
                },
            },
            Mode::Frames { .. } => Self {
                out: Some(outcome.stats.summary_line()),
                failed: match failure {
                    Some(e) => Some(format!("smoke: frames FAILED: {e}")),
                    None if outcome.stats.is_success() => None,
                    None => Some("smoke: frames FAILED: no zero-copy frames".into()),
                },
            },
            _ => {
                let (label, passed) = smoke(mode, outcome);
                match (failure, passed) {
                    (Some(e), _) => Self::failed(format!("{label} FAILED: {e}")),
                    (None, Some(passed)) => Self {
                        out: Some(format!("{label} {passed}")),
                        failed: None,
                    },
                    (None, None) => {
                        Self::failed(format!("{label} FAILED: the window closed first"))
                    }
                }
            }
        }
    }

    /// The verdict of a run that could not start: `mode`'s failure line for `e`.
    pub fn of_error(mode: &Mode, e: Box<dyn std::error::Error>) -> Self {
        let outcome = Outcome {
            failure: Some(e),
            ..Outcome::default()
        };
        Self::of(mode, &outcome)
    }

    fn failed(line: String) -> Self {
        Self {
            out: None,
            failed: Some(line),
        }
    }

    /// Prints the verdict and gives the run's exit code.
    pub fn print(&self) -> ExitCode {
        if let Some(out) = &self.out {
            println!("{out}");
        }
        match &self.failed {
            None => ExitCode::SUCCESS,
            Some(failed) => {
                eprintln!("{failed}");
                ExitCode::FAILURE
            }
        }
    }
}

/// A smoke's line prefix, and what follows it when the smoke reached its end.
fn smoke(mode: &Mode, o: &Outcome) -> (String, Option<String>) {
    let ok = |reached: bool| reached.then(|| "ok".to_owned());
    let (name, passed) = match mode {
        Mode::Nav => ("nav", o.nav_title.as_ref().map(|t| format!("ok {t}"))),
        Mode::Tabs => ("tabs", ok(o.tabs_passed)),
        Mode::Windows => ("windows", ok(o.windows_passed)),
        Mode::Split => ("split", ok(o.split_passed)),
        Mode::Pinch => ("pinch", o.pinch_passed.as_ref().map(|s| format!("ok {s}"))),
        Mode::Page => ("page", o.page_zoom.map(|z| format!("ok zoom={z}%"))),
        Mode::Parity => (
            "parity",
            o.parity_passed
                .as_ref()
                .map(|gaps| format!("ok; known gaps: {}", gaps.join(", "))),
        ),
        Mode::Block => ("block", ok(o.blocked_seen.is_some())),
        Mode::Settings => ("settings", ok(o.settings_passed)),
        Mode::Start => ("start", ok(o.start_passed)),
        Mode::Pages => ("pages", ok(o.pages_passed)),
        Mode::Data(_) => ("data", ok(o.data_passed)),
        Mode::Gallery => ("gallery", ok(o.gallery_passed)),
        Mode::Autofill { .. } => ("autofill", ok(o.autofill_passed)),
        Mode::Media => ("media", ok(o.media_passed)),
        Mode::Builtins => (
            "builtins",
            o.builtins_passed.as_ref().map(|w| format!("ok ({w})")),
        ),
        Mode::Video => ("video", ok(o.video_passed.is_some())),
        Mode::Boost => ("boost", ok(o.video_passed.is_some())),
        Mode::Ax => ("ax", o.ax_passed.map(|n| format!("ok ({n} items)"))),
        Mode::Mcp => ("mcp", o.mcp_passed.map(|n| format!("ok ({n} steps)"))),
        Mode::Overlay { .. } => ("overlay", ok(o.overlay_passed)),
        Mode::Session(phase) => {
            return (
                format!("smoke: session phase {phase:?}"),
                ok(o.session_passed),
            );
        }
        Mode::Passwords(phase) => {
            return (
                format!("smoke: passwords phase {phase:?}"),
                ok(o.passwords_passed),
            );
        }
        Mode::Browse { .. } | Mode::Frames { .. } | Mode::Measure { .. } => {
            unreachable!("not a pass-or-fail smoke")
        }
    };
    (format!("smoke: {name}"), passed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Phase;

    fn passed(out: &str) -> Verdict {
        Verdict {
            out: Some(out.into()),
            failed: None,
        }
    }

    fn failed(line: &str) -> Verdict {
        Verdict {
            out: None,
            failed: Some(line.into()),
        }
    }

    fn with_failure() -> Outcome {
        Outcome {
            failure: Some("boom".into()),
            ..Outcome::default()
        }
    }

    #[test]
    fn a_smoke_passes_fails_or_finds_its_window_closed() {
        let nav = Outcome {
            nav_title: Some("Two".into()),
            ..Outcome::default()
        };
        assert_eq!(Verdict::of(&Mode::Nav, &nav), passed("smoke: nav ok Two"));
        assert_eq!(
            Verdict::of(&Mode::Nav, &with_failure()),
            failed("smoke: nav FAILED: boom")
        );
        assert_eq!(
            Verdict::of(&Mode::Tabs, &Outcome::default()),
            failed("smoke: tabs FAILED: the window closed first")
        );
        let page = Outcome {
            page_zoom: Some(110),
            ..Outcome::default()
        };
        assert_eq!(
            Verdict::of(&Mode::Page, &page),
            passed("smoke: page ok zoom=110%")
        );
        let parity = Outcome {
            parity_passed: Some(vec!["a", "b"]),
            ..Outcome::default()
        };
        assert_eq!(
            Verdict::of(&Mode::Parity, &parity),
            passed("smoke: parity ok; known gaps: a, b")
        );
        let session = Outcome {
            session_passed: true,
            ..Outcome::default()
        };
        assert_eq!(
            Verdict::of(&Mode::Session(Phase::Read), &session),
            passed("smoke: session phase Read ok")
        );
        let boost = Outcome {
            video_passed: Some("x".into()),
            ..Outcome::default()
        };
        assert_eq!(Verdict::of(&Mode::Boost, &boost), passed("smoke: boost ok"));
    }

    /// A failure wins over a pass the run also reached.
    #[test]
    fn a_failure_wins() {
        let outcome = Outcome {
            tabs_passed: true,
            ..with_failure()
        };
        assert_eq!(
            Verdict::of(&Mode::Tabs, &outcome),
            failed("smoke: tabs FAILED: boom")
        );
        assert_eq!(
            Verdict::of_error(&Mode::Windows, "no GPU".into()),
            failed("smoke: windows FAILED: no GPU")
        );
    }

    #[test]
    fn frames_always_print_their_summary() {
        let mode = Mode::Frames {
            frames: 1,
            url: None,
        };
        let verdict = Verdict::of(&mode, &Outcome::default());
        assert_eq!(verdict.out, Some(Outcome::default().stats.summary_line()));
        assert_eq!(
            verdict.failed.as_deref(),
            Some("smoke: frames FAILED: no zero-copy frames")
        );
    }

    #[test]
    fn the_browser_and_measure_print_only_failures() {
        let browse = Mode::Browse {
            url: None,
            downloads: None,
            listener: None,
        };
        assert_eq!(
            Verdict::of(&browse, &Outcome::default()),
            Verdict::default()
        );
        assert_eq!(
            Verdict::of(&browse, &with_failure()),
            failed("wubble: boom")
        );
        let measure = Mode::Measure {
            scenario: crate::measure::Scenario::Scroll,
            begin_frames: false,
            out: None,
            load: None,
        };
        let measured = Outcome {
            measured: Some(Ok(())),
            ..Outcome::default()
        };
        assert_eq!(Verdict::of(&measure, &measured), Verdict::default());
        assert_eq!(
            Verdict::of(&measure, &Outcome::default()),
            failed("measure: FAILED: nothing was recorded")
        );
    }
}
