//! The page's context menu: what it lists ([`entries`], one table of rows)
//! and its lifecycle ([`transition`]). Pure; the host draws [`Menu`] and
//! executes [`MenuEffect`]s.
//!
//! Which entries show for a link, an image, a selection, an editable field
//! or the bare page is Chromium's policy (`chrome/browser/renderer_context_menu/
//! render_view_context_menu.cc`: `AppendLinkItems`, `AppendImageItems`,
//! `AppendEditableItems`, `AppendCopyItem`, `AppendPageItems`; BSD-3-Clause)
//! and Firefox's (`browser/base/content/nsContextMenu.sys.mjs`:
//! `initOpenItems`, `initClipboardItems`; MPL-2.0), copied as the rows of
//! [`ROWS`], not as code. Command ids are CEF's `cef_menu_id_t`
//! (`include/internal/cef_types.h`, BSD-3-Clause): CEF runs those itself
//! when the answer names them.
//!
//! Every request is answered exactly once: choosing, dismissing and a newer
//! request each answer; CEF dropping the menu or the view going answer
//! nothing (welding already gave the default).
//!
//! A tab row's menu ([`tab_entries`]) runs through the same machine as a
//! menu the shell opened itself ([`Menu::Shell`]): nothing to answer.

use wubble_core::action::Action;
use wubble_core::highlight;
use wubble_core::keys::Keymap;
use wubble_core::model::{Command as ModelCommand, Model};
use wubble_core::omnibox;
use wubble_core::pagetheme::PageTheme;
use wubble_core::space::{ContainerId, Side, SpaceId, SpaceKind};
use wubble_core::tab::TabId;
use wubble_core::tabmenu::{self, Facts, Run};
use wubble_core::window::{Closing, Shown, WindowId, WindowKind, WindowTarget};
use wubble_engine::page::{ContextMenu, MediaKind, RequestId};

/// CEF's `cef_menu_id_t` values the menu uses.
pub mod cef_id {
    pub const BACK: i32 = 100;
    pub const FORWARD: i32 = 101;
    pub const RELOAD: i32 = 102;
    pub const UNDO: i32 = 110;
    pub const REDO: i32 = 111;
    pub const CUT: i32 = 112;
    pub const COPY: i32 = 113;
    pub const PASTE: i32 = 114;
    pub const DELETE: i32 = 116;
    pub const SELECT_ALL: i32 = 117;
    /// The first spelling suggestion; CEF numbers up to five from here.
    pub const SPELLCHECK_SUGGESTION_0: i32 = 200;
    pub const SPELLCHECK_SUGGESTION_LAST: i32 = 204;
    pub const ADD_TO_DICTIONARY: i32 = 206;
}

/// What a media entry does to the element under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaOp {
    /// Play if paused, else pause.
    PlayPause,
    /// Toggle muted.
    Mute,
    /// Toggle looping.
    Loop,
    /// Toggle the element's own controls.
    Controls,
}

/// A model command from the tab row's table (`wubble_core::tabmenu`). Those rows carry ids and
/// flags only, never a float, so their equality is total.
#[derive(Clone, Debug, PartialEq)]
pub struct TabCommand(pub ModelCommand);
impl Eq for TabCommand {}

/// What choosing an entry does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// A CEF menu command: the answer names it and CEF runs it.
    Engine(i32),
    /// Open the URL in a new tab next to the page's.
    OpenInNewTab(String),
    /// Open the URL in a new tab, then show that tab in a window of its own.
    OpenInNewWindow(String),
    /// Run a script in the frame CEF named (`View::run_script_in`).
    FrameScript { frame: String, script: &'static str },
    /// Open the URL in a new private window (`Command::NewPrivateWindow`).
    OpenInPrivateWindow(String),
    /// Open the URL in a peek over the page (`Command::OpenGlance`).
    OpenInGlance(String),
    /// Pop the video at this point (physical px, view-relative) out into wubble's
    /// picture-in-picture (`wubble_core::pip`).
    PictureInPicture { x: i32, y: i32 },
    /// The frame of the video at this point (physical px, view-relative) as a PNG, to the
    /// clipboard or the downloads folder (`output`).
    VideoFrame {
        x: i32,
        y: i32,
        output: wubble_core::shot::ShotOutput,
    },
    /// The OS's emoji and symbols picker over the focused field (macOS's Character Viewer,
    /// `orderFrontCharacterPalette:`).
    EmojiPalette,
    /// Put the text on the clipboard.
    CopyText(String),
    /// Download the URL with the page's cookies, asking where to save it.
    SaveAs(String),
    /// Search the default engine for the text.
    Search(String),
    /// Open DevTools on the element at this point (physical px, view-relative).
    Inspect { x: i32, y: i32 },
    /// Act on the media element at this point (physical px, view-relative), in the frame CEF
    /// named (`frame`, empty: the main frame), whose source is `source`.
    Media {
        op: MediaOp,
        x: i32,
        y: i32,
        frame: String,
        source: String,
    },
    /// Put the image at the URL on the clipboard (`View::download_image`).
    CopyImage(String),
    /// Paste the clipboard's text in the frame CEF named (empty: none named); not a CEF
    /// command, so the host can paste it where it carries the pages' clipboard.
    PastePlain(String),
    /// Put the tab beside the focused page (split view).
    SplitWith(TabId),
    /// Take the tab out of its split.
    Unsplit(TabId),
    /// Open the command bar to choose a page beside the focused one.
    OpenBeside,
    /// Run an action, as from a key.
    Perform(Action),
    /// A model command (the tab row menu's rows, `wubble_core::tabmenu`).
    Model(TabCommand),
    /// Flip this tab's mute.
    ToggleMute(TabId),
    /// Mute this tab's site, or unmute it (`sound.muted_sites`).
    ToggleMuteSite(TabId),
    /// Reload this tab's page.
    ReloadTab(TabId),
    /// Ask for a sidebar folder's new name.
    RenameFolder(wubble_core::space::FolderId),
    /// Block, or stop blocking, on this site (the blocker's exceptions).
    ToggleBlocker(String),
    /// Show the tab in another window, or a new one.
    MoveToWindow(TabId, WindowTarget),
    /// Open the pop-ups the page's tab held back.
    OpenBlockedPopups,
    /// Let the site open pop-ups from now on (`Permission::Popups`), and open the held ones.
    AllowPopups(String),
    /// The Little window's page becomes a tab of this Space; the window closes.
    PromoteLittle(WindowId, SpaceId),
    /// A Space's tabs never sleep (`true`), or may sleep again.
    SpaceNeverSleep(SpaceId, bool),
    /// The site joins the never-sleep list, or the listed entry leaves it (it is on the list
    /// exactly when the entry is already there).
    ToggleNeverSleepSite(String),
    /// Save a stream the page plays: the tab's detected `item`, its `choice`-th way to save
    /// (`wubble_core::media::select::choices`).
    SaveMedia { item: u32, choice: usize },
    /// Show this site's pages in this theme (dark mode for pages).
    SetPageTheme(String, PageTheme),
    /// Capture the element at this point (physical px, view-relative).
    ShotElement((i32, i32)),
    /// Turn this Boost on or off (its store id).
    ToggleBoost(i64),
    /// Open the Boost editor on this Boost.
    EditBoost(i64),
    /// Delete browsing data (`clear_entries`).
    ClearData(ClearData),
    /// Cancel the private downloads this close would end, then close (`close_entries`).
    CancelDownloadsAndClose(Closing),
    /// A question's "keep" answer: the menu closes, nothing else happens.
    Dismiss,
    /// Turn caret browsing on, and stop asking first when `dont_ask` (`caret_entries`).
    CaretBrowsing { dont_ask: bool },
    /// An edit of the command bar's field (`address_entries`).
    FieldEdit(FieldEdit),
    /// Load this clipboard text from the address field it was pasted in, as typed there.
    PasteAndGo(AddressField, String),
    /// Turn Always Show Full URLs on or off (`settings::UI_FULL_URLS`).
    SetFullUrls(bool),
    /// A zoom row of the zoom chip's popover: the action, then the popover again on the new
    /// percent (Chrome's zoom bubble stays while its buttons are pressed).
    ZoomStep(Action),
    /// A divider between groups; never chosen.
    Separator,
    /// A line of text (always disabled); never chosen.
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    pub command: Command,
    pub enabled: bool,
    /// An `IconPaths` name drawn before the label; empty for none.
    pub icon: &'static str,
    /// The key that does the same, as the keymap prints it; empty for none.
    pub shortcut: String,
    /// Destructive (closes something): drawn in the danger colour.
    pub danger: bool,
}

/// What the menu needs besides the click: the page's navigation (from the
/// view's `NavState`) and the search engine's name, if one is chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavFlags {
    pub back: bool,
    pub forward: bool,
    pub search: Option<&'static str>,
    /// The chosen engine's search by image URL (`omnibox::EngineInfo::image`).
    pub image: Option<&'static str>,
    /// The page is a private window's: "Open Link in Private Window" is not offered there
    /// (Chrome's incognito window has no "Open link in incognito window").
    pub private: bool,
    /// The page's zoom while it is not the default (`zoom::indicator`): the page menu offers
    /// to reset it.
    pub zoomed: Option<u32>,
}

/// The context a row's predicates read.
struct Ctx<'a> {
    menu: &'a ContextMenu,
    nav: NavFlags,
}

impl Ctx<'_> {
    fn link(&self) -> bool {
        !self.menu.link_url.is_empty()
    }
    fn image(&self) -> bool {
        self.menu.media == MediaKind::Image && !self.menu.media_url.is_empty()
    }
    /// A video or audio element.
    fn media(&self) -> bool {
        matches!(self.menu.media, MediaKind::Video | MediaKind::Audio)
    }
    fn media_url(&self) -> bool {
        self.media() && !self.menu.media_url.is_empty()
    }
    fn media_has(&self, flag: u32) -> bool {
        self.menu.media_flags.has(flag)
    }
    fn video(&self) -> bool {
        self.menu.media == MediaKind::Video
    }
    /// An image a search engine can fetch: its URL is on the web, and the engine has an image
    /// search.
    fn image_search(&self) -> bool {
        self.image()
            && self.nav.image.is_some()
            && wubble_core::url::is_recordable(&self.menu.media_url)
    }
    /// The `view-source:` URL of the frame clicked in, when that is a frame other than the
    /// page's own and one Chromium can show the source of (a web or file frame, not
    /// `about:srcdoc`).
    fn frame_source(&self) -> Option<String> {
        (self.menu.frame_url != self.menu.page_url)
            .then(|| wubble_core::url::view_source(&self.menu.frame_url))
            .flatten()
    }
    /// The selection is one web address (Chromium's "Go to <url>", which replaces the search
    /// row): the URL it names.
    fn selected_url(&self) -> Option<String> {
        match omnibox::classify(&self.menu.selection_text) {
            omnibox::Intent::Url(url)
                if url.starts_with("http://") || url.starts_with("https://") =>
            {
                Some(url)
            }
            _ => None,
        }
    }
    fn misspelled(&self) -> bool {
        self.menu.editable && !self.menu.misspelled_word.is_empty()
    }
    /// A selection outside a field on a page a text fragment can name, in its main frame: a
    /// fragment names text of the page's own document, never a frame's (Chrome offers none
    /// there).
    fn highlight(&self) -> Option<String> {
        (self.selection() && self.main_frame())
            .then(|| highlight::link_to_highlight(&self.menu.page_url, &self.menu.selection_text))
            .flatten()
    }
    fn editable(&self) -> bool {
        self.menu.editable
    }
    fn selection(&self) -> bool {
        !self.menu.editable && !self.menu.selection_text.is_empty()
    }
    /// Nothing more specific was hit: the page's own entries.
    fn page(&self) -> bool {
        !self.link() && !self.image() && !self.media() && !self.editable() && !self.selection()
    }
    fn edit(&self, flag: u32) -> bool {
        self.menu.edit.allows(flag)
    }
    fn at(&self) -> (i32, i32) {
        (self.menu.x, self.menu.y)
    }
    fn media_op(&self, op: MediaOp) -> Command {
        let (x, y) = self.at();
        Command::Media {
            op,
            x,
            y,
            frame: self.menu.frame.clone(),
            source: self.menu.media_url.clone(),
        }
    }
    /// Clicked in the page's own document, not a frame's.
    fn main_frame(&self) -> bool {
        self.menu.frame_url.is_empty() || self.menu.frame_url == self.menu.page_url
    }
    /// A video in the page's own document: the kit (`kit.js`) reaches it at the point.
    fn main_video(&self) -> bool {
        self.video() && self.main_frame()
    }
    /// "Video" or "Audio", for the media entries' labels.
    fn kind(&self) -> &'static str {
        if self.video() { "Video" } else { "Audio" }
    }
}

/// Chromium's `kMaxSelectionTextLength` for the search entry's label is 50
/// characters; longer selections end in an ellipsis.
const SEARCH_LABEL_CHARS: usize = 50;

/// `Search <engine> for “<selection>”`, the selection shortened and on one line.
pub fn search_label(engine: Option<&str>, text: &str) -> String {
    format!(
        "Search {} for \u{201c}{}\u{201d}",
        engine.unwrap_or("the Web"),
        short_text(text)
    )
}

/// `text` on one line, cut to [`SEARCH_LABEL_CHARS`] with an ellipsis.
fn short_text(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = flat.chars().take(SEARCH_LABEL_CHARS).collect();
    let more = if flat.chars().count() > SEARCH_LABEL_CHARS {
        "\u{2026}"
    } else {
        ""
    };
    format!("{short}{more}")
}

/// How many characters of a host a menu row names (QA CRIT-05, HIGH-03): a longer one is cut in
/// its middle, so the row keeps both the start and the domain and stays on one line.
const MENU_HOST_CHARS: usize = 20;

