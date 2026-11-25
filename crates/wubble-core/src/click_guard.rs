//! What a click aimed at, and what it may lead to: the rules against the "invisible pane"
//! abuse (a transparent overlay over the page whose click opens an ad in a new tab, a
//! popunder, or sends the page to another site) and against downloads nobody asked for.
//!
//! The blocker's content script measures the element the pointer went down on, before any
//! page handler runs ([`Click`]); the host judges with these rules what the click led to: a
//! window ([`opened`]), a top-level navigation ([`navigated`]) or a download ([`download`]).
//! What is held is never dropped in silence: a window or navigation waits in the address
//! pill's blocked pop-ups ("open anyway"), a download asks in the permission prompt.
//!
//! Design references, no code copied: Chromium's popup blocker and its abusive experience
//! intervention (`components/blocked_content`, `components/subresource_filter`: a click grants
//! a gesture, so only what the click visibly aimed at is trusted), Safari's "Allow downloads
//! on <site>?" (a per-site choice, asked on a site's first download), Chromium's automatic
//! downloads setting (`DownloadRequestLimiter`: a download without a gesture asks).

use crate::site::Decision;

/// The element a click went down on, as the page drew it.
#[derive(Clone, Debug, PartialEq)]
pub struct Click {
    /// Its own opacity times its ancestors' (0 to 1).
    pub opacity: f32,
    /// The share of its frame's viewport its box covers (0 to 1).
    pub cover: f32,
    /// It draws something itself: a background colour or image, text, a picture, a video, a
    /// form control (the page itself counts as drawn).
    pub painted: bool,
    /// It is, or is inside, a link, a button or a form control.
    pub control: bool,
    /// The link it is in, absolute.
    pub href: Option<String>,
    /// That link has a `download` attribute.
    pub download: bool,
}

/// Below this opacity an element is not seen.
pub const UNSEEN_OPACITY: f32 = 0.1;
/// An element that draws nothing over at least this share of the page is an overlay.
pub const OVERLAY_COVER: f32 = 0.5;
/// A window or navigation this soon after a click is the click's doing.
pub const OPEN_WITHIN_MS: u64 = 1_000;
/// A download this soon after a click may be the click's (its server answers first).
pub const DOWNLOAD_WITHIN_MS: u64 = 10_000;

impl Click {
    /// Drawn so faintly, or not at all over most of the page, that nobody aimed at it. The
    /// content script's overlay removal reads the same two numbers (`cosmetic.js`, `unclick`).
    pub fn unseen(&self) -> bool {
        self.opacity < UNSEEN_OPACITY || (self.cover >= OVERLAY_COVER && !self.painted)
    }
}

/// Why a window a click opened is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// The click landed on something not visibly drawn: an invisible overlay.
    Unseen,
    /// The click was on a link to another site, and the page opened a third one instead.
    NotTheLink,
}

/// The registrable site of a URL (`None` for a page without one: `about:blank`, internal).
fn site(url: &str) -> Option<String> {
    crate::tidy::site_of(url)
}

/// The click of `age_ms` ago, if it is recent enough to have led to what happens now.
fn recent(click: Option<&Click>, age_ms: u64, within: u64) -> Option<&Click> {
    click.filter(|_| age_ms <= within)
}

/// Whether the window the page at `from` opened at `to` right after `click` is held. Only a
/// window on another site is ever held (a window on `about:blank` counts as another site: it is
/// where popunders start). A click on a visible link or button that opens what it shows, a
/// sign-in pop-up from a button, a share window from its link, all open.
pub fn opened(click: Option<&Click>, age_ms: u64, from: &str, to: &str) -> Option<Held> {
    let click = recent(click, age_ms, OPEN_WITHIN_MS)?;
    let to = site(to);
    if site(from) == to && to.is_some() {
        return None;
    }
    if click.unseen() {
        return Some(Held::Unseen);
    }
    // a web link that goes to another site, followed by a window on yet another one (an
    // `href="#"` or `javascript:` link is the page's own button)
    match click.href.as_deref().and_then(site) {
        Some(link) if Some(&link) != site(from).as_ref() && Some(&link) != to.as_ref() => {
            Some(Held::NotTheLink)
        }
        _ => None,
    }
}

/// Whether the page at `from` going to `to` right after `click` is held: only a navigation to
/// another site from an invisible overlay (a redirect behind a visible link is the link's).
pub fn navigated(click: Option<&Click>, age_ms: u64, from: &str, to: &str) -> bool {
    recent(click, age_ms, OPEN_WITHIN_MS).is_some_and(|c| c.unseen() && site(from) != site(to))
}

