//! The extension tab APIs of the patched CEF (`patches/chromium`, docs/research/extensions.md
//! "The patch queue"): chrome.tabs, chrome.windows and chrome.action on wubble's own tabs and
//! windows. The patches export one plain C table, `cef_wubble_ext_v1` (their
//! `wubble_ext_capi.h`, mirrored by [`ffi`]); this is the only file that names it. A stock CEF
//! has no such symbol: [`ExtApi::find`] gets `None` and everything here does nothing, which is
//! today's behaviour (the APIs see no tab and no window).
//!
//! Data flows one way: the host pushes a projection of its model
//! ([`wubble_core::extensions::project`]) in [`ViewId`] terms, the engine sends it in browser
//! ids whenever it or a view's browser changes, and CEF turns it into Chromium's window
//! controllers and tab events. What extensions ask to change comes back as [`ExtRequest`]s for
//! the host to apply to its model. Everything runs on CEF's UI thread, which is the engine's
//! (external message pump); the delegate's callbacks only queue. The delegate's shape (the host
//! opens, selects and removes tabs on its own model) follows samuelmaddock/electron-browser-shell
//! @3817c42 `packages/electron-chrome-extensions/src/browser/api/tabs.ts` (MIT); no code copied.

use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::sync::Mutex;

use wubble_core::extensions::{ExtWindow, ExtWindowKind};
use wubble_core::space::ContainerId;
use wubble_core::window::WindowId;

use crate::app::ViewId;

/// `wubble_ext_capi.h`, field for field.
#[allow(non_camel_case_types)]
pub(crate) mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    pub const VERSION: c_int = 1;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct cef_wubble_ext_window_t {
        pub window_id: c_int,
        pub focused: c_int,
        pub type_: c_int,
        pub x: c_int,
        pub y: c_int,
        pub width: c_int,
        pub height: c_int,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct cef_wubble_ext_tab_t {
        pub browser_id: c_int,
        pub window_id: c_int,
        pub index: c_int,
        pub active: c_int,
        pub pinned: c_int,
    }

    #[repr(C)]
    pub struct cef_wubble_ext_delegate_t {
        pub ctx: *mut c_void,
        pub open_tab: unsafe extern "C" fn(
            ctx: *mut c_void,
            request_id: c_int,
            profile_path: *const c_char,
            url: *const c_char,
            window_id: c_int,
            active: c_int,
        ),
        pub close_tab: unsafe extern "C" fn(ctx: *mut c_void, browser_id: c_int),
        pub activate_tab: unsafe extern "C" fn(ctx: *mut c_void, browser_id: c_int),
        pub action_changed: unsafe extern "C" fn(ctx: *mut c_void, extension_id: *const c_char),
    }

    #[repr(C)]
    pub struct cef_wubble_ext_action_t {
        pub title: *mut c_char,
        pub badge: *mut c_char,
        pub badge_bg: u32,
        pub badge_fg: u32,
        pub enabled: c_int,
        pub popup_url: *mut c_char,
        pub icon_png: *mut u8,
        pub icon_png_len: usize,
        pub icon_path: *mut c_char,
    }

    #[repr(C)]
    pub struct cef_wubble_ext_t {
        pub size: usize,
        pub version: c_int,
        pub set_delegate: unsafe extern "C" fn(delegate: *const cef_wubble_ext_delegate_t),
        pub set_state: unsafe extern "C" fn(
            windows: *const cef_wubble_ext_window_t,
            window_count: usize,
            tabs: *const cef_wubble_ext_tab_t,
            tab_count: usize,
        ),
        pub tab_opened: unsafe extern "C" fn(request_id: c_int, browser_id: c_int),
        pub action_click: unsafe extern "C" fn(
            extension_id: *const c_char,
            browser_id: c_int,
            popup_url: *mut *mut c_char,
        ) -> c_int,
        pub action_state: unsafe extern "C" fn(
            extension_id: *const c_char,
            browser_id: c_int,
            out: *mut cef_wubble_ext_action_t,
        ) -> c_int,
        pub free_string: unsafe extern "C" fn(string: *mut c_char),
        pub free_action: unsafe extern "C" fn(action: *mut cef_wubble_ext_action_t),
    }
}

/// One tab of the projection, by its view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtViewTab {
    pub view: ViewId,
    pub window: WindowId,
    pub index: u32,
    pub active: bool,
    pub pinned: bool,
}

