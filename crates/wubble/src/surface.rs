//! The page's own prompts, drawn by the shell: JS dialogs ([`Dialog`]) and
//! the find bar ([`FindBar`]), each one enum with a pure transition, plus
//! [`route`], which says which surface a [`PageEvent`] is for (the
//! permission prompt and sign-in sheet are in `prompt.rs`, the error panel
//! in `page_error.rs`, the file chooser in `chooser.rs`). The host
//! keeps one [`Surfaces`] per tab and draws only the visible tab's, so a
//! dialog is tab-modal: switching tabs leaves it waiting, and it shows again
//! on return.
//!
//! Dialog wording follows Chromium's `components/javascript_dialogs`
//! strings (`IDS_JAVASCRIPT_MESSAGEBOX_TITLE` "%s says",
//! `IDS_BEFOREUNLOAD_MESSAGEBOX_*`; BSD-3-Clause). Find-bar behaviour
//! (Enter next, Shift+Enter previous, Esc closes, an "n of m" counter, Mod+G
//! reopening the last search) follows Chromium `find_bar_view.cc` and
//! Firefox `findbar.js` (MPL-2.0); behaviour only, no code.

use wubble_engine::page::{JsDialogKind, PageEvent, Rect, RequestId};

use crate::chooser::{self, DialogSpec};
use crate::menu::{self, Menu, MenuEvent};
use crate::page_error::{self, ErrorEvent, PageError};
use crate::prompt::{
    self, Auth, AuthEvent, AuthRequest, PermissionEvent, PermissionPrompt, PermissionRequest,
};

