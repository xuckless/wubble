//! Controls kit B on the testing backend: the specimen (`ui/kit/specimen/slide.slint`) builds,
//! every state has its element id, the roles and values a screen reader gets are the design's
//! (slider min/max/value, tab list and tabs, radio group and radios, the seam's range), keys go
//! where the design sends them, and the slider's focus ring shows after a Tab and not after a
//! click. The registry's callbacks are served here by the same `wubble_core::controls`
//! machines the shell's registry runs. No timing: mock time only.

use std::cell::RefCell;
use std::rc::Rc;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::PointerEventButton;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, SharedString};
use wubble_core::controls::slider::{Cfg, Key as SliderKey};
use wubble_core::controls::{Ring, Segmented, Slider};
use wubble_core::kit_gen::settings::SWATCH;

slint::slint! {
    #[edition = "1.19"]
    import { SlideSpecimen, PillState } from "ui/kit/specimen/slide.slint";
    import { Controls, ControlView } from "ui/components/controls/registry.slint";
    export { SlideSpecimen, PillState, Controls, ControlView }
}

/// The registry, as small as the specimen needs: the machines by slot, views written on demand.
enum Slot {
    Slider(Slider),
    Segmented(Segmented),
    Swatches(Ring),
    Other,
}

fn view(s: &Slot) -> ControlView {
    match s {
        Slot::Slider(s) => {
            let v = s.view();
            ControlView {
                x: v.x as f32,
                scale_x: v.scale_x as f32,
                scale_y: v.scale_y as f32,
                origin: v.origin as f32,
                value: v.value as f32,
                held: v.held,
                bubble_x: v.bubble_x as f32,
                lean: v.lean as f32,
                ..Default::default()
            }
        }
        Slot::Segmented(s) => {
            let v = s.view();
            ControlView {
                x: v.left as f32,
                w: v.width as f32,
                scale_x: v.scale_x as f32,
                scale_y: v.scale_y as f32,
                value: v.selected as f32,
                held: v.held,
                ..Default::default()
            }
        }
        Slot::Swatches(r) => {
            let v = r.view();
            ControlView {
                x: v.x as f32,
                scale_x: v.scale_x as f32,
                scale_y: v.scale_y as f32,
                origin: v.origin as f32,
                value: r.idx() as f32,
                ..Default::default()
            }
        }
        Slot::Other => ControlView {
            scale_x: 1.0,
            scale_y: 1.0,
            ..Default::default()
        },
    }
}

type Reg = Rc<RefCell<Vec<Slot>>>;

fn wire(app: &SlideSpecimen) -> Reg {
    let reg: Reg = Rc::default();
    let views = Rc::new(slint::VecModel::<ControlView>::default());
    let g = app.global::<Controls>();
    g.set_views(views.clone().into());
    let write = {
        let (reg, views) = (reg.clone(), views.clone());
        move |id: i32| {
            let r = reg.borrow();
            let i = id as usize;
            if let Some(s) = r.get(i) {
                views.set_row_data(i, view(s));
            }
        }
    };
    let (r, v) = (reg.clone(), views.clone());
    g.on_acquire(move |kind, value| {
        let slot = match kind {
            1 => Slot::Slider(Slider::new(f64::from(value), 300.0, Cfg::default())),
            2 => Slot::Segmented(Segmented::new(value.max(0.0) as usize)),
            6 => Slot::Swatches(Ring::new(value.max(0.0) as usize)),
            _ => Slot::Other,
        };
        v.push(view(&slot));
        r.borrow_mut().push(slot);
        r.borrow().len() as i32 - 1
    });
    let (r, w) = (reg.clone(), write.clone());
    g.on_configure(move |id, min, max, step| {
        if let Some(Slot::Slider(s)) = r.borrow_mut().get_mut(id as usize) {
            s.configure(Cfg::new(f64::from(min), f64::from(max), f64::from(step)));
        }
        w(id);
    });
    let (r, w) = (reg.clone(), write.clone());
    g.on_resize(move |id, width| {
        if let Some(Slot::Slider(s)) = r.borrow_mut().get_mut(id as usize) {
            s.set_width(f64::from(width));
        }
        w(id);
    });
    let (r, w) = (reg.clone(), write.clone());
    g.on_segment(move |id, i, left, width| {
        match r.borrow_mut().get_mut(id as usize) {
            Some(Slot::Segmented(s)) => {
                s.set_segment(i as usize, f64::from(left), f64::from(width))
            }
            Some(Slot::Swatches(ring)) => ring.place(i as usize),
            _ => {}
        }
        w(id);
    });
    let (r, w) = (reg.clone(), write.clone());
    g.on_set_value(move |id, value| {
        match r.borrow_mut().get_mut(id as usize) {
            Some(Slot::Slider(s)) => s.set(f64::from(value)),
            Some(Slot::Segmented(s)) => s.set(value.max(0.0) as usize),
            Some(Slot::Swatches(ring)) => ring.set(value.max(0.0) as usize),
            _ => {}
        }
        w(id);
    });
    let (r, w) = (reg.clone(), write.clone());
    g.on_key(move |id, code, shift| {
        let key = SliderKey::from_code(code);
        let v = match r.borrow_mut().get_mut(id as usize) {
            Some(Slot::Slider(s)) => key.and_then(|k| s.key(k, shift)).unwrap_or(s.view().value),
            Some(Slot::Segmented(s)) => s.key(key.and_then(SliderKey::dir).unwrap_or(0)) as f64,
            _ => -1.0,
        };
        w(id);
        v as f32
    });
    let (r, w) = (reg.clone(), write.clone());
    g.on_press(move |id, px| {
        if let Some(Slot::Slider(s)) = r.borrow_mut().get_mut(id as usize) {
            s.press(f64::from(px));
        }
        w(id);
        false
    });
    // a drag stands in for the shell's frame too (one step), but its view reaches Slint only
    // with the release's: the frame and the pointer-up handled before `changed shown` runs
    let r = reg.clone();
    g.on_drag(move |id, px| {
        if let Some(Slot::Slider(s)) = r.borrow_mut().get_mut(id as usize) {
            s.drag(f64::from(px));
            s.step(1.0 / 120.0);
        }
    });
    let (r, w) = (reg.clone(), write);
    g.on_release(move |id| {
        let v = match r.borrow_mut().get_mut(id as usize) {
            Some(Slot::Slider(s)) => s.release().map_or(-1.0, |v| v as f32),
            _ => -1.0,
        };
        w(id);
        v
    });
    g.set_wired(true);
    reg
}

