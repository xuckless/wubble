//! Keyboard link hints: short labels on the page's clickable elements; typing one clicks it.
//!
//! All hint logic is here; the page script (`crates/wubble/assets/hints.js`) only measures the
//! targets and draws the labels it is given. [`Hints`] is the one state, [`transition`] its pure
//! machine, and its effects are what the host does to the page.
//!
//! [`labels`] is ported from Vimium (`content_scripts/link_hints.js`,
//! `AlphabetHints.hintStrings`, MIT, Phil Crosby and Ilya Sukhar): breadth-first strings, sorted,
//! reversed, so no label is a prefix of another and a full label is always a unique match.

use serde_json::Value;

/// The home row (the default) and Vimium's own `linkHintCharacters`.
pub const ALPHABETS: [(&str, &str); 2] = [("home-row", "asdfghjkl"), ("vimium", "sadfjklewcmpgh")];

/// The stored words of [`ALPHABETS`]: the setting's choices.
pub const ALPHABET_WORDS: [&str; 2] = [ALPHABETS[0].0, ALPHABETS[1].0];

/// The letters of the alphabet the setting names, the home row for anything else.
pub fn alphabet(word: &str) -> &'static str {
    ALPHABETS
        .iter()
        .find(|(w, _)| *w == word)
        .map_or(ALPHABETS[0].1, |(_, letters)| letters)
}

/// `count` distinct labels over `alphabet`, none a prefix of another, the shortest first spread
/// over the page. Empty for fewer than two letters (no prefix-free set exists).
pub fn labels(count: usize, alphabet: &str) -> Vec<String> {
    let chars: Vec<char> = alphabet.chars().collect();
    if chars.len() < 2 || count == 0 {
        return vec![];
    }
    let mut hints = vec![String::new()];
    let mut offset = 0;
    while hints.len() - offset < count || hints.len() == 1 {
        let hint = hints[offset].clone();
        offset += 1;
        hints.extend(chars.iter().map(|ch| format!("{ch}{hint}")));
    }
    let mut out = hints[offset..offset + count].to_vec();
    out.sort();
    out.into_iter().map(|s| s.chars().rev().collect()).collect()
}

/// One clickable element, as the page script measured it: its box in viewport CSS pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// A link's address, for Shift (open in a new tab).
    pub href: Option<String>,
}

