//! The host's half of the built-ins that replace extensions: dark mode for pages, reader mode,
//! keyboard link hints and screenshots. Each has its pure machine in `wubble_core` (`pagetheme`,
//! `reader`, `hints`, `shot`); here their effects reach the page (DevTools calls and page
//! scripts through `View::cdp` and `View::eval`, `pagescripts.rs`), and the answers come back as
//! events. The state here is the one owner of each built-in's state while the app runs.

use std::collections::{BTreeMap, BTreeSet};

use slint::ComponentHandle;
use slint::platform::PointerEventButton;
use slint::winit_030::winit::keyboard::{Key, NamedKey};
use wubble_core::hints::{self, HintEffect, HintEvent, HintKey, Hints};
use wubble_core::pagetheme::{self, MediaPrefs, PageTheme, Shown, ThemeTable};
use wubble_core::reader::{
    self, Reader, ReaderColors, ReaderEffect, ReaderEvent, ReaderPrefs, ScrollAt,
};
use wubble_core::settings as defs;
use wubble_core::shot::{self, Shot, ShotEffect, ShotEvent, ShotOutput, ShotRequest};
use wubble_core::tab::TabId;
use wubble_engine::input::{KeyInput, Point};
use wubble_engine::nav::WebEvent;
use wubble_engine::page::PageEvent;
use wubble_ui::{AppWindow, Palette};

use super::{Host, new_tab};
use crate::builtin_smoke as smoke;
use crate::pagescripts::{self, HintColors, hex};
use crate::shot_io::{self, Saved, Saver};

/// What the built-ins hold.
pub(super) struct Builtins {
    /// `settings::PAGE_THEME` and `settings::PAGE_THEME_SITES`.
    theme_default: PageTheme,
    theme_sites: ThemeTable,
    /// `settings::PAGES_MOTION` and `settings::PAGES_CONTRAST`, shown on every page.
    media: MediaPrefs,
    /// What each live view was last told (`pagetheme::sync`); absent: nothing yet.
    applied: BTreeMap<TabId, Shown>,
    /// `settings::PAGES_MIN_FONT_SIZE`.
    min_font: u32,
    /// The page-theme calls in flight, to report a refused one.
    theme_calls: BTreeSet<u64>,
    reader: BTreeMap<TabId, Reader>,
    reader_prefs: ReaderPrefs,
    /// Tabs showing a PDF in Chromium's viewer, and the content-type questions in flight.
    pdf: BTreeSet<TabId>,
    kind_probes: BTreeMap<u64, TabId>,
    /// Hint mode and the tab whose page shows it; `None`: off.
    hints: Option<(TabId, Hints)>,
    alphabet: &'static str,
    /// The screenshot in flight and the tab it is of; `None`: idle.
    shot: Option<(TabId, Shot)>,
    output: ShotOutput,
    /// Started on the first screenshot.
    saver: Option<Saver>,
}

impl Default for Builtins {
    fn default() -> Self {
        Self {
            theme_default: PageTheme::Auto,
            theme_sites: ThemeTable::new(),
            media: MediaPrefs::default(),
            applied: BTreeMap::new(),
            min_font: 0,
            theme_calls: BTreeSet::new(),
            reader: BTreeMap::new(),
            reader_prefs: ReaderPrefs::default(),
            pdf: BTreeSet::new(),
            kind_probes: BTreeMap::new(),
            hints: None,
            alphabet: hints::alphabet(""),
            shot: None,
            output: ShotOutput::Both,
            saver: None,
        }
    }
}

/// A built-in action on the visible page (`perform::Plan::Builtin`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Builtin {
    CyclePageTheme,
    LinkHints,
    ToggleReader,
    Shot(ShotRequest),
    /// The Boost editor on a new Boost for the visible site.
    NewBoost,
    /// The menu of what browsing data to clear (`Host::clear_data_menu`).
    ClearData,
}

impl Host {
    pub(super) fn builtin(&mut self, action: Builtin) {
        let Some(tab) = self.shown() else {
            return;
        };
        match action {
            Builtin::CyclePageTheme => self.cycle_page_theme(tab),
            Builtin::LinkHints => self.hint_event(tab, HintEvent::Toggle),
            Builtin::ToggleReader => self.reader_event(tab, ReaderEvent::Toggle),
            Builtin::Shot(request) => self.shot_event(tab, ShotEvent::Request(request)),
            Builtin::NewBoost => self.new_boost(),
            Builtin::ClearData => self.clear_data_menu(tab),
        }
    }

