//! Everything a page asks the shell for or tells it beyond navigation:
//! popup widgets, JS dialogs, context menus, downloads, permissions, file
//! choosers, HTTP auth, certificate errors, find, zoom, fullscreen, audio,
//! console messages, load errors, IME and drags.
//!
//! Pure: plain data, no CEF types. [`crate::view::View`] translates welding's
//! callbacks into [`PageEvent`]s (some through the state machines in
//! `popup`, `download`, `find`, `media`, `zoom` and `drag`) and the host
//! takes them with [`crate::app::Engine::take_page_events`].
//!
//! A request carries a [`RequestId`]; the host answers it through the
//! matching `View::answer_*` method. welding holds the CEF callback under
//! that id and gives every request a default answer (cancel, deny, decline)
//! when the view closes, when the host never answers before then, or when a
//! page asks more than 64 times at once, so an unanswered request never
//! hangs the page. A stale id (already answered, dismissed by the page, view
//! closing) makes `answer_*` return false.
//!
//! These are a separate stream from [`crate::nav::WebEvent`] only because
//! the host matches `WebEvent` exhaustively; folding them in is one line in
//! the host.

use std::collections::VecDeque;
use std::path::PathBuf;

/// The token the host quotes back to answer one request. Unique per view
/// for the view's life; never 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(pub u32);

/// One download, for as long as the engine runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DownloadId(pub u32);

/// Physical pixels relative to the view's top-left. A popup widget may
/// extend past the view's edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl From<welding::PopupRect> for Rect {
    fn from(r: welding::PopupRect) -> Self {
        Self {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

/// Which JS dialog a page opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
    /// "Leave site?" before a navigation (or a reload). Never shown when the
    /// shell closes a tab: the view closes without asking the page.
    BeforeUnload {
        is_reload: bool,
    },
}

impl From<welding::JsDialogKind> for DialogKind {
    fn from(kind: welding::JsDialogKind) -> Self {
        match kind {
            welding::JsDialogKind::Alert => Self::Alert,
            welding::JsDialogKind::Confirm => Self::Confirm,
            welding::JsDialogKind::Prompt => Self::Prompt,
            welding::JsDialogKind::BeforeUnload { is_reload } => Self::BeforeUnload { is_reload },
        }
    }
}

impl From<DialogKind> for welding::JsDialogKind {
    fn from(kind: DialogKind) -> Self {
        match kind {
            DialogKind::Alert => Self::Alert,
            DialogKind::Confirm => Self::Confirm,
            DialogKind::Prompt => Self::Prompt,
            DialogKind::BeforeUnload { is_reload } => Self::BeforeUnload { is_reload },
        }
    }
}

impl DialogKind {
    /// The answer a dismissed dialog gets (welding's one table): cancel,
    /// except `beforeunload`, which leaves the page.
    pub fn default_accept(self) -> bool {
        welding::JsDialogKind::from(self).default_accept()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChooserMode {
    Open,
    OpenMultiple,
    OpenFolder,
    Save,
}

impl From<welding::FileChooserMode> for FileChooserMode {
    fn from(mode: welding::FileChooserMode) -> Self {
        match mode {
            welding::FileChooserMode::Open => Self::Open,
            welding::FileChooserMode::OpenMultiple => Self::OpenMultiple,
            welding::FileChooserMode::OpenFolder => Self::OpenFolder,
            welding::FileChooserMode::Save => Self::Save,
        }
    }
}

/// One `accept` entry: a MIME type (`image/*`) or extension (`.png`), the
/// extensions it stands for, and a description; the last two may be empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AcceptFilter {
    pub filter: String,
    pub extensions: Vec<String>,
    pub description: String,
}

impl From<welding::AcceptFilter> for AcceptFilter {
    fn from(f: welding::AcceptFilter) -> Self {
        Self {
            filter: f.filter,
            extensions: f.extensions,
            description: f.description,
        }
    }
}

/// What a page asked for. Anything welding does not name is `Other` with
/// CEF's raw bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
    Camera,
    Microphone,
    Geolocation,
    Notifications,
    Clipboard,
    MidiSysex,
    PointerLock,
    KeyboardLock,
    IdleDetection,
    LocalFonts,
    StorageAccess,
    ProtectedMediaIdentifier,
    DesktopAudioCapture,
    DesktopVideoCapture,
    Other(u32),
}

/// The engine's names and welding's, one row each.
const PERMISSIONS: [(Permission, welding::PermissionKind); 14] = {
    use welding::PermissionKind as K;
    [
        (Permission::Camera, K::CameraStream),
        (Permission::Microphone, K::MicStream),
        (Permission::Geolocation, K::Geolocation),
        (Permission::Notifications, K::Notifications),
        (Permission::Clipboard, K::Clipboard),
        (Permission::MidiSysex, K::MidiSysex),
        (Permission::PointerLock, K::PointerLock),
        (Permission::KeyboardLock, K::KeyboardLock),
        (Permission::IdleDetection, K::IdleDetection),
        (Permission::LocalFonts, K::LocalFonts),
        (Permission::StorageAccess, K::StorageAccess),
        (
            Permission::ProtectedMediaIdentifier,
            K::ProtectedMediaIdentifier,
        ),
        (Permission::DesktopAudioCapture, K::DesktopAudioCapture),
        (Permission::DesktopVideoCapture, K::DesktopVideoCapture),
    ]
};

impl From<welding::PermissionKind> for Permission {
    fn from(kind: welding::PermissionKind) -> Self {
        match kind {
            welding::PermissionKind::Other(bit) => Self::Other(bit),
            kind => PERMISSIONS
                .iter()
                .find(|(_, k)| *k == kind)
                .map_or(Self::Other(0), |(p, _)| *p),
        }
    }
}

impl From<Permission> for welding::PermissionKind {
    fn from(p: Permission) -> Self {
        match p {
            Permission::Other(bit) => Self::Other(bit),
            p => PERMISSIONS
                .iter()
                .find(|(e, _)| *e == p)
                .map_or(Self::Other(0), |(_, k)| *k),
        }
    }
}

/// A certificate error, by Chromium net error code (`net/base/net_error_list.h`,
/// as `cef_errorcode_t` in CEF 156). Codes the table does not name are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertError {
    CommonNameInvalid,
    DateInvalid,
    AuthorityInvalid,
    ContainsErrors,
    NoRevocationMechanism,
    UnableToCheckRevocation,
    Revoked,
    Invalid,
    WeakSignatureAlgorithm,
    NonUniqueName,
    WeakKey,
    NameConstraintViolation,
    ValidityTooLong,
    CertificateTransparencyRequired,
    KnownInterceptionBlocked,
    SelfSignedLocalNetwork,
    Other(i32),
}

