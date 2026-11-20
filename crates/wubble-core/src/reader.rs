//! Reader mode: the page's article, extracted by Mozilla's Readability in the page and shown in
//! the tab as a clean document styled with wubble's tokens. The extraction and the document are
//! the page script's (`crates/wubble/assets/reader.js`); this is the lifecycle ([`Reader`],
//! [`transition`]) and the typography ([`ReaderPrefs`], [`css_vars`]).
//!
//! Why a document in the tab and not a native Slint view: Slint 1.19's `StyledText` has no
//! headings, images, block quotes or code blocks, and the images would need wubble to fetch them
//! itself, which it must not (nothing phones home; CEF is the only network stack).
//!
//! Leaving reader mode reloads the page: the reader replaced the page's document in place. The
//! page comes back where it was (QA HIGH-11): the reader answers the page's scroll position as it
//! opens; leaving, a script turns off Chromium's own restore for the reload (it would take the
//! reader's offset, the two sharing one history entry), its answer starts the reload, and once
//! the page has loaded it is scrolled back there.

use std::collections::HashMap;

use crate::apply::value_of;
use crate::settings;

/// Where a page was scrolled, CSS px.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollAt {
    pub x: i32,
    pub y: i32,
}

/// Reader mode of one tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reader {
    /// The page as it is; `readerable` when the probe found an article (the pill's glyph).
    Off { readerable: bool },
    /// The probe (Readability's `isProbablyReaderable`) runs as script `id`.
    Probing(u64),
    /// The extraction runs as script `id`.
    Extracting(u64),
    /// The article shows; the page was at `back` when it opened.
    On { back: ScrollAt },
    /// The reader is leaving: the script turning Chromium's scroll restore off runs (as
    /// `script` once sent); its answer reloads the page.
    Leaving { back: ScrollAt, script: Option<u64> },
    /// The page reloads, to be scrolled back to `back` once loaded.
    Reloading { back: ScrollAt },
}

impl Default for Reader {
    fn default() -> Self {
        Self::Off { readerable: false }
    }
}

impl Reader {
    /// Whether the pill offers reader mode (or shows it is on).
    pub fn available(self) -> bool {
        matches!(self, Self::Off { readerable: true } | Self::On { .. })
    }

