//! The print sheet's host side: it runs `wubble_core::print::Sheet`'s effects
//! (CEF's PDF, `lp`, the downloads list) and shows the sheet, a store overlay whose content
//! (the sheet, its job) is the content map's (`layers::Content::Print`). Printers come
//! from the CUPS command line (`lpstat`), on a thread; a system without it
//! (Windows) offers Save as PDF only.

use std::path::Path;
use std::process::Command as Process;

use wubble_core::overlay::{Kind, Layer, Phase};
use wubble_core::print::{self, Destination, Effect, Event, Margins, Paper, PrintSettings, Sheet};
use wubble_core::tab::TabId;
use wubble_ui::{AppWindow, PrintSheetView};

use super::Host;
use super::layers::Content;

/// The sheet's first destination.
const SAVE_AS_PDF: &str = "Save as PDF";

/// The printers `lpstat` lists and its default; none when CUPS is not there.
fn printers() -> (Vec<String>, Option<String>) {
    let run = |arg: &str| {
        Process::new("lpstat")
            .arg(arg)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    (
        print::parse_lpstat_e(&run("-e")),
        print::parse_lpstat_d(&run("-d")),
    )
}

/// Runs `lp`; its complaint when it refuses the job.
fn lp(args: &[String]) -> Result<(), String> {
    let out = Process::new("lp")
        .args(args)
        .output()
        .map_err(|e| format!("cannot run lp: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if why.is_empty() {
            format!("lp failed ({})", out.status)
        } else {
            why
        })
    }
}

/// The sheet's controls as settings (indices as the sheet orders them).
#[allow(clippy::too_many_arguments)]
pub(super) fn edited_settings(
    printers: &[String],
    destination: i32,
    pages: &str,
    landscape: bool,
    paper: i32,
    margins: i32,
    scale: i32,
    background: bool,
    copies: i32,
) -> PrintSettings {
    let destination = usize::try_from(destination - 1)
        .ok()
        .and_then(|i| printers.get(i))
        .map_or(Destination::Pdf, |p| Destination::Printer(p.clone()));
    PrintSettings {
        destination,
        pages: pages.to_owned(),
        landscape,
        paper: match paper {
            1 => Paper::A4,
            2 => Paper::Legal,
            _ => Paper::Letter,
        },
        margins: match margins {
            1 => Margins::None,
            2 => Margins::Minimum,
            _ => Margins::Default,
        },
        scale: u32::try_from(scale)
            .unwrap_or(100)
            .clamp(print::SCALE_MIN, print::SCALE_MAX),
        background,
        copies: u32::try_from(copies)
            .unwrap_or(1)
            .clamp(1, print::COPIES_MAX),
    }
}

/// What the sheet shows for `sheet` (`None`: closed).
fn view(sheet: Option<&Sheet>, serial: i32) -> PrintSheetView {
    let Some(sheet) = sheet else {
        return PrintSheetView::default();
    };
    let (settings, printers, error, busy) = match sheet {
        Sheet::Open {
            settings,
            printers,
            error,
            ..
        } => (settings, printers, error.clone().unwrap_or_default(), false),
        Sheet::Working {
            settings, printers, ..
        } => (settings, printers, String::new(), true),
    };
    let destinations: Vec<slint::SharedString> = std::iter::once(SAVE_AS_PDF.into())
        .chain(printers.iter().map(|p| p.as_str().into()))
        .collect();
    let destination = match &settings.destination {
        Destination::Pdf => 0,
        Destination::Printer(p) => printers.iter().position(|q| q == p).map_or(0, |i| i + 1),
    };
    PrintSheetView {
        open: true,
        busy,
        serial,
        error: error.into(),
        destinations: std::rc::Rc::new(slint::VecModel::from(destinations)).into(),
        destination: destination as i32,
        pages: settings.pages.as_str().into(),
        landscape: settings.landscape,
        paper: match settings.paper {
            Paper::Letter => 0,
            Paper::A4 => 1,
            Paper::Legal => 2,
        },
        margins: match settings.margins {
            Margins::Default => 0,
            Margins::None => 1,
            Margins::Minimum => 2,
        },
        scale: settings.scale as i32,
        background: settings.background,
        copies: settings.copies as i32,
    }
}

