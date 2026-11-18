// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! wubble change (see `WUBBLE_CHANGES.md`): the CEF handlers whose bodies do
//! not depend on the platform, written once instead of once per producer,
//! and [`Requests`], every question CEF is waiting on the host to answer.
//!
//! Moved here from the three `*/cef_backed.rs`: the request, download,
//! permission and context-menu handlers. New: the JS dialog and file dialog
//! handlers, the certificate-error and permission-dismissed hooks, and a
//! context menu that keeps CEF's model and waits for the host instead of
//! being emptied.
//!
//! No audio handler: returning true from `GetAudioParameters` makes CEF
//! capture the page's audio *instead of* playing it (checked on Linux: the
//! page's PulseAudio stream is corked while the capture runs), so it cannot
//! serve as an "audible" signal.
//!
//! The handler shapes follow CEF's own sample client (chromiumembedded/cef
//! `tests/cefclient/browser/client_handler.cc`: `OnBeforeContextMenu`,
//! `RunContextMenu`, `OnJSDialog`, `OnBeforeUnloadDialog`,
//! `OnResetDialogState`, `OnFileDialog`, `OnCertificateError`,
//! `OnBeforeDownload`, `OnDownloadUpdated`) and the contracts in libcef
//! (`menu_manager.cc`, `javascript_dialog_manager.cc`,
//! `file_dialog_manager.cc`, `download_manager_delegate_impl.cc`), BSD-3-Clause;
//! no code is copied.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cef::*;

use crate::downloads::{DownloadControl, DownloadOp};
use crate::error::WeldError;
use crate::held::{Answer, Held, RequestId};
use crate::permissions::{Pending, PermissionKind};
use crate::surface::{
    CefSurfaceConfig, CefSurfaceEvent, MenuSource, NavigationEvent, RequestAction, RequestFilter,
    ResponseInfo, ResponseObserver,
    ResourceRequest, WebEventQueue,
};

fn push(events: &WebEventQueue, event: NavigationEvent) {
    events.push(CefSurfaceEvent::Navigation(event));
}

/// CEF hands strings back as `CefStringUserfree`, which is empty when unset.
pub(crate) fn cef_string(s: cef::CefStringUserfree) -> String {
    let raw: Option<&cef::sys::_cef_string_utf16_t> = (&s).into();
    raw.map(|r| cef::CefStringUtf16::from(*r).to_string())
        .unwrap_or_default()
}

fn opt_string(s: Option<&cef::CefString>) -> String {
    s.map(|s| s.to_string()).unwrap_or_default()
}

fn stale(what: &'static str) -> WeldError {
    WeldError::PlatformUnsupported(what)
}

// ── New windows (wubble change 33) ────────────────────────────────────────

/// The event for a popup CEF is about to open (welding cancels it: one
/// surface per producer). Shared by the three platform life-span handlers.
pub(crate) fn new_window_requested(
    url: String,
    disposition: cef::WindowOpenDisposition,
    user_gesture: bool,
    features: Option<&cef::PopupFeatures>,
) -> NavigationEvent {
    NavigationEvent::NewWindowRequested {
        url,
        user_gesture,
        kind: new_window_kind(disposition),
        size: features.and_then(popup_size),
    }
}

fn new_window_kind(d: cef::WindowOpenDisposition) -> crate::surface::NewWindowKind {
    use crate::surface::NewWindowKind as K;
    use cef::WindowOpenDisposition as D;
    match d {
        D::NEW_FOREGROUND_TAB | D::SINGLETON_TAB | D::SWITCH_TO_TAB => K::ForegroundTab,
        D::NEW_BACKGROUND_TAB => K::BackgroundTab,
        D::NEW_POPUP => K::Popup,
        D::NEW_WINDOW => K::Window,
        _ => K::Other,
    }
}

fn popup_size(f: &cef::PopupFeatures) -> Option<(u32, u32)> {
    if f.width_set == 0 || f.height_set == 0 {
        return None;
    }
    Some((u32::try_from(f.width).ok()?, u32::try_from(f.height).ok()?))
}

// ── Frame messages (wubble changes 27, 34) ─────────────────────────────────

/// A renderer process message from the context script: queued with the
/// browser's own view of the sending frame (id, URL, main or not), so a page
/// cannot claim another frame or origin. Whether `msg` was a frame message.
/// Shared by the three platform clients.
pub(crate) fn frame_message_received(
    events: &WebEventQueue,
    frame: Option<&mut cef::Frame>,
    msg: &mut cef::ProcessMessage,
) -> bool {
    if cef::CefString::from(&msg.name()).to_string() != crate::app::FRAME_MESSAGE {
        return false;
    }
    let (Some(frame), Some(args)) = (frame, msg.argument_list()) else {
        return true;
    };
    let text = cef::CefString::from(&args.string(0)).to_string();
    events.push_frame_message(CefSurfaceEvent::FrameMessage {
        frame_id: cef_string(frame.identifier()),
        frame_url: cef_string(frame.url()),
        is_main: frame.is_main() != 0,
        pid: sender_pid(args.int(1)),
        text,
    });
    true
}

/// The renderer pid argument (wubble change 34): 0 when absent or not a pid.
pub(crate) fn sender_pid(raw: std::os::raw::c_int) -> u32 {
    u32::try_from(raw).unwrap_or(0)
}

/// Sends `text` to the context script's registered function in `frame_id`.
pub(crate) fn send_frame_message(
    browser: &cef::Browser,
    frame_id: &str,
    text: &str,
) -> Result<(), WeldError> {
    let id: cef::CefString = frame_id.into();
    let frame = browser
        .frame_by_identifier(Some(&id))
        .ok_or_else(|| WeldError::BrowserOp("that frame is gone".into()))?;
    let name: cef::CefString = crate::app::FRAME_MESSAGE.into();
    let mut message = cef::process_message_create(Some(&name))
        .ok_or_else(|| WeldError::BrowserOp("could not create a process message".into()))?;
    let args = message
        .argument_list()
        .ok_or_else(|| WeldError::BrowserOp("process message has no argument list".into()))?;
    let text: cef::CefString = text.into();
    args.set_string(0, Some(&text));
    frame.send_process_message(cef::ProcessId::RENDERER, Some(&mut message));
    Ok(())
}

// ── Defaults ──────────────────────────────────────────────────────────────

impl Answer for cef::AuthCallback {
    fn answer_default(self) {
        self.cancel();
    }
}

