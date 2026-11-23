//! The shell's side of extensions, as pure rules: the words of the install prompt (what an
//! extension asks for), the install prompt's and the toolbar popover's lifecycles, the popup's
//! size, and the options page's address. The host executes the effects; nothing here draws.
//!
//! Ported:
//! - The popup's size limits: Chromium `chrome/browser/ui/views/extensions/extension_popup.cc`
//!   (`ExtensionPopup::kMinWidth`/`kMinHeight` 25, `kMaxWidth` 800, `kMaxHeight` 600), BSD-3-Clause.
//! - The permission wording: Chromium `chrome/app/generated_resources.grd`
//!   (`IDS_EXTENSION_PROMPT_WARNING_*`), the same sentences, BSD-3-Clause. Chromium shows no
//!   line for a permission that is not worth one (storage, alarms, scripting, activeTab, ...);
//!   neither does this.

use super::{Extension, ExtensionId};
use crate::tab::TabId;

/// The pane key of an action popup's view: no tab has it (tab ids count up from 1; this fits
/// Slint's `int` pane ids, one below the DevTools panel's `devtools::PANEL_TAB`).
pub const POPUP_TAB: TabId = TabId(i32::MAX as u64 - 1);

/// The popup's smallest size, CSS px (`ExtensionPopup::kMinWidth`, `kMinHeight`).
pub const POPUP_MIN: (u32, u32) = (25, 25);
/// The popup's largest size (`ExtensionPopup::kMaxWidth`, `kMaxHeight`).
pub const POPUP_MAX: (u32, u32) = (800, 600);

/// The popup the page's `content` (its scroll size) asks for, kept between the limits.
pub fn popup_size(content: (u32, u32)) -> (u32, u32) {
    (
        content.0.clamp(POPUP_MIN.0, POPUP_MAX.0),
        content.1.clamp(POPUP_MIN.1, POPUP_MAX.1),
    )
}

/// The page an extension's options open as a tab: `chrome-extension://<id>/<page>`; `None`
/// without an options page, or one that is not a plain path inside the extension.
pub fn options_url(ext: &Extension) -> Option<String> {
    let page = ext.options_page.as_deref()?.trim_start_matches('/');
    let plain = !page.is_empty()
        && !page.contains(':')
        && !page.contains('\\')
        && page.split('/').all(|part| part != "..");
    plain.then(|| format!("chrome-extension://{}/{page}", ext.id))
}

/// A permission that reaches every site.
fn all_sites(p: &str) -> bool {
    matches!(p, "<all_urls>" | "*://*/*" | "http://*/*" | "https://*/*")
}

/// The host of a match pattern (`https://*.example.com/*`), or `None` for a name that is not one.
fn pattern_host(p: &str) -> Option<&str> {
    let rest = p.split_once("://")?.1;
    let host = rest.split('/').next()?;
    let host = host.trim_start_matches("*.");
    (!host.is_empty() && host != "*").then_some(host)
}

/// The sentence a named API permission is shown as; the table is the data (Chromium's wording).
const API_LINES: &[(&str, &str)] = &[
    ("tabs", "Read your browsing history"),
    ("history", "Read and change your browsing history"),
    ("webNavigation", "Read your browsing history"),
    ("bookmarks", "Read and change your bookmarks"),
    ("downloads", "Manage your downloads"),
    ("cookies", "Read and change your cookies"),
    ("clipboardRead", "Read data you copy and paste"),
    ("clipboardWrite", "Modify data you copy and paste"),
    ("geolocation", "Detect your physical location"),
    ("notifications", "Display notifications"),
    (
        "nativeMessaging",
        "Communicate with cooperating native applications",
    ),
    ("webRequest", "Block page content"),
    ("declarativeNetRequest", "Block page content"),
    ("management", "Manage your apps, extensions and themes"),
    ("proxy", "Read and change your proxy settings"),
    ("privacy", "Change your privacy-related settings"),
    (
        "topSites",
        "Read a list of your most frequently visited websites",
    ),
    ("desktopCapture", "Capture content of your screen"),
    (
        "tabCapture",
        "Read and change the content of the tabs you capture",
    ),
];