/// A host as a menu row names it: whole when short, else cut in the middle (`elide_middle`).
pub fn menu_host(host: &str) -> std::borrow::Cow<'_, str> {
    wubble_core::url::elide_middle(host, MENU_HOST_CHARS)
}

/// The script that does `op` to the video or audio element the menu was opened on, run in
/// the frame that holds it (`View::run_script_in`). CEF has no media-player API, so this is
/// Chromium's `MediaPlayerAction` done from the page's side. In the main frame the element is
/// the one at `x, y` (physical px, view-relative: CSS px times `devicePixelRatio`, the device
/// scale times the zoom); in a subframe the point is the view's, not the frame's, and CEF does
/// not say where the frame sits, so the element is the one playing `source` (the menu's media
/// URL, its `currentSrc`), or the frame's only one. Twins playing the same source in one frame
/// get the first.
pub fn media_script(op: MediaOp, x: i32, y: i32, source: &str) -> String {
    let act = match op {
        MediaOp::PlayPause => "m.paused ? m.play() : m.pause()",
        MediaOp::Mute => "m.muted = !m.muted",
        MediaOp::Loop => "m.loop = !m.loop",
        MediaOp::Controls => "m.controls = !m.controls",
    };
    let source = serde_json::to_string(source).unwrap_or_else(|_| "\"\"".into());
    format!(
        "(() => {{ const all = [...document.querySelectorAll('video, audio')]; \
         const e = window === window.top \
         ? document.elementFromPoint({x} / devicePixelRatio, {y} / devicePixelRatio) : null; \
         const m = (e && e.closest('video, audio')) || all.find(m => m.currentSrc === {source}) \
         || (all.length === 1 ? all[0] : null); if (m) {{ {act}; }} }})()"
    )
}

/// One row of the table. `group` separates entries: a divider goes between
/// two shown groups.
struct Row {
    group: u8,
    label: fn(&Ctx) -> String,
    show: fn(&Ctx) -> bool,
    enabled: fn(&Ctx) -> bool,
    command: fn(&Ctx) -> Command,
}

/// The rows only macOS has (the Character Viewer, Writing Direction).
const MAC: bool = cfg!(target_os = "macos");

const fn always(_: &Ctx) -> bool {
    true
}

use wubble_engine::page::EditFlags as F;
use wubble_engine::page::MediaFlags as M;

/// A row with a fixed label.
macro_rules! row {
    ($group:literal, $label:literal, $show:expr, $enabled:expr, $command:expr) => {
        Row {
            group: $group,
            label: |_| $label.into(),
            show: $show,
            enabled: $enabled,
            command: $command,
        }
    };
}

/// Every fixed entry, in menu order (Chromium's `AppendLinkItems`,
/// `AppendImageItems`, `AppendMediaItems`, `AppendEditableItems`,
/// `AppendCopyItem`, `AppendSearchProvider`, `AppendPageItems`,
/// `AppendDeveloperItems`, and the frame rows of `AppendFrameItems`). Spelling suggestions go
/// first and come from the request itself. Picture in Picture is wubble's own
/// (`wubble_core::pip`): CEF's windowless views have none of Chromium's.
///
/// A group number only has to differ from its neighbours' (a divider goes where it changes):
/// the spelling rows lead the table in groups 0 and 11 so "Add to Dictionary" sits under the
/// suggestions and above the link rows.
const ROWS: &[Row] = &[
    row!(
        0,
        "No Spelling Suggestions",
        |c| c.misspelled() && c.menu.suggestions.is_empty(),
        |_| false,
        |_| Command::Info
    ),
    // CEF runs it itself: `AddWordToDictionary` for the request's misspelled word.
    row!(11, "Add to Dictionary", |c| c.misspelled(), always, |_| {
        Command::Engine(cef_id::ADD_TO_DICTIONARY)
    }),
    row!(1, "Open Link in New Tab", |c| c.link(), always, |c| {
        Command::OpenInNewTab(c.menu.link_url.clone())
    }),
    row!(1, "Open Link in New Window", |c| c.link(), always, |c| {
        Command::OpenInNewWindow(c.menu.link_url.clone())
    }),
    // Chromium's IDC_CONTENT_CONTEXT_OPENLINKOFFTHERECORD, left out in an incognito window
    row!(
        1,
        "Open Link in Private Window",
        |c| c.link() && !c.nav.private,
        always,
        |c| { Command::OpenInPrivateWindow(c.menu.link_url.clone()) }
    ),
    // Chrome's "Open link in split view": a new tab beside the page
    row!(1, "Open Link in Split View", |c| c.link(), always, |c| {
        Command::Model(TabCommand(ModelCommand::OpenBeside {
            url: c.menu.link_url.clone(),
        }))
    }),
    // wubble's peek, where Arc and Zen offer theirs
    row!(1, "Open Link in a Peek", |c| c.link(), always, |c| {
        Command::OpenInGlance(c.menu.link_url.clone())
    }),
    row!(1, "Save Link As\u{2026}", |c| c.link(), always, |c| {
        Command::SaveAs(c.menu.link_url.clone())
    }),
    row!(1, "Copy Link Address", |c| c.link(), always, |c| {
        Command::CopyText(c.menu.link_url.clone())
    }),
    row!(2, "Open Image in New Tab", |c| c.image(), always, |c| {
        Command::OpenInNewTab(c.menu.media_url.clone())
    }),
    row!(2, "Save Image As\u{2026}", |c| c.image(), always, |c| {
        Command::SaveAs(c.menu.media_url.clone())
    }),
    // Chromium shows it only for an image that has loaded (`has_image_contents`).
    row!(
        2,
        "Copy Image",
        |c| c.image() && c.menu.has_image,
        always,
        |c| { Command::CopyImage(c.menu.media_url.clone()) }
    ),
    row!(2, "Copy Image Address", |c| c.image(), always, |c| {
        Command::CopyText(c.menu.media_url.clone())
    }),
    Row {
        group: 2,
        label: |c| format!("Search {} for Image", c.nav.search.unwrap_or_default()),
        show: |c| c.image_search(),
        enabled: always,
        command: |c| {
            Command::OpenInNewTab(omnibox::fill(
                c.nav.image.unwrap_or_default(),
                &c.menu.media_url,
            ))
        },
    },
    Row {
        group: 3,
        label: |c| {
            if c.media_has(M::PAUSED) {
                "Play"
            } else {
                "Pause"
            }
            .into()
        },
        show: |c| c.media(),
        enabled: always,
        command: |c| c.media_op(MediaOp::PlayPause),
    },
    Row {
        group: 3,
        label: |c| {
            if c.media_has(M::MUTED) {
                "Unmute"
            } else {
                "Mute"
            }
            .into()
        },
        show: |c| c.media() && c.media_has(M::HAS_AUDIO),
        enabled: always,
        command: |c| c.media_op(MediaOp::Mute),
    },
    Row {
        group: 3,
        label: |c| {
            if c.media_has(M::LOOP) {
                "Stop Looping"
            } else {
                "Loop"
            }
            .into()
        },
        show: |c| c.media(),
        enabled: always,
        command: |c| c.media_op(MediaOp::Loop),
    },
    Row {
        group: 3,
        label: |c| {
            if c.media_has(M::CONTROLS) {
                "Hide Controls"
            } else {
                "Show Controls"
            }
            .into()
        },
        show: |c| c.media(),
        enabled: |c| c.media_has(M::CAN_TOGGLE_CONTROLS),
        command: |c| c.media_op(MediaOp::Controls),
    },
    // Chromium's IDC_CONTENT_CONTEXT_PICTUREINPICTURE: wubble's own (the video fullscreen in its
    // view, the view in a small window), on the video under the pointer
    row!(3, "Picture in Picture", |c| c.main_video(), always, |c| {
        let (x, y) = c.at();
        Command::PictureInPicture { x, y }
    }),
    Row {
        group: 4,
        label: |c| format!("Open {} in New Tab", c.kind()),
        show: |c| c.media_url(),
        enabled: always,
        command: |c| Command::OpenInNewTab(c.menu.media_url.clone()),
    },
    Row {
        group: 4,
        label: |c| format!("Save {} As\u{2026}", c.kind()),
        show: |c| c.media_url(),
        enabled: |c| c.media_has(M::CAN_SAVE),
        command: |c| Command::SaveAs(c.menu.media_url.clone()),
    },
    Row {
        group: 4,
        label: |c| format!("Copy {} Address", c.kind()),
        show: |c| c.media_url(),
        enabled: always,
        command: |c| Command::CopyText(c.menu.media_url.clone()),
    },
    // Chromium M120's IDC_CONTENT_CONTEXT_COPYVIDEOFRAME and _SAVEVIDEOFRAMEAS: the frame on
    // screen, the downloads folder for the saved one
    row!(4, "Copy Video Frame", |c| c.main_video(), always, |c| {
        let (x, y) = c.at();
        Command::VideoFrame {
            x,
            y,
            output: wubble_core::shot::ShotOutput::Clipboard,
        }
    }),
    row!(
        4,
        "Save Video Frame As\u{2026}",
        |c| c.main_video(),
        always,
        |c| {
            let (x, y) = c.at();
            Command::VideoFrame {
                x,
                y,
                output: wubble_core::shot::ShotOutput::File,
            }
        }
    ),
    // Chromium's IDC_CONTENT_CONTEXT_EMOJI, first of a field's rows (macOS's Character Viewer;
    // Windows, where Chrome also has it, is parked)
    row!(
        12,
        "Emoji & Symbols",
        |c| c.editable() && MAC,
        always,
        |_| Command::EmojiPalette
    ),
    row!(5, "Undo", |c| c.editable(), |c| c.edit(F::CAN_UNDO), |_| {
        Command::Engine(cef_id::UNDO)
    }),
    row!(5, "Redo", |c| c.editable(), |c| c.edit(F::CAN_REDO), |_| {
        Command::Engine(cef_id::REDO)
    }),
    row!(6, "Cut", |c| c.editable(), |c| c.edit(F::CAN_CUT), |_| {
        Command::Engine(cef_id::CUT)
    }),
    row!(
        6,
        "Copy",
        |c| c.editable() || c.selection(),
        |c| c.selection() || c.edit(F::CAN_COPY),
        |_| { Command::Engine(cef_id::COPY) }
    ),
    row!(
        6,
        "Copy Link to Highlight",
        |c| c.highlight().is_some(),
        always,
        |c| { Command::CopyText(c.highlight().unwrap_or_default()) }
    ),
    row!(
        6,
        "Paste",
        |c| c.editable(),
        |c| c.edit(F::CAN_PASTE),
        |_| { Command::Engine(cef_id::PASTE) }
    ),
    row!(
        6,
        "Paste as Plain Text",
        |c| c.editable(),
        |c| c.edit(F::CAN_PASTE),
        |c| { Command::PastePlain(c.menu.frame.clone()) }
    ),
    row!(
        6,
        "Delete",
        |c| c.editable(),
        |c| c.edit(F::CAN_DELETE),
        |_| { Command::Engine(cef_id::DELETE) }
    ),
    row!(
        6,
        "Select All",
        |c| c.editable(),
        |c| c.edit(F::CAN_SELECT_ALL),
        |_| { Command::Engine(cef_id::SELECT_ALL) }
    ),
    // macOS's Writing Direction submenu (Chromium `AppendPlatformEditableItems`), as rows: the
    // focused field's `dir`, as Chromium's `UpdateTextDirection` sets it
    row!(
        13,
        "Writing Direction: Default",
        |c| c.editable() && MAC,
        always,
        |c| Command::FrameScript {
            frame: c.menu.frame.clone(),
            script: "document.activeElement && document.activeElement.removeAttribute('dir')",
        }
    ),
    row!(
        13,
        "Writing Direction: Left to Right",
        |c| c.editable() && MAC,
        always,
        |c| Command::FrameScript {
            frame: c.menu.frame.clone(),
            script: "document.activeElement && (document.activeElement.dir = 'ltr')",
        }
    ),
    row!(
        13,
        "Writing Direction: Right to Left",
        |c| c.editable() && MAC,
        always,
        |c| Command::FrameScript {
            frame: c.menu.frame.clone(),
            script: "document.activeElement && (document.activeElement.dir = 'rtl')",
        }
    ),
    Row {
        group: 7,
        label: |c| {
            format!(
                "Go to {}",
                short_text(&c.selected_url().unwrap_or_default())
            )
        },
        show: |c| c.selected_url().is_some(),
        enabled: always,
        command: |c| Command::OpenInNewTab(c.selected_url().unwrap_or_default()),
    },
    Row {
        group: 7,
        label: |c| search_label(c.nav.search, &c.menu.selection_text),
        show: |c| !c.menu.selection_text.trim().is_empty() && c.selected_url().is_none(),
        enabled: always,
        command: |c| Command::Search(c.menu.selection_text.clone()),
    },
    row!(7, "Print\u{2026}", |c| c.selection(), always, |_| {
        Command::Perform(Action::Print)
    }),
    row!(
        8,
        "Back",
        |c| c.page(),
        |c| c.nav.back,
        |_| Command::Engine(cef_id::BACK)
    ),
    row!(8, "Forward", |c| c.page(), |c| c.nav.forward, |_| {
        Command::Engine(cef_id::FORWARD)
    }),
    row!(8, "Reload", |c| c.page(), always, |_| Command::Engine(
        cef_id::RELOAD
    )),
    row!(9, "Save Page As\u{2026}", |c| c.page(), always, |c| {
        Command::SaveAs(c.menu.page_url.clone())
    }),
    row!(9, "Print\u{2026}", |c| c.page(), always, |_| {
        Command::Perform(Action::Print)
    }),
    // wubble: the zoom chip's Reset, where Chrome's app menu has its Zoom row
    Row {
        group: 9,
        label: |c| format!("Reset Zoom ({}%)", c.nav.zoomed.unwrap_or_default()),
        show: |c| c.page() && c.nav.zoomed.is_some(),
        enabled: always,
        command: |_| Command::Perform(Action::ZoomReset),
    },
    row!(10, "View Page Source", |c| c.page(), always, |_| {
        Command::Perform(Action::ViewSource)
    }),
    row!(
        10,
        "View Frame Source",
        |c| c.frame_source().is_some(),
        always,
        |c| Command::OpenInNewTab(c.frame_source().unwrap_or_default())
    ),
    row!(
        10,
        "Reload Frame",
        |c| c.frame_source().is_some(),
        always,
        |c| {
            Command::FrameScript {
                frame: c.menu.frame.clone(),
                script: "location.reload()",
            }
        }
    ),
    row!(
        10,
        "Open Frame in New Tab",
        |c| c.frame_source().is_some(),
        always,
        |c| Command::OpenInNewTab(c.menu.frame_url.clone())
    ),
    // wubble's screenshots: the element under the pointer (an image, a link, a block).
    row!(
        10,
        "Take Screenshot of Element",
        |c| !c.editable(),
        always,
        |c| { Command::ShotElement(c.at()) }
    ),
    row!(10, "Inspect", always, always, |c| {
        let (x, y) = c.at();
        Command::Inspect { x, y }
    }),
];