    // ----- settings

    pub(super) fn set_page_themes(
        &mut self,
        default: PageTheme,
        sites: ThemeTable,
        media: MediaPrefs,
    ) {
        self.builtins.theme_default = default;
        self.builtins.theme_sites = sites;
        self.builtins.media = media;
        let tabs: Vec<_> = self.views.keys().copied().collect();
        for tab in tabs {
            self.sync_page_theme(tab, None);
        }
        self.redraw_builtins();
    }

    pub(super) fn set_min_font_size(&mut self, px: u32) {
        self.builtins.min_font = px;
        if let Some(tab) = self.views.keys().next().copied() {
            self.sync_min_font(tab);
        }
    }

    /// Tells Chromium the minimum font size through `tab`'s view. A preference of the one
    /// request context every view shares, so one live view is enough: on a change, and when
    /// the first view opens (none was there to take it before).
    fn sync_min_font(&mut self, tab: TabId) {
        let px = self.builtins.min_font;
        if let Some(view) = self.tab_view(tab)
            && let Err(e) = view.set_minimum_font_size(px)
        {
            eprintln!("wubble: minimum font size: {e}");
        }
    }

    pub(super) fn set_reader_prefs(&mut self, prefs: ReaderPrefs) {
        self.builtins.reader_prefs = prefs;
        let on: Vec<_> = self
            .builtins
            .reader
            .iter()
            .filter(|(_, r)| r.is_on())
            .map(|(t, _)| *t)
            .collect();
        let script = self
            .w
            .window
            .upgrade()
            .map(|w| pagescripts::reader_set(&reader::css_vars(&prefs, &dark_colors(&w))));
        for tab in on {
            if let (Some(script), Some(view)) = (&script, self.tab_view(tab)) {
                view.eval(script);
            }
        }
        self.redraw_builtins();
    }

    pub(super) fn set_shot_output(&mut self, output: ShotOutput) {
        self.builtins.output = output;
    }

    pub(super) fn set_hint_alphabet(&mut self, alphabet: &'static str) {
        self.builtins.alphabet = alphabet;
    }

    // ----- engine events

    /// A view opened for `tab` on `url`: it was told nothing yet.
    pub(super) fn builtins_view_opened(&mut self, tab: TabId, url: &str) {
        self.builtins.applied.remove(&tab);
        self.sync_page_theme(tab, Some(url));
        if self.views.len() == 1 {
            self.sync_min_font(tab);
        }
        self.boosts_view_opened(tab);
    }

    /// `tab` is about to load `url`: its theme goes first, so the page never shows in the
    /// other one.
    pub(super) fn builtins_loading(&mut self, tab: TabId, url: &str) {
        self.sync_page_theme(tab, Some(url));
    }

    /// The tab's view is gone (closed, discarded): forget what it was told.
    pub(super) fn builtins_view_gone(&mut self, tab: TabId) {
        self.builtins.applied.remove(&tab);
        self.builtins.pdf.remove(&tab);
        self.builtins.reader.remove(&tab);
        if self.hints_tab() == Some(tab) {
            self.builtins.hints = None;
        }
        self.shot_page_gone(tab);
        self.boosts_view_gone(tab);
        self.media_view_gone(tab);
    }

    /// No answer can come from `tab`'s page any more: a screenshot of it ends.
    fn shot_page_gone(&mut self, tab: TabId) {
        if self.shot_tab() == Some(tab) {
            self.shot_event(tab, ShotEvent::PageGone);
        }
    }