/// Whether the user aimed at a download: a recent click on a visibly drawn link or button.
pub fn aimed(click: Option<&Click>, age_ms: u64) -> bool {
    recent(click, age_ms, DOWNLOAD_WITHIN_MS)
        .is_some_and(|c| !c.unseen() && (c.control || c.download))
}

/// What a download gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// It starts.
    Go,
    /// It waits for "Allow <site> to download files?".
    Ask,
    /// The site is blocked from downloading; the host says so (never in silence).
    Refuse,
}

/// Every case of a download: the site's remembered choice (`Permission::Downloads`), whether
/// a click aimed at it ([`aimed`]), whether the site already downloaded this run; one row each.
/// To change the policy, change a row.
pub const DOWNLOADS: &[(Option<Decision>, bool, bool, Start)] = &[
    (Some(Decision::Allow), true, true, Start::Go),
    (Some(Decision::Allow), true, false, Start::Go),
    (Some(Decision::Allow), false, true, Start::Go),
    (Some(Decision::Allow), false, false, Start::Go),
    (Some(Decision::Block), true, true, Start::Refuse),
    (Some(Decision::Block), true, false, Start::Refuse),
    (Some(Decision::Block), false, true, Start::Refuse),
    (Some(Decision::Block), false, false, Start::Refuse),
    // a site that downloaded already, and a click on its download link or button
    (None, true, true, Start::Go),
    // the site's first download, aimed or not (Safari asks then too)
    (None, true, false, Start::Ask),
    // nothing the user clicked asked for it
    (None, false, true, Start::Ask),
    (None, false, false, Start::Ask),
];