/// The entries for one right-click, dividers included.
pub fn entries(menu: &ContextMenu, nav: NavFlags) -> Vec<Entry> {
    let ctx = Ctx { menu, nav };
    // Group 0: CEF's spelling suggestions for a misspelled word in a field.
    let suggestions = ctx
        .misspelled()
        .then_some(menu.suggestions.iter())
        .into_iter()
        .flatten()
        .zip(cef_id::SPELLCHECK_SUGGESTION_0..=cef_id::SPELLCHECK_SUGGESTION_LAST)
        .map(|(word, id)| {
            (
                0,
                Entry {
                    label: word.clone(),
                    command: Command::Engine(id),
                    enabled: true,
                    icon: "",
                    shortcut: String::new(),
                    danger: false,
                },
            )
        });
    let fixed = ROWS.iter().filter(|r| (r.show)(&ctx)).map(|r| {
        (
            r.group,
            Entry {
                label: (r.label)(&ctx),
                command: (r.command)(&ctx),
                enabled: (r.enabled)(&ctx),
                icon: "",
                shortcut: String::new(),
                danger: false,
            },
        )
    });
    grouped(suggestions.chain(fixed))
}

/// Entries in table order with a divider wherever the group changes.
fn grouped(rows: impl Iterator<Item = (u8, Entry)>) -> Vec<Entry> {
    let mut out: Vec<Entry> = vec![];
    let mut last = None;
    for (group, entry) in rows {
        if last.is_some_and(|g| g != group) {
            out.push(Entry {
                label: String::new(),
                command: Command::Separator,
                enabled: false,
                icon: "",
                shortcut: String::new(),
                danger: false,
            });
        }
        last = Some(group);
        out.push(entry);
    }
    out
}

/// Which address field a menu is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressField {
    /// The command bar's field: its text rows act on what is typed.
    Bar,
    /// The address pill: the page's address.
    Pill,
}

/// An edit of the command bar's field from its menu. All but Delete are the field's own
/// (`FieldEdit` in model.slint); Delete is the host's edit of the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldEdit {
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
}

/// What the address menu needs to know when it opens.
#[derive(Clone, Copy, Debug)]
pub struct FieldFacts<'a> {
    pub field: AddressField,
    /// Text is selected in the bar's field.
    pub selected: bool,
    /// The bar's field holds text.
    pub text: bool,
    /// The clipboard's text; empty when it holds none (or cannot be read here).
    pub clipboard: &'a str,
    /// Always Show Full URLs is on.
    pub full_urls: bool,
}

impl FieldFacts<'_> {
    fn bar(&self) -> bool {
        self.field == AddressField::Bar
    }
    fn pill(&self) -> bool {
        self.field == AddressField::Pill
    }
    fn pasteable(&self) -> bool {
        !self.clipboard.trim().is_empty()
    }
}

/// One row of the address menu: its group (a divider between groups), when it shows and is
/// enabled, its label, what it runs and the action whose key does the same.
struct FieldRow {
    group: u8,
    show: fn(&FieldFacts) -> bool,
    enabled: fn(&FieldFacts) -> bool,
    label: fn(&FieldFacts) -> String,
    command: fn(&FieldFacts) -> Command,
    key: Option<Action>,
}

/// The address field's menu, in order: Chromium's omnibox menu
/// (`chrome/browser/ui/views/omnibox/omnibox_view_views.cc` `UpdateContextMenu`, BSD-3-Clause;
/// rows and labels only). The bar's field has the text rows and Paste and Go; the pill has
/// Copy and Edit Address and Paste and Go; both end in Always Show Full URLs. Undo is left out:
/// Slint's field has no undo the host can run.
const FIELD_ROWS: &[FieldRow] = &[
    FieldRow {
        group: 0,
        show: |f| f.bar(),
        enabled: |f| f.selected,
        label: |_| "Cut".into(),
        command: |_| Command::FieldEdit(FieldEdit::Cut),
        key: Some(Action::Cut),
    },
    FieldRow {
        group: 0,
        show: |f| f.bar(),
        enabled: |f| f.selected,
        label: |_| "Copy".into(),
        command: |_| Command::FieldEdit(FieldEdit::Copy),
        key: Some(Action::Copy),
    },
    FieldRow {
        group: 0,
        show: |f| f.bar(),
        enabled: |f| f.pasteable(),
        label: |_| "Paste".into(),
        command: |_| Command::FieldEdit(FieldEdit::Paste),
        key: Some(Action::Paste),
    },
    FieldRow {
        group: 0,
        show: |_| true,
        enabled: |f| f.pasteable(),
        label: |f| paste_label(f.clipboard),
        command: |f| Command::PasteAndGo(f.field, f.clipboard.trim().to_owned()),
        key: None,
    },
    FieldRow {
        group: 0,
        show: |f| f.bar(),
        enabled: |f| f.selected,
        label: |_| "Delete".into(),
        command: |_| Command::FieldEdit(FieldEdit::Delete),
        key: None,
    },
    FieldRow {
        group: 1,
        show: |f| f.bar(),
        enabled: |f| f.text,
        label: |_| "Select All".into(),
        command: |_| Command::FieldEdit(FieldEdit::SelectAll),
        key: Some(Action::SelectAll),
    },
    FieldRow {
        group: 1,
        show: |f| f.pill(),
        enabled: |_| true,
        label: |_| "Copy Address".into(),
        command: |_| Command::Perform(Action::CopyUrl),
        key: Some(Action::CopyUrl),
    },
    FieldRow {
        group: 1,
        show: |f| f.pill(),
        enabled: |_| true,
        label: |_| "Edit Address".into(),
        command: |_| Command::Perform(Action::FocusAddress),
        key: Some(Action::FocusAddress),
    },
    FieldRow {
        group: 2,
        show: |_| true,
        enabled: |_| true,
        label: |_| "Always Show Full URLs".into(),
        command: |f| Command::SetFullUrls(!f.full_urls),
        key: None,
    },
];

/// The key that runs `action`, as the keymap prints it; empty for none.
fn key_text(keymap: &Keymap, action: Action) -> String {
    keymap
        .shortcut_for(action)
        .map(|c| c.to_string())
        .unwrap_or_default()
}

/// The reload button's menu (its right click): Chrome's `ReloadButton` menu
/// (`chrome/browser/ui/views/toolbar/reload_button.cc`, `IDS_RELOAD_MENU_*`, BSD-3-Clause; rows
/// and labels only), Normal and Hard Reload, and Empty Cache and Hard Reload while DevTools
/// inspects the tab (`devtools`), each with its key.
pub fn reload_entries(devtools: bool, keymap: &Keymap) -> Vec<Entry> {
    [
        (Action::Reload, "Normal Reload"),
        (Action::HardReload, "Hard Reload"),
        (Action::EmptyCacheHardReload, "Empty Cache and Hard Reload"),
    ]
    .into_iter()
    .filter(|&(action, _)| devtools || action != Action::EmptyCacheHardReload)
    .map(|(action, label)| Entry {
        label: label.into(),
        command: Command::Perform(action),
        enabled: true,
        icon: "",
        shortcut: key_text(keymap, action),
        danger: false,
    })
    .collect()
}

/// The zoom chip's popover for a page at `percent` (Chrome's omnibox zoom bubble,
/// `chrome/browser/ui/views/location_bar/zoom_bubble_view.cc`, BSD-3-Clause; rows only): the
/// percent, then −, + and Reset to the user's `default`, each with its key.
pub fn zoom_entries(percent: u32, default: u32, keymap: &Keymap) -> Vec<Entry> {
    let row = |label: String, action| Entry {
        label,
        command: Command::ZoomStep(action),
        enabled: true,
        icon: "",
        shortcut: key_text(keymap, action),
        danger: false,
    };
    vec![
        Entry {
            label: format!("Zoom {percent}%"),
            command: Command::Info,
            enabled: false,
            icon: "",
            shortcut: String::new(),
            danger: false,
        },
        row("Zoom Out".into(), Action::ZoomOut),
        row("Zoom In".into(), Action::ZoomIn),
        row(format!("Reset to {default}%"), Action::ZoomReset),
    ]
}

/// The address field's menu (`FIELD_ROWS`), each row's shortcut from the keymap.
pub fn address_entries(facts: FieldFacts, keymap: &Keymap) -> Vec<Entry> {
    let rows = FIELD_ROWS.iter().filter(|r| (r.show)(&facts)).map(|r| {
        let command = (r.command)(&facts);
        let on = command == Command::SetFullUrls(false);
        let entry = Entry {
            label: (r.label)(&facts),
            command,
            enabled: (r.enabled)(&facts),
            // the switch's state, as a check
            icon: if on { "check" } else { "" },
            shortcut: r.key.map(|a| key_text(keymap, a)).unwrap_or_default(),
            danger: false,
        };
        (r.group, entry)
    });
    grouped(rows)
}

/// The paste row's label, from what the clipboard's text would do as typed
/// (`omnibox::classify`): Chromium's `IDS_PASTE_AND_GO` "Paste and go to <url>" and
/// `IDS_PASTE_AND_SEARCH` "Paste and search for “<text>”", the text cut as the page menu's
/// search row; plain "Paste and Go" when the clipboard holds no text.
pub fn paste_label(clipboard: &str) -> String {
    if clipboard.trim().is_empty() {
        return "Paste and Go".into();
    }
    match omnibox::classify(clipboard) {
        omnibox::Intent::Url(url) => format!(
            "Paste and Go to {}",
            wubble_core::url::elide_middle(&url, SEARCH_LABEL_CHARS)
        ),
        omnibox::Intent::Search(q) => {
            format!("Paste and Search for \u{201c}{}\u{201d}", short_text(&q))
        }
    }
}

/// The tab row's menu: the rows of `wubble_core::tabmenu::ROWS` that apply to `tab`, in table
/// order, with the split entries in the table's split slot and each row's shortcut from the
/// keymap where an action does the same.
pub fn tab_entries(model: &Model, tab: TabId, facts: Facts, keymap: &Keymap) -> Vec<Entry> {
    let key = |a: Action| key_text(keymap, a);
    let visible = model.visible_tab() == Some(tab);
    let mut out = vec![];
    for e in tabmenu::entries(model, tab, facts) {
        let (command, shortcut) = match e.run {
            Run::SplitEntries(t) => {
                out.extend(split_entries(model, t));
                continue;
            }
            Run::WindowEntries(t) => {
                out.extend(window_entries(model, t));
                continue;
            }
            Run::SleepEntries(t) => {
                out.extend(sleep_entries(model, t));
                continue;
            }
            Run::Separator => (Command::Separator, String::new()),
            Run::Reload(t) => (
                Command::ReloadTab(t),
                if visible {
                    key(Action::Reload)
                } else {
                    String::new()
                },
            ),
            Run::Copy(text) => (Command::CopyText(text), String::new()),
            Run::RenameFolder(f) => (Command::RenameFolder(f), String::new()),
            Run::ToggleMute(t) => (
                Command::ToggleMute(t),
                if visible {
                    key(Action::ToggleMute)
                } else {
                    String::new()
                },
            ),
            Run::ToggleMuteSite(t) => (Command::ToggleMuteSite(t), String::new()),
            Run::Model(c) => {
                let shortcut = match &c {
                    ModelCommand::CloseTab(_) if visible => key(Action::CloseTab),
                    ModelCommand::PinTab(_) | ModelCommand::UnpinTab(_) if visible => {
                        key(Action::TogglePinTab)
                    }
                    _ => String::new(),
                };
                (Command::Model(TabCommand(c)), shortcut)
            }
        };
        let separator = command == Command::Separator;
        out.push(Entry {
            label: e.label,
            command,
            enabled: !separator,
            icon: e.icon,
            shortcut,
            danger: e.danger,
        });
    }
    // the split slot may have been empty: no doubled divider
    out.dedup_by(|a, b| a.command == Command::Separator && b.command == Command::Separator);
    out
}

