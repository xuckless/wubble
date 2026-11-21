//! `upload_file` and the page's file choosers in a session's tab: what a call in flight on the
//! tab means for a chooser that opens there (the host maps its waiting calls to [`Stage`]s and
//! acts on the verdicts).

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
}