impl Answer for Pending {
    fn answer_default(self) {
        match self {
            Pending::Prompt { callback, .. } => callback.cont(cef::PermissionRequestResult::DENY),
            Pending::Media(callback, _) => callback.cont(0),
        }
    }
}

/// A held JS dialog; its kind decides the default.
pub(crate) struct JsDialog {
    callback: cef::JsdialogCallback,
    kind: crate::surface::JsDialogKind,
}

impl Answer for JsDialog {
    fn answer_default(self) {
        self.callback.cont(self.kind.default_accept().into(), None);
    }
}

impl Answer for cef::FileDialogCallback {
    fn answer_default(self) {
        self.cancel();
    }
}

/// The certificate-error callback (`cef_callback_t`).
impl Answer for cef::Callback {
    fn answer_default(self) {
        self.cancel();
    }
}

impl Answer for cef::RunContextMenuCallback {
    fn answer_default(self) {
        self.cancel();
    }
}

/// A download waiting for its destination, with what `DownloadStarted` will
/// report once the host answers.
pub(crate) struct AskedPath {
    callback: cef::BeforeDownloadCallback,
    download: crate::DownloadId,
    url: String,
    suggested_filename: String,
    total_bytes_expected: Option<u64>,
}

impl Answer for AskedPath {
    /// Dropping the callback unanswered is CEF's cancel: its destructor runs
    /// the download target callback with an empty path. `Continue("")`
    /// would instead save to the temp directory.
    fn answer_default(self) {
        drop(self.callback);
    }
}

impl DownloadControl for cef::DownloadItemCallback {
    fn apply(&self, op: DownloadOp) {
        match op {
            DownloadOp::Cancel => self.cancel(),
            DownloadOp::Pause => self.pause(),
            DownloadOp::Resume => self.resume(),
        }
    }
}

impl MenuSource for cef::MenuModel {
    fn count(&self) -> usize {
        ImplMenuModel::count(self)
    }
    fn kind_at(&self, index: usize) -> i64 {
        *self.type_at(index).as_ref() as i64
    }
    fn command_id_at(&self, index: usize) -> i32 {
        ImplMenuModel::command_id_at(self, index)
    }
    fn label_at(&self, index: usize) -> String {
        cef_string(ImplMenuModel::label_at(self, index))
    }
    fn visible_at(&self, index: usize) -> bool {
        self.is_visible_at(index) != 0
    }
    fn enabled_at(&self, index: usize) -> bool {
        self.is_enabled_at(index) != 0
    }
    fn checked_at(&self, index: usize) -> bool {
        self.is_checked_at(index) != 0
    }
    fn sub_menu_at(&self, index: usize) -> Option<Self> {
        ImplMenuModel::sub_menu_at(self, index)
    }
}

// ── Requests ──────────────────────────────────────────────────────────────

/// Everything CEF is waiting on the host for, one table per kind, plus the
/// drag-source switch. Shared by the handlers and the producer.
pub(crate) struct Requests {
    events: Arc<WebEventQueue>,
    pub(crate) auth: crate::auth::AuthChallenges,
    pub(crate) permissions: crate::permissions::Permissions,
    pub(crate) downloads: crate::downloads::Downloads,
    dialogs: Mutex<Held<JsDialog>>,
    files: Mutex<Held<cef::FileDialogCallback>>,
    certificates: Mutex<Held<cef::Callback>>,
    menus: Mutex<Held<cef::RunContextMenuCallback>>,
    /// Whether `StartDragging` may hand a page drag to the host.
    drag_source: AtomicBool,
}

/// Hold `callback`, or answer it with its default when the table is full.
/// The lock is released before any answer: a CEF callback may call straight
/// back into a handler that takes it.
fn hold<C: Answer>(table: &Mutex<Held<C>>, callback: C) -> Option<RequestId> {
    // wubble change: CEF asks a new question from no callback's frame, so
    // the handles it closed itself (`Held::retire`) are released here.
    let (held, retired) = {
        let mut table = table.lock().unwrap();
        (table.hold(callback), table.take_retired())
    };
    drop(retired);
    match held {
        Ok(id) => Some(id),
        Err(callback) => {
            log::warn!("weld: too many requests of one kind waiting; answering the default");
            callback.answer_default();
            None
        },
    }
}

fn take<C: Answer>(table: &Mutex<Held<C>>, id: RequestId) -> Option<C> {
    table.lock().unwrap().take(id)
}

/// Answer everything in `table` with its default, outside the lock. Returns
/// the ids that were waiting.
fn drain<C: Answer>(table: &Mutex<Held<C>>) -> Vec<RequestId> {
    let (pending, retired) = {
        let mut table = table.lock().unwrap();
        (table.drain(), table.take_retired())
    };
    drop(retired);
    pending
        .into_iter()
        .map(|(id, callback)| {
            callback.answer_default();
            id
        })
        .collect()
}

impl Requests {
    pub(crate) fn new(events: Arc<WebEventQueue>, config: &CefSurfaceConfig) -> Self {
        let requests = Self {
            events,
            auth: Default::default(),
            permissions: Default::default(),
            downloads: Default::default(),
            dialogs: Default::default(),
            files: Default::default(),
            certificates: Default::default(),
            menus: Default::default(),
            drag_source: AtomicBool::new(false),
        };
        requests.auth.set_enabled(config.handle_auth_challenges);
        requests.permissions.set_enabled(config.handle_permission_requests);
        requests.downloads.set_dir(config.download_dir.clone());
        requests.downloads.set_ask(config.ask_download_path);
        requests
    }

    /// The browser closed: answer every request still waiting.
    pub(crate) fn drain_default(&self) {
        drain(&self.auth.pending);
        drain(&self.permissions.pending);
        drain(&self.downloads.asked);
        drain(&self.dialogs);
        drain(&self.files);
        drain(&self.certificates);
        drain(&self.menus);
    }

    /// wubble change: the main frame committed a new page. Auth challenges,
    /// certificate errors, file choosers and permission requests of the old
    /// one are stale: CEF gives up on them without telling the handler (only
    /// permission prompts get `OnDismissPermissionPrompt`), so they are
    /// answered with their default here and the host told which ids to
    /// close. Dialogs and menus have their own reset hooks; download paths
    /// outlive the page.
    pub(crate) fn page_left(&self) {
        let mut ids = drain(&self.auth.pending);
        ids.extend(drain(&self.permissions.pending));
        ids.extend(drain(&self.files));
        ids.extend(drain(&self.certificates));
        if !ids.is_empty() {
            push(&self.events, NavigationEvent::RequestsDismissed { ids });
        }
    }