    /// What the built-ins take from navigation. Called before the host's own handling.
    pub(super) fn builtins_web(&mut self, tab: TabId, event: &WebEvent) {
        match event {
            WebEvent::UrlChanged(url) => {
                self.sync_page_theme(tab, Some(url));
                self.boosts_document(tab);
            }
            WebEvent::LoadStarted => {
                self.media_page_changed(tab);
                self.boosts_document(tab);
                self.builtins.pdf.remove(&tab);
                self.reader_event(tab, ReaderEvent::PageStarted);
                if self.hints_tab() == Some(tab) {
                    self.hint_event(tab, HintEvent::Cancel);
                }
                if self.shot_tab() == Some(tab) {
                    self.shot_event(tab, ShotEvent::Cancel);
                }
            }
            WebEvent::LoadingChanged { loading: false, .. } => {
                if let Some(id) = self
                    .tab_view(tab)
                    .and_then(|v| v.eval(pagescripts::CONTENT_TYPE))
                {
                    self.builtins.kind_probes.insert(id.0, tab);
                }
                let detect = self.builtins.reader_prefs.detect;
                self.reader_event(tab, ReaderEvent::Loaded { detect });
            }
            // A new renderer was told nothing; nor is its page the reader any more.
            WebEvent::RendererGone => {
                self.builtins.applied.remove(&tab);
                self.reader_event(tab, ReaderEvent::PageStarted);
                self.shot_page_gone(tab);
            }
            _ => {}
        }
    }

    /// The answers to the built-ins' calls and scripts.
    pub(super) fn builtins_answer(&mut self, tab: TabId, event: &PageEvent) {
        match event {
            PageEvent::CdpReply { id, result } => {
                if self.builtins.theme_calls.remove(&id.0) {
                    if let Err(e) = result {
                        eprintln!("wubble: page theme refused: {e}");
                    }
                    return;
                }
                if self.shot_state() == Shot::Capturing(id.0) {
                    let data = result.as_ref().map_err(Clone::clone).and_then(|v| {
                        v.get("data")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .ok_or_else(|| "no image in the answer".to_string())
                    });
                    if let Err(e) = &data {
                        eprintln!("wubble: screenshot: {e}");
                    }
                    self.shot_event(tab, ShotEvent::Captured(id.0, data));
                }
            }
            PageEvent::EvalResult { id, result } => {
                let id = id.0;
                let text = result.as_deref().ok();
                if self.builtins.kind_probes.remove(&id).is_some() {
                    if text == Some(pagescripts::PDF_TYPE) {
                        self.builtins.pdf.insert(tab);
                    }
                    return;
                }
                match self.builtins.reader.get(&tab).copied() {
                    Some(Reader::Probing(want)) if want == id => {
                        let readerable = text == Some("true");
                        self.reader_event(tab, ReaderEvent::Probed(id, readerable));
                        return;
                    }
                    Some(Reader::Leaving {
                        script: Some(want), ..
                    }) if want == id => {
                        if let Err(e) = result {
                            eprintln!("wubble: reader: {e}");
                        }
                        self.reader_event(tab, ReaderEvent::Left(id));
                        return;
                    }
                    Some(Reader::Extracting(want)) if want == id => {
                        let built = text
                            .and_then(pagescripts::reader_opened)
                            .map(|(x, y)| ScrollAt { x, y });
                        if let Err(e) = result {
                            eprintln!("wubble: reader: {e}");
                        }
                        self.reader_event(tab, ReaderEvent::Extracted(id, built));
                        return;
                    }
                    _ => {}
                }
                if matches!(&self.builtins.hints, Some((_, Hints::Collecting(want))) if *want == id)
                {
                    let targets = text.map(hints::targets_from_json).unwrap_or_default();
                    self.hint_event(tab, HintEvent::Targets(id, targets));
                    return;
                }
                if matches!(self.shot_state(), Shot::Measuring { id: want, .. } if want == id) {
                    let metrics = text.and_then(shot::Metrics::from_json);
                    self.shot_event(tab, ShotEvent::Measured(id, metrics));
                }
            }
            _ => {}
        }
    }

    /// The tab shows a PDF in Chromium's viewer (its document's content type).
    pub(super) fn is_pdf(&self, tab: TabId) -> bool {
        self.builtins.pdf.contains(&tab)
    }

    // ----- dark mode for pages

    /// Tells `tab`'s view the theme its page (`url`, else the one it shows) wants, if it was
    /// told another.
    fn sync_page_theme(&mut self, tab: TabId, url: Option<&str>) {
        let url = match url {
            Some(u) => u.to_owned(),
            None => match self.tab_view(tab) {
                Some(view) => view.nav().page.url.clone(),
                None => return,
            },
        };
        let b = &self.builtins;
        let wanted = (
            pagetheme::wanted(&b.theme_sites, &url, b.theme_default),
            b.media,
        );
        let Some(shown) = pagetheme::sync(b.applied.get(&tab).copied(), wanted) else {
            return;
        };
        let Some(view) = self.tab_view(tab) else {
            return;
        };
        let mut sent = vec![];
        for (method, params) in pagetheme::calls(shown) {
            if let Some(id) = view.cdp(method, &params) {
                sent.push(id.0);
            }
        }
        if sent.len() == 2 {
            self.builtins.applied.insert(tab, shown);
        }
        self.builtins.theme_calls.extend(sent);
    }

