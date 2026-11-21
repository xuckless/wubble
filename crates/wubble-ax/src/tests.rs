//! Fixture trees in CEF's value shape (written as JSON, converted to [`AxValue`] as the engine
//! converts `cef::Value`).

use super::*;
use serde_json::{Value, json};
use value::{parse_locations, parse_updates};

fn ax(v: &Value) -> AxValue {
    AxValue::of(v)
}

/// A small page: a heading, a paragraph with a link, a text field, a button, inside a
/// scrolled container that offsets its children.
fn page() -> Value {
    json!({
        "ax_tree_id": "tree-a",
        "updates": [{
            "has_tree_data": true,
            "tree_data": { "tree_id": "tree-a", "title": "T", "loaded": true, "focus_id": 6 },
            "root_id": 1,
            "nodes": [
                { "id": 1, "role": "rootWebArea", "state": ["focusable"],
                  "location": { "x": 0.0, "y": 0.0, "width": 800.0, "height": 600.0 },
                  "child_ids": [2, 3],
                  "attributes": { "name": "Page title" } },
                { "id": 2, "role": "heading", "offset_container_id": 1,
                  "location": { "x": 10.0, "y": 20.0, "width": 300.0, "height": 40.0 },
                  "attributes": { "name": "Welcome", "hierarchicalLevel": 2 } },
                { "id": 3, "role": "genericContainer", "offset_container_id": 1,
                  "location": { "x": 0.0, "y": 100.0, "width": 800.0, "height": 400.0 },
                  "child_ids": [4, 6, 7],
                  "attributes": { "scrollY": 50 } },
                { "id": 4, "role": "paragraph", "offset_container_id": 3,
                  "location": { "x": 5.0, "y": 60.0, "width": 400.0, "height": 20.0 },
                  "child_ids": [5] },
                { "id": 5, "role": "link", "state": ["focusable", "visited"], "offset_container_id": 3,
                  "location": { "x": 5.0, "y": 60.0, "width": 80.0, "height": 20.0 },
                  "actions": ["doDefault"],
                  "attributes": { "name": "More", "url": "https://example.test/more" } },
                { "id": 6, "role": "textField", "state": ["focusable", "editable"],
                  "offset_container_id": 3,
                  "location": { "x": 5.0, "y": 100.0, "width": 200.0, "height": 24.0 },
                  "attributes": { "name": "Email", "value": "a@b", "placeholder": "you@" } },
                { "id": 7, "role": "button", "state": ["focusable"], "offset_container_id": 3,
                  "location": { "x": 5.0, "y": 140.0, "width": 60.0, "height": 24.0 },
                  "attributes": { "name": "Send", "restriction": "disabled" } }
            ]
        }],
        "events": [{ "event_type": "loadComplete", "id": 1 }]
    })
}

fn first(page: &mut PageTree, v: Value) -> TreeUpdate {
    page.apply(&parse_updates(&ax(&v))).expect("an update")
}

fn by_id(update: &TreeUpdate, id: u64) -> &Node {
    &update
        .nodes
        .iter()
        .find(|(n, _)| n.0 == id)
        .unwrap_or_else(|| panic!("node {id} in the update"))
        .1
}

#[test]
fn roles_names_and_levels_map() {
    let mut tree = PageTree::default();
    let u = first(&mut tree, page());
    assert_eq!(u.nodes.len(), 7);
    assert_eq!(u.tree, Some(TreeInfo::new(NodeId(1))));
    assert_eq!(by_id(&u, 1).role(), Role::RootWebArea);
    assert_eq!(by_id(&u, 1).label(), Some("Page title"));
    let h = by_id(&u, 2);
    assert_eq!(
        (h.role(), h.label(), h.level()),
        (Role::Heading, Some("Welcome"), Some(2))
    );
    assert_eq!(by_id(&u, 3).role(), Role::GenericContainer);
    let link = by_id(&u, 5);
    assert_eq!(link.role(), Role::Link);
    assert_eq!(link.url(), Some("https://example.test/more"));
    assert!(link.is_visited());
    let field = by_id(&u, 6);
    assert_eq!(field.role(), Role::TextInput);
    assert_eq!(
        (field.label(), field.value(), field.placeholder()),
        (Some("Email"), Some("a@b"), Some("you@"))
    );
    let button = by_id(&u, 7);
    assert_eq!((button.role(), button.is_disabled()), (Role::Button, true));
}

