//! `--shot <PNG>` (hand run only): the window, written as a PNG when a smoke quits, to compare
//! the shell with the design repo's screenshots. With `--smoke settings` the smoke also seeds
//! the design's scene first: three Spaces (Personal, Work, Study), a few tabs, and the window
//! at the screenshot's 1360x880.

use std::cell::RefCell;
use std::path::PathBuf;

use slint::ComponentHandle;
use wubble_core::model::{Command, Model};
use wubble_core::pages::InternalPage;
use wubble_core::space::{ContainerId, Rgb};
use wubble_core::tab::TabId;
use wubble_ui::AppWindow;

/// The design screenshot's logical window size: `notes/ref-settings.png` is a 1.7x capture of
/// a 1360x880 window (tab rows on a 40 px pitch are 68 px apart in it).
pub const SIZE: (f32, f32) = (1360.0, 880.0);

thread_local! {
    static SHOT: RefCell<Option<(PathBuf, slint::Weak<AppWindow>)>> = const { RefCell::new(None) };
    static PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static FROZEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PARITY: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// `--shot-dir`'s folder (`--smoke gallery`), before the window exists.
pub fn request_dir(dir: PathBuf) {
    DIR.with(|d| *d.borrow_mut() = Some(dir));
}

/// The gallery's file for scene `index` (counted from 0): `dir/NN-name.png`, so a listing
/// sorts in the order the scenes were shot.
pub fn scene_path(dir: &std::path::Path, index: usize, name: &str) -> PathBuf {
    dir.join(format!("{:02}-{name}.png", index + 1))
}

/// `--shot`'s file and the fidelity harness's options, before the window exists.
pub fn request(args: crate::cli::ShotArgs) {
    PATH.with(|p| *p.borrow_mut() = args.path);
    FROZEN.with(|f| f.set(args.frozen_backdrop));
    PARITY.with(|p| *p.borrow_mut() = args.parity);
}

/// `--frozen-backdrop`: the liquid holds still, so two runs shoot the same backdrop.
pub fn frozen_backdrop() -> bool {
    FROZEN.with(std::cell::Cell::get)
}

/// `--parity`'s manifest: the design's Spaces and tabs the gallery seeds before its walk.
pub fn parity() -> Option<PathBuf> {
    PARITY.with(|p| p.borrow().clone())
}

/// Whether a shot was asked for.
pub fn requested() -> bool {
    PATH.with(|p| p.borrow().is_some())
}

/// The window the shot is taken of; sized like the design's screenshot.
pub fn attach(window: &AppWindow) {
    let path = PATH.with(|p| p.borrow().clone());
    if path.is_some() || DIR.with(|d| d.borrow().is_some()) {
        let (w, h) = size();
        window.window().set_size(slint::LogicalSize::new(w, h));
    }
    if let Some(path) = path {
        SHOT.with(|s| *s.borrow_mut() = Some((path, window.as_weak())));
    }
}

/// Writes `window` as `dir/NN-name.png` for the gallery's scene `index`; the window's own
/// size, so `WUBBLE_SHOT_SIZE` and `SLINT_SCALE_FACTOR` set the PNG's.
pub fn take_scene(window: &AppWindow, index: usize, name: &str) -> Result<PathBuf, String> {
    let dir = DIR
        .with(|d| d.borrow().clone())
        .ok_or("no --shot-dir was given")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = scene_path(&dir, index, name);
    take_to(window, &path)?;
    Ok(path)
}

/// Writes frame `k` of the gallery scene `index`'s burst as `dir/NN-name/fKKK.png`, and its
/// time since the show (`ms`) as a line `KKK <ms>` of `dir/NN-name/times.txt`.
pub fn take_frame(
    window: &AppWindow,
    index: usize,
    name: &str,
    k: u32,
    ms: f64,
) -> Result<PathBuf, String> {
    let dir = DIR
        .with(|d| d.borrow().clone())
        .ok_or("no --shot-dir was given")?;
    let burst = scene_path(&dir, index, name).with_extension("");
    std::fs::create_dir_all(&burst).map_err(|e| e.to_string())?;
    let path = burst.join(format!("f{k:03}.png"));
    take_to(window, &path)?;
    let times = burst.join("times.txt");
    let line = format!("{k:03} {ms:.2}\n");
    let mut text = if k == 0 {
        String::new()
    } else {
        std::fs::read_to_string(&times).unwrap_or_default()
    };
    text.push_str(&line);
    std::fs::write(&times, text).map_err(|e| e.to_string())?;
    Ok(path)
}

/// [`SIZE`], or `WUBBLE_SHOT_SIZE=<w>x<h>` (logical px): to shoot at another window's size,
/// such as a screenshot's.
fn size() -> (f32, f32) {
    std::env::var("WUBBLE_SHOT_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or(SIZE)
}

/// The design's Spaces (`slint/tokens.slint` Spaces: names and accents) and a few tabs, two of
/// them pinned as favourites, as commands in order; `model` is read for the opened tabs' ids.
pub fn seed(dispatch: &mut dyn FnMut(Command), model: impl Fn() -> Model) {
    let first = model().active_space();
    let space = |name: &str, tint| Command::NewSpace {
        name: name.into(),
        tint,
        container: ContainerId::DEFAULT,
    };
    dispatch(Command::RenameSpace {
        space: first,
        name: "Personal".into(),
    });
    dispatch(Command::SetSpaceTint {
        space: first,
        tint: Rgb(0x7C, 0xC8, 0xFF),
    });
    dispatch(space("Work", Rgb(0xB8, 0xA6, 0xFF)));
    dispatch(space("Study", Rgb(0x8A, 0xF2, 0xD0)));
    let pages = [
        InternalPage::History,
        InternalPage::Bookmarks,
        InternalPage::Downloads,
        InternalPage::Passwords,
    ];
    for page in pages {
        dispatch(Command::OpenInternal(page));
    }
    let pinned = [InternalPage::Downloads, InternalPage::Passwords];
    let tabs: Vec<TabId> = model()
        .tabs
        .iter()
        .filter(|(_, t)| pinned.iter().any(|p| p.url() == t.url))
        .map(|(id, _)| *id)
        .collect();
    for tab in tabs {
        dispatch(Command::PinTab(tab));
    }
}

/// How long the window settles (springs, fades, the scene's frames) before the shot.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Whether quitting waits for a shot: then the shot is written after [`SETTLE`], and `quit`
/// runs after it.
pub fn defer(quit: fn()) -> bool {
    if SHOT.with(|s| s.borrow().is_none()) {
        return false;
    }
    scroll_page();
    slint::Timer::single_shot(SETTLE, move || {
        take();
        quit();
    });
    true
}

/// `WUBBLE_SHOT_SCROLL=<px>`: scroll the page this far before the shot (a wheel over the
/// page), to compare a part of a native page further down.
fn scroll_page() {
    let Some(px) = std::env::var("WUBBLE_SHOT_SCROLL")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
    else {
        return;
    };
    let Some(window) = SHOT.with(|s| s.borrow().as_ref().and_then(|(_, w)| w.upgrade())) else {
        return;
    };
    wheel(&window, px);
}

/// A wheel over the page, `px` logical px down.
pub fn wheel(window: &AppWindow, px: f32) {
    let position = slint::LogicalPosition::new(
        window.get_content_x() + window.get_content_width() * 0.7,
        window.get_content_y() + 200.0,
    );
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: -px,
        });
}

/// Writes the shot, if one was asked for and not yet written.
fn take() {
    let Some((path, window)) = SHOT.with(|s| s.borrow_mut().take()) else {
        return;
    };
    let Some(window) = window.upgrade() else {
        return;
    };
    match take_to(&window, &path) {
        Ok(()) => println!("smoke: shot {}", path.display()),
        Err(e) => eprintln!("smoke: shot failed: {e}"),
    }
}

/// `window` as rendered, written to `path`.
fn take_to(window: &AppWindow, path: &std::path::Path) -> Result<(), String> {
    let shot = window.window().take_snapshot().map_err(|e| e.to_string())?;
    write_png(path, &shot)
}

fn write_png(
    path: &std::path::Path,
    shot: &slint::SharedPixelBuffer<slint::Rgba8Pixel>,
) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), shot.width(), shot.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(shot.as_bytes())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_files_sort_in_shooting_order() {
        let dir = std::path::Path::new("/tmp/g");
        assert_eq!(
            scene_path(dir, 0, "sidebar-extended"),
            PathBuf::from("/tmp/g/01-sidebar-extended.png")
        );
        assert!(scene_path(dir, 8, "a") < scene_path(dir, 9, "a"));
    }
}