    /// wubble change: CEF dismissed permission prompt `prompt_id` itself
    /// (navigation, or after an answer). Still held means the host never
    /// answered: forget it and tell the host.
    pub(crate) fn permission_prompt_dismissed(&self, prompt_id: u64) {
        let dismissed = self.permissions.pending.lock().unwrap().take_where(
            |p| matches!(p, Pending::Prompt { prompt_id: held, .. } if *held == prompt_id),
        );
        // CEF already closed the prompt; the callback is dropped unanswered.
        if let Some((id, _)) = dismissed {
            push(&self.events, NavigationEvent::RequestsDismissed { ids: vec![id] });
        }
    }

    pub(crate) fn set_drag_source(&self, enabled: bool) {
        self.drag_source.store(enabled, Ordering::Release);
    }

    pub(crate) fn drag_source(&self) -> bool {
        self.drag_source.load(Ordering::Acquire)
    }

    pub(crate) fn answer_auth(
        &self,
        id: RequestId,
        credentials: Option<(&str, &str)>,
    ) -> Result<(), WeldError> {
        let callback = take(&self.auth.pending, id).ok_or(stale("no auth challenge is waiting on that id"))?;
        match credentials {
            Some((user, pass)) => {
                let user: cef::CefString = user.into();
                let pass: cef::CefString = pass.into();
                callback.cont(Some(&user), Some(&pass));
            },
            None => callback.cancel(),
        }
        Ok(())
    }

    /// Grant or deny everything the request asked for.
    pub(crate) fn answer_permission_all(&self, id: RequestId, grant: bool) -> Result<(), WeldError> {
        let pending =
            take(&self.permissions.pending, id).ok_or(stale("no permission request is waiting on that id"))?;
        match pending {
            Pending::Prompt { callback, .. } => callback.cont(if grant {
                cef::PermissionRequestResult::ACCEPT
            } else {
                cef::PermissionRequestResult::DENY
            }),
            Pending::Media(callback, requested) => callback.cont(if grant { requested } else { 0 }),
        }
        Ok(())
    }

    pub(crate) fn answer_permission(
        &self,
        id: RequestId,
        granted: &[PermissionKind],
    ) -> Result<(), WeldError> {
        let pending =
            take(&self.permissions.pending, id).ok_or(stale("no permission request is waiting on that id"))?;
        use crate::permissions::{all_granted, decode, decode_media};
        match pending {
            Pending::Prompt { callback, requested, .. } => {
                callback.cont(if all_granted(&decode(requested), granted) {
                    cef::PermissionRequestResult::ACCEPT
                } else {
                    cef::PermissionRequestResult::DENY
                })
            },
            Pending::Media(callback, requested) => {
                let all = all_granted(&decode_media(requested), granted);
                callback.cont(if all { requested } else { 0 })
            },
        }
        Ok(())
    }

    pub(crate) fn answer_dialog(&self, id: RequestId, accept: bool, text: &str) -> Result<(), WeldError> {
        let dialog = take(&self.dialogs, id).ok_or(stale("no JS dialog is waiting on that id"))?;
        let text: cef::CefString = text.into();
        dialog.callback.cont(accept.into(), Some(&text));
        Ok(())
    }

    pub(crate) fn answer_file_chooser(
        &self,
        id: RequestId,
        paths: Option<&[PathBuf]>,
    ) -> Result<(), WeldError> {
        let callback = take(&self.files, id).ok_or(stale("no file chooser is waiting on that id"))?;
        match paths {
            Some(paths) => {
                let mut list = cef::CefStringList::new();
                for path in paths {
                    list.append(&path.to_string_lossy());
                }
                callback.cont(Some(&mut list));
            },
            None => callback.cancel(),
        }
        Ok(())
    }

    pub(crate) fn answer_certificate(&self, id: RequestId, proceed: bool) -> Result<(), WeldError> {
        let callback =
            take(&self.certificates, id).ok_or(stale("no certificate error is waiting on that id"))?;
        if proceed {
            callback.cont();
        } else {
            callback.cancel();
        }
        Ok(())
    }

    pub(crate) fn answer_context_menu(&self, id: RequestId, command: Option<i32>) -> Result<(), WeldError> {
        let callback = take(&self.menus, id).ok_or(stale("no context menu is waiting on that id"))?;
        match command {
            Some(command) => callback.cont(command, cef::EventFlags::default()),
            None => callback.cancel(),
        }
        Ok(())
    }

    pub(crate) fn answer_download_path(&self, id: RequestId, path: Option<&Path>) -> Result<(), WeldError> {
        let asked =
            take(&self.downloads.asked, id).ok_or(stale("no download is waiting for a path on that id"))?;
        let Some(path) = path else {
            asked.answer_default();
            return Ok(());
        };
        self.downloads.mark_started(asked.download);
        push(&self.events, NavigationEvent::DownloadStarted {
            id: asked.download,
            url: asked.url,
            suggested_filename: asked.suggested_filename,
            destination_path: path.to_path_buf(),
            total_bytes_expected: asked.total_bytes_expected,
        });
        let path: cef::CefString = path.to_string_lossy().as_ref().into();
        // false: no system save dialog; the host already chose.
        asked.callback.cont(Some(&path), 0);
        Ok(())
    }

    pub(crate) fn steer_download(&self, id: crate::DownloadId, op: DownloadOp) -> Result<(), WeldError> {
        if self.downloads.steer(id, op) {
            Ok(())
        } else {
            Err(stale("no running download has that id"))
        }
    }
}

// ── The shared handlers, built once per producer ──────────────────────────

/// The handlers whose bodies are the same on every platform. Each platform's
/// `WeldClient` returns them from its accessors.
#[derive(Clone)]
pub(crate) struct SharedHandlers {
    pub(crate) request: cef::RequestHandler,
    pub(crate) download: cef::DownloadHandler,
    pub(crate) permission: cef::PermissionHandler,
    pub(crate) context_menu: cef::ContextMenuHandler,
    pub(crate) jsdialog: cef::JsdialogHandler,
    pub(crate) dialog: cef::DialogHandler,
}