/// The sidebar's own menu, a right click on its empty glass (QA LOW-02): what the sidebar
/// already offers, a tab, a Space and a folder in the active Space, with their keys.
pub fn sidebar_entries(model: &Model, keymap: &Keymap) -> Vec<Entry> {
    let key = |a: Action| key_text(keymap, a);
    let entry = |label: &str, command, shortcut| Entry {
        label: label.into(),
        command,
        enabled: true,
        icon: "",
        shortcut,
        danger: false,
    };
    vec![
        entry(
            "New Tab",
            Command::Perform(Action::NewTab),
            key(Action::NewTab),
        ),
        entry(
            "New Space",
            Command::Perform(Action::NewSpace),
            key(Action::NewSpace),
        ),
        entry(
            "New Folder",
            Command::Model(TabCommand(ModelCommand::NewFolder {
                space: model.active_space(),
                name: "New Folder".into(),
            })),
            String::new(),
        ),
    ]
}

/// A folder header's menu (`tabmenu::folder_entries`).
pub fn folder_entries(model: &Model, folder: wubble_core::space::FolderId) -> Vec<Entry> {
    tabmenu::folder_entries(model, folder)
        .into_iter()
        .map(|e| Entry {
            label: e.label,
            command: match e.run {
                Run::Model(c) => Command::Model(TabCommand(c)),
                Run::RenameFolder(f) => Command::RenameFolder(f),
                _ => Command::Separator,
            },
            enabled: true,
            icon: e.icon,
            shortcut: String::new(),
            danger: e.danger,
        })
        .collect()
}

/// The tab row's never-sleep switches: its Space, and its site (when it has one). A site that
/// is on the list only through a parent entry (`example.com` for `docs.example.com`) offers to
/// take that entry off.
pub fn sleep_entries(model: &Model, tab: TabId) -> Vec<Entry> {
    let entry = |label: String, command| Entry {
        label,
        command,
        enabled: true,
        icon: "awake",
        shortcut: String::new(),
        danger: false,
    };
    let mut out = vec![];
    if let Some(space) = model
        .location(tab)
        .and_then(|(s, _)| model.spaces.iter().find(|k| k.id == s))
    {
        out.push(if space.never_sleep {
            entry(
                "Let Tabs in This Space Sleep".into(),
                Command::SpaceNeverSleep(space.id, false),
            )
        } else {
            entry(
                "Never Sleep Tabs in This Space".into(),
                Command::SpaceNeverSleep(space.id, true),
            )
        });
    }
    if let Some(host) = model
        .tabs
        .get(&tab)
        .and_then(|t| wubble_core::routing::host_key(&t.url))
        .filter(|_| {
            // a web page: internal pages have no site to keep awake
            model
                .tabs
                .get(&tab)
                .is_some_and(|t| !model.is_internal(t.id))
        })
    {
        let listed = model
            .never_sites(model.side_of_tab(tab))
            .iter()
            .find(|s| wubble_core::routing::host_in(&[s.as_str()], &host));
        out.push(match listed {
            Some(site) => entry(
                format!("Let {} Sleep", menu_host(site)),
                Command::ToggleNeverSleepSite(site.clone()),
            ),
            None => entry(
                format!("Never Sleep {}", menu_host(&host)),
                Command::ToggleNeverSleepSite(host),
            ),
        });
    }
    out
}

/// "Move to New Window", then a row per other Main window, named after the Space it shows
/// (the always-available path where a drag cannot reach another window, Wayland).
pub fn window_entries(model: &Model, tab: TabId) -> Vec<Entry> {
    let entry = |label: String, to| Entry {
        label,
        command: Command::MoveToWindow(tab, to),
        enabled: true,
        icon: "move",
        shortcut: String::new(),
        danger: false,
    };
    let own = model.window_of(tab);
    // a private window's tab moves only among private windows (a new one is private too), a
    // normal tab only among normal ones
    let side = model.side_of_tab(tab);
    let others = model.windows.iter().filter(|w| {
        w.kind() == WindowKind::Main
            && Some(w.id) != own
            && w.space() != model.location(tab).map(|l| l.0)
            && model.side_of_window(w.id) == side
    });
    std::iter::once(entry("Move to New Window".into(), WindowTarget::New))
        .chain(others.filter_map(|w| {
            let space = model.spaces.iter().find(|s| Some(s.id) == w.space())?;
            Some(entry(
                format!("Move to Window: {}", space.name),
                WindowTarget::Existing(w.id),
            ))
        }))
        .collect()
}

/// The page menu's rows of a Little window ("Open as Tab" into the active Space, "Open in
/// <Space>" for the others) or a Popup window ("Open as Tab" in the Main window); none for a
/// Main window. A Little or Popup window has no sidebar, so this menu is its way out.
pub fn window_page_entries(model: &Model, window: WindowId) -> Vec<Entry> {
    let Some(w) = model.window(window) else {
        return vec![];
    };
    let entry = |label: String, command| Entry {
        label,
        command,
        enabled: true,
        icon: "",
        shortcut: String::new(),
        danger: false,
    };
    // a Little window's page is never a private window's: it goes to the last normal Space
    let active = model.normal_space();
    match w.shown {
        Shown::Little(_) => std::iter::once(entry(
            "Open as Tab".into(),
            Command::PromoteLittle(window, active),
        ))
        .chain(
            model
                .spaces
                .iter()
                .filter(|s| s.id != active && s.kind != SpaceKind::Private)
                .map(|s| {
                    entry(
                        format!("Open in {}", s.name),
                        Command::PromoteLittle(window, s.id),
                    )
                }),
        )
        .collect(),
        // a private window's pop-up becomes a tab of the private window it came from, any other
        // one a tab of the focused Main window, or of the last normal one when that is private
        Shown::Popup(tab) => {
            let main = model.main_window();
            let to = if model.side_of_tab(tab) == Side::Private {
                model
                    .location(tab)
                    .and_then(|(space, _)| model.space_window(space))
                    .unwrap_or(main)
            } else if model.side_of_window(main) == Side::Private {
                model.space_window(model.normal_space()).unwrap_or(main)
            } else {
                main
            };
            vec![entry(
                "Open as Tab".into(),
                Command::MoveToWindow(tab, WindowTarget::Existing(to)),
            )]
        }
        // its controls are its way out
        Shown::Space(_) | Shown::Pip(_) => vec![],
    }
}

/// The question a close asks while it would cancel `count` downloads of private tabs (Chrome's
/// "If you close all Incognito windows now, N downloads will be cancelled"): what goes, then
/// the answers. Dismissing it keeps the window, as "keep" does.
pub fn close_entries(closing: Closing, count: usize) -> Vec<Entry> {
    let (what, them) = if count == 1 {
        ("1 download".to_owned(), "Download")
    } else {
        (format!("{count} downloads"), "Downloads")
    };
    let (close, keep) = match closing {
        Closing::Window(_) => ("Close", "Keep Window Open"),
        Closing::App => ("Quit", "Keep wubble Open"),
        Closing::Restart => ("Restart", "Keep wubble Open"),
    };
    let entry = |label: String, command, enabled, danger| Entry {
        label,
        command,
        enabled,
        icon: "",
        shortcut: String::new(),
        danger,
    };
    vec![
        entry(
            format!("{what} from private windows will be cancelled"),
            Command::Info,
            false,
            false,
        ),
        entry(
            format!("Cancel {them} and {close}"),
            Command::CancelDownloadsAndClose(closing),
            true,
            true,
        ),
        entry(keep.into(), Command::Dismiss, true, false),
    ]
}

/// The address pill's pop-up menu: open what the page asked for, or allow the site.
pub fn popup_entries(count: usize, site: Option<&str>) -> Vec<Entry> {
    let mut out = vec![Entry {
        label: if count == 1 {
            "Open the Blocked Pop-up".into()
        } else {
            format!("Open {count} Blocked Pop-ups")
        },
        command: Command::OpenBlockedPopups,
        enabled: true,
        icon: "",
        shortcut: String::new(),
        danger: false,
    }];
    out.extend(site.map(|site| Entry {
        label: format!("Always Allow Pop-ups on {}", menu_host(site)),
        command: Command::AllowPopups(site.to_owned()),
        enabled: true,
        icon: "shield",
        shortcut: String::new(),
        danger: false,
    }));
    out
}

/// The split entries of `tab`'s row (the design has no tab menu; these are its command-bar
/// rows, `browser/command.tsx`, for the row's tab): the focused page splits with the next tab or
/// a page chosen beside it; another tab of the Space goes beside the focused page; a pane leaves
/// its split.
fn split_entries(model: &Model, tab: TabId) -> Vec<Entry> {
    let entry = |label: &str, command| Entry {
        label: label.into(),
        command,
        enabled: true,
        icon: "split",
        shortcut: String::new(),
        danger: false,
    };
    if model.split_of(tab).is_some() {
        return vec![entry("Take Out of Split View", Command::Unsplit(tab))];
    }
    let room = model.pane_room();
    let in_space = model.order(model.active_space()).contains(&tab);
    if !room || !in_space {
        return vec![];
    }
    if model.visible_tab() == Some(tab) {
        let mut out = vec![];
        if model.split_candidate().is_some() {
            out.push(entry(
                "Split View with the Next Tab",
                Command::Perform(Action::SplitNext),
            ));
        }
        out.push(entry("Open Beside\u{2026}", Command::OpenBeside));
        out
    } else {
        vec![entry("Open in Split View", Command::SplitWith(tab))]
    }
}

/// Hosts the blocker menu names; the rest are summed in one row.
pub const BLOCKER_MENU_HOSTS: usize = 5;

/// The address pill's blocker menu: what was blocked (in all, then per
/// host, most first: `hosts` as [`crate::blocking::Blocking::blocked_hosts`]
/// gives them), and the switch for this site.
pub fn blocker_entries(
    count: u32,
    hosts: &[(String, u32)],
    host: &str,
    allowed: bool,
) -> Vec<Entry> {
    let info = |label: String| Entry {
        label,
        command: Command::Info,
        enabled: false,
        icon: "",
        shortcut: String::new(),
        danger: false,
    };
    let plural = |n: u32| if n == 1 { "" } else { "s" };
    let mut out = vec![info(format!("Blocked {count} request{}", plural(count)))];
    out.extend(
        hosts
            .iter()
            .take(BLOCKER_MENU_HOSTS)
            .map(|(h, n)| info(format!("{} \u{2014} {n}", menu_host(h)))),
    );
    let rest = hosts.get(BLOCKER_MENU_HOSTS..).unwrap_or_default();
    if !rest.is_empty() {
        let n: u32 = rest.iter().map(|(_, n)| n).sum();
        let more = u32::try_from(rest.len()).unwrap_or(u32::MAX);
        out.push(info(format!(
            "{more} more host{} \u{2014} {n}",
            plural(more)
        )));
    }
    out.push(Entry {
        label: if allowed {
            format!("Turn on blocking on {}", menu_host(host))
        } else {
            format!("Turn off blocking on {}", menu_host(host))
        },
        command: Command::ToggleBlocker(host.to_owned()),
        enabled: true,
        icon: "shield",
        shortcut: String::new(),
        danger: false,
    });
    out
}

/// What a "Clear Browsing Data" row deletes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClearData {
    /// Cookies, HTTP cache and HTTP auth of these containers.
    Containers(Vec<ContainerId>),
    /// Everything this origin stored (cookies, local storage, IndexedDB,
    /// service workers) in the tab's container.
    Site(String),
}

/// `Action::ClearBrowsingData`'s menu for `tab` (on `origin`, if a web page): the rows say what
/// goes, and choosing one is the confirmation; dismissing it clears nothing.
pub fn clear_entries(model: &Model, tab: TabId, origin: Option<&str>) -> Vec<Entry> {
    let container = model.tabs.get(&tab).map(|t| t.container);
    // the containers on the tab's side of the private line: a private window clears its
    // in-memory container only, a normal one never names it
    let mine = model.side_of_tab(tab);
    let side: Vec<ContainerId> = model
        .containers
        .iter()
        .filter(|c| c.storage.side() == mine)
        .map(|c| c.id)
        .collect();
    let name = container
        .and_then(|c| model.containers.iter().find(|k| k.id == c))
        .map_or("this container", |c| c.name.as_str());
    let row = |label: String, command| Entry {
        label,
        command,
        enabled: true,
        icon: "",
        shortcut: String::new(),
        danger: true,
    };
    let mut out = vec![Entry {
        label: "Delete cookies and cached files (sites will sign you out)".into(),
        command: Command::Info,
        enabled: false,
        icon: "",
        shortcut: String::new(),
        danger: false,
    }];
    if let Some(origin) = origin.filter(|_| container.is_some()) {
        let site = origin.split_once("://").map_or(origin, |(_, h)| h);
        out.push(row(
            format!("Forget {} in {name}", menu_host(site)),
            Command::ClearData(ClearData::Site(origin.to_owned())),
        ));
    }
    if let Some(c) = container.filter(|_| side.len() > 1) {
        out.push(row(
            format!("Clear {name}"),
            Command::ClearData(ClearData::Containers(vec![c])),
        ));
    }
    out.push(row(
        if side.len() > 1 {
            "Clear All Containers".into()
        } else {
            "Clear Cookies and Cache".into()
        },
        Command::ClearData(ClearData::Containers(side)),
    ));
    out
}

/// F7's question before caret browsing turns on (`wubble_core::caret`): Chrome's dialog
/// (`chrome/browser/ui/views/accessibility/caret_browsing_dialog_delegate.cc`, its strings
/// `IDS_CARET_BROWSING_*`, BSD-3-Clause; strings only) as a menu, whose rows are its "Turn on"
/// button with and without "Don't ask again"; dismissing it is Cancel.
pub fn caret_entries() -> Vec<Entry> {
    let line = |label: &str, command, enabled| Entry {
        label: label.into(),
        command,
        enabled,
        icon: "",
        shortcut: String::new(),
        danger: false,
    };
    vec![
        line("Turn on caret browsing?", Command::Info, false),
        line(
            "Move through the text of pages with the arrow keys; F7 turns it off",
            Command::Info,
            false,
        ),
        line("Turn On", Command::CaretBrowsing { dont_ask: false }, true),
        line(
            "Turn On, Don\u{2019}t Ask Again",
            Command::CaretBrowsing { dont_ask: true },
            true,
        ),
    ]
}

