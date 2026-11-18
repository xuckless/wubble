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

/// `Title (720p).mp4`; the note is left out when empty.
pub fn media_file_name(title: &str, note: &str, container: Container) -> String {
    let title = if title.trim().is_empty() {
        "video"
    } else {
        title.trim()
    };
    let ext = container.ext();
    let name = if note.is_empty() {
        format!("{title}.{ext}")
    } else {
        format!("{title} ({note}).{ext}")
    };
    sanitize_file_name(&name)
}

/// The name a track of a pair is kept under when it cannot be joined: `Title (video).mp4`.
pub fn track_file_name(title: &str, track: &str, container: Container) -> String {
    media_file_name(title, track, container)
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
    fn names_are_sanitised_and_unique() {
        assert_eq!(
            media_file_name("A/B: c?", "720p", Container::Mp4),
            "B_ c_ (720p).mp4"
        );
        assert_eq!(media_file_name("  ", "", Container::Ts), "video.ts");
        assert_eq!(media_file_name("CON", "", Container::Mp3), "_CON.mp3");
        let long = "\u{1F3B5}".repeat(100);
        let name = media_file_name(&long, "1080p", Container::Webm);
        assert!(name.len() <= 255 && name.ends_with(".webm"));
        assert!(name.is_char_boundary(name.len() - ".webm".len()));
        assert_eq!(
            track_file_name("T", "audio", Container::M4a),
            "T (audio).m4a"
        );
        let taken = |p: &Path| p.ends_with("T (720p).mp4");
        let path = unique_name(
            Path::new("/d"),
            &media_file_name("T", "720p", Container::Mp4),
            taken,
        );
        assert_eq!(path.unwrap(), Path::new("/d/T (720p) (1).mp4"));
    }
}