    /// The visible site's theme, one step on (the action and the pill menu's rows).
    fn cycle_page_theme(&mut self, tab: TabId) {
        let url = self.model.tabs.get(&tab).map(|t| t.url.clone());
        let Some(url) = url else { return };
        let current = pagetheme::wanted(
            &self.builtins.theme_sites,
            &url,
            self.builtins.theme_default,
        );
        match wubble_core::zoom::host_of(&url) {
            Some(host) => self.set_site_theme(&host, current.next()),
            None => {
                if let Some(window) = self.w.window.upgrade() {
                    self.toast(&window, "This page has no site to set a theme for");
                }
            }
        }
    }

    /// The pill menu's choice: `host`'s pages show as `theme`.
    pub(super) fn set_site_theme(&mut self, host: &str, theme: PageTheme) {
        let table = pagetheme::next_table(
            self.builtins.theme_sites.clone(),
            host,
            theme,
            self.builtins.theme_default,
        );
        // Applied as the setting: the table becomes `theme_sites` there.
        self.write_setting(
            defs::PAGE_THEME_SITES.key,
            &wubble_store::text_of::<wubble_store::PageThemeSites>(&table),
        );
        if let Some(window) = self.w.window.upgrade() {
            self.toast(&window, &format!("Page theme: {}", theme.label()));
        }
    }

    /// The page theme the site menu marks for `url`.
    pub(super) fn page_theme_of(&self, url: &str) -> PageTheme {
        pagetheme::wanted(&self.builtins.theme_sites, url, self.builtins.theme_default)
    }

    // ----- reader mode

    fn reader_event(&mut self, tab: TabId, event: ReaderEvent) {
        let state = self.builtins.reader.get(&tab).copied().unwrap_or_default();
        let (next, effects) = reader::transition(state, event);
        self.builtins.reader.insert(tab, next);
        for effect in effects {
            match effect {
                ReaderEffect::Probe => {
                    let id = self
                        .tab_view(tab)
                        .and_then(|v| v.eval(&pagescripts::reader_probe()));
                    if let Some(id) = id {
                        self.reader_event(tab, ReaderEvent::ProbeSent(id.0));
                    }
                }
                ReaderEffect::Extract => {
                    let Some(window) = self.w.window.upgrade() else {
                        continue;
                    };
                    let vars = reader::css_vars(&self.builtins.reader_prefs, &dark_colors(&window));
                    let id = self
                        .tab_view(tab)
                        .and_then(|v| v.eval(&pagescripts::reader_open(&vars)));
                    if let Some(id) = id {
                        self.reader_event(tab, ReaderEvent::ExtractSent(id.0));
                    }
                }
                ReaderEffect::Leave => {
                    let id = self
                        .tab_view(tab)
                        .and_then(|v| v.eval(&pagescripts::reader_leave()));
                    let event = match id {
                        Some(id) => ReaderEvent::LeaveSent(id.0),
                        // no page to leave
                        None => ReaderEvent::PageStarted,
                    };
                    self.reader_event(tab, event);
                }
                ReaderEffect::Reload => {
                    if let Some(view) = self.tab_view(tab) {
                        view.reload();
                    }
                }
                ReaderEffect::Restore(at) => {
                    if let Some(view) = self.tab_view(tab) {
                        view.eval(&pagescripts::reader_restore(at.x, at.y));
                    }
                }
                ReaderEffect::Toast(text) => {
                    if let Some(window) = self.w.window.upgrade() {
                        self.toast(&window, text);
                    }
                }
            }
        }
        if next != state {
            self.redraw_builtins();
        }
    }

