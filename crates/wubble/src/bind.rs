//! The only code that writes the shell's models: each call replaces them
//! wholesale from a derived value (tab counts are small).

use std::rc::Rc;

use slint::{Color, ModelRc, VecModel};
use wubble_core::space::Rgb;
use wubble_ui::{AppWindow, CommandResult, SpaceItem, TabItem};

use crate::bar::BarRow;
use crate::shell::{ShellView, TabRow};

fn color(Rgb(r, g, b): Rgb) -> Color {
    Color::from_rgb_u8(r, g, b)
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(items)))
}

fn tab_items(rows: &[TabRow]) -> ModelRc<TabItem> {
    model(
        rows.iter()
            .map(|r| TabItem {
                id: r.id,
                title: r.title.as_str().into(),
                glyph: r.glyph.as_str().into(),
                tint: color(r.tint),
            })
            .collect(),
    )
}

pub fn render(window: &AppWindow, v: &ShellView) {
    window.set_pinned_tabs(tab_items(&v.pinned));
    window.set_tabs(tab_items(&v.tabs));
    window.set_spaces(model(
        v.spaces
            .iter()
            .map(|s| SpaceItem {
                id: s.id,
                name: s.name.as_str().into(),
                tint: color(s.tint),
            })
            .collect(),
    ));
    window.set_active_tab_id(v.active_tab);
    window.set_active_space_id(v.active_space);
    window.set_address_host(v.host.as_str().into());
    window.set_address_path(v.path.as_str().into());
}

pub fn render_bar(window: &AppWindow, rows: &[BarRow]) {
    window.set_command_results(model(
        rows.iter()
            .map(|r| CommandResult {
                kind: r.kind,
                title: r.title.as_str().into(),
                subtitle: r.subtitle.as_str().into(),
                shortcut: "".into(),
            })
            .collect(),
    ));
}