pub fn download(site: Option<Decision>, aimed: bool, before: bool) -> Start {
    DOWNLOADS
        .iter()
        .find(|(s, a, b, _)| *s == site && *a == aimed && *b == before)
        .map_or(Start::Ask, |row| row.3)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A visible button.
    fn button() -> Click {
        Click {
            opacity: 1.0,
            cover: 0.01,
            painted: true,
            control: true,
            href: None,
            download: false,
        }
    }

    fn link(href: &str) -> Click {
        Click {
            href: Some(href.into()),
            ..button()
        }
    }

    /// A full-page transparent `<a>` (the "invisible pane").
    fn pane() -> Click {
        Click {
            opacity: 1.0,
            cover: 1.0,
            painted: false,
            control: true,
            href: Some("https://ads.test/go".into()),
            download: false,
        }
    }

    #[test]
    fn what_is_unseen() {
        let rows = [
            ("a visible button", button(), false),
            ("the invisible pane", pane(), true),
            (
                "opacity 0",
                Click {
                    opacity: 0.0,
                    ..button()
                },
                true,
            ),
            (
                "opacity 0.09",
                Click {
                    opacity: 0.09,
                    ..button()
                },
                true,
            ),
            (
                "opacity 0.1 is seen",
                Click {
                    opacity: UNSEEN_OPACITY,
                    ..button()
                },
                false,
            ),
            (
                "half the page, drawing nothing",
                Click {
                    cover: OVERLAY_COVER,
                    painted: false,
                    ..button()
                },
                true,
            ),
            (
                "just under half the page, drawing nothing",
                Click {
                    cover: 0.49,
                    painted: false,
                    ..button()
                },
                false,
            ),
            (
                "a drawn full-page panel",
                Click {
                    cover: 1.0,
                    ..button()
                },
                false,
            ),
            (
                "a small unpainted box (a stretched card link)",
                Click {
                    cover: 0.2,
                    painted: false,
                    ..button()
                },
                false,
            ),
        ];
        for (name, click, unseen) in rows {
            assert_eq!(click.unseen(), unseen, "{name}");
        }
    }

    const PAGE: &str = "https://news.test/story";

    #[test]
    fn windows() {
        type Row<'a> = (&'a str, Option<Click>, u64, &'a str, Option<Held>);
        let rows: Vec<Row> = vec![
            (
                "the pane opens an ad",
                Some(pane()),
                50,
                "https://ads.test/x",
                Some(Held::Unseen),
            ),
            (
                "the pane opens a blank popunder",
                Some(pane()),
                50,
                "about:blank",
                Some(Held::Unseen),
            ),
            (
                "the pane opens a page of the site",
                Some(pane()),
                50,
                "https://www.news.test/b",
                None,
            ),
            (
                "the pane, a second too late",
                Some(pane()),
                OPEN_WITHIN_MS + 1,
                "https://ads.test/x",
                None,
            ),
            (
                "the pane, at the limit",
                Some(pane()),
                OPEN_WITHIN_MS,
                "https://ads.test/x",
                Some(Held::Unseen),
            ),
            ("no click known", None, 0, "https://ads.test/x", None),
            (
                "a button's sign-in pop-up",
                Some(button()),
                50,
                "https://accounts.test/o",
                None,
            ),
            (
                "a link opens what it shows",
                Some(link("https://other.test/a")),
                50,
                "https://other.test/a",
                None,
            ),
            (
                "a link opens another page of its site",
                Some(link("https://other.test/a")),
                50,
                "https://www.other.test/b",
                None,
            ),
            (
                "a link to one site, a window on another",
                Some(link("https://other.test/a")),
                50,
                "https://ads.test/x",
                Some(Held::NotTheLink),
            ),
            (
                "a link to one site, a blank window",
                Some(link("https://other.test/a")),
                50,
                "about:blank",
                Some(Held::NotTheLink),
            ),
            (
                "an href=# link's sign-in pop-up",
                Some(link("https://news.test/story#")),
                50,
                "https://accounts.test/o",
                None,
            ),
            (
                "a javascript: link",
                Some(link("javascript:void(0)")),
                50,
                "https://accounts.test/o",
                None,
            ),
        ];
        for (name, click, age, to, held) in rows {
            assert_eq!(opened(click.as_ref(), age, PAGE, to), held, "{name}");
        }
    }

    #[test]
    fn navigations() {
        let rows: Vec<(&str, Option<Click>, u64, &str, bool)> = vec![
            (
                "the pane sends the page away",
                Some(pane()),
                50,
                "https://ads.test/x",
                true,
            ),
            (
                "the pane, within the site",
                Some(pane()),
                50,
                "https://news.test/next",
                false,
            ),
            (
                "the pane, long ago",
                Some(pane()),
                OPEN_WITHIN_MS + 1,
                "https://ads.test/x",
                false,
            ),
            (
                "a visible link elsewhere",
                Some(link("https://ads.test/x")),
                50,
                "https://ads.test/x",
                false,
            ),
            (
                "a visible link's redirect",
                Some(link("https://t.test/r")),
                50,
                "https://ads.test/x",
                false,
            ),
            ("no click", None, 0, "https://ads.test/x", false),
        ];
        for (name, click, age, to, held) in rows {
            assert_eq!(navigated(click.as_ref(), age, PAGE, to), held, "{name}");
        }
    }

    #[test]
    fn aimed_downloads() {
        let text = Click {
            control: false,
            ..button()
        };
        let rows: Vec<(&str, Option<Click>, u64, bool)> = vec![
            ("a download button", Some(button()), 200, true),
            (
                "a link with download",
                Some(Click {
                    control: false,
                    download: true,
                    ..button()
                }),
                200,
                true,
            ),
            ("a slow server", Some(button()), DOWNLOAD_WITHIN_MS, true),
            (
                "too long ago",
                Some(button()),
                DOWNLOAD_WITHIN_MS + 1,
                false,
            ),
            ("a click on text", Some(text), 200, false),
            ("the invisible pane", Some(pane()), 200, false),
            ("no click (a script, a key)", None, 0, false),
        ];
        for (name, click, age, want) in rows {
            assert_eq!(aimed(click.as_ref(), age), want, "{name}");
        }
    }

    #[test]
    fn every_download_case_has_one_row() {
        for site in [None, Some(Decision::Allow), Some(Decision::Block)] {
            for aimed in [true, false] {
                for before in [true, false] {
                    let n = DOWNLOADS
                        .iter()
                        .filter(|(s, a, b, _)| *s == site && *a == aimed && *b == before)
                        .count();
                    assert_eq!(n, 1, "{site:?} aimed={aimed} before={before}");
                }
            }
        }
    }

    #[test]
    fn the_download_policy() {
        use Start::*;
        let rows = [
            (
                "an allowed site, anything",
                Some(Decision::Allow),
                false,
                false,
                Go,
            ),
            (
                "a blocked site, even aimed",
                Some(Decision::Block),
                true,
                true,
                Refuse,
            ),
            (
                "an aimed download from a site that downloaded",
                None,
                true,
                true,
                Go,
            ),
            ("a site's first download asks", None, true, false, Ask),
            ("a script's download asks", None, false, true, Ask),
            ("a script's first download asks", None, false, false, Ask),
        ];
        for (name, site, aimed, before, want) in rows {
            assert_eq!(download(site, aimed, before), want, "{name}");
        }
    }
}