/// Read off CEF 156's `cef_errorcode_t` (cef-dll-sys 156.6 bindings).
const CERT_ERRORS: [(i32, CertError); 16] = [
    (-200, CertError::CommonNameInvalid),
    (-201, CertError::DateInvalid),
    (-202, CertError::AuthorityInvalid),
    (-203, CertError::ContainsErrors),
    (-204, CertError::NoRevocationMechanism),
    (-205, CertError::UnableToCheckRevocation),
    (-206, CertError::Revoked),
    (-207, CertError::Invalid),
    (-208, CertError::WeakSignatureAlgorithm),
    (-210, CertError::NonUniqueName),
    (-211, CertError::WeakKey),
    (-212, CertError::NameConstraintViolation),
    (-213, CertError::ValidityTooLong),
    (-214, CertError::CertificateTransparencyRequired),
    (-217, CertError::KnownInterceptionBlocked),
    (-219, CertError::SelfSignedLocalNetwork),
];

impl From<i32> for CertError {
    fn from(code: i32) -> Self {
        CERT_ERRORS
            .iter()
            .find(|(c, _)| *c == code)
            .map_or(Self::Other(code), |(_, e)| *e)
    }
}

impl CertError {
    pub fn code(self) -> i32 {
        match self {
            Self::Other(code) => code,
            e => CERT_ERRORS
                .iter()
                .find(|(_, x)| *x == e)
                .map_or(0, |(c, _)| *c),
        }
    }
}