/// What an extension asked of wubble's model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtRequest {
    /// Open `url` as a tab (tabs.create, runtime.openOptionsPage, clients.openWindow); answer
    /// with [`crate::app::Engine::answer_open_tab`]. `container` is the asking extension's
    /// (its Chrome profile's): requests from CEF's global profile, which runs a copy of each
    /// extension too, are refused before they get here, so one extension event opens one tab
    /// per container at most, each in its own profile. `window`: `None` is the current.
    OpenTab {
        request: i32,
        container: ContainerId,
        url: String,
        window: Option<WindowId>,
        active: bool,
    },
    CloseTab(ViewId),
    ActivateTab(ViewId),
    /// The extension's action changed (badge, title, icon, popup) for some tab.
    ActionChanged {
        extension: String,
    },
}

/// What the host did about an [`ExtRequest::OpenTab`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabOpened {
    /// A tab with this view.
    View(ViewId),
    /// A tab on a page wubble draws itself (`wubble://`): no view, so no tab object.
    Page,
    Refused,
}

/// What a click on an extension's action did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionClick {
    /// Nothing ran (no such action, hidden on the tab).
    None,
    /// `action.onClicked` or the blocked scripts ran.
    Clicked,
    /// The action has a popup: show this page.
    Popup(String),
}

/// An extension's action as one tab sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionState {
    pub title: String,
    pub badge: String,
    /// ARGB.
    pub badge_bg: u32,
    pub badge_fg: u32,
    pub enabled: bool,
    /// Empty: no popup.
    pub popup_url: String,
    /// The icon the extension set for the tab, as PNG.
    pub icon_png: Vec<u8>,
    /// The manifest's default icon file.
    pub icon_path: String,
}

/// A request as CEF reports it, by browser id.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Raw {
    OpenTab {
        request: i32,
        profile: String,
        url: String,
        window: i32,
        active: bool,
    },
    Close(i32),
    Activate(i32),
    Action(String),
}

/// Where the delegate's callbacks queue. Leaked once per process, so CEF can never call into
/// freed memory.
type Queue = Mutex<VecDeque<Raw>>;

/// The table, if this CEF has it, and the state pushed to it.
pub(crate) struct ExtApi {
    table: Option<&'static ffi::cef_wubble_ext_t>,
    queue: &'static Queue,
    windows: Vec<ExtWindow>,
    tabs: Vec<ExtViewTab>,
    /// What CEF was last sent; `None` before the first send.
    sent: Option<(
        Vec<ffi::cef_wubble_ext_window_t>,
        Vec<ffi::cef_wubble_ext_tab_t>,
    )>,
}

/// The table, checked: `None` when absent or of another shape.
fn checked(table: *const ffi::cef_wubble_ext_t) -> Option<&'static ffi::cef_wubble_ext_t> {
    // SAFETY: the pointer is the static table libcef exports, or null; only `size` and
    // `version` (the first two fields every version keeps) are read before they are checked.
    let t = unsafe { table.as_ref()? };
    (t.size == std::mem::size_of::<ffi::cef_wubble_ext_t>() && t.version == ffi::VERSION)
        .then_some(t)
}

/// `cef_wubble_ext_v1` among the symbols already loaded (libcef is linked by the binary).
#[cfg(target_os = "linux")]
fn lookup() -> Option<&'static ffi::cef_wubble_ext_t> {
    let this = libloading::os::unix::Library::this();
    // SAFETY: the symbol, when present, is `const cef_wubble_ext_t* cef_wubble_ext_v1(void)`.
    let get = unsafe {
        this.get::<unsafe extern "C" fn() -> *const ffi::cef_wubble_ext_t>(b"cef_wubble_ext_v1\0")
    }
    .ok()?;
    // SAFETY: as above; it returns a pointer to a static.
    checked(unsafe { get() })
}

/// The patch queue is built for Linux only so far (TODO.md); elsewhere CEF is stock.
#[cfg(not(target_os = "linux"))]
fn lookup() -> Option<&'static ffi::cef_wubble_ext_t> {
    None
}

/// The table, looked up once per process: the one owner of whether this engine has the
/// extension patches.
fn table() -> Option<&'static ffi::cef_wubble_ext_t> {
    static TABLE: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    let at = TABLE.get_or_init(|| lookup().map(|t| std::ptr::from_ref(t) as usize));
    // SAFETY: the address is of the static table `lookup` found and checked.
    at.map(|a| unsafe { &*(a as *const ffi::cef_wubble_ext_t) })
}