impl SharedHandlers {
    pub(crate) fn build(
        events: Arc<WebEventQueue>,
        requests: Arc<Requests>,
        scripts: Arc<crate::app::PendingScripts>,
        metrics: Arc<Mutex<crate::view::ViewMetrics>>,
        // wubble change: checks each request before it is sent.
        request_filter: Option<RequestFilter>,
        // wubble change 36: sees each response's headers.
        response_observer: Option<ResponseObserver>,
    ) -> Self {
        Self {
            request: WeldRequestHandler::new(
                events.clone(),
                requests.clone(),
                scripts,
                request_filter,
                response_observer,
            ),
            download: WeldDownloadHandler::new(events.clone(), requests.clone()),
            permission: WeldPermissionHandler::new(events.clone(), requests.clone()),
            context_menu: WeldContextMenuHandler::new(events.clone(), requests.clone(), metrics),
            jsdialog: WeldJsDialogHandler::new(events.clone(), requests.clone()),
            dialog: WeldDialogHandler::new(events, requests),
        }
    }
}

// ── Request handler (moved; certificate errors added) ─────────────────────

cef::wrap_request_handler! {
    struct WeldRequestHandler {
        events: Arc<WebEventQueue>,
        requests: Arc<Requests>,
        scripts: Arc<crate::app::PendingScripts>,
        request_filter: Option<RequestFilter>,
        response_observer: Option<ResponseObserver>,
    }

    impl RequestHandler {
        // wubble change: a resource-request handler only when the host set
        // a filter or a response observer (change 36), so without either
        // CEF's default handling is untouched.
        // Shape from cefclient `ClientHandler::GetResourceRequestHandler`
        // (chromiumembedded/cef, BSD-3-Clause); no code copied.
        fn resource_request_handler(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            _request: Option<&mut cef::Request>,
            is_navigation: ::std::os::raw::c_int,
            _is_download: ::std::os::raw::c_int,
            request_initiator: Option<&cef::CefString>,
            _disable_default_handling: Option<&mut ::std::os::raw::c_int>,
        ) -> Option<cef::ResourceRequestHandler> {
            if self.request_filter.is_none() && self.response_observer.is_none() {
                return None;
            }
            Some(WeldResourceRequestHandler::new(
                self.request_filter.clone(),
                self.response_observer.clone(),
                opt_string(request_initiator),
                is_navigation != 0,
                Arc::new(Mutex::new(None)),
            ))
        }

        fn auth_credentials(
            &self,
            _browser: Option<&mut cef::Browser>,
            origin_url: Option<&cef::CefString>,
            is_proxy: ::std::os::raw::c_int,
            host: Option<&cef::CefString>,
            port: ::std::os::raw::c_int,
            realm: Option<&cef::CefString>,
            scheme: Option<&cef::CefString>,
            callback: Option<&mut cef::AuthCallback>,
        ) -> ::std::os::raw::c_int {
            let Some(callback) = callback else { return 0 };
            let id = if self.requests.auth.is_enabled() {
                // Held for the host to answer. CEF's auth callback is
                // reference-counted and may be answered later.
                match hold(&self.requests.auth.pending, callback.clone()) {
                    Some(id) => id,
                    None => return 1,
                }
            } else {
                // Declined now rather than held: a host that never answers
                // would otherwise keep this request open forever. Reported
                // with id 0, which is never waiting.
                callback.cancel();
                0
            };
            push(&self.events, NavigationEvent::AuthChallenged {
                id,
                origin_url: opt_string(origin_url),
                host: opt_string(host),
                port: port.clamp(0, u16::MAX as _) as u16,
                realm: opt_string(realm),
                scheme: opt_string(scheme),
                is_proxy: is_proxy != 0,
            });
            1
        }

        // wubble change (31, 33): a link the page opens elsewhere (a middle or Ctrl
        // click, Shift click) goes to the host as a new-window request, as
        // `on_before_popup` does; without this CEF navigates in place
        // (windowless browsers have no tab strip to put it in). Shape from
        // cefclient `ClientHandler::OnOpenURLFromTab` (chromiumembedded/cef,
        // BSD-3-Clause); no code copied.
        fn on_open_urlfrom_tab(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            target_url: Option<&cef::CefString>,
            target_disposition: cef::WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            push(
                &self.events,
                new_window_requested(
                    opt_string(target_url),
                    target_disposition,
                    user_gesture != 0,
                    None,
                ),
            );
            1 // handled: no navigation here
        }

        // New: held until the host proceeds or cancels; cancel by default.
        fn on_certificate_error(
            &self,
            _browser: Option<&mut cef::Browser>,
            cert_error: cef::Errorcode,
            request_url: Option<&cef::CefString>,
            _ssl_info: Option<&mut cef::Sslinfo>,
            callback: Option<&mut cef::Callback>,
        ) -> ::std::os::raw::c_int {
            // No callback: returning false cancels the request at once.
            let Some(callback) = callback else { return 0 };
            let Some(id) = hold(&self.requests.certificates, callback.clone()) else { return 1 };
            push(&self.events, NavigationEvent::CertificateError {
                id,
                url: opt_string(request_url),
                error_code: *cert_error.as_ref() as i32,
            });
            1
        }

        fn on_render_process_terminated(
            &self,
            _browser: Option<&mut cef::Browser>,
            status: cef::TerminationStatus,
            error_code: ::std::os::raw::c_int,
            error_string: Option<&cef::CefString>,
        ) {
            let status = crate::surface::termination_status(status);
            let error_string = opt_string(error_string);
            log::error!(
                "weld: CEF render process terminated ({status:?}, code {error_code}, {error_string})"
            );
            push(&self.events, NavigationEvent::ContentProcessTerminated {
                status,
                error_code,
                error_string,
            });
            self.scripts.fail_all("renderer process terminated");
        }
    }
}

// ── Resource-request handler (wubble change) ──────────────────────────────

/// wubble change: the top-level page a request belongs to. A main-frame
/// request is its own top page. Otherwise the main frame's URL, unless the
/// request's own first party (`site_for_cookies`: scheme + registrable
/// domain) names another site: on the IO thread the browser-side frame URL
/// can still be the previous page's right after a cross-document commit,
/// while the request carries the page it was made for (Brave uses the
/// request's top-frame origin for the same reason). The main frame's URL is
/// kept when it agrees, since it has the full host a per-site switch needs.
/// Last comes the initiator (no browser for service-worker requests).
fn top_url(
    url: &str,
    main_frame: bool,
    main_frame_url: Option<String>,
    first_party: &str,
    initiator: &str,
) -> String {
    if main_frame {
        return url.to_owned();
    }
    let first_party = (!first_party.is_empty()).then_some(first_party);
    match (main_frame_url.filter(|u| !u.is_empty()), first_party) {
        (Some(main), Some(site)) if !in_site(url_host(&main), url_host(site)) => site.to_owned(),
        (Some(main), _) => main,
        (None, Some(site)) => site.to_owned(),
        (None, None) => initiator.to_owned(),
    }
}