/// The media element a right-click landed on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaKind {
    #[default]
    None,
    Image,
    Video,
    Audio,
    Canvas,
    File,
    Plugin,
}

impl From<welding::ContextMenuMedia> for MediaKind {
    fn from(m: welding::ContextMenuMedia) -> Self {
        use welding::ContextMenuMedia as M;
        match m {
            M::None => Self::None,
            M::Image => Self::Image,
            M::Video => Self::Video,
            M::Audio => Self::Audio,
            M::Canvas => Self::Canvas,
            M::File => Self::File,
            M::Plugin => Self::Plugin,
        }
    }
}

/// CEF's `cef_context_menu_edit_state_flags_t`: which edit commands the
/// field under the cursor allows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditFlags(pub u32);

impl EditFlags {
    pub const CAN_UNDO: u32 = 1 << 0;
    pub const CAN_REDO: u32 = 1 << 1;
    pub const CAN_CUT: u32 = 1 << 2;
    pub const CAN_COPY: u32 = 1 << 3;
    pub const CAN_PASTE: u32 = 1 << 4;
    pub const CAN_DELETE: u32 = 1 << 5;
    pub const CAN_SELECT_ALL: u32 = 1 << 6;

    pub fn allows(self, flag: u32) -> bool {
        self.0 & flag == flag
    }
}

/// One entry of CEF's default menu. Pick by `command`; CEF runs the standard
/// commands (back, reload, copy, paste, spelling suggestions, ...) itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub command: i32,
    pub label: String,
    pub kind: MenuItemKind,
    pub enabled: bool,
    pub checked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItemKind {
    Command,
    Check,
    Radio,
    Separator,
    SubMenu(Vec<MenuItem>),
}

impl From<welding::MenuItem> for MenuItem {
    fn from(item: welding::MenuItem) -> Self {
        use welding::MenuItemKind as K;
        Self {
            command: item.command_id,
            label: item.label,
            kind: match item.kind {
                K::Command => MenuItemKind::Command,
                K::Check => MenuItemKind::Check,
                K::Radio => MenuItemKind::Radio,
                K::Separator => MenuItemKind::Separator,
                K::SubMenu(items) => {
                    MenuItemKind::SubMenu(items.into_iter().map(Into::into).collect())
                }
            },
            enabled: item.enabled,
            checked: item.checked,
        }
    }
}

/// What a right-click landed on, and CEF's menu for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextMenu {
    /// Physical pixels relative to the view.
    pub x: i32,
    pub y: i32,
    pub link_url: String,
    /// The media element's source (an image's URL, ...).
    pub media_url: String,
    pub media: MediaKind,
    pub has_image: bool,
    pub page_url: String,
    pub frame_url: String,
    pub selection_text: String,
    pub editable: bool,
    pub edit: EditFlags,
    pub misspelled_word: String,
    pub suggestions: Vec<String>,
    pub items: Vec<MenuItem>,
}

/// `cef_log_severity_t` of a console message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Verbose,
    Info,
    Warning,
    Error,
    Fatal,
}

impl LogLevel {
    /// CEF: DEFAULT 0, VERBOSE/DEBUG 1, INFO 2, WARNING 3, ERROR 4, FATAL 5.
    pub fn from_severity(severity: i32) -> Self {
        match severity {
            1 => Self::Verbose,
            3 => Self::Warning,
            4 => Self::Error,
            5 => Self::Fatal,
            _ => Self::Info,
        }
    }
}

/// A set of drag effects (CEF's `cef_drag_operations_mask_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DragOps(pub u32);

impl DragOps {
    pub const NONE: Self = Self(0);
    pub const COPY: Self = Self(1 << 0);
    pub const LINK: Self = Self(1 << 1);
    pub const MOVE: Self = Self(1 << 4);
}

/// What is being dragged, in either direction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DragData {
    pub files: Vec<PathBuf>,
    pub link_url: Option<String>,
    pub link_title: Option<String>,
    pub text: Option<String>,
    pub html: Option<String>,
    pub base_url: Option<String>,
}