/// Whether this engine carries wubble's extension patches (`cef_wubble_ext_v1`). Without them an
/// extension would load but could not see wubble's tabs, so the browser offers no extensions:
/// none is loaded, the footer button, the Settings section and installing are gone, and the
/// installed list is kept as it is for an engine that has them. Known before CEF starts (libcef
/// is linked into the binary); [`crate::app::Engine::ext_apis`] gives the same answer.
pub fn available() -> bool {
    table().is_some()
}

impl ExtApi {
    /// The table of the running CEF, with wubble's delegate set. Call after CEF initialised.
    pub(crate) fn find() -> Self {
        let api = Self::with(table());
        if let Some(table) = api.table {
            let delegate = ffi::cef_wubble_ext_delegate_t {
                ctx: std::ptr::from_ref(api.queue).cast_mut().cast(),
                open_tab,
                close_tab,
                activate_tab,
                action_changed,
            };
            // SAFETY: CEF copies the delegate; its ctx outlives CEF (leaked).
            unsafe { (table.set_delegate)(&delegate) };
        }
        api
    }

    fn with(table: Option<&'static ffi::cef_wubble_ext_t>) -> Self {
        Self {
            table,
            queue: Box::leak(Box::default()),
            windows: vec![],
            tabs: vec![],
            sent: None,
        }
    }

    pub(crate) fn available(&self) -> bool {
        self.table.is_some()
    }

    /// Stops CEF asking: pending open requests fail. Called when the shutdown begins.
    pub(crate) fn release(&mut self) {
        if let Some(table) = self.table.take() {
            // SAFETY: null clears the delegate.
            unsafe { (table.set_delegate)(std::ptr::null()) };
        }
    }

    pub(crate) fn set_projection(&mut self, windows: Vec<ExtWindow>, tabs: Vec<ExtViewTab>) {
        self.windows = windows;
        self.tabs = tabs;
    }

    /// Sends the projection in browser ids if it differs from what CEF has; `browser` is a
    /// view's browser id while it is open.
    pub(crate) fn sync(&mut self, browser: impl Fn(ViewId) -> Option<i32>) {
        let Some(table) = self.table else { return };
        let Some(state) = translate(&self.windows, &self.tabs, browser, self.sent.as_ref()) else {
            return;
        };
        let (windows, tabs) = &state;
        // SAFETY: both slices live through the call; CEF copies them.
        unsafe { (table.set_state)(windows.as_ptr(), windows.len(), tabs.as_ptr(), tabs.len()) };
        self.sent = Some(state);
    }

    /// Answers open request `request` with the tab's browser (0: refused, -1: a tab on a page
    /// wubble draws itself, which has none).
    pub(crate) fn tab_opened(&self, request: i32, browser: i32) {
        if let Some(table) = self.table {
            // SAFETY: plain values.
            unsafe { (table.tab_opened)(request, browser) };
        }
    }

    pub(crate) fn action_click(&self, extension: &str, browser: i32) -> ActionClick {
        let (Some(table), Ok(id)) = (self.table, CString::new(extension)) else {
            return ActionClick::None;
        };
        let mut popup: *mut c_char = std::ptr::null_mut();
        // SAFETY: `id` lives through the call; CEF sets `popup` to a string we free below.
        let ran = unsafe { (table.action_click)(id.as_ptr(), browser, &mut popup) };
        let url = take_string(popup, table.free_string);
        match ran {
            1 => ActionClick::Clicked,
            2 => ActionClick::Popup(url),
            _ => ActionClick::None,
        }
    }

    pub(crate) fn action_state(&self, extension: &str, browser: i32) -> Option<ActionState> {
        let (table, id) = (self.table?, CString::new(extension).ok()?);
        let mut out = ffi::cef_wubble_ext_action_t {
            title: std::ptr::null_mut(),
            badge: std::ptr::null_mut(),
            badge_bg: 0,
            badge_fg: 0,
            enabled: 0,
            popup_url: std::ptr::null_mut(),
            icon_png: std::ptr::null_mut(),
            icon_png_len: 0,
            icon_path: std::ptr::null_mut(),
        };
        // SAFETY: `out` is a valid action for CEF to fill; freed by `take_action`.
        let found = unsafe { (table.action_state)(id.as_ptr(), browser, &mut out) };
        (found == 1).then(|| take_action(&mut out, table.free_action))
    }