/// wubble change: the host of an absolute URL (no userinfo handling: CEF's
/// frame and first-party URLs carry none).
fn url_host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', ':', '?', '#']).next().unwrap_or_default()
}

/// wubble change: whether `host` is `site` or one of its subdomains.
fn in_site(host: &str, site: &str) -> bool {
    let (host, site) = (host.to_ascii_lowercase(), site.to_ascii_lowercase());
    host == site || host.strip_suffix(site.as_str()).is_some_and(|h| h.ends_with('.'))
}

cef::wrap_resource_request_handler! {
    struct WeldResourceRequestHandler {
        filter: Option<RequestFilter>,
        // wubble change 36.
        observer: Option<ResponseObserver>,
        initiator: String,
        is_navigation: bool,
        // wubble change 35: what `RequestAction::Serve` decided, taken by
        // `resource_handler` (CEF calls it after `on_before_resource_load`).
        served: Arc<Mutex<Option<Served>>>,
    }

    impl ResourceRequestHandler {
        // Called on the IO thread; the only place CEF lets the request be
        // changed (a changed URL is a redirect).
        fn on_before_resource_load(
            &self,
            browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            request: Option<&mut cef::Request>,
            _callback: Option<&mut cef::Callback>,
        ) -> cef::ReturnValue {
            let Some(request) = request else { return cef::ReturnValue::CONTINUE };
            let Some(filter) = &self.filter else { return cef::ReturnValue::CONTINUE };
            let url = cef_string(request.url());
            let resource_type = request.resource_type();
            let main_frame = resource_type == cef::ResourceType::MAIN_FRAME;
            let main_frame_url = browser
                .as_ref()
                .and_then(|b| b.main_frame())
                .map(|f| cef_string(f.url()));
            let checked = ResourceRequest {
                top_url: top_url(
                    &url,
                    main_frame,
                    main_frame_url,
                    &cef_string(request.first_party_for_cookies()),
                    &self.initiator,
                ),
                url,
                initiator: self.initiator.clone(),
                resource_type: resource_type.get_raw(),
                is_navigation: self.is_navigation,
                browser_id: browser.map_or(0, |b| b.identifier()),
            };
            let action = filter.check(&checked);
            let mut served = self.served.lock().unwrap_or_else(|e| e.into_inner());
            *served = None;
            match action {
                RequestAction::Allow => cef::ReturnValue::CONTINUE,
                RequestAction::Cancel => cef::ReturnValue::CANCEL,
                RequestAction::Redirect(to) => {
                    request.set_url(Some(&cef::CefString::from(to.as_str())));
                    cef::ReturnValue::CONTINUE
                }
                RequestAction::Serve(data_url) => match Served::from_data_url(&data_url) {
                    Some(body) => {
                        *served = Some(body);
                        cef::ReturnValue::CONTINUE
                    }
                    None => cef::ReturnValue::CANCEL,
                },
            }
        }

        // wubble change 36: reports the response's headers; never changes
        // the request (returns false). Called on the IO thread. Shape from
        // cefclient `ClientHandler::OnResourceResponse` (BSD-3-Clause); no
        // code copied.
        fn on_resource_response(
            &self,
            browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            request: Option<&mut cef::Request>,
            response: Option<&mut cef::Response>,
        ) -> ::std::os::raw::c_int {
            let (Some(observer), Some(request), Some(response)) = (&self.observer, request, response)
            else {
                return 0;
            };
            let header = |name: &str| cef_string(response.header_by_name(Some(&cef::CefString::from(name))));
            let info = ResponseInfo::from_headers(
                cef_string(request.url()),
                browser.map_or(0, |b| b.identifier()),
                request.resource_type().get_raw(),
                response.status(),
                &cef_string(response.mime_type()),
                header,
            );
            observer.seen(&info);
            0
        }

        // wubble change 35: serves what `on_before_resource_load` decided;
        // `None` (the default) lets the request go to the network.
        fn resource_handler(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            _request: Option<&mut cef::Request>,
        ) -> Option<cef::ResourceHandler> {
            let served = self.served.lock().unwrap_or_else(|e| e.into_inner()).take()?;
            Some(WeldServedResource::new(Arc::new(served), Arc::new(Mutex::new(0))))
        }
    }
}

/// wubble change 35: a response given as a `data:` URL.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Served {
    mime: String,
    body: Vec<u8>,
}

impl Served {
    /// `data:<mime>[;param...];base64,<payload>`; anything else is `None`.
    fn from_data_url(url: &str) -> Option<Self> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let (head, payload) = url.strip_prefix("data:")?.split_once(',')?;
        let head = head.strip_suffix(";base64")?;
        let mime = head.split(';').next().unwrap_or_default().trim();
        let mime = if mime.is_empty() { "text/plain" } else { mime };
        let body = STANDARD.decode(payload).ok()?;
        Some(Self { mime: mime.to_owned(), body })
    }
}

// wubble change 35: one `Served` response, read from memory on the IO
// thread. The `open` / `read` contract from CEF `cef_resource_handler.h`
// and libcef_dll `CefStreamResourceHandler` (BSD-3-Clause); no code copied.
cef::wrap_resource_handler! {
    struct WeldServedResource {
        served: Arc<Served>,
        offset: Arc<Mutex<usize>>,
    }

    impl ResourceHandler {
        fn open(
            &self,
            _request: Option<&mut cef::Request>,
            handle_request: Option<&mut ::std::os::raw::c_int>,
            _callback: Option<&mut cef::Callback>,
        ) -> ::std::os::raw::c_int {
            if let Some(handle) = handle_request {
                *handle = 1;
            }
            1
        }

        fn response_headers(
            &self,
            response: Option<&mut cef::Response>,
            response_length: Option<&mut i64>,
            _redirect_url: Option<&mut cef::CefString>,
        ) {
            if let Some(response) = response {
                response.set_status(200);
                response.set_status_text(Some(&cef::CefString::from("OK")));
                response.set_mime_type(Some(&cef::CefString::from(self.served.mime.as_str())));
                // A stub answers a cross-origin `fetch` / XHR too.
                response.set_header_by_name(
                    Some(&cef::CefString::from("Access-Control-Allow-Origin")),
                    Some(&cef::CefString::from("*")),
                    1,
                );
            }
            if let Some(length) = response_length {
                *length = self.served.body.len() as i64;
            }
        }

        fn read(
            &self,
            data_out: *mut u8,
            bytes_to_read: ::std::os::raw::c_int,
            bytes_read: Option<&mut ::std::os::raw::c_int>,
            _callback: Option<&mut cef::ResourceReadCallback>,
        ) -> ::std::os::raw::c_int {
            let mut offset = self.offset.lock().unwrap_or_else(|e| e.into_inner());
            let rest = self.served.body.get(*offset..).unwrap_or_default();
            let n = rest.len().min(usize::try_from(bytes_to_read).unwrap_or(0));
            if n > 0 && !data_out.is_null() {
                // SAFETY: CEF hands a buffer of `bytes_to_read` bytes, and `n`
                // is at most that and at most what is left of the body.
                unsafe { std::ptr::copy_nonoverlapping(rest.as_ptr(), data_out, n) };
                *offset += n;
            } else if n > 0 {
                return 0;
            }
            if let Some(read) = bytes_read {
                *read = n as ::std::os::raw::c_int;
            }
            // 0 with 0 bytes read: the response is complete.
            (n > 0) as ::std::os::raw::c_int
        }
    }
}

