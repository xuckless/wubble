//! A saved stream's file name: the title (the page's, trimmed of the site's suffix, or the
//! stream's own), the quality, the container's extension, through the downloads' own
//! sanitising (`download::sanitize_file_name`, Chromium's rules); uniqueness is
//! `download::unique_name`'s.

use super::Container;
use crate::download::sanitize_file_name;

/// Separators sites put between a page's title and their name.
const SUFFIX_SEPARATORS: [&str; 4] = [" - ", " | ", " \u{2014} ", " \u{b7} "];

/// Words a site suffix has at most ("Video - YouTube", "Clip | The Daily Site").
const SUFFIX_WORDS: usize = 3;

/// A page title without a trailing site name.
pub fn page_title(title: &str) -> &str {
    let title = title.trim();
    SUFFIX_SEPARATORS
        .iter()
        .filter_map(|sep| title.rfind(sep).map(|at| (at, sep.len())))
        .max_by_key(|(at, _)| *at)
        .filter(|&(at, len)| at > 0 && title[at + len..].split_whitespace().count() <= SUFFIX_WORDS)
        .map_or(title, |(at, _)| title[..at].trim_end())
}

/// Extensions a title may carry from a file name (a media page's title is its file's name).
const MEDIA_EXTS: [&str; 14] = [
    "mp4", "m4v", "mov", "webm", "mkv", "ogv", "ogg", "opus", "mp3", "m4a", "aac", "ts", "flac",
    "wav",
];

/// The title without a trailing media extension (`clip.mp4` is `clip`): the saved file adds its
/// own, and `clip.mp4.mp4` names nothing.
fn without_media_ext(title: &str) -> &str {
    match title.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty() && MEDIA_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)) =>
        {
            stem
        }
        _ => title,
    }
}

/// A title from a stream's URL, for a page without one: the last path segment, decoded, with
/// no query and no media extension (`.../Clip%20One.webm?x=1` is `Clip One`).
pub fn url_title(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let last = path.rsplit('/').next().unwrap_or_default();
    let decoded = ::url::form_urlencoded::parse(last.replace('+', "%2B").as_bytes())
        .next()
        .map(|(k, v)| {
            if v.is_empty() {
                k.into_owned()
            } else {
                format!("{k}={v}")
            }
        })
        .unwrap_or_default();
    without_media_ext(&decoded).to_owned()
}

/// `Title (720p).mp4` (or `Title (audio).m4a`, a track of a pair that could not be joined);
/// the note is left out when empty.
pub fn media_file_name(title: &str, note: &str, container: Container) -> String {
    let title = without_media_ext(title.trim());
    let title = if title.is_empty() { "video" } else { title };
    let ext = container.ext();
    let name = if note.is_empty() {
        format!("{title}.{ext}")
    } else {
        format!("{title} ({note}).{ext}")
    };
    sanitize_file_name(&name)
}

/// A subtitle file beside the video: the video's file name without its extension (`stem`),
/// the language when it is a plain code (`en`, `pt-BR`; anything else is left out), and the
/// subtitles' extension: `Title (720p).en.vtt`, which players pick up beside `Title (720p).mp4`.
pub fn subtitle_file_name(stem: &str, lang: Option<&str>, container: Container) -> String {
    let stem = if stem.trim().is_empty() {
        "video"
    } else {
        stem.trim()
    };
    let lang = lang.map(str::trim).filter(|l| {
        (1..=16).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    let name = match lang {
        Some(lang) => format!("{stem}.{lang}.{}", container.ext()),
        None => format!("{stem}.{}", container.ext()),
    };
    sanitize_file_name(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::unique_name;
    use std::path::Path;

    #[test]
    fn site_suffixes_go() {
        assert_eq!(page_title("A Song - YouTube"), "A Song");
        assert_eq!(
            page_title("Rock - Paper - Scissors | The Daily Site"),
            "Rock - Paper - Scissors"
        );
        assert_eq!(
            page_title("Up - and away with a long tail here"),
            "Up - and away with a long tail here"
        );
        assert_eq!(page_title(" - YouTube"), "- YouTube");
        assert_eq!(page_title("Plain"), "Plain");
    }

    #[test]
    fn a_title_from_a_url_is_its_file_name_decoded() {
        assert_eq!(
            url_title("https://a.test/v/Clip%20One.webm?x=1#t"),
            "Clip One"
        );
        assert_eq!(url_title("https://a.test/v/a+b.mp4"), "a+b");
        assert_eq!(url_title("https://a.test/dl?id=5"), "dl");
        assert_eq!(url_title("https://a.test/"), "");
    }

    #[test]
    fn subtitles_take_the_videos_name_and_their_language() {
        let cases = [
            (
                "Clip (720p)",
                Some("en"),
                Container::Vtt,
                "Clip (720p).en.vtt",
            ),
            ("Clip", Some("pt-BR"), Container::Srt, "Clip.pt-BR.srt"),
            ("Clip", None, Container::Ttml, "Clip.ttml"),
            ("Clip", Some(" fr "), Container::Vtt, "Clip.fr.vtt"),
            ("Clip", Some("English (auto)"), Container::Vtt, "Clip.vtt"),
            ("Clip", Some("../x"), Container::Vtt, "Clip.vtt"),
            ("Clip", Some(""), Container::Vtt, "Clip.vtt"),
            ("  ", Some("en"), Container::Vtt, "video.en.vtt"),
            ("A/B", Some("de"), Container::Vtt, "B.de.vtt"),
        ];
        for (stem, lang, container, want) in cases {
            assert_eq!(
                subtitle_file_name(stem, lang, container),
                want,
                "{stem} {lang:?}"
            );
        }
    }

    #[test]
    fn names_are_sanitised_and_unique() {
        assert_eq!(
            media_file_name("A/B: c?", "720p", Container::Mp4),
            "B_ c_ (720p).mp4"
        );
        assert_eq!(media_file_name("  ", "", Container::Ts), "video.ts");
        // A media page's title is its file's name: the extension is not doubled.
        assert_eq!(
            media_file_name("Crane_fly.webm", "", Container::Webm),
            "Crane_fly.webm"
        );
        assert_eq!(
            media_file_name("Clip.MP4", "720p", Container::Mp4),
            "Clip (720p).mp4"
        );
        assert_eq!(media_file_name("v1.2", "", Container::Mp4), "v1.2.mp4");
        assert_eq!(media_file_name("CON", "", Container::Mp3), "_CON.mp3");
        let long = "\u{1F3B5}".repeat(100);
        let name = media_file_name(&long, "1080p", Container::Webm);
        assert!(name.len() <= 255 && name.ends_with(".webm"));
        assert!(name.is_char_boundary(name.len() - ".webm".len()));
        let taken = |p: &Path| p.ends_with("T (720p).mp4");
        let path = unique_name(
            Path::new("/d"),
            &media_file_name("T", "720p", Container::Mp4),
            taken,
        );
        assert_eq!(path.unwrap(), Path::new("/d/T (720p) (1).mp4"));
    }
}