    /// The requests CEF made since the last call, with browser ids turned into views
    /// (`view_of`) and profile paths into containers (under `root`); requests about a browser
    /// that is no view are dropped, open requests from a profile that is no container refused.
    pub(crate) fn take(
        &self,
        view_of: impl Fn(i32) -> Option<ViewId>,
        root: &Path,
    ) -> Vec<ExtRequest> {
        let raw: Vec<Raw> = self
            .queue
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default();
        raw.into_iter()
            .filter_map(|r| match r {
                Raw::OpenTab {
                    request,
                    profile,
                    url,
                    window,
                    active,
                } => match container_of(root, Path::new(&profile)) {
                    Some(container) => Some(ExtRequest::OpenTab {
                        request,
                        container,
                        url,
                        window: u32::try_from(window).ok().map(WindowId),
                        active,
                    }),
                    None => {
                        self.tab_opened(request, 0);
                        None
                    }
                },
                Raw::Close(b) => view_of(b).map(ExtRequest::CloseTab),
                Raw::Activate(b) => view_of(b).map(ExtRequest::ActivateTab),
                Raw::Action(extension) => Some(ExtRequest::ActionChanged { extension }),
            })
            .collect()
    }
}

/// The C state for `windows` and `tabs` (tabs whose view has no browser left out), or `None`
/// when it is what CEF already has.
fn translate(
    windows: &[ExtWindow],
    tabs: &[ExtViewTab],
    browser: impl Fn(ViewId) -> Option<i32>,
    sent: Option<&(
        Vec<ffi::cef_wubble_ext_window_t>,
        Vec<ffi::cef_wubble_ext_tab_t>,
    )>,
) -> Option<(
    Vec<ffi::cef_wubble_ext_window_t>,
    Vec<ffi::cef_wubble_ext_tab_t>,
)> {
    let id = |w: WindowId| c_int::try_from(w.0).unwrap_or(c_int::MAX);
    let w = windows
        .iter()
        .map(|w| ffi::cef_wubble_ext_window_t {
            window_id: id(w.id),
            focused: c_int::from(w.focused),
            type_: match w.kind {
                ExtWindowKind::Normal => 0,
                ExtWindowKind::Popup => 1,
            },
            ..Default::default()
        })
        .collect();
    let t = tabs
        .iter()
        .filter_map(|t| {
            Some(ffi::cef_wubble_ext_tab_t {
                browser_id: browser(t.view)?,
                window_id: id(t.window),
                index: c_int::try_from(t.index).unwrap_or(c_int::MAX),
                active: c_int::from(t.active),
                pinned: c_int::from(t.pinned),
            })
        })
        .collect();
    let state = (w, t);
    (sent != Some(&state)).then_some(state)
}

/// The container whose Chrome profile is at `profile` (`storage::container_dir` under `root`).
fn container_of(root: &Path, profile: &Path) -> Option<ContainerId> {
    if profile.parent() != Some(root) {
        return None;
    }
    let name = profile.file_name()?.to_str()?;
    crate::storage::container_of_dir_name(name)
}

/// Copies and frees a string CEF handed over.
fn take_string(s: *mut c_char, free: unsafe extern "C" fn(*mut c_char)) -> String {
    if s.is_null() {
        return String::new();
    }
    // SAFETY: CEF hands over a NUL-terminated string it allocated; freed once, here.
    let out = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
    // SAFETY: as above.
    unsafe { free(s) };
    out
}

/// Copies an action CEF filled, then frees it once.
fn take_action(
    a: &mut ffi::cef_wubble_ext_action_t,
    free: unsafe extern "C" fn(*mut ffi::cef_wubble_ext_action_t),
) -> ActionState {
    let text = |s: *mut c_char| {
        if s.is_null() {
            String::new()
        } else {
            // SAFETY: NUL-terminated strings CEF allocated; freed below with the action.
            unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
        }
    };
    let icon_png = if a.icon_png.is_null() {
        vec![]
    } else {
        // SAFETY: CEF allocated `icon_png_len` bytes there.
        unsafe { std::slice::from_raw_parts(a.icon_png, a.icon_png_len) }.to_vec()
    };
    let state = ActionState {
        title: text(a.title),
        badge: text(a.badge),
        badge_bg: a.badge_bg,
        badge_fg: a.badge_fg,
        enabled: a.enabled != 0,
        popup_url: text(a.popup_url),
        icon_png,
        icon_path: text(a.icon_path),
    };
    // SAFETY: frees what CEF allocated, once.
    unsafe { free(a) };
    state
}