impl From<welding::DragPayload> for DragData {
    fn from(p: welding::DragPayload) -> Self {
        Self {
            files: p.files.into_iter().map(|f| f.path).collect(),
            link_url: p.link_url,
            link_title: p.link_title,
            text: p.fragment_text,
            html: p.fragment_html,
            base_url: p.fragment_base_url,
        }
    }
}

impl From<DragData> for welding::DragPayload {
    fn from(d: DragData) -> Self {
        Self {
            files: d
                .files
                .into_iter()
                .map(|path| welding::DragFile {
                    path,
                    display_name: None,
                })
                .collect(),
            link_url: d.link_url,
            link_title: d.link_title,
            fragment_text: d.text,
            fragment_html: d.html,
            fragment_base_url: d.base_url,
        }
    }
}

/// Edit commands on the focused frame (the context menu's and macOS's
/// Edit menu; key chords reach the page as keys).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    PasteMatchStyle,
    Delete,
    SelectAll,
}

/// What the host learns about a page beyond navigation, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageEvent {
    /// A `<select>` dropdown, date picker or autofill list opened. Its frames
    /// come from `View::take_popup_frame`; draw them at `rect` as a layer
    /// above the view (it may extend outside the view).
    PopupShown(Rect),
    PopupMoved(Rect),
    PopupHidden,
    /// Answer with `View::answer_dialog`.
    DialogRequested {
        id: RequestId,
        kind: DialogKind,
        origin: String,
        message: String,
        /// A prompt's default input.
        default_text: String,
    },
    /// The page dropped the dialog it showed (it navigated): close it.
    DialogsReset,
    /// Answer with `View::answer_context_menu`. CEF's own menu never shows.
    ContextMenuRequested {
        id: RequestId,
        menu: Box<ContextMenu>,
    },
    /// CEF closed the menu itself (navigation): close it.
    ContextMenuDismissed,
    /// Answer with `View::answer_permission`. Nothing is granted unasked.
    PermissionRequested {
        id: RequestId,
        origin: String,
        permissions: Vec<Permission>,
    },
    /// Answer with `View::answer_file_chooser`.
    FileChooserRequested {
        id: RequestId,
        mode: FileChooserMode,
        title: String,
        default_path: PathBuf,
        accept: Vec<AcceptFilter>,
    },
    /// Answer with `View::answer_auth`. Carries no credentials.
    AuthRequested {
        id: RequestId,
        url: String,
        host: String,
        port: u16,
        realm: String,
        scheme: String,
        proxy: bool,
    },
    /// Answer with `View::answer_certificate`; the default is cancel.
    CertificateError {
        id: RequestId,
        url: String,
        error: CertError,
    },
    /// Answer with `View::answer_download_path`; no file dialog is shown.
    DownloadRequested {
        id: RequestId,
        download: DownloadId,
        url: String,
        suggested_name: String,
        total: Option<u64>,
    },
    DownloadStarted {
        download: DownloadId,
        url: String,
        suggested_name: String,
        path: PathBuf,
        total: Option<u64>,
    },
    DownloadProgress {
        download: DownloadId,
        received: u64,
        total: Option<u64>,
    },
    DownloadPaused(DownloadId),
    DownloadResumed(DownloadId),
    DownloadFinished {
        download: DownloadId,
        path: PathBuf,
    },
    DownloadFailed {
        download: DownloadId,
        error: String,
    },
    DownloadCancelled(DownloadId),
    /// `active` is 1-based, 0 when nothing is highlighted; `done` marks the
    /// last update of a search.
    FindResult {
        matches: u32,
        active: u32,
        done: bool,
    },
    ZoomChanged {
        percent: u32,
    },
    /// The page entered or left fullscreen. The host grows the view to the
    /// window (or screen) and maps Esc to `View::exit_fullscreen`.
    FullscreenChanged(bool),
    AudibleChanged(bool),
    MutedChanged(bool),
    Console {
        level: LogLevel,
        message: String,
        source: String,
        line: i32,
    },
    /// A main-frame load failed (Chromium net error `code`); the shell can
    /// show an error page for `url`. Navigations replaced by another one
    /// (ERR_ABORTED, -3) are not reported.
    LoadFailed {
        url: String,
        code: i32,
        text: String,
    },
    /// Where the text being composed sits (physical pixels), to place the
    /// IME candidate window. Only while a composition is active.
    ImeBounds {
        rect: Rect,
        selection: (u32, u32),
    },
    /// The page started dragging (only after `View::set_drag_source(true)`);
    /// run the drag and end it with `View::drag_source_ended`. Physical
    /// pixels.
    DragStarted {
        data: DragData,
        allowed: DragOps,
        x: i32,
        y: i32,
    },
}