/// One stream of the media menu: its id, its title, and what is known of it.
pub struct MediaRow<'a> {
    pub item: u32,
    pub title: &'a str,
    pub found: &'a wubble_core::media::detect::Found,
}

/// The media menu: per stream a title line, then a row per way to save it (or why it cannot be
/// saved). Without a muxer (`can_join`) a row for separate video and audio says it is two files.
pub fn media_entries(rows: &[MediaRow], can_join: bool) -> Vec<Entry> {
    use wubble_core::media::detect::Found;
    let info = |label: String| Entry {
        label,
        command: Command::Info,
        enabled: false,
        icon: "",
        shortcut: String::new(),
        danger: false,
    };
    let mut out = Vec::new();
    if rows.is_empty() {
        // The strip's control was pressed and the downloader has nothing for the page.
        return vec![
            info("Nothing to save was found on this page".into()),
            info("A player that streams in small pieces may not be offered".into()),
        ];
    }
    for (i, row) in rows.iter().enumerate() {
        if i > 0 {
            out.push(Entry {
                label: String::new(),
                command: Command::Separator,
                enabled: false,
                icon: "",
                shortcut: String::new(),
                danger: false,
            });
        }
        out.push(info(row.title.to_owned()));
        match row.found {
            Found::Resolving => out.push(info("Reading the stream\u{2026}".into())),
            Found::Refused(refusal) => {
                out.push(info(refusal.message()));
                if *refusal == wubble_core::media::Refusal::LookupOff {
                    out.push(Entry {
                        label: "Open Settings".into(),
                        command: Command::Perform(Action::OpenSettings),
                        enabled: true,
                        icon: "",
                        shortcut: String::new(),
                        danger: false,
                    });
                }
            }
            Found::Ready(variants) => {
                out.extend(
                    wubble_core::media::select::choices(variants)
                        .into_iter()
                        .enumerate()
                        .map(|(choice, c)| Entry {
                            label: if c.pair && !can_join {
                                format!("Save {} \u{b7} video and audio as two files", c.label)
                            } else {
                                format!("Save {}", c.label)
                            },
                            command: Command::SaveMedia {
                                item: row.item,
                                choice,
                            },
                            enabled: true,
                            icon: "download",
                            shortcut: String::new(),
                            danger: false,
                        }),
                );
            }
        }
    }
    out
}

/// The site menu's page-theme rows (dark mode for pages): one per theme, the current one
/// marked and not chosen again.
pub fn page_theme_entries(host: &str, current: PageTheme) -> Vec<Entry> {
    let mut out = vec![Entry {
        label: format!("Page theme on {}", menu_host(host)),
        command: Command::Separator,
        enabled: false,
        icon: "",
        shortcut: String::new(),
        danger: false,
    }];
    out.extend(PageTheme::ALL.into_iter().map(|theme| Entry {
        label: if theme == current {
            format!("\u{2713} {}", theme.label())
        } else {
            format!("    {}", theme.label())
        },
        command: Command::SetPageTheme(host.to_owned(), theme),
        enabled: theme != current,
        icon: "",
        shortcut: String::new(),
        danger: false,
    }));
    out
}

/// The open menu's content (`overlay::Kind::Menu` says whether it is open): a page's, or one the
/// shell opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Menu {
    Open {
        id: RequestId,
        /// The pointer, physical pixels relative to the view.
        at: (i32, i32),
        entries: Vec<Entry>,
    },
    /// The shell's own menu (a tab row's): no request to answer.
    Shell {
        /// The pointer, physical pixels in the window.
        at: (i32, i32),
        entries: Vec<Entry>,
    },
}

impl Menu {
    /// Whether the page keeps keyboard focus while this menu holds the shell's: a page's own
    /// menu is part of it, as Chromium's native menu, which never blurs the page. Blurring it
    /// would make CEF cancel the request the menu is about to answer (`SetFocus(false)` calls
    /// `CancelContextMenu`), closing the menu the moment it takes focus.
    pub fn keeps_page_focus(&self) -> bool {
        matches!(self, Menu::Open { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEvent {
    Requested {
        id: RequestId,
        menu: Box<ContextMenu>,
        nav: NavFlags,
        /// The window's own rows, after the page's (a Little or Popup window's way out).
        extra: Vec<Entry>,
    },
    /// The shell opens its own menu.
    ShellRequested { at: (i32, i32), entries: Vec<Entry> },
    /// The user picked entry `i`.
    Chosen(usize),
    /// Outside click, Escape, or the tab hiding it.
    Dismiss,
    /// CEF closed the menu itself (it already answered).
    Reset,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MenuEffect {
    Answer {
        id: RequestId,
        command: Option<i32>,
    },
    /// An app command, after the dismissing answer.
    Run(Command),
}

/// One menu event on the open menu (`None`: none open; the view gone is the overlays' store's,
/// no event comes here); the menu back, `None` when it closes.
pub fn transition(state: Option<Menu>, event: MenuEvent) -> (Option<Menu>, Vec<MenuEffect>) {
    use MenuEvent::*;
    let dismiss = |id| MenuEffect::Answer { id, command: None };
    // CEF holds one menu at a time; a newer one replaces the old, which is dismissed
    let replaced = |state: &Option<Menu>| match state {
        Some(Menu::Open { id, .. }) => vec![dismiss(*id)],
        Some(Menu::Shell { .. }) | None => vec![],
    };
    match (state, event) {
        (
            state,
            Requested {
                id,
                menu,
                nav,
                extra,
            },
        ) => {
            let mut effects = replaced(&state);
            let mut entries = entries(&menu, nav);
            if !entries.is_empty() && !extra.is_empty() {
                entries.push(Entry {
                    label: String::new(),
                    command: Command::Separator,
                    enabled: false,
                    icon: "",
                    shortcut: String::new(),
                    danger: false,
                });
            }
            entries.extend(extra);
            if entries.is_empty() {
                effects.push(dismiss(id));
                return (None, effects);
            }
            let open = Menu::Open {
                id,
                at: (menu.x, menu.y),
                entries,
            };
            (Some(open), effects)
        }
        (Some(Menu::Open { id, at, entries }), Chosen(i)) => {
            match entries.get(i).filter(|e| e.enabled).map(|e| &e.command) {
                None | Some(Command::Separator) => (Some(Menu::Open { id, at, entries }), vec![]),
                Some(Command::Engine(command)) => (
                    None,
                    vec![MenuEffect::Answer {
                        id,
                        command: Some(*command),
                    }],
                ),
                Some(command) => (None, vec![dismiss(id), MenuEffect::Run(command.clone())]),
            }
        }
        (Some(Menu::Open { id, .. }), Dismiss) => (None, vec![dismiss(id)]),
        (state, ShellRequested { at, entries }) => {
            let effects = replaced(&state);
            if entries.is_empty() {
                return (None, effects);
            }
            (Some(Menu::Shell { at, entries }), effects)
        }
        (Some(Menu::Shell { at, entries }), Chosen(i)) => {
            match entries.get(i).filter(|e| e.enabled).map(|e| &e.command) {
                None | Some(Command::Separator) => (Some(Menu::Shell { at, entries }), vec![]),
                Some(command) => (None, vec![MenuEffect::Run(command.clone())]),
            }
        }
        (_, Reset) => (None, vec![]),
        (Some(Menu::Shell { .. }) | None, Dismiss) | (None, Chosen(_)) => (None, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_engine::page::{EditFlags, MediaFlags};

    #[test]
    fn the_clear_menu_names_what_goes() {
        let commands = |e: &[Entry]| -> Vec<Command> {
            e.iter()
                .filter(|e| e.enabled)
                .map(|e| e.command.clone())
                .collect()
        };
        let m = Model::new(std::time::Instant::now());
        let (m, _) = wubble_core::model::apply(
            m,
            ModelCommand::NewTab {
                url: "https://a.test/x".into(),
                container: None,
                opener: None,
            },
        );
        let tab = *m.tabs.keys().next().unwrap();
        let d = ContainerId::DEFAULT;
        // One container: the site, then everything; every row destructive.
        let e = clear_entries(&m, tab, Some("https://a.test"));
        assert_eq!(
            commands(&e),
            [
                Command::ClearData(ClearData::Site("https://a.test".into())),
                Command::ClearData(ClearData::Containers(vec![d])),
            ]
        );
        assert!(e.iter().filter(|e| e.enabled).all(|e| e.danger));
        assert_eq!(e[1].label, "Forget a.test in Default");
        // Two containers: this one alone, or all; no site on an internal page.
        let (m, _) = wubble_core::model::apply(
            m,
            ModelCommand::NewContainer {
                name: "Work".into(),
            },
        );
        let work = m.containers[1].id;
        let e = clear_entries(&m, tab, None);
        assert_eq!(
            commands(&e),
            [
                Command::ClearData(ClearData::Containers(vec![d])),
                Command::ClearData(ClearData::Containers(vec![d, work])),
            ]
        );
        assert_eq!(e[2].label, "Clear All Containers");
    }

    #[test]
    fn the_media_menu_lists_choices_and_refusals() {
        use wubble_core::media::detect::Found;
        use wubble_core::media::{Container, Refusal, Track, Variant};
        let ready = Found::Ready(vec![Variant::file(
            "https://a.test/v.mp4",
            Track::Muxed,
            Container::Mp4,
            Some(2000),
            true,
        )]);
        let protected = Found::Refused(Refusal::Protected);
        let resolving = Found::Resolving;
        let rows = [
            MediaRow {
                item: 4,
                title: "Clip",
                found: &ready,
            },
            MediaRow {
                item: 5,
                title: "Film",
                found: &protected,
            },
            MediaRow {
                item: 6,
                title: "Show",
                found: &resolving,
            },
        ];
        let entries = media_entries(&rows, true);
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Clip",
                "Save MP4 \u{b7} 2.0 KB",
                "",
                "Film",
                "Protected, cannot be saved",
                "",
                "Show",
                "Reading the stream\u{2026}"
            ]
        );
        assert_eq!(
            entries[1].command,
            Command::SaveMedia { item: 4, choice: 0 }
        );
        assert!(entries[1].enabled && !entries[4].enabled);
        assert_eq!(entries[2].command, Command::Separator);
    }

    #[test]
    fn a_youtube_lookup_that_is_off_points_to_settings() {
        use wubble_core::media::Refusal;
        use wubble_core::media::detect::Found;
        let off = Found::Refused(Refusal::LookupOff);
        let failed = Found::Refused(Refusal::Failed("YouTube: Sign in".into()));
        let rows = [
            MediaRow {
                item: 0,
                title: "A",
                found: &off,
            },
            MediaRow {
                item: 1,
                title: "B",
                found: &failed,
            },
        ];
        let e = media_entries(&rows, true);
        let labels: Vec<&str> = e.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "A",
                "YouTube format lookup is off",
                "Open Settings",
                "",
                "B",
                "Cannot read the stream: YouTube: Sign in"
            ]
        );
        assert_eq!(e[2].command, Command::Perform(Action::OpenSettings));
        assert!(e[2].enabled);
    }

    #[test]
    fn a_pair_without_a_muxer_says_it_is_two_files() {
        use wubble_core::media::detect::Found;
        use wubble_core::media::{Container, Track, Variant};
        let mut video = Variant::file(
            "https://a.test/v.mp4",
            Track::Video,
            Container::Mp4,
            None,
            false,
        );
        video.height = Some(720);
        let audio = Variant::file(
            "https://a.test/a.m4a",
            Track::Audio,
            Container::M4a,
            None,
            false,
        );
        let found = Found::Ready(vec![video, audio]);
        let rows = [MediaRow {
            item: 0,
            title: "Clip",
            found: &found,
        }];
        let label = |join| media_entries(&rows, join)[1].label.clone();
        assert_eq!(label(true), "Save 720p \u{b7} MP4");
        assert_eq!(
            label(false),
            "Save 720p \u{b7} MP4 \u{b7} video and audio as two files"
        );
    }

    #[test]
    fn the_blocker_menu_names_the_count_and_the_switch() {
        let on = blocker_entries(1, &[], "a.test", false);
        assert_eq!(on[0].label, "Blocked 1 request");
        assert!(!on[0].enabled);
        assert_eq!(on[1].label, "Turn off blocking on a.test");
        assert_eq!(on[1].command, Command::ToggleBlocker("a.test".into()));
        let off = blocker_entries(0, &[], "a.test", true);
        assert_eq!(off[0].label, "Blocked 0 requests");
        assert_eq!(off[1].label, "Turn on blocking on a.test");
    }

    /// The sidebar's empty glass offers what the sidebar already does, nothing new.
    #[test]
    fn the_sidebar_menu_offers_a_tab_a_space_and_a_folder() {
        let m = Model::new(std::time::Instant::now());
        let e = sidebar_entries(&m, &Keymap::defaults(wubble_core::keys::Platform::Linux));
        let labels: Vec<_> = e.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["New Tab", "New Space", "New Folder"]);
        assert_eq!(e[0].command, Command::Perform(Action::NewTab));
        assert_eq!(e[1].command, Command::Perform(Action::NewSpace));
        assert!(e.iter().all(|e| e.enabled && !e.danger));
    }

    /// A long host is cut in its middle, so the row keeps its start and its domain on one line
    /// (QA HIGH-03); the command still names the whole host.
    #[test]
    fn a_menu_row_cuts_a_long_host_in_its_middle() {
        let host = "a-very-long-subdomain.of-some-site.example.com";
        let e = blocker_entries(1, &[], host, false);
        assert_eq!(
            e[1].label,
            "Turn off blocking on a-very-lon\u{2026}ample.com"
        );
        assert_eq!(e[1].command, Command::ToggleBlocker(host.into()));
        assert_eq!(menu_host("en.wikipedia.org"), "en.wikipedia.org");
    }

    #[test]
    fn the_blocker_menu_breaks_the_count_down_by_host() {
        let hosts: Vec<(String, u32)> = (1..=7u32)
            .rev()
            .map(|n| (format!("h{n}.test"), n))
            .collect();
        let count = hosts.iter().map(|(_, n)| n).sum();
        let e = blocker_entries(count, &hosts, "a.test", false);
        assert_eq!(
            labels(&e),
            [
                "Blocked 28 requests",
                "h7.test \u{2014} 7",
                "h6.test \u{2014} 6",
                "h5.test \u{2014} 5",
                "h4.test \u{2014} 4",
                "h3.test \u{2014} 3",
                "2 more hosts \u{2014} 3",
                "Turn off blocking on a.test",
            ]
        );
        assert!(e[..7].iter().all(|e| !e.enabled));
    }

    #[test]
    fn the_site_menu_offers_each_page_theme() {
        let rows = page_theme_entries("a.test", PageTheme::Dark);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].label, "Page theme on a.test");
        assert!(!rows[0].enabled);
        assert_eq!(
            rows[2].command,
            Command::SetPageTheme("a.test".into(), PageTheme::Dark)
        );
        assert!(rows[2].label.starts_with('\u{2713}') && !rows[2].enabled);
        assert!(rows[1].enabled && rows[3].enabled);
    }