impl Target {
    /// The centre, viewport CSS pixels: where the click lands.
    pub fn centre(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// The targets in the page script's answer (`[{x, y, w, h, href}]`); malformed entries are
/// skipped, anything else is no target.
pub fn targets_from_json(text: &str) -> Vec<Target> {
    let Ok(Value::Array(items)) = serde_json::from_str(text) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|item| {
            let num = |k: &str| item.get(k)?.as_f64().filter(|v| v.is_finite());
            Some(Target {
                x: num("x")?,
                y: num("y")?,
                w: num("w")?,
                h: num("h")?,
                href: item
                    .get("href")
                    .and_then(Value::as_str)
                    .filter(|h| !h.is_empty())
                    .map(str::to_owned),
            })
        })
        .collect()
}

/// Hint mode of the visible page.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Hints {
    #[default]
    Off,
    /// The page script is measuring; its answer is the script `id`'s.
    Collecting(u64),
    /// Labels on screen; `typed` is what the user typed so far, `new_tab` whether Shift was held.
    Active {
        targets: Vec<Target>,
        labels: Vec<String>,
        typed: String,
        new_tab: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum HintEvent {
    /// The action: start, or stop if already on.
    Toggle,
    /// The measuring script went out as `id`.
    Sent(u64),
    /// The answer of script `id`.
    Targets(u64, Vec<Target>),
    /// A letter, lowercased by the host, and whether Shift was held.
    Key(char, bool),
    Backspace,
    /// Escape, a scroll, a navigation, a tab switch: the labels go.
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HintEffect {
    /// Run the measuring script, then report `Sent(id)`.
    Collect,
    /// Draw these labels (each at its target), `typed` marked as typed.
    Draw {
        items: Vec<(Target, String)>,
        typed: String,
    },
    /// A real pointer click at the target's centre.
    Click(Target),
    OpenInNewTab(String),
    /// Remove the labels from the page.
    Clear,
    Toast(&'static str),
}

/// The labels of `labels` that start with `typed`, with their targets.
fn matching(targets: &[Target], labels: &[String], typed: &str) -> Vec<(Target, String)> {
    targets
        .iter()
        .zip(labels)
        .filter(|(_, l)| l.starts_with(typed))
        .map(|(t, l)| (t.clone(), l.clone()))
        .collect()
}

/// One step of hint mode. `alphabet` labels the targets when they arrive.
pub fn transition(state: Hints, event: HintEvent, alphabet: &str) -> (Hints, Vec<HintEffect>) {
    use HintEffect as E;
    use HintEvent::*;
    match (state, event) {
        (Hints::Off, Toggle) => (Hints::Off, vec![E::Collect]),
        (Hints::Off, Sent(id)) => (Hints::Collecting(id), vec![]),
        (Hints::Collecting(_) | Hints::Active { .. }, Toggle | Cancel) => {
            (Hints::Off, vec![E::Clear])
        }
        (Hints::Collecting(want), Targets(id, targets)) if id == want => {
            let labels = labels(targets.len(), alphabet);
            if labels.is_empty() {
                return (Hints::Off, vec![E::Toast("Nothing to click on this page")]);
            }
            let items = matching(&targets, &labels, "");
            let active = Hints::Active {
                targets,
                labels,
                typed: String::new(),
                new_tab: false,
            };
            (
                active,
                vec![E::Draw {
                    items,
                    typed: String::new(),
                }],
            )
        }
        (
            Hints::Active {
                targets,
                labels,
                typed,
                new_tab,
            },
            Key(ch, shift),
        ) => {
            let next = format!("{typed}{ch}");
            let new_tab = new_tab || shift;
            let items = matching(&targets, &labels, &next);
            match items.as_slice() {
                // A key no label continues with is ignored.
                [] => (
                    Hints::Active {
                        targets,
                        labels,
                        typed,
                        new_tab,
                    },
                    vec![],
                ),
                [(target, label)] if *label == next => {
                    let go = match (&target.href, new_tab) {
                        (Some(href), true) => E::OpenInNewTab(href.clone()),
                        _ => E::Click(target.clone()),
                    };
                    (Hints::Off, vec![E::Clear, go])
                }
                _ => (
                    Hints::Active {
                        targets,
                        labels,
                        typed: next.clone(),
                        new_tab,
                    },
                    vec![E::Draw { items, typed: next }],
                ),
            }
        }
        (
            Hints::Active {
                targets,
                labels,
                mut typed,
                new_tab,
            },
            Backspace,
        ) => {
            typed.pop();
            let items = matching(&targets, &labels, &typed);
            let draw = E::Draw {
                items,
                typed: typed.clone(),
            };
            let active = Hints::Active {
                targets,
                labels,
                typed,
                new_tab,
            };
            (active, vec![draw])
        }
        // Stale answers, keys while measuring, a cancel with nothing on.
        (state, _) => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "asdfghjkl";

    fn prefix_free(labels: &[String]) -> bool {
        labels.iter().enumerate().all(|(i, a)| {
            labels
                .iter()
                .enumerate()
                .all(|(j, b)| i == j || !b.starts_with(a.as_str()))
        })
    }

    #[test]
    fn labels_are_unique_and_prefix_free() {
        for (n, alphabet) in [
            (1, HOME),
            (9, HOME),
            (10, HOME),
            (81, HOME),
            (82, HOME),
            (300, HOME),
            (5, "fjdk"),
            (17, "fjdk"),
            (64, "fjdk"),
        ] {
            let l = labels(n, alphabet);
            assert_eq!(l.len(), n, "{n} over {alphabet}");
            let mut unique = l.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), n, "{n} over {alphabet}: duplicates");
            assert!(prefix_free(&l), "{n} over {alphabet}: {l:?}");
            assert!(l.iter().all(|s| s.chars().all(|c| alphabet.contains(c))));
        }
    }

    #[test]
    fn labels_are_as_short_as_vimiums() {
        assert_eq!(labels(1, HOME), ["a"]);
        assert!(labels(9, HOME).iter().all(|l| l.len() == 1));
        // Ten targets: one letter opens up into nine two-letter labels.
        let ten = labels(10, HOME);
        assert_eq!(ten.iter().filter(|l| l.len() == 1).count(), 8);
        assert!(labels(81, HOME).iter().all(|l| l.len() == 2));
        assert!(labels(300, HOME).iter().all(|l| l.len() <= 3));
        assert!(labels(3, "a").is_empty() && labels(0, HOME).is_empty());
    }

    #[test]
    fn alphabets_are_the_settings_words() {
        assert_eq!(alphabet("home-row"), HOME);
        assert_eq!(alphabet("vimium"), "sadfjklewcmpgh");
        assert_eq!(alphabet("nope"), HOME);
        for (_, letters) in ALPHABETS {
            assert!(letters.chars().count() >= 4);
        }
    }

    #[test]
    fn targets_parse_from_the_scripts_answer() {
        let json = r#"[{"x":1,"y":2,"w":30,"h":10,"href":"https://a.test/"},
            {"x":5,"y":6,"w":7,"h":8,"href":""},{"x":"bad"},{"x":1,"y":1,"w":1,"h":1}]"#;
        let t = targets_from_json(json);
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].href.as_deref(), Some("https://a.test/"));
        assert_eq!(t[0].centre(), (16.0, 7.0));
        assert_eq!(t[1].href, None);
        assert!(targets_from_json("null").is_empty());
        assert!(targets_from_json("{").is_empty());
    }