// ── Download handler (moved; callback kept, optional host path) ───────────

cef::wrap_download_handler! {
    struct WeldDownloadHandler {
        events: Arc<WebEventQueue>,
        requests: Arc<Requests>,
    }

    impl DownloadHandler {
        fn can_download(
            &self,
            _browser: Option<&mut cef::Browser>,
            _url: Option<&cef::CefString>,
            _request_method: Option<&cef::CefString>,
        ) -> ::std::os::raw::c_int {
            // Refuse before anything is created when there is nowhere to put it,
            // rather than starting a transfer and cancelling it later.
            self.requests.downloads.is_enabled() as _
        }

        fn on_before_download(
            &self,
            _browser: Option<&mut cef::Browser>,
            download_item: Option<&mut cef::DownloadItem>,
            suggested_name: Option<&cef::CefString>,
            callback: Option<&mut cef::BeforeDownloadCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(item), Some(callback)) = (download_item, callback) else {
                return 0;
            };
            let downloads = &self.requests.downloads;
            let suggested = opt_string(suggested_name);
            let id = item.id();
            let url = cef_string(item.url());
            let total = item.total_bytes();
            let total_bytes_expected = (total > 0).then_some(total as u64);
            if downloads.asks_host() {
                let asked = AskedPath {
                    callback: callback.clone(),
                    download: id,
                    url: url.clone(),
                    suggested_filename: suggested.clone(),
                    total_bytes_expected,
                };
                if let Some(request) = hold(&downloads.asked, asked) {
                    push(&self.events, NavigationEvent::DownloadRequested {
                        id: request,
                        download: id,
                        url,
                        suggested_filename: suggested,
                        total_bytes_expected,
                    });
                }
                return 1;
            }
            let Some(destination) = downloads.destination_for(&suggested) else {
                // Answering with nothing cancels it, which is what a host
                // without a download directory asked for.
                return 0;
            };
            downloads.mark_started(id);
            push(&self.events, NavigationEvent::DownloadStarted {
                id,
                url,
                suggested_filename: suggested,
                destination_path: destination.clone(),
                total_bytes_expected,
            });
            let path: cef::CefString = destination.to_string_lossy().as_ref().into();
            // false: no system save dialog. The destination is already decided,
            // and a machine nobody is sitting at cannot answer one.
            callback.cont(Some(&path), 0);
            1
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut cef::Browser>,
            download_item: Option<&mut cef::DownloadItem>,
            callback: Option<&mut cef::DownloadItemCallback>,
        ) {
            let Some(item) = download_item else { return };
            let downloads = &self.requests.downloads;
            let id = item.id();
            // CEF updates an item before it asks where to put it. Staying quiet
            // until the download has been accepted keeps DownloadStarted first.
            if !downloads.has_started(id) {
                return;
            }
            // The newest control steers the download until it ends; it is
            // safe to keep past this call.
            if let Some(callback) = callback {
                downloads.set_control(id, Box::new(callback.clone()));
            }
            let path = PathBuf::from(cef_string(item.full_path()));
            let total = item.total_bytes();
            let total_bytes_expected = (total > 0).then_some(total as u64);
            let received = item.received_bytes().max(0) as u64;
            // wubble change: Chromium's INTERRUPTED (network, disk, server)
            // is neither complete nor cancelled. The control is kept, since
            // `resume_download` may retry it; CEF may also resume it itself,
            // and progress follows.
            if item.is_interrupted() != 0 {
                push(&self.events, NavigationEvent::DownloadInterrupted {
                    id,
                    reason: *item.interrupt_reason().as_ref() as i32,
                });
                return;
            }
            let complete = item.is_complete() != 0;
            let canceled = item.is_canceled() != 0;

            if complete || canceled || downloads.due_for_progress(id, std::time::Instant::now()) {
                push(&self.events, NavigationEvent::DownloadProgress {
                    id,
                    bytes_received: received,
                    total_bytes_expected,
                });
            }
            if canceled {
                push(&self.events, NavigationEvent::DownloadCancelled {
                    id,
                    destination_path: path,
                });
            } else if complete {
                push(&self.events, NavigationEvent::DownloadFinished {
                    id,
                    destination_path: path,
                    error: None,
                });
            }
            if complete || canceled {
                downloads.forget(id);
            }
        }
    }
}

// ── Permission handler (moved) ────────────────────────────────────────────

cef::wrap_permission_handler! {
    struct WeldPermissionHandler {
        events: Arc<WebEventQueue>,
        requests: Arc<Requests>,
    }

    impl PermissionHandler {
        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut cef::Browser>,
            prompt_id: u64,
            requesting_origin: Option<&cef::CefString>,
            requested_permissions: u32,
            callback: Option<&mut cef::PermissionPromptCallback>,
        ) -> ::std::os::raw::c_int {
            let Some(callback) = callback else { return 0 };
            let pending = Pending::Prompt {
                callback: callback.clone(),
                requested: requested_permissions,
                prompt_id,
            };
            let Some(id) = self.hold(pending) else { return 1 };
            push(&self.events, NavigationEvent::PermissionRequested {
                id,
                origin: opt_string(requesting_origin),
                permissions: crate::permissions::decode(requested_permissions),
                raw: requested_permissions,
            });
            1
        }

        fn on_request_media_access_permission(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            requesting_origin: Option<&cef::CefString>,
            requested_permissions: u32,
            callback: Option<&mut cef::MediaAccessCallback>,
        ) -> ::std::os::raw::c_int {
            let Some(callback) = callback else { return 0 };
            let pending = Pending::Media(callback.clone(), requested_permissions);
            let Some(id) = self.hold(pending) else { return 1 };
            push(&self.events, NavigationEvent::PermissionRequested {
                id,
                origin: opt_string(requesting_origin),
                // Media capture arrives on its own callback with its own
                // bits, so it is decoded from the media set rather than the
                // prompt set.
                permissions: crate::permissions::decode_media(requested_permissions),
                raw: requested_permissions,
            });
            1
        }

        // wubble change: navigation or browser close dismissed a prompt.
        fn on_dismiss_permission_prompt(
            &self,
            _browser: Option<&mut cef::Browser>,
            prompt_id: u64,
            _result: cef::PermissionRequestResult,
        ) {
            self.requests.permission_prompt_dismissed(prompt_id);
        }
    }
}

