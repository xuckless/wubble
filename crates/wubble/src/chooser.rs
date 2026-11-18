//! The page's file chooser, shown by the OS: the XDG desktop portal on Linux
//! (zenity where there is none), the native dialogs on macOS and Windows,
//! all through the `rfd` crate (MIT). Its async dialogs run on a thread of
//! their own; the continuation is a Slint task (`slint::spawn_local`), so the
//! event loop never blocks. The future resolves once, so the page gets
//! exactly one answer; cancel is `None`.
//!
//! [`dialog_spec`] is the pure part: what to ask the OS for.

use std::path::{Path, PathBuf};

use wubble_engine::page::{AcceptFilter, FileChooserMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    File,
    Files,
    Folder,
    Save,
}

/// What to ask the OS for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogSpec {
    pub pick: Pick,
    pub title: String,
    pub directory: Option<PathBuf>,
    /// The proposed name of a saved file.
    pub file_name: Option<String>,
    /// (name, extensions without dots); empty: any file.
    pub filters: Vec<(String, Vec<String>)>,
}

/// The name of the catch-all filter added after the page's own.
pub const ALL_FILES: &str = "All files";

pub fn dialog_spec(
    mode: FileChooserMode,
    title: &str,
    default_path: &Path,
    accept: &[AcceptFilter],
) -> DialogSpec {
    let pick = match mode {
        FileChooserMode::Open => Pick::File,
        FileChooserMode::OpenMultiple => Pick::Files,
        FileChooserMode::OpenFolder => Pick::Folder,
        FileChooserMode::Save => Pick::Save,
    };
    let empty = default_path.as_os_str().is_empty();
    // A proposed file (Save, or a path with an extension) is split into
    // its folder and name; anything else is the folder to start in.
    let is_file = !empty && (pick == Pick::Save || default_path.extension().is_some());
    let (directory, file_name) = if empty {
        (None, None)
    } else if is_file {
        let parent = default_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf);
        let name = (pick == Pick::Save)
            .then(|| default_path.file_name())
            .flatten()
            .map(|n| n.to_string_lossy().into_owned());
        (parent, name)
    } else {
        (Some(default_path.to_path_buf()), None)
    };
    let mut filters: Vec<(String, Vec<String>)> = if pick == Pick::Folder {
        vec![]
    } else {
        accept.iter().filter_map(filter_of).collect()
    };
    if !filters.is_empty() {
        filters.push((ALL_FILES.into(), vec!["*".into()]));
    }
    DialogSpec {
        pick,
        title: title.into(),
        directory,
        file_name,
        filters,
    }
}

/// One of the page's `accept` entries as a named list of extensions. CEF
/// expands MIME types (`image/*`) into extensions itself; an entry with
/// none is a bare extension (`.png`) or a type CEF could not expand, which
/// filters nothing.
fn filter_of(f: &AcceptFilter) -> Option<(String, Vec<String>)> {
    let mut extensions: Vec<String> = f
        .extensions
        .iter()
        .map(|e| e.trim().trim_start_matches('.').to_string())
        .filter(|e| !e.is_empty())
        .collect();
    if extensions.is_empty()
        && let Some(ext) = f.filter.trim().strip_prefix('.')
        && !ext.is_empty()
    {
        extensions.push(ext.to_string());
    }
    if extensions.is_empty() {
        return None;
    }
    let name = [&f.description, &f.filter]
        .into_iter()
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .map_or_else(|| extensions.join(", "), str::to_string);
    Some((name, extensions))
}

/// The page's answer: the chosen paths, or `None` (cancel) for nothing.
pub fn answer(picked: Option<Vec<PathBuf>>) -> Option<Vec<PathBuf>> {
    picked.filter(|p| !p.is_empty())
}

/// The OS dialog for `spec`, parented to `window` where the platform can.
pub fn dialog(spec: &DialogSpec, window: Option<&slint::Window>) -> rfd::AsyncFileDialog {
    let mut d = rfd::AsyncFileDialog::new();
    if !spec.title.is_empty() {
        d = d.set_title(&spec.title);
    }
    if let Some(dir) = &spec.directory {
        d = d.set_directory(dir);
    }
    if let Some(name) = &spec.file_name {
        d = d.set_file_name(name);
    }
    for (name, extensions) in &spec.filters {
        d = d.add_filter(name, extensions);
    }
    if let Some(window) = window {
        use slint::winit_030::WinitWindowAccessor;
        if let Some(parented) = window.with_winit_window(|w| d.clone().set_parent(w)) {
            d = parented;
        }
    }
    d
}