fn id(name: &str, app: &SlideSpecimen) -> ElementHandle {
    let qualified = format!("SlideSpecimen::{name}");
    let found: Vec<_> = ElementHandle::find_by_element_id(app, &qualified).collect();
    assert_eq!(found.len(), 1, "one element `{name}`");
    found.into_iter().next().expect("found")
}

/// The accessible element with `role` inside `parent`'s subtree, by label.
fn labelled(app: &SlideSpecimen, label: &str) -> Vec<ElementHandle> {
    ElementHandle::find_by_accessible_label(app, label).collect()
}

fn key(app: &SlideSpecimen, k: impl Into<SharedString> + Clone) {
    let text: SharedString = k.into();
    app.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    app.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn tab(app: &SlideSpecimen) {
    key(app, Key::Tab);
}

#[test]
fn the_specimen_holds_every_control_and_state() {
    init_integration_test_with_mock_time();
    let app = SlideSpecimen::new().expect("the specimen builds");
    let reg = wire(&app);
    // the controls took their slots before the registry was wired: create them again
    app.set_staged(false);
    assert_eq!(
        ElementHandle::find_by_element_id(&app, "SlideSpecimen::sl-rest").count(),
        0,
        "unstaged"
    );
    app.set_staged(true);
    app.show().expect("show");

    for name in [
        "sl-rest",
        "sl-held",
        "sl-focus",
        "sl-compact",
        "sl-disabled",
        "seg-rest",
        "seg-icons",
        "seg-narrow",
        "seg-compact",
        "seg-focus",
        "sw-rest",
        "sw-hover",
        "sw-focus",
        "th-rest",
        "th-on",
        "th-hover",
        "seam-rest",
        "seam-hover",
        "seam-held",
        "seam-focus",
        "seam-hidden",
    ] {
        id(name, &app);
    }

    // the pill's state: held or keyboard focus wins, then hover (all eight inputs)
    let states: Vec<PillState> = app.get_pill_states().iter().collect();
    let want = |i: usize| {
        if i & 2 != 0 || i & 4 != 0 {
            PillState::Held
        } else if i & 1 != 0 {
            PillState::Hover
        } else {
            PillState::Rest
        }
    };
    assert_eq!(states, (0..8).map(want).collect::<Vec<_>>());

    // the slider: role, range, value with its unit
    let zoom = role_of(&app, "Zoom", AccessibleRole::Slider);
    assert_eq!(zoom.accessible_value_minimum(), Some(50.0));
    assert_eq!(zoom.accessible_value_maximum(), Some(200.0));
    assert_eq!(zoom.accessible_value_step(), Some(10.0));
    assert_eq!(zoom.accessible_value().as_deref(), Some("110%"));
    // the segmented control: a tab list of tabs, the chosen one selected
    let space = role_of(&app, "Theme", AccessibleRole::TabList);
    let _ = space;
    let light = role_of(&app, "Light", AccessibleRole::Tab);
    assert_eq!(light.accessible_item_selected(), Some(false));
    let dark = role_of(&app, "Dark", AccessibleRole::Tab);
    assert_eq!(dark.accessible_item_selected(), Some(true));
    assert_eq!(dark.accessible_item_count(), Some(3));
    // the swatches: a radio group of radios, the value's checked
    role_of(&app, "Accent colour", AccessibleRole::RadioGroup);
    let mint = role_of(&app, "Mint", AccessibleRole::RadioButton);
    assert_eq!(mint.accessible_checked(), Some(true));
    assert_eq!(
        role_of(&app, "Sky", AccessibleRole::RadioButton).accessible_checked(),
        Some(false)
    );
    // the seam: the separator's range (a Slider: Slint has no separator role)
    let seam = role_of(
        &app,
        "Resize sidebar. Narrow it to collapse to icons.",
        AccessibleRole::Slider,
    );
    assert_eq!(seam.accessible_value_minimum(), Some(200.0));
    assert_eq!(seam.accessible_value_maximum(), Some(340.0));
    assert_eq!(seam.accessible_value().as_deref(), Some("252"));

    // each swatches' ring starts on its value's swatch, not sliding in from the first
    // (`sw-focus` is on lilac, the second)
    {
        let r = reg.borrow();
        let rings: Vec<&Ring> = r
            .iter()
            .filter_map(|s| match s {
                Slot::Swatches(ring) => Some(ring),
                _ => None,
            })
            .collect();
        assert!(rings.iter().any(|ring| ring.idx() == 1), "lilac's ring");
        for ring in rings {
            assert_eq!(ring.view().x, ring.idx() as f64 * SWATCH::pitch);
        }
    }

    // keys: the first Tab reaches the volume slider, and its ring shows (focus-visible)
    assert!(!app.get_slider_focus_ring());
    tab(&app);
    assert!(app.get_slider_focus_ring(), "Tab shows the ring");
    key(&app, Key::RightArrow);
    assert_eq!(app.get_rest_value(), 65.0, "an arrow steps");
    key(&app, Key::End);
    assert_eq!(app.get_rest_value(), 100.0);
    key(&app, Key::Home);
    assert_eq!(app.get_rest_value(), 0.0);
    key(&app, Key::PageUp);
    assert_eq!(app.get_rest_value(), 10.0, "Page: a tenth of the range");
    // a quick drag, released before a frame: the value it lands on is reported
    let volume = role_of(&app, "Tab volume", AccessibleRole::Slider);
    let (p, size) = (volume.absolute_position(), volume.size());
    let at = |f: f32| LogicalPosition::new(p.x + size.width * f, p.y + size.height / 2.0);
    for e in [
        WindowEvent::PointerPressed {
            position: at(0.3),
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerMoved { position: at(0.8) },
        WindowEvent::PointerReleased {
            position: at(0.8),
            button: PointerEventButton::Left,
        },
    ] {
        app.window().dispatch_event(e);
    }
    mock_elapsed_time(std::time::Duration::from_millis(16));
    let landed = app.get_rest_value();
    assert!(
        landed > 10.0,
        "the release's value reached the setting: {landed}"
    );
    assert_eq!(
        volume.accessible_value(),
        Some(SharedString::from(format!("{landed}"))),
        "the slider and the setting agree"
    );
    // a click focuses without the ring
    let seg = role_of(&app, "Space", AccessibleRole::TabList);
    seg.mock_single_click(PointerEventButton::Left);
    assert!(!app.get_slider_focus_ring());
    volume.mock_single_click(PointerEventButton::Left);
    mock_elapsed_time(std::time::Duration::from_millis(16));
    assert!(
        !app.get_slider_focus_ring(),
        "a click focuses without the ring"
    );
    let before = app.get_rest_value();
    key(&app, Key::LeftArrow);
    assert_eq!(
        app.get_rest_value(),
        before - 1.0,
        "the clicked slider takes keys"
    );

    // the segmented control: arrows choose the neighbour, clamped
    seg.mock_single_click(PointerEventButton::Left);
    let start = app.get_seg_chosen();
    key(&app, Key::RightArrow);
    key(&app, Key::RightArrow);
    key(&app, Key::RightArrow);
    assert_eq!(
        (start, app.get_seg_chosen()),
        (start, 2),
        "clamped at the last"
    );
    key(&app, Key::LeftArrow);
    assert_eq!(app.get_seg_chosen(), 1);

    // the swatches: arrows move the value
    let sky = role_of(&app, "Sky", AccessibleRole::RadioButton);
    sky.mock_single_click(PointerEventButton::Left);
    assert_eq!(
        app.get_swatch(),
        slint::Color::from_rgb_u8(0x7c, 0xc8, 0xff)
    );
    key(&app, Key::RightArrow);
    assert_eq!(
        app.get_swatch(),
        slint::Color::from_rgb_u8(0xb8, 0xa6, 0xff)
    );

    // the seam: arrows nudge
    seam.invoke_accessible_increment_action();
    assert_eq!(app.get_nudged(), 1);
    seam.invoke_accessible_decrement_action();
    seam.invoke_accessible_decrement_action();
    assert_eq!(app.get_nudged(), -1);
}

/// The one element labelled `label` with `role`.
fn role_of(app: &SlideSpecimen, label: &str, role: AccessibleRole) -> ElementHandle {
    let all = labelled(app, label);
    let mut found = all
        .into_iter()
        .filter(|e| e.accessible_role() == Some(role));
    let one = found
        .next()
        .unwrap_or_else(|| panic!("no {role:?} labelled `{label}`"));
    assert!(found.next().is_none(), "two {role:?} labelled `{label}`");
    one
}