impl WeldPermissionHandler {
    /// Held for the host, or denied now when the host does not answer
    /// permissions itself (reported with id 0, which is never waiting).
    /// `None` when the request was already answered by the cap.
    fn hold(&self, pending: Pending) -> Option<RequestId> {
        if self.requests.permissions.is_enabled() {
            hold(&self.requests.permissions.pending, pending)
        } else {
            pending.answer_default();
            Some(0)
        }
    }
}

// ── Context menu handler (moved; the model is kept and the menu held) ─────

cef::wrap_context_menu_handler! {
    struct WeldContextMenuHandler {
        events: Arc<WebEventQueue>,
        requests: Arc<Requests>,
        metrics: Arc<Mutex<crate::view::ViewMetrics>>,
    }

    impl ContextMenuHandler {
        // CEF's default model is left as built: CEF calls RunContextMenu only
        // for a non-empty model, and runs the standard commands itself when
        // the host picks one (menu_manager.cc `ExecuteDefaultCommand`).
        fn run_context_menu(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            params: Option<&mut cef::ContextMenuParams>,
            model: Option<&mut cef::MenuModel>,
            callback: Option<&mut cef::RunContextMenuCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(params), Some(callback)) = (params, callback) else { return 0 };
            // Everything is read now: CEF detaches the params when this
            // returns.
            let items = model
                .map(|model| crate::surface::menu_items(&*model, 1))
                .unwrap_or_default();
            // CEF reports DIP; every coordinate this API hands out or takes
            // is physical, so convert on the way out.
            let scale = self.metrics.lock().unwrap().scale();
            let flags = params.type_flags().as_ref().0 as u32;
            let mut suggestions = cef::CefStringList::new();
            params.dictionary_suggestions(Some(&mut suggestions));
            let Some(id) = hold(&self.requests.menus, callback.clone()) else { return 1 };
            push(&self.events, NavigationEvent::ContextMenuRequested {
                id,
                x: (params.xcoord() as f32 * scale).round() as i32,
                y: (params.ycoord() as f32 * scale).round() as i32,
                targets: crate::surface::context_menu_targets(flags),
                link_url: cef_string(params.link_url()),
                source_url: cef_string(params.source_url()),
                page_url: cef_string(params.page_url()),
                selection_text: cef_string(params.selection_text()),
                frame_url: cef_string(params.frame_url()),
                media: crate::surface::context_menu_media(*params.media_type().as_ref() as i64),
                has_image_contents: params.has_image_contents() != 0,
                editable: params.is_editable() != 0,
                edit_flags: params.edit_state_flags().as_ref().0 as u32,
                media_flags: params.media_state_flags().as_ref().0 as u32,
                misspelled_word: cef_string(params.misspelled_word()),
                suggestions: suggestions.into_iter().collect(),
                items,
            });
            1
        }

        // CEF closed the menu: the host answered it, or CEF cancelled it
        // itself (`CefMenuManager::CancelContextMenu`: the view blurred or
        // hid, a non-right mouse press or a wheel reached the page, the
        // browser is closing). Either way CEF has already run the callback
        // (menu_manager.cc `ExecuteCommandCallback` is what calls
        // `MenuClosed`), so anything still held is stale.
        //
        // wubble change: retired, not drained. On a cancel CEF calls
        // `Continue` through a raw pointer it holds no reference for, and
        // this runs inside that `Continue`; dropping the held handle here
        // released the last reference and freed the callback before
        // `Continue` returned (SIGSEGV in `BindStateHolder::Reset` under
        // `CefRenderWidgetHostViewOSR::SetFocus`). Answering it again would
        // also be a second answer.
        fn on_context_menu_dismissed(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
        ) {
            if !self.requests.menus.lock().unwrap().retire().is_empty() {
                push(&self.events, NavigationEvent::ContextMenuDismissed);
            }
        }
    }
}

// ── JS dialog handler (new) ───────────────────────────────────────────────

cef::wrap_jsdialog_handler! {
    struct WeldJsDialogHandler {
        events: Arc<WebEventQueue>,
        requests: Arc<Requests>,
    }

    impl JsdialogHandler {
        fn on_jsdialog(
            &self,
            _browser: Option<&mut cef::Browser>,
            origin_url: Option<&cef::CefString>,
            dialog_type: cef::JsdialogType,
            message_text: Option<&cef::CefString>,
            default_prompt_text: Option<&cef::CefString>,
            callback: Option<&mut cef::JsdialogCallback>,
            _suppress_message: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let kind = crate::surface::js_dialog_kind(*dialog_type.as_ref() as i64);
            self.request(kind, opt_string(origin_url), message_text, opt_string(default_prompt_text), callback)
        }

        fn on_before_unload_dialog(
            &self,
            _browser: Option<&mut cef::Browser>,
            message_text: Option<&cef::CefString>,
            is_reload: ::std::os::raw::c_int,
            callback: Option<&mut cef::JsdialogCallback>,
        ) -> ::std::os::raw::c_int {
            let kind = crate::surface::JsDialogKind::BeforeUnload { is_reload: is_reload != 0 };
            self.request(kind, String::new(), message_text, String::new(), callback)
        }

        // Called on navigation and after every dialog closes. Whatever is
        // still held is answered with its default and the host told.
        fn on_reset_dialog_state(&self, _browser: Option<&mut cef::Browser>) {
            if !drain(&self.requests.dialogs).is_empty() {
                push(&self.events, NavigationEvent::JsDialogsReset);
            }
        }
    }
}