impl PageEvent {
    /// The id of a request the host is expected to answer.
    pub fn request(&self) -> Option<RequestId> {
        match self {
            Self::DialogRequested { id, .. }
            | Self::ContextMenuRequested { id, .. }
            | Self::PermissionRequested { id, .. }
            | Self::FileChooserRequested { id, .. }
            | Self::AuthRequested { id, .. }
            | Self::CertificateError { id, .. }
            | Self::DownloadRequested { id, .. } => Some(*id),
            _ => None,
        }
    }

    /// Whether `next` may replace `self` at the end of the queue: only the
    /// latest of a burst matters.
    fn superseded_by(&self, next: &PageEvent) -> bool {
        match (self, next) {
            (
                Self::DownloadProgress { download: a, .. },
                Self::DownloadProgress { download: b, .. },
            ) => a == b,
            (Self::FindResult { .. }, Self::FindResult { .. })
            | (Self::ImeBounds { .. }, Self::ImeBounds { .. })
            | (Self::PopupMoved(_), Self::PopupMoved(_)) => true,
            _ => false,
        }
    }
}

/// At most this many events wait for the host before console messages, and
/// then other notices, are dropped. Requests are never dropped (welding caps
/// each kind per view).
pub const MAX_PAGE_EVENTS: usize = 1024;

/// The events waiting for the host, per view key `K`.
#[derive(Debug)]
pub struct PageQueue<K> {
    events: VecDeque<(K, PageEvent)>,
    dropped: u64,
}

impl<K> Default for PageQueue<K> {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            dropped: 0,
        }
    }
}

impl<K: PartialEq> PageQueue<K> {
    pub fn push(&mut self, key: K, event: PageEvent) {
        if let Some((k, last)) = self.events.back_mut()
            && *k == key
            && last.superseded_by(&event)
        {
            *last = event;
            return;
        }
        if self.events.len() >= MAX_PAGE_EVENTS && event.request().is_none() {
            let console = self
                .events
                .iter()
                .position(|(_, e)| matches!(e, PageEvent::Console { .. }));
            match console {
                Some(i) if !matches!(event, PageEvent::Console { .. }) => {
                    self.events.remove(i);
                }
                _ => {
                    self.dropped += 1;
                    return;
                }
            }
            self.dropped += 1;
        }
        self.events.push_back((key, event));
    }

    pub fn take(&mut self) -> Vec<(K, PageEvent)> {
        self.events.drain(..).collect()
    }