fn push(ctx: *mut c_void, raw: Raw) {
    // SAFETY: `ctx` is the leaked queue `ExtApi::find` passed.
    let queue = unsafe { &*ctx.cast::<Queue>() };
    if let Ok(mut q) = queue.lock() {
        q.push_back(raw);
    }
}

fn text(s: *const c_char) -> String {
    if s.is_null() {
        return String::new();
    }
    // SAFETY: CEF passes NUL-terminated strings valid for the call.
    unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
}

unsafe extern "C" fn open_tab(
    ctx: *mut c_void,
    request: c_int,
    profile: *const c_char,
    url: *const c_char,
    window: c_int,
    active: c_int,
) {
    push(
        ctx,
        Raw::OpenTab {
            request,
            profile: text(profile),
            url: text(url),
            window,
            active: active != 0,
        },
    );
}

unsafe extern "C" fn close_tab(ctx: *mut c_void, browser: c_int) {
    push(ctx, Raw::Close(browser));
}

unsafe extern "C" fn activate_tab(ctx: *mut c_void, browser: c_int) {
    push(ctx, Raw::Activate(browser));
}

unsafe extern "C" fn action_changed(ctx: *mut c_void, extension: *const c_char) {
    push(ctx, Raw::Action(text(extension)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn view(n: usize) -> ViewId {
        ViewId::for_test(n)
    }

    fn window(id: u32, focused: bool) -> ExtWindow {
        ExtWindow {
            id: WindowId(id),
            focused,
            kind: ExtWindowKind::Normal,
        }
    }

    fn tab(v: usize, index: u32) -> ExtViewTab {
        ExtViewTab {
            view: view(v),
            window: WindowId(1),
            index,
            active: index == 0,
            pinned: false,
        }
    }

    #[test]
    fn translation_drops_views_without_a_browser_and_skips_an_unchanged_state() {
        let windows = [window(1, true)];
        let tabs = [tab(0, 0), tab(1, 1)];
        // View 1 has no browser (closed).
        let browser = |v: ViewId| (v == view(0)).then_some(40);
        let state = translate(&windows, &tabs, browser, None).unwrap();
        assert_eq!(
            state.0,
            [ffi::cef_wubble_ext_window_t {
                window_id: 1,
                focused: 1,
                ..Default::default()
            }]
        );
        assert_eq!(
            state.1,
            [ffi::cef_wubble_ext_tab_t {
                browser_id: 40,
                window_id: 1,
                index: 0,
                active: 1,
                pinned: 0,
            }]
        );
        assert_eq!(translate(&windows, &tabs, browser, Some(&state)), None);
        // The view's browser coming back is a change.
        assert!(translate(&windows, &tabs, |_| Some(40), Some(&state)).is_some());
    }

    static FREED: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn count_free(a: *mut ffi::cef_wubble_ext_action_t) {
        FREED.fetch_add(1, Ordering::SeqCst);
        // SAFETY: the test's own allocations, made below with CString/Vec.
        unsafe {
            let a = &mut *a;
            for s in [a.title, a.badge, a.popup_url, a.icon_path] {
                if !s.is_null() {
                    drop(CString::from_raw(s));
                }
            }
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                a.icon_png,
                a.icon_png_len,
            )));
        }
    }

    #[test]
    fn an_action_is_copied_then_freed_once() {
        let s = |v: &str| CString::new(v).unwrap().into_raw();
        let png: Box<[u8]> = vec![0x89u8, b'P', b'N', b'G'].into_boxed_slice();
        let png_len = png.len();
        let png = Box::into_raw(png).cast::<u8>();
        let mut raw = ffi::cef_wubble_ext_action_t {
            title: s("Smoke"),
            badge: s("7"),
            badge_bg: 0xff00_00ff,
            badge_fg: 0xffff_ffff,
            enabled: 1,
            popup_url: s(""),
            icon_png: png,
            icon_png_len: png_len,
            icon_path: std::ptr::null_mut(),
        };
        let before = FREED.load(Ordering::SeqCst);
        let state = take_action(&mut raw, count_free);
        assert_eq!(FREED.load(Ordering::SeqCst), before + 1);
        assert_eq!(
            state,
            ActionState {
                title: "Smoke".into(),
                badge: "7".into(),
                badge_bg: 0xff00_00ff,
                badge_fg: 0xffff_ffff,
                enabled: true,
                popup_url: String::new(),
                icon_png: vec![0x89, b'P', b'N', b'G'],
                icon_path: String::new(),
            }
        );
    }

    /// Every open answer the test table got, (request, browser).
    static ANSWERS: Mutex<Vec<(c_int, c_int)>> = Mutex::new(vec![]);

    /// A table of `size` and `version` that does nothing but record open answers.
    fn table(size: usize, version: c_int) -> &'static ffi::cef_wubble_ext_t {
        unsafe extern "C" fn set_delegate(_: *const ffi::cef_wubble_ext_delegate_t) {}
        unsafe extern "C" fn set_state(
            _: *const ffi::cef_wubble_ext_window_t,
            _: usize,
            _: *const ffi::cef_wubble_ext_tab_t,
            _: usize,
        ) {
        }
        unsafe extern "C" fn tab_opened(request: c_int, browser: c_int) {
            ANSWERS.lock().unwrap().push((request, browser));
        }
        unsafe extern "C" fn action_click(
            _: *const c_char,
            _: c_int,
            _: *mut *mut c_char,
        ) -> c_int {
            0
        }
        unsafe extern "C" fn action_state(
            _: *const c_char,
            _: c_int,
            _: *mut ffi::cef_wubble_ext_action_t,
        ) -> c_int {
            0
        }
        unsafe extern "C" fn free_string(_: *mut c_char) {}
        unsafe extern "C" fn free_action(_: *mut ffi::cef_wubble_ext_action_t) {}
        Box::leak(Box::new(ffi::cef_wubble_ext_t {
            size,
            version,
            set_delegate,
            set_state,
            tab_opened,
            action_click,
            action_state,
            free_string,
            free_action,
        }))
    }

    #[test]
    fn a_table_of_another_size_or_version_is_rejected() {
        let table = |size, version| std::ptr::from_ref(table(size, version));
        let size = std::mem::size_of::<ffi::cef_wubble_ext_t>();
        assert!(checked(table(size, ffi::VERSION)).is_some());
        assert!(checked(table(size - 8, ffi::VERSION)).is_none());
        assert!(checked(table(size, 2)).is_none());
        assert!(checked(std::ptr::null()).is_none());
    }

    #[test]
    fn a_profile_path_names_its_container() {
        let root = Path::new("/cache");
        assert_eq!(
            container_of(root, Path::new("/cache/container-3")),
            Some(ContainerId(3))
        );
        assert_eq!(container_of(root, Path::new("/cache")), None);
        assert_eq!(
            container_of(root, Path::new("/elsewhere/container-3")),
            None
        );
        assert_eq!(container_of(root, Path::new("/cache/container-x")), None);
    }

    #[test]
    fn requests_name_views_and_containers() {
        let api = ExtApi::with(None);
        let ctx = std::ptr::from_ref(api.queue).cast_mut().cast();
        let url = CString::new("https://example.com/").unwrap();
        let profile = CString::new("/cache/container-2").unwrap();
        let ext = CString::new("abc").unwrap();
        // SAFETY: the callbacks as CEF would call them, with live strings.
        unsafe {
            open_tab(ctx, 5, profile.as_ptr(), url.as_ptr(), -1, 0);
            close_tab(ctx, 40);
            activate_tab(ctx, 41);
            action_changed(ctx, ext.as_ptr());
        }
        let got = api.take(|b| (b == 40).then(|| view(0)), Path::new("/cache"));
        assert_eq!(
            got,
            [
                ExtRequest::OpenTab {
                    request: 5,
                    container: ContainerId(2),
                    url: "https://example.com/".into(),
                    window: None,
                    active: false,
                },
                ExtRequest::CloseTab(view(0)),
                ExtRequest::ActionChanged {
                    extension: "abc".into()
                },
            ]
        );
        assert!(api.take(|_| None, Path::new("/cache")).is_empty());
    }

    #[test]
    fn an_open_request_from_a_profile_that_is_no_container_is_refused() {
        let api = ExtApi::with(Some(table(
            std::mem::size_of::<ffi::cef_wubble_ext_t>(),
            ffi::VERSION,
        )));
        let ctx = std::ptr::from_ref(api.queue).cast_mut().cast();
        let url = CString::new("https://example.com/").unwrap();
        let global = CString::new("/cache").unwrap();
        // SAFETY: the callback as CEF would call it, with live strings.
        unsafe { open_tab(ctx, 77, global.as_ptr(), url.as_ptr(), -1, 1) };
        assert!(api.take(|_| None, Path::new("/cache")).is_empty());
        assert!(ANSWERS.lock().unwrap().contains(&(77, 0)));
    }
}