    /// The reader's controls: a `reader.*` setting to `value`, a number or a choice's index.
    pub(crate) fn reader_setting(&mut self, key: &str, value: i32) {
        let Some(def) = defs::find(key).filter(|d| d.key.starts_with("reader.")) else {
            return;
        };
        let text = match def.kind {
            defs::SettingKind::Choice(words) => usize::try_from(value)
                .ok()
                .and_then(|i| words.get(i))
                .map(|w| (*w).to_owned()),
            _ => Some(value.to_string()),
        };
        if let Some(text) = text {
            self.write_setting(key, &text);
        }
    }

    /// The pill's book glyph and the controls' close button.
    pub(crate) fn reader_toggled(&mut self) {
        self.perform(wubble_core::action::Action::ToggleReader);
    }

    // ----- link hints

    /// The tab whose page shows hints.
    fn hints_tab(&self) -> Option<TabId> {
        self.builtins.hints.as_ref().map(|(t, _)| *t)
    }

    fn hint_event(&mut self, tab: TabId, event: HintEvent) {
        // Hints live on one page; anything for another first clears that one.
        let state = match self.builtins.hints.take() {
            Some((on, state)) if on == tab => state,
            Some((other, state)) => {
                let (_, effects) =
                    hints::transition(state, HintEvent::Cancel, self.builtins.alphabet);
                self.hint_effects(other, effects);
                Hints::Off
            }
            None => Hints::Off,
        };
        let (next, effects) = hints::transition(state, event, self.builtins.alphabet);
        self.builtins.hints = (next != Hints::Off).then_some((tab, next));
        self.hint_effects(tab, effects);
    }

    fn hint_effects(&mut self, tab: TabId, effects: Vec<HintEffect>) {
        for effect in effects {
            match effect {
                HintEffect::Collect => {
                    let id = self
                        .tab_view(tab)
                        .and_then(|v| v.eval(&pagescripts::hints_collect()));
                    if let Some(id) = id {
                        self.hint_event(tab, HintEvent::Sent(id.0));
                    }
                }
                HintEffect::Draw { items, typed } => {
                    let Some(window) = self.w.window.upgrade() else {
                        continue;
                    };
                    let items: Vec<_> = items.into_iter().map(|(t, l)| ((t.x, t.y), l)).collect();
                    let script = pagescripts::hints_draw(&items, &typed, &hint_colors(&window));
                    if let Some(view) = self.tab_view(tab) {
                        view.eval(&script);
                    }
                }
                HintEffect::Clear => {
                    if let Some(view) = self.tab_view(tab) {
                        view.eval(&pagescripts::hints_clear());
                    }
                }
                HintEffect::Click(target) => self.hint_click(tab, target.centre()),
                HintEffect::OpenInNewTab(url) => self.dispatch(new_tab(url, Some(tab))),
                HintEffect::Toast(text) => {
                    if let Some(window) = self.w.window.upgrade() {
                        self.toast(&window, text);
                    }
                }
            }
        }
    }

    /// A real click (trusted: user activation, popups, `<select>` and file inputs behave) at a
    /// point of the page's viewport, CSS px.
    fn hint_click(&mut self, tab: TabId, (x, y): (f64, f64)) {
        let zoom = self.tab_view(tab).map_or(100, |v| v.zoom_percent());
        let f = f64::from(zoom) / 100.0;
        let at = Point {
            x: (x * f) as f32,
            y: (y * f) as f32,
        };
        let pane = crate::shell::pane_id(tab);
        self.pointer_moved(pane, at);
        self.pointer_button(pane, PointerEventButton::Left, true, at);
        self.pointer_button(pane, PointerEventButton::Left, false, at);
    }

    /// A key while hint mode is on takes the key from everything else: a letter types, Escape
    /// leaves, Backspace takes one back. Returns whether it was taken.
    /// A chord with Ctrl, Alt or Super is not taken: it goes on to the keymap (the hint
    /// action's own chord turns hints off there).
    pub(super) fn hint_key(&mut self, key: &KeyInput) -> bool {
        let Some(tab) = self.hints_tab() else {
            return false;
        };
        if self.shown() != Some(tab) {
            self.hint_event(tab, HintEvent::Cancel);
            return false;
        }
        let m = key.modifiers.keys;
        let chord = m.control_key() || m.alt_key() || m.super_key();
        let hint_key = match &key.unmodified {
            Key::Named(NamedKey::Escape) => Some(HintKey::Escape),
            Key::Named(NamedKey::Backspace) => Some(HintKey::Backspace),
            Key::Character(c) => c.chars().next().map(HintKey::Char),
            _ => None,
        };
        // Modifiers and the rest: kept from the page while the labels show, unless a chord.
        let Some(hint_key) = hint_key else {
            return !chord;
        };
        let Some(event) = hints::key_event(hint_key, chord, m.shift_key()) else {
            return false;
        };
        if key.pressed {
            self.hint_event(tab, event);
        }
        true
    }