impl Host {
    /// Ctrl+P and the menu's Print: the print sheet for `tab`, once the printers are listed.
    /// A PDF in Chromium's viewer is saved as itself, a download of its address: printing the
    /// viewer gives a blank page, and CEF has no print dialog here for the viewer's own button.
    pub(super) fn print(&mut self, tab: TabId) {
        if self.is_pdf(tab) {
            let url = self.committed.get(&tab).cloned();
            if let (Some(url), Some(view)) = (url, self.tab_view(tab)) {
                view.start_download(&url);
            }
            return;
        }
        if self.print_sheet(false).is_some() {
            return;
        }
        let id = self.w.id;
        let spawned = std::thread::Builder::new()
            .name("wubble-lpstat".into())
            .spawn(move || {
                let (printers, default) = printers();
                let _ = slint::invoke_from_event_loop(move || {
                    crate::with_host_in(id, |host| {
                        host.print_event(Event::Open {
                            tab,
                            printers,
                            default,
                        })
                    })
                });
            });
        if spawned.is_err() {
            self.print_event(Event::Open {
                tab,
                printers: vec![],
                default: None,
            });
        }
    }

    /// The sheet's controls changed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn print_edited(
        &mut self,
        destination: i32,
        pages: &str,
        landscape: bool,
        paper: i32,
        margins: i32,
        scale: i32,
        background: bool,
        copies: i32,
    ) {
        let Some(Sheet::Open { printers, .. }) = self.print_sheet(false) else {
            return;
        };
        let settings = edited_settings(
            &printers,
            destination,
            pages,
            landscape,
            paper,
            margins,
            scale,
            background,
            copies,
        );
        self.print_event(Event::Edit(settings));
    }

    /// The sheet's Print / Save (true) or Cancel.
    pub(crate) fn print_answered(&mut self, accept: bool) {
        if !accept {
            return self.print_event(Event::Cancel);
        }
        let Some(Sheet::Open { tab, settings, .. }) = self.print_sheet(false) else {
            return;
        };
        let path = match &settings.destination {
            Destination::Pdf => {
                let title = self
                    .model
                    .tabs
                    .get(&tab)
                    .map(crate::shell::tab_title)
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "page".into());
                match self.download_path(&format!("{title}.pdf")) {
                    Some(path) => path,
                    None => {
                        if let Some(window) = self.w.window.upgrade() {
                            self.toast(&window, "Cannot save: no downloads folder");
                        }
                        return;
                    }
                }
            }
            Destination::Printer(_) => {
                // A directory only this user can open (0700): a fixed name in the shared temp
                // dir could be planted as a link that CEF would write through.
                if self.print_dir.is_none() {
                    self.print_dir = tempfile::Builder::new()
                        .prefix("wubble-print-")
                        .tempdir()
                        .ok();
                }
                match &self.print_dir {
                    Some(dir) => dir.path().join(format!("job-{}.pdf", self.print_serial)),
                    None => {
                        if let Some(window) = self.w.window.upgrade() {
                            self.toast(&window, "Cannot print: no temporary folder");
                        }
                        return;
                    }
                }
            }
        };
        self.print_event(Event::Submit(path));
    }

    /// CEF finished the PDF at `path`: the sheet's, or false when the sheet did not ask.
    pub(super) fn print_rendered(&mut self, path: &Path, ok: bool) -> bool {
        match self.print_sheet(false) {
            Some(Sheet::Working { path: p, .. }) if p == path => {
                self.print_event(Event::Rendered { ok });
                true
            }
            _ => false,
        }
    }

    /// The tab closed (before its overlays go with its view): a sheet for it closes.
    pub(super) fn print_tab_closed(&mut self, tab: TabId) {
        self.print_event(Event::TabClosed(tab));
    }

    /// The sheet is open (`--smoke parity`).
    pub(super) fn print_open(&self) -> bool {
        matches!(self.print_sheet(false), Some(Sheet::Open { .. }))
    }

    /// The print sheet held (one at a time, app-wide): while its overlay is open, or (`cancel`)
    /// while a rule that just closed it has it answer.
    fn print_sheet(&self, cancel: bool) -> Option<Sheet> {
        self.contents
            .of(Kind::PrintSheet)
            .find(|&(_, l, _)| cancel || self.ui.phase(self.w.id, l) == Some(Phase::Open))
            .and_then(|(_, _, c)| match c {
                Content::Print(sheet) => Some(sheet.clone()),
                _ => None,
            })
    }

    pub(crate) fn print_event(&mut self, event: Event) {
        if let Event::Open { tab, .. } = &event
            && !self.views.contains_key(tab)
        {
            return;
        }
        self.print_step(event, false);
    }

    /// One print event on the sheet held (`cancel`: the one a rule just closed). A sheet back
    /// opens its overlay, or stays in it; a printing sheet a rule closed opens again (no rule
    /// stops a job). A sheet the overlays refuse (another tab shows now) ends as its tab closing.
    pub(super) fn print_step(&mut self, event: Event, cancel: bool) {
        let opening = matches!(event, Event::Open { .. });
        let sheet = self.print_sheet(cancel);
        let printed = match &sheet {
            Some(Sheet::Open { tab, .. } | Sheet::Working { tab, .. }) => Some(*tab),
            None => match &event {
                Event::Open { tab, .. } => Some(*tab),
                _ => None,
            },
        };
        let Some(tab) = printed else {
            return;
        };
        let url = self.committed.get(&tab).cloned();
        let (next, mut effects) = print::transition(sheet, event);
        if opening && matches!(next, Some(Sheet::Open { .. })) {
            self.print_serial += 1;
        }
        let layer = Layer::new(Kind::PrintSheet, Some(tab));
        let refused = self.content_after(self.w.id, layer, next.map(Content::Print), false);
        if let Some(Content::Print(sheet)) = refused {
            effects.extend(print::transition(Some(sheet), Event::TabClosed(tab)).1);
        }
        let printed = Some(tab);
        for effect in effects {
            self.print_effect(effect, url.as_deref(), printed);
        }
        if let Some(window) = self.w.window.upgrade() {
            self.render_print(&window);
        }
    }

    /// `printed`: the tab the sheet prints, whose side decides whether its PDF's row is stored.
    fn print_effect(&mut self, effect: Effect, url: Option<&str>, printed: Option<TabId>) {
        match effect {
            Effect::PrintToPdf { tab, path, options } => {
                let started = self.tab_view(tab).map(|v| v.print_to_pdf(&path, &options));
                match started {
                    Some(Ok(())) => self.downloads.reserve(&path),
                    _ => self.print_event(Event::Rendered { ok: false }),
                }
            }
            Effect::Lp(args) => {
                let id = self.w.id;
                let spawned =
                    std::thread::Builder::new()
                        .name("wubble-lp".into())
                        .spawn(move || {
                            let result = lp(&args);
                            let _ = slint::invoke_from_event_loop(move || {
                                crate::with_host_in(id, |host| {
                                    host.print_event(Event::Sent(result))
                                })
                            });
                        });
                if let Err(e) = spawned {
                    self.print_event(Event::Sent(Err(e.to_string())));
                }
            }
            Effect::ListDownload(path) => {
                self.downloads.release(&path);
                let url = url.map_or_else(|| wubble_core::dnd::file_uri(&path), str::to_owned);
                self.add_finished_download(&url, &path, printed);
            }
            Effect::RemoveFile(path) => {
                self.downloads.release(&path);
                let _ = std::fs::remove_file(&path);
            }
            Effect::Toast(text) => {
                if let Some(window) = self.w.window.upgrade() {
                    self.toast(&window, &text);
                }
            }
        }
    }

    fn render_print(&self, window: &AppWindow) {
        let sheet = self.print_sheet(false);
        window.set_print_sheet_view(view(sheet.as_ref(), self.print_serial));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sheet_s_indices_are_its_settings() {
        let printers = vec!["A".to_owned(), "B".to_owned()];
        let s = edited_settings(&printers, 2, "1-2", true, 1, 2, 500, true, 0);
        assert_eq!(s.destination, Destination::Printer("B".into()));
        assert_eq!(s.paper, Paper::A4);
        assert_eq!(s.margins, Margins::Minimum);
        assert_eq!(s.scale, 200);
        assert_eq!(s.copies, 1);
        assert!(s.landscape && s.background);
        let s = edited_settings(&printers, 9, "", false, 7, -1, -5, false, 3);
        assert_eq!(s.destination, Destination::Pdf);
        assert_eq!(
            (s.paper, s.margins, s.scale, s.copies),
            (Paper::Letter, Margins::Default, 100, 3)
        );
    }

    #[test]
    fn the_view_lists_save_as_pdf_first() {
        let open = Sheet::Open {
            tab: TabId(1),
            settings: PrintSettings {
                destination: Destination::Printer("B".into()),
                ..PrintSettings::default()
            },
            printers: vec!["A".into(), "B".into()],
            error: Some("x".into()),
        };
        let v = view(Some(&open), 3);
        assert!(v.open && !v.busy);
        assert_eq!(v.destination, 2);
        assert_eq!(v.error, "x");
        assert_eq!(v.serial, 3);
        use slint::Model as _;
        assert_eq!(v.destinations.row_data(0).as_deref(), Some(SAVE_AS_PDF));
        assert!(!view(None, 3).open);
    }
}