#[test]
fn children_keep_their_order() {
    let mut tree = PageTree::default();
    let u = first(&mut tree, page());
    let ids = |id| {
        by_id(&u, id)
            .children()
            .iter()
            .map(|n| n.0)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(1), [2, 3]);
    assert_eq!(ids(3), [4, 6, 7]);
    assert_eq!(ids(4), [5]);
}

#[test]
fn bounds_add_the_offset_containers_less_their_scroll() {
    let mut tree = PageTree::default();
    let u = first(&mut tree, page());
    assert_eq!(
        by_id(&u, 2).bounds(),
        Some(Rect::new(10.0, 20.0, 310.0, 60.0))
    );
    // the link: (5, 60) in container 3, which sits at (0, 100) in the root and is scrolled 50
    assert_eq!(
        by_id(&u, 5).bounds(),
        Some(Rect::new(5.0, 110.0, 85.0, 130.0))
    );
}

#[test]
fn a_transformed_container_maps_what_it_contains() {
    // From CEF 156 (`ax_smoke` with the help link in
    // `<div style="transform:translate(100px,50px) scale(2);transform-origin:0 0">`): the div
    // sits at (0, 0) in the root with its layout offset (8, 186.688) in its transform.
    let v = json!({
        "ax_tree_id": "tree-t",
        "updates": [{
            "root_id": 15,
            "nodes": [
                { "id": 15, "role": "rootWebArea", "offset_container_id": 0,
                  "location": { "x": 0.0, "y": 0.0, "width": 800.0, "height": 600.0 },
                  "child_ids": [31], "attributes": { "scrollX": 0, "scrollY": 0 } },
                { "id": 31, "role": "genericContainer", "offset_container_id": 15,
                  "location": { "x": 0.0, "y": 0.0, "width": 784.0, "height": 18.0 },
                  "transform": "[ 2 0 0 108\n  0 2 0 236.688\n  0 0 1 0\n  0 0 0 1 ]\n",
                  "child_ids": [32] },
                { "id": 32, "role": "link", "offset_container_id": 31,
                  "location": { "x": 0.0, "y": 0.0, "width": 31.0, "height": 17.0 },
                  "attributes": { "name": "Help" } }
            ]
        }]
    });
    let mut tree = PageTree::default();
    let u = first(&mut tree, v);
    let b = by_id(&u, 32).bounds().expect("bounds");
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (108.0, 236.688, 170.0, 270.688));
    let items = tree.flatten(usize::MAX);
    assert_eq!(items[0].bounds.x, 108.0);
    // A transform that is not 16 numbers is left out.
    let mut odd = PageTree::default();
    let mut v = page();
    v["updates"][0]["nodes"][1]["transform"] = json!("[ 1 0 ]");
    let u = first(&mut odd, v);
    assert_eq!(
        by_id(&u, 2).bounds(),
        Some(Rect::new(10.0, 20.0, 310.0, 60.0))
    );
}

#[test]
fn focus_comes_from_the_tree_data_and_stays() {
    let mut tree = PageTree::default();
    assert_eq!(first(&mut tree, page()).focus, NodeId(6));
    // a later update without tree data keeps the focus
    let u = first(
        &mut tree,
        json!({ "ax_tree_id": "tree-a", "updates": [{ "nodes": [
            { "id": 2, "role": "heading", "offset_container_id": 1,
              "location": { "x": 10.0, "y": 20.0, "width": 300.0, "height": 40.0 },
              "attributes": { "name": "Hello", "hierarchicalLevel": 2 } }
        ]}]}),
    );
    assert_eq!(u.focus, NodeId(6));
    assert_eq!(u.nodes.len(), 1, "only the changed node");
    assert_eq!(by_id(&u, 2).label(), Some("Hello"));
}

