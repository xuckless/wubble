//! The print sheet: what a print asks for, the CUPS command line that prints
//! it, and the sheet's lifecycle.
//!
//! Chromium renders the page to a PDF (CEF `PrintToPDF`, whose settings are
//! [`PdfOptions`]); "Save as PDF" keeps that file in the downloads folder, a
//! printer gets it through `lp` (CUPS, Linux and macOS). Printers are listed by
//! `lpstat -e` and `lpstat -d`, not libcups: no headers or link-time library
//! needed (TODO.md Decided "Printing"). Option names follow CEF's
//! `cef_pdf_print_settings_t` (include/internal/cef_types.h, BSD-3-Clause, no
//! code copied).

use std::path::PathBuf;

use crate::tab::TabId;

/// Where a print goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Destination {
    #[default]
    Pdf,
    /// A CUPS destination by name.
    Printer(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Paper {
    #[default]
    Letter,
    A4,
    Legal,
}

impl Paper {
    /// Portrait width and height in inches.
    pub fn inches(self) -> (f64, f64) {
        match self {
            Paper::Letter => (8.5, 11.0),
            Paper::A4 => (210.0 / 25.4, 297.0 / 25.4),
            Paper::Legal => (8.5, 14.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Margins {
    /// Chromium's default (about 0.4 inch).
    #[default]
    Default,
    None,
    /// A quarter inch all round.
    Minimum,
}

/// [`Margins::Minimum`], inches.
const MINIMUM_MARGIN: f64 = 0.25;

/// Scale limits, percent (Chromium's print preview allows 10 to 200).
pub const SCALE_MIN: u32 = 10;
pub const SCALE_MAX: u32 = 200;
/// Copies a printer job may ask for.
pub const COPIES_MAX: u32 = 99;

/// What the sheet's controls say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintSettings {
    pub destination: Destination,
    /// As typed: "" (all pages) or "1-3, 5".
    pub pages: String,
    pub landscape: bool,
    pub paper: Paper,
    pub margins: Margins,
    /// Percent.
    pub scale: u32,
    /// Background colours and images.
    pub background: bool,
    pub copies: u32,
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            destination: Destination::Pdf,
            pages: String::new(),
            landscape: false,
            paper: Paper::Letter,
            margins: Margins::Default,
            scale: 100,
            background: false,
            copies: 1,
        }
    }
}

/// How CEF renders the PDF (welding's `PdfOptions`, inches).
#[derive(Clone, Debug, PartialEq)]
pub struct PdfOptions {
    pub landscape: bool,
    pub background: bool,
    /// 1.0 is 100 %.
    pub scale: f64,
    /// Portrait width and height: CEF turns the page itself for landscape.
    pub paper_inches: (f64, f64),
    pub margins: PdfMargins,
    /// CEF's form, "1-3,5"; empty for every page.
    pub page_ranges: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PdfMargins {
    Default,
    None,
    /// Top, right, bottom, left, inches.
    Custom([f64; 4]),
}

/// Why a page range was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeError {
    /// Not a number, a range or a list of them.
    Malformed,
    /// Pages count from 1.
    Zero,
    /// "5-3": CEF fails the print.
    Reversed,
}

impl RangeError {
    pub fn message(&self) -> &'static str {
        match self {
            RangeError::Malformed => "Pages: use numbers and ranges, like 1-3, 5",
            RangeError::Zero => "Pages start at 1",
            RangeError::Reversed => "A range runs from the lower page to the higher",
        }
    }
}

/// The highest page number a range may name (a typo, not a document).
const PAGE_MAX: u32 = 100_000;

/// `pages` as CEF's page ranges ("1-3,5"), or why it cannot be: "" and
/// spaces are every page; each part is a page or a "from-to" range.
pub fn parse_range(pages: &str) -> Result<String, RangeError> {
    let page = |s: &str| -> Result<u32, RangeError> {
        let n: u32 = s.trim().parse().map_err(|_| RangeError::Malformed)?;
        match n {
            0 => Err(RangeError::Zero),
            n if n > PAGE_MAX => Err(RangeError::Malformed),
            n => Ok(n),
        }
    };
    if pages.trim().is_empty() {
        return Ok(String::new());
    }
    let mut parts = vec![];
    for part in pages.split(',') {
        let part = match part.split_once('-') {
            Some((from, to)) => {
                let (from, to) = (page(from)?, page(to)?);
                if from > to {
                    return Err(RangeError::Reversed);
                }
                format!("{from}-{to}")
            }
            None => page(part)?.to_string(),
        };
        parts.push(part);
    }
    Ok(parts.join(","))
}

/// What CEF is asked to render for `settings`.
pub fn to_pdf_options(settings: &PrintSettings) -> Result<PdfOptions, RangeError> {
    Ok(PdfOptions {
        landscape: settings.landscape,
        background: settings.background,
        scale: f64::from(settings.scale.clamp(SCALE_MIN, SCALE_MAX)) / 100.0,
        paper_inches: settings.paper.inches(),
        margins: match settings.margins {
            Margins::Default => PdfMargins::Default,
            Margins::None => PdfMargins::None,
            Margins::Minimum => PdfMargins::Custom([MINIMUM_MARGIN; 4]),
        },
        page_ranges: parse_range(&settings.pages)?,
    })
}

/// `lp`'s arguments to print `pdf` on `printer`. The PDF is already laid out
/// (paper, orientation, pages), so only the copies are the printer's.
pub fn lp_args(printer: &str, copies: u32, pdf: &std::path::Path) -> Vec<String> {
    vec![
        "-d".into(),
        printer.into(),
        "-n".into(),
        copies.clamp(1, COPIES_MAX).to_string(),
        "--".into(),
        pdf.to_string_lossy().into_owned(),
    ]
}

/// The destinations `lpstat -e` lists, one name per line.
pub fn parse_lpstat_e(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains(' '))
        .map(str::to_owned)
        .collect()
}