    /// Events lost to the cap so far.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console(n: i32) -> PageEvent {
        PageEvent::Console {
            level: LogLevel::Info,
            message: String::new(),
            source: String::new(),
            line: n,
        }
    }

    fn progress(download: u32, received: u64) -> PageEvent {
        PageEvent::DownloadProgress {
            download: DownloadId(download),
            received,
            total: None,
        }
    }

    #[test]
    fn permission_names_round_trip_through_welding() {
        for (p, k) in PERMISSIONS {
            assert_eq!(Permission::from(k), p);
            assert_eq!(welding::PermissionKind::from(p), k);
        }
        let odd = welding::PermissionKind::Other(1 << 30);
        assert_eq!(Permission::from(odd), Permission::Other(1 << 30));
        assert_eq!(
            welding::PermissionKind::from(Permission::Other(1 << 30)),
            odd
        );
    }

    #[test]
    fn cert_errors_decode_and_keep_unknown_codes() {
        assert_eq!(CertError::from(-202), CertError::AuthorityInvalid);
        assert_eq!(CertError::from(-201), CertError::DateInvalid);
        assert_eq!(CertError::from(-219), CertError::SelfSignedLocalNetwork);
        assert_eq!(CertError::from(-209), CertError::Other(-209));
        for (code, e) in CERT_ERRORS {
            assert_eq!(e.code(), code);
        }
        assert_eq!(CertError::Other(-216).code(), -216);
    }

    #[test]
    fn log_levels_follow_cef_severity() {
        let got: Vec<_> = (0..7).map(LogLevel::from_severity).collect();
        use LogLevel::*;
        assert_eq!(got, vec![Info, Verbose, Info, Warning, Error, Fatal, Info]);
    }

    #[test]
    fn dialog_defaults_come_from_welding() {
        assert!(!DialogKind::Confirm.default_accept());
        assert!(!DialogKind::Prompt.default_accept());
        assert!(DialogKind::BeforeUnload { is_reload: true }.default_accept());
    }

    #[test]
    fn edit_flags_test_single_bits() {
        let f = EditFlags(EditFlags::CAN_COPY | EditFlags::CAN_SELECT_ALL);
        assert!(f.allows(EditFlags::CAN_COPY));
        assert!(!f.allows(EditFlags::CAN_PASTE));
    }

    #[test]
    fn a_burst_of_progress_keeps_only_the_latest() {
        let mut q = PageQueue::default();
        q.push(1, progress(7, 10));
        q.push(1, progress(7, 20));
        q.push(1, progress(8, 5));
        q.push(1, progress(8, 6));
        q.push(2, progress(8, 1));
        assert_eq!(
            q.take(),
            vec![
                (1, progress(7, 20)),
                (1, progress(8, 6)),
                (2, progress(8, 1))
            ]
        );
    }

    #[test]
    fn coalescing_never_reorders_across_other_events() {
        let mut q = PageQueue::default();
        q.push(1, progress(7, 10));
        q.push(1, PageEvent::DownloadPaused(DownloadId(7)));
        q.push(1, progress(7, 20));
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn find_results_coalesce() {
        let mut q = PageQueue::default();
        for n in 0..5 {
            q.push(
                0,
                PageEvent::FindResult {
                    matches: n,
                    active: 1,
                    done: false,
                },
            );
        }
        assert_eq!(
            q.take(),
            vec![(
                0,
                PageEvent::FindResult {
                    matches: 4,
                    active: 1,
                    done: false
                }
            )]
        );
    }

    #[test]
    fn a_full_queue_drops_console_first_and_never_a_request() {
        let mut q = PageQueue::default();
        for n in 0..MAX_PAGE_EVENTS as i32 {
            q.push(0, console(n));
        }
        q.push(0, console(-1));
        assert_eq!((q.len(), q.dropped()), (MAX_PAGE_EVENTS, 1));
        // A notice evicts the oldest console message.
        q.push(0, PageEvent::AudibleChanged(true));
        assert_eq!((q.len(), q.dropped()), (MAX_PAGE_EVENTS, 2));
        // A request always gets in.
        q.push(
            0,
            PageEvent::CertificateError {
                id: RequestId(1),
                url: String::new(),
                error: CertError::Invalid,
            },
        );
        assert_eq!(q.len(), MAX_PAGE_EVENTS + 1);
        let events = q.take();
        assert_eq!(
            events[0].1,
            console(1),
            "the oldest was not the one evicted"
        );
        assert!(
            events
                .iter()
                .any(|(_, e)| *e == PageEvent::AudibleChanged(true))
        );
        assert!(q.is_empty());
    }

    #[test]
    fn a_full_queue_of_notices_drops_the_newest_notice() {
        let mut q = PageQueue::default();
        for _ in 0..MAX_PAGE_EVENTS {
            q.push(0, PageEvent::PopupHidden);
        }
        q.push(0, PageEvent::MutedChanged(true));
        assert_eq!((q.len(), q.dropped()), (MAX_PAGE_EVENTS, 1));
    }

    #[test]
    fn only_requests_carry_an_id() {
        assert_eq!(PageEvent::PopupHidden.request(), None);
        assert_eq!(
            PageEvent::DownloadRequested {
                id: RequestId(3),
                download: DownloadId(9),
                url: String::new(),
                suggested_name: String::new(),
                total: None
            }
            .request(),
            Some(RequestId(3))
        );
    }
}