    /// `--smoke builtins` types a label as the keys would.
    pub(crate) fn hint_typed(&mut self, ch: char, shift: bool) {
        if let Some(tab) = self.hints_tab() {
            self.hint_event(tab, HintEvent::Key(ch, shift));
        }
    }

    /// The page scrolled under the labels: they no longer sit on their targets.
    pub(super) fn hints_scrolled(&mut self) {
        if let Some(tab) = self.hints_tab() {
            self.hint_event(tab, HintEvent::Cancel);
        }
    }

    // ----- screenshots

    /// The tab a screenshot is in flight of.
    fn shot_tab(&self) -> Option<TabId> {
        self.builtins.shot.map(|(t, _)| t)
    }

    fn shot_state(&self) -> Shot {
        self.builtins.shot.map(|(_, s)| s).unwrap_or_default()
    }

    fn shot_event(&mut self, tab: TabId, event: ShotEvent) {
        // One screenshot at a time, of the page it started on.
        let (tab, state) = self.builtins.shot.unwrap_or((tab, Shot::Idle));
        let (next, effects) = shot::transition(state, event);
        self.builtins.shot = (next != Shot::Idle).then_some((tab, next));
        for effect in effects {
            match effect {
                ShotEffect::ShowOverlay | ShotEffect::HideOverlay => self.redraw_builtins(),
                ShotEffect::Measure { at, rect } => {
                    let id = self
                        .tab_view(tab)
                        .and_then(|v| v.eval(&shot::measure_script(at)));
                    let event = match id {
                        Some(id) => ShotEvent::MeasureSent { id: id.0, rect },
                        None => ShotEvent::SendFailed,
                    };
                    self.shot_event(tab, event);
                }
                ShotEffect::Capture(capture) => {
                    let id = self
                        .tab_view(tab)
                        .and_then(|v| v.cdp("Page.captureScreenshot", &shot::params(capture)));
                    let event = match id {
                        Some(id) => ShotEvent::CaptureSent(id.0),
                        None => ShotEvent::SendFailed,
                    };
                    self.shot_event(tab, event);
                }
                ShotEffect::Save(png) => self.save_shot(tab, png, false),
                ShotEffect::Toast(text) => {
                    if let Some(window) = self.w.window.upgrade() {
                        self.toast(&window, text);
                    }
                }
            }
        }
    }

    /// Hands the PNG to the saver thread: the file in the downloads folder, the clipboard, as
    /// the setting says. Its outcome comes back on the Slint thread as `Saved`. `frame`: a video
    /// frame from the video controls, not the screenshot machine's.
    pub(super) fn save_shot(&mut self, tab: TabId, png: String, frame: bool) {
        let output = self.builtins.output;
        let title = self
            .model
            .tabs
            .get(&tab)
            .map(crate::shell::tab_title)
            .unwrap_or_default();
        let name = shot::file_name(&title, chrono::Local::now().naive_local());
        let path = output.file().then(|| self.download_path(&name)).flatten();
        let job = shot_io::Job {
            picture: shot_io::Picture::Png {
                base64: png,
                path: path.clone(),
            },
            clipboard: output.clipboard(),
        };
        if let Some(path) = &path {
            self.downloads.reserve(path);
        }
        let file_wanted = output.file();
        let saver = self.saver();
        saver.save(job, move |mut saved| {
            if file_wanted && saved.file.is_none() {
                // No downloads folder to write to.
                saved.file = Some(false);
            }
            let _ = slint::invoke_from_event_loop(move || {
                crate::with_host(|host| host.shot_saved(tab, path, saved, frame));
            });
        });
    }

    /// The worker that owns the clipboard's images (screenshots, Copy Image).
    fn saver(&mut self) -> &Saver {
        self.builtins.saver.get_or_insert_with(Saver::spawn)
    }