#[test]
fn actions_follow_role_and_state() {
    let mut tree = PageTree::default();
    let u = first(&mut tree, page());
    let link = by_id(&u, 5);
    assert!(link.supports_action(Action::Click));
    assert!(link.supports_action(Action::Focus));
    assert!(link.supports_action(Action::ScrollIntoView));
    let heading = by_id(&u, 2);
    assert!(!heading.supports_action(Action::Click));
    assert!(!heading.supports_action(Action::Focus));
    assert!(
        !by_id(&u, 1).supports_action(Action::ScrollIntoView),
        "not the root"
    );
}

#[test]
fn node_id_to_clear_drops_the_old_subtree() {
    let mut tree = PageTree::default();
    first(&mut tree, page());
    // container 3 is cleared and now holds only a new paragraph 8
    let u = first(
        &mut tree,
        json!({ "ax_tree_id": "tree-a", "updates": [{ "node_id_to_clear": 3, "nodes": [
            { "id": 3, "role": "genericContainer", "offset_container_id": 1,
              "location": { "x": 0.0, "y": 100.0, "width": 800.0, "height": 400.0 },
              "child_ids": [8] },
            { "id": 8, "role": "paragraph", "offset_container_id": 3,
              "location": { "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0 } }
        ]}]}),
    );
    assert_eq!(
        by_id(&u, 3)
            .children()
            .iter()
            .map(|n| n.0)
            .collect::<Vec<_>>(),
        [8]
    );
    // the focused field went with the old subtree: focus falls back to the root
    assert_eq!(u.focus, NodeId(1));
    let nodes = &tree.trees["tree-a"].nodes;
    assert!(!nodes.contains_key(&6) && !nodes.contains_key(&5));
}

#[test]
fn the_subtree_id_is_stable_per_document() {
    assert_eq!(tree_id_of("tree-a"), tree_id_of("tree-a"));
    assert_ne!(tree_id_of("tree-a"), tree_id_of("tree-b"));
    assert_ne!(tree_id_of("tree-a"), TreeId::ROOT);
    let mut tree = PageTree::default();
    assert_eq!(first(&mut tree, page()).tree_id, tree_id_of("tree-a"));
    let graft = graft_node(tree_id_of("tree-a"));
    assert_eq!(graft.tree_id(), Some(tree_id_of("tree-a")));
}

#[test]
fn a_new_document_starts_the_mirror_over() {
    let mut tree = PageTree::default();
    assert_eq!(tree.document(), None);
    first(&mut tree, page());
    assert_eq!(tree.document(), Some("tree-a"));
    let u = first(
        &mut tree,
        json!({ "ax_tree_id": "tree-b", "updates": [{ "root_id": 1, "nodes": [
            { "id": 1, "role": "rootWebArea", "attributes": { "name": "Next" } }
        ]}]}),
    );
    assert_eq!(u.tree_id, tree_id_of("tree-b"));
    assert_eq!(u.nodes.len(), 1);
    assert_eq!(tree.trees.len(), 1, "the old document is gone");
    assert_eq!(tree.trees["tree-b"].nodes.len(), 1);
    assert_eq!(tree.tree(), Some(tree_id_of("tree-b")));
    assert_eq!(tree.document(), Some("tree-b"));
}

#[test]
fn a_moved_container_moves_what_it_contains() {
    let mut tree = PageTree::default();
    first(&mut tree, page());
    let moves = parse_locations(&ax(&json!([
        { "id": 3, "ax_tree_id": "tree-a",
          "new_location": { "offset_container_id": 1,
                            "bounds": { "x": 0.0, "y": 200.0, "width": 800.0, "height": 400.0 } } },
        { "id": 3, "ax_tree_id": "another-frame",
          "new_location": { "bounds": { "x": 9.0, "y": 9.0, "width": 9.0, "height": 9.0 } } }
    ])));
    assert_eq!(moves.len(), 2);
    let mut updates = tree.move_nodes(&moves);
    assert_eq!(updates.len(), 1, "a frame not on this page moves nothing");
    let u = updates.remove(0);
    let mut ids: Vec<u64> = u.nodes.iter().map(|(n, _)| n.0).collect();
    ids.sort();
    assert_eq!(
        ids,
        [3, 4, 5, 6, 7],
        "the container and everything placed in it"
    );
    assert_eq!(
        by_id(&u, 5).bounds(),
        Some(Rect::new(5.0, 210.0, 85.0, 230.0))
    );
    assert!(tree.move_nodes(&[]).is_empty());
}

#[test]
fn requests_become_page_actions() {
    let mut tree = PageTree::default();
    first(&mut tree, page());
    let request = |action, tree_id, node| ActionRequest {
        action,
        target_tree: tree_id,
        target_node: NodeId(node),
        data: None,
    };
    let here = tree_id_of("tree-a");
    assert_eq!(
        tree.action(&request(Action::Click, here, 5)),
        Some(PageAction::Click { x: 45.0, y: 120.0 })
    );
    assert_eq!(
        tree.action(&request(Action::Focus, here, 6)),
        Some(PageAction::Focus {
            tree: "tree-a".into(),
            node: 6
        })
    );
    assert_eq!(
        tree.action(&request(Action::ScrollIntoView, here, 7)),
        Some(PageAction::ScrollIntoView {
            tree: "tree-a".into(),
            node: 7
        })
    );
    assert_eq!(tree.action(&request(Action::Click, TreeId::ROOT, 5)), None);
    assert_eq!(tree.action(&request(Action::Click, here, 99)), None);
    assert_eq!(tree.action(&request(Action::Expand, here, 5)), None);
}

#[test]
fn missing_keys_read_as_defaults() {
    let u = parse_updates(&ax(
        &json!({ "updates": [{ "nodes": [{ "id": 4 }, { "role": "x" }] }] }),
    ));
    assert_eq!(u.tree_id, "");
    assert_eq!(
        u.changes[0].nodes.len(),
        1,
        "a node without an id is skipped"
    );
    assert_eq!(u.changes[0].nodes[0].location, value::Location::default());
    assert_eq!(role_of("somethingNew"), Role::GenericContainer);
    assert!(parse_locations(&AxValue::Null).is_empty());
}

/// An iframe's document: its own tree, a child of `tree-a`, hosted by `tree-a`'s node 8.
fn frame() -> Value {
    json!({
        "ax_tree_id": "frame-1",
        "updates": [{
            "has_tree_data": true,
            "tree_data": { "tree_id": "frame-1", "parent_tree_id": "tree-a", "focus_id": 2 },
            "root_id": 1,
            "nodes": [
                { "id": 1, "role": "rootWebArea",
                  "location": { "x": 0.0, "y": 0.0, "width": 300.0, "height": 150.0 },
                  "child_ids": [2] },
                { "id": 2, "role": "button", "offset_container_id": 1,
                  "location": { "x": 10.0, "y": 10.0, "width": 40.0, "height": 20.0 },
                  "attributes": { "name": "Play" } }
            ]
        }]
    })
}

/// `tree-a` with node 8, an iframe at (400, 0) hosting `frame-1`, added under the root.
fn host_change() -> Value {
    json!({
        "ax_tree_id": "tree-a",
        "updates": [{
            "nodes": [
                { "id": 1, "role": "rootWebArea", "state": ["focusable"],
                  "location": { "x": 0.0, "y": 0.0, "width": 800.0, "height": 600.0 },
                  "child_ids": [2, 3, 8],
                  "attributes": { "name": "Page title" } },
                { "id": 8, "role": "iframe", "offset_container_id": 1,
                  "location": { "x": 400.0, "y": 0.0, "width": 300.0, "height": 150.0 },
                  "attributes": { "childTreeId": "frame-1" } }
            ]
        }]
    })
}

#[test]
fn frames_keep_their_own_subtree_grafted_at_their_host() {
    let mut tree = PageTree::default();
    // the frame's tree can come before the page's; main and frame updates interleave
    let f = first(&mut tree, frame());
    assert_eq!(f.tree_id, tree_id_of("frame-1"));
    first(&mut tree, page());
    let host = first(&mut tree, host_change());
    first(&mut tree, frame());
    assert_eq!(
        tree.trees.len(),
        2,
        "neither update wiped the other document"
    );
    assert_eq!(tree.trees["tree-a"].nodes.len(), 8);
    assert_eq!(tree.tree(), Some(tree_id_of("tree-a")));
    let graft = by_id(&host, 8);
    assert_eq!(graft.tree_id(), Some(tree_id_of("frame-1")));
    assert!(graft.children().is_empty());
    assert_eq!(by_id(&host, 1).children().len(), 3);

    // a click in the frame lands where the frame sits in the page
    let click = tree.action(&ActionRequest {
        action: Action::Click,
        target_tree: tree_id_of("frame-1"),
        target_node: NodeId(2),
        data: None,
    });
    assert_eq!(click, Some(PageAction::Click { x: 430.0, y: 20.0 }));

    // a frame's location change reaches only its own document
    let moves = parse_locations(&ax(&json!([
        { "id": 2, "ax_tree_id": "frame-1",
          "new_location": { "offset_container_id": 1,
                            "bounds": { "x": 0.0, "y": 0.0, "width": 40.0, "height": 20.0 } } }
    ])));
    let updates = tree.move_nodes(&moves);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].tree_id, tree_id_of("frame-1"));

    // navigating: a new parentless document drops the old page and its frames
    first(
        &mut tree,
        json!({ "ax_tree_id": "tree-b", "updates": [{ "root_id": 1, "nodes": [
            { "id": 1, "role": "rootWebArea" }
        ]}]}),
    );
    assert_eq!(tree.trees.keys().collect::<Vec<_>>(), ["tree-b"]);
}

