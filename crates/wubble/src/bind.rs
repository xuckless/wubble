//! The only code that writes the shell's models: each call makes them equal
//! to a derived value. They are synced in place rather than replaced, so a
//! re-render (a title, the loading flag, the lifecycle tick every few
//! seconds) does not rebuild every row and drop its hover, press or drag.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use slint::{Color, ComponentHandle, Model, ModelRc, VecModel};
use wubble_core::space::Rgb;
use wubble_core::tab::TabId;
use wubble_engine::nav::Page;
use wubble_ui::{AppWindow, CommandResult, SpaceItem, TabItem};

use crate::bar::BarRow;
use crate::shell::{self, ShellView, TabRow};

fn color(Rgb(r, g, b): Rgb) -> Color {
    Color::from_rgb_u8(r, g, b)
}

/// `rows` into the model `current` already is, when it is one of ours;
/// otherwise (the first render) a new one through `set`.
pub(crate) fn sync<T: Clone + PartialEq + 'static>(
    current: ModelRc<T>,
    rows: Vec<T>,
    set: impl FnOnce(ModelRc<T>),
) {
    match current.as_any().downcast_ref::<VecModel<T>>() {
        Some(model) => wubble_ui::sync(model, rows),
        None => set(ModelRc::from(Rc::new(VecModel::from(rows)))),
    }
}

/// A page icon (straight RGBA, `width * height * 4` bytes) as a shell image: the one place
/// pixels become an `Image`, for icons that just arrived and icons the store restored. Shrunk
/// to `SHELL_ICON_PX` (twice the drawn size) with an area average, so the renderer's bilinear
/// sampling does not alias its edges.
pub fn favicon_image(width: u32, height: u32, rgba: &[u8]) -> slint::Image {
    use wubble_core::favicon::{SHELL_ICON_PX, shrink_rgba};
    let Some((width, height, rgba)) = shrink_rgba(width, height, rgba.to_vec(), SHELL_ICON_PX)
    else {
        return slint::Image::default();
    };
    slint::Image::from_rgba8(
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&rgba, width, height),
    )
}

fn tab_items(rows: &[TabRow], favicons: &BTreeMap<TabId, slint::Image>) -> Vec<TabItem> {
    rows.iter()
        .map(|r| TabItem {
            id: r.id,
            title: r.title.as_str().into(),
            url: r.url.as_str().into(),
            // None yet: the row shows the neutral glyph.
            favicon: shell::tab_id(r.id)
                .and_then(|id| favicons.get(&id))
                .cloned()
                .unwrap_or_default(),
            audible: r.audible,
            muted: r.muted,
            in_split: r.in_split,
            blocked: i32::try_from(r.blocked).unwrap_or(i32::MAX),
            kind: match r.kind {
                shell::RowKind::Tab => 0,
                shell::RowKind::Folder => 1,
            },
            depth: i32::from(r.depth),
            count: i32::try_from(r.count).unwrap_or(i32::MAX),
            collapsed: r.collapsed,
        })
        .collect()
}

pub fn render(window: &AppWindow, v: &ShellView, favicons: &BTreeMap<TabId, slint::Image>) {
    sync(
        window.get_pinned_tabs(),
        tab_items(&v.pinned, favicons),
        |m| window.set_pinned_tabs(m),
    );
    sync(window.get_tabs(), tab_items(&v.tabs, favicons), |m| {
        window.set_tabs(m)
    });
    let spaces = v
        .spaces
        .iter()
        .map(|s| SpaceItem {
            id: s.id,
            name: s.name.as_str().into(),
            tint: color(s.tint),
            icon: s.icon.id().into(),
        })
        .collect();
    sync(window.get_spaces(), spaces, |m| window.set_spaces(m));
    window.set_active_tab_id(v.active_tab);
    window.set_active_space_id(v.active_space);
    window.set_address_host(v.host.as_str().into());
    window.set_address_path(v.path.as_str().into());
    window.set_address_secure(v.secure);
    // Rows synced in place change no property, so no `changed` handler tells
    // the driver: tell it here.
    window.invoke_shell_model_changed();
}

/// Back and forward follow the visible view's history (none without a view), and the address
/// pill's beam its loading. Nothing on screen reads `address-loading` (the driver turns it into
/// the beam), so a change asks for the frame that lets the driver see it.
pub fn render_history(window: &AppWindow, page: Option<&Page>, loading: bool) {
    window.set_can_go_back(page.is_some_and(|p| p.can_go_back));
    window.set_can_go_forward(page.is_some_and(|p| p.can_go_forward));
    if window.get_address_loading() != loading {
        window.set_address_loading(loading);
        window.window().request_redraw();
    }
}

pub fn render_bar(
    window: &AppWindow,
    rows: &[BarRow],
    icons: &HashMap<String, wubble_store::Favicon>,
    tab_icons: &BTreeMap<TabId, slint::Image>,
) {
    let rows = rows
        .iter()
        .map(|r| CommandResult {
            kind: r.kind,
            title: r.title.as_str().into(),
            subtitle: r.subtitle.as_str().into(),
            shortcut: r.shortcut.as_str().into(),
            icon: r
                .icon_tab()
                .and_then(|t| tab_icons.get(&t).cloned())
                .or_else(|| {
                    r.icon_url()
                        .and_then(|u| icons.get(u))
                        .map(|f| favicon_image(f.width, f.height, &f.rgba))
                })
                .unwrap_or_default(),
        })
        .collect();
    sync(window.get_command_results(), rows, |m| {
        window.set_command_results(m)
    });
}