    #[test]
    fn any_page_but_a_field_offers_an_element_screenshot() {
        let menu = ContextMenu {
            x: 30,
            y: 40,
            ..Default::default()
        };
        let rows = entries(&menu, NavFlags::default());
        let shot = rows
            .iter()
            .find(|e| e.label == "Take Screenshot of Element")
            .unwrap();
        assert_eq!(shot.command, Command::ShotElement((30, 40)));
        let field = ContextMenu {
            editable: true,
            ..menu
        };
        assert!(
            !entries(&field, NavFlags::default())
                .iter()
                .any(|e| e.label == "Take Screenshot of Element")
        );
    }

    #[test]
    fn a_close_that_cancels_private_downloads_asks_first() {
        let w = WindowId(3);
        for (closing, count, want) in [
            (
                Closing::Window(w),
                1,
                [
                    "1 download from private windows will be cancelled",
                    "Cancel Download and Close",
                    "Keep Window Open",
                ],
            ),
            (
                Closing::App,
                3,
                [
                    "3 downloads from private windows will be cancelled",
                    "Cancel Downloads and Quit",
                    "Keep wubble Open",
                ],
            ),
            (
                Closing::Restart,
                2,
                [
                    "2 downloads from private windows will be cancelled",
                    "Cancel Downloads and Restart",
                    "Keep wubble Open",
                ],
            ),
        ] {
            let e = close_entries(closing, count);
            assert_eq!(labels(&e), want);
            assert_eq!(
                e.iter().map(|e| (e.enabled, e.danger)).collect::<Vec<_>>(),
                [(false, false), (true, true), (true, false)]
            );
            assert_eq!(e[1].command, Command::CancelDownloadsAndClose(closing));
            assert_eq!(e[2].command, Command::Dismiss);
        }
    }

    #[test]
    fn the_zoom_popover_names_the_percent_and_steps() {
        let km = Keymap::defaults(wubble_core::keys::Platform::Linux);
        let e = zoom_entries(125, 100, &km);
        assert_eq!(
            labels(&e),
            ["Zoom 125%", "Zoom Out", "Zoom In", "Reset to 100%"]
        );
        assert!(!e[0].enabled);
        assert_eq!(e[2].command, Command::ZoomStep(Action::ZoomIn));
        assert_eq!(e[3].command, Command::ZoomStep(Action::ZoomReset));
        // the page menu offers the reset only while the page is zoomed
        let zoomed = NavFlags {
            zoomed: Some(125),
            ..NavFlags::default()
        };
        let reset = |nav| {
            entries(&page(), nav)
                .into_iter()
                .find(|e| e.command == Command::Perform(Action::ZoomReset))
                .map(|e| e.label)
        };
        assert_eq!(reset(zoomed), Some("Reset Zoom (125%)".into()));
        assert_eq!(reset(NavFlags::default()), None);
    }

    #[test]
    fn the_reload_menu_offers_the_empty_cache_reload_with_devtools() {
        let km = Keymap::defaults(wubble_core::keys::Platform::Linux);
        for (devtools, want) in [
            (false, vec!["Normal Reload", "Hard Reload"]),
            (
                true,
                vec![
                    "Normal Reload",
                    "Hard Reload",
                    "Empty Cache and Hard Reload",
                ],
            ),
        ] {
            let e = reload_entries(devtools, &km);
            assert_eq!(labels(&e), want, "devtools {devtools}");
            assert_eq!(e[1].command, Command::Perform(Action::HardReload));
            assert_eq!(e[0].shortcut, "Ctrl+R");
        }
    }

    #[test]
    fn address_menus_follow_the_field_and_the_clipboard() {
        let km = Keymap::defaults(wubble_core::keys::Platform::Linux);
        let facts = |field, selected, text, clipboard| FieldFacts {
            field,
            selected,
            text,
            clipboard,
            full_urls: false,
        };
        let bar = address_entries(facts(AddressField::Bar, true, true, " example.com "), &km);
        assert_eq!(
            labels(&bar),
            [
                "Cut",
                "Copy",
                "Paste",
                "Paste and Go to https://example.com/",
                "Delete",
                "-",
                "Select All",
                "-",
                "Always Show Full URLs"
            ]
        );
        let paste = bar.iter().find(|e| e.label.starts_with("Paste and"));
        assert_eq!(
            paste.map(|e| &e.command),
            Some(&Command::PasteAndGo(
                AddressField::Bar,
                "example.com".into()
            ))
        );
        let cut = bar.iter().find(|e| e.label == "Cut");
        assert_eq!(cut.map(|e| e.shortcut.as_str()), Some("Ctrl+X"));
        let pill = address_entries(facts(AddressField::Pill, false, false, "two\nwords"), &km);
        assert_eq!(
            labels(&pill),
            [
                "Paste and Search for \u{201c}two words\u{201d}",
                "-",
                "Copy Address",
                "Edit Address",
                "-",
                "Always Show Full URLs"
            ]
        );
        // what is enabled: (name, facts, the rows that are)
        for (name, f, want) in [
            (
                "nothing selected, empty clipboard",
                facts(AddressField::Bar, false, true, ""),
                vec!["Select All", "Always Show Full URLs"],
            ),
            (
                "an empty field",
                facts(AddressField::Bar, false, false, "  "),
                vec!["Always Show Full URLs"],
            ),
            (
                "a selection",
                facts(AddressField::Bar, true, true, ""),
                vec![
                    "Cut",
                    "Copy",
                    "Delete",
                    "Select All",
                    "Always Show Full URLs",
                ],
            ),
            (
                "the pill, nothing to paste",
                facts(AddressField::Pill, false, false, ""),
                vec!["Copy Address", "Edit Address", "Always Show Full URLs"],
            ),
        ] {
            let entries = address_entries(f, &km);
            let on: Vec<&str> = entries
                .iter()
                .filter(|e| e.enabled)
                .map(|e| e.label.as_str())
                .collect();
            assert_eq!(on, want, "{name}");
        }
        // the empty clipboard's row says so; the switch shows its state and flips it
        let empty = address_entries(facts(AddressField::Pill, false, false, ""), &km);
        assert_eq!(empty[0].label, "Paste and Go");
        let full = |on| {
            let f = FieldFacts {
                full_urls: on,
                ..facts(AddressField::Pill, false, false, "")
            };
            let e = address_entries(f, &km);
            let last = e.last().expect("rows");
            (last.icon, last.command.clone())
        };
        assert_eq!(full(false), ("", Command::SetFullUrls(true)));
        assert_eq!(full(true), ("check", Command::SetFullUrls(false)));
        // a long address is cut in the middle, a long search at its end
        let long = format!("https://a.test/{}", "x".repeat(80));
        assert_eq!(
            paste_label(&long).chars().count(),
            "Paste and Go to ".len() + 50
        );
        let words = "word ".repeat(20);
        assert!(paste_label(&words).ends_with("\u{2026}\u{201d}"));
    }

    fn labels(entries: &[Entry]) -> Vec<&str> {
        entries
            .iter()
            .map(|e| match e.command {
                Command::Separator => "-",
                _ => e.label.as_str(),
            })
            .collect()
    }

    fn page() -> ContextMenu {
        ContextMenu {
            x: 10,
            y: 20,
            page_url: "https://a.test/".into(),
            ..Default::default()
        }
    }

    fn link() -> ContextMenu {
        ContextMenu {
            link_url: "https://a.test/next".into(),
            ..page()
        }
    }

    fn requested(id: u32, menu: ContextMenu) -> MenuEvent {
        MenuEvent::Requested {
            id: RequestId(id),
            menu: Box::new(menu),
            nav: NavFlags::default(),
            extra: vec![],
        }
    }

    #[test]
    fn the_bare_page_offers_navigation() {
        let nav = NavFlags {
            back: true,
            forward: false,
            ..NavFlags::default()
        };
        let e = entries(&page(), nav);
        assert_eq!(
            labels(&e),
            [
                "Back",
                "Forward",
                "Reload",
                "-",
                "Save Page As\u{2026}",
                "Print\u{2026}",
                "-",
                "View Page Source",
                "Take Screenshot of Element",
                "Inspect"
            ]
        );
        assert_eq!(
            e.iter().take(3).map(|e| e.enabled).collect::<Vec<_>>(),
            [true, false, true]
        );
        assert_eq!(e[4].command, Command::SaveAs("https://a.test/".into()));
        assert_eq!(e[7].command, Command::Perform(Action::ViewSource));
        assert_eq!(e[0].command, Command::Engine(cef_id::BACK));
    }

    #[test]
    fn inspect_carries_the_pointer_and_view_source_is_for_the_bare_page() {
        let e = entries(&page(), NavFlags::default());
        let inspect = e.iter().find(|e| e.label == "Inspect").unwrap();
        assert_eq!(inspect.command, Command::Inspect { x: 10, y: 20 });
        let source = e.iter().find(|e| e.label == "View Page Source").unwrap();
        assert_eq!(source.command, Command::Perform(Action::ViewSource));
        let e = entries(&link(), NavFlags::default());
        assert!(e.iter().all(|e| e.label != "View Page Source"));
    }

    #[test]
    fn a_link_offers_new_tab_and_copy() {
        let e = entries(&link(), NavFlags::default());
        assert_eq!(
            labels(&e),
            [
                "Open Link in New Tab",
                "Open Link in New Window",
                "Open Link in Private Window",
                "Open Link in Split View",
                "Open Link in a Peek",
                "Save Link As\u{2026}",
                "Copy Link Address",
                "-",
                "Take Screenshot of Element",
                "Inspect"
            ]
        );
        let next = || "https://a.test/next".to_owned();
        for (label, command) in [
            ("Open Link in New Tab", Command::OpenInNewTab(next())),
            ("Open Link in New Window", Command::OpenInNewWindow(next())),
            (
                "Open Link in Private Window",
                Command::OpenInPrivateWindow(next()),
            ),
            (
                "Open Link in Split View",
                Command::Model(TabCommand(ModelCommand::OpenBeside { url: next() })),
            ),
            ("Open Link in a Peek", Command::OpenInGlance(next())),
            ("Save Link As\u{2026}", Command::SaveAs(next())),
            ("Copy Link Address", Command::CopyText(next())),
            ("Take Screenshot of Element", Command::ShotElement((10, 20))),
            ("Inspect", Command::Inspect { x: 10, y: 20 }),
        ] {
            let row = e.iter().find(|e| e.label == label);
            assert_eq!(row.map(|e| &e.command), Some(&command), "{label}");
        }
    }

    #[test]
    fn a_linked_image_lists_both_groups() {
        let m = ContextMenu {
            media: MediaKind::Image,
            media_url: "https://a.test/i.png".into(),
            has_image: true,
            ..link()
        };
        let e = entries(&m, NavFlags::default());
        assert_eq!(
            labels(&e),
            [
                "Open Link in New Tab",
                "Open Link in New Window",
                "Open Link in Private Window",
                "Open Link in Split View",
                "Open Link in a Peek",
                "Save Link As\u{2026}",
                "Copy Link Address",
                "-",
                "Open Image in New Tab",
                "Save Image As\u{2026}",
                "Copy Image",
                "Copy Image Address",
                "-",
                "Take Screenshot of Element",
                "Inspect"
            ]
        );
        let copy = e.iter().find(|e| e.label == "Copy Image");
        assert_eq!(
            copy.map(|e| &e.command),
            Some(&Command::CopyImage("https://a.test/i.png".into()))
        );
        // An image that has not loaded has nothing to copy.
        let unloaded = ContextMenu {
            has_image: false,
            ..m.clone()
        };
        assert!(!labels(&entries(&unloaded, NavFlags::default())).contains(&"Copy Image"));
    }

    #[test]
    fn a_selection_offers_copy() {
        let m = ContextMenu {
            selection_text: "hello".into(),
            ..page()
        };
        let e = entries(&m, NavFlags::default());
        assert_eq!(
            labels(&e),
            [
                "Copy",
                "Copy Link to Highlight",
                "-",
                "Search the Web for \u{201c}hello\u{201d}",
                "Print\u{2026}",
                "-",
                "Take Screenshot of Element",
                "Inspect"
            ]
        );
        assert!(e[0].enabled);
        let search = |e: &[Entry]| {
            e.iter()
                .position(|e| matches!(&e.command, Command::Search(t) if t == "hello"))
        };
        assert!(search(&e).is_some());
        let named = NavFlags {
            search: Some("DuckDuckGo"),
            ..NavFlags::default()
        };
        let named = entries(&m, named);
        assert_eq!(
            search(&named).map(|i| named[i].label.as_str()),
            Some("Search DuckDuckGo for \u{201c}hello\u{201d}")
        );
    }