// ---------------------------------------------------------------- dialogs

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenDialog {
    pub id: RequestId,
    pub kind: JsDialogKind,
    pub origin: String,
    pub message: String,
    /// A prompt's input as the page proposed it.
    pub default_text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Dialog {
    #[default]
    None,
    Open(OpenDialog),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogEvent {
    Requested(OpenDialog),
    /// OK / Leave, with a prompt's text.
    Accept(String),
    /// Cancel / Stay, or Escape.
    Cancel,
    /// The page dropped its dialog (it navigated); CEF answered it.
    Reset,
    /// CEF gave up on these requests; welding answered them.
    Dismissed(Vec<RequestId>),
    /// The view closed; welding answered every held request.
    ViewGone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogAnswer {
    pub id: RequestId,
    pub accept: bool,
    pub text: String,
}

/// At most one answer per step, and exactly one per request over its life
/// unless the engine answered it itself (reset, dismissed, view gone).
pub fn dialog(state: Dialog, event: DialogEvent) -> (Dialog, Option<DialogAnswer>) {
    use DialogEvent::*;
    let answer = |d: &OpenDialog, accept: bool, text: String| DialogAnswer {
        id: d.id,
        accept,
        text,
    };
    match (state, event) {
        // A page can only wait on one dialog; a newer one replaces the old,
        // which gets its default.
        (Dialog::Open(old), Requested(new)) => {
            let a = answer(&old, old.kind.default_accept(), String::new());
            (Dialog::Open(new), Some(a))
        }
        (Dialog::None, Requested(new)) => (Dialog::Open(new), None),
        (Dialog::Open(d), Accept(text)) => {
            let text = if d.kind == JsDialogKind::Prompt {
                text
            } else {
                String::new()
            };
            (Dialog::None, Some(answer(&d, true, text)))
        }
        (Dialog::Open(d), Cancel) => (Dialog::None, Some(answer(&d, false, String::new()))),
        (Dialog::Open(d), Dismissed(ids)) if !ids.contains(&d.id) => (Dialog::Open(d), None),
        (_, Reset | Dismissed(_) | ViewGone) => (Dialog::None, None),
        (Dialog::None, Accept(_) | Cancel) => (Dialog::None, None),
    }
}

/// What the sheet says for one dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogText {
    pub title: String,
    pub message: String,
    pub accept: &'static str,
    /// Empty: no cancel button (an alert).
    pub cancel: &'static str,
    pub prompt: bool,
}

pub fn dialog_text(d: &OpenDialog) -> DialogText {
    let site = wubble_core::url::address_parts(&d.origin).0;
    let says = if site.is_empty() || site.ends_with(':') {
        "This page says".to_string()
    } else {
        format!("{site} says")
    };
    match d.kind {
        JsDialogKind::Alert => DialogText {
            title: says,
            message: d.message.clone(),
            accept: "OK",
            cancel: "",
            prompt: false,
        },
        JsDialogKind::Confirm | JsDialogKind::Prompt => DialogText {
            title: says,
            message: d.message.clone(),
            accept: "OK",
            cancel: "Cancel",
            prompt: d.kind == JsDialogKind::Prompt,
        },
        JsDialogKind::BeforeUnload { is_reload } => DialogText {
            title: if is_reload {
                "Reload site?"
            } else {
                "Leave site?"
            }
            .into(),
            message: "Changes you made may not be saved.".into(),
            accept: if is_reload { "Reload" } else { "Leave" },
            cancel: "Cancel",
            prompt: false,
        },
    }
}

// --------------------------------------------------------------- find bar

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindBar {
    /// `last`: the query of the last search, for Mod+G with the bar closed.
    Closed { last: String },
    Open {
        query: String,
        matches: u32,
        /// 1-based; 0 when nothing is highlighted.
        active: u32,
    },
}

impl Default for FindBar {
    fn default() -> Self {
        Self::Closed {
            last: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindEvent {
    /// Mod+F: open, or select the query of the open bar.
    Open,
    Query(String),
    Next,
    Prev,
    Close,
    Result {
        matches: u32,
        active: u32,
    },
    ViewGone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindEffect {
    /// A new search (or the next match of the same one).
    Find(String),
    Next,
    Prev,
    Stop,
    /// Put the keyboard in the bar's field, query selected.
    Focus,
}

pub fn find(state: FindBar, event: FindEvent) -> (FindBar, Vec<FindEffect>) {
    use FindEvent::*;
    let open = |query: String| FindBar::Open {
        query,
        matches: 0,
        active: 0,
    };
    match (state, event) {
        (FindBar::Closed { last }, Open) => {
            let effects = if last.is_empty() {
                vec![FindEffect::Focus]
            } else {
                vec![FindEffect::Focus, FindEffect::Find(last.clone())]
            };
            (open(last), effects)
        }
        // Mod+G with the bar closed reopens it on the last search, if any.
        (s @ FindBar::Closed { .. }, Next | Prev) if s.query().is_empty() => (s, vec![]),
        (FindBar::Closed { last }, Next | Prev) => {
            let (s, mut effects) = find(FindBar::Closed { last }, Open);
            effects.retain(|e| *e != FindEffect::Focus);
            (s, effects)
        }
        (s @ FindBar::Open { .. }, Open) => (s, vec![FindEffect::Focus]),
        (FindBar::Open { .. }, Query(q)) if q.is_empty() => (open(q), vec![FindEffect::Stop]),
        (FindBar::Open { .. }, Query(q)) => (open(q.clone()), vec![FindEffect::Find(q)]),
        (s @ FindBar::Open { .. }, Next | Prev) if s.query().is_empty() => (s, vec![]),
        (s @ FindBar::Open { .. }, Next) => (s, vec![FindEffect::Next]),
        (s @ FindBar::Open { .. }, Prev) => (s, vec![FindEffect::Prev]),
        (FindBar::Open { query, .. }, Result { matches, active }) => (
            FindBar::Open {
                query,
                matches,
                active,
            },
            vec![],
        ),
        (FindBar::Open { query, .. }, Close) => {
            (FindBar::Closed { last: query }, vec![FindEffect::Stop])
        }
        (FindBar::Open { query, .. }, ViewGone) => (FindBar::Closed { last: query }, vec![]),
        (s @ FindBar::Closed { .. }, Query(_) | Close | Result { .. } | ViewGone) => (s, vec![]),
    }
}

impl FindBar {
    pub fn query(&self) -> &str {
        match self {
            Self::Closed { last } => last,
            Self::Open { query, .. } => query,
        }
    }

    /// "n of m"; empty before anything was searched.
    pub fn counter(&self) -> String {
        match self {
            Self::Open {
                query,
                matches,
                active,
            } if !query.is_empty() => format!("{active} of {matches}"),
            _ => String::new(),
        }
    }
}

// ----------------------------------------------------------------- a tab

/// Everything the page has open in the shell, for one tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Surfaces {
    pub dialog: Dialog,
    pub menu: Menu,
    pub find: FindBar,
    /// The popup widget's rectangle, physical pixels relative to the view.
    pub popup: Option<Rect>,
    pub permission: PermissionPrompt,
    pub auth: Auth,
    pub error: PageError,
}

impl Surfaces {
    /// The tab's view closed (tab closed, slept or discarded, or the page
    /// closed itself): welding has answered every held request, so every
    /// surface closes without answering again.
    pub fn view_gone(self) -> Surfaces {
        let (dialog, answer) = dialog(self.dialog, DialogEvent::ViewGone);
        let (menu, menu_fx) = menu::transition(self.menu, MenuEvent::ViewGone);
        let (find, find_fx) = find(self.find, FindEvent::ViewGone);
        let (permission, granted) = prompt::permission(
            self.permission,
            PermissionEvent::ViewGone,
            &Default::default(),
        );
        let (auth, auth_answer) = prompt::auth(self.auth, AuthEvent::ViewGone);
        let (error, error_fx) = page_error::transition(self.error, ErrorEvent::ViewGone);
        debug_assert!(answer.is_none() && menu_fx.is_empty() && find_fx.is_empty());
        debug_assert!(granted.is_empty() && auth_answer.is_none() && error_fx.is_empty());
        Surfaces {
            dialog,
            menu,
            find,
            popup: None,
            permission,
            auth,
            error,
        }
    }
}

/// Where a page event goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Routed {
    /// What a drop at the pointer would do, during a drag over the page.
    DragCursor(wubble_engine::page::DragOps),
    /// The page started a drag (`page_drag`).
    Drag {
        data: wubble_engine::page::DragData,
        allowed: wubble_engine::page::DragOps,
    },
    Dialog(DialogEvent),
    /// A menu event; a request still needs the view's navigation flags.
    Menu(MenuEvent),
    Find(FindEvent),
    Popup(Option<Rect>),
    Fullscreen(bool),
    Muted(bool),
    Audible(bool),
    /// The main frame committed a new document.
    Committed,
    /// The page answered a freeze or thaw (`View::set_frozen`).
    Frozen {
        frozen: bool,
        ok: bool,
    },
    /// The page's CDP target id, for the docked DevTools frontend.
    TargetId(Result<String, ()>),
    /// IME composition bounds, physical pixels relative to the view.
    ImeBounds(Rect),
    /// CEF gave up on these requests: every surface showing one closes.
    Dismissed(Vec<RequestId>),
    Permission(PermissionEvent),
    Auth(AuthEvent),
    /// Ask the OS for files, then answer `id`.
    Chooser {
        id: RequestId,
        spec: DialogSpec,
    },
    /// A certificate error waiting for `id`'s answer.
    Cert {
        id: RequestId,
        url: String,
        code: i32,
    },
    Error(ErrorEvent),
    /// A download request, or news of one (`host/downloads.rs`).
    Download(PageEvent),
    /// A console line, for the debug log.
    Console(String),
    /// The page's icon, downloaded (`host/surfaces.rs`, `show_favicon`).
    Favicon(wubble_engine::favicon::FaviconImage),
    /// Autofill's message from one of the page's frames (`host/autofill.rs`).
    Frame {
        frame: wubble_engine::page::FrameInfo,
        text: wubble_core::secret::Secret,
    },
    /// A busy probe's report from one of the page's frames (`host/busy.rs`).
    Probe {
        frame: wubble_engine::page::FrameInfo,
        report: String,
    },
    /// A page saved as PDF, or not: the toast says which.
    Pdf {
        path: std::path::PathBuf,
        text: String,
        ok: bool,
    },
    /// A frame's copy or paste, for the system clipboard (`clipboard.rs`).
    Clipboard {
        frame: String,
        request: wubble_engine::page::ClipboardRequest,
    },
    /// A Ctrl+wheel the page left alone: one zoom step, in or out.
    WheelZoom {
        zoom_in: bool,
    },
    /// The hovered link's URL for the status bubble; empty: none.
    Status(String),
    /// A `title` tooltip under the pointer; empty: none.
    Tooltip(String),
    /// A link to a scheme another app handles (`wubble_core::external`).
    External {
        url: String,
        gesture: Option<bool>,
    },
    /// Copy Image's pixels, or why there are none.
    Image(Result<wubble_engine::favicon::Rgba, String>),
    Ignore,
}

pub fn route(event: PageEvent) -> Routed {
    match event {
        PageEvent::PopupShown(r) | PageEvent::PopupMoved(r) => Routed::Popup(Some(r)),
        PageEvent::PopupHidden => Routed::Popup(None),
        PageEvent::DialogRequested {
            id,
            kind,
            origin,
            message,
            default_text,
        } => Routed::Dialog(DialogEvent::Requested(OpenDialog {
            id,
            kind,
            origin,
            message,
            default_text,
        })),
        PageEvent::DialogsReset => Routed::Dialog(DialogEvent::Reset),
        PageEvent::RequestsDismissed(ids) => Routed::Dismissed(ids),
        PageEvent::ContextMenuRequested { id, menu } => Routed::Menu(MenuEvent::Requested {
            id,
            menu,
            nav: menu::NavFlags::default(),
            extra: vec![],
        }),
        PageEvent::ContextMenuDismissed => Routed::Menu(MenuEvent::Reset),
        PageEvent::FindResult {
            matches, active, ..
        } => Routed::Find(FindEvent::Result { matches, active }),
        // The host reads the zoom back when the user zooms; the view's own
        // reports (a load, an address change) are not the user's.
        PageEvent::ZoomChanged { .. } => Routed::Ignore,
        PageEvent::Clipboard { frame, request } => Routed::Clipboard { frame, request },
        PageEvent::WheelZoom { zoom_in } => Routed::WheelZoom { zoom_in },
        PageEvent::FullscreenChanged(on) => Routed::Fullscreen(on),
        PageEvent::MutedChanged(m) => Routed::Muted(m),
        PageEvent::Audible(a) => Routed::Audible(a),
        PageEvent::Frozen { frozen, ok } => Routed::Frozen { frozen, ok },
        PageEvent::TargetId(id) => Routed::TargetId(id),
        PageEvent::Committed => Routed::Committed,
        // taken before routing (`host/ax.rs`)
        PageEvent::AxTree(_) | PageEvent::AxLocation(_) => Routed::Ignore,
        PageEvent::ImeBounds { rect, .. } => Routed::ImeBounds(rect),
        PageEvent::PermissionRequested {
            id,
            origin,
            permissions,
        } => Routed::Permission(PermissionEvent::Requested(PermissionRequest {
            origin,
            ask: prompt::Ask::Engine {
                id,
                kinds: permissions,
            },
        })),
        PageEvent::AuthRequested {
            id,
            host,
            port,
            realm,
            proxy,
            ..
        } => Routed::Auth(AuthEvent::Requested(AuthRequest {
            id,
            host,
            port,
            realm,
            proxy,
        })),
        PageEvent::FileChooserRequested {
            id,
            mode,
            title,
            default_path,
            accept,
        } => Routed::Chooser {
            id,
            spec: chooser::dialog_spec(mode, &title, &default_path, &accept),
        },
        PageEvent::CertificateError { id, url, code } => Routed::Cert { id, url, code },
        PageEvent::FrameMessage { frame, text } => Routed::Frame { frame, text },
        PageEvent::Probe { frame, report } => Routed::Probe { frame, report },
        // The built-ins' answers: `host/builtins.rs` takes them before routing.
        PageEvent::CdpReply { .. } | PageEvent::EvalResult { .. } => Routed::Ignore,
        PageEvent::LoadFailed { url, code, .. } => {
            Routed::Error(ErrorEvent::LoadFailed { url, code })
        }
        e @ (PageEvent::DownloadRequested { .. }
        | PageEvent::DownloadStarted { .. }
        | PageEvent::DownloadProgress { .. }
        | PageEvent::DownloadPaused(_)
        | PageEvent::DownloadResumed(_)
        | PageEvent::DownloadFinished { .. }
        | PageEvent::DownloadFailed { .. }
        | PageEvent::DownloadCancelled(_)) => Routed::Download(e),
        PageEvent::Console {
            level,
            message,
            source,
            line,
        } => Routed::Console(format!("{level:?} {source}:{line}: {message}")),
        PageEvent::DragStarted { data, allowed, .. } => Routed::Drag { data, allowed },
        PageEvent::DragCursor(op) => Routed::DragCursor(op),
        PageEvent::FaviconImage(icon) => Routed::Favicon(icon),
        PageEvent::PdfPrinted { path, ok } => Routed::Pdf {
            text: pdf_notice(&path, ok),
            path,
            ok,
        },
        PageEvent::LinkStatus(text) => Routed::Status(text),
        PageEvent::Tooltip(text) => Routed::Tooltip(text),
        PageEvent::ExternalProtocol { url, gesture } => Routed::External { url, gesture },
        PageEvent::ImageDownloaded { image, .. } => Routed::Image(image),
    }
}

/// What the toast says when a page was printed to `path`, or could not be.
pub fn pdf_notice(path: &std::path::Path, ok: bool) -> String {
    match path.file_name().filter(|_| ok) {
        Some(name) => format!("Saved {}", name.to_string_lossy()),
        None => "Could not save the PDF".into(),
    }
}

/// `rect` moved inside `bounds` as far as it fits (a popup drawn past the
/// window's edge would be cut off); a rect larger than `bounds` keeps its
/// top-left corner at the bounds' top-left. Physical or logical alike.
pub fn clamp_into(rect: Rect, bounds: Rect) -> Rect {
    let axis = |pos: i32, len: u32, start: i32, size: u32| {
        let far = start + size as i32 - len as i32;
        pos.min(far).max(start)
    };
    Rect {
        x: axis(rect.x, rect.width, bounds.x, bounds.width),
        y: axis(rect.y, rect.height, bounds.y, bounds.height),
        ..rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: u32, kind: JsDialogKind) -> DialogEvent {
        DialogEvent::Requested(OpenDialog {
            id: RequestId(id),
            kind,
            origin: "https://a.test".into(),
            message: "hi".into(),
            default_text: "d".into(),
        })
    }

    fn answer(id: u32, accept: bool, text: &str) -> Option<DialogAnswer> {
        Some(DialogAnswer {
            id: RequestId(id),
            accept,
            text: text.into(),
        })
    }

    #[test]
    fn a_printed_page_is_reported_by_name() {
        let done = PageEvent::PdfPrinted {
            path: "/d/Page title.pdf".into(),
            ok: true,
        };
        assert_eq!(
            route(done),
            Routed::Pdf {
                path: "/d/Page title.pdf".into(),
                text: "Saved Page title.pdf".into(),
                ok: true,
            }
        );
        let failed = PageEvent::PdfPrinted {
            path: "/d/x.pdf".into(),
            ok: false,
        };
        assert_eq!(
            route(failed),
            Routed::Pdf {
                path: "/d/x.pdf".into(),
                text: "Could not save the PDF".into(),
                ok: false,
            }
        );
    }

    #[test]
    fn accept_and_cancel_answer_once() {
        let (s, a) = dialog(Dialog::None, req(1, JsDialogKind::Prompt));
        assert_eq!(a, None);
        let (s, a) = dialog(s, DialogEvent::Accept("typed".into()));
        assert_eq!((s.clone(), a), (Dialog::None, answer(1, true, "typed")));
        assert_eq!(dialog(s, DialogEvent::Accept("again".into())).1, None);

        let (s, _) = dialog(Dialog::None, req(2, JsDialogKind::Confirm));
        let (s, a) = dialog(s, DialogEvent::Cancel);
        assert_eq!((s.clone(), a), (Dialog::None, answer(2, false, "")));
        assert_eq!(dialog(s, DialogEvent::Cancel).1, None);
    }

    #[test]
    fn only_a_prompt_answers_with_text() {
        let (s, _) = dialog(Dialog::None, req(1, JsDialogKind::Confirm));
        assert_eq!(
            dialog(s, DialogEvent::Accept("x".into())).1,
            answer(1, true, "")
        );
    }

    #[test]
    fn a_new_request_answers_the_old_with_its_default() {
        let (s, _) = dialog(Dialog::None, req(1, JsDialogKind::Confirm));
        let (s, a) = dialog(s, req(2, JsDialogKind::BeforeUnload { is_reload: false }));
        assert_eq!(a, answer(1, false, ""));
        // beforeunload's default is leave.
        let (_, a) = dialog(s, req(3, JsDialogKind::Alert));
        assert_eq!(a, answer(2, true, ""));
    }

    #[test]
    fn the_engine_answering_closes_without_an_answer() {
        for event in [
            DialogEvent::Reset,
            DialogEvent::ViewGone,
            DialogEvent::Dismissed(vec![RequestId(1)]),
        ] {
            let (s, _) = dialog(Dialog::None, req(1, JsDialogKind::Alert));
            assert_eq!(dialog(s, event.clone()), (Dialog::None, None), "{event:?}");
        }
        // Another request's dismissal leaves this one open.
        let (s, _) = dialog(Dialog::None, req(1, JsDialogKind::Alert));
        let (s, a) = dialog(s.clone(), DialogEvent::Dismissed(vec![RequestId(9)]));
        assert!(matches!(s, Dialog::Open(_)) && a.is_none());
    }

    /// Every ordering of events answers each id at most once, and never
    /// one the engine answered.
    #[test]
    fn sequences_answer_each_id_at_most_once() {
        use DialogEvent::*;
        let events = [
            req(1, JsDialogKind::Alert),
            Accept(String::new()),
            Cancel,
            Reset,
            Dismissed(vec![RequestId(1), RequestId(2)]),
            ViewGone,
            req(2, JsDialogKind::Prompt),
            req(3, JsDialogKind::Confirm),
        ];
        // All sequences of length 4 over the events; the engine never
        // reuses an id, so sequences requesting one twice are skipped.
        let n = events.len();
        'sequence: for code in 0..n.pow(4) {
            let mut s = Dialog::None;
            let mut answered = vec![];
            let mut engine_answered = vec![];
            let mut requested = vec![];
            let mut c = code;
            for _ in 0..4 {
                let e = events[c % n].clone();
                c /= n;
                if let Requested(d) = &e {
                    if requested.contains(&d.id) {
                        continue 'sequence;
                    }
                    requested.push(d.id);
                }
                if let Dialog::Open(d) = &s
                    && match &e {
                        Reset | ViewGone => true,
                        Dismissed(ids) => ids.contains(&d.id),
                        _ => false,
                    }
                {
                    engine_answered.push(d.id);
                }
                let (next, a) = dialog(s, e);
                s = next;
                if let Some(a) = a {
                    assert!(!answered.contains(&a.id), "{code}: {:?} twice", a.id);
                    assert!(!engine_answered.contains(&a.id), "{code}: after the engine");
                    answered.push(a.id);
                }
            }
        }
    }

    #[test]
    fn dialog_text_by_kind() {
        let d = |kind| OpenDialog {
            id: RequestId(1),
            kind,
            origin: "https://a.test/".into(),
            message: "m".into(),
            default_text: String::new(),
        };
        let alert = dialog_text(&d(JsDialogKind::Alert));
        assert_eq!(
            (
                alert.title.as_str(),
                alert.accept,
                alert.cancel,
                alert.prompt
            ),
            ("a.test says", "OK", "", false)
        );
        assert!(dialog_text(&d(JsDialogKind::Prompt)).prompt);
        assert_eq!(dialog_text(&d(JsDialogKind::Confirm)).cancel, "Cancel");
        let leave = dialog_text(&d(JsDialogKind::BeforeUnload { is_reload: false }));
        assert_eq!(
            (leave.title.as_str(), leave.accept),
            ("Leave site?", "Leave")
        );
        let data = OpenDialog {
            origin: "data:text/html,x".into(),
            ..d(JsDialogKind::Alert)
        };
        assert_eq!(dialog_text(&data).title, "This page says");
    }

    #[test]
    fn find_searches_steps_and_counts() {
        let (s, fx) = find(FindBar::default(), FindEvent::Open);
        assert_eq!(fx, [FindEffect::Focus]);
        assert_eq!(s.counter(), "");
        // Nothing to step through yet.
        assert_eq!(find(s.clone(), FindEvent::Next).1, []);
        let (s, fx) = find(s, FindEvent::Query("needle".into()));
        assert_eq!(fx, [FindEffect::Find("needle".into())]);
        assert_eq!(s.counter(), "0 of 0");
        let (s, _) = find(
            s,
            FindEvent::Result {
                matches: 2,
                active: 1,
            },
        );
        assert_eq!(s.counter(), "1 of 2");
        assert_eq!(find(s.clone(), FindEvent::Next).1, [FindEffect::Next]);
        assert_eq!(find(s.clone(), FindEvent::Prev).1, [FindEffect::Prev]);
        assert_eq!(find(s.clone(), FindEvent::Open).1, [FindEffect::Focus]);
        let (s2, fx) = find(s.clone(), FindEvent::Query(String::new()));
        assert_eq!((s2.counter(), fx), (String::new(), vec![FindEffect::Stop]));
        let (s, fx) = find(s, FindEvent::Close);
        assert_eq!(fx, [FindEffect::Stop]);
        assert_eq!(
            s,
            FindBar::Closed {
                last: "needle".into()
            }
        );
        // A late result does not reopen it.
        assert_eq!(
            find(
                s.clone(),
                FindEvent::Result {
                    matches: 1,
                    active: 1
                }
            )
            .0,
            s
        );
    }

    #[test]
    fn find_next_reopens_the_last_search() {
        let closed = FindBar::Closed { last: "x".into() };
        let (s, fx) = find(closed.clone(), FindEvent::Next);
        assert_eq!(fx, [FindEffect::Find("x".into())]);
        assert!(matches!(s, FindBar::Open { .. }));
        let (_, fx) = find(closed, FindEvent::Open);
        assert_eq!(fx, [FindEffect::Focus, FindEffect::Find("x".into())]);
        // Nothing searched yet: Mod+G does nothing.
        assert_eq!(
            find(FindBar::default(), FindEvent::Prev),
            (FindBar::default(), vec![])
        );
    }

    #[test]
    fn find_view_gone_closes_without_stopping() {
        let (s, _) = find(FindBar::default(), FindEvent::Open);
        let (s, _) = find(s, FindEvent::Query("q".into()));
        assert_eq!(
            find(s, FindEvent::ViewGone),
            (FindBar::Closed { last: "q".into() }, vec![])
        );
    }

    #[test]
    fn routes() {
        let r = Rect {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        };
        assert_eq!(route(PageEvent::PopupShown(r)), Routed::Popup(Some(r)));
        assert_eq!(route(PageEvent::PopupMoved(r)), Routed::Popup(Some(r)));
        assert_eq!(route(PageEvent::PopupHidden), Routed::Popup(None));
        assert_eq!(
            route(PageEvent::DialogsReset),
            Routed::Dialog(DialogEvent::Reset)
        );
        assert_eq!(
            route(PageEvent::RequestsDismissed(vec![RequestId(3)])),
            Routed::Dismissed(vec![RequestId(3)])
        );
        assert_eq!(
            route(PageEvent::ContextMenuDismissed),
            Routed::Menu(MenuEvent::Reset)
        );
        assert_eq!(
            route(PageEvent::FindResult {
                matches: 2,
                active: 1,
                done: true
            }),
            Routed::Find(FindEvent::Result {
                matches: 2,
                active: 1
            })
        );
        assert_eq!(
            route(PageEvent::ZoomChanged { percent: 110 }),
            Routed::Ignore
        );
        assert_eq!(
            route(PageEvent::FullscreenChanged(true)),
            Routed::Fullscreen(true)
        );
        assert_eq!(route(PageEvent::MutedChanged(true)), Routed::Muted(true));
        assert_eq!(route(PageEvent::Audible(true)), Routed::Audible(true));
        assert_eq!(
            route(PageEvent::ImeBounds {
                rect: r,
                selection: (0, 1)
            }),
            Routed::ImeBounds(r)
        );
        let cert = PageEvent::CertificateError {
            id: RequestId(1),
            url: "https://a.test/".into(),
            code: -200,
        };
        assert_eq!(
            route(cert),
            Routed::Cert {
                id: RequestId(1),
                url: "https://a.test/".into(),
                code: -200
            }
        );
        assert_eq!(
            route(PageEvent::LoadFailed {
                url: "https://a.test/".into(),
                code: -105,
                text: "x".into()
            }),
            Routed::Error(ErrorEvent::LoadFailed {
                url: "https://a.test/".into(),
                code: -105
            })
        );
        assert_eq!(
            route(PageEvent::Console {
                level: wubble_engine::page::LogLevel::Warning,
                message: "m".into(),
                source: "s.js".into(),
                line: 3
            }),
            Routed::Console("Warning s.js:3: m".into())
        );
        let paused = PageEvent::DownloadPaused(wubble_engine::page::DownloadId(2));
        assert_eq!(route(paused.clone()), Routed::Download(paused));
        assert!(matches!(
            route(PageEvent::ContextMenuRequested {
                id: RequestId(4),
                menu: Box::default()
            }),
            Routed::Menu(MenuEvent::Requested {
                id: RequestId(4),
                ..
            })
        ));
        assert!(matches!(
            route(PageEvent::DialogRequested {
                id: RequestId(5),
                kind: JsDialogKind::Alert,
                origin: String::new(),
                message: String::new(),
                default_text: String::new()
            }),
            Routed::Dialog(DialogEvent::Requested(OpenDialog {
                id: RequestId(5),
                ..
            }))
        ));
    }

    /// Every request reaches something that answers it, so none waits.
    #[test]
    fn every_request_has_a_surface() {
        let requests = [
            PageEvent::PermissionRequested {
                id: RequestId(1),
                origin: String::new(),
                permissions: vec![],
            },
            PageEvent::AuthRequested {
                id: RequestId(1),
                url: String::new(),
                host: String::new(),
                port: 0,
                realm: String::new(),
                scheme: String::new(),
                proxy: false,
            },
            PageEvent::DownloadRequested {
                id: RequestId(1),
                download: wubble_engine::page::DownloadId(1),
                url: String::new(),
                original_url: String::new(),
                suggested_name: String::new(),
                total: None,
            },
            PageEvent::FileChooserRequested {
                id: RequestId(1),
                mode: wubble_engine::page::FileChooserMode::Open,
                title: String::new(),
                default_path: Default::default(),
                accept: vec![],
            },
            PageEvent::CertificateError {
                id: RequestId(1),
                url: String::new(),
                code: -202,
            },
        ];
        for e in requests {
            assert!(e.request().is_some());
            let routed = route(e.clone());
            assert!(
                matches!(
                    routed,
                    Routed::Permission(PermissionEvent::Requested(_))
                        | Routed::Auth(AuthEvent::Requested(_))
                        | Routed::Download(PageEvent::DownloadRequested { .. })
                        | Routed::Chooser { .. }
                        | Routed::Cert { .. }
                ),
                "{e:?} -> {routed:?}"
            );
        }
    }

    /// A tab put to sleep or closed with every surface open answers
    /// nothing more: the engine answered at view close.
    #[test]
    fn a_gone_view_closes_every_surface_without_answers() {
        let (dialog_open, _) = dialog(Dialog::None, req(1, JsDialogKind::Prompt));
        let (menu_open, _) = menu::transition(
            Menu::Closed,
            MenuEvent::Requested {
                id: RequestId(2),
                menu: Box::default(),
                nav: menu::NavFlags::default(),
                extra: vec![],
            },
        );
        let (find_open, _) = find(FindBar::default(), FindEvent::Open);
        let (find_open, _) = find(find_open, FindEvent::Query("q".into()));
        let s = Surfaces {
            dialog: dialog_open,
            menu: menu_open,
            find: find_open,
            popup: Some(Rect::default()),
            permission: PermissionPrompt::Open {
                shown: PermissionRequest {
                    origin: "https://a.test".into(),
                    ask: prompt::Ask::Engine {
                        id: RequestId(3),
                        kinds: vec![wubble_engine::page::PermissionKind::Geolocation],
                    },
                },
                queued: Default::default(),
            },
            auth: Auth::Open(AuthRequest {
                id: RequestId(4),
                host: "a.test".into(),
                port: 443,
                realm: String::new(),
                proxy: false,
            }),
            error: PageError::Crashed,
        };
        assert!(matches!(s.menu, Menu::Open { .. }));
        let gone = s.view_gone();
        assert_eq!(
            gone,
            Surfaces {
                find: FindBar::Closed { last: "q".into() },
                ..Surfaces::default()
            }
        );
    }

    #[test]
    fn clamp_moves_a_rect_inside() {
        let bounds = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 50,
        };
        let r = |x, y, width, height| Rect {
            x,
            y,
            width,
            height,
        };
        assert_eq!(clamp_into(r(10, 10, 20, 20), bounds), r(10, 10, 20, 20));
        assert_eq!(clamp_into(r(90, 40, 20, 20), bounds), r(80, 30, 20, 20));
        assert_eq!(clamp_into(r(-5, -9, 20, 20), bounds), r(0, 0, 20, 20));
        assert_eq!(clamp_into(r(30, 30, 200, 80), bounds), r(0, 0, 200, 80));
    }
}