/// How many sites a line names before it counts them (Chromium: "on N sites").
const SITES_NAMED: usize = 3;

/// What an extension asking for `permissions` ([`super::Manifest::permissions`]) can do, one
/// line each, most far-reaching first; empty when nothing is worth a line.
pub fn permission_lines(permissions: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = vec![];
    let add = |lines: &mut Vec<String>, line: String| {
        if !lines.contains(&line) {
            lines.push(line);
        }
    };
    if permissions.iter().any(|p| all_sites(p)) {
        add(
            &mut lines,
            "Read and change all your data on all websites".into(),
        );
    } else {
        let mut hosts: Vec<&str> = vec![];
        for host in permissions.iter().filter_map(|p| pattern_host(p)) {
            if !hosts.contains(&host) {
                hosts.push(host);
            }
        }
        match hosts.len() {
            0 => {}
            n if n <= SITES_NAMED => add(
                &mut lines,
                format!("Read and change your data on {}", hosts.join(", ")),
            ),
            n => add(
                &mut lines,
                format!("Read and change your data on {n} sites"),
            ),
        }
    }
    for p in permissions {
        if let Some((_, line)) = API_LINES.iter().find(|(name, _)| name == p) {
            add(&mut lines, (*line).to_owned());
        }
    }
    lines
}

/// The settings row's one line under an extension's name.
pub fn summary(ext: &Extension) -> String {
    let lines = permission_lines(&ext.permissions);
    match lines.first() {
        None => format!("Version {}", ext.version),
        Some(first) if lines.len() == 1 => format!("Version {} \u{b7} {first}", ext.version),
        Some(first) => format!(
            "Version {} \u{b7} {first}, and {} more",
            ext.version,
            lines.len() - 1
        ),
    }
}

/// The install prompt's title and message for `ext`: what it asks for, in Chromium's words.
pub fn prompt_text(ext: &Extension) -> (String, String) {
    let lines = permission_lines(&ext.permissions);
    let asks = if lines.is_empty() {
        "It asks for no special access.".to_owned()
    } else {
        let list: Vec<String> = lines.iter().map(|l| format!("\u{2022} {l}")).collect();
        format!("It can:\n{}", list.join("\n"))
    };
    (
        format!("Add \u{201c}{}\u{201d}?", ext.name),
        format!("Version {}\n\n{asks}", ext.version),
    )
}

/// The install prompt: the extension that waits for the user's answer. Nothing is installed
/// before Allow (`Chromium: ExtensionInstallPrompt`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Prompt {
    #[default]
    Closed,
    Asking(Extension),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptEvent {
    /// An extension was read from the file the user picked.
    Ask(Extension),
    /// The prompt's "Allow in private windows" box (off at first): what the install keeps
    /// ([`Extension::private`]).
    AllowPrivate(bool),
    Allow,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptEffect {
    Install(Extension),
}

pub fn prompt_transition(state: Prompt, event: PromptEvent) -> (Prompt, Vec<PromptEffect>) {
    match (state, event) {
        // a second pick replaces the first: only the last is asked
        (_, PromptEvent::Ask(ext)) => (Prompt::Asking(ext), vec![]),
        (Prompt::Asking(ext), PromptEvent::Allow) => {
            (Prompt::Closed, vec![PromptEffect::Install(ext)])
        }
        (Prompt::Asking(mut ext), PromptEvent::AllowPrivate(on)) => {
            ext.private = on;
            (Prompt::Asking(ext), vec![])
        }
        (Prompt::Closed, PromptEvent::AllowPrivate(_)) => (Prompt::Closed, vec![]),
        (_, PromptEvent::Allow | PromptEvent::Cancel) => (Prompt::Closed, vec![]),
    }
}

/// What a click on an extension's action did (`wubble_engine::ext_api::ActionClick` without
/// the engine's types).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clicked {
    /// Nothing ran.
    Nothing,
    /// `action.onClicked` ran.
    Ran,
    /// The action has a popup page.
    Popup(String),
}