/// Runs the dialog to its end: the chosen paths, or `None` for cancel.
pub async fn pick(spec: DialogSpec, dialog: rfd::AsyncFileDialog) -> Option<Vec<PathBuf>> {
    let handles = match spec.pick {
        Pick::File => dialog.pick_file().await.map(|f| vec![f]),
        Pick::Files => dialog.pick_files().await,
        Pick::Folder => dialog.pick_folder().await.map(|f| vec![f]),
        Pick::Save => dialog.save_file().await.map(|f| vec![f]),
    };
    answer(handles.map(|h| h.into_iter().map(|f| f.path().to_path_buf()).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept(filter: &str, extensions: &[&str], description: &str) -> AcceptFilter {
        AcceptFilter {
            filter: filter.into(),
            extensions: extensions.iter().map(|e| e.to_string()).collect(),
            description: description.into(),
        }
    }

    #[test]
    fn modes() {
        let p = Path::new("");
        for (mode, pick) in [
            (FileChooserMode::Open, Pick::File),
            (FileChooserMode::OpenMultiple, Pick::Files),
            (FileChooserMode::OpenFolder, Pick::Folder),
            (FileChooserMode::Save, Pick::Save),
        ] {
            let spec = dialog_spec(mode, "T", p, &[]);
            assert_eq!(
                spec,
                DialogSpec {
                    pick,
                    title: "T".into(),
                    directory: None,
                    file_name: None,
                    filters: vec![],
                }
            );
        }
    }

    #[test]
    fn default_paths() {
        let open = |p: &str| dialog_spec(FileChooserMode::Open, "", Path::new(p), &[]);
        assert_eq!(open("/home/u").directory, Some("/home/u".into()));
        let file = open("/home/u/a.txt");
        assert_eq!(
            (file.directory, file.file_name),
            (Some("/home/u".into()), None)
        );
        let save = dialog_spec(FileChooserMode::Save, "", Path::new("/d/report.pdf"), &[]);
        assert_eq!(
            (save.directory, save.file_name),
            (Some("/d".into()), Some("report.pdf".into()))
        );
        let bare = dialog_spec(FileChooserMode::Save, "", Path::new("notes"), &[]);
        assert_eq!(
            (bare.directory, bare.file_name),
            (None, Some("notes".into()))
        );
    }

    #[test]
    fn filters() {
        let spec = dialog_spec(
            FileChooserMode::Open,
            "",
            Path::new(""),
            &[
                accept("image/*", &[".png", ".jpg"], "Images"),
                accept(".pdf", &[], ""),
                accept("application/x-unknown", &[], ""),
                accept("text/csv", &[".csv", " "], ""),
            ],
        );
        assert_eq!(
            spec.filters,
            vec![
                (
                    "Images".to_string(),
                    vec!["png".to_string(), "jpg".to_string()]
                ),
                (".pdf".to_string(), vec!["pdf".to_string()]),
                ("text/csv".to_string(), vec!["csv".to_string()]),
                (ALL_FILES.to_string(), vec!["*".to_string()]),
            ]
        );
        // Nothing usable: no filter at all.
        let none = dialog_spec(
            FileChooserMode::Open,
            "",
            Path::new(""),
            &[accept("application/x-unknown", &[], "")],
        );
        assert!(none.filters.is_empty());
        // A folder has no filters.
        let folder = dialog_spec(
            FileChooserMode::OpenFolder,
            "",
            Path::new(""),
            &[accept(".pdf", &[], "")],
        );
        assert!(folder.filters.is_empty());
    }

    #[test]
    fn nothing_chosen_is_cancel() {
        assert_eq!(answer(None), None);
        assert_eq!(answer(Some(vec![])), None);
        assert_eq!(
            answer(Some(vec!["/a".into()])),
            Some(vec![PathBuf::from("/a")])
        );
    }
}
