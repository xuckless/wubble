//! `upload_file` and the page's file choosers in a session's tab: what a call in flight on the
//! tab means for a chooser that opens there (the host maps its waiting calls to [`Stage`]s and
//! acts on the verdicts).

use super::Mode;

/// What one call in flight on a tab is doing, as a file chooser opening there sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// `upload_file` clicked its element (its input fence may not have answered yet): the
    /// chooser the page opens now is its.
    UploadClicked,
    /// `upload_file` in any other stage: its paths being checked, its element located,
    /// scrolled to, read, given the files, or the page's tree awaited.
    Upload,
    /// Another call whose input or script may open a chooser: a click, a key, typed text, a
    /// script (`evaluate`) and the settle after them.
    Input,
    /// A call that sends the page no input.
    Other,
}

/// Whether an `upload_file` is in flight on the tab whose calls are `stages`: a second one is
/// refused while it is (one at a time per tab).
pub fn upload_in_flight(stages: &[Stage]) -> bool {
    stages
        .iter()
        .any(|s| matches!(s, Stage::Upload | Stage::UploadClicked))
}

/// What becomes of a file chooser that opened in a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chooser {
    /// Not a session's tab, or the owner's own in a windowed session with no agent call in
    /// flight on the tab: the OS dialog, as in any tab.
    Owner,
    /// The clicked `upload_file`'s: its files answer it.
    Answer,
    /// Cancelled: a headless session's tab (no one could answer it), or an agent call's input
    /// or script may have opened it. The calls that may have ([`Stage::Input`],
    /// [`Stage::UploadClicked`]) fail with [`CHOOSER_CANCELLED`].
    Cancel,
}

/// What becomes of a chooser that opened in a tab driven by a session in `mode` (`None`: no
/// session's), with `stages` in flight there.
pub fn chooser(mode: Option<Mode>, stages: &[Stage]) -> Chooser {
    let clicked = stages
        .iter()
        .filter(|s| **s == Stage::UploadClicked)
        .count();
    let input = stages.contains(&Stage::Input);
    match mode {
        None => Chooser::Owner,
        // with another click or script in flight beside it, whose chooser it is cannot be told
        Some(_) if clicked == 1 && !input => Chooser::Answer,
        Some(Mode::Headless) => Chooser::Cancel,
        Some(Mode::Windowed) if input || upload_in_flight(stages) => Chooser::Cancel,
        Some(Mode::Windowed) => Chooser::Owner,
    }
}

/// The advice to a call whose input or script opened a file chooser, which was cancelled.
pub const CHOOSER_CANCELLED: &str = "this opened a file chooser, which agent sessions do not \
     show (it was cancelled): to attach files, call upload_file with the file input's ref, or \
     with the button that opens the chooser";

#[cfg(test)]
mod tests {
    use super::*;
    use Stage::*;

    #[test]
    fn one_upload_at_a_time_whatever_stage_it_is_in() {
        assert!(!upload_in_flight(&[]));
        assert!(!upload_in_flight(&[Input, Other]));
        assert!(upload_in_flight(&[Other, Upload]));
        assert!(upload_in_flight(&[UploadClicked]));
    }

    #[test]
    fn a_chooser_is_the_owners_unless_an_agent_could_have_opened_it() {
        let w = Some(Mode::Windowed);
        let h = Some(Mode::Headless);
        assert_eq!(chooser(None, &[Input]), Chooser::Owner);
        // the owner's own, in a windowed session's tab with nothing in flight
        assert_eq!(chooser(w, &[]), Chooser::Owner);
        assert_eq!(chooser(w, &[Other]), Chooser::Owner);
        assert_eq!(chooser(w, &[Input]), Chooser::Cancel);
        assert_eq!(chooser(w, &[Upload]), Chooser::Cancel);
        // no one could answer one in a headless tab
        assert_eq!(chooser(h, &[]), Chooser::Cancel);
        assert_eq!(chooser(h, &[Other]), Chooser::Cancel);
        // the clicked upload's, unless another click or script is in flight beside it
        assert_eq!(chooser(w, &[UploadClicked]), Chooser::Answer);
        assert_eq!(chooser(h, &[UploadClicked, Other]), Chooser::Answer);
        assert_eq!(chooser(h, &[UploadClicked, Input]), Chooser::Cancel);
        assert_eq!(chooser(w, &[UploadClicked, Input]), Chooser::Cancel);
    }
}