    /// A field's whole menu: on macOS with Emoji & Symbols first and the writing directions
    /// after Select All.
    fn field_labels() -> Vec<&'static str> {
        let mac = cfg!(target_os = "macos");
        let emoji: &[&str] = if mac { &["Emoji & Symbols", "-"] } else { &[] };
        let direction: &[&str] = if mac {
            &[
                "-",
                "Writing Direction: Default",
                "Writing Direction: Left to Right",
                "Writing Direction: Right to Left",
            ]
        } else {
            &[]
        };
        [
            emoji,
            &[
                "Undo",
                "Redo",
                "-",
                "Cut",
                "Copy",
                "Paste",
                "Paste as Plain Text",
                "Delete",
                "Select All",
            ],
            direction,
            &["-", "Inspect"],
        ]
        .concat()
    }

    #[test]
    fn editable_entries_follow_every_flag() {
        let all = [
            ("Undo", EditFlags::CAN_UNDO),
            ("Redo", EditFlags::CAN_REDO),
            ("Cut", EditFlags::CAN_CUT),
            ("Copy", EditFlags::CAN_COPY),
            ("Paste", EditFlags::CAN_PASTE),
            ("Delete", EditFlags::CAN_DELETE),
            ("Select All", EditFlags::CAN_SELECT_ALL),
        ];
        for bits in 0..(1u32 << all.len()) {
            let m = ContextMenu {
                editable: true,
                edit: EditFlags(bits),
                frame: "f1".into(),
                ..page()
            };
            let e = entries(&m, NavFlags::default());
            assert_eq!(labels(&e), field_labels());
            let plain = e.iter().find(|e| e.label == "Paste as Plain Text").unwrap();
            assert_eq!(plain.enabled, bits & EditFlags::CAN_PASTE != 0);
            assert_eq!(plain.command, Command::PastePlain("f1".into()));
            for (label, flag) in all {
                let entry = e.iter().find(|e| e.label == label).unwrap();
                assert_eq!(entry.enabled, bits & flag != 0, "{label} with {bits:#b}");
            }
        }
    }

    #[test]
    fn a_misspelled_word_lists_suggestions_first() {
        let m = ContextMenu {
            editable: true,
            misspelled_word: "teh".into(),
            suggestions: (0..7).map(|i| format!("s{i}")).collect(),
            ..page()
        };
        let e = entries(&m, NavFlags::default());
        // CEF numbers five suggestions.
        assert_eq!(labels(&e)[..6], ["s0", "s1", "s2", "s3", "s4", "-"]);
        assert_eq!(
            e[4].command,
            Command::Engine(cef_id::SPELLCHECK_SUGGESTION_LAST)
        );
        // Suggestions without a misspelled word are not shown.
        let m = ContextMenu {
            misspelled_word: String::new(),
            ..m
        };
        assert_eq!(
            labels(&entries(&m, NavFlags::default()))[0],
            field_labels()[0]
        );
    }

    /// Each row Chromium adds to a target, where it adds it: the whole menu's labels, dividers
    /// as "-".
    #[test]
    fn rows_follow_chromiums_groups() {
        let google = NavFlags {
            search: Some("Google"),
            image: Some("https://g.test/i?u={q}"),
            ..NavFlags::default()
        };
        let image = |url: &str| ContextMenu {
            media: MediaKind::Image,
            media_url: url.into(),
            has_image: true,
            ..page()
        };
        let spelling = |suggestions: &[&str]| ContextMenu {
            editable: true,
            misspelled_word: "teh".into(),
            suggestions: suggestions.iter().map(|s| s.to_string()).collect(),
            ..page()
        };
        let frame = |url: &str| ContextMenu {
            frame_url: url.into(),
            frame: "7".into(),
            ..page()
        };
        let selection = |page_url: &str| ContextMenu {
            selection_text: "hello world".into(),
            page_url: page_url.into(),
            ..page()
        };
        let image_rows = [
            "Open Image in New Tab",
            "Save Image As\u{2026}",
            "Copy Image",
            "Copy Image Address",
        ];
        let field_rows = field_labels();
        let bare = [
            "Back",
            "Forward",
            "Reload",
            "-",
            "Save Page As\u{2026}",
            "Print\u{2026}",
            "-",
        ];
        let tail = ["Take Screenshot of Element", "Inspect"];
        let search = "Search Google for \u{201c}hello world\u{201d}";
        let url_selection = ContextMenu {
            selection_text: " https://b.test/x ".into(),
            ..page()
        };
        let cases: [(&str, ContextMenu, NavFlags, Vec<&str>); 11] = [
            (
                "image search on an engine that has one",
                image("https://a.test/i.png"),
                google,
                [&image_rows[..], &["Search Google for Image", "-"], &tail].concat(),
            ),
            (
                "none on an engine without",
                image("https://a.test/i.png"),
                NavFlags::default(),
                [&image_rows[..], &["-"], &tail].concat(),
            ),
            (
                "none for an image that is not on the web",
                image("data:image/png;base64,AA=="),
                google,
                [&image_rows[..], &["-"], &tail].concat(),
            ),
            (
                "suggestions, then the dictionary",
                spelling(&["the", "tea"]),
                google,
                [
                    &["the", "tea", "-", "Add to Dictionary", "-"][..],
                    &field_rows,
                ]
                .concat(),
            ),
            (
                "nothing to suggest",
                spelling(&[]),
                google,
                [
                    &["No Spelling Suggestions", "-", "Add to Dictionary", "-"][..],
                    &field_rows,
                ]
                .concat(),
            ),
            (
                "a frame's rows follow the page's source",
                frame("https://b.test/f"),
                google,
                [
                    &bare[..],
                    &[
                        "View Page Source",
                        "View Frame Source",
                        "Reload Frame",
                        "Open Frame in New Tab",
                    ],
                    &tail,
                ]
                .concat(),
            ),
            (
                "the page's own frame has none",
                frame("https://a.test/"),
                google,
                [&bare[..], &["View Page Source"], &tail].concat(),
            ),
            (
                "nor a frame with no address",
                frame("about:srcdoc"),
                google,
                [&bare[..], &["View Page Source"], &tail].concat(),
            ),
            (
                "the highlight follows Copy",
                selection("https://a.test/"),
                google,
                [
                    &[
                        "Copy",
                        "Copy Link to Highlight",
                        "-",
                        search,
                        "Print\u{2026}",
                        "-",
                    ][..],
                    &tail,
                ]
                .concat(),
            ),
            (
                "none on a page that has no fragments",
                selection("wubble://start"),
                google,
                [&["Copy", "-", search, "Print\u{2026}", "-"][..], &tail].concat(),
            ),
            (
                "a selected address is gone to, not searched",
                url_selection,
                google,
                [
                    &[
                        "Copy",
                        "Copy Link to Highlight",
                        "-",
                        "Go to https://b.test/x",
                        "Print\u{2026}",
                        "-",
                    ][..],
                    &tail,
                ]
                .concat(),
            ),
        ];
        for (what, menu, nav, want) in cases {
            assert_eq!(labels(&entries(&menu, nav)), want, "{what}");
        }
    }

    #[test]
    fn the_new_rows_run_what_chrome_runs() {
        let google = NavFlags {
            search: Some("Google"),
            image: Some("https://g.test/i?u={q}"),
            ..NavFlags::default()
        };
        let command = |menu: &ContextMenu, label: &str| {
            entries(menu, google)
                .into_iter()
                .find(|e| e.label == label)
                .unwrap_or_else(|| panic!("{label}"))
                .command
        };
        let image = ContextMenu {
            media: MediaKind::Image,
            media_url: "https://a.test/i.png".into(),
            ..page()
        };
        assert_eq!(
            command(&image, "Search Google for Image"),
            Command::OpenInNewTab("https://g.test/i?u=https%3A%2F%2Fa.test%2Fi.png".into())
        );
        let word = ContextMenu {
            editable: true,
            misspelled_word: "teh".into(),
            ..page()
        };
        assert_eq!(
            command(&word, "Add to Dictionary"),
            Command::Engine(cef_id::ADD_TO_DICTIONARY)
        );
        let none = entries(&word, google);
        assert_eq!(none[0].command, Command::Info);
        assert!(!none[0].enabled);
        let selection = ContextMenu {
            selection_text: "hello world".into(),
            ..page()
        };
        assert_eq!(
            command(&selection, "Copy Link to Highlight"),
            Command::CopyText("https://a.test/#:~:text=hello%20world".into())
        );
        // the same selection in a frame: no fragment of the page names it
        let in_frame = ContextMenu {
            frame_url: "https://b.test/f".into(),
            ..selection.clone()
        };
        assert!(
            entries(&in_frame, google)
                .iter()
                .all(|e| e.label != "Copy Link to Highlight")
        );
        let frame = ContextMenu {
            frame_url: "https://b.test/f".into(),
            frame: "7".into(),
            ..page()
        };
        assert_eq!(
            command(&frame, "View Frame Source"),
            Command::OpenInNewTab("view-source:https://b.test/f".into())
        );
        assert_eq!(
            command(&frame, "Open Frame in New Tab"),
            Command::OpenInNewTab("https://b.test/f".into())
        );
        let address = ContextMenu {
            selection_text: "example.com/x".into(),
            ..page()
        };
        assert_eq!(
            command(&address, "Go to https://example.com/x"),
            Command::OpenInNewTab("https://example.com/x".into())
        );
        // not a web address: searched
        let script = ContextMenu {
            selection_text: "javascript:alert(1)".into(),
            ..page()
        };
        assert!(
            entries(&script, google)
                .iter()
                .all(|e| !e.label.starts_with("Go to"))
        );
        assert_eq!(
            command(&frame, "Reload Frame"),
            Command::FrameScript {
                frame: "7".into(),
                script: "location.reload()"
            }
        );
    }

    #[test]
    fn the_search_label_is_one_short_line() {
        assert_eq!(
            search_label(None, "  two\n words "),
            "Search the Web for \u{201c}two words\u{201d}"
        );
        let long = "x".repeat(60);
        let label = search_label(Some("Google"), &long);
        assert!(label.ends_with("\u{2026}\u{201d}"), "{label}");
        assert_eq!(
            label.chars().filter(|&c| c == 'x').count(),
            SEARCH_LABEL_CHARS
        );
    }

    #[test]
    fn the_save_control_with_nothing_to_save_says_so() {
        let entries = media_entries(&[], false);
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .all(|e| e.command == Command::Info && !e.enabled)
        );
    }

    #[test]
    fn media_entries_follow_the_element_state() {
        let video = |flags| ContextMenu {
            media: MediaKind::Video,
            media_url: "https://a.test/v.webm".into(),
            media_flags: MediaFlags(flags),
            x: 7,
            y: 9,
            ..page()
        };
        let e = entries(
            &video(MediaFlags::PAUSED | MediaFlags::CAN_TOGGLE_CONTROLS),
            NavFlags::default(),
        );
        assert_eq!(
            labels(&e),
            [
                "Play",
                "Loop",
                "Show Controls",
                "Picture in Picture",
                "-",
                "Open Video in New Tab",
                "Save Video As\u{2026}",
                "Copy Video Address",
                "Copy Video Frame",
                "Save Video Frame As\u{2026}",
                "-",
                "Take Screenshot of Element",
                "Inspect"
            ]
        );
        let command = |label: &str| e.iter().find(|e| e.label == label).map(|e| &e.command);
        assert_eq!(
            command("Picture in Picture"),
            Some(&Command::PictureInPicture { x: 7, y: 9 })
        );
        use wubble_core::shot::ShotOutput;
        assert_eq!(
            command("Copy Video Frame"),
            Some(&Command::VideoFrame {
                x: 7,
                y: 9,
                output: ShotOutput::Clipboard
            })
        );
        assert_eq!(
            command("Save Video Frame As\u{2026}"),
            Some(&Command::VideoFrame {
                x: 7,
                y: 9,
                output: ShotOutput::File
            })
        );
        // a video in a subframe: the kit cannot reach it at the view's point
        let framed = ContextMenu {
            frame_url: "https://b.test/player".into(),
            ..video(MediaFlags::PAUSED)
        };
        let framed = entries(&framed, NavFlags::default());
        assert!(
            framed
                .iter()
                .all(|e| e.label != "Picture in Picture" && !e.label.contains("Video Frame"))
        );
        // audio has neither
        let audio = ContextMenu {
            media: MediaKind::Audio,
            ..video(MediaFlags::PAUSED)
        };
        assert!(
            entries(&audio, NavFlags::default())
                .iter()
                .all(|e| e.label != "Picture in Picture")
        );
        assert_eq!(
            e[0].command,
            Command::Media {
                op: MediaOp::PlayPause,
                x: 7,
                y: 9,
                frame: String::new(),
                source: "https://a.test/v.webm".into(),
            }
        );
        assert!(e[2].enabled);
        // Saving needs CAN_SAVE.
        assert!(!e[6].enabled);
        let all =
            MediaFlags::HAS_AUDIO | MediaFlags::MUTED | MediaFlags::LOOP | MediaFlags::CONTROLS;
        let e = entries(&video(all), NavFlags::default());
        assert_eq!(
            labels(&e)[..4],
            ["Pause", "Unmute", "Stop Looping", "Hide Controls"]
        );
        assert!(!e[3].enabled);
    }

    #[test]
    fn media_scripts_act_on_the_element_under_the_point_or_by_its_source() {
        let s = media_script(MediaOp::Loop, 30, 40, "https://a.test/v\".mp4");
        assert!(
            s.contains("elementFromPoint(30 / devicePixelRatio, 40 / devicePixelRatio)"),
            "{s}"
        );
        assert!(
            s.contains("window === window.top"),
            "only the main frame uses the point: {s}"
        );
        assert!(
            s.contains(r#"m.currentSrc === "https://a.test/v\".mp4""#),
            "a subframe's element by its source, quoted: {s}"
        );
        assert!(s.contains("all.length === 1 ? all[0]"), "{s}");
        assert!(s.contains("m.loop = !m.loop"), "{s}");
    }

    #[test]
    fn engine_commands_answer_with_the_command() {
        let (s, fx) = transition(None, requested(1, page()));
        assert!(fx.is_empty());
        let (s, fx) = transition(s, MenuEvent::Chosen(2));
        assert_eq!(s, None);
        assert_eq!(
            fx,
            [MenuEffect::Answer {
                id: RequestId(1),
                command: Some(cef_id::RELOAD)
            }]
        );
    }

    #[test]
    fn app_commands_dismiss_then_run() {
        let (s, _) = transition(None, requested(1, link()));
        let (s, fx) = transition(s, MenuEvent::Chosen(0));
        assert_eq!(s, None);
        assert_eq!(
            fx,
            [
                MenuEffect::Answer {
                    id: RequestId(1),
                    command: None
                },
                MenuEffect::Run(Command::OpenInNewTab("https://a.test/next".into()))
            ]
        );
    }

    #[test]
    fn disabled_entries_and_separators_are_not_chosen() {
        // Back and Forward are disabled without history.
        let (s, _) = transition(None, requested(1, page()));
        let (s, fx) = transition(s, MenuEvent::Chosen(0));
        assert!(fx.is_empty());
        let (s, fx) = transition(s, MenuEvent::Chosen(99));
        assert!(fx.is_empty());
        assert!(matches!(s, Some(Menu::Open { .. })));
    }

    #[test]
    fn only_the_page_s_own_menu_keeps_its_focus() {
        let open = transition(None, requested(1, link())).0.unwrap();
        assert!(open.keeps_page_focus());
        let shell = Menu::Shell {
            at: (0, 0),
            entries: vec![],
        };
        assert!(
            !shell.keeps_page_focus(),
            "the shell's menu is not the page's"
        );
        // CEF closed it (window blurred, tab hidden): the page is free to blur.
        assert_eq!(transition(Some(open), MenuEvent::Reset).0, None);
    }

    #[test]
    fn every_request_is_answered_at_most_once() {
        use MenuEvent::*;
        let events = || {
            vec![
                requested(1, page()),
                requested(2, link()),
                Chosen(0),
                Chosen(0),
                Dismiss,
                requested(3, page()),
                Dismiss,
                Dismiss,
                requested(4, page()),
                Reset,
                Dismiss,
                requested(5, page()),
                Reset,
                Chosen(2),
            ]
        };
        let mut s: Option<Menu> = None;
        let mut answered = vec![];
        for e in events() {
            let (next, fx) = transition(s, e);
            s = next;
            for f in fx {
                if let MenuEffect::Answer { id, .. } = f {
                    assert!(!answered.contains(&id), "{id:?} answered twice");
                    answered.push(id);
                }
            }
        }
        // 4 and 5 were answered by CEF / welding, not by the menu.
        assert_eq!(answered, [RequestId(1), RequestId(2), RequestId(3)]);
    }

    #[test]
    fn a_tab_row_menu_maps_the_core_table_with_shortcuts_and_split_entries() {
        let keymap = Keymap::defaults(wubble_core::keys::Platform::Linux);
        let m = tabs(3);
        let visible = m.visible_tab().unwrap();
        let e = tab_entries(&m, visible, Facts::default(), &keymap);
        let labels: Vec<&str> = e.iter().map(|e| e.label.as_str()).collect();
        assert!(labels.contains(&"Pin Tab") && labels.contains(&"Duplicate Tab"));
        assert!(
            labels.contains(&"Open Beside\u{2026}"),
            "split entries spliced in"
        );
        let close = e.iter().find(|e| e.label == "Close Tab").unwrap();
        assert!(close.danger && !close.shortcut.is_empty());
        assert_eq!(
            close.command,
            Command::Model(TabCommand(ModelCommand::CloseTab(visible)))
        );
        let mute = e.iter().find(|e| e.label == "Mute Tab").unwrap();
        assert_eq!(mute.command, Command::ToggleMute(visible));
        let site = e.iter().find(|e| e.label == "Mute Site").unwrap();
        assert_eq!(site.command, Command::ToggleMuteSite(visible));
        assert!(labels.contains(&"New Tab Below"));
        // dividers never lead, trail or double
        assert!(e.first().unwrap().command != Command::Separator);
        assert!(e.last().unwrap().command != Command::Separator);
        assert!(
            e.windows(2).all(
                |w| !(w[0].command == Command::Separator && w[1].command == Command::Separator)
            )
        );
        // a hidden tab gets no shortcuts (they act on the visible one)
        let hidden = tab_entries(&m, TabId(1), Facts::default(), &keymap);
        assert!(hidden.iter().all(|e| e.shortcut.is_empty()));
    }

    fn tabs(n: usize) -> Model {
        use wubble_core::model::{Command as Cmd, apply};
        (0..n).fold(Model::new(std::time::Instant::now()), |m, i| {
            apply(
                m,
                Cmd::NewTab {
                    url: format!("https://{i}.test/"),
                    container: None,
                    opener: None,
                },
            )
            .0
        })
    }

    #[test]
    fn a_tab_row_offers_the_split_entries_that_apply() {
        use wubble_core::model::{Command as Cmd, apply};
        let commands = |m: &Model, t: u64| -> Vec<Command> {
            split_entries(m, TabId(t))
                .into_iter()
                .map(|e| e.command)
                .collect()
        };
        let m = tabs(1);
        assert_eq!(commands(&m, 1), [Command::OpenBeside]);
        let m = tabs(3);
        assert_eq!(
            commands(&m, 3),
            [Command::Perform(Action::SplitNext), Command::OpenBeside]
        );
        assert_eq!(commands(&m, 1), [Command::SplitWith(TabId(1))]);
        let (m, _) = apply(m, Cmd::SplitWith(TabId(1)));
        assert_eq!(commands(&m, 1), [Command::Unsplit(TabId(1))]);
        assert_eq!(commands(&m, 3), [Command::Unsplit(TabId(3))]);
        let (m, _) = apply(m, Cmd::SplitWith(TabId(2)));
        assert_eq!(commands(&m, 2), [Command::Unsplit(TabId(2))]);
        // a full split takes no more
        let (m, _) = apply(m, Cmd::Unsplit(TabId(2)));
        let (m, _) = apply(m, Cmd::SplitWith(TabId(2)));
        assert!(commands(&m, 99).is_empty());
    }

    #[test]
    fn a_tab_row_offers_the_other_windows() {
        use wubble_core::model::{Command as Cmd, apply};
        use wubble_core::window::WindowId;
        let m = tabs(2);
        let rows = |m: &Model, t: u64| -> Vec<(String, Command)> {
            window_entries(m, TabId(t))
                .into_iter()
                .map(|e| (e.label, e.command))
                .collect()
        };
        let new = |t| {
            (
                "Move to New Window".to_string(),
                Command::MoveToWindow(TabId(t), WindowTarget::New),
            )
        };
        assert_eq!(rows(&m, 1), [new(1)]);
        let (m, _) = apply(m, Cmd::NewWindow { space: None });
        let to_first = (
            "Move to Window: Space 1".to_string(),
            Command::MoveToWindow(TabId(1), WindowTarget::Existing(WindowId(0))),
        );
        let to_second = (
            "Move to Window: Space 2".to_string(),
            Command::MoveToWindow(TabId(1), WindowTarget::Existing(WindowId(1))),
        );
        assert_eq!(rows(&m, 1), [new(1), to_second]);
        let (m, _) = apply(
            m,
            Cmd::MoveTabToWindow {
                tab: TabId(1),
                to: WindowTarget::Existing(WindowId(1)),
            },
        );
        // the window showing its Space is not offered; the other one is
        assert_eq!(rows(&m, 1), [new(1), to_first]);
        // the tab menu carries them (the table's window slot)
        let all = tab_entries(
            &m,
            TabId(1),
            Facts::default(),
            &Keymap::defaults(wubble_core::keys::Platform::Linux),
        );
        assert!(all.iter().any(|e| e.command == rows(&m, 1)[1].1));
    }

    #[test]
    fn the_tab_menu_offers_never_sleep_for_the_space_and_the_site() {
        use wubble_core::model::{Command as Cmd, apply};
        let m = tabs(1);
        let rows = |m: &Model| -> Vec<(String, Command)> {
            sleep_entries(m, TabId(1))
                .into_iter()
                .map(|e| (e.label, e.command))
                .collect()
        };
        let space = m.spaces[0].id;
        let host = wubble_core::routing::host_key(&m.tabs[&TabId(1)].url).unwrap();
        assert_eq!(
            rows(&m),
            [
                (
                    "Never Sleep Tabs in This Space".to_string(),
                    Command::SpaceNeverSleep(space, true)
                ),
                (
                    format!("Never Sleep {host}"),
                    Command::ToggleNeverSleepSite(host.clone())
                ),
            ]
        );
        let (m, _) = apply(m, Cmd::SetSpaceNeverSleep { space, on: true });
        let (m, _) = apply(
            m,
            Cmd::SetNeverSleepSites {
                side: Side::Normal,
                sites: vec![host.clone()],
            },
        );
        assert_eq!(
            rows(&m),
            [
                (
                    "Let Tabs in This Space Sleep".to_string(),
                    Command::SpaceNeverSleep(space, false)
                ),
                (
                    format!("Let {host} Sleep"),
                    Command::ToggleNeverSleepSite(host.clone())
                ),
            ]
        );
        // the tab menu carries them (the table's sleep slot)
        let all = tab_entries(
            &m,
            TabId(1),
            Facts::default(),
            &Keymap::defaults(wubble_core::keys::Platform::Linux),
        );
        assert!(all.iter().any(|e| e.command == rows(&m)[1].1));
    }

    #[test]
    fn little_and_popup_windows_offer_a_way_out() {
        use wubble_core::model::{Command as Cmd, apply};
        use wubble_core::space::{ContainerId, Rgb};
        let (m, _) = apply(
            tabs(1),
            Cmd::NewSpace {
                name: "Work".into(),
                tint: Rgb(0, 0, 0),
                container: ContainerId::DEFAULT,
            },
        );
        assert!(
            window_page_entries(&m, WindowId(0)).is_empty(),
            "a Main window"
        );
        let (m, _) = apply(
            m,
            Cmd::OpenLittle {
                url: "https://l.test".into(),
                space: None,
                container: None,
            },
        );
        let little = m.focused();
        let rows: Vec<(String, Command)> = window_page_entries(&m, little)
            .into_iter()
            .map(|e| (e.label, e.command))
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "Open as Tab".to_string(),
                    Command::PromoteLittle(little, m.spaces[0].id)
                ),
                (
                    "Open in Work".to_string(),
                    Command::PromoteLittle(little, m.spaces[1].id)
                ),
            ]
        );
        let (m, _) = apply(
            m,
            Cmd::OpenPopupWindow {
                url: "https://p.test".into(),
                opener: TabId(1),
                size: None,
                pos: None,
            },
        );
        let popup = m.focused();
        let tab = m.window(popup).unwrap().tab().unwrap();
        assert_eq!(
            window_page_entries(&m, popup)[0].command,
            Command::MoveToWindow(tab, WindowTarget::Existing(WindowId(0)))
        );
    }

    #[test]
    fn the_popup_menu() {
        let e = popup_entries(2, Some("a.test"));
        assert_eq!(e[0].label, "Open 2 Blocked Pop-ups");
        assert_eq!(e[0].command, Command::OpenBlockedPopups);
        assert_eq!(e[1].command, Command::AllowPopups("a.test".into()));
        assert_eq!(
            popup_entries(1, None).len(),
            1,
            "an opaque origin is not remembered"
        );
    }

    #[test]
    fn the_shell_s_menu_runs_its_entry_and_answers_nothing() {
        let entries = split_entries(&tabs(3), TabId(1));
        let (open, fx) = transition(
            None,
            MenuEvent::ShellRequested {
                at: (5, 6),
                entries: entries.clone(),
            },
        );
        assert!(fx.is_empty());
        assert_eq!(
            open,
            Some(Menu::Shell {
                at: (5, 6),
                entries
            })
        );
        let (closed, fx) = transition(open.clone(), MenuEvent::Chosen(0));
        assert_eq!(closed, None);
        assert_eq!(fx, [MenuEffect::Run(Command::SplitWith(TabId(1)))]);
        assert_eq!(transition(open.clone(), MenuEvent::Chosen(9)).0, open);
        assert_eq!(transition(open, MenuEvent::Dismiss), (None, vec![]));
        // nothing to show: no menu
        let (state, fx) = transition(
            None,
            MenuEvent::ShellRequested {
                at: (0, 0),
                entries: vec![],
            },
        );
        assert_eq!((state, fx), (None, vec![]));
        // the shell's menu replaces a page's, which is answered
        let (page, _) = transition(None, requested(1, page()));
        let (_, fx) = transition(
            page,
            MenuEvent::ShellRequested {
                at: (0, 0),
                entries: split_entries(&tabs(1), TabId(1)),
            },
        );
        assert_eq!(
            fx,
            [MenuEffect::Answer {
                id: RequestId(1),
                command: None
            }]
        );
    }
}
