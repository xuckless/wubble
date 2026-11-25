//! uBlock Origin's `$popup` and `$popunder` network options, which adblock 0.13 does not know
//! (it drops a rule with an option it does not recognise). The rules are taken out of each list
//! into an engine of their own, rewritten to a request type that engine is asked with:
//!
//! - `$popup` (and `$all`, which in uBO includes it): a window the page opens is matched by its
//!   URL, with the opener as the page (`domain=`, first or third party), as a `main_frame`
//!   request.
//! - `$popunder`: the window that was opened is the page, and the opener is what is matched
//!   (uBO: "the popup is the opener, and the opener is the popup"), as an `other` request.
//!
//! Semantics from gorhill/uBlock `src/js/static-net-filtering.js` (`popup`, `popunder`, `all`)
//! and its wiki ("Static filter syntax"), GPL-3.0; no code copied.

/// Each option this engine keeps, and the type it becomes there.
const OPTIONS: &[(&str, &str)] = &[
    ("popup", "document"),
    ("all", "document"),
    ("popunder", "other"),
];

/// The request type a `$popup` rule is asked with.
pub(crate) const POPUP_TYPE: &str = "main_frame";
/// The request type a `$popunder` rule is asked with.
pub(crate) const POPUNDER_TYPE: &str = "other";

/// Cosmetic and scriptlet rules carry one of these; none is a network rule.
const COSMETIC: &[&str] = &["##", "#@#", "#?#", "#$#", "#%#"];

/// `line` as the pop-up engine's rule, if it is a network rule with one of [`OPTIONS`].
fn rule(line: &str) -> Option<String> {
    let line = line.trim();
    if line.starts_with('!') || line.starts_with('[') || COSMETIC.iter().any(|c| line.contains(c))
    {
        return None;
    }
    let (pattern, options) = line.rsplit_once('$')?;
    let mut hit = false;
    let options: Vec<&str> = options
        .split(',')
        .map(|o| match OPTIONS.iter().find(|(name, _)| *name == o.trim()) {
            Some((_, kind)) => {
                hit = true;
                *kind
            }
            None => o,
        })
        .collect();
    hit.then(|| format!("{pattern}${}", options.join(",")))
}

/// The pop-up rules of a list text (already preprocessed), one per line.
pub(crate) fn rules(text: &str) -> String {
    let mut out = String::new();
    for r in text.lines().filter_map(rule) {
        out.push_str(&r);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_popup_rules_are_taken_and_rewritten() {
        for (line, want) in [
            ("||ads.test^$popup", Some("||ads.test^$document")),
            (
                "||ads.test^$popup,third-party",
                Some("||ads.test^$document,third-party"),
            ),
            (
                "||ads.test^$popup,domain=a.test|b.test",
                Some("||ads.test^$document,domain=a.test|b.test"),
            ),
            ("@@||ok.test^$popup", Some("@@||ok.test^$document")),
            ("||bad.test^$all", Some("||bad.test^$document")),
            ("||site.test^$popunder", Some("||site.test^$other")),
            ("/^https?:\\/\\/x\\.test\\/$/$popup", Some("/^https?:\\/\\/x\\.test\\/$/$document")),
            ("  ||ads.test^$popup  ", Some("||ads.test^$document")),
            ("||ads.test^", None),
            ("||ads.test^$script", None),
            ("||ads.test^$popups", None),
            ("||ads.test^$~popup", None),
            ("! ||ads.test^$popup", None),
            ("[Adblock Plus 2.0]", None),
            ("example.com##.popup", None),
            ("example.com##+js(nowoif, $popup)", None),
            ("", None),
        ] {
            assert_eq!(rule(line).as_deref(), want, "{line}");
        }
    }

    #[test]
    fn a_list_s_rules_one_per_line() {
        assert_eq!(
            rules("||a.test^$popup\n||b.test^\n||c.test^$popunder\n"),
            "||a.test^$document\n||c.test^$other\n"
        );
        assert_eq!(rules(""), "");
    }
}