// ----- the real tree (`ax_smoke`'s dump of CEF 156, checked in)

/// `cargo run -p wubble-engine --example ax_smoke -- --dump FILE` on its bundled page, the
/// page's temporary path replaced.
const AX_SMOKE: &str = include_str!("fixtures/ax_smoke.jsonl");

fn smoke_tree() -> PageTree {
    let mut tree = PageTree::default();
    for line in AX_SMOKE.lines().filter(|l| !l.is_empty()) {
        let v = AxValue::from_json(line).expect("CEF's JSON");
        tree.apply(&parse_updates(&v));
    }
    tree
}

#[test]
fn the_real_tree_reads_in_document_order() {
    let lines: Vec<String> = smoke_tree()
        .flatten(usize::MAX)
        .iter()
        .map(|i| i.line())
        .collect();
    assert_eq!(
        lines,
        [
            "heading 1 \"Welcome\" @8,21 784x37",
            "staticText \"Read the\" @8,80 60x17",
            "link \"docs\" @68,80 29x17",
            "staticText \"first.\" @98,80 34x17",
            "heading 2 \"Sign in\" @8,118 784x27",
            "staticText \"Email\" @8,167 38x17",
            "textField \"Email\" value=\"a@b\" @50,166 185x21",
            "checkBox \"Remember me\" checked @243,168 13x13",
            "button \"Send\" @361,166 47x21",
            "link \"Help\" @8,187 31x17",
        ]
    );
}