/// The default destination in `lpstat -d`'s answer
/// ("system default destination: NAME"; "no system default destination").
pub fn parse_lpstat_d(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let (_, name) = l.split_once("default destination:")?;
        let name = name.trim();
        (!name.is_empty()).then(|| name.to_owned())
    })
}

// ----------------------------------------------------------------- the sheet

/// Where a submitted print is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// CEF is writing the PDF.
    Rendering,
    /// `lp` has the PDF.
    Sending,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Sheet {
    #[default]
    Closed,
    Open {
        tab: TabId,
        settings: PrintSettings,
        printers: Vec<String>,
        /// Why the last try did not print, shown on the sheet.
        error: Option<String>,
    },
    Working {
        tab: TabId,
        settings: PrintSettings,
        printers: Vec<String>,
        /// The PDF being written: the saved file, or the printer's temporary one.
        path: PathBuf,
        stage: Stage,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Ctrl+P or the menu's Print: the sheet for `tab`, the printers there
    /// are and the system's default one, which is chosen first.
    Open {
        tab: TabId,
        printers: Vec<String>,
        default: Option<String>,
    },
    /// A control changed.
    Edit(PrintSettings),
    /// The Print / Save button, with the file the host picked: a unique name
    /// in the downloads folder for a PDF, a temporary file for a printer.
    Submit(PathBuf),
    Cancel,
    /// CEF wrote the PDF (or not).
    Rendered {
        ok: bool,
    },
    /// `lp` took the job, or why not.
    Sent(Result<(), String>),
    /// The tab went away.
    TabClosed(TabId),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    PrintToPdf {
        tab: TabId,
        path: PathBuf,
        options: PdfOptions,
    },
    /// Run `lp` with these arguments on a thread; answer with `Event::Sent`.
    Lp(Vec<String>),
    /// A finished file in the downloads list.
    ListDownload(PathBuf),
    /// A printer's temporary PDF is no longer needed.
    RemoveFile(PathBuf),
    Toast(String),
}

fn name(path: &std::path::Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

pub fn transition(sheet: Sheet, event: Event) -> (Sheet, Vec<Effect>) {
    use Effect as F;
    match (sheet, event) {
        (
            Sheet::Closed,
            Event::Open {
                tab,
                printers,
                default,
            },
        ) => {
            let destination = default
                .filter(|d| printers.contains(d))
                .map_or(Destination::Pdf, Destination::Printer);
            let settings = PrintSettings {
                destination,
                ..PrintSettings::default()
            };
            let open = Sheet::Open {
                tab,
                settings,
                printers,
                error: None,
            };
            (open, vec![])
        }
        (Sheet::Open { tab, printers, .. }, Event::Edit(settings)) => {
            // A range being typed is checked as it changes; any edit retires
            // the last failure's message.
            let error = parse_range(&settings.pages)
                .err()
                .map(|e| e.message().to_owned());
            let open = Sheet::Open {
                tab,
                settings,
                printers,
                error,
            };
            (open, vec![])
        }
        (
            Sheet::Open {
                tab,
                settings,
                printers,
                ..
            },
            Event::Submit(path),
        ) => match to_pdf_options(&settings) {
            Ok(options) => (
                Sheet::Working {
                    tab,
                    settings,
                    printers,
                    path: path.clone(),
                    stage: Stage::Rendering,
                },
                vec![F::PrintToPdf { tab, path, options }],
            ),
            Err(e) => {
                let open = Sheet::Open {
                    tab,
                    settings,
                    printers,
                    error: Some(e.message().to_owned()),
                };
                (open, vec![])
            }
        },
        (Sheet::Open { .. }, Event::Cancel) => (Sheet::Closed, vec![]),
        (
            Sheet::Working {
                tab,
                settings,
                printers,
                path,
                stage: Stage::Rendering,
            },
            Event::Rendered { ok },
        ) => match (&settings.destination, ok) {
            (Destination::Pdf, true) => (
                Sheet::Closed,
                vec![
                    F::Toast(format!("Saved {}", name(&path))),
                    F::ListDownload(path),
                ],
            ),
            (Destination::Printer(printer), true) => {
                let args = lp_args(printer, settings.copies, &path);
                let working = Sheet::Working {
                    tab,
                    settings,
                    printers,
                    path,
                    stage: Stage::Sending,
                };
                (working, vec![F::Lp(args)])
            }
            // CEF says only that it failed: a range past the last page is the
            // usual reason (the page cannot be counted beforehand).
            (destination, false) => {
                let mut effects = vec![];
                if matches!(destination, Destination::Printer(_)) {
                    effects.push(F::RemoveFile(path));
                }
                let error = if settings.pages.trim().is_empty() {
                    "Could not print this page"
                } else {
                    "Could not print these pages: is the range past the last page?"
                };
                let open = Sheet::Open {
                    tab,
                    settings,
                    printers,
                    error: Some(error.into()),
                };
                (open, effects)
            }
        },
        (
            Sheet::Working {
                tab,
                settings,
                printers,
                path,
                stage: Stage::Sending,
            },
            Event::Sent(result),
        ) => {
            let printer = match &settings.destination {
                Destination::Printer(p) => p.clone(),
                Destination::Pdf => String::new(),
            };
            match result {
                Ok(()) => (
                    Sheet::Closed,
                    vec![F::RemoveFile(path), F::Toast(format!("Sent to {printer}"))],
                ),
                Err(why) => {
                    let open = Sheet::Open {
                        tab,
                        settings,
                        printers,
                        error: Some(format!("{printer}: {why}")),
                    };
                    (open, vec![F::RemoveFile(path)])
                }
            }
        }
        (Sheet::Open { tab, .. }, Event::TabClosed(closed)) if tab == closed => {
            (Sheet::Closed, vec![])
        }
        // The PDF of a closed tab is never written; `lp` already has its file.
        (
            Sheet::Working {
                tab,
                path,
                stage: Stage::Rendering,
                settings,
                ..
            },
            Event::TabClosed(closed),
        ) if tab == closed => {
            let effects = match settings.destination {
                Destination::Printer(_) => vec![F::RemoveFile(path)],
                Destination::Pdf => vec![],
            };
            (Sheet::Closed, effects)
        }
        // A second Ctrl+P, a click while working, a stale answer.
        (sheet, _) => (sheet, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: TabId = TabId(1);

    #[test]
    fn page_ranges_are_normalised_for_cef() {
        assert_eq!(parse_range(""), Ok(String::new()));
        assert_eq!(parse_range("  "), Ok(String::new()));
        assert_eq!(parse_range("3"), Ok("3".into()));
        assert_eq!(parse_range(" 1 - 3 , 5,7-7 "), Ok("1-3,5,7-7".into()));
    }

    #[test]
    fn bad_page_ranges_say_why() {
        assert_eq!(parse_range("5-3"), Err(RangeError::Reversed));
        assert_eq!(parse_range("0"), Err(RangeError::Zero));
        assert_eq!(parse_range("1-0"), Err(RangeError::Zero));
        assert_eq!(parse_range("a"), Err(RangeError::Malformed));
        assert_eq!(parse_range("1,,2"), Err(RangeError::Malformed));
        assert_eq!(parse_range("1-2-3"), Err(RangeError::Malformed));
        assert_eq!(parse_range("-3"), Err(RangeError::Malformed));
        assert_eq!(parse_range("99999999999"), Err(RangeError::Malformed));
        assert_eq!(parse_range("100001"), Err(RangeError::Malformed));
    }

    #[test]
    fn options_are_cef_s_inches_and_fractions() {
        let mut s = PrintSettings::default();
        let o = to_pdf_options(&s).unwrap();
        assert_eq!(o.paper_inches, (8.5, 11.0));
        assert_eq!(o.scale, 1.0);
        assert_eq!(o.margins, PdfMargins::Default);
        assert!(!o.landscape && !o.background);
        s.paper = Paper::A4;
        s.landscape = true;
        s.scale = 500;
        s.margins = Margins::Minimum;
        s.background = true;
        s.pages = "2-4".into();
        let o = to_pdf_options(&s).unwrap();
        let (w, h) = o.paper_inches;
        assert!((w - 8.2677).abs() < 1e-3 && (h - 11.6929).abs() < 1e-3);
        assert!(o.landscape && o.background);
        assert_eq!(o.scale, 2.0);
        assert_eq!(o.margins, PdfMargins::Custom([0.25; 4]));
        assert_eq!(o.page_ranges, "2-4");
        s.scale = 0;
        s.paper = Paper::Legal;
        s.margins = Margins::None;
        let o = to_pdf_options(&s).unwrap();
        assert_eq!(o.scale, 0.1);
        assert_eq!(o.paper_inches, (8.5, 14.0));
        assert_eq!(o.margins, PdfMargins::None);
    }

    #[test]
    fn lp_prints_the_file_on_the_printer() {
        let args = lp_args("Office", 3, std::path::Path::new("/tmp/-x.pdf"));
        assert_eq!(args, ["-d", "Office", "-n", "3", "--", "/tmp/-x.pdf"]);
        assert_eq!(lp_args("P", 0, "/a".as_ref())[3], "1");
        assert_eq!(lp_args("P", 500, "/a".as_ref())[3], "99");
    }

    #[test]
    fn lpstat_answers_are_read() {
        assert_eq!(
            parse_lpstat_e("Office_Laser\nPDF\n\n"),
            ["Office_Laser", "PDF"]
        );
        // `lpstat -e` with no printers prints to stderr; stdout is empty.
        assert!(parse_lpstat_e("").is_empty());
        assert!(parse_lpstat_e("lpstat: No destinations added.").is_empty());
        assert_eq!(
            parse_lpstat_d("system default destination: Office_Laser\n"),
            Some("Office_Laser".into())
        );
        assert_eq!(parse_lpstat_d("no system default destination\n"), None);
    }

    fn open(printers: &[&str], default: Option<&str>) -> Sheet {
        transition(
            Sheet::Closed,
            Event::Open {
                tab: T,
                printers: printers.iter().map(|p| p.to_string()).collect(),
                default: default.map(str::to_owned),
            },
        )
        .0
    }

    fn settings(sheet: &Sheet) -> PrintSettings {
        match sheet {
            Sheet::Open { settings, .. } | Sheet::Working { settings, .. } => settings.clone(),
            Sheet::Closed => panic!("closed"),
        }
    }

    fn error(sheet: &Sheet) -> Option<String> {
        match sheet {
            Sheet::Open { error, .. } => error.clone(),
            _ => None,
        }
    }

    #[test]
    fn the_sheet_opens_on_the_default_printer_or_pdf() {
        assert_eq!(settings(&open(&[], None)).destination, Destination::Pdf);
        assert_eq!(
            settings(&open(&["A", "B"], Some("B"))).destination,
            Destination::Printer("B".into())
        );
        // A default that is not listed is not chosen.
        assert_eq!(
            settings(&open(&["A"], Some("Z"))).destination,
            Destination::Pdf
        );
        let (again, fx) = transition(
            open(&[], None),
            Event::Open {
                tab: TabId(2),
                printers: vec![],
                default: None,
            },
        );
        assert!(fx.is_empty());
        assert!(matches!(again, Sheet::Open { tab: T, .. }));
    }

    #[test]
    fn a_bad_range_keeps_the_sheet_open_with_its_reason() {
        let bad = PrintSettings {
            pages: "4-2".into(),
            ..PrintSettings::default()
        };
        let (s, _) = transition(open(&[], None), Event::Edit(bad));
        assert_eq!(error(&s).as_deref(), Some(RangeError::Reversed.message()));
        let (s, fx) = transition(s, Event::Submit("/d/p.pdf".into()));
        assert!(fx.is_empty());
        assert!(matches!(s, Sheet::Open { .. }));
        // Fixing it clears the message.
        let (s, _) = transition(s, Event::Edit(PrintSettings::default()));
        assert_eq!(error(&s), None);
    }

    #[test]
    fn save_as_pdf_renders_then_lists_the_file() {
        let (s, fx) = transition(open(&[], None), Event::Submit("/d/Page.pdf".into()));
        assert!(matches!(
            s,
            Sheet::Working {
                stage: Stage::Rendering,
                ..
            }
        ));
        assert!(matches!(
            &fx[..],
            [Effect::PrintToPdf { tab: T, path, .. }] if path == std::path::Path::new("/d/Page.pdf")
        ));
        let (s, _) = transition(s.clone(), Event::Cancel);
        assert!(matches!(s, Sheet::Working { .. }), "working ignores cancel");
        let (s, fx) = transition(s, Event::Rendered { ok: true });
        assert_eq!(s, Sheet::Closed);
        assert_eq!(
            fx,
            [
                Effect::Toast("Saved Page.pdf".into()),
                Effect::ListDownload("/d/Page.pdf".into())
            ]
        );
    }

    #[test]
    fn a_failed_render_reopens_with_an_error() {
        let ranged = PrintSettings {
            pages: "40".into(),
            ..PrintSettings::default()
        };
        let (s, _) = transition(open(&[], None), Event::Edit(ranged));
        let (s, _) = transition(s, Event::Submit("/d/p.pdf".into()));
        let (s, fx) = transition(s, Event::Rendered { ok: false });
        assert!(fx.is_empty());
        assert!(error(&s).unwrap().contains("past the last page"));
    }

    #[test]
    fn a_printer_gets_the_pdf_through_lp_then_it_is_removed() {
        let s = open(&["Office"], Some("Office"));
        let (s, _) = transition(s, Event::Submit("/tmp/w.pdf".into()));
        let (s, fx) = transition(s, Event::Rendered { ok: true });
        assert_eq!(
            fx,
            [Effect::Lp(lp_args("Office", 1, "/tmp/w.pdf".as_ref()))]
        );
        let (done, fx) = transition(s.clone(), Event::Sent(Ok(())));
        assert_eq!(done, Sheet::Closed);
        assert_eq!(
            fx,
            [
                Effect::RemoveFile("/tmp/w.pdf".into()),
                Effect::Toast("Sent to Office".into())
            ]
        );
        let (failed, fx) = transition(s, Event::Sent(Err("no filter".into())));
        assert_eq!(fx, [Effect::RemoveFile("/tmp/w.pdf".into())]);
        assert_eq!(error(&failed).as_deref(), Some("Office: no filter"));
    }

    #[test]
    fn closing_the_tab_closes_the_sheet() {
        let (s, fx) = transition(open(&[], None), Event::TabClosed(TabId(9)));
        assert!(fx.is_empty() && matches!(s, Sheet::Open { .. }));
        let (s, _) = transition(s, Event::TabClosed(T));
        assert_eq!(s, Sheet::Closed);
        let printing = open(&["P"], Some("P"));
        let (s, _) = transition(printing, Event::Submit("/tmp/t.pdf".into()));
        let (s, fx) = transition(s, Event::TabClosed(T));
        assert_eq!(s, Sheet::Closed);
        assert_eq!(fx, [Effect::RemoveFile("/tmp/t.pdf".into())]);
    }
}