    /// Copy Image's pixels arrived (`View::download_image`): onto the clipboard, on the worker
    /// that owns the clipboard's images; the toast says how it went.
    pub(super) fn copy_image_done(
        &mut self,
        window: &AppWindow,
        image: Result<wubble_engine::favicon::Rgba, String>,
    ) {
        let image = match image {
            Ok(image) => image,
            Err(e) => {
                eprintln!("wubble: copy image: {e}");
                return self.toast(window, "Could not copy the image");
            }
        };
        let job = shot_io::Job {
            picture: shot_io::Picture::Rgba {
                width: image.width,
                height: image.height,
                bytes: image.rgba,
            },
            clipboard: true,
        };
        self.saver().save(job, |saved| {
            let _ = slint::invoke_from_event_loop(move || {
                crate::with_host(|host| {
                    if saved.clipboard != Some(true)
                        && let Some(window) = host.w.window.upgrade()
                    {
                        host.toast(&window, "Could not copy the image");
                    }
                });
            });
        });
    }

    fn shot_saved(
        &mut self,
        tab: TabId,
        path: Option<std::path::PathBuf>,
        saved: Saved,
        frame: bool,
    ) {
        if let Some(path) = &path {
            self.downloads.release(path);
        }
        let text = shot::saved_text(saved.file, saved.clipboard);
        if frame {
            if let Some(window) = self.w.window.upgrade() {
                self.toast(&window, text);
            }
            return;
        }
        self.shot_event(tab, ShotEvent::Saved(text));
        if self.checks.builtin_smoke.is_some() {
            let seen = self.builtin_smoke_seen();
            self.builtin_smoke_step(smoke::Input::Saved(saved, &seen));
        }
    }

    /// Where the visible pane's page starts in the content area, logical px.
    pub(super) fn content_origin(&self, window: &AppWindow) -> (f32, f32) {
        let origin = self.focus_origin(window);
        (
            origin.0 - window.get_content_x(),
            origin.1 - window.get_content_y(),
        )
    }

