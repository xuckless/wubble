//! `upload_file` and the page's file choosers in a session's tab: what a call in flight on the
//! tab means for a chooser that opens there (the host maps its waiting calls to [`Stage`]s and
//! acts on the verdicts), the page script that reads the element the files go to, and the pure
//! reading of its answer.
//!
//! The files go to an element, never to "the next chooser": an `<input type=file>` named by ref
//! takes them through DevTools `DOM.setFileInputFiles` (Blink fires `input` and `change`; no
//! click, no chooser), as Playwright's `setInputFiles` does; any other element (a custom upload
//! button, a label) is clicked while the tab's DevTools session intercepts file choosers
//! (`Page.setInterceptFileChooserDialog`), and the `<input>` that opened the chooser
//! (`Page.fileChooserOpened` names it) takes them the same way once its document's origin is
//! one of the session's sites. `accept=` is not enforced (Chromium's DevTools and Playwright do
//! not either).

use serde_json::Value;

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

/// How a file chooser reached the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    /// CEF's file dialog (`OnFileDialog`): the tab's DevTools session did not intercept it
    /// (none asked it to, or it opened where that session does not reach: a frame of another
    /// site, in its own process).
    Dialog,
    /// `Page.fileChooserOpened`: intercepted, nothing shown; `multiple` for `selectMultiple`.
    DevTools { multiple: bool },
}

/// What becomes of a file chooser that opened in a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chooser {
    /// Not a session's tab, or the owner's own in a windowed session with no agent call in
    /// flight on the tab: the OS dialog, as in any tab.
    Owner,
    /// The clicked `upload_file`'s: its files go to the element that opened it.
    Answer,
    /// The clicked `upload_file`'s, but its files do not fit: it fails with this.
    Refuse(&'static str),
    /// Cancelled: a headless session's tab (no one could answer it), or an agent call's input
    /// or script may have opened it. The calls that may have ([`Stage::Input`],
    /// [`Stage::UploadClicked`]) fail: [`CHOOSER_CANCELLED`], or [`CHOOSER_UNREACHED`] for a
    /// clicked upload whose chooser DevTools did not catch.
    Cancel,
    /// Intercepted, and no call waits for it: DevTools showed nothing, nothing to answer.
    Drop,
}

/// What becomes of a chooser that `opened` in a tab driven by a session in `mode` (`None`: no
/// session's), with `stages` in flight there and `files` paths in the clicked upload's call.
pub fn chooser(mode: Option<Mode>, stages: &[Stage], opened: Opened, files: usize) -> Chooser {
    let clicked = stages
        .iter()
        .filter(|s| **s == Stage::UploadClicked)
        .count();
    let input = stages.contains(&Stage::Input);
    match opened {
        // with another click or script in flight beside it, whose chooser it is cannot be told
        Opened::DevTools { multiple } if clicked == 1 && !input => {
            if files > 1 && !multiple {
                Chooser::Refuse(ONE_FILE)
            } else {
                Chooser::Answer
            }
        }
        Opened::DevTools { .. } if clicked > 0 || input => Chooser::Cancel,
        Opened::DevTools { .. } => Chooser::Drop,
        Opened::Dialog => match mode {
            None => Chooser::Owner,
            // a clicked upload's chooser that DevTools did not catch is never answered: which
            // element and frame asked cannot be told
            Some(Mode::Headless) => Chooser::Cancel,
            Some(Mode::Windowed) if input || upload_in_flight(stages) => Chooser::Cancel,
            Some(Mode::Windowed) => Chooser::Owner,
        },
    }
}

/// A file input given more files than it takes.
const ONE_FILE: &str = "this file input takes one file (it has no `multiple`): give one path";

/// The advice to a call whose input or script opened a file chooser, which was cancelled.
pub const CHOOSER_CANCELLED: &str = "this opened a file chooser, which agent sessions do not \
     show (it was cancelled): to attach files, call upload_file with the file input's ref, or \
     with the button that opens the chooser";

/// The failure of a clicked `upload_file` whose chooser reached the browser without passing
/// the tab's DevTools session, which names the element and frame it came from.
pub const CHOOSER_UNREACHED: &str = "the click opened a file chooser where wubble cannot tell \
     which element and frame asked (a frame of another site, in its own process): it was \
     cancelled and no files were given";

/// Called on the node the files would go to (`Runtime.callFunctionOn`): whether it is an
/// `<input type=file>`, whether it takes several files or a folder, and its document's origin.
pub const TARGET_SCRIPT: &str = "function(){\
const d=this.ownerDocument,w=d&&d.defaultView;\
const file=this.nodeType===1&&this.localName==='input'&&\
this.namespaceURI==='http://www.w3.org/1999/xhtml'&&this.type==='file';\
return{file,multiple:file&&!!this.multiple,folder:file&&!!this.webkitdirectory,\
origin:w?String(w.origin):''}}";