impl WeldJsDialogHandler {
    fn request(
        &self,
        kind: crate::surface::JsDialogKind,
        origin_url: String,
        message: Option<&cef::CefString>,
        default_text: String,
        callback: Option<&mut cef::JsdialogCallback>,
    ) -> ::std::os::raw::c_int {
        // No callback to answer through: let CEF handle (suppress) it.
        let Some(callback) = callback else { return 0 };
        let dialog = JsDialog {
            callback: callback.clone(),
            kind,
        };
        if let Some(id) = hold(&self.requests.dialogs, dialog) {
            push(&self.events, NavigationEvent::JsDialogRequested {
                id,
                kind,
                origin_url,
                message: opt_string(message),
                default_text,
            });
        }
        // Handled, answered now or later. Returning false would let Alloy
        // run its own dialog, a real window under windowless rendering.
        1
    }
}

// ── File dialog handler (new) ─────────────────────────────────────────────

cef::wrap_dialog_handler! {
    struct WeldDialogHandler {
        events: Arc<WebEventQueue>,
        requests: Arc<Requests>,
    }

    impl DialogHandler {
        fn on_file_dialog(
            &self,
            _browser: Option<&mut cef::Browser>,
            mode: cef::FileDialogMode,
            title: Option<&cef::CefString>,
            default_file_path: Option<&cef::CefString>,
            accept_filters: Option<&mut cef::CefStringList>,
            accept_extensions: Option<&mut cef::CefStringList>,
            accept_descriptions: Option<&mut cef::CefStringList>,
            callback: Option<&mut cef::FileDialogCallback>,
        ) -> ::std::os::raw::c_int {
            // Returning false would show CEF's own dialog: a GTK window.
            let Some(callback) = callback else { return 0 };
            let list = |l: Option<&mut cef::CefStringList>| {
                l.map(crate::surface::borrowed_string_list).unwrap_or_default()
            };
            let accept = crate::surface::accept_filters(
                list(accept_filters),
                list(accept_extensions),
                list(accept_descriptions),
            );
            if let Some(id) = hold(&self.requests.files, callback.clone()) {
                push(&self.events, NavigationEvent::FileChooserRequested {
                    id,
                    mode: crate::surface::file_chooser_mode(*mode.as_ref() as i64),
                    title: opt_string(title),
                    default_path: PathBuf::from(opt_string(default_file_path)),
                    accept,
                });
            }
            1
        }
    }
}

// wubble change: the request filter's top-page rule.
#[cfg(test)]
mod request_filter_tests {
    use super::{Served, top_url};

    // wubble change 35.
    #[test]
    fn served_data_urls_decode_base64_only() {
        let gif = Served::from_data_url("data:image/gif;base64,R0lGODlhAQABAAAAACw=").unwrap();
        assert_eq!(gif.mime, "image/gif");
        assert_eq!(&gif.body[..6], b"GIF89a");
        let js = Served::from_data_url("data:application/javascript;charset=utf-8;base64,KGZ1bmN0aW9uKCl7fSkoKTs=").unwrap();
        assert_eq!((js.mime.as_str(), js.body.as_slice()), ("application/javascript", &b"(function(){})();"[..]));
        let empty = Served::from_data_url("data:;base64,").unwrap();
        assert_eq!((empty.mime.as_str(), empty.body.len()), ("text/plain", 0));
        assert_eq!(Served::from_data_url("data:text/plain,plain"), None);
        assert_eq!(Served::from_data_url("data:text/plain;base64,@@@"), None);
        assert_eq!(Served::from_data_url("https://x.test/;base64,AA=="), None);
    }

    #[test]
    fn top_url_prefers_the_request_then_main_frame_then_initiator() {
        let main = Some("https://page.test/".to_owned());
        assert_eq!(top_url("https://new.test/", true, main.clone(), "https://x.test/", "https://i.test"), "https://new.test/");
        assert_eq!(top_url("https://cdn.test/a.js", false, main.clone(), "", "https://i.test"), "https://page.test/");
        assert_eq!(top_url("https://cdn.test/a.js", false, Some(String::new()), "", "https://i.test"), "https://i.test");
        assert_eq!(top_url("https://cdn.test/a.js", false, None, "", ""), "");
        assert_eq!(top_url("https://cdn.test/a.js", false, None, "https://page.test/", "https://i.test"), "https://page.test/");
    }

    #[test]
    fn top_url_trusts_the_request_first_party_over_a_stale_main_frame() {
        // The frame still reads the previous page A while B's requests run.
        let stale = Some("https://a.test/old".to_owned());
        assert_eq!(top_url("https://ads.test/x.js", false, stale, "https://b.test/", ""), "https://b.test/");
        // Agreeing site: the frame URL keeps its full host (per-site switch).
        let main = Some("https://WWW.b.test:8443/page".to_owned());
        assert_eq!(top_url("https://ads.test/x.js", false, main.clone(), "https://b.test/", ""), "https://WWW.b.test:8443/page");
        // A suffix that is not a subdomain is another site.
        let main = Some("https://notb.test/".to_owned());
        assert_eq!(top_url("https://ads.test/x.js", false, main, "https://b.test/", ""), "https://b.test/");
    }
}

#[cfg(test)]
mod new_window_tests {
    use super::{new_window_kind, popup_size};
    use crate::surface::NewWindowKind as K;
    use cef::WindowOpenDisposition as D;

    #[test]
    fn dispositions_map_to_kinds() {
        assert_eq!(new_window_kind(D::NEW_FOREGROUND_TAB), K::ForegroundTab);
        assert_eq!(new_window_kind(D::NEW_BACKGROUND_TAB), K::BackgroundTab);
        assert_eq!(new_window_kind(D::NEW_POPUP), K::Popup);
        assert_eq!(new_window_kind(D::NEW_WINDOW), K::Window);
        assert_eq!(new_window_kind(D::NEW_PICTURE_IN_PICTURE), K::Other);
    }

    #[test]
    fn size_needs_both_features() {
        let f = |w: i32, ws: i32, h: i32, hs: i32| cef::PopupFeatures {
            width: w,
            width_set: ws,
            height: h,
            height_set: hs,
            ..Default::default()
        };
        assert_eq!(popup_size(&f(400, 1, 300, 1)), Some((400, 300)));
        assert_eq!(popup_size(&f(400, 1, 300, 0)), None);
        assert_eq!(popup_size(&f(-1, 1, 300, 1)), None);
    }
}

#[cfg(test)]
mod frame_message_tests {
    use super::sender_pid;

    #[test]
    fn the_sender_pid_is_kept_and_a_bad_one_is_zero() {
        assert_eq!(sender_pid(4242), 4242);
        assert_eq!(sender_pid(0), 0);
        assert_eq!(sender_pid(-1), 0);
    }
}
