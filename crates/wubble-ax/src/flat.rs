//! The page as a flat list in reading order ([`PageTree::flatten`]): what a bridge that cannot
//! graft a subtree under the page pane can still show (one shell element per item, in document
//! order, at the node's bounds). It keeps what a screen reader reads and operates (headings,
//! links, buttons, fields, check boxes, radio buttons, images with a name, canvases, and the text between
//! them) and drops the structure (containers, lists, tables, landmarks): an item stands for
//! its whole subtree, so the static text inside a link or a heading is not read twice.

use crate::value::{AxNode, Bounds};
use crate::{DocTree, PageTree};

/// What an item is, for the shell's element per kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Heading,
    Link,
    Button,
    TextField,
    CheckBox,
    RadioButton,
    Image,
    Text,
}

/// Chromium role name to the item kind; `None`: a container, read through.
fn kind_of(role: &str) -> Option<Kind> {
    Some(match role {
        "heading" => Kind::Heading,
        "link" => Kind::Link,
        "button" | "popUpButton" | "toggleButton" | "comboBoxSelect" => Kind::Button,
        "textField" | "searchBox" | "textFieldWithComboBox" | "spinButton" => Kind::TextField,
        "checkBox" | "switch" => Kind::CheckBox,
        "radioButton" => Kind::RadioButton,
        // a canvas is a picture; an agent names one by ref to point inside it
        "image" | "canvas" => Kind::Image,
        "staticText" => Kind::Text,
        _ => return None,
    })
}

/// One item of the flat page.
#[derive(Clone, Debug, PartialEq)]
pub struct AxItem {
    /// The document (Chromium's `AXTreeID`) and the node in it.
    pub tree: String,
    pub id: i32,
    pub kind: Kind,
    /// Chromium's role name.
    pub role: String,
    pub name: String,
    pub value: String,
    /// A link's target (Chromium's `url`); empty for anything else.
    pub url: String,
    /// A heading's level (0: none).
    pub level: i32,
    /// A check box's or radio button's state.
    pub checked: Option<bool>,
    pub focused: bool,
    /// In the page's CSS pixels (frames placed at their host).
    pub bounds: Bounds,
}

impl AxItem {
    /// One line for a dump: role, level, name, value, state, bounds.
    pub fn line(&self) -> String {
        let mut out = self.role.clone();
        if self.level > 0 {
            out.push_str(&format!(" {}", self.level));
        }
        out.push_str(&format!(" {:?}", self.name));
        if !self.value.is_empty() {
            out.push_str(&format!(" value={:?}", self.value));
        }
        if let Some(c) = self.checked {
            out.push_str(if c { " checked" } else { " unchecked" });
        }
        if self.focused {
            out.push_str(" focused");
        }
        let b = self.bounds;
        out.push_str(&format!(
            " @{:.0},{:.0} {:.0}x{:.0}",
            b.x, b.y, b.width, b.height
        ));
        out
    }
}

/// Where a screen reader's action on an item can land: at its centre, or not before the page
/// scrolls by `(dx, dy)` (CSS pixels) to bring that centre to the middle of the viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reach {
    At { x: f64, y: f64 },
    ScrollBy { dx: f64, dy: f64 },
}

/// The [`Reach`] of an item at `item` (an [`AxItem::bounds`]) in a viewport of `viewport`'s
/// size ([`PageTree::viewport`]): a click outside the viewport hits nothing.
pub fn reach(item: Bounds, viewport: Bounds) -> Reach {
    let (x, y) = (item.x + item.width / 2.0, item.y + item.height / 2.0);
    let inside = |at: f64, size: f64| (0.0..size).contains(&at);
    let off = |at: f64, size: f64| {
        if inside(at, size) {
            0.0
        } else {
            at - size / 2.0
        }
    };
    if inside(x, viewport.width) && inside(y, viewport.height) {
        Reach::At { x, y }
    } else {
        Reach::ScrollBy {
            dx: off(x, viewport.width),
            dy: off(y, viewport.height),
        }
    }
}

impl PageTree {
    /// The page's viewport: its document's root bounds (CSS pixels).
    pub fn viewport(&self) -> Option<Bounds> {
        let doc = self.trees.get(&self.root)?;
        Some(doc.nodes.get(&doc.root?)?.location.bounds)
    }

    /// The page's items in document order, frames read at their host, at most `cap` (a page
    /// of thousands of nodes keeps its first ones).
    pub fn flatten(&self, cap: usize) -> Vec<AxItem> {
        let mut out = vec![];
        if let Some(doc) = self.trees.get(&self.root) {
            self.flatten_doc(doc, (0.0, 0.0), cap, &mut out, 0);
        }
        out
    }

    /// The focused node: the page document's `focused_tree_id` and `focus_id`, else the page
    /// document's own focus.
    fn focused(&self) -> Option<(&str, i32)> {
        match &self.focus {
            Some((tree, id)) => Some((tree.as_str(), *id)),
            None => self
                .trees
                .get(&self.root)
                .and_then(|d| d.focus.map(|f| (self.root.as_str(), f))),
        }
    }

    fn flatten_doc(
        &self,
        doc: &DocTree,
        origin: (f64, f64),
        cap: usize,
        out: &mut Vec<AxItem>,
        depth: usize,
    ) {
        // frames inside frames, bounded in case of a cycle
        if depth > self.trees.len() {
            return;
        }
        let Some(root) = doc.root else { return };
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if out.len() >= cap {
                return;
            }
            let Some(node) = doc.nodes.get(&id) else {
                continue;
            };
            // an `ignored` node (a layout wrapper) is read through like a container
            if node.has_state("invisible") {
                continue;
            }
            if let Some(child) = node.text("childTreeId") {
                if let Some(frame) = self.trees.get(child) {
                    let b = doc.absolute(node);
                    self.flatten_doc(frame, (origin.0 + b.x, origin.1 + b.y), cap, out, depth + 1);
                }
                continue;
            }
            match kind_of(&node.role).filter(|k| *k != Kind::Text || node.text("name").is_some()) {
                Some(kind)
                    if kind != Kind::Image
                        || node.text("name").is_some()
                        || node.role == "canvas" =>
                {
                    out.push(self.item(doc, node, kind, origin));
                }
                // a nameless image or text, or a container: read through
                _ => stack.extend(node.child_ids.iter().rev()),
            }
        }
    }

    fn item(&self, doc: &DocTree, node: &AxNode, kind: Kind, origin: (f64, f64)) -> AxItem {
        let mut b = doc.absolute(node);
        b.x += origin.0;
        b.y += origin.1;
        let checked = match node.text("checkedState") {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ if matches!(kind, Kind::CheckBox | Kind::RadioButton) => Some(false),
            _ => None,
        };
        AxItem {
            tree: doc.tree_id.clone(),
            id: node.id,
            kind,
            role: node.role.clone(),
            name: node.text("name").unwrap_or_default().trim().to_owned(),
            value: node.text("value").unwrap_or_default().to_owned(),
            url: match kind {
                Kind::Link => node.text("url").unwrap_or_default().to_owned(),
                _ => String::new(),
            },
            level: node.int("hierarchicalLevel").unwrap_or(0),
            checked,
            focused: self.focused() == Some((doc.tree_id.as_str(), node.id)),
            bounds: b,
        }
    }
}