    pub fn is_on(self) -> bool {
        matches!(self, Self::On { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderEvent {
    /// A new page started loading in the tab (a link out of the reader included).
    PageStarted,
    /// The page finished loading: probe it, if probing is on.
    Loaded {
        detect: bool,
    },
    ProbeSent(u64),
    /// The probe's answer: script `id` says whether the page reads as an article.
    Probed(u64, bool),
    /// The action, the pill's glyph.
    Toggle,
    ExtractSent(u64),
    /// Script `id` built the reader, the page having been at the position given, or found no
    /// article (`None`).
    Extracted(u64, Option<ScrollAt>),
    LeaveSent(u64),
    /// Script `id`, the leaving one, answered.
    Left(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderEffect {
    /// Run the probe, then report `ProbeSent`.
    Probe,
    /// Run the extraction, then report `ExtractSent`.
    Extract,
    /// Leaving the reader: run the script turning Chromium's scroll restore off for the reload,
    /// then report `LeaveSent`.
    Leave,
    /// Reload the page.
    Reload,
    /// The page loaded again after the reader: scroll it back to where it was.
    Restore(ScrollAt),
    Toast(&'static str),
}

pub fn transition(state: Reader, event: ReaderEvent) -> (Reader, Vec<ReaderEffect>) {
    use ReaderEffect as E;
    use ReaderEvent::*;
    match (state, event) {
        // the reload that leaves the reader
        (Reader::Leaving { back, script: None }, LeaveSent(id)) => (
            Reader::Leaving {
                back,
                script: Some(id),
            },
            vec![],
        ),
        (
            Reader::Leaving {
                back,
                script: Some(want),
            },
            Left(id),
        ) if id == want => (Reader::Reloading { back }, vec![E::Reload]),
        (Reader::Reloading { .. }, PageStarted) => (state, vec![]),
        (Reader::Reloading { back }, Loaded { detect }) => {
            let mut effects = vec![E::Restore(back)];
            if detect {
                effects.push(E::Probe);
            }
            (Reader::default(), effects)
        }
        (_, PageStarted) => (Reader::default(), vec![]),
        (Reader::Off { .. }, Loaded { detect: true }) => (state, vec![E::Probe]),
        (Reader::Off { .. }, ProbeSent(id)) => (Reader::Probing(id), vec![]),
        (Reader::Probing(want), Probed(id, readerable)) if id == want => {
            (Reader::Off { readerable }, vec![])
        }
        // Toggling with nothing found is allowed: the user knows better than the heuristic.
        (Reader::Off { .. } | Reader::Probing(_), Toggle) => (state, vec![E::Extract]),
        (Reader::Off { .. } | Reader::Probing(_), ExtractSent(id)) => {
            (Reader::Extracting(id), vec![])
        }
        (Reader::Extracting(want), Extracted(id, Some(back))) if id == want => {
            (Reader::On { back }, vec![])
        }
        (Reader::Extracting(want), Extracted(id, None)) if id == want => (
            Reader::Off { readerable: false },
            vec![E::Toast("Nothing to read on this page")],
        ),
        (Reader::On { back }, Toggle) => (Reader::Leaving { back, script: None }, vec![E::Leave]),
        // Stale answers, a second toggle while extracting, loads while the reader shows.
        (state, _) => (state, vec![]),
    }
}

/// The reader's column width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderWidth {
    Narrow,
    Medium,
    Wide,
}

pub const WIDTH_WORDS: [&str; 3] = ["narrow", "medium", "wide"];

impl ReaderWidth {
    pub const ALL: [ReaderWidth; 3] = [Self::Narrow, Self::Medium, Self::Wide];

    pub fn id(self) -> &'static str {
        WIDTH_WORDS[self as usize]
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.id() == id)
    }

    /// The column's maximum width, CSS px.
    pub fn px(self) -> u32 {
        match self {
            Self::Narrow => 560,
            Self::Medium => 680,
            Self::Wide => 820,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderFont {
    Serif,
    Sans,
}

pub const FONT_WORDS: [&str; 2] = ["serif", "sans"];

impl ReaderFont {
    pub const ALL: [ReaderFont; 2] = [Self::Serif, Self::Sans];

    pub fn id(self) -> &'static str {
        FONT_WORDS[self as usize]
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
    }

    /// The CSS font stack. The page cannot use the shell's bundled fonts, so these are the
    /// system's.
    pub fn stack(self) -> &'static str {
        match self {
            Self::Serif => "Charter, 'Iowan Old Style', Georgia, 'Noto Serif', serif",
            Self::Sans => "system-ui, -apple-system, 'Segoe UI', 'Noto Sans', sans-serif",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderTheme {
    /// wubble's own dark tokens (`Palette`), read from the shell at run time.
    Dark,
    Light,
    Sepia,
}

pub const THEME_WORDS: [&str; 3] = ["dark", "light", "sepia"];

impl ReaderTheme {
    pub const ALL: [ReaderTheme; 3] = [Self::Dark, Self::Light, Self::Sepia];

    pub fn id(self) -> &'static str {
        THEME_WORDS[self as usize]
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// The reader's colours, `#rrggbb`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderColors {
    pub background: String,
    pub ink: String,
    pub muted: String,
    pub rule: String,
    pub link: String,
}

impl ReaderColors {
    fn of(c: [&str; 5]) -> Self {
        let [background, ink, muted, rule, link] = c.map(str::to_owned);
        Self {
            background,
            ink,
            muted,
            rule,
            link,
        }
    }

    /// The light and sepia themes: reader-only colours, so they live here, once. (The dark one
    /// is the shell's `Palette`, which has no light mode yet.)
    pub fn of_theme(theme: ReaderTheme, dark: &ReaderColors) -> Self {
        match theme {
            ReaderTheme::Dark => dark.clone(),
            ReaderTheme::Light => Self::of(["#fbfbfa", "#1d1d1f", "#6e6e73", "#d9d9de", "#0a5fcc"]),
            ReaderTheme::Sepia => Self::of(["#f4ecd8", "#3b2f22", "#7a6a55", "#dccfb4", "#8a4b12"]),
        }
    }
}

/// Typography of the reader (`settings::READER_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReaderPrefs {
    /// Body text, CSS px.
    pub size: u32,
    pub width: ReaderWidth,
    pub font: ReaderFont,
    pub theme: ReaderTheme,
    /// Probe pages for an article (the pill's glyph).
    pub detect: bool,
}

impl ReaderPrefs {
    pub fn from_stored(stored: &HashMap<String, String>) -> Self {
        use settings as s;
        Self {
            size: read(&s::READER_SIZE, stored, |w| w.parse().ok()),
            width: read(&s::READER_WIDTH, stored, ReaderWidth::from_id),
            font: read(&s::READER_FONT, stored, ReaderFont::from_id),
            theme: read(&s::READER_THEME, stored, ReaderTheme::from_id),
            detect: value_of(&s::READER_DETECT, stored) == "1",
        }
    }
}

/// `def`'s stored value, or its declared default when the stored one does not parse.
fn read<T>(
    def: &settings::SettingDef,
    stored: &HashMap<String, String>,
    parse: impl Fn(&str) -> Option<T>,
) -> T {
    parse(value_of(def, stored))
        .or_else(|| parse(def.default))
        .expect("a setting's default parses")
}

impl Default for ReaderPrefs {
    fn default() -> Self {
        Self::from_stored(&HashMap::new())
    }
}

/// The CSS custom properties the reader's stylesheet reads, set on the document element.
pub fn css_vars(prefs: &ReaderPrefs, dark: &ReaderColors) -> Vec<(&'static str, String)> {
    let c = ReaderColors::of_theme(prefs.theme, dark);
    vec![
        ("--wb-size", format!("{}px", prefs.size)),
        ("--wb-width", format!("{}px", prefs.width.px())),
        ("--wb-font", prefs.font.stack().to_owned()),
        ("--wb-bg", c.background),
        ("--wb-ink", c.ink),
        ("--wb-muted", c.muted),
        ("--wb-rule", c.rule),
        ("--wb-link", c.link),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ReaderEffect as E;
    use ReaderEvent::*;

    const ON: Reader = Reader::On {
        back: ScrollAt { x: 0, y: 0 },
    };

    fn dark() -> ReaderColors {
        ReaderColors::of(["#08090d", "#f4f5f8", "#a0a4b0", "#2a2d36", "#8ec5ff"])
    }

    #[test]
    fn probe_then_toggle_then_exit() {
        let s = Reader::default();
        assert_eq!(transition(s, Loaded { detect: false }), (s, vec![]));
        let (s, e) = transition(s, Loaded { detect: true });
        assert_eq!(e, vec![E::Probe]);
        let (s, _) = transition(s, ProbeSent(4));
        assert_eq!(s, Reader::Probing(4));
        let (s, e) = transition(s, Probed(4, true));
        assert_eq!((s, e), (Reader::Off { readerable: true }, vec![]));
        assert!(s.available());
        let (s, e) = transition(s, Toggle);
        assert_eq!(e, vec![E::Extract]);
        let (s, _) = transition(s, ExtractSent(5));
        assert_eq!(s, Reader::Extracting(5));
        let back = ScrollAt { x: 0, y: 1800 };
        let (s, e) = transition(s, Extracted(5, Some(back)));
        assert_eq!((s, e), (Reader::On { back }, vec![]));
        let (s, e) = transition(s, Toggle);
        let leaving = Reader::Leaving { back, script: None };
        assert_eq!((s, e), (leaving, vec![E::Leave]));
        let (s, _) = transition(s, LeaveSent(6));
        // another script's answer does not reload
        assert_eq!(transition(s, Left(5)), (s, vec![]));
        let (s, e) = transition(s, Left(6));
        assert_eq!((s, e), (Reader::Reloading { back }, vec![E::Reload]));
        // the reload starts, then loads: the page goes back to where it was, after its layout
        let (s, e) = transition(s, PageStarted);
        assert_eq!((s, e), (Reader::Reloading { back }, vec![]));
        let (s, e) = transition(s, Loaded { detect: true });
        assert_eq!(
            (s, e),
            (Reader::default(), vec![E::Restore(back), E::Probe])
        );
        assert_eq!(
            transition(Reader::Reloading { back }, Loaded { detect: false }),
            (Reader::default(), vec![E::Restore(back)])
        );
        // the reader's pill is not offered while the page reloads; a navigation away while
        // the script runs leaves nothing to restore
        assert!(!leaving.available() && !Reader::Reloading { back }.available());
        assert_eq!(
            transition(leaving, PageStarted),
            (Reader::default(), vec![])
        );
    }

    #[test]
    fn nothing_readable_says_so_and_stale_answers_are_ignored() {
        let (s, e) = transition(Reader::Extracting(2), Extracted(2, None));
        assert_eq!(s, Reader::Off { readerable: false });
        assert_eq!(e, vec![E::Toast("Nothing to read on this page")]);
        for (state, event) in [
            (
                Reader::Extracting(2),
                Extracted(1, Some(ScrollAt::default())),
            ),
            (Reader::Probing(2), Probed(1, true)),
            (Reader::Off { readerable: false }, Probed(1, true)),
            (ON, Extracted(1, Some(ScrollAt::default()))),
            (Reader::Extracting(2), Toggle),
            (ON, Loaded { detect: true }),
            (
                Reader::Leaving {
                    back: ScrollAt::default(),
                    script: None,
                },
                Toggle,
            ),
            (Reader::Extracting(2), Loaded { detect: true }),
        ] {
            assert_eq!(
                transition(state, event),
                (state, vec![]),
                "{state:?} {event:?}"
            );
        }
    }

    #[test]
    fn toggle_works_without_a_probe() {
        for state in [Reader::Off { readerable: false }, Reader::Probing(3)] {
            assert_eq!(transition(state, Toggle), (state, vec![E::Extract]));
            assert_eq!(transition(state, ExtractSent(9)).0, Reader::Extracting(9));
        }
    }

    #[test]
    fn a_new_page_always_starts_off() {
        for state in [
            ON,
            Reader::Extracting(1),
            Reader::Probing(1),
            Reader::Off { readerable: true },
        ] {
            assert_eq!(transition(state, PageStarted), (Reader::default(), vec![]));
        }
        assert!(!Reader::default().available());
        assert!(ON.available());
    }

    #[test]
    fn words_round_trip() {
        for w in ReaderWidth::ALL {
            assert_eq!(ReaderWidth::from_id(w.id()), Some(w));
        }
        for f in ReaderFont::ALL {
            assert_eq!(ReaderFont::from_id(f.id()), Some(f));
        }
        for t in ReaderTheme::ALL {
            assert_eq!(ReaderTheme::from_id(t.id()), Some(t));
        }
        assert_eq!(ReaderTheme::from_id("blue"), None);
    }

    #[test]
    fn prefs_come_from_the_settings() {
        let p = ReaderPrefs::default();
        assert_eq!(
            p,
            ReaderPrefs {
                size: 18,
                width: ReaderWidth::Medium,
                font: ReaderFont::Serif,
                theme: ReaderTheme::Dark,
                detect: true,
            }
        );
        let stored: HashMap<String, String> = [
            ("reader.size", "22"),
            ("reader.width", "wide"),
            ("reader.font", "sans"),
            ("reader.theme", "sepia"),
            ("reader.detect", "0"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let p = ReaderPrefs::from_stored(&stored);
        assert_eq!(p.size, 22);
        assert_eq!(p.width, ReaderWidth::Wide);
        assert_eq!(p.font, ReaderFont::Sans);
        assert_eq!(p.theme, ReaderTheme::Sepia);
        assert!(!p.detect);
        // A corrupt stored value falls back to the declared default.
        let bad: HashMap<String, String> = [("reader.size", "big"), ("reader.font", "comic")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let p = ReaderPrefs::from_stored(&bad);
        assert_eq!((p.size, p.font), (18, ReaderFont::Serif));
    }

    #[test]
    fn css_vars_carry_the_typography_and_theme() {
        let p = ReaderPrefs::default();
        let vars = css_vars(&p, &dark());
        let get = |k| vars.iter().find(|(n, _)| *n == k).unwrap().1.clone();
        assert_eq!(get("--wb-size"), "18px");
        assert_eq!(get("--wb-width"), "680px");
        assert!(get("--wb-font").ends_with("serif"));
        assert_eq!(get("--wb-bg"), "#08090d");
        let sepia = css_vars(
            &ReaderPrefs {
                theme: ReaderTheme::Sepia,
                ..p
            },
            &dark(),
        );
        assert!(sepia.contains(&("--wb-bg", "#f4ecd8".into())));
        assert_eq!(vars.len(), 8);
    }
}
