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

/// welding's plain-data types, used as they are (no CEF types in them).
/// `JsDialogKind::default_accept` is the one table of dialog defaults.
pub use welding::{AcceptFilter, FileChooserMode, JsDialogKind, PermissionKind};

/// The token the host quotes back to answer one request. Unique across
/// every kind of request and every view for the engine's life (welding
/// counts them once per process); never 0.
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

/// CEF's `cef_context_menu_media_state_flags_t`: the state of the media
/// element under the cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MediaFlags(pub u32);

impl MediaFlags {
    pub const PAUSED: u32 = 1 << 1;
    pub const MUTED: u32 = 1 << 2;
    pub const LOOP: u32 = 1 << 3;
    pub const CAN_SAVE: u32 = 1 << 4;
    pub const HAS_AUDIO: u32 = 1 << 5;
    pub const CAN_TOGGLE_CONTROLS: u32 = 1 << 6;
    pub const CONTROLS: u32 = 1 << 7;
    pub const CAN_PICTURE_IN_PICTURE: u32 = 1 << 10;
    pub const CAN_LOOP: u32 = 1 << 12;

    pub fn has(self, flag: u32) -> bool {
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
    /// CEF's id of the frame clicked in, for `View::run_script_in`; empty
    /// when CEF named none.
    pub frame: String,
    pub selection_text: String,
    pub editable: bool,
    pub edit: EditFlags,
    pub media_flags: MediaFlags,
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
pub use wubble_core::pagedrag::DragOps;

/// What is being dragged, in either direction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DragData {
    pub files: Vec<PathBuf>,
    pub link_url: Option<String>,
    pub link_title: Option<String>,
    pub text: Option<String>,
    pub html: Option<String>,
    pub base_url: Option<String>,
    /// A dragged picture's own bytes and file name (page drags only). Boxed, as is the
    /// image: the events carrying a drag stay small.
    pub file_contents: Option<Box<DragFileContents>>,
    /// The thumbnail under the pointer (page drags only).
    pub image: Option<Box<DragImage>>,
}

pub use welding::{DragFileContents, DragImage};

impl wubble_core::pagedrag::Payload for DragData {
    fn link(&self) -> Option<&str> {
        self.link_url.as_deref().filter(|u| !u.is_empty())
    }

    fn text(&self) -> Option<&str> {
        self.text.as_deref().filter(|t| !t.is_empty())
    }
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
            file_contents: p.file_contents.map(Box::new),
            image: p.image.map(Box::new),
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
            file_contents: d.file_contents.map(|c| *c),
            image: d.image.map(|i| *i),
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

/// What a frame asks of the host's clipboard ([`PageEvent::Clipboard`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardRequest {
    /// The page put this text on the clipboard (a copy or cut).
    Copy(String),
    /// The selection changed: the primary selection's text.
    Primary(String),
    /// The page is pasting: answer with [`crate::view::View::answer_clipboard`].
    /// `primary`: a middle click's paste, from the primary selection.
    Paste { primary: bool },
    /// `navigator.clipboard.readText` passed its permission check: answer with
    /// the read's id.
    Read(u32),
}

/// The content scripts' channels on welding's one frame message (wubble-helper
/// `compose`: `name\ntext` both ways). The names must match the helper's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameChannel {
    /// `autofill.js`: [`PageEvent::FrameMessage`].
    Autofill,
    /// `clipboard.js`: [`PageEvent::Clipboard`].
    Clipboard,
    /// `zoom.js`: [`PageEvent::WheelZoom`].
    Zoom,
    /// `probes.js`: [`PageEvent::Probe`].
    Probes,
    /// `cosmetic.js`: answered by the engine itself with the content filter's
    /// rules ([`crate::cosmetic`]); no [`PageEvent`].
    Cosmetic,
}

impl FrameChannel {
    const ALL: [(FrameChannel, &'static str); 5] = [
        (FrameChannel::Autofill, "autofill"),
        (FrameChannel::Clipboard, "clipboard"),
        (FrameChannel::Zoom, "zoom"),
        (FrameChannel::Probes, "probes"),
        (FrameChannel::Cosmetic, "cosmetic"),
    ];

    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(c, _)| *c == self)
            .map_or("", |(_, n)| n)
    }

    /// The channel a frame message came on, and its text; `None` for a message
    /// on no known channel.
    pub fn split(message: &str) -> Option<(FrameChannel, &str)> {
        let (name, text) = message.split_once('\n')?;
        let channel = Self::ALL.iter().find(|(_, n)| *n == name)?.0;
        Some((channel, text))
    }

    /// `text` for this channel's script.
    pub fn join(self, text: &str) -> String {
        format!("{}\n{text}", self.name())
    }
}