    /// The overlay's drag, from corner `a` to corner `b` (logical px in the content area). The
    /// overlay sits over the visible page: if that is no longer the page it was opened on, the
    /// pick is cancelled.
    pub(crate) fn shot_picked(&mut self, a: (f32, f32), b: (f32, f32)) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let Some(tab) = self.shot_tab() else {
            return;
        };
        if self.shown() != Some(tab) {
            self.shot_event(tab, ShotEvent::Cancel);
            return;
        }
        let (x0, y0) = self.content_origin(&window);
        let scale = window.window().scale_factor();
        let size = self.pane_layout(&window, tab).size;
        let view = (
            f64::from(size.width) / f64::from(scale),
            f64::from(size.height) / f64::from(scale),
        );
        let at = |p: (f32, f32)| (f64::from(p.0 - x0), f64::from(p.1 - y0));
        let zoom = self.tab_view(tab).map_or(100, |v| v.zoom_percent());
        let event = match shot::RectF::from_drag(at(a), at(b), view) {
            Some(region) => ShotEvent::Picked(shot::region_css(region, zoom)),
            None => ShotEvent::Cancel,
        };
        self.shot_event(tab, event);
    }

    /// The overlay's toolbar: `full` the whole page, else what the view shows.
    pub(crate) fn shot_overlay_capture(&mut self, full: bool) {
        let request = if full {
            ShotRequest::FullPage
        } else {
            ShotRequest::Visible
        };
        self.builtin(Builtin::Shot(request));
    }

    pub(crate) fn shot_cancelled(&mut self) {
        if let Some(tab) = self.shot_tab().or(self.shown()) {
            self.shot_event(tab, ShotEvent::Cancel);
        }
    }

    /// The context menu's "Take Screenshot of Element", at the menu's point (physical px).
    pub(super) fn shot_element(&mut self, tab: TabId, at: (i32, i32)) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let scale = window.window().scale_factor();
        let zoom = self.tab_view(tab).map_or(100, |v| v.zoom_percent());
        let (x, y) = shot::point_css(at, scale, zoom);
        self.shot_event(tab, ShotEvent::Request(ShotRequest::Element { x, y }));
    }

    // ----- `--smoke builtins`

    fn builtin_smoke_seen(&mut self) -> smoke::Seen {
        let tab = self.shown();
        let (scale, view) = match (self.w.window.upgrade(), tab) {
            (Some(window), Some(t)) => {
                let size = self.pane_layout(&window, t).size;
                (window.window().scale_factor(), (size.width, size.height))
            }
            _ => (1.0, (0, 0)),
        };
        let loading = self
            .view()
            .is_some_and(|v| matches!(v.nav().phase, wubble_engine::nav::Phase::Loading { .. }));
        smoke::Seen {
            framed: tab.is_some_and(|t| self.w.frames.contains_key(&t)),
            loading,
            reader: tab
                .and_then(|t| self.builtins.reader.get(&t).copied())
                .unwrap_or_default(),
            hints: self
                .builtins
                .hints
                .as_ref()
                .map(|(_, h)| h.clone())
                .unwrap_or_default(),
            scale,
            view,
        }
    }

    pub(super) fn builtin_smoke_worked(&mut self) {
        if self.checks.builtin_smoke.is_some() {
            let seen = self.builtin_smoke_seen();
            self.builtin_smoke_step(smoke::Input::Worked(&seen));
        }
    }

    pub(super) fn builtin_smoke_step(&mut self, input: smoke::Input) {
        let Some(state) = self.checks.builtin_smoke.take() else {
            return;
        };
        let (state, actions) = smoke::next(state, input);
        self.checks.builtin_smoke = Some(state);
        for action in actions {
            match action {
                smoke::Action::Setting(key, value) => {
                    self.write_setting(key, value);
                }
                smoke::Action::Perform(action) => self.perform(action),
                smoke::Action::Eval(script) => {
                    if let Some(view) = self.view() {
                        view.eval(script);
                    }
                }
                smoke::Action::Type(label) => {
                    for ch in label.chars() {
                        self.hint_typed(ch, false);
                    }
                }
                smoke::Action::Load(url) => {
                    if let Some(tab) = self.shown() {
                        self.dispatch(wubble_core::model::Command::Navigate { tab, url });
                    }
                }
                smoke::Action::Pick(a, b) => {
                    let Some(window) = self.w.window.upgrade() else {
                        continue;
                    };
                    let (x, y) = self.content_origin(&window);
                    self.shot_picked((a.0 + x, a.1 + y), (b.0 + x, b.1 + y));
                }
                smoke::Action::Pass(what) => {
                    self.builtins_passed = Some(what);
                    crate::quit();
                }
                smoke::Action::Fail(reason) => self.fail(reason),
            }
        }
    }

    // ----- the shell

    fn redraw_builtins(&mut self) {
        if let Some(window) = self.w.window.upgrade() {
            self.render_builtins(&window);
        }
    }

    /// The pill's reader glyph, the reader's controls and the screenshot overlay.
    pub(super) fn render_builtins(&mut self, window: &AppWindow) {
        let tab = self.shown();
        let reader = tab
            .and_then(|t| self.builtins.reader.get(&t).copied())
            .unwrap_or_default();
        window.set_reader_available(reader.available());
        window.set_reader_on(reader.is_on());
        let p = self.builtins.reader_prefs;
        window.set_reader_size(i32::try_from(p.size).unwrap_or(i32::MAX));
        window.set_reader_width(p.width as i32);
        window.set_reader_font(p.font as i32);
        window.set_reader_theme(p.theme as i32);
        window.set_shot_picking(self.shot_state() == Shot::Picking);
        let (x, _) = self.content_origin(window);
        let width = tab.map_or(0.0, |t| {
            self.pane_layout(window, t).size.width as f32 / window.window().scale_factor()
        });
        window.set_shot_pane_x(x);
        window.set_shot_pane_width(width);
    }
}

/// The reader's dark theme: the shell's own tokens.
fn dark_colors(window: &AppWindow) -> ReaderColors {
    let p = window.global::<Palette>();
    ReaderColors {
        background: hex(p.get_web()),
        ink: hex(p.get_ink()),
        muted: hex(p.get_mist()),
        rule: hex(p.get_rim()),
        link: hex(p.get_sky()),
    }
}

/// The hint labels: amber on the page's ink, in the shell's mono face.
fn hint_colors(window: &AppWindow) -> HintColors {
    let p = window.global::<Palette>();
    let mono = window.global::<wubble_ui::Fonts>().get_mono();
    HintColors {
        label: hex(p.get_amber()),
        ink: hex(p.get_web()),
        font: format!("'{mono}', ui-monospace, monospace"),
    }
}