/// The footer button's panel: the list of actions, or one action's popup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Panel {
    #[default]
    Closed,
    List,
    Popup {
        /// Its page's size once known, between [`POPUP_MIN`] and [`POPUP_MAX`].
        size: Option<(u32, u32)>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelEvent {
    /// The footer button.
    Toggle,
    /// A press on `extension`'s action in the list, and what it did.
    Clicked(ExtensionId, Clicked),
    /// The popup page's content size.
    Sized(u32, u32),
    /// Escape, a press outside, a tab switch.
    Dismiss,
    /// The popup page closed itself (`window.close()`).
    PageClosed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelEffect {
    /// Open the popup page in a windowless view of the extension's profile.
    OpenPopup {
        extension: ExtensionId,
        url: String,
    },
    ClosePopup,
}

pub fn panel_transition(state: Panel, event: PanelEvent) -> (Panel, Vec<PanelEffect>) {
    let close = |state: &Panel| match state {
        Panel::Popup { .. } => vec![PanelEffect::ClosePopup],
        _ => vec![],
    };
    match (state, event) {
        (Panel::Closed, PanelEvent::Toggle) => (Panel::List, vec![]),
        (state, PanelEvent::Toggle | PanelEvent::Dismiss) => {
            let effects = close(&state);
            (Panel::Closed, effects)
        }
        (Panel::List, PanelEvent::Clicked(extension, Clicked::Popup(url))) => (
            Panel::Popup { size: None },
            vec![PanelEffect::OpenPopup { extension, url }],
        ),
        // the click ran the extension's code: the list has done its work
        (Panel::List, PanelEvent::Clicked(_, Clicked::Ran)) => (Panel::Closed, vec![]),
        (Panel::Popup { .. }, PanelEvent::Sized(w, h)) => (
            Panel::Popup {
                size: Some(popup_size((w, h))),
            },
            vec![],
        ),
        (state @ Panel::Popup { .. }, PanelEvent::PageClosed) => {
            let effects = close(&state);
            (Panel::Closed, effects)
        }
        (state, _) => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::{Source, parse_manifest};
    use std::path::PathBuf;

    fn ext(manifest: &str) -> Extension {
        let dir = PathBuf::from("/x");
        let m = parse_manifest(manifest).unwrap();
        let id = m.id(&dir).unwrap();
        Extension::new(id, m, Source::Unpacked { dir })
    }

    fn manifest(extra: &str) -> String {
        format!(r#"{{"manifest_version":3,"name":"E","version":"1.0"{extra}}}"#)
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn id() -> ExtensionId {
        ExtensionId::parse("abcdefghijklmnopabcdefghijklmnop").unwrap()
    }

    #[test]
    fn the_popup_stays_between_chromiums_limits() {
        assert_eq!(popup_size((0, 0)), (25, 25));
        assert_eq!(popup_size((24, 26)), (25, 26));
        assert_eq!(popup_size((320, 480)), (320, 480));
        assert_eq!(popup_size((801, 601)), (800, 600));
        assert_eq!(popup_size((u32::MAX, u32::MAX)), (800, 600));
    }

    #[test]
    fn quiet_permissions_have_no_line() {
        let p = strings(&[
            "storage",
            "scripting",
            "activeTab",
            "alarms",
            "contextMenus",
        ]);
        assert!(permission_lines(&p).is_empty());
    }

    #[test]
    fn all_sites_swallow_the_named_ones() {
        let p = strings(&["https://a.test/*", "<all_urls>", "tabs"]);
        assert_eq!(
            permission_lines(&p),
            [
                "Read and change all your data on all websites",
                "Read your browsing history"
            ]
        );
    }

    #[test]
    fn named_sites_are_listed_then_counted() {
        let few = strings(&["https://*.a.test/*", "http://a.test/x", "https://b.test/*"]);
        assert_eq!(
            permission_lines(&few),
            ["Read and change your data on a.test, b.test"]
        );
        let many = strings(&[
            "https://a.test/*",
            "https://b.test/*",
            "https://c.test/*",
            "https://d.test/*",
        ]);
        assert_eq!(
            permission_lines(&many),
            ["Read and change your data on 4 sites"]
        );
    }

    #[test]
    fn two_permissions_with_one_sentence_show_it_once() {
        let p = strings(&[
            "tabs",
            "webNavigation",
            "webRequest",
            "declarativeNetRequest",
        ]);
        assert_eq!(
            permission_lines(&p),
            ["Read your browsing history", "Block page content"]
        );
    }

    #[test]
    fn the_summary_counts_what_the_first_line_leaves() {
        let quiet = ext(&manifest(r#","permissions":["storage"]"#));
        assert_eq!(summary(&quiet), "Version 1.0");
        let one = ext(&manifest(r#","permissions":["tabs"]"#));
        assert_eq!(
            summary(&one),
            "Version 1.0 \u{b7} Read your browsing history"
        );
        let more = ext(&manifest(r#","permissions":["tabs","bookmarks"]"#));
        assert_eq!(
            summary(&more),
            "Version 1.0 \u{b7} Read your browsing history, and 1 more"
        );
    }

    #[test]
    fn the_options_page_is_a_path_in_the_extension() {
        let url = |page: &str| {
            options_url(&ext(&manifest(&format!(
                r#","options_ui":{{"page":{page:?}}}"#
            ))))
        };
        let e = ext(&manifest(r#","options_ui":{"page":"o/options.html"}"#));
        assert_eq!(
            options_url(&e),
            Some(format!("chrome-extension://{}/o/options.html", e.id))
        );
        assert!(url("/options.html").unwrap().ends_with("/options.html"));
        assert_eq!(url("../x.html"), None);
        assert_eq!(url("https://evil.test/"), None);
        assert_eq!(url(""), None);
        assert_eq!(options_url(&ext(&manifest(""))), None);
        // the older key
        let old = ext(&manifest(r#","options_page":"old.html""#));
        assert!(options_url(&old).unwrap().ends_with("/old.html"));
        // and the newer one wins
        let both = ext(&manifest(
            r#","options_page":"old.html","options_ui":{"page":"new.html"}"#,
        ));
        assert!(options_url(&both).unwrap().ends_with("/new.html"));
    }

    #[test]
    fn the_prompt_names_the_extension_and_lists_what_it_asks() {
        let quiet = ext(&manifest(""));
        let (title, message) = prompt_text(&quiet);
        assert_eq!(title, "Add \u{201c}E\u{201d}?");
        assert_eq!(message, "Version 1.0\n\nIt asks for no special access.");
        let loud = ext(&manifest(r#","permissions":["tabs","downloads"]"#));
        assert_eq!(
            prompt_text(&loud).1,
            "Version 1.0\n\nIt can:\n\u{2022} Read your browsing history\n\u{2022} Manage your downloads"
        );
    }

    #[test]
    fn the_popup_pane_is_nobody_s_tab() {
        assert_ne!(POPUP_TAB, crate::devtools::PANEL_TAB);
        assert!(POPUP_TAB.0 <= i32::MAX as u64);
    }

    #[test]
    fn nothing_is_installed_before_allow() {
        let e = ext(&manifest(""));
        let (p, fx) = prompt_transition(Prompt::Closed, PromptEvent::Ask(e.clone()));
        assert_eq!((&p, fx.len()), (&Prompt::Asking(e.clone()), 0));
        let (p2, fx) = prompt_transition(p.clone(), PromptEvent::Cancel);
        assert_eq!((p2, fx.len()), (Prompt::Closed, 0));
        let (p2, fx) = prompt_transition(p, PromptEvent::Allow);
        assert_eq!(p2, Prompt::Closed);
        assert_eq!(fx, [PromptEffect::Install(e)]);
        // an answer with nothing asked does nothing
        let (p, fx) = prompt_transition(Prompt::Closed, PromptEvent::Allow);
        assert_eq!((p, fx.len()), (Prompt::Closed, 0));
    }

    #[test]
    fn the_prompt_s_private_box_goes_with_the_install() {
        let e = ext(&manifest(""));
        let allowed = Extension {
            private: true,
            ..e.clone()
        };
        // (ticks of the box, in order, then what Allow installs)
        for (ticks, installs) in [
            (vec![], e.clone()),
            (vec![true], allowed.clone()),
            (vec![true, false], e.clone()),
            (vec![false, true], allowed.clone()),
        ] {
            let mut p = prompt_transition(Prompt::Closed, PromptEvent::Ask(e.clone())).0;
            for on in &ticks {
                let (next, fx) = prompt_transition(p, PromptEvent::AllowPrivate(*on));
                assert!(fx.is_empty());
                p = next;
            }
            let (p, fx) = prompt_transition(p, PromptEvent::Allow);
            assert_eq!((p, fx), (Prompt::Closed, vec![PromptEffect::Install(installs)]));
        }
        // a new pick starts unticked; a tick with nothing asked does nothing
        let p = prompt_transition(Prompt::Asking(allowed), PromptEvent::Ask(e.clone())).0;
        assert_eq!(p, Prompt::Asking(e));
        let (p, fx) = prompt_transition(Prompt::Closed, PromptEvent::AllowPrivate(true));
        assert_eq!((p, fx.len()), (Prompt::Closed, 0));
    }

    #[test]
    fn a_second_pick_replaces_the_one_waiting() {
        let (a, b) = (
            ext(&manifest("")),
            ext(&manifest("").replace("\"E\"", "\"F\"")),
        );
        let (p, _) = prompt_transition(Prompt::Asking(a), PromptEvent::Ask(b.clone()));
        assert_eq!(p, Prompt::Asking(b));
    }

    #[test]
    fn the_button_opens_and_closes_the_list() {
        let (p, fx) = panel_transition(Panel::Closed, PanelEvent::Toggle);
        assert_eq!((&p, fx.len()), (&Panel::List, 0));
        let (p, fx) = panel_transition(p, PanelEvent::Toggle);
        assert_eq!((p, fx.len()), (Panel::Closed, 0));
        let (p, _) = panel_transition(Panel::List, PanelEvent::Dismiss);
        assert_eq!(p, Panel::Closed);
    }

    #[test]
    fn an_action_without_a_popup_fires_and_closes_the_list() {
        let (p, fx) = panel_transition(Panel::List, PanelEvent::Clicked(id(), Clicked::Ran));
        assert_eq!((p, fx.len()), (Panel::Closed, 0));
        // a click that ran nothing leaves the list as it was
        let (p, fx) = panel_transition(Panel::List, PanelEvent::Clicked(id(), Clicked::Nothing));
        assert_eq!((p, fx.len()), (Panel::List, 0));
    }

    #[test]
    fn a_popup_opens_sizes_and_closes_with_its_view() {
        let url = "chrome-extension://x/p.html".to_owned();
        let (p, fx) = panel_transition(
            Panel::List,
            PanelEvent::Clicked(id(), Clicked::Popup(url.clone())),
        );
        assert_eq!(
            fx,
            [PanelEffect::OpenPopup {
                extension: id(),
                url: url.clone()
            }]
        );
        assert_eq!(p, Panel::Popup { size: None });
        let (p, fx) = panel_transition(p, PanelEvent::Sized(5000, 10));
        assert!(fx.is_empty());
        assert_eq!(
            p,
            Panel::Popup {
                size: Some((800, 25))
            }
        );
        for event in [
            PanelEvent::Dismiss,
            PanelEvent::Toggle,
            PanelEvent::PageClosed,
        ] {
            let (closed, fx) = panel_transition(p.clone(), event);
            assert_eq!((closed, fx), (Panel::Closed, vec![PanelEffect::ClosePopup]));
        }
    }

    #[test]
    fn a_size_outside_a_popup_is_ignored() {
        for state in [Panel::Closed, Panel::List] {
            let (p, fx) = panel_transition(state.clone(), PanelEvent::Sized(100, 100));
            assert_eq!((p, fx.len()), (state, 0));
        }
    }
}