/// A frame of the page as the browser process sees it (CEF's own frame), not
/// as the page describes itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    /// CEF's frame identifier: [`crate::view::View::send_frame_message`] names it.
    pub id: String,
    /// The frame's URL when the message arrived.
    pub url: String,
    pub is_main: bool,
    /// The renderer process the frame runs in, as it reported itself (welding change 34); 0
    /// when unknown. Several tabs may share one.
    pub pid: u32,
}

/// What the host learns about a page beyond navigation, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageEvent {
    /// An accessibility tree change (CEF's `OnAccessibilityTreeChange`), as the JSON CEF
    /// wrote of its value; only after [`crate::view::View::set_accessibility`] turned it on.
    /// `wubble_ax::AxValue::from_json` reads it.
    AxTree(String),
    /// An accessibility location change (`OnAccessibilityLocationChange`), likewise.
    AxLocation(String),
    /// A frame's clipboard request, when the engine runs with
    /// `EngineConfig::clipboard_bridge` (wubble-helper `clipboard.js` on
    /// [`FrameChannel::Clipboard`]). `frame` names the frame for the answer.
    Clipboard {
        frame: String,
        request: ClipboardRequest,
    },
    /// A Ctrl+wheel the page left alone (`zoom.js`): zoom one step in or out,
    /// as Chromium does.
    WheelZoom {
        zoom_in: bool,
    },
    /// The page's icon, downloaded by CEF: raw RGBA, at most 64x64.
    ///
    /// The host (wubble-store holds the cache; `crates/wubble` is not edited
    /// here):
    ///
    /// ```ignore
    /// PageEvent::FaviconImage(img) => {
    ///     // Show: slint::Image from
    ///     //   SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&img.rgba, img.width, img.height)
    ///     // Persist, only if the view's `nav().page.favicon == img.url` (drops a
    ///     // late image of the previous page), on the store thread:
    ///     //   store.put_favicon(&page_url, &Favicon { icon_url: img.url, width: img.width,
    ///     //       height: img.height, rgba: img.rgba }, now_ms).await?;
    /// }
    /// // Restored/discarded tabs and command-bar rows, one query:
    /// //   let icons = store.favicons_for(&urls).await?;  // HashMap<page url, Favicon>
    /// // At startup: store.prune_favicons(4 << 20).await?;
    /// ```
    FaviconImage(crate::favicon::FaviconImage),
    /// `View::print_to_pdf` finished; `ok` is false when no file was written.
    PdfPrinted {
        path: PathBuf,
        ok: bool,
    },
    /// A `<select>` dropdown, date picker or autofill list opened. Its frames
    /// come from `View::take_popup_frame`; draw them at `rect` as a layer
    /// above the view (it may extend outside the view).
    PopupShown(Rect),
    PopupMoved(Rect),
    PopupHidden,
    /// Answer with `View::answer_dialog`.
    DialogRequested {
        id: RequestId,
        kind: JsDialogKind,
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
        permissions: Vec<PermissionKind>,
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
    /// `code` is Chromium's net error (`ERR_CERT_*`, -200 to -219,
    /// `net/base/net_error_list.h`).
    CertificateError {
        id: RequestId,
        url: String,
        code: i32,
    },
    /// CEF gave up on these requests without an answer (their page was left,
    /// or a permission prompt was dismissed): close what shows them.
    RequestsDismissed(Vec<RequestId>),
    /// Answer with `View::answer_download_path`; no file dialog is shown.
    DownloadRequested {
        id: RequestId,
        download: DownloadId,
        url: String,
        /// The URL the download was started for (before redirects): what
        /// `View::start_download` was given.
        original_url: String,
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
    /// The download was interrupted (network, disk or server error). Not
    /// an end: `View::resume_download` retries it, CEF may resume it itself
    /// (`DownloadResumed` follows), `View::cancel_download` ends it.
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
    MutedChanged(bool),
    /// The answer of `View::probe_audible` changed: a media element of the
    /// main document plays with sound (or stopped).
    Audible(bool),
    /// The page answered `View::set_frozen` (CDP `Page.setWebLifecycleState`): `ok` false when
    /// Chromium refused (the page is not the active top-level page, say).
    Frozen {
        frozen: bool,
        ok: bool,
    },
    /// The page answered `View::request_target_id` with its CDP target id (`Err`: refused).
    TargetId(Result<String, ()>),
    /// The main frame committed a new document (not a same-document navigation): what the old
    /// documents' frames reported through frame messages is stale.
    Committed,
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
    /// During a drag over the view: what a drop at the pointer would do
    /// (CEF `UpdateDragCursor`; `DragOps::NONE`: not allowed there).
    DragCursor(DragOps),
    /// The context script (wubble-helper's `autofill.js`) in `frame` sent
    /// `text`. It may hold form values, so it is a `Secret`: never printed.
    /// Answer with `View::send_frame_message`.
    FrameMessage {
        frame: FrameInfo,
        text: wubble_core::secret::Secret,
    },
    /// A busy probe's report from `frame` (wubble-helper `probes.js` on
    /// [`FrameChannel::Probes`]): what in the frame must keep its tab awake.
    /// The first one, empty, tells which renderer the frame runs in.
    Probe {
        frame: FrameInfo,
        report: String,
    },
    /// The answer to `View::cdp`: the call's result, or the protocol's error.
    CdpReply {
        id: crate::cdp::CdpId,
        result: Result<serde_json::Value, String>,
    },
    /// The answer to `View::eval`: the script's value as JSON text, or why it
    /// failed (it threw, or the page went away first).
    EvalResult {
        id: crate::cdp::EvalId,
        result: Result<String, String>,
    },
    /// The hovered link's URL for the status bubble; empty hides it.
    LinkStatus(String),
    /// A `title` tooltip to show near the pointer; empty hides it.
    Tooltip(String),
    /// The page went to a scheme Chromium cannot handle (`mailto:`, `tel:`,
    /// an app's scheme). Nothing was opened: the host asks and hands it to
    /// the OS. `gesture` is whether a user gesture started it, when CEF said.
    ExternalProtocol {
        url: String,
        gesture: Option<bool>,
    },
    /// The view's navigation rule ([`crate::view::View::set_navigation_rule`]) refused a
    /// top-level navigation to `url` (`redirect`: one hop of a redirect) before it was sent:
    /// the page stays where it is.
    NavigationRefused {
        url: String,
        redirect: bool,
    },
    /// `View::download_image` finished: the image's pixels as RGBA, or why
    /// there are none.
    ImageDownloaded {
        url: String,
        image: Result<crate::favicon::Rgba, String>,
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

    /// A notice a full queue may lose: console output and the intermediate
    /// updates a later one supersedes. Requests and every event that starts
    /// or ends something (a download, a dialog, a drag, fullscreen) are
    /// never dropped.
    fn droppable(&self) -> bool {
        matches!(
            self,
            Self::Console { .. }
                | Self::DownloadProgress { .. }
                | Self::FindResult { done: false, .. }
                | Self::ImeBounds { .. }
                | Self::PopupMoved(_)
                | Self::LinkStatus(_)
                | Self::Tooltip(_)
        )
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
            | (Self::PopupMoved(_), Self::PopupMoved(_))
            | (Self::LinkStatus(_), Self::LinkStatus(_))
            | (Self::Tooltip(_), Self::Tooltip(_)) => true,
            _ => false,
        }
    }
}

/// At most this many events wait for the host before droppable notices
/// (console output, intermediate progress, find, IME and popup moves) are
/// dropped, oldest first. Other events are always kept: requests are capped
/// by welding per kind and view, and the rest follow real lifecycles.
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
        if self.events.len() >= MAX_PAGE_EVENTS {
            match self.events.iter().position(|(_, e)| e.droppable()) {
                Some(i) => {
                    self.events.remove(i);
                }
                None if event.droppable() => {
                    self.dropped += 1;
                    return;
                }
                None => {}
            }
            if self.events.len() < MAX_PAGE_EVENTS {
                self.dropped += 1;
            }
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

/// Whether `url` belongs to another app (`mailto:`, `tel:`, an app's scheme) rather than to
/// the engine: the same list the engine cancels such navigations by
/// ([`PageEvent::ExternalProtocol`]), so the host can hand a URL over before opening a tab
/// or loading it.
pub fn is_external(url: &str) -> bool {
    !welding::chromium_handles(url)
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
    fn external_urls_are_the_ones_the_engine_does_not_load() {
        assert!(is_external("mailto:a@b"));
        assert!(is_external("magnet:?xt=1"));
        assert!(!is_external("https://a.test/"));
        assert!(!is_external("about:blank"));
    }

    #[test]
    fn log_levels_follow_cef_severity() {
        let got: Vec<_> = (0..7).map(LogLevel::from_severity).collect();
        use LogLevel::*;
        assert_eq!(got, vec![Info, Verbose, Info, Warning, Error, Fatal, Info]);
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
    fn status_and_tooltip_texts_coalesce() {
        let mut q = PageQueue::default();
        q.push(1, PageEvent::LinkStatus("a".into()));
        q.push(1, PageEvent::LinkStatus(String::new()));
        q.push(1, PageEvent::Tooltip("t".into()));
        q.push(1, PageEvent::Tooltip("u".into()));
        assert_eq!(
            q.take(),
            vec![
                (1, PageEvent::LinkStatus(String::new())),
                (1, PageEvent::Tooltip("u".into()))
            ]
        );
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
    fn a_full_queue_drops_the_oldest_droppable_and_never_a_request() {
        let mut q = PageQueue::default();
        for n in 0..MAX_PAGE_EVENTS as i32 {
            q.push(0, console(n));
        }
        // A droppable notice replaces the oldest droppable one.
        q.push(0, console(-1));
        assert_eq!((q.len(), q.dropped()), (MAX_PAGE_EVENTS, 1));
        q.push(0, PageEvent::MutedChanged(true));
        assert_eq!((q.len(), q.dropped()), (MAX_PAGE_EVENTS, 2));
        q.push(
            0,
            PageEvent::CertificateError {
                id: RequestId(1),
                url: String::new(),
                code: -207,
            },
        );
        assert_eq!((q.len(), q.dropped()), (MAX_PAGE_EVENTS, 3));
        let events = q.take();
        assert_eq!(
            events[0].1,
            console(3),
            "the oldest was not the one evicted"
        );
        assert!(
            events
                .iter()
                .any(|(_, e)| *e == PageEvent::MutedChanged(true))
        );
        assert!(
            events
                .iter()
                .any(|(_, e)| e.request() == Some(RequestId(1)))
        );
        assert!(q.is_empty());
    }

    /// A host that fell behind must still hear that a download, dialog or
    /// drag ended, or it shows them forever (or CEF waits on the drag).
    #[test]
    fn a_full_queue_keeps_events_that_end_something() {
        let mut q = PageQueue::default();
        for _ in 0..MAX_PAGE_EVENTS {
            q.push(0, PageEvent::PopupHidden);
        }
        for event in [
            PageEvent::DownloadCancelled(DownloadId(1)),
            PageEvent::DialogsReset,
            PageEvent::ContextMenuDismissed,
            PageEvent::FullscreenChanged(false),
            PageEvent::RequestsDismissed(vec![RequestId(4)]),
        ] {
            q.push(0, event.clone());
            assert_eq!(
                q.take().last().map(|(_, e)| e),
                Some(&event),
                "{event:?} dropped"
            );
            for _ in 0..MAX_PAGE_EVENTS {
                q.push(0, PageEvent::PopupHidden);
            }
        }
        assert_eq!(q.dropped(), 0);
        // With nothing droppable queued, a droppable notice is the one lost.
        q.push(0, console(0));
        assert_eq!(q.dropped(), 1);
    }

    #[test]
    fn only_requests_carry_an_id() {
        assert_eq!(PageEvent::PopupHidden.request(), None);
        assert_eq!(
            PageEvent::DownloadRequested {
                id: RequestId(3),
                download: DownloadId(9),
                url: String::new(),
                original_url: String::new(),
                suggested_name: String::new(),
                total: None
            }
            .request(),
            Some(RequestId(3))
        );
    }
}
