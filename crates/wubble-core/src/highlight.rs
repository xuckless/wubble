//! "Copy Link to Highlight": the page's URL with a text fragment naming the selection
//! (`#:~:text=`, WICG Scroll-to-Text Fragment).
//!
//! Chromium's generator (`components/shared_highlighting/core/browser`, BSD-3-Clause) asks the
//! page for the selection's block structure and widens the selector until it is unique. wubble
//! has only the selection's text, so it builds the selector the generator starts from: the
//! exact text when it is short and within one block, else its start and end words.

use crate::url::percent_encode_into;

/// Chromium's `kExactTextMaxChars`: a longer selection is named by its start and end.
const EXACT_TEXT_CHARS: usize = 300;
/// Words kept at each end of a range selector.
const RANGE_WORDS: usize = 3;

/// A fragment's text: everything outside the unreserved set escaped, and `-` too, which the
/// directive syntax uses for its prefix and suffix (`,` and `&` are escaped already).
fn encode(text: &str) -> String {
    let mut out = String::new();
    percent_encode_into(text, &mut out);
    out.replace('-', "%2D")
}

/// `page_url` with a text fragment for `selection`; `None` for a page that is not `http(s)` or
/// a selection with no text. Any fragment the URL had is replaced, as Chromium does.
pub fn link_to_highlight(page_url: &str, selection: &str) -> Option<String> {
    if !(page_url.starts_with("http://") || page_url.starts_with("https://")) {
        return None;
    }
    let base = page_url.split('#').next().unwrap_or(page_url);
    // One entry per line (a block boundary in the selection), whitespace collapsed.
    let lines: Vec<Vec<&str>> = selection
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|words| !words.is_empty())
        .collect();
    let (first, last) = (lines.first()?, lines.last()?);
    let text = first.join(" ");
    let exact = lines.len() == 1
        && (text.chars().count() <= EXACT_TEXT_CHARS || first.len() <= 2 * RANGE_WORDS);
    let directive = if exact {
        encode(&text)
    } else {
        let head = first[..first.len().min(RANGE_WORDS)].join(" ");
        let tail = last[last.len().saturating_sub(RANGE_WORDS)..].join(" ");
        format!("{},{}", encode(&head), encode(&tail))
    };
    Some(format!("{base}#:~:text={directive}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fragment_names_the_selection() {
        let long = "word ".repeat(100);
        let long_line = format!("alpha beta gamma {long}omega psi chi");
        let cases: [(&str, &str, Option<&str>); 13] = [
            // short, one block: the exact text, whitespace collapsed
            (
                "https://a.test/p",
                "hello  world",
                Some("https://a.test/p#:~:text=hello%20world"),
            ),
            (
                "https://a.test/p",
                "  hi \n",
                Some("https://a.test/p#:~:text=hi"),
            ),
            // `-`, `,` and `&` are directive syntax
            (
                "https://a.test/p",
                "a-b, c&d",
                Some("https://a.test/p#:~:text=a%2Db%2C%20c%26d"),
            ),
            (
                "https://a.test/",
                "caf\u{e9}",
                Some("https://a.test/#:~:text=caf%C3%A9"),
            ),
            // an existing fragment is replaced, the query kept
            (
                "https://a.test/p?q=1#top",
                "x",
                Some("https://a.test/p?q=1#:~:text=x"),
            ),
            // long: start and end words
            (
                "https://a.test/p",
                long_line.as_str(),
                Some("https://a.test/p#:~:text=alpha%20beta%20gamma,omega%20psi%20chi"),
            ),
            // across blocks: the first block's start, the last block's end
            (
                "https://a.test/p",
                "one two three four\nfive six seven eight",
                Some("https://a.test/p#:~:text=one%20two%20three,six%20seven%20eight"),
            ),
            (
                "https://a.test/p",
                "Title\nBody",
                Some("https://a.test/p#:~:text=Title,Body"),
            ),
            // nothing to name, or a page that has no fragments
            ("https://a.test/p", " \n ", None),
            ("https://a.test/p", "", None),
            ("file:///tmp/a.html", "x", None),
            ("wubble://start", "x", None),
            ("about:blank", "x", None),
        ];
        for (page, selection, want) in cases {
            assert_eq!(
                link_to_highlight(page, selection).as_deref(),
                want,
                "{page} {selection:?}"
            );
        }
    }
}