#[test]
fn the_real_tree_maps_to_accesskit() {
    let v = AxValue::from_json(AX_SMOKE.lines().next().unwrap()).unwrap();
    let mut tree = PageTree::default();
    let u = tree.apply(&parse_updates(&v)).expect("an update");
    let role = |r: Role| u.nodes.iter().filter(|(_, n)| n.role() == r).count();
    assert_eq!(role(Role::RootWebArea), 1);
    assert_eq!(role(Role::Heading), 2);
    assert_eq!(role(Role::Link), 2);
    assert_eq!(role(Role::TextInput), 1);
    assert_eq!(role(Role::CheckBox), 1);
    assert_eq!(role(Role::Button), 1);
    assert_eq!(role(Role::Form), 1);
    let check = u
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::CheckBox)
        .unwrap();
    assert_eq!(check.1.toggled(), Some(Toggled::True));
    assert_eq!(check.1.label(), Some("Remember me"));
    let h1 = u
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::Heading)
        .unwrap();
    assert_eq!((h1.1.label(), h1.1.level()), (Some("Welcome"), Some(1)));
    // The root's offset container is 0 (none): its bounds are the page's.
    assert_eq!(u.tree.as_ref().map(|t| t.root), Some(NodeId(15)));
    // The text field takes a value (`setValue`); nothing in this tree has `doDefault`.
    let field = u
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::TextInput)
        .unwrap();
    assert!(field.1.supports_action(Action::SetValue));
}