    fn target(i: usize) -> Target {
        Target {
            x: i as f64 * 10.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            href: Some(format!("https://a.test/{i}")),
        }
    }

    /// Toggle, sent as script 7, and `n` targets back: hint mode on.
    fn active(n: usize) -> Hints {
        let (s, e) = transition(Hints::Off, HintEvent::Toggle, HOME);
        assert_eq!((s.clone(), e), (Hints::Off, vec![HintEffect::Collect]));
        let (s, _) = transition(s, HintEvent::Sent(7), HOME);
        assert_eq!(s, Hints::Collecting(7));
        let targets: Vec<_> = (0..n).map(target).collect();
        let (s, e) = transition(s, HintEvent::Targets(7, targets), HOME);
        assert!(matches!(e.as_slice(), [HintEffect::Draw { items, .. }] if items.len() == n));
        s
    }

    #[test]
    fn a_full_label_clicks_its_target() {
        let s = active(3);
        let Hints::Active { labels, .. } = &s else {
            panic!()
        };
        let first = labels[0].chars().next().unwrap();
        let (s, e) = transition(s, HintEvent::Key(first, false), HOME);
        assert_eq!(s, Hints::Off);
        assert_eq!(e, vec![HintEffect::Clear, HintEffect::Click(target(0))]);
    }

    #[test]
    fn shift_opens_a_link_in_a_new_tab() {
        let s = active(3);
        let Hints::Active { labels, .. } = &s else {
            panic!()
        };
        let ch = labels[1].chars().next().unwrap();
        let (_, e) = transition(s, HintEvent::Key(ch, true), HOME);
        assert_eq!(
            e,
            vec![
                HintEffect::Clear,
                HintEffect::OpenInNewTab("https://a.test/1".into())
            ]
        );
    }

    #[test]
    fn a_prefix_narrows_a_wrong_key_is_ignored_and_backspace_widens() {
        let s = active(10);
        let Hints::Active { labels, .. } = &s else {
            panic!()
        };
        let long = labels.iter().find(|l| l.len() == 2).unwrap().clone();
        let mut chars = long.chars();
        let (a, b) = (chars.next().unwrap(), chars.next().unwrap());
        let (s, e) = transition(s, HintEvent::Key(a, false), HOME);
        assert!(
            matches!(&e[..], [HintEffect::Draw { items, typed }] if items.len() == 2 && typed.len() == 1)
        );
        let (s2, e) = transition(s.clone(), HintEvent::Key('z', false), HOME);
        assert_eq!((&s2, e), (&s, vec![]));
        let (s, e) = transition(s, HintEvent::Backspace, HOME);
        assert!(
            matches!(&e[..], [HintEffect::Draw { items, typed }] if items.len() == 10 && typed.is_empty())
        );
        let (s, _) = transition(s, HintEvent::Key(a, false), HOME);
        let (s, e) = transition(s, HintEvent::Key(b, false), HOME);
        assert_eq!(s, Hints::Off);
        assert!(matches!(&e[..], [HintEffect::Clear, HintEffect::Click(_)]));
    }

    #[test]
    fn cancel_and_toggle_clear_and_stale_answers_are_ignored() {
        for event in [HintEvent::Cancel, HintEvent::Toggle] {
            assert_eq!(
                transition(active(2), event.clone(), HOME),
                (Hints::Off, vec![HintEffect::Clear])
            );
            assert_eq!(
                transition(Hints::Collecting(3), event, HOME),
                (Hints::Off, vec![HintEffect::Clear])
            );
        }
        let stale = HintEvent::Targets(2, vec![target(0)]);
        assert_eq!(
            transition(Hints::Collecting(3), stale.clone(), HOME),
            (Hints::Collecting(3), vec![])
        );
        assert_eq!(transition(Hints::Off, stale, HOME), (Hints::Off, vec![]));
        assert_eq!(
            transition(Hints::Off, HintEvent::Cancel, HOME),
            (Hints::Off, vec![])
        );
        assert_eq!(
            transition(Hints::Collecting(3), HintEvent::Key('a', false), HOME),
            (Hints::Collecting(3), vec![])
        );
    }

    #[test]
    fn nothing_to_click_says_so() {
        let (s, e) = transition(Hints::Collecting(1), HintEvent::Targets(1, vec![]), HOME);
        assert_eq!(s, Hints::Off);
        assert_eq!(e, vec![HintEffect::Toast("Nothing to click on this page")]);
    }
}