/// The element the files would go to, as [`TARGET_SCRIPT`] read it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub file: bool,
    pub multiple: bool,
    pub folder: bool,
    /// Its document's origin (`null` for an opaque one).
    pub origin: String,
}

/// The page's answer to [`TARGET_SCRIPT`] (`Runtime.callFunctionOn` by value), or `None`.
pub fn parse_target(v: &Value) -> Option<Target> {
    let v = v.get("result")?.get("value")?;
    Some(Target {
        file: v.get("file")?.as_bool()?,
        multiple: v["multiple"].as_bool().unwrap_or(false),
        folder: v["folder"].as_bool().unwrap_or(false),
        origin: v["origin"].as_str().unwrap_or_default().to_owned(),
    })
}

/// Whether `files` paths may go to `target`, a file input: one unless it takes several, never
/// to a folder picker.
pub fn fits(target: &Target, files: usize) -> Result<(), &'static str> {
    if target.folder {
        Err("this file input picks a folder (webkitdirectory), not files")
    } else if files > 1 && !target.multiple {
        Err(ONE_FILE)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Stage::*;
    use serde_json::json;

    #[test]
    fn one_upload_at_a_time_whatever_stage_it_is_in() {
        assert!(!upload_in_flight(&[]));
        assert!(!upload_in_flight(&[Input, Other]));
        assert!(upload_in_flight(&[Other, Upload]));
        assert!(upload_in_flight(&[UploadClicked]));
    }

    #[test]
    fn a_dialog_is_the_owners_unless_an_agent_could_have_opened_it() {
        let w = Some(Mode::Windowed);
        let h = Some(Mode::Headless);
        let d = Opened::Dialog;
        assert_eq!(chooser(None, &[Input], d, 0), Chooser::Owner);
        // the owner's own, in a windowed session's tab with nothing in flight
        assert_eq!(chooser(w, &[], d, 0), Chooser::Owner);
        assert_eq!(chooser(w, &[Other], d, 0), Chooser::Owner);
        assert_eq!(chooser(w, &[Input], d, 0), Chooser::Cancel);
        assert_eq!(chooser(w, &[Upload], d, 0), Chooser::Cancel);
        // no one could answer one in a headless tab
        assert_eq!(chooser(h, &[], d, 0), Chooser::Cancel);
        assert_eq!(chooser(h, &[Other], d, 0), Chooser::Cancel);
        // a clicked upload's that DevTools did not catch is never answered
        assert_eq!(chooser(w, &[UploadClicked], d, 1), Chooser::Cancel);
        assert_eq!(chooser(h, &[UploadClicked], d, 1), Chooser::Cancel);
    }

    #[test]
    fn a_chooser_devtools_caught_goes_to_the_one_clicked_upload() {
        let dt = |multiple| Opened::DevTools { multiple };
        let h = Some(Mode::Headless);
        let w = Some(Mode::Windowed);
        assert_eq!(chooser(h, &[UploadClicked], dt(false), 1), Chooser::Answer);
        assert_eq!(
            chooser(w, &[UploadClicked, Other], dt(true), 3),
            Chooser::Answer
        );
        assert_eq!(
            chooser(h, &[UploadClicked], dt(false), 2),
            Chooser::Refuse(ONE_FILE)
        );
        // a click or script in flight beside it: whose it is cannot be told
        assert_eq!(
            chooser(h, &[UploadClicked, Input], dt(false), 1),
            Chooser::Cancel
        );
        // an upload still locating its element did not open it
        assert_eq!(chooser(h, &[Upload, Input], dt(false), 1), Chooser::Cancel);
        assert_eq!(chooser(h, &[Upload], dt(false), 1), Chooser::Drop);
        assert_eq!(chooser(None, &[], dt(false), 1), Chooser::Drop);
    }

    #[test]
    fn the_target_is_read_and_files_fit_it() {
        let read = json!({"result": {"value":
            {"file": true, "multiple": false, "folder": false, "origin": "http://a.test"}}});
        let t = parse_target(&read).unwrap();
        assert_eq!(
            t,
            Target {
                file: true,
                multiple: false,
                folder: false,
                origin: "http://a.test".into()
            }
        );
        assert_eq!(fits(&t, 1), Ok(()));
        assert_eq!(fits(&t, 2), Err(ONE_FILE));
        let many = Target {
            multiple: true,
            ..t.clone()
        };
        assert_eq!(fits(&many, 2), Ok(()));
        let folder = Target {
            folder: true,
            ..t.clone()
        };
        assert!(fits(&folder, 1).unwrap_err().contains("folder"));
        assert_eq!(parse_target(&json!({"result": {}})), None);
        assert_eq!(
            parse_target(&json!({"result": {"value": {"file": false}}})).map(|t| t.file),
            Some(false)
        );
    }
}