#[test]
fn the_page_focus_is_the_focused_tree_s_node() {
    // The page document's tree data names the focused frame tree and its node.
    let page = json!({
        "ax_tree_id": "page",
        "updates": [{
            "has_tree_data": true,
            "tree_data": { "tree_id": "page", "focus_id": 1, "focused_tree_id": "frame" },
            "root_id": 1,
            "nodes": [
                { "id": 1, "role": "rootWebArea", "child_ids": [2],
                  "location": { "x": 0.0, "y": 0.0, "width": 800.0, "height": 600.0 } },
                { "id": 2, "role": "iframe", "offset_container_id": 1,
                  "location": { "x": 100.0, "y": 50.0, "width": 300.0, "height": 200.0 },
                  "attributes": { "childTreeId": "frame" } }
            ]
        }]
    });
    let frame = json!({
        "ax_tree_id": "frame",
        "updates": [{
            "has_tree_data": true,
            "tree_data": { "tree_id": "frame", "parent_tree_id": "page", "focus_id": 1 },
            "root_id": "1",
            "nodes": [
                { "id": "1", "role": "rootWebArea", "child_ids": ["2"],
                  "location": { "x": 0.0, "y": 0.0, "width": 300.0, "height": 200.0 } },
                { "id": "2", "role": "button", "offset_container_id": "1",
                  "location": { "x": 10.0, "y": 20.0, "width": 50.0, "height": 20.0 },
                  "attributes": { "name": "In frame" } }
            ]
        }]
    });
    let mut tree = PageTree::default();
    tree.apply(&parse_updates(&ax(&page)));
    tree.apply(&parse_updates(&ax(&frame)));
    let items = tree.flatten(usize::MAX);
    assert_eq!(items.len(), 1, "{items:?}");
    let b = &items[0];
    // String ids read as ints (cefclient's `CastToInt`); the frame placed at its host.
    assert_eq!(
        (b.tree.as_str(), b.id, b.name.as_str()),
        ("frame", 2, "In frame")
    );
    assert_eq!((b.bounds.x, b.bounds.y), (110.0, 70.0));
    assert!(!b.focused, "the focus is the frame's root, not the button");
    let focus = json!({
        "ax_tree_id": "page",
        "updates": [{
            "has_tree_data": true,
            "tree_data": { "tree_id": "page", "focus_id": 2, "focused_tree_id": "frame" },
            "nodes": []
        }]
    });
    tree.apply(&parse_updates(&ax(&focus)));
    assert!(tree.flatten(usize::MAX)[0].focused);
}

#[test]
fn an_item_off_the_viewport_is_scrolled_to_before_it_is_reached() {
    let tree = smoke_tree();
    let viewport = tree.viewport().expect("the root's bounds");
    assert_eq!((viewport.width, viewport.height), (800.0, 600.0));
    let send = tree.flatten(usize::MAX)[8].bounds;
    assert_eq!(
        reach(send, viewport),
        Reach::At {
            x: send.x + send.width / 2.0,
            y: send.y + send.height / 2.0
        },
        "on screen"
    );
    let b = |x, y| Bounds {
        x,
        y,
        width: 40.0,
        height: 20.0,
    };
    // Below the fold: its centre goes to the middle; then it is reached.
    assert_eq!(
        reach(b(10.0, 2400.0), viewport),
        Reach::ScrollBy {
            dx: 0.0,
            dy: 2410.0 - 300.0
        }
    );
    assert_eq!(
        reach(b(10.0, 290.0), viewport),
        Reach::At { x: 30.0, y: 300.0 }
    );
    // Above the top and off to the right.
    assert_eq!(
        reach(b(900.0, -100.0), viewport),
        Reach::ScrollBy {
            dx: 920.0 - 400.0,
            dy: -90.0 - 300.0
        }
    );
    assert_eq!(PageTree::default().viewport(), None);
}

