//! A segmented WebVTT stream put together. An HLS subtitle rendition is a playlist of small
//! WebVTT files, each with its own `WEBVTT` header (and often an `X-TIMESTAMP-MAP`); joined as
//! they come, the file has a header in the middle, which players stop at. [`merge_webvtt`] keeps
//! the first header and the cues: later headers go, and a cue repeated across a segment
//! boundary (a line that spans two segments is in both) is kept once. As yt-dlp's
//! `yt_dlp/webvtt.py` does for its HLS subtitles (yt-dlp/yt-dlp, Unlicense; rules ported, no
//! code copied). Cue times are kept as written: `X-TIMESTAMP-MAP` maps them to the stream's
//! MPEG-TS clock, which the saved video's own start cancels (a map of `MPEGTS:900000,
//! LOCAL:00:00:00.000` is ten seconds on both).

/// The cues of WebVTT segments joined as they came, under the first segment's header.
pub fn merge_webvtt(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut header: Option<Vec<&str>> = None;
    let mut cues: Vec<String> = Vec::new();
    // Blocks are separated by blank lines; a block that opens with `WEBVTT` is a header.
    let mut block: Vec<&str> = Vec::new();
    for line in text.lines() {
        // A segment that did not end with a blank line: its successor's header still starts a
        // block of its own.
        if is_header(line) && !block.is_empty() {
            end_block(&mut block, &mut header, &mut cues);
        }
        if line.trim().is_empty() {
            end_block(&mut block, &mut header, &mut cues);
        } else {
            block.push(line);
        }
    }
    end_block(&mut block, &mut header, &mut cues);
    let mut out = header.map_or_else(|| "WEBVTT".to_owned(), |h| h.join("\n"));
    for cue in cues {
        out.push_str("\n\n");
        out.push_str(&cue);
    }
    out.push('\n');
    out
}

/// A segment's first line (`WEBVTT`, maybe after a byte order mark, maybe with a title).
fn is_header(line: &str) -> bool {
    let line = line.trim_start_matches('\u{feff}');
    line == "WEBVTT" || line.starts_with("WEBVTT ") || line.starts_with("WEBVTT\t")
}

/// One block read: the first header is kept (without its timestamp map), later ones dropped; a
/// cue is kept unless it repeats the one before.
fn end_block<'t>(
    block: &mut Vec<&'t str>,
    header: &mut Option<Vec<&'t str>>,
    cues: &mut Vec<String>,
) {
    match block.first() {
        None => return,
        Some(first) if is_header(first) => {
            if header.is_none() {
                *header = Some(
                    block
                        .iter()
                        .map(|l| l.trim_start_matches('\u{feff}'))
                        .filter(|l| !l.starts_with("X-TIMESTAMP-MAP"))
                        .collect(),
                );
            }
        }
        Some(_) => {
            let cue = block.join("\n");
            if cues.last() != Some(&cue) {
                cues.push(cue);
            }
        }
    }
    block.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_merge_under_one_header() {
        let cases = [
            (
                "two segments with timestamp maps, a cue spanning both",
                "WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:900000,LOCAL:00:00:00.000\n\n00:00.000 --> 00:02.000\nOne\n\n00:01.500 --> 00:04.000\nSpans\n\nWEBVTT\nX-TIMESTAMP-MAP=MPEGTS:900000,LOCAL:00:00:00.000\n\n00:01.500 --> 00:04.000\nSpans\n\n00:04.000 --> 00:06.000\nTwo\n",
                "WEBVTT\n\n00:00.000 --> 00:02.000\nOne\n\n00:01.500 --> 00:04.000\nSpans\n\n00:04.000 --> 00:06.000\nTwo\n",
            ),
            (
                "a segment without a trailing blank line, CRLF, a BOM",
                "\u{feff}WEBVTT - Title\r\n\r\n1\r\n00:00.000 --> 00:01.000\r\nA\r\n\u{feff}WEBVTT\r\n\r\n00:01.000 --> 00:02.000\r\nB",
                "WEBVTT - Title\n\n1\n00:00.000 --> 00:01.000\nA\n\n00:01.000 --> 00:02.000\nB\n",
            ),
            (
                "one plain file stays as it is",
                "WEBVTT\n\nNOTE made by hand\n\n00:00.000 --> 00:01.000 line:90%\n<v Bob>Hi\n",
                "WEBVTT\n\nNOTE made by hand\n\n00:00.000 --> 00:01.000 line:90%\n<v Bob>Hi\n",
            ),
            (
                "a header-only segment adds nothing",
                "WEBVTT\n\n00:00.000 --> 00:01.000\nA\n\nWEBVTT\n\n",
                "WEBVTT\n\n00:00.000 --> 00:01.000\nA\n",
            ),
            ("nothing at all", "", "WEBVTT\n"),
            (
                "the same words at two times are two cues",
                "WEBVTT\n\n00:00.000 --> 00:01.000\nYes\n\n00:05.000 --> 00:06.000\nYes\n",
                "WEBVTT\n\n00:00.000 --> 00:01.000\nYes\n\n00:05.000 --> 00:06.000\nYes\n",
            ),
        ];
        for (name, input, want) in cases {
            assert_eq!(merge_webvtt(input), want, "{name}");
        }
    }
}
