//! `--smoke builtins`: the built-ins through the host's real paths, on bundled pages. Bounded by
//! counts of CEF work runs per step, never by time. Run by hand (it opens a window).
//!
//! 1. `article.html`: screenshots go to a file only (the smoke never touches the clipboard).
//! 2. Dark mode: the default page theme set to Dark, then Light. Both DevTools calls are
//!    accepted each time, and the page's `prefers-color-scheme` and the colour its
//!    `@media (prefers-color-scheme: dark)` rule sets follow. (Forced dark paints without changing
//!    computed style; it is checked by eye.)
//! 3. Reader: the probe found the article; toggling builds the reader (its heading is the
//!    article's title, its image is there); toggling again reloads the page as it was.
//! 4. Link hints: on; typing the label of the page's `#hinted` link clicks it (its handler sets
//!    the title).
//! 5. Screenshots of `tall.html`: the full page is at least as tall as the page, the visible one
//!    the view's size, a dragged 100 x 80 region that size (device pixels).
//! 6. PDF: a bundled PDF opens as a PDF document (Chromium's viewer draws it; its look is
//!    checked by eye: the viewer is a guest the page's script cannot see), and Print saves the original file to Downloads (printing the viewer would save
//!    a blank page).

use serde_json::Value;
use wubble_core::action::Action as UserAction;
use wubble_core::hints::Hints;
use wubble_core::reader::Reader;
use wubble_engine::nav::WebEvent;
use wubble_engine::page::PageEvent;

use crate::shot_io::Saved;

/// Runs of CEF work one step may take before the smoke gives up.
pub const MAX_STEP_RUNS: u32 = 3_000;

/// `article.html`'s title, the reader's heading.
pub const ARTICLE_TITLE: &str = "The Quiet Physics of Soap Films";
/// The title `article.html`'s `#hinted` link sets when clicked.
pub const HINTED: &str = "wubble hinted";
/// The colours `article.html`'s body has without and with `prefers-color-scheme: dark`.
pub const LIGHT_BG: &str = "rgb(250, 250, 250)";
pub const DARK_BG: &str = "rgb(18, 52, 86)";
/// `tall.html`'s height, CSS px.
pub const TALL: f64 = 3000.0;
/// The region dragged on `tall.html`, view logical px.
pub const REGION: ((f32, f32), (f32, f32)) = ((20.0, 30.0), (120.0, 110.0));

/// The DevTools calls one page-theme change sends (`pagetheme::calls`).
const THEME_CALLS: u32 = 2;

/// The page's colour scheme and body colour, tagged so its answer is known.
const THEME_CHECK: &str = "({check:'theme',dark:matchMedia('(prefers-color-scheme: dark)').matches,\
    bg:getComputedStyle(document.body).backgroundColor})";
const READER_CHECK: &str = "({check:'reader',\
    h1:(document.querySelector('main.wb-reader h1')||{}).textContent||'',\
    img:!!document.querySelector('main.wb-reader img')})";
const PDF_CHECK: &str = "({check:'pdf',type:document.contentType})";
const RESTORED_CHECK: &str =
    "({check:'restored',reader:!!document.querySelector('main.wb-reader'),title:document.title})";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Loaded,
    /// The page theme was set; waiting for this many more replies to its DevTools calls (the
    /// only ones out then).
    ThemeReplies {
        theme: Theme,
        left: u32,
    },
    ThemeChecked(Theme),
    Readerable,
    ReaderOn,
    ReaderChecked,
    ReaderLeft,
    Reloading,
    Restored,
    HintsShown,
    Hinted,
    TallLoaded,
    FullPage,
    Visible,
    Region,
    PdfLoaded,
    PdfChecked,
    PdfSaved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    fn word(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BuiltinSmoke {
    At {
        step: Step,
        runs: u32,
        /// The pages the smoke opens: `tall.html` and the PDF.
        tall: String,
        pdf: String,
    },
    Done,
}

impl BuiltinSmoke {
    pub fn new(tall: String, pdf: String) -> Self {
        Self::At {
            step: Step::Loaded,
            runs: 0,
            tall,
            pdf,
        }
    }
}

/// What the smoke reads of the host after a run of CEF work.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    /// The visible page has a frame on screen.
    pub framed: bool,
    pub loading: bool,
    pub reader: Reader,
    pub hints: Hints,
    /// The window's scale factor and the view's size, physical px.
    pub scale: f32,
    pub view: (u32, u32),
}

