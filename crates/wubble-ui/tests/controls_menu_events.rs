//! The kit menu's and search capsule's events on the testing backend (mock time, one test: one
//! platform per process). The pointer: crossing a separator between two rows, or the panel's
//! padding, keeps the menu's hover (no Leave, so the highlight glides instead of reappearing in
//! place); leaving the panel straight from a row sends Leave. A menu whose registry is not wired
//! (an unhandled `menu-event` returns the zero effect) chooses nothing. Escape in the empty
//! search capsule goes on to the scope around it, as on the web; with a query it clears it.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};
use wubble_core::controls::menu::{self, Event, MenuState, RowKind};

slint::slint! {
    #[edition = "1.19"]
    import { Menu } from "ui/kit/controls/menu.slint";
    import { SearchCapsule } from "ui/kit/controls/search-capsule.slint";
    import { Controls, ControlView } from "ui/components/controls/registry.slint";
    import { MenuEntry } from "ui/components/model.slint";
    export { Controls, ControlView, MenuEntry }
    export component Pointer inherits Window {
        width: 600px;
        height: 400px;
        in property <bool> on;
        menu := Menu {
            width: 600px;
            height: 400px;
            open: root.on;
            panel-x: 100px;
            panel-y: 50px;
            entries: [
                { label: "A", enabled: true },
                { separator: true },
                { label: "B", enabled: true },
            ];
        }
    }
    export component Bare inherits Window {
        width: 600px;
        height: 400px;
        in property <bool> on;
        in property <[MenuEntry]> rows: [{ label: "Close tab", enabled: true }];
        out property <int> chosen-count;
        Menu {
            width: 600px;
            height: 400px;
            open: root.on;
            entries: root.rows;
            chosen(i) => {
                root.chosen-count += 1;
            }
        }
    }
    export component Esc inherits Window {
        width: 400px;
        height: 100px;
        out property <int> outer-escapes;
        out property <int> escaped-calls;
        in-out property <string> query <=> sc.text;
        public function focus-field() {
            sc.take-focus();
        }
        FocusScope {
            key-pressed(event) => {
                if event.text == Key.Escape {
                    root.outer-escapes += 1;
                }
                accept
            }
            sc := SearchCapsule {
                width: 300px;
                escaped => {
                    root.escaped-calls += 1;
                }
            }
        }
    }
}

fn pump() {
    slint::platform::update_timers_and_animations();
    mock_elapsed_time(Duration::from_millis(16));
}

fn view_open() -> ControlView {
    ControlView {
        scale_x: 1.0,
        scale_y: 1.0,
        origin: 0.5,
        hl: -1,
        jump: true,
        focus: -1,
        done: -1,
        phase: 1,
        ..ControlView::default()
    }
}

fn escape(w: &slint::Window) {
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    pump();
}

#[test]
fn menu_pointer_and_capsule_escape_events() {
    init_integration_test_with_mock_time();

    // ── The pointer over the panel: (kind, arg) of every hover event the menu sends.
    let p = Pointer::new().unwrap();
    let events = Rc::new(RefCell::new(Vec::<(i32, i32)>::new()));
    let g = p.global::<Controls>();
    g.set_views(ModelRc::from(Rc::new(VecModel::from(vec![view_open()]))));
    g.on_acquire(|_, _| 0);
    let e = events.clone();
    g.on_menu_event(move |_, kind, arg, _, _| {
        if kind == 3 || kind == 4 {
            e.borrow_mut().push((kind, arg));
        }
        Default::default()
    });
    p.show().unwrap();
    p.set_on(true);
    pump();
    mock_elapsed_time(Duration::from_secs(2));
    // the panel at (100, 50), 236 wide: padding 6, row A 36, the separator 8, row B 36
    let at = |y: f32| {
        p.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(218.0, y),
        });
        pump();
    };
    at(300.0); // the scrim
    at(53.0); // the top padding
    at(74.0); // row A
    at(96.0); // the separator
    at(118.0); // row B
    p.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(500.0, 300.0),
    });
    pump();
    let seen = events.borrow().clone();
    assert_eq!(
        seen,
        vec![(3, 0), (3, 2), (4, 0)],
        "the padding and the separator are the menu's; leaving from a row is a Leave"
    );

    // The same events through the machine: B is lit by a glide, not a jump.
    let rows = vec![RowKind::Item, RowKind::Separator, RowKind::Item];
    let (mut m, _) = menu::transition(&MenuState::default(), Event::Rows(rows));
    m = menu::transition(
        &m,
        Event::Open {
            via_key: false,
            start: None,
        },
    )
    .0;
    for (kind, arg) in &seen[..2] {
        assert_eq!(*kind, 3);
        m = menu::transition(&m, Event::Enter(usize::try_from(*arg).unwrap())).0;
    }
    assert_eq!((m.hl, m.jump), (Some(2), false), "B glides in from A");

    // ── No registry: the zero effect chooses nothing, for new rows or an open.
    let b = Bare::new().unwrap();
    b.show().unwrap();
    b.set_rows(ModelRc::from(Rc::new(VecModel::from(vec![MenuEntry {
        label: "Reload".into(),
        enabled: true,
        ..MenuEntry::default()
    }]))));
    pump();
    b.set_on(true);
    pump();
    b.set_on(false);
    pump();
    assert_eq!(b.get_chosen_count(), 0, "nothing chosen without a choice");

    // ── Escape in the search capsule.
    let s = Esc::new().unwrap();
    s.show().unwrap();
    s.invoke_focus_field();
    pump();
    escape(s.window());
    assert_eq!(
        (s.get_escaped_calls(), s.get_outer_escapes()),
        (1, 1),
        "empty: escaped() and the key goes on"
    );
    s.set_query("q".into());
    pump();
    escape(s.window());
    assert_eq!(s.get_query(), "");
    assert_eq!(
        (s.get_escaped_calls(), s.get_outer_escapes()),
        (1, 1),
        "with a query: cleared and consumed"
    );
}