#[test]
fn flatten_skips_the_hidden_and_stops_at_the_cap() {
    let mut tree = PageTree::default();
    first(&mut tree, page());
    let all = tree.flatten(usize::MAX);
    assert_eq!(
        all.iter().map(|i| i.kind).collect::<Vec<_>>(),
        [Kind::Heading, Kind::Link, Kind::TextField, Kind::Button]
    );
    assert!(all[2].focused, "focus_id 6 of the page's own tree data");
    assert_eq!(all[0].level, 2);
    // the link: scrolled container 3 at (0, 100), scrolled 50
    assert_eq!((all[1].bounds.x, all[1].bounds.y), (5.0, 110.0));
    assert_eq!(tree.flatten(2).len(), 2);
    let hide = json!({
        "ax_tree_id": "tree-a",
        "updates": [{ "nodes": [
            { "id": 7, "role": "button", "state": ["invisible"], "offset_container_id": 3,
              "attributes": { "name": "Send" } }
        ] }]
    });
    tree.apply(&parse_updates(&ax(&hide)));
    assert_eq!(tree.flatten(usize::MAX).len(), 3);
}

#[test]
fn a_link_item_carries_its_target() {
    let mut tree = PageTree::default();
    first(&mut tree, page());
    let items = tree.flatten(usize::MAX);
    let link = items
        .iter()
        .find(|i| i.kind == Kind::Link)
        .expect("the link");
    assert_eq!(link.url, "https://example.test/more");
    assert!(
        items
            .iter()
            .filter(|i| i.kind != Kind::Link)
            .all(|i| i.url.is_empty())
    );
}

#[test]
fn file_input_name_and_value_update_when_files_are_attached() {
    // Shape from CEF 156 on `<label for="f1">Resume</label><input id="f1" type="file">`.
    let initial = json!({
        "ax_tree_id": "tree-file",
        "updates": [{
            "has_tree_data": true,
            "tree_data": { "tree_id": "tree-file", "focus_id": 11 },
            "root_id": 11,
            "nodes": [
                { "id": 11, "role": "rootWebArea", "offset_container_id": 0,
                  "location": { "x": 0.0, "y": 0.0, "width": 800.0, "height": 600.0 },
                  "child_ids": [17] },
                { "id": 17, "role": "button", "state": ["focusable"], "offset_container_id": 11,
                  "location": { "x": 64.0, "y": 8.0, "width": 253.0, "height": 21.0 },
                  "attributes": {
                      "htmlId": "f1", "htmlTag": "input", "inputType": "file",
                      "name": "Resume: No file chosen", "value": "No file chosen"
                  } }
            ]
        }]
    });
    let mut tree = PageTree::default();
    first(&mut tree, initial);
    let items = tree.flatten(usize::MAX);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "Resume: No file chosen");
    assert_eq!(items[0].value, "No file chosen");

    // Blink's incremental update after `answer_file_chooser` (or `input.files = ...`).
    let attached = json!({
        "ax_tree_id": "tree-file",
        "updates": [{
            "nodes": [
                { "id": 17, "role": "button", "state": ["focusable"], "offset_container_id": 11,
                  "location": { "x": 64.0, "y": 8.0, "width": 253.0, "height": 21.0 },
                  "attributes": {
                      "htmlId": "f1", "htmlTag": "input", "inputType": "file",
                      "name": "Resume: resume.pdf", "value": "resume.pdf"
                  } }
            ]
        }]
    });
    first(&mut tree, attached);
    let items = tree.flatten(usize::MAX);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "Resume: resume.pdf");
    assert_eq!(items[0].value, "resume.pdf");
}