#[derive(Clone, Copy, Debug)]
pub enum Input<'a> {
    Worked(&'a Seen),
    Web(&'a WebEvent),
    Page(&'a PageEvent),
    /// A screenshot was saved; the host as it is then.
    Saved(Saved, &'a Seen),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Setting(&'static str, &'static str),
    Perform(UserAction),
    Eval(&'static str),
    /// Type a hint label.
    Type(String),
    Load(String),
    /// Drag on the screenshot overlay, view logical px.
    Pick((f32, f32), (f32, f32)),
    Pass(String),
    Fail(String),
}

/// A tagged check's answer.
fn check(event: &PageEvent) -> Option<Value> {
    let PageEvent::EvalResult {
        result: Ok(text), ..
    } = event
    else {
        return None;
    };
    let v: Value = serde_json::from_str(text).ok()?;
    v.get("check").is_some().then_some(v)
}

/// The label of the target linking to `#hinted`.
fn hinted_label(hints: &Hints) -> Option<String> {
    let Hints::Active {
        targets, labels, ..
    } = hints
    else {
        return None;
    };
    targets
        .iter()
        .zip(labels)
        .find(|(t, _)| t.href.as_deref().is_some_and(|h| h.ends_with("#hinted")))
        .map(|(_, l)| l.clone())
}

/// Whether `got` is `want` device pixels, give or take rounding.
fn near(got: u32, want: f64) -> bool {
    (f64::from(got) - want).abs() <= 2.0
}

fn near_or_more(got: u32, want: f64) -> bool {
    f64::from(got) >= want - 2.0
}

pub fn next(state: BuiltinSmoke, input: Input) -> (BuiltinSmoke, Vec<Action>) {
    use {Input::*, Step::*};
    let BuiltinSmoke::At {
        step,
        runs,
        tall,
        pdf,
    } = state.clone()
    else {
        return (state, vec![]);
    };
    let to = |step| BuiltinSmoke::At {
        step,
        runs: 0,
        tall: tall.clone(),
        pdf: pdf.clone(),
    };
    let fail = |why: String| (BuiltinSmoke::Done, vec![Action::Fail(why)]);
    match (step, input) {
        (_, Worked(_)) if runs >= MAX_STEP_RUNS => fail(format!(
            "stuck at {step:?} for {MAX_STEP_RUNS} runs of CEF work"
        )),
        (Loaded, Worked(seen)) if seen.framed && !seen.loading => (
            to(ThemeReplies {
                theme: Theme::Dark,
                left: THEME_CALLS,
            }),
            vec![
                Action::Setting("screenshot.output", "file"),
                Action::Setting("pages.theme", "dark"),
            ],
        ),
        (ThemeReplies { theme, left }, Page(PageEvent::CdpReply { result, .. })) => match result {
            Err(e) => fail(format!("a page-theme call was refused: {e}")),
            Ok(_) if left <= 1 => (to(ThemeChecked(theme)), vec![Action::Eval(THEME_CHECK)]),
            Ok(_) => (
                to(ThemeReplies {
                    theme,
                    left: left - 1,
                }),
                vec![],
            ),
        },
        (ThemeChecked(theme), Page(event))
            if check(event).is_some_and(|v| v["check"] == "theme") =>
        {
            let v = check(event).unwrap_or_default();
            let (dark, bg) = (v["dark"].as_bool(), v["bg"].as_str().unwrap_or_default());
            let want = match theme {
                Theme::Dark => (Some(true), DARK_BG),
                Theme::Light => (Some(false), LIGHT_BG),
            };
            if (dark, bg) != want {
                return fail(format!(
                    "page theme {}: prefers dark {dark:?}, body {bg}, not {want:?}",
                    theme.word()
                ));
            }
            match theme {
                Theme::Dark => (
                    to(ThemeReplies {
                        theme: Theme::Light,
                        left: THEME_CALLS,
                    }),
                    vec![Action::Setting("pages.theme", "light")],
                ),
                Theme::Light => (to(Readerable), vec![Action::Setting("pages.theme", "auto")]),
            }
        }
        (Readerable, Worked(seen)) if seen.reader == (Reader::Off { readerable: true }) => (
            to(ReaderOn),
            vec![Action::Perform(UserAction::ToggleReader)],
        ),
        (ReaderOn, Worked(seen)) if seen.reader == Reader::On => {
            (to(ReaderChecked), vec![Action::Eval(READER_CHECK)])
        }
        (ReaderChecked, Page(event)) if check(event).is_some_and(|v| v["check"] == "reader") => {
            let v = check(event).unwrap_or_default();
            if v["h1"] != ARTICLE_TITLE || v["img"] != true {
                return fail(format!("the reader shows {v}"));
            }
            (
                to(ReaderLeft),
                vec![Action::Perform(UserAction::ToggleReader)],
            )
        }
        // Leaving reloads the page: its load starts, then ends.
        (ReaderLeft, Web(WebEvent::LoadStarted)) => (to(Reloading), vec![]),
        (Reloading, Web(WebEvent::LoadingChanged { loading: false, .. })) => {
            (to(Restored), vec![Action::Eval(RESTORED_CHECK)])
        }
        (Restored, Page(event)) if check(event).is_some_and(|v| v["check"] == "restored") => {
            let v = check(event).unwrap_or_default();
            if v["reader"] != false || v["title"] != ARTICLE_TITLE {
                return fail(format!("leaving the reader left {v}"));
            }
            (to(HintsShown), vec![Action::Perform(UserAction::LinkHints)])
        }
        (HintsShown, Worked(seen)) if matches!(seen.hints, Hints::Active { .. }) => {
            match hinted_label(&seen.hints) {
                Some(label) => (to(Hinted), vec![Action::Type(label)]),
                None => fail("no hint on the #hinted link".into()),
            }
        }
        (Hinted, Web(WebEvent::TitleChanged(title))) if title == HINTED => {
            (to(TallLoaded), vec![Action::Load(tall.clone())])
        }
        (TallLoaded, Web(WebEvent::LoadingChanged { loading: false, .. })) => (
            to(FullPage),
            vec![Action::Perform(UserAction::ScreenshotFullPage)],
        ),
        (FullPage, Saved(saved, seen)) => match saved.size {
            Some((_, h))
                if saved.file == Some(true) && near_or_more(h, TALL * f64::from(seen.scale)) =>
            {
                (
                    to(Visible),
                    vec![Action::Perform(UserAction::ScreenshotVisible)],
                )
            }
            _ => fail(format!(
                "the full page saved as {saved:?}, not {TALL} CSS px tall at {}x",
                seen.scale
            )),
        },
        (Visible, Saved(saved, seen)) => match saved.size {
            Some((w, h))
                if saved.file == Some(true)
                    && near(w, f64::from(seen.view.0))
                    && near(h, f64::from(seen.view.1)) =>
            {
                let (a, b) = REGION;
                (
                    to(Region),
                    vec![
                        Action::Perform(UserAction::ScreenshotRegion),
                        Action::Pick(a, b),
                    ],
                )
            }
            _ => fail(format!(
                "the visible page saved as {saved:?}, not the view's {:?}",
                seen.view
            )),
        },
        (Region, Saved(saved, seen)) if region_ok(&saved, seen.scale) => {
            (to(PdfLoaded), vec![Action::Load(pdf.clone())])
        }
        (Region, Saved(saved, seen)) => fail(format!(
            "the region saved as {saved:?}, not {REGION:?} at {}x",
            seen.scale
        )),
        (PdfLoaded, Web(WebEvent::LoadingChanged { loading: false, .. })) => {
            (to(PdfChecked), vec![Action::Eval(PDF_CHECK)])
        }
        (PdfChecked, Page(event)) if check(event).is_some_and(|v| v["check"] == "pdf") => {
            let v = check(event).unwrap_or_default();
            if v["type"] != "application/pdf" {
                return fail(format!("the PDF is not in Chromium's viewer: {v}"));
            }
            (to(PdfSaved), vec![Action::Perform(UserAction::Print)])
        }
        (PdfSaved, Page(PageEvent::DownloadFinished { path, .. })) => {
            if path.extension().is_some_and(|e| e == "pdf") {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                (
                    BuiltinSmoke::Done,
                    vec![Action::Pass(format!("the PDF saved as {name}"))],
                )
            } else {
                fail(format!("Print on the PDF saved {}", path.display()))
            }
        }
        (_, Worked(_)) => (
            BuiltinSmoke::At {
                step,
                runs: runs + 1,
                tall,
                pdf,
            },
            vec![],
        ),
        _ => (state, vec![]),
    }
}

/// Whether a region screenshot came out the dragged size (`REGION`, at `scale`).
pub fn region_ok(saved: &Saved, scale: f32) -> bool {
    let ((x0, y0), (x1, y1)) = REGION;
    let s = f64::from(scale);
    saved
        .size
        .is_some_and(|(w, h)| near(w, f64::from(x1 - x0) * s) && near(h, f64::from(y1 - y0) * s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::hints::Target;
    use wubble_engine::cdp::{CdpId, EvalId};

    fn seen() -> Seen {
        Seen {
            framed: true,
            loading: false,
            reader: Reader::default(),
            hints: Hints::Off,
            scale: 1.0,
            view: (800, 600),
        }
    }

    fn accepted() -> PageEvent {
        PageEvent::CdpReply {
            id: CdpId(1),
            result: Ok(Value::Null),
        }
    }

    fn answer(json: &str) -> PageEvent {
        PageEvent::EvalResult {
            id: EvalId(1),
            result: Ok(json.into()),
        }
    }

    fn step(s: BuiltinSmoke) -> Step {
        match s {
            BuiltinSmoke::At { step, .. } => step,
            BuiltinSmoke::Done => panic!("done"),
        }
    }

    #[test]
    fn dark_then_light_then_the_reader() {
        let s = BuiltinSmoke::new("tall".into(), "pdf".into());
        let (s, a) = next(s, Input::Worked(&seen()));
        assert_eq!(a[1], Action::Setting("pages.theme", "dark"));
        let (s, a) = next(s, Input::Page(&accepted()));
        assert!(a.is_empty());
        let (s, a) = next(s, Input::Worked(&seen()));
        assert!(a.is_empty());
        let (s, a) = next(s, Input::Page(&accepted()));
        assert_eq!(a, vec![Action::Eval(THEME_CHECK)]);
        let dark = answer(&format!(
            r#"{{"check":"theme","dark":true,"bg":"{DARK_BG}"}}"#
        ));
        let (s, a) = next(s, Input::Page(&dark));
        assert_eq!(a, vec![Action::Setting("pages.theme", "light")]);
        let (s, _) = next(s, Input::Page(&accepted()));
        let (s, a) = next(s, Input::Page(&accepted()));
        assert_eq!(a, vec![Action::Eval(THEME_CHECK)]);
        let light = answer(&format!(
            r#"{{"check":"theme","dark":false,"bg":"{LIGHT_BG}"}}"#
        ));
        let (s, _) = next(s, Input::Page(&light));
        assert_eq!(step(s.clone()), Step::Readerable);
        let found = Seen {
            reader: Reader::Off { readerable: true },
            ..seen()
        };
        let (s, a) = next(s, Input::Worked(&found));
        assert_eq!(a, vec![Action::Perform(UserAction::ToggleReader)]);
        let on = Seen {
            reader: Reader::On,
            ..seen()
        };
        let (s, _) = next(s, Input::Worked(&on));
        let built = answer(&format!(
            r#"{{"check":"reader","h1":"{ARTICLE_TITLE}","img":true}}"#
        ));
        let (s, a) = next(s, Input::Page(&built));
        assert_eq!(a, vec![Action::Perform(UserAction::ToggleReader)]);
        assert_eq!(step(s), Step::ReaderLeft);
    }

    #[test]
    fn a_wrong_colour_or_a_refused_call_fails() {
        let s = BuiltinSmoke::At {
            step: Step::ThemeChecked(Theme::Dark),
            runs: 0,
            tall: String::new(),
            pdf: String::new(),
        };
        let wrong = answer(&format!(
            r#"{{"check":"theme","dark":true,"bg":"{LIGHT_BG}"}}"#
        ));
        assert!(
            matches!(next(s, Input::Page(&wrong)), (BuiltinSmoke::Done, a) if matches!(&a[..], [Action::Fail(_)]))
        );
        let s = BuiltinSmoke::At {
            step: Step::ThemeReplies {
                theme: Theme::Dark,
                left: 2,
            },
            runs: 0,
            tall: String::new(),
            pdf: String::new(),
        };
        let refused = PageEvent::CdpReply {
            id: CdpId(1),
            result: Err("no".into()),
        };
        assert!(matches!(
            next(s, Input::Page(&refused)).0,
            BuiltinSmoke::Done
        ));
    }

    #[test]
    fn hints_type_the_links_label_and_screenshots_follow() {
        let s = BuiltinSmoke::At {
            step: Step::HintsShown,
            runs: 0,
            tall: "tall".into(),
            pdf: "pdf".into(),
        };
        let target = |href: &str| Target {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            href: Some(href.into()),
        };
        let shown = Seen {
            hints: Hints::Active {
                targets: vec![target("https://a.test/"), target("data:x#hinted")],
                labels: vec!["a".into(), "s".into()],
                typed: String::new(),
                new_tab: false,
            },
            ..seen()
        };
        let (s, a) = next(s, Input::Worked(&shown));
        assert_eq!(a, vec![Action::Type("s".into())]);
        let (s, a) = next(s, Input::Web(&WebEvent::TitleChanged(HINTED.into())));
        assert_eq!(a, vec![Action::Load("tall".into())]);
        let loaded = WebEvent::LoadingChanged {
            loading: false,
            can_go_back: true,
            can_go_forward: false,
        };
        let (s, a) = next(s, Input::Web(&loaded));
        assert_eq!(a, vec![Action::Perform(UserAction::ScreenshotFullPage)]);
        let host = seen();
        let short = Saved {
            file: Some(true),
            clipboard: None,
            size: Some((800, 600)),
        };
        assert!(matches!(
            next(s.clone(), Input::Saved(short, &host)).0,
            BuiltinSmoke::Done
        ));
        let tall = Saved {
            size: Some((800, 3000)),
            ..short
        };
        let (s, a) = next(s, Input::Saved(tall, &host));
        assert_eq!(a, vec![Action::Perform(UserAction::ScreenshotVisible)]);
        let (s, a) = next(s, Input::Saved(short, &host));
        assert_eq!(a[0], Action::Perform(UserAction::ScreenshotRegion));
        let region = Saved {
            size: Some((100, 80)),
            ..short
        };
        assert!(region_ok(&region, 1.0));
        assert!(!region_ok(&short, 1.0));
        assert!(!region_ok(&region, 2.0));
        let (s, a) = next(s, Input::Saved(region, &host));
        assert_eq!(a, vec![Action::Load("pdf".into())]);
        let (s, a) = next(s, Input::Web(&loaded));
        assert_eq!(a, vec![Action::Eval(PDF_CHECK)]);
        let viewer = answer(r#"{"check":"pdf","type":"application/pdf"}"#);
        let (s, a) = next(s, Input::Page(&viewer));
        assert_eq!(a, vec![Action::Perform(UserAction::Print)]);
        let saved = PageEvent::DownloadFinished {
            download: wubble_engine::page::DownloadId(1),
            path: "/d/doc.pdf".into(),
        };
        let (s, a) = next(s, Input::Page(&saved));
        assert_eq!(s, BuiltinSmoke::Done);
        assert!(matches!(&a[..], [Action::Pass(_)]));
    }

    #[test]
    fn a_stuck_step_fails() {
        let mut s = BuiltinSmoke::At {
            step: Step::Readerable,
            runs: 0,
            tall: String::new(),
            pdf: String::new(),
        };
        let mut last = vec![];
        for _ in 0..=MAX_STEP_RUNS {
            (s, last) = next(s, Input::Worked(&seen()));
        }
        assert_eq!(s, BuiltinSmoke::Done);
        assert!(matches!(&last[..], [Action::Fail(e)] if e.contains("Readerable")));
    }

    #[test]
    fn the_bundled_pages_have_what_the_smoke_needs() {
        let article = include_str!("../assets/article.html");
        for needle in [
            ARTICLE_TITLE,
            HINTED,
            "#hinted",
            "prefers-color-scheme: dark",
            "#123456",
            "#fafafa",
            "<img",
            "<pre>",
            "<ul>",
        ] {
            assert!(article.contains(needle), "article.html lacks {needle}");
        }
        assert!(include_str!("../assets/tall.html").contains("height: 3000px"));
        let pdf = include_bytes!("../assets/doc.pdf");
        assert!(pdf.starts_with(b"%PDF-1."));
        assert!(pdf.windows(6).any(|w| w == b"needle"));
    }
}
